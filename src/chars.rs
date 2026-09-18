//! The run a reader has picked out of a listing: where it started, where it has got to,
//! where the keyboard moves it, what each row draws of it and what it copies. It is the
//! run of **characters**, and the rows it touches are the run of rows -- the place the two
//! panes point at each other through ([`CharSelection::rows`]).
//!
//! A column here is a **UTF-16 unit** into the row's text as the row draws it, which is
//! the unit the text engine answers a pointer in and takes a highlight in. A row's text is
//! a [`Line`].
//!
//! Everywhere else in the app a column is a **byte offset** into the file's line, which is
//! what a language server is asked in and answers in (`src/lsp.rs`). So this module owns
//! both counts and every conversion between them ([`columns_of`], [`bytes_of`], and
//! [`slice_of`], which refuses a cut the other two would round): the drawing side
//! converts, and nothing else has to know how a character is counted. [`byte_of_char`] is
//! the same kind of fact for the count a length written for a reader is in.

use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

/// A place in a listing: a row, and a column in UTF-16 units of that row's text. Ordered
/// by row first, which is what puts the two ends of a selection in listing order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Caret {
    pub row: usize,
    pub col: usize,
}

/// A run of characters: where the reader started and where they have got to. The gesture
/// itself -- whether the button is still down -- is the run's `dragging` in `ui/marks.rs`,
/// since a sweep moves the caret and the rows at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CharSelection {
    /// The end that stays put while the other moves.
    anchor: Caret,
    /// Where it has got to. Can be before the anchor, which is what a sweep upwards is.
    lead: Caret,
    /// The column a run of vertical moves is aiming for: the one the lead had before the
    /// first of them, kept while the rows passed through are too short to reach it, so
    /// moving down through a short row and on comes back to it. `None` after anything
    /// that puts the lead at a column of its own -- a press, a sweep, a sideways key.
    goal: Option<usize>,
}

/// A move the keyboard makes of the caret. The sideways ones step by character or by
/// word and cross from a row's start to the row above's end and from its end to the row
/// below's start; the vertical ones keep the column ([`CharSelection::goal`]); the rest
/// go to an end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    RowStart,
    RowEnd,
    ListingStart,
    ListingEnd,
    PageUp,
    PageDown,
}

impl CharSelection {
    /// The empty run a press starts: both ends where the pointer went down.
    pub fn at(caret: Caret) -> Self {
        CharSelection {
            anchor: caret,
            lead: caret,
            goal: None,
        }
    }

    /// The run between two ends, which is what a double press makes of a word and a
    /// triple press of a row's text.
    pub fn between(anchor: Caret, lead: Caret) -> Self {
        CharSelection {
            anchor,
            lead,
            goal: None,
        }
    }

    /// The run reaching from the anchor to `lead`: a sweep, or a shift-click.
    pub fn extended(self, lead: Caret) -> Self {
        CharSelection {
            lead,
            goal: None,
            ..self
        }
    }

    /// The run swept out **by rows** to `row`, as a sweep from a gutter goes: the whole
    /// of every row from the anchor's to `row`, the anchor at its row's start and the
    /// lead at `row`'s end going down, and the other way round going up. Back on the
    /// anchor's own row it is the caret the press left, at the row's start.
    pub fn by_rows(self, row: usize) -> Self {
        let anchor = self.anchor.row;
        if row == anchor {
            return CharSelection::at(Caret {
                row: anchor,
                col: 0,
            });
        }
        let down = row > anchor;
        CharSelection::between(
            Caret {
                row: anchor,
                col: if down { 0 } else { END },
            },
            Caret {
                row,
                col: if down { END } else { 0 },
            },
        )
    }

    /// The run with every row put through `row_of`; `None` where any row has no answer.
    /// Each end is mapped as itself, so the caret stays the end it was swept to.
    pub fn mapped(self, row_of: impl Fn(usize) -> Option<usize>) -> Option<Self> {
        let map = |caret: Caret| {
            Some(Caret {
                row: row_of(caret.row)?,
                col: caret.col,
            })
        };
        Some(CharSelection {
            anchor: map(self.anchor)?,
            lead: map(self.lead)?,
            goal: self.goal,
        })
    }

    /// The run collapsed to its lead: what Escape makes of a selection.
    pub fn collapsed(self) -> Self {
        CharSelection::at(self.lead)
    }

    /// The run after the keyboard has moved its lead by `motion`: from the anchor to the
    /// new lead with `extend` (Shift held), and collapsed to the new lead without. `line`
    /// answers a row's text, `length` is how many rows the listing has and `page` how
    /// many a screen of it holds. The lead is first clamped to the listing and to its
    /// row's text -- a sweep past the rows leaves it at [`END`] -- and nothing moves in a
    /// listing of no rows. A move at a listing's end stays put rather than wrapping.
    pub fn moved(
        self,
        motion: Motion,
        extend: bool,
        line: impl Fn(usize) -> Line,
        length: usize,
        page: usize,
    ) -> Self {
        let Some(last) = length.checked_sub(1) else {
            return self;
        };
        let row = self.lead.row.min(last);
        // The row the lead is on, read once and scanned once: every sideways step is a
        // step over these characters.
        let here_line = line(row);
        let atoms = here_line.atoms();
        let here_units = here_line.units();
        let col = self.lead.col.min(here_units);
        let here = Caret { row, col };
        // How many units a row is: this one from the text already in hand, any other
        // read for it.
        let units = |at: usize| {
            if at == row {
                here_units
            } else {
                line(at).units()
            }
        };
        let start_of = |row: usize| Caret { row, col: 0 };
        let end_of = |row: usize| Caret {
            row,
            col: units(row),
        };
        // A vertical move: to `to`, at the goal column or as near it as the row reaches,
        // and the goal kept for the next.
        let vertical = |to: usize| {
            let goal = self.goal.unwrap_or(col);
            (
                Caret {
                    row: to,
                    col: goal.min(units(to)),
                },
                Some(goal),
            )
        };
        let page = page.max(1);

        let (lead, goal) = match motion {
            Motion::Left | Motion::WordLeft => {
                let step = if motion == Motion::WordLeft {
                    word_before
                } else {
                    before
                };
                (
                    match (step(&atoms, col), row.checked_sub(1)) {
                        (Some(col), _) => Caret { row, col },
                        (None, Some(above)) => end_of(above),
                        (None, None) => here,
                    },
                    None,
                )
            }
            Motion::Right | Motion::WordRight => {
                let step = if motion == Motion::WordRight {
                    word_after
                } else {
                    after
                };
                (
                    match (step(&atoms, col), row < last) {
                        (Some(col), _) => Caret { row, col },
                        (None, true) => start_of(row + 1),
                        (None, false) => here,
                    },
                    None,
                )
            }
            Motion::Up => vertical(row.saturating_sub(1)),
            Motion::Down => vertical((row + 1).min(last)),
            Motion::PageUp => vertical(row.saturating_sub(page)),
            Motion::PageDown => vertical(row.saturating_add(page).min(last)),
            Motion::RowStart => (start_of(row), None),
            Motion::RowEnd => (end_of(row), None),
            Motion::ListingStart => (start_of(0), None),
            Motion::ListingEnd => (end_of(last), None),
        };
        CharSelection {
            anchor: if extend { self.anchor } else { lead },
            lead,
            goal,
        }
    }

    /// Whether nothing is between the ends, which is what a click without a sweep leaves:
    /// nothing to draw, and nothing to copy.
    pub fn is_empty(self) -> bool {
        self.anchor == self.lead
    }

    /// Where the run has got to: the end the caret is drawn at.
    pub fn lead(self) -> Caret {
        self.lead
    }

    /// Where it started: the end that stays put while the lead moves.
    pub fn anchor(self) -> Caret {
        self.anchor
    }

    /// The two ends in listing order, whichever way round they were picked.
    pub fn ends(self) -> (Caret, Caret) {
        if self.lead < self.anchor {
            (self.lead, self.anchor)
        } else {
            (self.anchor, self.lead)
        }
    }

    /// The rows the run touches, in listing order whichever way round it was swept: what
    /// the pair on the other side is lit for, and what a copy with nothing selected
    /// takes. A run within one row is that row alone.
    pub fn rows(self) -> RangeInclusive<usize> {
        let (first, last) = self.ends();
        first.row..=last.row
    }

    /// Whether `row` is one of them.
    pub fn contains_row(self, row: usize) -> bool {
        self.rows().contains(&row)
    }

    /// What row `row` draws of the run, as the range of its `units` to highlight: from the
    /// first end's column on its row, to the second end's on its own, and the whole of
    /// every row between. `None` for a row outside the run, and for an empty run.
    pub fn of_row(self, row: usize, units: usize) -> Option<(usize, usize)> {
        if self.is_empty() {
            return None;
        }
        let (from, to) = self.ends();
        if row < from.row || row > to.row {
            return None;
        }
        let start = if row == from.row {
            from.col.min(units)
        } else {
            0
        };
        let end = if row == to.row {
            to.col.min(units)
        } else {
            units
        };
        Some((start, end))
    }

    /// The text of the run: what each row draws of it, in listing order, joined with
    /// newlines. `line` answers a row's text; a row past the listing answers an empty one.
    pub fn copy(self, line: impl Fn(usize) -> Line) -> String {
        if self.is_empty() {
            return String::new();
        }
        let (from, to) = self.ends();
        (from.row..=to.row)
            .map(|row| {
                let line = line(row);
                let (start, end) = self.of_row(row, line.units()).unwrap_or((0, 0));
                line.slice(start, end)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The column standing for the end of a row's text, whatever its length: clamped to the
/// row's units wherever a column is drawn or copied.
pub const END: usize = usize::MAX;

/// How many UTF-16 units `text` is: the unit a row's columns are counted in.
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The UTF-16 columns of the byte range `bytes` in `line`: a run of the file's line as
/// the row drawing it counts one.
///
/// Both ends are clamped into the line and rounded **down** to a character boundary, so a
/// range out of a stale answer names a run of the line rather than panicking
/// (`AGENTS.md`: never panic on file input, and a server's answer is one).
pub fn columns_of(line: &str, bytes: Range<usize>) -> Range<usize> {
    let at = |byte: usize| units(&line[..boundary(line, byte)]);
    let start = at(bytes.start);
    start..at(bytes.end).max(start)
}

/// The byte range of the UTF-16 columns `columns` in `line`: the other way round, and the
/// same rounding -- a column inside a character two units wide is that character's start.
pub fn bytes_of(line: &str, columns: Range<usize>) -> Range<usize> {
    // Round down: the byte `byte_of_column` names, boundary or not.
    let at = |column| byte_of_column(line, column).unwrap_or_else(|rounded| rounded);
    let start = at(columns.start);
    start..at(columns.end).max(start)
}

/// The text of `line` between the UTF-16 offsets `units`, and `None` where either end
/// falls inside a character, or where the range is empty or the wrong way round.
///
/// The opposite rule to [`bytes_of`]'s, and on purpose. `bytes_of` rounds an end down to
/// the start of the character it is inside, which is what a run named by a stale answer
/// wants: somewhere near beats a panic. A cut has to be refusable instead. The one caller
/// cuts a drawn row's spans at the edges of a link (`cut_at`, `src/ui/code_row.rs`), and a
/// piece taken from inside a character would shift what the row draws without saying so;
/// refusing lets the caller keep the span whole.
pub fn slice_of(line: &str, units: Range<usize>) -> Option<&str> {
    // Refuse: only a boundary, never a byte `byte_of_column` had to round to.
    let from = byte_of_column(line, units.start).ok()?;
    let to = byte_of_column(line, units.end).ok()?;
    (from < to).then(|| &line[from..to])
}

/// Where character `nth` of `text` begins in its bytes, and the text's length when it has
/// no more than `nth` of them: where text kept to `nth` characters is cut.
///
/// One walk, and the answer says both things an elision asks -- where the kept part ends,
/// and, by being short of `text.len()`, that there is more past it. Always a character
/// boundary, so the slice it names cannot panic. Counted in `char`s, which is what a
/// length written for a reader is counted in; a column is UTF-16 units and is
/// [`bytes_of`]'s business.
pub fn byte_of_char(text: &str, nth: usize) -> usize {
    text.char_indices()
        .nth(nth)
        .map_or(text.len(), |(at, _)| at)
}

/// The last character boundary of `line` at or before `byte`, and the line's length for a
/// byte past its end.
fn boundary(line: &str, byte: usize) -> usize {
    let mut at = byte.min(line.len());
    while !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Where UTF-16 column `column` falls in `line`'s bytes: `Ok` where the column is a place
/// in the line, `Err` where it is not -- the start of the character it is inside, or the
/// line's length for a column past the end.
///
/// **The one walk both conversions are made of**, and the two answers are what a policy
/// chooses between. [`bytes_of`] takes the byte either way, which rounds a column down;
/// [`slice_of`] keeps only the `Ok`, which refuses a cut. Each is one line at its own
/// caller, so a third policy is another line and not another walk.
fn byte_of_column(line: &str, column: usize) -> Result<usize, usize> {
    let mut seen = 0;
    for (at, character) in line.char_indices() {
        if seen == column {
            return Ok(at);
        }
        if seen + character.len_utf16() > column {
            return Err(at);
        }
        seen += character.len_utf16();
    }
    // The end of the line is a place in it; anything past that is not.
    if seen == column {
        Ok(line.len())
    } else {
        Err(line.len())
    }
}

/// Where the one-based `line` and `column` **rustc** counts in is in `text`, as a UTF-16
/// offset from its start: what an editor moves a cursor to (`src/ui/pad.rs`).
///
/// rustc counts a column in *characters*, so a tab is one and an accented letter is one,
/// and it separates lines by `\n` alone, having normalised `\r\n` before it numbered
/// anything.
///
/// Two clamps, and they are the same decision twice. `text` is the source **as it is
/// now**, which is not necessarily the source the build was told about -- the reader has
/// usually typed since -- so a column past the end of its line lands at the end of that
/// line, and a line past the end of the text at the end of the text. Being taken to
/// roughly the right place beats not being taken anywhere, and there is nothing here that
/// can fail.
pub fn offset_of(text: &str, line: usize, column: usize) -> usize {
    let line = line.saturating_sub(1);
    let column = column.saturating_sub(1);
    // A character at a time, since the column being counted from is a character count.
    let upto = |row: &str, take: usize| row.chars().take(take).map(char::len_utf16).sum::<usize>();

    let mut offset = 0;
    for (index, row) in text.split_inclusive('\n').enumerate() {
        if index == line {
            // The line break is no part of the line: a column past the end of the text on
            // it stops before the break rather than landing on the line below.
            let row = row.trim_end_matches('\n').trim_end_matches('\r');
            return offset + upto(row, column);
        }
        offset += units(row);
    }

    offset
}

/// A listing's box on screen, in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Where a sweep has got to once the pointer has left the rows: the row on screen nearest
/// the pointer, and the x on that row to ask for the column at -- the pointer's own where
/// it is level with the box, and otherwise the box's near edge, so a pointer past the
/// left or right edge reaches the column at that edge and not the row's end, and the
/// view can be scrolled sideways under it a little at a time.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Reach {
    pub row: usize,
    pub x: f32,
}

/// Where a sweep reaches once the pointer has left the rows: above the box, the first row
/// on screen; below it, the last row on screen; left or right of it, the row level with
/// the pointer; and under the last row of a listing shorter than its box, that row -- each
/// at the x of the pointer clamped into the box (see [`Reach`]). `None` while the pointer
/// is over a row, which answers for itself. `rows_top` is where row 0 sits relative to
/// the box's top -- at or below it before any scroll, above it after -- and `length` how
/// many rows the listing has.
///
/// Nothing at all for a box whose edges do not read left to right, a NaN among them
/// included: freya lays out in `f32` and both clamps below want their bounds in order.
pub fn beyond(
    bounds: Bounds,
    rows_top: f32,
    row_height: f32,
    length: usize,
    x: f32,
    y: f32,
) -> Option<Reach> {
    let last = length.checked_sub(1)?;
    if !(row_height > 0.0) || !(bounds.left <= bounds.right) {
        return None;
    }
    let row_at = |y: f32| ((y - bounds.top - rows_top) / row_height).floor().max(0.0) as usize;
    let inside_x = x >= bounds.left && x < bounds.right;
    let inside_y = y >= bounds.top && y < bounds.bottom;
    let x = x.clamp(bounds.left, (bounds.right - 1.0).max(bounds.left));
    if inside_x && inside_y {
        return (row_at(y) > last).then_some(Reach { row: last, x });
    }
    // The rows on screen, which a sweep beyond the box reaches and no further. Never
    // below the first: a box under half a pixel tall, laid out across a row boundary,
    // has a last row above its first, and `clamp` panics on a range in that order.
    let first_seen = row_at(bounds.top).min(last);
    let last_seen = row_at(bounds.bottom - 0.5).min(last).max(first_seen);
    let row = if y < bounds.top {
        first_seen
    } else if y >= bounds.bottom {
        last_seen
    } else {
        row_at(y).clamp(first_seen, last_seen)
    };
    Some(Reach { row, x })
}

/// What kind of character one is, for a step by word: a word is a run of one kind, and
/// whitespace is what a step passes over first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// Alphanumerics and the underscore: an identifier, a number, a mnemonic.
    Word,
    /// Everything else that is not whitespace: `[`, `,`, `::`.
    Punct,
    Space,
}

impl Class {
    fn of(c: char) -> Self {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Punct
        }
    }
}

/// One character of a row as the steps below see it: the columns it spans and its kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Atom {
    start: usize,
    end: usize,
    class: Class,
}

/// The column of the character before `col` -- the boundary a Left steps to -- and
/// `None` at the row's start. A column inside a character is that character's start.
fn before(atoms: &[Atom], col: usize) -> Option<usize> {
    atoms
        .iter()
        .rev()
        .find(|atom| atom.start < col)
        .map(|atom| atom.start)
}

/// The column after the character at `col` -- the boundary a Right steps to -- and
/// `None` at the row's end. A column inside a character is that character's end.
fn after(atoms: &[Atom], col: usize) -> Option<usize> {
    atoms
        .iter()
        .find(|atom| atom.end > col)
        .map(|atom| atom.end)
}

/// The start of the word before `col`: back over any whitespace, then over the run of
/// characters of one kind -- alphanumerics and underscores, or punctuation -- that ends
/// there. `None` at the row's start.
fn word_before(atoms: &[Atom], col: usize) -> Option<usize> {
    let mut i = atoms.iter().rposition(|atom| atom.start < col)?;
    while atoms[i].class == Class::Space {
        let Some(before) = i.checked_sub(1) else {
            return Some(0);
        };
        i = before;
    }
    let class = atoms[i].class;
    while i > 0 && atoms[i - 1].class == class {
        i -= 1;
    }
    Some(atoms[i].start)
}

/// The end of the word after `col`: over any whitespace, then over the run of characters
/// of one kind that starts there. `None` at the row's end.
fn word_after(atoms: &[Atom], col: usize) -> Option<usize> {
    let mut i = atoms.iter().position(|atom| atom.end > col)?;
    while atoms[i].class == Class::Space {
        i += 1;
        if i == atoms.len() {
            // Trailing whitespace: the row's end, which is the last character's.
            return atoms.last().map(|atom| atom.end);
        }
    }
    let class = atoms[i].class;
    while i + 1 < atoms.len() && atoms[i + 1].class == class {
        i += 1;
    }
    Some(atoms[i].end)
}

/// A row's text as it is drawn. Shared, so a row whose text is already held -- a line
/// of a source file -- is not copied to be drawn.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Line(Arc<str>);

impl Line {
    pub fn text(text: impl Into<Arc<str>>) -> Self {
        Line(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// How many units the text engine counts the row as.
    pub fn units(&self) -> usize {
        units(&self.0)
    }

    /// The row character by character: the columns each spans and the character. **The
    /// row's one walk**, which the atoms and a slice are both made of, so no two of them
    /// can put a column in a different place.
    fn cells(&self) -> impl Iterator<Item = (Range<usize>, char)> + '_ {
        self.0.chars().scan(0, |col, character| {
            let start = *col;
            *col += character.len_utf16();
            Some((start..*col, character))
        })
    }

    /// The row's characters, each as the columns it spans and what kind it is. What every
    /// step along the row is a step over, so none can land inside a character two units
    /// wide.
    fn atoms(&self) -> Vec<Atom> {
        self.cells()
            .map(|(span, character)| Atom {
                start: span.start,
                end: span.end,
                class: Class::of(character),
            })
            .collect()
    }

    /// The text between two columns. A column inside a character that is two units wide
    /// rounds outward, so nothing here can cut a character in half.
    pub fn slice(&self, from: usize, to: usize) -> String {
        let (from, to) = (from.min(to), from.max(to));
        self.cells()
            .filter(|(span, _)| span.end > from && span.start < to)
            .map(|(_, character)| character)
            .collect()
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests;

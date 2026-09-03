//! The run of characters a reader has picked out of a listing, beside the run of rows in
//! `rows.rs`: where it started, where it has got to, where the keyboard moves it, what
//! each row draws of it and what it copies.
//!
//! A column is a **UTF-16 unit** into the row's text as the row draws it, which is the
//! unit the text engine answers a pointer in and takes a highlight in; nothing here
//! converts. A row's text is a [`Line`] of pieces, because a row can hold an element that
//! is not text -- a relocation link -- which the engine counts as one unit and which copies
//! as the whole name it shows.

use std::fmt;

/// A place in a listing: a row, and a column in UTF-16 units of that row's text. Ordered
/// by row first, which is what puts the two ends of a selection in listing order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Caret {
    pub row: usize,
    pub col: usize,
}

/// A run of characters: where the reader started and where they have got to. The gesture
/// itself -- whether the button is still down -- is the row run's `dragging`, since a sweep
/// moves both at once.
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
        let units = |row: usize| line(row).units();
        let row = self.lead.row.min(last);
        let col = self.lead.col.min(units(row));
        let here = Caret { row, col };
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
            Motion::Left => (
                match (line(row).before(col), row.checked_sub(1)) {
                    (Some(col), _) => Caret { row, col },
                    (None, Some(above)) => end_of(above),
                    (None, None) => here,
                },
                None,
            ),
            Motion::Right => (
                match (line(row).after(col), row < last) {
                    (Some(col), _) => Caret { row, col },
                    (None, true) => start_of(row + 1),
                    (None, false) => here,
                },
                None,
            ),
            Motion::WordLeft => (
                match (line(row).word_before(col), row.checked_sub(1)) {
                    (Some(col), _) => Caret { row, col },
                    (None, Some(above)) => end_of(above),
                    (None, None) => here,
                },
                None,
            ),
            Motion::WordRight => (
                match (line(row).word_after(col), row < last) {
                    (Some(col), _) => Caret { row, col },
                    (None, true) => start_of(row + 1),
                    (None, false) => here,
                },
                None,
            ),
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

    /// The two ends in listing order, whichever way round they were picked.
    pub fn ends(self) -> (Caret, Caret) {
        if self.lead < self.anchor {
            (self.lead, self.anchor)
        } else {
            (self.anchor, self.lead)
        }
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
pub fn beyond(
    bounds: Bounds,
    rows_top: f32,
    row_height: f32,
    length: usize,
    x: f32,
    y: f32,
) -> Option<Reach> {
    let last = length.checked_sub(1)?;
    if !(row_height > 0.0) {
        return None;
    }
    let row_at = |y: f32| ((y - bounds.top - rows_top) / row_height).floor().max(0.0) as usize;
    let inside_x = x >= bounds.left && x < bounds.right;
    let inside_y = y >= bounds.top && y < bounds.bottom;
    let x = x.clamp(bounds.left, (bounds.right - 1.0).max(bounds.left));
    if inside_x && inside_y {
        return (row_at(y) > last).then_some(Reach { row: last, x });
    }
    // The rows on screen, which a sweep beyond the box reaches and no further.
    let first_seen = row_at(bounds.top).min(last);
    let last_seen = row_at(bounds.bottom - 0.5).min(last);
    let row = if y < bounds.top {
        first_seen
    } else if y >= bounds.bottom {
        last_seen
    } else {
        row_at(y).clamp(first_seen, last_seen)
    };
    Some(Reach { row, x })
}

/// One piece of a row's text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Piece {
    Text(String),
    /// An element drawn in the row's text in place of a name -- a relocation link -- which
    /// the text engine counts as one unit and which copies as the name.
    Inline(String),
}

impl Piece {
    fn units(&self) -> usize {
        match self {
            Piece::Text(text) => text.encode_utf16().count(),
            Piece::Inline(_) => 1,
        }
    }
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
    /// An inline element, a word of its own.
    Inline,
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

/// One character of a row as the steps see it: the columns it spans and its kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Atom {
    start: usize,
    end: usize,
    class: Class,
}

/// A row's text as it is drawn, in pieces.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Line {
    pub pieces: Vec<Piece>,
}

impl Line {
    /// A row that is plain text.
    pub fn text(text: impl Into<String>) -> Self {
        Line {
            pieces: vec![Piece::Text(text.into())],
        }
    }

    pub fn push_text(&mut self, text: impl Into<String>) {
        self.pieces.push(Piece::Text(text.into()));
    }

    pub fn push_inline(&mut self, name: impl Into<String>) {
        self.pieces.push(Piece::Inline(name.into()));
    }

    /// How many units the text engine counts the row as.
    pub fn units(&self) -> usize {
        self.pieces.iter().map(Piece::units).sum()
    }

    /// The row's characters, each as the columns it spans and what kind it is; an inline
    /// element is one of its own kind. What every step along the row is a step over, so
    /// none can land inside a character two units wide.
    fn atoms(&self) -> Vec<Atom> {
        let mut atoms = Vec::new();
        let mut col = 0;
        for piece in &self.pieces {
            match piece {
                Piece::Inline(_) => {
                    atoms.push(Atom {
                        start: col,
                        end: col + 1,
                        class: Class::Inline,
                    });
                    col += 1;
                }
                Piece::Text(text) => {
                    for c in text.chars() {
                        let end = col + c.len_utf16();
                        atoms.push(Atom {
                            start: col,
                            end,
                            class: Class::of(c),
                        });
                        col = end;
                    }
                }
            }
        }
        atoms
    }

    /// The column of the character before `col` -- the boundary a Left steps to -- and
    /// `None` at the row's start. A column inside a character is that character's start.
    pub fn before(&self, col: usize) -> Option<usize> {
        self.atoms()
            .iter()
            .rev()
            .find(|atom| atom.start < col)
            .map(|atom| atom.start)
    }

    /// The column after the character at `col` -- the boundary a Right steps to -- and
    /// `None` at the row's end. A column inside a character is that character's end.
    pub fn after(&self, col: usize) -> Option<usize> {
        self.atoms()
            .iter()
            .find(|atom| atom.end > col)
            .map(|atom| atom.end)
    }

    /// The start of the word before `col`: back over any whitespace, then over the run
    /// of characters of one kind -- alphanumerics and underscores, or punctuation -- that
    /// ends there. `None` at the row's start.
    pub fn word_before(&self, col: usize) -> Option<usize> {
        let atoms = self.atoms();
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

    /// The end of the word after `col`: over any whitespace, then over the run of
    /// characters of one kind that starts there. `None` at the row's end.
    pub fn word_after(&self, col: usize) -> Option<usize> {
        let atoms = self.atoms();
        let mut i = atoms.iter().position(|atom| atom.end > col)?;
        while atoms[i].class == Class::Space {
            i += 1;
            if i == atoms.len() {
                return Some(self.units());
            }
        }
        let class = atoms[i].class;
        while i + 1 < atoms.len() && atoms[i + 1].class == class {
            i += 1;
        }
        Some(atoms[i].end)
    }

    /// The text between two columns. A column inside a character that is two units wide
    /// rounds outward, so nothing here can cut a character in half; an inline element is
    /// copied whole when its one unit is inside the range.
    pub fn slice(&self, from: usize, to: usize) -> String {
        let (from, to) = (from.min(to), from.max(to));
        let mut out = String::new();
        let mut at = 0;
        for piece in &self.pieces {
            let units = piece.units();
            let (start, end) = (at, at + units);
            at = end;
            if end <= from || start >= to {
                continue;
            }
            match piece {
                Piece::Inline(name) => out.push_str(name),
                Piece::Text(text) => {
                    let mut col = start;
                    for c in text.chars() {
                        let next = col + c.len_utf16();
                        if next > from && col < to {
                            out.push(c);
                        }
                        col = next;
                    }
                }
            }
        }
        out
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) | Piece::Inline(text) => f.write_str(text)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

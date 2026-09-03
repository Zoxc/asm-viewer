//! The run of characters a reader has picked out of a listing, beside the run of rows in
//! `rows.rs`: where it started, where it has got to, what each row draws of it and what it
//! copies.
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
}

impl CharSelection {
    /// The empty run a press starts: both ends where the pointer went down.
    pub fn at(caret: Caret) -> Self {
        CharSelection {
            anchor: caret,
            lead: caret,
        }
    }

    /// The run between two ends, which is what a double press makes of a word and a
    /// triple press of a row's text.
    pub fn between(anchor: Caret, lead: Caret) -> Self {
        CharSelection { anchor, lead }
    }

    /// The run reaching from the anchor to `lead`: a sweep, or a shift-click.
    pub fn extended(self, lead: Caret) -> Self {
        CharSelection { lead, ..self }
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

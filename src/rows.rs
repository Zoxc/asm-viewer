//! The run of rows a reader has picked out of a listing, and what a click, a shift-click
//! and a drag do to it. The run of characters a sweep over a row's text makes lives
//! beside it in `chars.rs`; this is the place the two panes point at each other through,
//! and the gesture both runs share.

use std::ops::RangeInclusive;

/// A run of rows: where the reader started, where they have got to, and whether the
/// pointer is still down. There is no empty value — the UI holds an `Option` around this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowSelection {
    /// The end that stays put while the other moves.
    pub(crate) anchor: usize,
    /// Where it has got to. Can be *before* the anchor, which is what a drag upwards is.
    pub(crate) lead: usize,
    /// Whether the button is still down, which is what tells a row entered under the
    /// pointer from the pointer merely passing over it.
    pub(crate) dragging: bool,
}

impl RowSelection {
    /// The run reaching from the anchor to `row` — a shift-click. It arms the drag as
    /// well, so holding the button after one and sweeping on carries on from there.
    pub fn extended(self, row: usize) -> Self {
        RowSelection {
            lead: row,
            dragging: true,
            ..self
        }
    }

    /// The rows themselves, in listing order whichever way round they were picked.
    pub fn rows(self) -> RangeInclusive<usize> {
        self.anchor.min(self.lead)..=self.anchor.max(self.lead)
    }

    pub fn contains(self, row: usize) -> bool {
        self.rows().contains(&row)
    }
}

#[cfg(test)]
mod tests;

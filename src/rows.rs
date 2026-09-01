//! The run of rows a reader has picked out of a listing, and what a click, a shift-click
//! and a drag do to it.
//!
//! Framework-free and unit-tested for the reason [`crate::lanes`] is: this is geometry
//! decided by cases — an anchor, a lead that can be either side of it, and a drag that is
//! only a drag while the button is down — with no pixels in it and nothing about how the
//! run is painted or copied.
//!
//! Why a *run* of rows rather than one row at a time: freya's character selection wants one
//! rope and one `paragraph()` per line, which an assembly row — a gutter of rects, an
//! address label and up to three elements, one of them a clickable relocation target — is
//! not, so the honest thing here is rows. `notes/Goals.md` carries that as a `[D]` with the
//! reasoning read out of the freya sources. Given that, a run is what a
//! reader wants to paste — a basic block, a loop, the lines a bug is in — and it costs the
//! two lists nothing, since they already work out per row what each row is.

use std::ops::RangeInclusive;

/// A run of rows: where the reader started, where they have got to, and whether the
/// pointer is still down.
///
/// There is no empty value. A selection either exists or there is none at all, which the
/// UI holds as an `Option` around this — an anchor with nothing selected is a state
/// nothing would draw and nothing would copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowSelection {
    /// Where the run was started, and the end that stays put while the other moves.
    anchor: usize,
    /// Where it has got to. Can be before the anchor, above it in the listing, which is
    /// what a drag upwards is.
    lead: usize,
    /// Whether the pointer is still down on it, which is what tells a row moving under
    /// the pointer from a row the pointer merely passes over.
    dragging: bool,
}

impl RowSelection {
    /// The one-row run a press starts, which is a drag until the button comes up.
    pub fn at(row: usize) -> Self {
        RowSelection {
            anchor: row,
            lead: row,
            dragging: true,
        }
    }

    /// Every row of a listing `rows` long, and `None` for a listing with no rows: there is
    /// no such thing as a selection of nothing.
    pub fn all(rows: usize) -> Option<Self> {
        rows.checked_sub(1).map(|last| RowSelection {
            anchor: 0,
            lead: last,
            dragging: false,
        })
    }

    /// The run reaching from the anchor to `row` — a shift-click, and the one gesture that
    /// can select more rows than fit on screen, since a drag can only reach the rows the
    /// scroll view has mounted.
    ///
    /// A shift-click is a press like any other, so it arms the drag as well: holding the
    /// button after one and sweeping on carries on from where it reached.
    pub fn extended(self, row: usize) -> Self {
        RowSelection {
            lead: row,
            dragging: true,
            ..self
        }
    }

    /// The same, but only while the button is down. A row entered with no button held is
    /// the pointer passing over it, and the panes' own hover is what answers that.
    pub fn dragged_to(self, row: usize) -> Self {
        if self.dragging {
            self.extended(row)
        } else {
            self
        }
    }

    /// The run with the drag over. The rows it holds are untouched: letting go is the end
    /// of the gesture and not the end of the selection.
    pub fn released(self) -> Self {
        RowSelection {
            dragging: false,
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

    /// The run as text: each row's own line, in listing order, newline-separated.
    ///
    /// Here rather than at the two call sites because the order is the part worth pinning:
    /// a drag that started at the bottom and ended at the top copies the same text as one
    /// that went the other way, which is what every other listing in the world does and is
    /// exactly the thing that would be got wrong by pasting `anchor..lead` in.
    pub fn text(self, line: impl Fn(usize) -> String) -> String {
        self.rows().map(line).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_selects_the_row_it_is_on_and_starts_a_drag() {
        let selection = RowSelection::at(7);
        assert_eq!(selection.rows(), 7..=7);
        assert!(selection.contains(7));
        assert!(!selection.contains(6));
        assert_eq!(selection.dragged_to(9).rows(), 7..=9);
    }

    #[test]
    fn a_run_reads_the_same_way_round_whichever_way_it_was_picked() {
        let down = RowSelection::at(4).extended(9);
        let up = RowSelection::at(9).extended(4);
        assert_eq!(down.rows(), 4..=9);
        assert_eq!(up.rows(), 4..=9);
    }

    #[test]
    fn extending_moves_the_lead_and_leaves_the_anchor() {
        // Two shift-clicks either side of the anchor, which is what makes the second one
        // a correction of the first rather than a run from wherever the first one ended.
        let selection = RowSelection::at(5).extended(9).extended(2);
        assert_eq!(selection.rows(), 2..=5);
    }

    #[test]
    fn a_row_entered_with_the_button_up_is_not_a_drag() {
        let selection = RowSelection::at(3).released();
        assert_eq!(selection.dragged_to(20).rows(), 3..=3);
        // Shift-clicking still extends it: letting go ended the gesture, not the run.
        // And it arms the next drag, so sweeping on from a shift-click carries on.
        let extended = selection.extended(20);
        assert_eq!(extended.rows(), 3..=20);
        assert_eq!(extended.dragged_to(25).rows(), 3..=25);
    }

    #[test]
    fn releasing_keeps_the_rows() {
        let selection = RowSelection::at(3).dragged_to(6);
        assert_eq!(selection.released().rows(), 3..=6);
    }

    #[test]
    fn select_all_covers_the_listing_and_an_empty_listing_selects_nothing() {
        let selection = RowSelection::all(3).unwrap();
        assert_eq!(selection.rows(), 0..=2);
        // Not a drag: nothing is under the pointer, so a row entered afterwards must not
        // be swept into it.
        assert_eq!(selection.dragged_to(9).rows(), 0..=2);
        assert_eq!(RowSelection::all(0), None);
    }

    #[test]
    fn text_is_the_rows_in_listing_order() {
        let line = |row: usize| format!("row {row}");
        assert_eq!(
            RowSelection::at(2).extended(4).text(line),
            "row 2\nrow 3\nrow 4"
        );
        // The backwards drag copies forwards.
        assert_eq!(
            RowSelection::at(4).extended(2).text(line),
            "row 2\nrow 3\nrow 4"
        );
        assert_eq!(RowSelection::at(1).text(line), "row 1");
    }
}

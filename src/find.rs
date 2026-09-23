//! What a find bar asks of one code pane: which hit a step goes to. Framework-free.
//!
//! Where a pattern hits in a line is [`Matcher::marks`](crate::filter::Matcher::marks)
//! asked of the line as it is drawn, so the three toggles mean one thing in all four
//! boxes. A hit's columns are bytes of that line (`src/chars.rs`), which is what the
//! caret, the selection and the wash are all placed by.

use std::ops::Range;

use crate::chars::{Caret, CharSelection};

/// One hit in a listing: the line it is on and the columns it covers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    pub row: usize,
    pub columns: Range<usize>,
}

/// Which way a step goes. Named here, beside the step itself, so the ask a bar holds, the
/// buttons that write it and the walk through an object's code all say it the one way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// On to the hit after the one the pane is on.
    Forward,
    /// Back to the one before it.
    Back,
}

/// Which hit a step goes to: the one after `at`, or the one before it going back, wrapping
/// at the ends. `None` where there is nothing to step to. `run` is what the pane has
/// picked out now.
///
/// **`at` is the hit the pane is on, for as long as `run` is still that hit.** A step
/// leaves the run on the hit it landed on, and asking the caret again would answer that
/// hit rather than the next. Anything that has moved the run since -- a click, a key --
/// leaves `at` naming a place the reader has left, so the step reads the caret instead,
/// as a first step does: a find starts from where the reader is looking.
pub fn step(
    hits: &[Hit],
    at: Option<usize>,
    run: CharSelection,
    direction: Direction,
) -> Option<usize> {
    let last = hits.len().checked_sub(1)?;

    let on = |hit: &Hit| {
        let start = Caret {
            row: hit.row,
            col: hit.columns.start,
        };
        let end = Caret {
            row: hit.row,
            col: hit.columns.end,
        };
        run.ends() == (start, end)
    };
    if let Some(at) = at.filter(|at| hits.get(*at).is_some_and(on)) {
        return Some(match direction {
            Direction::Back => at.checked_sub(1).unwrap_or(last),
            Direction::Forward => (at + 1) % hits.len(),
        });
    }

    let place = |hit: &Hit| (hit.row, hit.columns.start);
    let from = run.lead();
    let caret = (from.row, from.col);
    match direction {
        // The last hit that ends at or before the caret, so a caret sitting inside one
        // steps out of it rather than back onto it.
        Direction::Back => hits
            .iter()
            .rposition(|hit| (hit.row, hit.columns.end) <= caret)
            .or(Some(last)),
        // The first hit at or after the caret: a caret put at the start of a hit by a
        // click means that hit, which is what the reader pointed at.
        Direction::Forward => hits.iter().position(|hit| place(hit) >= caret).or(Some(0)),
    }
}

#[cfg(test)]
mod tests;

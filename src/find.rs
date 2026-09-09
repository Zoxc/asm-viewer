//! What a find bar asks of one code pane: where a pattern hits in a line as it is drawn,
//! and which hit a step goes to. Framework-free.
//!
//! Not `filter.rs` and not `fuzzy.rs`. Those two ask a question about a *row* -- does this
//! name belong in the list, and how well -- and answer yes or no. This one asks where in a
//! line the pattern is, in the columns a pane counts, because what comes back is washed
//! and selected in the text itself. The pattern is a [`Matcher`] all the same, so the
//! three toggles mean one thing in all four boxes.
//!
//! **Columns and not bytes.** A pane counts a column in UTF-16 units of the line *as
//! drawn* (`src/chars.rs`), which is what the caret, the selection and the wash are all
//! placed by, so that is what a hit is in.

use std::ops::Range;

use crate::chars::{self, Caret, Line, Piece};
use crate::filter::Matcher;

/// One hit in a listing: the line it is on and the columns it covers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    pub row: usize,
    pub columns: Range<usize>,
}

/// Where `matcher` hits in `line`, as columns, in order.
///
/// **A run of adjacent text is matched whole**, not piece by piece. An assembly line is
/// pushed one span at a time -- the mnemonic, the padding, each operand -- so matching
/// each piece on its own would answer nothing for `mov rax`, which is three of them.
///
/// **An inline element matches as a unit.** It is one column to the text engine and its
/// text is a whole symbol name (`Piece::Inline`), so there are no columns inside it to
/// mark: either the pattern is somewhere in the name, and the one column the element is
/// drawn at is the hit, or it is not. That is also what a reader sees, the element being
/// drawn as one thing. It ends the run either way: a pattern cannot straddle it.
pub fn hits_in(line: &Line, matcher: &Matcher) -> Vec<Range<usize>> {
    let mut hits = Vec::new();
    // The run being gathered: where it starts, in columns, and its text so far.
    let mut start = 0;
    let mut run = String::new();
    let mut column = 0;

    let flush = |start: usize, run: &mut String, hits: &mut Vec<Range<usize>>| {
        for bytes in matcher.marks(run) {
            let columns = chars::columns_of(run, bytes);
            hits.push(start + columns.start..start + columns.end);
        }
        run.clear();
    };

    for piece in &line.pieces {
        match piece {
            Piece::Text(text) => {
                if run.is_empty() {
                    start = column;
                }
                run.push_str(text);
                column += chars::units(text);
            }
            Piece::Inline(name) => {
                flush(start, &mut run, &mut hits);
                if matcher.marked(name) {
                    hits.push(column..column + 1);
                }
                column += 1;
            }
        }
    }
    flush(start, &mut run, &mut hits);

    hits
}

/// Which hit a step goes to: the one after `at`, or before it with `back`, wrapping at the
/// ends. `None` where there is nothing to step to.
///
/// **`at` is where the pane already is**, and it wins over the caret: once a step has
/// landed the caret sits inside that hit, and asking the caret again would answer the hit
/// the pane is on rather than the next one. The caret is what a *first* step reads --
/// the bar just opened, or the reader has clicked since -- so a find starts from where
/// they are looking and not from the top.
pub fn step(hits: &[Hit], at: Option<usize>, from: Caret, back: bool) -> Option<usize> {
    let last = hits.len().checked_sub(1)?;

    if let Some(at) = at.filter(|at| *at <= last) {
        return Some(match back {
            true => at.checked_sub(1).unwrap_or(last),
            false => (at + 1) % hits.len(),
        });
    }

    let place = |hit: &Hit| (hit.row, hit.columns.start);
    let caret = (from.row, from.col);
    match back {
        // The last hit that ends at or before the caret, so a caret sitting inside one
        // steps out of it rather than back onto it.
        true => hits
            .iter()
            .rposition(|hit| (hit.row, hit.columns.end) <= caret)
            .or(Some(last)),
        // The first hit at or after the caret: a caret put at the start of a hit by a
        // click means that hit, which is what the reader pointed at.
        false => hits.iter().position(|hit| place(hit) >= caret).or(Some(0)),
    }
}

#[cfg(test)]
mod tests;

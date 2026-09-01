//! The symbols a source line was compiled into, and which of them a tab follows.
//!
//! The crate answers a file and a line with the symbols holding code from it
//! ([`Object::symbols_at_line`]), one object at a time. This asks that of every object
//! that is open and then chooses, because one line compiles into as many symbols as there
//! are instantiations of it, times as many objects as hold one — 9 374 of them for
//! `core/src/ptr/mod.rs:848` on this app's own binary.
//!
//! Both halves are blocking: the first ask against an object builds its whole index, which
//! is seconds on a large one. They belong on the analysis worker and nowhere else.

use std::collections::HashMap;
use std::sync::Arc;

use analysis::{Object, Symbol};

/// Every symbol in `objects` holding code compiled from `file` at `line`, object by object
/// and, within one, in the crate's own address-then-name order.
///
/// `file` is matched exactly, on the string the debug info said: two objects whose
/// `DW_AT_comp_dir` disagree do not join, and nothing here asks the filesystem about a
/// path.
pub fn compiled_from(objects: &[Arc<Object>], file: &str, line: u32) -> Vec<Symbol> {
    objects
        .iter()
        .flat_map(|object| {
            object
                .symbols_at_line(file, line)
                .into_iter()
                .map(|data| Symbol {
                    object: object.clone(),
                    data,
                })
        })
        .collect()
}

/// Which of `candidates` a tab follows: the one visited most recently, else the first.
///
/// `recent` is where the reader has been, newest first, **with the symbol already on
/// screen at its head** — which is the whole of what keeps reading down the lines of a
/// generic function inside one instantiation. Nothing is pushed onto the history between
/// two clicks in one function, so without that head the answer would fall through to the
/// order below, which differs line by line.
///
/// And that order is arbitrary: the first candidate is the lowest-addressed symbol of the
/// first object that answered. It is a tie-break and not a judgement; Step 5's picker is
/// where a reader says which instance they meant.
pub fn pick(candidates: &[Symbol], recent: &[Symbol]) -> Option<Symbol> {
    // Indexed rather than scanned: one line can answer with thousands of symbols and a
    // history holds two hundred, so the nested walk is a million pointer compares. The
    // key is `Symbol`'s own equality written out -- both `Arc`s, not just the data's.
    let where_at: HashMap<(usize, usize), usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, symbol)| (identity(symbol), index))
        .collect();

    recent
        .iter()
        .find_map(|symbol| where_at.get(&identity(symbol)))
        .and_then(|index| candidates.get(*index))
        .or_else(|| candidates.first())
        .cloned()
}

/// A symbol as a hashable key: the pair of `Arc` addresses its `PartialEq` compares.
fn identity(symbol: &Symbol) -> (usize, usize) {
    (
        Arc::as_ptr(&symbol.object).addr(),
        Arc::as_ptr(&symbol.data).addr(),
    )
}

#[cfg(test)]
mod tests;

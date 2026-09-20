//! The symbols a source line was compiled into, and which of them a tab follows.
//!
//! The crate answers a file and a line with the symbols holding code from it
//! ([`Object::symbols_from_lines`]), one object at a time. This asks that of every object
//! that is open and then chooses, because one line compiles into as many symbols as there
//! are instantiations of it, times as many objects as hold one — 9 374 of them for
//! `core/src/ptr/mod.rs:848` on this app's own binary.
//!
//! A function is the same question over its lines: every symbol holding code from any of
//! them, each once, which is how the picker lists a generic function's instances.
//!
//! Both halves are blocking: the first ask against an object builds its whole index, which
//! is seconds on a large one. They belong on the analysis worker and nowhere else.

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::sync::Arc;

use analysis::{Object, PlacedAddress, Symbol, SymbolData};

/// Every symbol in `objects` holding code compiled from `file` over `lines`, object by
/// object and, within one, in the crate's own order: by placed address. A symbol holding
/// code from several of the lines is one hit; one line is `line..=line`.
///
/// `file` is matched exactly, on the string the debug info said: two objects whose
/// `DW_AT_comp_dir` disagree do not join, and nothing here asks the filesystem about a
/// path.
pub fn compiled_from(
    objects: &[Arc<Object>],
    file: &str,
    lines: RangeInclusive<u32>,
) -> Vec<Symbol> {
    objects
        .iter()
        .flat_map(|object| {
            object
                .symbols_from_lines(file, lines.clone())
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
/// generic function inside one instantiation. Nothing is recorded between two clicks in
/// one function, so without that head the answer would fall through to the order below,
/// which differs line by line.
///
/// And that order is arbitrary: the first candidate is the lowest-placed symbol of the
/// first object that answered. It is a tie-break and not a judgement; the Locations panel
/// is where a reader says which instance they meant.
pub fn pick(candidates: &[Symbol], recent: &[Symbol]) -> Option<Symbol> {
    // Indexed rather than scanned: one line can answer with thousands of symbols and the
    // record of visits holds two hundred, so the nested walk is a million pointer compares.
    let offered: HashSet<&Symbol> = candidates.iter().collect();

    recent
        .iter()
        .find(|symbol| offered.contains(symbol))
        .or_else(|| candidates.first())
        .cloned()
}

/// The lowest **placed** address any of `symbols` starts at, or [`None`] for none of them
/// that is in a section.
///
/// Placed ([`SymbolData::placed`]), which is the section's bias added: that is the space
/// the listing of a whole object's code draws in and the space `symbol_at_placed` answers
/// in, so a place worked out here names the row a reader would land on.
///
/// For an answer of [`Object::symbols_from_lines`] this is its first symbol's place, since
/// the crate answers in placed order and every symbol it names is in a section. It is still
/// the lowest and not the first because it takes any slice, and one built some other way
/// carries neither guarantee.
pub fn lowest_placed(symbols: &[Arc<SymbolData>]) -> Option<PlacedAddress> {
    symbols
        .iter()
        .filter(|data| data.section.is_some())
        .map(|data| data.placed(data.address))
        .min()
}

#[cfg(test)]
mod tests;

//! Line info read the other way round: a source file and a line in it, to the symbols whose
//! code that line produced.
//!
//! The forward direction ([`Object::line_info`]) is asked about one address range and answers
//! with rows. Nothing there can answer "which functions was this line compiled into", which is
//! a question about the whole object rather than about one symbol — so it is answered from an
//! index, built on the first ask and never at parse time, the way [`super::DwarfCache`] builds
//! the context itself.
//!
//! Two things are decided here and written down rather than left to be discovered:
//!
//! * **A file is matched exactly, on the string `addr2line` renders.** That is by construction
//!   the string [`LineInfo::files`](super::LineInfo::files) spells, so a caller holding a file
//!   name out of the forward direction can hand it straight back. Nothing here normalises a
//!   path or asks the filesystem about one: a path in DWARF is what the producer said, not a
//!   place. Two objects whose `DW_AT_comp_dir` disagree therefore do not join, which is a
//!   cross-object question and not this crate's to answer.
//! * **The answer is symbols, not ranges.** Where inside a symbol the line's code sits is the
//!   forward direction's question and is already answered, so a caller wanting the ranges asks
//!   the symbol it was given. One definition of "which rows are this line's" rather than two
//!   that can drift.
//!
//! **What it costs, measured** (release, first ask against a fully parsed file). On
//! `viewer-sample` — one object, 115 577 symbols, 267 MB of DWARF — 2.2 s, of which **2.0 s is
//! taking every symbol's extent** and 0.23 s is the line-program walk; the index is 2 096 files
//! and 624 544 `(line, symbol)` pairs, 10 MB of them, and holding the line programs the walk
//! parsed takes the process from 756 MB to 1.23 GB. On `libanalysis-sample.rlib` — 196 objects,
//! 4 164 symbols — 94 ms for all of them together, 862 files and 25 870 pairs. Every ask after
//! the first is two binary searches: 5 µs, or 750 µs for the worst line in the repo.
//!
//! So the extent pass is nine tenths of the build, and it is paid deliberately: it is a DIE
//! walk of the whole object, which is what an extent that agrees with
//! [`SymbolData::line_info`] costs. Attributing by [`SymbolData::estimate_size`] instead would
//! be one binary search per row and no DWARF at all, and would let the index name a symbol
//! whose own line info does not name the line back — the one thing a caller walking index →
//! symbol → rows cannot survive.
//!
//! That one line maps into many symbols is not theoretical: `core/src/ptr/mod.rs:848` —
//! `drop_in_place` — answers with **9 374** of `viewer-sample`'s symbols.

use super::{without_panicking, Dwarf};
use crate::{Object, SymbolData};
use object::SymbolIndex;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// Every source file one object's DWARF names, and per file the `(line, symbol)` pairs its
/// rows landed in — sorted by line and deduplicated, so a line range is two binary searches.
///
/// Symbols are held as [`SymbolIndex`] rather than as `Arc<SymbolData>`: the index is a field
/// of the object whose symbols they are, and keeping strong references from it to them would
/// be an object holding itself up.
#[derive(Default)]
pub(super) struct SourceIndex {
    files: HashMap<Arc<str>, Vec<(u32, SymbolIndex)>>,
}

/// One symbol's extent in the address space the DWARF is read in — biased, so it is directly
/// comparable with the addresses the context answers with.
struct SymbolRange {
    start: u64,
    end: u64,
    /// The furthest `end` of this entry and every entry before it. The ranges are sorted by
    /// `start` and may still overlap (an alias, a split cold part), so a backwards search
    /// needs a bound that is monotone; this is it. `addr2line`'s own unit index is built the
    /// same way and for the same reason.
    max_end: u64,
    symbol: SymbolIndex,
}

impl SourceIndex {
    /// Walk every line program once and invert it. A build that panics is an **empty** index,
    /// which is the same "says nothing" answer everything else in this module gives.
    pub(super) fn build(object: &Object, dwarf: &Dwarf) -> SourceIndex {
        without_panicking(|| SourceIndex::build_inner(object, dwarf)).unwrap_or_default()
    }

    fn build_inner(object: &Object, dwarf: &Dwarf) -> SourceIndex {
        // **Before the context lock, and this is load-bearing.** `SymbolData::extent` reaches
        // `Dwarf::extent_inner`, which locks the context itself, and a `Mutex` is not
        // reentrant — computing an extent inside the row loop below would deadlock the first
        // object anyone asked a source question of.
        let ranges = symbol_ranges(object, dwarf);

        // A poisoned lock means a previous query panicked; the context is only ever read, so
        // recover rather than propagate. `line_info_inner`'s rule.
        let context = dwarf.context.lock().unwrap_or_else(|e| e.into_inner());

        // The whole address space in one pass. Safe where `subprogram_extent` had to decline
        // `u64::MAX`: that unchecked `probe + 1` is in `find_units`, and this goes through
        // `find_units_range`, which takes the bound as given.
        let Ok(rows) = context.find_location_range(0, u64::MAX) else {
            return SourceIndex::default();
        };

        // Keyed by the borrowed name while the context is alive, so a row costs no allocation
        // and a file is turned into an `Arc<str>` once.
        let mut files: HashMap<&str, Vec<(u32, SymbolIndex)>> = HashMap::new();

        for (address, length, location) in rows {
            // A row naming no file or no line points at nothing a reader could ask for.
            // DWARF line 0 is already `None` by the time `addr2line` has spoken.
            let (Some(file), Some(line)) = (location.file, location.line) else {
                continue;
            };
            let Some(end) = address.checked_add(length) else {
                continue;
            };
            if address >= end {
                continue;
            }

            let entry = files.entry(file).or_default();
            for range in intersecting(&ranges, address, end) {
                entry.push((line, range.symbol));
            }
        }

        let files = files
            .into_iter()
            .filter(|(_, entries)| !entries.is_empty())
            .map(|(file, mut entries)| {
                entries.sort_unstable_by_key(|(line, symbol)| (*line, symbol.0));
                entries.dedup();
                (Arc::from(file), entries)
            })
            .collect();

        drop(context);

        SourceIndex { files }
    }

    /// The `(line, symbol)` pairs for one file over `first..=last`, in line order. Inclusive
    /// at the top so that a single line is a range this cannot fail to express, `u32::MAX`
    /// included.
    fn lookup(&self, file: &str, first: u32, last: u32) -> &[(u32, SymbolIndex)] {
        let Some(entries) = self.files.get(file) else {
            return &[];
        };
        let start = entries.partition_point(|(line, _)| *line < first);
        let end = entries.partition_point(|(line, _)| *line <= last);
        &entries[start..end]
    }
}

/// Every symbol of `object` that has bytes, as a range in the address space the DWARF is read
/// in, sorted by start and carrying the running `max_end`.
///
/// The extent is [`SymbolData::extent`] and not the next-symbol estimate, because that is the
/// extent everything else uses: it is what [`SymbolData::line_info`] asks about, so the index
/// and the forward direction cannot disagree about what a symbol covers.
fn symbol_ranges(object: &Object, dwarf: &Dwarf) -> Vec<SymbolRange> {
    let mut ranges: Vec<SymbolRange> = object
        .symbols
        .iter()
        .filter_map(|(&symbol, data)| {
            let section = data.section.as_ref()?;
            let start = data.address.checked_add(dwarf.bias(section.index))?;
            let end = start.checked_add(data.extent(object)?)?;
            (start < end).then_some(SymbolRange {
                start,
                end,
                max_end: end,
                symbol,
            })
        })
        .collect();

    // `symbols` is a `HashMap`, so the order it iterates in is not the file's. Sorting by the
    // symbol index under the address is what keeps the built index a property of the file
    // rather than of a hash seed.
    ranges.sort_unstable_by_key(|range| (range.start, range.symbol.0));

    let mut max_end = 0;
    for range in &mut ranges {
        max_end = max_end.max(range.end);
        range.max_end = max_end;
    }

    ranges
}

/// The symbols whose extent overlaps `[start, end)`. Usually one, occasionally two — a symbol
/// aliasing another, or a `DW_AT_high_pc` reaching over an assembler label.
fn intersecting(
    ranges: &[SymbolRange],
    start: u64,
    end: u64,
) -> impl Iterator<Item = &SymbolRange> {
    // Everything that could overlap begins before the row ends.
    let pos = ranges.partition_point(|range| range.start < end);
    ranges[..pos]
        .iter()
        .rev()
        // Nothing before an entry whose whole prefix ends at or before `start` can overlap
        // either, which is what stops this walking back to the beginning of the object.
        .take_while(move |range| range.max_end > start)
        .filter(move |range| range.end > start)
}

impl Object {
    /// The symbols holding code compiled from `file`, over `lines`.
    ///
    /// `file` is matched exactly against the string DWARF renders, which is what
    /// [`LineInfo::files`](super::LineInfo::files) hands out. Empty for every reason at once:
    /// no DWARF, debug info in a format this does not read, an empty range, a file this object
    /// does not name, or a line no code came from.
    ///
    /// The answer is deduplicated and in address order; **which of several is wanted is the
    /// caller's** — one line compiles into as many symbols as there are instantiations of it,
    /// times as many objects as hold one.
    ///
    /// Worker-thread work by construction: the first call against an object walks every unit's
    /// line program and takes every symbol's extent, and every call afterwards is two binary
    /// searches.
    pub fn symbols_from_source(&self, file: &str, lines: Range<u32>) -> Vec<Arc<SymbolData>> {
        let Some(last) = lines.end.checked_sub(1) else {
            return Vec::new();
        };
        self.symbols_between(file, lines.start, last)
    }

    /// [`symbols_from_source`](Self::symbols_from_source) for one line, which is the common
    /// question and the one spelling of it that stays right at `u32::MAX`.
    pub fn symbols_at_line(&self, file: &str, line: u32) -> Vec<Arc<SymbolData>> {
        self.symbols_between(file, line, line)
    }

    fn symbols_between(&self, file: &str, first: u32, last: u32) -> Vec<Arc<SymbolData>> {
        if first > last {
            return Vec::new();
        }
        let Some(dwarf) = self.dwarf() else {
            return Vec::new();
        };

        let index = dwarf.index.get_or_init(|| SourceIndex::build(self, dwarf));

        // One symbol answering for several of the lines asked about is one hit, not several.
        let mut found: Vec<SymbolIndex> = index
            .lookup(file, first, last)
            .iter()
            .map(|(_, symbol)| *symbol)
            .collect();
        found.sort_unstable_by_key(|symbol| symbol.0);
        found.dedup();

        let mut symbols: Vec<Arc<SymbolData>> = found
            .into_iter()
            .filter_map(|symbol| self.symbols.get(&symbol).cloned())
            .collect();
        // Address order, since that is the order the listing they name is in. The sort is
        // stable and the input was in index order, so a tie is broken by the file's own.
        symbols.sort_by(|a, b| a.address.cmp(&b.address).then_with(|| a.name.cmp(&b.name)));
        symbols
    }
}

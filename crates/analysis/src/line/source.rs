//! Line info read the other way round: a source file and a line in it, to the symbols whose
//! code that line produced.
//!
//! The forward direction ([`Object::line_info`]) is asked about one address range and answers
//! with rows. Nothing there can answer "which functions was this line compiled into", which is
//! a question about the whole object rather than about one symbol — so it is answered from an
//! index, built on the first ask and never at parse time, the way [`super::DebugInfoCache`]
//! builds the debug info itself.
//!
//! Two things are decided here and written down rather than left to be discovered:
//!
//! * **A file is matched exactly, on the string the backend renders.** That is by construction
//!   the string [`LineInfo::files`](super::LineInfo::files) spells, so a caller holding a file
//!   name out of the forward direction can hand it straight back -- and which names there are
//!   at all is a question of its own ([`Object::source_files`]), for a caller that holds an
//!   object and wants one file of it. Nothing here normalises a path or asks the filesystem
//!   about one: a path in debug info is what the producer said, not a place. Two objects whose `DW_AT_comp_dir` disagree therefore do not join, which is
//!   a cross-object question and not this crate's to answer.
//! * **The answer is symbols, not ranges.** Where inside a symbol the line's code sits is the
//!   forward direction's question and is already answered, so a caller wanting the ranges asks
//!   the symbol it was given. One definition of "which rows are this line's" rather than two
//!   that can drift.
//! * **The build has a budget** ([`budget`]). The index is one pair per row per symbol
//!   covering it, and neither number is the app's to choose, so a file naming one address a
//!   hundred thousand times is answered with an empty index rather than with the tens of
//!   gigabytes it asked for.
//!
//! **What it costs, measured** (release, first ask against a fully parsed file). On
//! `viewer-sample` — one object, 115 577 symbols, 267 MB of DWARF — **0.43 s**, down from
//! 2.2 s of which 2.0 s was taking every symbol's extent: its `.eh_frame` now states 115 096 of
//! them and the DIE walk is left the 481 it does not cover, so the 0.23 s line-program walk is
//! most of what remains; the index is 2 096 files and 624 544 `(line, position)` pairs, 5 MB of
//! them, and holding the line programs the walk parsed takes the process from 756 MB to
//! 1.23 GB. On `libanalysis-sample.rlib` — 196 objects, 4 164 symbols — 94 ms for all of them
//! together, 862 files and 25 870 pairs. Every ask after the first is two binary searches:
//! 5 µs, or 750 µs for the worst line in the repo.
//!
//! The extent pass was nine tenths of the build while it was a DIE walk of the whole object,
//! and that walk is still what a symbol no unwind entry covers pays, deliberately: it is what
//! an extent that agrees with
//! [`SymbolData::line_info`] costs. Attributing by [`SymbolData::estimate_size`] instead would
//! be one binary search per row and no DWARF at all, and would let the index name a symbol
//! whose own line info does not name the line back — the one thing a caller walking index →
//! symbol → rows cannot survive.
//!
//! That one line maps into many symbols is not theoretical: `core/src/ptr/mod.rs:848` —
//! `drop_in_place` — answers with **9 374** of `viewer-sample`'s symbols.

use super::intervals::Intervals;
use super::DebugInfo;
use crate::{Object, PlacedAddress, SymbolData};
use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::Arc;

/// Every source file one object's debug info names, and per file the `(line, position)` pairs
/// its rows landed in — a position being a symbol's place in [`Object::placed`] — sorted and
/// deduplicated, so a line range is two binary searches.
///
/// A position and not an `Arc<SymbolData>` because the index is a field of the object whose
/// symbols they are, and strong references from it to them would be an object holding itself
/// up. A position rather than the symbol index it stands for because `placed` is sorted by
/// `(placed address, symbol index)`, so sorting positions *is* the order an answer is wanted
/// in and nothing has to be recovered to answer one. Both are good for the object's life:
/// [`Object::new`] builds `placed` once and nothing rewrites it.
#[derive(Default)]
pub(super) struct SourceIndex {
    files: HashMap<Arc<str>, Vec<(u32, u32)>>,
}

impl SourceIndex {
    /// Walk every line program once and attribute each row to the `ranges` it falls in. No
    /// net of its own: the calls that reach a dependency, the extents and the walk, are each
    /// guarded at the seam.
    ///
    /// Given the ranges rather than the object, so the visitor has no object to ask
    /// ([`DebugInfo::each_row`]).
    fn build(ranges: &Intervals<PlacedAddress, u32>, debug: &DebugInfo) -> SourceIndex {
        // Keyed by the name each row spells, allocated once per distinct file: the visitor is
        // handed a borrow that ends with the call, so the key cannot be the borrow itself.
        let mut files: HashMap<Arc<str>, Vec<(u32, u32)>> = HashMap::new();

        // What the walk has cost against what it is allowed ([`budget`]). Sticky, and no
        // backend's walk can be cut short, so past the budget a row is attributed to nothing
        // and the whole index is dropped below: one missing the rows it skipped would be
        // wrong where an empty one only says nothing.
        let mut rows = 0usize;
        let mut pairs = 0usize;
        let mut over = false;

        // The whole address space in one pass; every row the backend hands over names a file
        // and a line, and covers at least one byte.
        debug.each_row(&mut |range, file, line| {
            rows += 1;
            over |= pairs > budget(rows);
            if over {
                return;
            }

            let entry = match files.get_mut(file) {
                Some(entry) => entry,
                None => files.entry(Arc::from(file)).or_default(),
            };
            // Usually one symbol, occasionally two: a symbol aliasing another, or a
            // `DW_AT_high_pc` reaching over an assembler label.
            for &position in ranges.over(range.start..range.end) {
                pairs += 1;
                entry.push((line, position));
            }
        });

        if over {
            return SourceIndex::default();
        }

        let files = files
            .into_iter()
            .filter(|(_, entries)| !entries.is_empty())
            .map(|(file, mut entries)| {
                entries.sort_unstable();
                entries.dedup();
                (file, entries)
            })
            .collect();

        SourceIndex { files }
    }

    /// The `(line, position)` pairs for one file over `first..=last`, in line order. Inclusive
    /// at the top so that a single line is a range this cannot fail to express, `u32::MAX`
    /// included.
    fn lookup(&self, file: &str, first: u32, last: u32) -> &[(u32, u32)] {
        let Some(entries) = self.files.get(file) else {
            return &[];
        };
        let start = entries.partition_point(|(line, _)| *line < first);
        let end = entries.partition_point(|(line, _)| *line <= last);
        &entries[start..end]
    }
}

/// The most `(line, position)` pairs a build may push: 64 per row walked, never fewer than
/// 64 Ki and never more than 64 Mi.
///
/// Neither factor of the index's size is the app's to choose. A symbol table may name one
/// address any number of times and nothing folds them — [`SymbolData::extent`] answers each
/// alias its own declared size — so a row can be attributed to as many symbols as the file
/// names, and how many rows there are is the line program's to say. 100 000 symbols at one
/// address, with 100 000 rows over them, is a 3 MB file asking for 10^10 pairs — tens of
/// gigabytes, and Rust aborts on an allocation failure, the one failure no `catch_unwind`
/// here sees.
///
/// The rate is what such a file inflates, and the ceiling is what a file with rows enough
/// would get around the rate with. A real file attributes a row to one symbol, occasionally
/// two: the app's own 451 MB debug binary pushes 1 964 064 pairs over 2 112 859 rows, 0.93
/// each against the 64 allowed, and the floor leaves a small object room for aliases the rate
/// alone would not.
fn budget(rows: usize) -> usize {
    const PER_ROW: usize = 64;
    const FLOOR: usize = 64 << 10;
    const CEILING: usize = 64 << 20;

    FLOOR
        .saturating_add(rows.saturating_mul(PER_ROW))
        .min(CEILING)
}

/// Every symbol of `object` that has bytes, as a range in the address space the debug info is
/// read in, to its position in [`Object::placed`]. The position and not the symbol index is
/// what the index keeps, which is what makes an answer's order a sort of positions.
///
/// The extent is [`SymbolData::extent`] and not the next-symbol estimate, because that is the
/// extent everything else uses: it is what [`SymbolData::line_info`] asks about, so the index
/// and the forward direction cannot disagree about what a symbol covers.
///
/// The ranges are biased, and the rows come back in the same space: the DWARF backend is read
/// at [`Section::bias`](crate::Section::bias), and a `.pdb` describes a linked image, where
/// every bias is 0.
fn symbol_ranges(object: &Object) -> Intervals<PlacedAddress, u32> {
    let ranges = object
        .placed_symbols()
        .iter()
        .enumerate()
        .filter_map(|(position, entry)| {
            // A file naming more than `u32::MAX` placed symbols loses the ones past that,
            // which is a smaller thing than either a wider index or a panic.
            let position = u32::try_from(position).ok()?;
            let start = entry.placed;
            let end = start.checked_add(entry.symbol.extent(object)?.bytes)?;
            Some((start..end, position))
        });
    Intervals::new(ranges)
}

impl Object {
    /// The symbols holding code compiled from `file`, over the **inclusive** range `lines`.
    ///
    /// `file` is matched exactly against the string the debug info renders, which is what
    /// [`LineInfo::files`](super::LineInfo::files) hands out. Empty for every reason at once:
    /// no debug info, debug info in a format this does not read, a range running backwards, a
    /// file this object does not name, or a line no code came from.
    ///
    /// The answer is deduplicated and in [`Object::placed`]'s order: by placed address
    /// ([`SymbolData::placed`]), then by symbol index. That is the order the listing of the
    /// object's code draws them in, so the first is the one it draws first. **Which of several
    /// is wanted is the caller's** — one line compiles into as many symbols as there are
    /// instantiations of it, times as many objects as hold one.
    ///
    /// Inclusive because that is the shape the index answers in ([`SourceIndex::lookup`]) and
    /// the one a caller holding a function's first and last line has, `u32::MAX` included.
    /// [`symbols_at_line`](Self::symbols_at_line) asks about one line.
    ///
    /// Worker-thread work by construction: the first call against an object walks every unit's
    /// line program and takes every symbol's extent, and every call afterwards is two binary
    /// searches.
    pub fn symbols_from_lines(
        &self,
        file: &str,
        lines: RangeInclusive<u32>,
    ) -> Vec<Arc<SymbolData>> {
        let (first, last) = (*lines.start(), *lines.end());
        if first > last {
            return Vec::new();
        }
        let Some(index) = self.source_index() else {
            return Vec::new();
        };

        // One symbol answering for several of the lines asked about is one hit, not several.
        // Sorting the positions is all it takes to put the answer in `Object::placed`'s
        // order, which is the order the listing draws them in: that list is sorted by
        // `(placed address, symbol index)` already. Not by `address`: that is the section's
        // own, and in a relocatable object every `.text.<name>` starts at 0.
        let mut found: Vec<u32> = index
            .lookup(file, first, last)
            .iter()
            .map(|&(_, position)| position)
            .collect();
        found.sort_unstable();
        found.dedup();

        // Every position came out of that list, so each names an entry.
        let placed = self.placed_symbols();
        found
            .into_iter()
            .filter_map(|position| placed.get(position as usize))
            .map(|entry| entry.symbol.clone())
            .collect()
    }

    /// [`symbols_from_lines`](Self::symbols_from_lines) for one line.
    pub fn symbols_at_line(&self, file: &str, line: u32) -> Vec<Arc<SymbolData>> {
        self.symbols_from_lines(file, line..=line)
    }

    /// Every line of `file` this object has code compiled from, ascending and without
    /// repeats.
    ///
    /// The whole file in one answer, where [`symbols_from_lines`](Self::symbols_from_lines)
    /// is a range and names what it found: the Source pane marks its gutter from this and
    /// wants the set once per file rather than a question per row. It says which lines
    /// produced code and not what they produced, so an unmarked line is one no open object
    /// compiled anything from.
    ///
    /// Empty for [`symbols_from_lines`]'s reasons, and worker-thread work for its reason
    /// too: the first call against an object builds the index.
    pub fn lines_from_source(&self, file: &str) -> Vec<u32> {
        let Some(index) = self.source_index() else {
            return Vec::new();
        };

        // The entries are sorted by line, so the repeats a line with several symbols
        // makes are adjacent and `dedup` is the whole of it.
        let mut lines: Vec<u32> = index
            .lookup(file, 0, u32::MAX)
            .iter()
            .map(|(line, _)| *line)
            .collect();
        lines.dedup();
        lines
    }

    /// Every source file this object has code compiled from, in name order and without
    /// repeats.
    ///
    /// The keys of the index the questions above are asked of, so a name out of here can be
    /// handed straight back to either. It is the files that produced **code** and not every
    /// file the line program names: an entry no row landed in is never added.
    ///
    /// Sorted, because the index is a `HashMap` and the order it iterates in is a hash seed's
    /// rather than the file's -- an answer that changed between runs of one binary is not one
    /// a caller can pick from.
    ///
    /// Empty for [`symbols_from_lines`](Self::symbols_from_lines)'s reasons, and worker-thread
    /// work for its reason too: the first call against an object builds the index.
    pub fn source_files(&self) -> Vec<Arc<str>> {
        let Some(index) = self.source_index() else {
            return Vec::new();
        };

        let mut files: Vec<Arc<str>> = index.files.keys().cloned().collect();
        files.sort_unstable();
        files
    }

    /// The index the questions above are asked of, built on the first call against this
    /// object, or [`None`] when it has no debug info this reads.
    fn source_index(&self) -> Option<&SourceIndex> {
        let debug = self.debug_info()?;
        Some(
            debug
                .index
                .get_or_init(|| SourceIndex::build(&symbol_ranges(self), debug)),
        )
    }
}

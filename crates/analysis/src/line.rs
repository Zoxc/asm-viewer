//! Line-number information, read lazily out of what an [`Object`] was parsed from. Nothing
//! here runs at parse time; the first query builds the backend, and an object with no debug
//! info caches that answer too.
//!
//! This file is the **seam**: what every backend answers and the rules every answer obeys,
//! naming no debug format. The two questions — the rows covering an address range, and a
//! function's declared extent — are asked of a [`DebugInfo`], which dispatches by `match` to
//! the one backend the object has: [`dwarf`], which is the only module that knows `gimli` and
//! `addr2line`. A row out of any backend goes through one [`RowCollector`], so the invariants
//! [`LineInfo`] promises hold whoever produced them.
//!
//! This is the forward direction — an address range in, source rows out. The reverse — a file
//! and a line, out to the symbols compiled from them — is [`source`], a file of its own
//! because it is a whole-object index rather than a query, built on the same seam.

use crate::{Object, Section, SymbolData};
use object::SectionIndex;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

mod dwarf;
mod source;

use source::SourceIndex;

/// An [`Object`]'s debug info, or the fact that it has none, worked out at most once. Caching
/// the *absence* is what keeps a stripped binary from re-scanning its section table per query.
#[derive(Default)]
pub struct DebugInfoCache(OnceLock<Option<DebugInfo>>);

/// One object's debug info, whichever format it is in, built once and kept for the object's
/// lifetime.
pub(crate) struct DebugInfo {
    backend: Backend,

    /// The line info inverted — file and line to the symbols compiled from it — built whole
    /// on the first source question and never before one. Here and not in a backend, because
    /// it is built from what every backend answers ([`DebugInfo::each_row`]) and not from any
    /// one's internals. A `OnceLock` and not a `Mutex` like the backends' own caches, because
    /// unlike them it is not filled in a unit at a time: see [`source`].
    index: OnceLock<SourceIndex>,
}

/// The formats read. A closed set dispatched by `match`, monomorphised, nothing boxed.
enum Backend {
    Dwarf(dwarf::Dwarf),
}

impl DebugInfo {
    /// Build the debug info for one object, or [`None`] when it has none this reads. Never an
    /// error: foreign debug info and corrupt debug info are both simply "no line info".
    pub(crate) fn load(object: &Object) -> Option<DebugInfo> {
        without_panicking(|| DebugInfo::load_inner(object)).flatten()
    }

    fn load_inner(object: &Object) -> Option<DebugInfo> {
        let file = object::File::parse(object.data.bytes()).ok()?;
        let backend = dwarf::Dwarf::load(&file, &object.sections).map(Backend::Dwarf)?;
        Some(DebugInfo {
            backend,
            index: OnceLock::new(),
        })
    }

    /// How far the section with this index was moved by [`crate::section_biases`]; 0 for a
    /// section that was not moved, and for every section of a linked image.
    fn bias(&self, section: SectionIndex) -> u64 {
        match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.bias(section),
        }
    }

    /// The rows covering `range` **within one section**, resolved in one pass.
    fn line_info(&self, section: &Section, range: Range<u64>) -> Option<Arc<LineInfo>> {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.line_info(dwarf.bias(section.index), range),
        })
        .flatten()
        .map(Arc::new)
    }

    /// The declared extent of the function beginning at `address` **within one section**, or
    /// [`None`] when the debug info does not say.
    fn extent(&self, section: &Section, address: u64) -> Option<u64> {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.extent(dwarf.bias(section.index), address),
        })
        .flatten()
    }

    /// Every row that names a file and a line, whatever the object, handed to `visit` as
    /// `(range, file, line)` in the **biased** address space ([`Section::bias`] already
    /// applied). The backend's own lock is held for the whole walk, so `visit` must not ask
    /// this object anything — see [`source`] for the one caller and the order it keeps.
    fn each_row(&self, visit: &mut dyn FnMut(Range<u64>, &str, u32)) {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.each_row(visit),
        });
    }
}

/// Run a backend with a net under it, turning a panic into "no line info".
///
/// Not general defensiveness: known, reachable bugs in the dependencies behind the seam, all
/// unchecked arithmetic on numbers a debug section states and none of them something this
/// crate can validate without parsing the debug info twice. In `addr2line` 0.21, a line-table
/// row's length is `next.address - row.address`, and nothing stops a line program from moving
/// its address backwards; and a range is `low_pc + high_pc` wherever `high_pc` is a length,
/// which overflows for a length running off the end of the address space — that one while the
/// context is being *built*, which is why the guard is around [`DebugInfo::load`] too.
///
/// Sound because a panic leaves nothing half-written: a backend is only ever read, and the
/// lock a panic poisons is recovered explicitly.
fn without_panicking<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Rows as a backend hands them over, and the one path from there to a [`LineInfo`]: files
/// deduplicated in first-seen order, and [`finish`](Self::finish) making the rows ascending,
/// non-overlapping and coalesced. Every backend feeds this, so the invariants are made in one
/// place rather than promised by each.
#[derive(Default)]
pub(super) struct RowCollector {
    rows: Vec<LineRow>,
    files: Vec<Arc<str>>,
    indices: HashMap<Arc<str>, usize>,
}

impl RowCollector {
    /// The index a file name will have in [`LineInfo::files`], interning it on first sight.
    pub(super) fn file(&mut self, name: &str) -> usize {
        match self.indices.get(name) {
            Some(index) => *index,
            None => {
                let name: Arc<str> = Arc::from(name);
                self.files.push(name.clone());
                self.indices.insert(name, self.files.len() - 1);
                self.files.len() - 1
            }
        }
    }

    /// One row, in the address space the caller's answer is in. A row covering nothing is
    /// dropped here, so no backend has to check.
    pub(super) fn push(
        &mut self,
        range: Range<u64>,
        file: Option<usize>,
        line: Option<u32>,
        column: Option<u32>,
    ) {
        if range.start >= range.end {
            return;
        }
        self.rows.push(LineRow {
            range,
            file,
            line,
            column,
        });
    }

    /// The rows made to hold [`LineInfo`]'s invariants, or [`None`] when there are none:
    /// "there is debug info but it says nothing about this range" and "there is no debug
    /// info" are the same answer to a caller.
    pub(super) fn finish(self) -> Option<LineInfo> {
        let RowCollector {
            mut rows, files, ..
        } = self;

        // Units are visited in range order and rows within a unit ascend, but two units may
        // cover overlapping addresses, so sort rather than assume.
        rows.sort_by_key(|row| (row.range.start, row.range.end));

        // Then clip so the rows genuinely do not overlap, which [`LineInfo::row_at`] needs to
        // binary-search them: it looks for the last row starting at or before an address, and
        // a row nested inside a longer one makes that answer arbitrary. The row that starts
        // first keeps the addresses it covers, and one left with nothing goes.
        let mut covered = 0;
        rows.retain_mut(|row| {
            row.range.start = row.range.start.max(covered);
            if row.range.start >= row.range.end {
                return false;
            }
            covered = row.range.end;
            true
        });

        // Coalesce runs that say the same thing: a line program emits a row per
        // is_stmt/discriminator change as well as per source position.
        rows.dedup_by(|next, row| {
            let same = row.range.end == next.range.start
                && row.file == next.file
                && row.line == next.line
                && row.column == next.column;
            if same {
                row.range.end = next.range.end;
            }
            same
        });

        (!rows.is_empty()).then(|| LineInfo { rows, files })
    }
}

/// One run of instructions and the source position the debug info gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRow {
    /// The instruction addresses this row covers, clipped to the range that was asked about
    /// and in the same address space as [`SymbolData::address`].
    pub range: Range<u64>,
    /// An index into [`LineInfo::files`], or [`None`] when the row names no file.
    pub file: Option<usize>,
    /// The line number. Genuinely optional: DWARF's line 0 means "these instructions belong
    /// to no source line", which is neither line 0 nor line 1.
    pub line: Option<u32>,
    /// The column number, [`None`] both when the producer emitted no column at all and when
    /// it emitted 0, the "left edge of the line" marker.
    pub column: Option<u32>,
}

/// A source position, for callers that want one address answered rather than a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// The line info covering one address range, resolved in a single pass, so a caller holding a
/// symbol's instructions asks once and then answers each of them locally with
/// [`row_at`](Self::row_at).
///
/// The rows are ascending, non-overlapping and coalesced, but *not* contiguous —
/// compiler-generated instructions belonging to no source line leave gaps, and
/// [`row_at`](Self::row_at) returns [`None`] there rather than inventing a position.
/// Non-overlapping is an invariant of this type, established by scoping the query to a
/// section ([`crate::section_biases`]) and by the clipping in [`RowCollector::finish`]; where
/// two rows genuinely covered one address, the one that starts first keeps it.
pub struct LineInfo {
    rows: Vec<LineRow>,
    files: Vec<Arc<str>>,
}

impl LineInfo {
    /// Every row, ascending by address and non-overlapping.
    pub fn rows(&self) -> &[LineRow] {
        &self.rows
    }

    /// The source files these rows touch, deduplicated, in the order they were first seen.
    /// [`LineRow::file`] indexes into this.
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    /// The row covering `address`, or [`None`] when no row does. The last row starting at or
    /// before `address` is the only candidate *because* the rows do not overlap.
    pub fn row_at(&self, address: u64) -> Option<&LineRow> {
        let index = match self
            .rows
            .binary_search_by_key(&address, |row| row.range.start)
        {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let row = &self.rows[index];
        (address < row.range.end).then_some(row)
    }

    /// The file a row names.
    pub fn file_of(&self, row: &LineRow) -> Option<&str> {
        Some(&*self.files[row.file?])
    }

    /// `(file, line, column)` for a single instruction address.
    pub fn location(&self, address: u64) -> Option<Location<'_>> {
        let row = self.row_at(address)?;
        Some(Location {
            file: self.file_of(row),
            line: row.line,
            column: row.column,
        })
    }
}

impl Object {
    /// The line info for an address range **within one section**, building this object's
    /// debug info on the first call and reusing it afterwards.
    ///
    /// The section is not decoration: in a relocatable object every section starts at 0, so
    /// `range` on its own does not say which code it means. See [`crate::section_biases`].
    ///
    /// [`None`] means "no line info" for every reason at once: no debug info, debug info in a
    /// format this does not read (CodeView), debug info that will not parse, or debug info
    /// that says nothing about this range.
    ///
    /// Worker-thread work by construction: the first call parses the debug info's tables, and
    /// each call parses the line program of every unit covering the range, once per unit for
    /// the object's lifetime.
    pub fn line_info(&self, section: &Section, range: Range<u64>) -> Option<Arc<LineInfo>> {
        self.debug_info()?.line_info(section, range)
    }

    /// How many bytes of code the debug info says the function starting at `address` **within
    /// one section** is, or [`None`] when it does not say. Cached per unit visited; see
    /// [`SymbolData::extent`] for how it and the next-symbol estimate bound each other.
    pub fn function_extent(&self, section: &Section, address: u64) -> Option<u64> {
        self.debug_info()?.extent(section, address)
    }

    /// This object's debug info, built at most once — including the "there is none" answer.
    fn debug_info(&self) -> Option<&DebugInfo> {
        self.debug_info
            .0
            .get_or_init(|| DebugInfo::load(self))
            .as_ref()
    }
}

impl SymbolData {
    /// The line info for this symbol's instructions, over the same extent
    /// [`assembly`](Self::assembly) decodes.
    pub fn line_info(&self, object: &Object) -> Option<Arc<LineInfo>> {
        let section = self.section.as_ref()?;
        let end = self.address.checked_add(self.extent(object)?)?;
        object.line_info(section, self.address..end)
    }

    /// What the debug info says this symbol's extent is, [`None`] when it says nothing.
    pub fn debug_extent(&self, object: &Object) -> Option<u64> {
        object.function_extent(self.section.as_ref()?, self.address)
    }
}

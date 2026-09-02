//! Line-number information, read lazily out of what an [`Object`] was parsed from. The first
//! query builds the backend, and an object with no debug info caches that answer too. The
//! one exception is a PE whose `.pdb` is found and matches: [`DebugInfo::pdb`] opens it at
//! parse time, because the procedures and publics it names are symbols the image itself does
//! not declare, and the backend built there is seeded into the object's cache
//! ([`DebugInfoCache::preloaded`]) so nothing is opened twice — the line tables themselves
//! are still decoded on the first question about them.
//!
//! This file is the **seam**: what every backend answers and the rules every answer obeys,
//! naming no debug format. The two questions — the rows covering an address range, and a
//! function's declared extent — are asked of a [`DebugInfo`], which dispatches by `match` to
//! the one backend the object has: [`dwarf`] for debug sections in the object itself, the
//! only module that knows `gimli` and `addr2line`; [`pdb`] for a PE whose debug directory
//! names a `.pdb` beside it, the only module that knows `pdb2`. A row out of any backend goes
//! through one [`RowCollector`], so the invariants [`LineInfo`] promises hold whoever
//! produced them.
//!
//! This is the forward direction — an address range in, source rows out. The reverse — a file
//! and a line, out to the symbols compiled from them — is [`source`], a file of its own
//! because it is a whole-object index rather than a query, built on the same seam.

use crate::{Object, Section, SymbolData};
use object::SectionIndex;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

mod dwarf;
mod pdb;
mod source;

pub(crate) use pdb::{Procedure, Public};
use source::SourceIndex;

/// An [`Object`]'s debug info, or the fact that it has none, worked out at most once. Caching
/// the *absence* is what keeps a stripped binary from re-scanning its section table per query.
#[derive(Default)]
pub struct DebugInfoCache(OnceLock<Option<DebugInfo>>);

impl DebugInfoCache {
    /// A cache already holding the backend the parse built — [`DebugInfo::pdb`]'s — so the
    /// first line question finds it there instead of opening the `.pdb` a second time.
    pub(crate) fn preloaded(info: DebugInfo) -> DebugInfoCache {
        DebugInfoCache(OnceLock::from(Some(info)))
    }
}

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
    Pdb(pdb::Pdb),
}

impl DebugInfo {
    /// Build the debug info for one object, or [`None`] when it has none this reads. Never an
    /// error: foreign debug info and corrupt debug info are both simply "no line info".
    pub(crate) fn load(object: &Object) -> Option<DebugInfo> {
        without_panicking(|| DebugInfo::load_inner(object)).flatten()
    }

    fn load_inner(object: &Object) -> Option<DebugInfo> {
        let file = object::File::parse(object.data.bytes()).ok()?;
        // Debug sections in the object itself first — a MinGW or clang PE can carry DWARF
        // — and a `.pdb` beside it only for an object that has none.
        let backend = match dwarf::Dwarf::load(&file, &object.sections) {
            Some(dwarf) => Backend::Dwarf(dwarf),
            None => Backend::Pdb(pdb::Pdb::load(&file, &object.path)?),
        };
        Some(DebugInfo::of(backend))
    }

    fn of(backend: Backend) -> DebugInfo {
        DebugInfo {
            backend,
            index: OnceLock::new(),
        }
    }

    /// The PDB backend built **eagerly**, for `parse_object`: the `.pdb` a PE at `path` names
    /// — found and matched as [`load`](Self::load) would find it — together with every
    /// procedure it records and every public it names for code, which the parse takes as
    /// symbols in that order. [`None`] for anything that
    /// is not a PE with a matching `.pdb`, and for a PE carrying DWARF of its own, which
    /// `load` would answer from that and not from the PDB: the two paths pick the same
    /// backend, and a parse never builds a DWARF context. Under the same net as `load`, the
    /// walk included, so a `pdb2` panic in either is "no PDB" and the lazy path is left to
    /// try again.
    pub(crate) fn pdb(
        file: &object::File<'_>,
        path: &Path,
    ) -> Option<(DebugInfo, Vec<Procedure>, Vec<Public>)> {
        without_panicking(|| {
            if dwarf::Dwarf::present(file) {
                return None;
            }
            let pdb = pdb::Pdb::load(file, path)?;
            let procedures = pdb.procedures();
            let publics = pdb.publics();
            Some((DebugInfo::of(Backend::Pdb(pdb)), procedures, publics))
        })
        .flatten()
    }

    /// How far the section with this index was moved by [`crate::section_biases`]; 0 for a
    /// section that was not moved, and for every section of a linked image — which is the
    /// only kind of object a `.pdb` describes.
    fn bias(&self, section: SectionIndex) -> u64 {
        match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.bias(section),
            Backend::Pdb(_) => 0,
        }
    }

    /// The rows covering `range` **within one section**, resolved in one pass.
    fn line_info(&self, section: &Section, range: Range<u64>) -> Option<Arc<LineInfo>> {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.line_info(dwarf.bias(section.index), range),
            Backend::Pdb(pdb) => pdb.line_info(range),
        })
        .flatten()
        .map(Arc::new)
    }

    /// The declared extent of the function beginning at `address` **within one section**, or
    /// [`None`] when the debug info does not say.
    fn extent(&self, section: &Section, address: u64) -> Option<u64> {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.extent(dwarf.bias(section.index), address),
            Backend::Pdb(pdb) => pdb.extent(address),
        })
        .flatten()
    }

    /// Every row that names a file and a line, whatever the object, handed to `visit` as
    /// `(range, file, line)` in the **biased** address space ([`Section::bias`] already
    /// applied). A backend may hold its own lock for the whole walk, so `visit` must not ask
    /// this object anything — see [`source`] for the one caller and the order it keeps.
    fn each_row(&self, visit: &mut dyn FnMut(Range<u64>, &str, u32)) {
        without_panicking(|| match &self.backend {
            Backend::Dwarf(dwarf) => dwarf.each_row(visit),
            Backend::Pdb(pdb) => pdb.each_row(visit),
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
/// context is being *built*, which is why the guard is around [`DebugInfo::load`] too. In
/// `pdb2` 0.10, a module's line data is sliced out of its stream at `start..start + size`
/// unchecked, a line block's size has its header subtracted unchecked, and a section offset
/// plus a length is a plain `+` (`notes/upstream/pdb2.md`).
///
/// Sound because a panic leaves nothing half-written: a backend is only ever read, and the
/// lock a panic poisons is recovered explicitly.
fn without_panicking<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// A checksum the debug info records for a source file, so a reader can tell the file they
/// have from the one the compiler read. Which algorithm is the producer's choice — clang-cl
/// and rustc write MD5, MSVC since 2022 SHA-256 — so a hash carries its own kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceHash {
    Md5([u8; 16]),
    Sha1([u8; 20]),
    Sha256([u8; 32]),
}

/// All three digests of one file's bytes, computed together, so a file read once answers a
/// [`SourceHash`] of any kind. The bytes hashed are the file's as read, not a decoding of
/// them: the compiler hashed the bytes too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceDigests {
    md5: [u8; 16],
    sha1: [u8; 20],
    sha256: [u8; 32],
}

impl SourceDigests {
    pub fn of(bytes: &[u8]) -> SourceDigests {
        use md5::Digest as _;
        SourceDigests {
            md5: md5::Md5::digest(bytes).into(),
            sha1: sha1::Sha1::digest(bytes).into(),
            sha256: sha2::Sha256::digest(bytes).into(),
        }
    }
}

impl SourceHash {
    /// Whether the bytes these digests were taken of are the bytes this hash was recorded
    /// for.
    pub fn matches(&self, digests: &SourceDigests) -> bool {
        match self {
            SourceHash::Md5(hash) => *hash == digests.md5,
            SourceHash::Sha1(hash) => *hash == digests.sha1,
            SourceHash::Sha256(hash) => *hash == digests.sha256,
        }
    }
}

/// Rows as a backend hands them over, and the one path from there to a [`LineInfo`]: files
/// deduplicated in first-seen order, and [`finish`](Self::finish) making the rows ascending,
/// non-overlapping and coalesced. Every backend feeds this, so the invariants are made in one
/// place rather than promised by each.
#[derive(Default)]
pub(super) struct RowCollector {
    rows: Vec<LineRow>,
    files: Vec<Arc<str>>,
    hashes: Vec<Option<SourceHash>>,
    indices: HashMap<Arc<str>, usize>,
}

impl RowCollector {
    /// The index a file name will have in [`LineInfo::files`], interning it on first sight
    /// along with the hash recorded for it — the first hash seen for a name is the one kept.
    pub(super) fn file(&mut self, name: &str, hash: Option<SourceHash>) -> usize {
        match self.indices.get(name) {
            Some(index) => *index,
            None => {
                let name: Arc<str> = Arc::from(name);
                self.files.push(name.clone());
                self.hashes.push(hash);
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
            mut rows,
            files,
            hashes,
            ..
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

        (!rows.is_empty()).then(|| LineInfo {
            rows,
            files,
            hashes,
        })
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
    /// Parallel to `files`: the checksum the debug info recorded for each, where it did.
    hashes: Vec<Option<SourceHash>>,
}

impl LineInfo {
    /// Line info from rows and files handed over directly, made to hold the invariants
    /// below the way a backend's rows are, or [`None`] when no row covers anything. Each
    /// row's `file` indexes `files` as given. For code that has line info to stand in for
    /// what a backend would have said — a test of the app's panes, say — and nothing else.
    pub fn new(rows: Vec<LineRow>, files: Vec<(Arc<str>, Option<SourceHash>)>) -> Option<LineInfo> {
        let mut collector = RowCollector::default();
        let indices: Vec<usize> = files
            .iter()
            .map(|(name, hash)| collector.file(name, *hash))
            .collect();
        for row in rows {
            let file = row.file.and_then(|file| indices.get(file).copied());
            collector.push(row.range, file, row.line, row.column);
        }
        collector.finish()
    }

    /// Every row, ascending by address and non-overlapping.
    pub fn rows(&self) -> &[LineRow] {
        &self.rows
    }

    /// The source files these rows touch, deduplicated, in the order they were first seen.
    /// [`LineRow::file`] indexes into this.
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    /// The checksum the debug info recorded for the file at this index of
    /// [`files`](Self::files), or [`None`] where it recorded none (DWARF, as read here) or
    /// the index is not a file's.
    pub fn hash_of(&self, file: usize) -> Option<SourceHash> {
        self.hashes.get(file).copied().flatten()
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
    /// format this does not read (CodeView embedded in a COFF object), a `.pdb` that is
    /// missing or not this image's, debug info that will not parse, or debug info that says
    /// nothing about this range.
    ///
    /// Worker-thread work by construction: the first call parses the debug info's tables —
    /// unless the parse already opened the `.pdb` beside a PE for its procedures — and each
    /// call parses the line program of every unit or module covering the range, once per
    /// unit for the object's lifetime.
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

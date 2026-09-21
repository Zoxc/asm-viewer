//! Line-number information, read lazily out of what an [`Object`] was parsed from. The first
//! query builds the backend, and an object with no debug info caches that answer too. The
//! one exception is a debug file that names functions the image itself does not:
//! [`DebugInfo::declared`] builds the backend at parse time for those names, and it is
//! handed to [`Object::preloaded`] to seed its cache so nothing is opened
//! twice — the line tables themselves are still decoded on the first question about them.
//!
//! This file is the **seam**: what every backend answers and the rules every answer obeys,
//! naming no debug format. The two questions — the rows covering an address range, and a
//! function's declared extent — are asked of a [`DebugInfo`], which puts them to the one
//! backend the object has ([`LineBackend`]): [`dwarf`] for debug sections in the object
//! itself, the only module that knows DWARF's debug sections and `addr2line` (`gimli`'s
//! call-frame reader is `unwind.rs`'s); [`pdb`] for a PE whose debug directory names a
//! `.pdb` beside it, the only module that knows `pdb2`. A row out of any backend goes
//! through one [`RowCollector`], which clips it to what was asked about and takes the
//! section's bias off it, so the invariants [`LineInfo`] promises hold whoever produced
//! them. The third answer — the functions a backend names that the image does not
//! — is a [`Declared`] record, with no format in it either: the PDB is the only backend with
//! any today, and the parse takes them from the seam rather than from a backend.
//!
//! This is the forward direction — an address range in, source rows out. The reverse — a file
//! and a line, out to the symbols compiled from them — is [`source`], a file of its own
//! because it is a whole-object index rather than a query, built on the same seam.

use crate::model::covering;
use crate::parse::Name;
use crate::{Bias, Object, PlacedAddress, Section, SectionAddress, SymbolData};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

mod dwarf;
mod pdb;
mod source;

use source::SourceIndex;

/// A function a debug file names that the image itself does not.
pub(crate) struct Declared {
    pub(crate) name: Name,
    pub(crate) address: SectionAddress,
    /// The stated length, or 0 where the record has none.
    pub(crate) len: u64,
}

/// An [`Object`]'s debug info, or the fact that it has none, worked out at most once. Caching
/// the *absence* is what keeps a stripped binary from re-scanning its section table per query.
pub(crate) struct DebugInfoCache(OnceLock<Option<DebugInfo>>);

impl DebugInfoCache {
    /// A cache holding `preloaded`, the backend the parse built ([`DebugInfo::declared`]'s),
    /// so the first line question finds it there and does not open the debug file again.
    /// [`None`] means nothing was loaded yet: the first question loads it, which is not the
    /// same as the cached [`None`] of an object found to have no debug info.
    pub(crate) fn new(preloaded: Option<DebugInfo>) -> DebugInfoCache {
        DebugInfoCache(match preloaded {
            Some(info) => OnceLock::from(Some(info)),
            None => OnceLock::new(),
        })
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

/// The formats read: a closed set, so adding one is a variant here, an impl of
/// [`LineBackend`] and an arm of [`Backend::pick`]. An enum and not a boxed trait object
/// because the set is closed and because what crosses threads is asserted on the concrete
/// types (`lib.rs`).
enum Backend {
    Dwarf(dwarf::Dwarf),
    Pdb(pdb::Pdb),
}

/// The three questions a backend answers, and the one space it answers them in.
///
/// **Placed addresses throughout**, and that is not a concession to one format. DWARF states
/// a flat address — zero plus a relocation in a relocatable object, the link-time virtual
/// address in a linked image — and the DWARF backend relocates its private copy of the debug
/// sections with the parse's own layout, so its context reads in the space every section of
/// the object shares. CodeView states a `section:offset` pair and maps it to an RVA and then
/// onto the image base, which is that same space for an image nothing placed. So the bias is
/// not a backend's business at all: it is the seam's conversion between the space
/// [`LineInfo`] is keyed in — a section's own — and the space both backends already read in,
/// and no backend has one.
///
/// Dispatched dynamically, where `Assembly::decode` matches on the architecture and compiles
/// a backend in: there a backend's call sits in a per-instruction loop and the inlining is
/// the point, here every call is one per question and takes a backend's lock on its first
/// line, so a virtual call is noise.
trait LineBackend {
    /// The rows over `query`, pushed into `rows` as the backend reads them. The collector is
    /// the seam's and holds the query: it clips each row and takes the section's bias off
    /// ([`RowCollector::push`]), so a backend pushes what the debug info said and nothing
    /// else.
    fn line_info(&self, query: Range<PlacedAddress>, rows: &mut RowCollector);

    /// The declared extent of the function beginning at `address`, or [`None`] when the
    /// debug info does not say.
    fn extent(&self, address: PlacedAddress) -> Option<u64>;

    /// Every row that names a file and a line, handed to `visit` as `(range, file, line)`.
    /// A backend may hold its own lock for the whole walk; see [`DebugInfo::each_row`].
    fn each_row(&self, visit: &mut dyn FnMut(Range<PlacedAddress>, &str, u32));
}

impl Backend {
    /// The backend the seam's rule picks, for both paths that pick one: debug sections in
    /// the object itself where there are any — a MinGW or clang PE can carry DWARF — and the
    /// `.pdb` beside it only for an object with none. `dwarf` builds the DWARF backend, or
    /// answers [`None`] to decline one — what the parse passes, so that a parse never builds
    /// a context.
    fn pick(
        file: &object::File<'_>,
        path: &Path,
        dwarf: impl FnOnce() -> Option<dwarf::Dwarf>,
    ) -> Option<Backend> {
        if dwarf::Dwarf::present(file) {
            return dwarf().map(Backend::Dwarf);
        }
        Some(Backend::Pdb(pdb::Pdb::load(file, path)?))
    }
}

impl DebugInfo {
    /// Build the debug info for one object, or [`None`] when it has none this reads. Never an
    /// error: foreign debug info and corrupt debug info are both simply "no line info".
    pub(crate) fn load(object: &Object) -> Option<DebugInfo> {
        without_panicking(|| DebugInfo::load_inner(object)).flatten()
    }

    fn load_inner(object: &Object) -> Option<DebugInfo> {
        let file = object::File::parse(object.data.bytes()).ok()?;
        let backend = Backend::pick(&file, &object.path, || dwarf::Dwarf::load(&file))?;
        Some(DebugInfo::of(backend))
    }

    fn of(backend: Backend) -> DebugInfo {
        DebugInfo {
            backend,
            index: OnceLock::new(),
        }
    }

    /// The backend built **eagerly**, for `parse_object`, with every function it names that
    /// the image itself does not — today a PE's matching `.pdb` and nothing else. [`None`]
    /// where no backend has any such names, which is every other file.
    ///
    /// **The order of the records is their precedence**: the parse takes them in it and
    /// gives an address to the first that claims it, so a backend appends its own in the
    /// order it wants them believed.
    ///
    /// The backend is [`Backend::pick`]'s, handed no way to build a DWARF context, so the
    /// parse picks what [`load`](Self::load) will pick without paying for one. Under the
    /// same net as `load`, the walk included, so a panic in either is "no debug info here"
    /// and the lazy path is left to try again.
    pub(crate) fn declared(
        file: &object::File<'_>,
        path: &Path,
    ) -> Option<(DebugInfo, Vec<Declared>)> {
        without_panicking(|| {
            let backend = Backend::pick(file, path, || None)?;
            // Unreachable while the PDB is the only backend that names anything, `pick`
            // having been given no way to build the other one.
            let Backend::Pdb(pdb) = &backend else {
                return None;
            };
            let declared = pdb.declared();
            Some((DebugInfo::of(backend), declared))
        })
        .flatten()
    }

    /// The rows covering `range` **within one section**, resolved in one pass.
    ///
    /// The section's bias is applied here and taken off here: the query goes up into the
    /// placed space every backend reads in, and the collector brings each row back down
    /// once it has clipped it ([`RowCollector::over`]).
    fn line_info(&self, section: &Section, range: Range<SectionAddress>) -> Option<Arc<LineInfo>> {
        // Saturating rather than wrapping, so an absurd range asks about less than it meant
        // to instead of about something else.
        let query = section.place_saturating(range.start)..section.place_saturating(range.end);
        without_panicking(|| {
            let mut rows = RowCollector::over(query.clone(), section.bias());
            self.backend().line_info(query.clone(), &mut rows);
            rows.finish()
        })
        .flatten()
        .map(Arc::new)
    }

    /// The declared extent of the function beginning at `address` **within one section**, or
    /// [`None`] when the debug info does not say.
    fn extent(&self, section: &Section, address: SectionAddress) -> Option<u64> {
        let probe = section.place_checked(address)?;
        without_panicking(|| self.backend().extent(probe)).flatten()
    }

    /// Every row that names a file and a line, whatever the object, handed to `visit` as
    /// `(range, file, line)` in the **placed** address space ([`Section::bias`] already
    /// applied, and never taken off). A backend may hold its own lock for the whole walk,
    /// and `extent` and `line_info` take the same one, so `visit` must not ask the object
    /// anything: the one caller, `SourceIndex::build`, is handed the extents it needs
    /// instead of the object.
    fn each_row(&self, visit: &mut dyn FnMut(Range<PlacedAddress>, &str, u32)) {
        without_panicking(|| self.backend().each_row(visit));
    }

    /// The one backend this object has, as the three questions the seam puts.
    fn backend(&self) -> &dyn LineBackend {
        match &self.backend {
            Backend::Dwarf(dwarf) => dwarf,
            Backend::Pdb(pdb) => pdb,
        }
    }
}

/// Run a backend with a net under it, turning a panic into "no line info".
///
/// Not general defensiveness: known, reachable bugs in the dependencies behind the seam, all
/// unchecked arithmetic on numbers a debug section states and none of them something this
/// crate can validate without parsing the debug info twice. In `addr2line` 0.27, a line-table
/// row's length is `next.address - row.address`, and nothing stops a line program from moving
/// its address backwards. In `pdb2` 0.10, a module's line data is sliced out of its stream at
/// `start..start + size` unchecked, a line block's size has its header subtracted unchecked,
/// and a section offset plus a length is a plain `+` (`notes/upstream/pdb2.md`).
///
/// The guard is around [`DebugInfo::load`] as well as the queries, since a backend reads the
/// file to build itself: the PDB's eager open is there, and `addr2line`'s context walks every
/// unit's ranges.
///
/// Sound because a panic leaves nothing half-written: a backend is only ever read, and the
/// lock a panic poisons is recovered explicitly ([`recovered`]).
fn without_panicking<T>(f: impl FnOnce() -> T) -> Option<T> {
    crate::guard::guard(f)
}

/// A backend's lock, taken whether or not it is poisoned. The other half of
/// [`without_panicking`]'s soundness: a poisoned lock here is a guarded panic's, and a
/// backend is only ever read, so nothing is left half-written and the poison says nothing
/// worth propagating. Every lock a backend takes goes through this — one taken with a plain
/// `unwrap` would turn the next guarded panic into a permanent "no line info" for the object.
pub(super) fn recovered<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// Rows as a backend hands them over, and the one path from there to a [`LineInfo`]: each
/// row clipped to the range asked about and moved out of the placed space the backends
/// answer in ([`push`](Self::push)), files deduplicated in first-seen order, and
/// [`finish`](Self::finish) making the rows ascending, non-overlapping and coalesced. Every
/// backend feeds this, so the invariants are made in one place rather than promised by each.
struct RowCollector {
    /// The range asked about, in the space rows are pushed in.
    query: Range<PlacedAddress>,
    /// What comes off a row to put it in the space [`LineInfo`] is keyed in — the section's
    /// own — **after** it has been clipped to the query. The order is the rule, and it is
    /// [`push`](Self::push)'s.
    bias: Bias,
    rows: Vec<LineRow>,
    files: Vec<FileEntry>,
    indices: HashMap<Arc<str>, usize>,
}

/// One source file that rows name: its name as the debug info spells it, and the checksum
/// the debug info recorded for it, where it did.
struct FileEntry {
    name: Arc<str>,
    hash: Option<SourceHash>,
}

impl RowCollector {
    /// Rows over `query`, pushed in the placed space `bias` puts a section's addresses in
    /// and answered in that section's own.
    fn over(query: Range<PlacedAddress>, bias: Bias) -> RowCollector {
        RowCollector {
            query,
            bias,
            rows: Vec::new(),
            files: Vec::new(),
            indices: HashMap::new(),
        }
    }

    /// Rows clipped to nothing and moved by nothing, for a caller whose rows are already in
    /// the space it wants them in: a whole module decoded at once, or rows handed straight
    /// over.
    fn whole() -> RowCollector {
        RowCollector::over(PlacedAddress::ZERO..PlacedAddress::MAX, Bias::NONE)
    }

    /// The index a file name will have in [`LineInfo::files`], interning it on first sight
    /// along with the hash recorded for it — the first hash seen for a name is the one kept.
    fn file(&mut self, name: &str, hash: Option<SourceHash>) -> usize {
        match self.indices.get(name) {
            Some(index) => *index,
            None => {
                let name: Arc<str> = Arc::from(name);
                let index = self.files.len();
                self.files.push(FileEntry {
                    name: name.clone(),
                    hash,
                });
                self.indices.insert(name, index);
                index
            }
        }
    }

    /// One row, in the placed space every backend answers in, clipped to the query and moved
    /// into the section's own space. A row with nothing left inside the query is dropped
    /// here, and a column of 0 — which both formats write for "no column" — is taken as
    /// none, so no backend has to check either.
    ///
    /// **Both ends are clipped before the bias comes off**, and that order is the rule this
    /// owns. `addr2line` 0.27 hands back the row containing the query's start, which may
    /// begin before it, clips nothing at the top, and checks nowhere that a row ends past
    /// its start at all: a line program that moves its address backwards — a second
    /// `DW_LNE_set_address` in one sequence, relocated differently or not relocated — has
    /// rows lying below where the section was placed. Subtracting the bias first turned such
    /// a row into one running to the end of the address space, which [`LineInfo::row_at`]
    /// then answered with for every address the real rows left uncovered.
    fn push(
        &mut self,
        range: Range<PlacedAddress>,
        file: Option<usize>,
        line: Option<u32>,
        column: Option<u32>,
    ) {
        let start = range.start.max(self.query.start);
        let end = range.end.min(self.query.end);
        if start >= end {
            return;
        }
        let (Some(start), Some(end)) =
            (start.local_checked(self.bias), end.local_checked(self.bias))
        else {
            return;
        };
        self.rows.push(LineRow {
            range: start..end,
            file,
            line,
            column: column.filter(|&column| column != 0),
        });
    }

    /// The rows made to hold [`LineInfo`]'s invariants, or [`None`] when there are none:
    /// "there is debug info but it says nothing about this range" and "there is no debug
    /// info" are the same answer to a caller.
    fn finish(self) -> Option<LineInfo> {
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
        let mut covered = SectionAddress::ZERO;
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
    pub range: Range<SectionAddress>,
    /// An index into [`LineInfo::files`], read with [`LineInfo::file`], or [`None`] when the
    /// row names no file.
    pub file: Option<usize>,
    /// The line number. Genuinely optional: DWARF's line 0 means "these instructions belong
    /// to no source line", which is neither line 0 nor line 1.
    pub line: Option<u32>,
    /// The column number, [`None`] both when the producer emitted no column at all and when
    /// it emitted 0, the "left edge of the line" marker.
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
/// section ([`crate::sections::section_biases`]) and by the clipping in [`RowCollector::finish`];
/// where two rows genuinely covered one address, the one that starts first keeps it.
pub struct LineInfo {
    rows: Vec<LineRow>,
    /// The files the rows name, each with its checksum. [`LineRow::file`] indexes this.
    files: Vec<FileEntry>,
}

impl LineInfo {
    /// Line info from rows and files handed over directly, made to hold the invariants
    /// below the way a backend's rows are, or [`None`] when no row covers anything. Each
    /// row's `file` indexes `files` as given. For code that has line info to stand in for
    /// what a backend would have said — a test of the app's panes, say — and nothing else.
    pub fn new(rows: Vec<LineRow>, files: Vec<(Arc<str>, Option<SourceHash>)>) -> Option<LineInfo> {
        let mut collector = RowCollector::whole();
        let indices: Vec<usize> = files
            .iter()
            .map(|(name, hash)| collector.file(name, *hash))
            .collect();
        for row in rows {
            let file = row.file.and_then(|file| indices.get(file).copied());
            // The rows are the caller's own and nothing placed them, so they go in and come
            // back out as the same numbers.
            let range = row.range.start.unplaced()..row.range.end.unplaced();
            collector.push(range, file, row.line, row.column);
        }
        collector.finish()
    }

    /// Every row, ascending by address and non-overlapping.
    pub fn rows(&self) -> &[LineRow] {
        &self.rows
    }

    /// The rows that overlap `range`, not clipped to it. The rows ascend and do not overlap,
    /// so two binary searches find them. For a backend that keeps line info it decoded
    /// earlier and answers a smaller range out of it.
    fn rows_over(&self, range: Range<SectionAddress>) -> &[LineRow] {
        let first = self
            .rows
            .partition_point(|row| row.range.end <= range.start);
        let last = self.rows.partition_point(|row| row.range.start < range.end);
        // `get`: a backwards range puts `first` past `last`.
        self.rows.get(first..last).unwrap_or(&[])
    }

    /// The source files these rows touch, deduplicated, in the order they were first seen.
    /// [`LineRow::file`] is a position in this order, read with [`file`](Self::file).
    pub fn files(&self) -> impl Iterator<Item = &Arc<str>> {
        self.files.iter().map(|entry| &entry.name)
    }

    /// The file at this index of [`files`](Self::files), or [`None`] when the index is not a
    /// file's.
    pub fn file(&self, index: usize) -> Option<&Arc<str>> {
        self.files.get(index).map(|entry| &entry.name)
    }

    /// The checksum the debug info recorded for the file of this name, or [`None`] where it
    /// recorded none (DWARF, as read here) or these rows name no such file.
    pub fn hash_for(&self, file: &str) -> Option<SourceHash> {
        self.files
            .iter()
            .find(|entry| *entry.name == *file)
            .and_then(|entry| entry.hash)
    }

    /// The file at this index and its checksum, for a backend that passes these rows on
    /// through a [`RowCollector`] of its own.
    fn file_with_hash(&self, index: usize) -> Option<(&str, Option<SourceHash>)> {
        let entry = self.files.get(index)?;
        Some((&entry.name, entry.hash))
    }

    /// The row covering `address`, or [`None`] when no row does. [`covering`]'s one
    /// candidate, the last row starting at or before `address`, is the only one *because*
    /// the rows do not overlap.
    pub fn row_at(&self, address: SectionAddress) -> Option<&LineRow> {
        let index = covering(&self.rows, |row| row.range.clone(), address)?;
        self.rows.get(index)
    }

    /// Where the code at `address` opens: the file it was compiled from and the line of it.
    ///
    /// The row covering `address` answers both, falling back to the first row that names a
    /// file at all — a prologue the debug info places on no line leaves
    /// [`row_at`](Self::row_at) with nothing to say. **One row for both answers**, so the
    /// line is a line of the file and not of another.
    ///
    /// [`None`] where no file is named at all. Where no row names one but these rows came
    /// with a file anyway — every row that named it was clipped away — that file is the
    /// answer and no line comes with it.
    pub fn opening(&self, address: SectionAddress) -> Option<(&Arc<str>, Option<u32>)> {
        let opening = self
            .row_at(address)
            .filter(|row| row.file.is_some())
            .or_else(|| self.rows.iter().find(|row| row.file.is_some()));
        let file = opening
            .and_then(|row| row.file)
            .and_then(|file| self.file(file))
            .or_else(|| self.files().next())?;
        Some((file, opening.and_then(|row| row.line)))
    }
}

impl Object {
    /// The line info for an address range **within one section**, building this object's
    /// debug info on the first call and reusing it afterwards.
    ///
    /// The section is not decoration: in a relocatable object every section starts at 0, so
    /// `range` on its own does not say which code it means. See
    /// [`crate::sections::section_biases`].
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
    pub fn line_info(
        &self,
        section: &Section,
        range: Range<SectionAddress>,
    ) -> Option<Arc<LineInfo>> {
        self.debug_info()?.line_info(section, range)
    }

    /// How many bytes of code the debug info says the function starting at `address` **within
    /// one section** is, or [`None`] when it does not say. Cached per unit visited; see
    /// [`SymbolData::extent`] for how it and the next-symbol estimate bound each other.
    pub fn function_extent(&self, section: &Section, address: SectionAddress) -> Option<u64> {
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
    ///
    /// It works that extent out itself at no extra cost: [`extent`](Self::extent) is
    /// memoized per symbol. So even a caller with the assembly in hand, which has already
    /// paid for it, asks this rather than [`Object::line_info`] over
    /// [`Assembly::range`](crate::Assembly::range).
    pub fn line_info(&self, object: &Object) -> Option<Arc<LineInfo>> {
        let section = self.section.as_ref()?;
        let end = self.address.checked_add(self.extent(object)?.bytes)?;
        object.line_info(section, self.address..end)
    }

    /// What the debug info says this symbol's extent is, [`None`] when it says nothing.
    pub fn debug_extent(&self, object: &Object) -> Option<u64> {
        object.function_extent(self.section.as_ref()?, self.address)
    }
}

#[cfg(test)]
mod tests;

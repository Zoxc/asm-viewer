//! The PDB backend of [`super`]: a PE image's `.pdb`, found by the CodeView record in its
//! debug directory and read with `pdb2`, the one module in the crate that knows that crate.
//!
//! A PDB is not embedded in the binary but a **second file**, so this backend is the only one
//! that touches the filesystem: [`find`] tries the recorded path and the two places a `.pdb`
//! is shipped beside its binary, and takes the first whose GUID and age are the image's — a
//! stale `.pdb` being worse than none. The file stays open for the object's lifetime and is
//! read a page at a time through [`BoundedFile`], never whole: a `rustc_driver` PDB is 268 MB.
//!
//! Addresses come out of a PDB as `section:offset` pairs. Every one goes through the PDB's
//! own [`AddressMap`] to an RVA — which is also where an OMAP-rearranged image is undone — and
//! then onto the image base, so everything this backend answers is in the same virtual
//! address space a linked image's `Section::address` and `SymbolData::address` already are.
//! A linked image has no section bias, so [`Pdb`] has no notion of one.
//!
//! Line info in a PDB is **per module** (one object file the linker took in), found from an
//! address by the DBI's section contributions. A module is decoded whole the first time an
//! address in it is asked about — its rows into one [`LineInfo`], its procedures into a
//! table of extents — and kept, the way the DWARF backend keeps a unit's subprogram extents.
//!
//! Nothing here recurses, and nothing here catches a panic: the guard is [`super::DebugInfo`]'s.

use super::{LineInfo, RowCollector, SourceHash};
use object::Object as _;
use pdb2::{AddressMap, DebugInformation, FallibleIterator, PdbInternalRva, StringTable, PDB};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One image's `.pdb`, opened and matched once and kept for the object's lifetime.
pub(super) struct Pdb {
    /// Every stream read goes through `&mut PDB`, and `PDB` is `Send` but not `Sync`: the
    /// same Mutex-for-`Sync` reasoning as the DWARF backend's context. Taken per module
    /// loaded and released before the module is decoded, so no other lock nests under it.
    pdb: Mutex<PDB<'static, BoundedFile>>,

    /// The DBI stream, owned: modules are found in it by index.
    dbi: DebugInformation<'static>,

    /// The `/names` stream, or [`None`] when the PDB has none — rows then name no file, and
    /// extents still answer.
    strings: Option<StringTable<'static>>,

    address_map: AddressMap<'static>,

    /// What an RVA is added to for the address space the image's sections are in.
    image_base: u64,

    /// How many modules the DBI lists, so [`Pdb::each_row`] knows where to stop.
    module_count: usize,

    /// Every section contribution as a virtual address range and the module it belongs to,
    /// sorted by start with a running `max_end`, so a range is mapped to its modules by a
    /// bounded backwards walk. Built once at load.
    contributions: Vec<Contribution>,

    /// The modules decoded so far, by index. [`None`] remembers a module with no stream, no
    /// rows and no procedures, so it is not re-read for every symbol in it.
    modules: Mutex<HashMap<usize, Option<Arc<ModuleLines>>>>,
}

/// One section contribution, in virtual addresses.
struct Contribution {
    start: u64,
    end: u64,
    /// The furthest `end` of this entry and every entry before it, the bound a backwards
    /// walk stops at; the same shape as `source.rs`'s `SymbolRange::max_end`.
    max_end: u64,
    module: usize,
}

/// One module's line info, decoded whole on first touch.
struct ModuleLines {
    /// The module's rows in virtual addresses, already through [`RowCollector::finish`]:
    /// ascending and non-overlapping, so the rows over a range are two `partition_point`s.
    lines: LineInfo,
    /// The start address of every `S_GPROC32`/`S_LPROC32` with a length, to that length. The
    /// first procedure read at an address keeps it.
    procedures: HashMap<u64, u64>,
}

impl Pdb {
    /// Open and match the `.pdb` this image names, or [`None`]: for an image with no CodeView
    /// record, a `.pdb` that is nowhere it is looked for, one that is not the image's, or one
    /// whose tables will not read.
    pub(super) fn load(file: &object::File<'_>, path: &Path) -> Option<Pdb> {
        let codeview = file.pdb_info().ok()??;
        let image_base = file.relative_address_base();

        let recorded = String::from_utf8_lossy(codeview.path());
        let (mut pdb, dbi) = find(&recorded, codeview.guid(), codeview.age(), path)?;

        let address_map = pdb.address_map().ok()?;
        let strings = pdb.string_table().ok();
        let module_count = dbi.modules().ok()?.count().ok()?;

        let mut contributions = Vec::new();
        let mut listed = dbi.section_contributions().ok()?;
        // A malformed tail stops the walk where it goes wrong and keeps what was read.
        while let Ok(Some(contribution)) = listed.next() {
            let Some(start) = contribution.offset.to_internal_rva(&address_map) else {
                continue;
            };
            let Some(end) = start.0.checked_add(contribution.size) else {
                continue;
            };
            for range in address_map.rva_ranges(start..PdbInternalRva(end)) {
                let (Some(start), Some(end)) = (
                    image_base.checked_add(u64::from(range.start.0)),
                    image_base.checked_add(u64::from(range.end.0)),
                ) else {
                    continue;
                };
                if start < end {
                    contributions.push(Contribution {
                        start,
                        end,
                        max_end: end,
                        module: contribution.module,
                    });
                }
            }
        }
        contributions.sort_unstable_by_key(|c| (c.start, c.end, c.module));
        let mut max_end = 0;
        for contribution in &mut contributions {
            max_end = max_end.max(contribution.end);
            contribution.max_end = max_end;
        }

        Some(Pdb {
            pdb: Mutex::new(pdb),
            dbi,
            strings,
            address_map,
            image_base,
            module_count,
            contributions,
            modules: Mutex::default(),
        })
    }

    /// The rows over `range`, out of every module contributing to it.
    pub(super) fn line_info(&self, range: Range<u64>) -> Option<LineInfo> {
        let mut rows = RowCollector::default();
        for module in self.modules_over(range.clone()) {
            let Some(module) = self.module(module) else {
                continue;
            };
            let lines = &module.lines;
            let first = lines
                .rows
                .partition_point(|row| row.range.end <= range.start);
            let last = lines
                .rows
                .partition_point(|row| row.range.start < range.end);
            for row in &lines.rows[first..last] {
                let file = row
                    .file
                    .map(|file| rows.file(&lines.files[file], lines.hashes[file]));
                rows.push(
                    row.range.start.max(range.start)..row.range.end.min(range.end),
                    file,
                    row.line,
                    row.column,
                );
            }
        }
        rows.finish()
    }

    /// The length of the procedure beginning at `address`, or [`None`] when no module
    /// contributes there or none of its procedures begins at that address.
    pub(super) fn extent(&self, address: u64) -> Option<u64> {
        let end = address.checked_add(1)?;
        self.modules_over(address..end)
            .into_iter()
            .filter_map(|module| self.module(module))
            .find_map(|module| module.procedures.get(&address).copied())
    }

    /// Every row of every module that names a file and a line. Each module is loaded under
    /// the PDB's lock and visited after it is released; the `modules` lock is held for no
    /// longer than a lookup.
    pub(super) fn each_row(&self, visit: &mut dyn FnMut(Range<u64>, &str, u32)) {
        for index in 0..self.module_count {
            let Some(module) = self.module(index) else {
                continue;
            };
            let lines = &module.lines;
            for row in &lines.rows {
                let (Some(file), Some(line)) = (row.file, row.line) else {
                    continue;
                };
                visit(row.range.clone(), &lines.files[file], line);
            }
        }
    }

    /// The modules with a contribution overlapping `range`, each once, in index order.
    fn modules_over(&self, range: Range<u64>) -> Vec<usize> {
        let pos = self
            .contributions
            .partition_point(|contribution| contribution.start < range.end);
        let mut modules: Vec<usize> = self.contributions[..pos]
            .iter()
            .rev()
            .take_while(|contribution| contribution.max_end > range.start)
            .filter(|contribution| contribution.end > range.start)
            .map(|contribution| contribution.module)
            .collect();
        modules.sort_unstable();
        modules.dedup();
        modules
    }

    /// The module with this index, decoded on first ask and remembered — including as
    /// [`None`] when it has nothing to say.
    fn module(&self, index: usize) -> Option<Arc<ModuleLines>> {
        // A poisoned lock means a previous query panicked. A module is inserted only once it
        // is whole, so nothing here is left half-written by one: recover, do not propagate.
        let modules = self.modules.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(module) = modules.get(&index) {
            return module.clone();
        }
        drop(modules);

        let decoded = self.decode(index).map(Arc::new);

        let mut modules = self.modules.lock().unwrap_or_else(|e| e.into_inner());
        modules.entry(index).or_insert(decoded).clone()
    }

    fn decode(&self, index: usize) -> Option<ModuleLines> {
        let module = self.dbi.modules().ok()?.nth(index).ok()??;
        let info = {
            let mut pdb = self.pdb.lock().unwrap_or_else(|e| e.into_inner());
            pdb.module_info(&module).ok()??
        };

        let mut rows = RowCollector::default();
        if let Ok(program) = info.line_program() {
            // Each file is resolved through the string table once per module, not per row.
            let mut files: HashMap<u32, Option<usize>> = HashMap::new();
            let mut lines = program.lines();
            // A malformed tail stops the walk where it goes wrong and keeps what was read.
            while let Ok(Some(line)) = lines.next() {
                let Some(start) = line.offset.to_internal_rva(&self.address_map) else {
                    continue;
                };
                // A row without a length is one whose successor sits *below* it — a shape
                // only assemblers emit — and it is dropped rather than given an end.
                let Some(end) = line.length.and_then(|len| start.0.checked_add(len)) else {
                    continue;
                };
                let file = *files.entry(line.file_index.0).or_insert_with(|| {
                    let info = program.get_file_info(line.file_index).ok()?;
                    let name = self.strings.as_ref()?.get(info.name).ok()?.to_string();
                    let hash = match info.checksum {
                        pdb2::FileChecksum::Md5(bytes) => {
                            bytes.try_into().ok().map(SourceHash::Md5)
                        }
                        pdb2::FileChecksum::Sha1(bytes) => {
                            bytes.try_into().ok().map(SourceHash::Sha1)
                        }
                        pdb2::FileChecksum::Sha256(bytes) => {
                            bytes.try_into().ok().map(SourceHash::Sha256)
                        }
                        pdb2::FileChecksum::None => None,
                    };
                    Some(rows.file(&name, hash))
                });
                // CodeView's line 0 is DWARF's: instructions belonging to no line. Column 0 is
                // the "no column" it writes when asked for none.
                let line_number = (line.line_start != 0).then_some(line.line_start);
                let column = line.column_start.filter(|&column| column != 0);
                for range in self.address_map.rva_ranges(start..PdbInternalRva(end)) {
                    let (Some(start), Some(end)) = (
                        self.image_base.checked_add(u64::from(range.start.0)),
                        self.image_base.checked_add(u64::from(range.end.0)),
                    ) else {
                        continue;
                    };
                    rows.push(start..end, file, line_number, column);
                }
            }
        }

        let mut procedures = HashMap::new();
        if let Ok(mut symbols) = info.symbols() {
            while let Ok(Some(symbol)) = symbols.next() {
                let Ok(pdb2::SymbolData::Procedure(procedure)) = symbol.parse() else {
                    continue;
                };
                if procedure.len == 0 {
                    continue;
                }
                let Some(start) = procedure
                    .offset
                    .to_rva(&self.address_map)
                    .and_then(|rva| self.image_base.checked_add(u64::from(rva.0)))
                else {
                    continue;
                };
                // Two procedures at one address is a function and its alias; the first one
                // read keeps the address.
                procedures.entry(start).or_insert(u64::from(procedure.len));
            }
        }

        let lines = rows.finish();
        if lines.is_none() && procedures.is_empty() {
            return None;
        }
        Some(ModuleLines {
            lines: lines.unwrap_or_else(|| LineInfo {
                rows: Vec::new(),
                files: Vec::new(),
                hashes: Vec::new(),
            }),
            procedures,
        })
    }
}

/// Open the `.pdb` an image's CodeView record describes, trying in order: the recorded path
/// itself where it is absolute; the recorded file name beside the binary (the build
/// machine's directory is gone, the name is not); and the binary's own name with a `.pdb`
/// extension beside it, which is how a `foo.dll` ships as `foo.dll` + `foo.pdb`. The first
/// candidate that opens as a PDB and **matches** is taken.
///
/// Matching is the GUID and the age both: the GUID says which build, and the age which
/// relink of it — an incremental relink keeps the GUID and bumps the age, and its `.pdb`
/// then describes code the image no longer has. The age compared is the DBI's, which is
/// what the linker wrote; the info stream's own age is bumped by tools that rewrite a PDB
/// afterwards (source indexing, `pdbstr`) and may legitimately exceed the image's. A PDB
/// so old it states no DBI age predates the line-table format read here, and is declined.
fn find(
    recorded: &str,
    guid: [u8; 16],
    age: u32,
    binary: &Path,
) -> Option<(PDB<'static, BoundedFile>, DebugInformation<'static>)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut candidate = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };

    let recorded_path = Path::new(recorded);
    if recorded_path.is_absolute() {
        candidate(recorded_path.to_path_buf());
    }
    let beside = binary.parent().unwrap_or(Path::new(""));
    // The recorded path is split on both separators: it was written by a Windows linker
    // whatever this is running on.
    if let Some(name) = recorded
        .rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty())
    {
        candidate(beside.join(name));
    }
    candidate(binary.with_extension("pdb"));

    candidates.into_iter().find_map(|path| {
        let file = BoundedFile::open(&path)?;
        let mut pdb = PDB::open(file).ok()?;
        let info = pdb.pdb_information().ok()?;
        // `Uuid::from_fields` read the file's mixed-endian bytes as little-endian fields, and
        // `to_bytes_le` writes them back the same way: this compares the bytes on disk with
        // the bytes in the image, whichever way round the uuid crate spells them.
        if info.guid.to_bytes_le() != guid {
            return None;
        }
        let dbi = pdb.debug_information().ok()?;
        if dbi.age() != Some(age) {
            return None;
        }
        Some((pdb, dbi))
    })
}

/// A `.pdb` on disk, read a page at a time, with every read **bounded by the file's length
/// before anything is allocated**.
///
/// `pdb2`'s own `Source` for a `Read + Seek` sizes a `Vec` by the stream directory's declared
/// length before reading a byte, so a PDB whose directory says a stream is four gigabytes
/// long asks for four gigabytes — never a panic, so no guard would catch it, but an abort on
/// a file the user merely opened. This is the same class as `section_data`'s lying
/// compressed size, and the same answer: a declared size is weighed against the bytes there
/// are, and a stream that claims more than the file holds is an I/O error to `pdb2`, which
/// reports it and reads nothing.
#[derive(Debug)]
struct BoundedFile {
    file: File,
    len: u64,
}

impl BoundedFile {
    fn open(path: &Path) -> Option<BoundedFile> {
        let file = File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(BoundedFile {
            file,
            len: metadata.len(),
        })
    }
}

/// The bytes one `view` read, owned.
#[derive(Debug)]
struct Bytes(Vec<u8>);

impl pdb2::SourceView<'_> for Bytes {
    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl<'s> pdb2::Source<'s> for BoundedFile {
    fn view(
        &mut self,
        slices: &[pdb2::SourceSlice],
    ) -> Result<Box<dyn pdb2::SourceView<'s> + Send + Sync>, io::Error> {
        let out_of_range = || io::Error::from(io::ErrorKind::UnexpectedEof);

        // Every slice within the file, and the total no more than the file: a valid MSF
        // never lists a page twice, so a stream cannot honestly be longer than its file.
        let mut total: u64 = 0;
        for slice in slices {
            let size = u64::try_from(slice.size).map_err(|_| out_of_range())?;
            let end = slice.offset.checked_add(size).ok_or_else(out_of_range)?;
            if end > self.len {
                return Err(out_of_range());
            }
            total = total.checked_add(size).ok_or_else(out_of_range)?;
        }
        if total > self.len {
            return Err(out_of_range());
        }
        let total = usize::try_from(total).map_err(|_| out_of_range())?;

        let mut bytes = vec![0u8; total];
        let mut filled = 0;
        for slice in slices {
            self.file.seek(SeekFrom::Start(slice.offset))?;
            self.file
                .read_exact(&mut bytes[filled..filled + slice.size])?;
            filled += slice.size;
        }
        Ok(Box::new(Bytes(bytes)))
    }
}

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
//! The PDB is also the one debug format that names functions the image does not: a `/DEBUG`
//! image has no COFF symbol table, so a stripped `.exe` declares its entry point and a DLL
//! its exports and nothing else, while the PDB knows every function. [`Pdb::procedures`]
//! walks every module's symbols once for its `S_GPROC32`/`S_LPROC32` records — name,
//! address, length — and `parse_object` takes them as symbols beside the image's own. That
//! walk is at **parse time**, so the `.pdb` is opened there for an image that has one and
//! the backend built then is the one kept for the line questions later (`DebugInfo::pdb`);
//! the line tables are still decoded lazily. The walk reads each module's stream and keeps
//! nothing of it but the procedures it hands back, and a module asked about later is read
//! again — the simpler of the two shapes, and the re-read is exactly the first-question cost
//! the lazy path had before: holding every module's procedure table from the walk would
//! duplicate what the symbols now carry as their declared size, and the stream would still
//! have to be read again for its lines.
//!
//! Behind the procedures come the **publics** ([`Pdb::publics`]): the linker's own table of
//! every externally visible symbol, `S_PUB32` records in the symbol records stream, each a
//! decorated name and an address and nothing else — no length, no lines. They are what
//! survives a stripped PDB (`/PDBSTRIPPED` keeps the publics and drops every module stream),
//! and what names a function in a module that shipped without debug info, a thunk, or
//! assembler code: in `rustc_driver.dll`'s PDB 2250 of the 2907 modules have no stream at
//! all. The walk takes the ones flagged as code or a function, and `parse_object` takes them
//! after the procedures under its one-per-address rule, so a public is only ever the name of
//! an address nothing else named. Read whole once at parse, held no longer than the walk.
//!
//! Nothing here recurses, and nothing here catches a panic: the guard is [`super::DebugInfo`]'s.

use super::{LineInfo, RowCollector, SourceHash};
use object::Object as _;
use pdb2::{
    AddressMap, DebugInformation, FallibleIterator, PdbInternalRva, PdbInternalSectionOffset,
    StringTable, PDB,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// How many walks of a DBI module list have been started, so a test can pin that a question
/// over every module costs one walk and not one per module. Test builds only.
#[cfg(test)]
pub(super) static WALKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One image's `.pdb`, opened and matched once and kept for the object's lifetime.
pub(super) struct Pdb {
    /// Every stream read goes through `&mut PDB`, and `PDB` is `Send` but not `Sync`: the
    /// same Mutex-for-`Sync` reasoning as the DWARF backend's context. Taken per module
    /// loaded and released before the module is decoded, so no other lock nests under it;
    /// held across the whole of [`Pdb::procedures`] and of [`Pdb::publics`], which run at
    /// parse before anything else can ask.
    pdb: Mutex<PDB<'static, BoundedFile>>,

    /// The DBI stream, owned: modules are found in it by index.
    dbi: DebugInformation<'static>,

    /// The `/names` stream, or [`None`] when the PDB has none — rows then name no file, and
    /// extents still answer.
    strings: Option<StringTable<'static>>,

    address_map: AddressMap<'static>,

    /// What an RVA is added to for the address space the image's sections are in.
    image_base: u64,

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

/// One function a PDB names, as `parse_object` takes it: an `S_GPROC32`/`S_LPROC32` record
/// with a length, its address already in the image's virtual address space.
pub(crate) struct Procedure {
    /// The name as the record spells it — the compiler's display name (`add`,
    /// `core::ptr::drop_in_place<T>`), not a mangled one.
    pub(crate) name: String,
    pub(crate) address: u64,
    /// The record's length, never 0.
    pub(crate) len: u64,
}

/// One public symbol a PDB names for code, as `parse_object` takes it: an `S_PUB32` record
/// flagged as code or a function, its address already in the image's virtual address space.
/// A public has no length.
pub(crate) struct Public {
    /// The name as the linker saw it — decorated (`?add@@YAHHH@Z`, `_ZN4core3ptr…`), or
    /// plain for C — so the demangler has something to say about it where it has nothing
    /// about a procedure's display name.
    pub(crate) name: String,
    pub(crate) address: u64,
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
            contributions,
            modules: Mutex::default(),
        })
    }

    /// Every procedure with a length in every module, in module order and then the order
    /// the module's symbols are in. One pass over every module stream, under the PDB's lock
    /// for the whole walk; a module whose stream will not read, or a record that will not
    /// parse, is skipped and the walk goes on. Two records at one address are both handed
    /// back — the caller's one-per-address rule decides between them.
    pub(super) fn procedures(&self) -> Vec<Procedure> {
        let mut procedures = Vec::new();
        let Ok(mut modules) = self.module_list() else {
            return procedures;
        };
        let mut pdb = self.pdb.lock().unwrap_or_else(|e| e.into_inner());
        // A malformed tail stops the walk where it goes wrong and keeps what was read.
        while let Ok(Some(module)) = modules.next() {
            let Ok(Some(info)) = pdb.module_info(&module) else {
                continue;
            };
            let Ok(mut symbols) = info.symbols() else {
                continue;
            };
            while let Ok(Some(symbol)) = symbols.next() {
                let Ok(pdb2::SymbolData::Procedure(procedure)) = symbol.parse() else {
                    continue;
                };
                if procedure.len == 0 {
                    continue;
                }
                let Some(address) = self.address(procedure.offset) else {
                    continue;
                };
                procedures.push(Procedure {
                    name: procedure.name.to_string().into_owned(),
                    address,
                    len: u64::from(procedure.len),
                });
            }
        }
        procedures
    }

    /// Every public flagged as code or a function, in the order the symbol records stream
    /// holds them. The stream is read whole once — it is the one stream the publics are in
    /// — and dropped with the walk; a record that will not parse is skipped, and a malformed
    /// tail stops the walk where it goes wrong and keeps what was read. Which of the two
    /// flags a linker sets is its own: `rust-lld` marks a function `function` alone, so
    /// either is taken, and the caller's code-section lookup is what keeps a public out of
    /// the data sections. Two records at one address are both handed back — the caller's
    /// one-per-address rule decides between them, and drops one at an address a procedure
    /// already named.
    pub(super) fn publics(&self) -> Vec<Public> {
        let mut publics = Vec::new();
        let mut pdb = self.pdb.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(table) = pdb.global_symbols() else {
            return publics;
        };
        let mut symbols = table.iter();
        while let Ok(Some(symbol)) = symbols.next() {
            let Ok(pdb2::SymbolData::Public(public)) = symbol.parse() else {
                continue;
            };
            if !(public.code || public.function) {
                continue;
            }
            let Some(address) = self.address(public.offset) else {
                continue;
            };
            publics.push(Public {
                name: public.name.to_string().into_owned(),
                address,
            });
        }
        publics
    }

    /// A `section:offset` the PDB states, as an address in the image's own space: through
    /// the address map to an RVA and onto the image base, or [`None`] where either fails.
    fn address(&self, offset: PdbInternalSectionOffset) -> Option<u64> {
        let rva = offset.to_rva(&self.address_map)?;
        self.image_base.checked_add(u64::from(rva.0))
    }

    /// The rows over `range`, out of every module contributing to it.
    pub(super) fn line_info(&self, range: Range<u64>) -> Option<LineInfo> {
        let mut rows = RowCollector::default();
        let over = self.modules_over(range.clone());
        // One walk for the lot: `module` alone would start a walk per module not yet decoded.
        if over.iter().any(|&index| self.remembered(index).is_none()) {
            self.walk(Some(&over));
        }
        for module in over {
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

    /// Every row of every module that names a file and a line. Every module is decoded in
    /// one walk of the module list and visited from the table after. Each is loaded under
    /// the PDB's lock and visited once it is released; the `modules` lock is held for no
    /// longer than a lookup.
    pub(super) fn each_row(&self, visit: &mut dyn FnMut(Range<u64>, &str, u32)) {
        let count = self.walk(None);
        for index in 0..count {
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
        if let Some(module) = self.remembered(index) {
            return module;
        }
        self.walk(Some(&[index]));
        self.remembered(index).flatten()
    }

    /// The module with this index if it has been decoded: the outer [`None`] is "not yet",
    /// the inner one "nothing to say".
    fn remembered(&self, index: usize) -> Option<Option<Arc<ModuleLines>>> {
        // A poisoned lock means a previous query panicked. A module is inserted only once it
        // is whole, so nothing here is left half-written by one: recover, do not propagate.
        let modules = self.modules.lock().unwrap_or_else(|e| e.into_inner());
        modules.get(&index).cloned()
    }

    /// Decode the modules `wanted` names, remembering each, and answer how many modules were
    /// walked past. `wanted` is ascending; [`None`] wants every module.
    ///
    /// The DBI module list is a chain of variable-length records, so an index is reached only
    /// by parsing every record before it. Decoding each module from a walk of its own costs
    /// the square of a count the file states, and a big enough module list turns that into a
    /// hang on the analysis thread. One walk serves however many modules are wanted, and
    /// stops after the last of them.
    fn walk(&self, wanted: Option<&[usize]>) -> usize {
        let last = match wanted {
            Some(&[.., last]) => Some(last),
            Some([]) => return 0,
            None => None,
        };
        let Ok(mut modules) = self.module_list() else {
            return 0;
        };
        let mut count = 0;
        // A malformed tail stops the walk where it goes wrong and keeps what was read.
        while let Ok(Some(module)) = modules.next() {
            let index = count;
            count += 1;
            let asked = wanted.is_none_or(|wanted| wanted.binary_search(&index).is_ok());
            if asked && self.remembered(index).is_none() {
                let decoded = self.decode(&module).map(Arc::new);
                let mut modules = self.modules.lock().unwrap_or_else(|e| e.into_inner());
                modules.entry(index).or_insert(decoded);
            }
            if last.is_some_and(|last| index >= last) {
                break;
            }
        }
        // A wanted module the walk never reached — past the end of the list, or past where a
        // malformed one stopped it — has nothing to say. Remembering that is what keeps it
        // from starting a walk of its own on every later ask.
        if let Some(wanted) = wanted {
            let mut modules = self.modules.lock().unwrap_or_else(|e| e.into_inner());
            for &index in wanted {
                modules.entry(index).or_insert(None);
            }
        }
        count
    }

    /// The DBI module list from the front, the one place it is walked from.
    fn module_list(&self) -> pdb2::Result<pdb2::ModuleIter<'_>> {
        #[cfg(test)]
        WALKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.dbi.modules()
    }

    fn decode(&self, module: &pdb2::Module<'_>) -> Option<ModuleLines> {
        let info = {
            let mut pdb = self.pdb.lock().unwrap_or_else(|e| e.into_inner());
            pdb.module_info(module).ok()??
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
                let Some(start) = self.address(procedure.offset) else {
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

#[cfg(test)]
mod tests;

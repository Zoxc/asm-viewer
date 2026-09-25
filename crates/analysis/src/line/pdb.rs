//! The PDB backend of [`super`]: a PE image's `.pdb`, found by the CodeView record in its
//! debug directory and read with `pdb2`, the one module in the crate that knows that crate.
//!
//! A PDB is not embedded in the binary but a **second file**, so this backend is the only one
//! that touches the filesystem: [`find`] tries the two places a `.pdb` is shipped beside its
//! binary and then the path the image records, and takes the first whose GUID and age are the
//! image's — a stale `.pdb` being worse than none. The recorded path is the binary's own
//! bytes, so it is taken as a name and never as a host to reach ([`candidates`]). The file
//! stays open for the object's lifetime and is read a page at a time through [`BoundedFile`],
//! never whole: a `rustc_driver` PDB is 268 MB.
//!
//! Addresses come out of a PDB as `section:offset` pairs. Every one goes through the PDB's
//! own [`AddressMap`] to an RVA — which is also where an OMAP-rearranged image is undone — and
//! then onto the image base, so everything this backend reads is in the same virtual
//! address space a linked image's `Section::address` and `SymbolData::address` already are.
//! A linked image is one nothing placed, so that space is also the seam's **placed** one,
//! the two being a bias of [`Bias::NONE`] apart: [`own`] and [`placed`] are that, said
//! rather than read off a type.
//!
//! Line info in a PDB is **per module** (one object file the linker took in), found from an
//! address by the DBI's section contributions. A module is decoded whole the first time an
//! address in it is asked about — its rows into one [`LineInfo`], its procedures into a
//! table of extents — and kept, the way the DWARF backend keeps a unit's subprogram extents.
//!
//! The PDB is also the one debug format that names functions the image does not: a `/DEBUG`
//! image has no COFF symbol table, so a stripped `.exe` declares its entry point and a DLL
//! its exports and nothing else, while the PDB knows every function. [`Pdb::declared`] is
//! that answer, and it is asked at **parse time**, so the `.pdb` is opened there for an
//! image that has one and the backend built then is the one kept for the line questions
//! later ([`super::DebugInfo::declared`]); the line tables are still decoded lazily.
//!
//! It walks every module's symbols once for its `S_GPROC32`/`S_LPROC32` records — name,
//! address, length — and hands those **procedures** back first. The walk reads each module's
//! stream and keeps nothing of it but what it hands back, and a module asked about later is
//! read again — the simpler of the two shapes, and the re-read is exactly the first-question
//! cost the lazy path had before: holding every module's procedure table from the walk would
//! duplicate what the symbols now carry as their declared size, and the stream would still
//! have to be read again for its lines. Both reads take a module's procedures from the one
//! walk, [`Pdb::procedures_in`].
//!
//! Behind them come the **publics**: the linker's own table of every externally visible
//! symbol, `S_PUB32` records in the symbol records stream, each a decorated name and an
//! address and nothing else — no length, no lines. They are what survives a stripped PDB
//! (`/PDBSTRIPPED` keeps the publics and drops every module stream), and what names a
//! function in a module that shipped without debug info, a thunk, or assembler code: in
//! `rustc_driver.dll`'s PDB 2250 of the 2907 modules have no stream at all. The walk takes
//! the ones flagged as code or a function. Read whole once at parse, held no longer than the
//! walk.
//!
//! Nothing here recurses, and nothing here catches a panic: the guard is [`super::DebugInfo`]'s.
//! A symbol record is parsed only if it is a kind the walks use ([`PROCEDURES`]).

use super::intervals::Intervals;
use super::{recovered, Declared, LineBackend, LineInfo, RowCollector, SourceHash};
use crate::parse::Name;
use crate::{open_regular, Bias, Links, PlacedAddress, Regular, SectionAddress};
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
use std::sync::{Arc, Mutex, OnceLock};

/// One image's `.pdb`, opened and matched once and kept for the object's lifetime.
pub(super) struct Pdb {
    /// Every stream read goes through `&mut PDB`, and `PDB` is `Send` but not `Sync`: the
    /// same Mutex-for-`Sync` reasoning as the DWARF backend's context. Taken per module
    /// loaded and released before the module is decoded, so no other lock nests under it;
    /// held across the whole of [`Pdb::declared`], which runs at parse before anything else
    /// can ask.
    pdb: Mutex<PDB<'static, BoundedFile>>,

    /// The DBI stream, owned: modules are found in it by index.
    dbi: DebugInformation<'static>,

    /// The `/names` stream, or [`None`] when the PDB has none — rows then name no file, and
    /// extents still answer.
    strings: Option<StringTable<'static>>,

    address_map: AddressMap<'static>,

    /// What an RVA is added to for the address space the image's sections are in.
    image_base: SectionAddress,

    /// Every section contribution as a virtual address range, to the module it belongs to.
    /// Built once at load.
    contributions: Intervals<SectionAddress, usize>,

    /// The modules decoded so far, by index. [`None`] remembers a module with no stream, no
    /// rows and no procedures, so it is not re-read for every symbol in it.
    modules: Mutex<HashMap<usize, Option<Arc<ModuleLines>>>>,

    /// How many modules the list holds, once one walk has decoded them all. Past that no
    /// question walks the list again, not even for a module the list does not hold.
    every: OnceLock<usize>,

    /// How many walks of the DBI module list this PDB has started, so a test can pin that a
    /// question over every module costs one walk and not one per module. Test builds only.
    #[cfg(test)]
    walks: std::sync::atomic::AtomicUsize,
}

/// One module's line info, decoded whole on first touch.
struct ModuleLines {
    /// The module's rows in virtual addresses, already through [`RowCollector::finish`], so
    /// the rows over a range are [`LineInfo::rows_over`]. [`None`] for a module with
    /// procedures and no rows.
    lines: Option<LineInfo>,
    /// The start address of every `S_GPROC32`/`S_LPROC32` with a length, to that length. The
    /// first procedure read at an address keeps it.
    procedures: HashMap<SectionAddress, u64>,
}

/// The symbol record kinds that are procedures: `S_GPROC32`, `S_LPROC32` and their `_ST`,
/// `_ID` and `_DPC` spellings, all of which `pdb2` parses as a `ProcedureSymbol`.
///
/// **The walks hand `pdb2` only these and [`PUBLICS`] to parse**, and read every other record
/// by its kind alone. The parse of some other kinds trusts what the record states
/// (`notes/upstream/pdb2.md`): an `S_CALLEES` or `S_CALLERS` allocates the count it states
/// before checking the record holds that many, so a record of a few bytes asks for 16 GiB,
/// an abort no guard catches. Others assert in a debug build. The crate uses none of them.
/// `pdb2` keeps its own constants private, so the numbers are spelled here.
const PROCEDURES: [pdb2::SymbolKind; 8] = [
    0x100a, // S_LPROC32_ST
    0x100b, // S_GPROC32_ST
    0x110f, // S_LPROC32
    0x1110, // S_GPROC32
    0x1146, // S_LPROC32_ID
    0x1147, // S_GPROC32_ID
    0x1155, // S_LPROC32_DPC
    0x1156, // S_LPROC32_DPC_ID
];

/// The symbol record kinds that are publics, `S_PUB32` and its `_ST` spelling: the other
/// kind the walks parse ([`PROCEDURES`]).
const PUBLICS: [pdb2::SymbolKind; 2] = [
    0x1009, // S_PUB32_ST
    0x110e, // S_PUB32
];

impl Pdb {
    /// Open and match the `.pdb` this image names, or [`None`]: for an image with no CodeView
    /// record, a `.pdb` that is nowhere it is looked for, one that is not the image's, or one
    /// whose tables will not read.
    pub(super) fn load(file: &object::File<'_>, path: &Path) -> Option<Pdb> {
        let codeview = file.pdb_info().ok()??;
        let image_base = SectionAddress::new(file.relative_address_base());

        let recorded = String::from_utf8_lossy(codeview.path());
        let (mut pdb, dbi) = find(&recorded, codeview.guid(), codeview.age(), path)?;

        let address_map = pdb.address_map().ok()?;
        let strings = pdb.string_table().ok();

        let mut contributions = Vec::new();
        let mut listed = dbi.section_contributions().ok()?;
        // A malformed tail stops the walk where it goes wrong and keeps what was read.
        while let Ok(Some(contribution)) = listed.next() {
            let ranges = rebased(
                &address_map,
                image_base,
                contribution.offset,
                contribution.size,
            );
            contributions.extend(ranges.map(|range| (range, contribution.module)));
        }
        let contributions = Intervals::new(contributions);

        Some(Pdb {
            pdb: Mutex::new(pdb),
            dbi,
            strings,
            address_map,
            image_base,
            contributions,
            modules: Mutex::default(),
            every: OnceLock::new(),
            #[cfg(test)]
            walks: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Every function this PDB names, in the order the parse claims addresses in: every
    /// module's **procedures** first, then the **publics**. Both walks are under the PDB's
    /// lock, taken once, and both run at parse before anything else can ask.
    ///
    /// **Procedures before publics**, because a procedure carries the compiler's display
    /// name and a length where a public carries the linker's decorated spelling and an
    /// address, so a public only ever names what no procedure did: a function in a module
    /// that shipped without debug info, a thunk, assembler code, or — a stripped PDB having
    /// no module streams — every function there is. Two records at one address are both
    /// handed back either way; the caller's one-per-address rule is what decides.
    pub(super) fn declared(&self) -> Vec<Declared> {
        let mut declared = Vec::new();
        let mut pdb = recovered(&self.pdb);

        // Every procedure with a length, in module order and then the order the module's
        // symbols are in. One pass over every module stream; a module whose stream will not
        // read, or a record that will not parse, is skipped and the walk goes on. A
        // procedure's name is the compiler's display name (`add`,
        // `core::ptr::drop_in_place<T>`), which no demangler claims.
        for (_, module) in self.modules() {
            let Ok(Some(info)) = pdb.module_info(&module) else {
                continue;
            };
            declared.extend(
                self.procedures_in(&info)
                    .map(|(address, procedure)| Declared {
                        name: Name::Informative(procedure.name.to_string().into_owned()),
                        address,
                        len: Some(u64::from(procedure.len)),
                    }),
            );
        }

        // Then every public flagged as code or a function, in the order the symbol records
        // stream holds them. The stream is read whole once — it is the one stream the
        // publics are in — and dropped with the walk; a record that will not parse is
        // skipped, and a malformed tail stops the walk where it goes wrong and keeps what
        // was read. Which of the two flags a linker sets is its own: `rust-lld` marks a
        // function `function` alone, so either is taken, and the caller's code-section
        // lookup is what keeps a public out of the data sections. A public's name is the
        // linker's, decorated (`?add@@YAHHH@Z`, `_ZN4core3ptr…`) or plain for C, so it goes
        // through the demanglers; and it has no length. Only a public is parsed
        // ([`PUBLICS`]).
        let Ok(table) = pdb.global_symbols() else {
            return declared;
        };
        let mut symbols = table.iter();
        while let Ok(Some(symbol)) = symbols.next() {
            if !PUBLICS.contains(&symbol.raw_kind()) {
                continue;
            }
            let Ok(pdb2::SymbolData::Public(public)) = symbol.parse() else {
                continue;
            };
            if !(public.code || public.function) {
                continue;
            }
            let Some(address) = self.address(public.offset) else {
                continue;
            };
            declared.push(Declared {
                name: Name::Symbol(public.name.to_string().into_owned()),
                address,
                len: None,
            });
        }
        declared
    }

    /// Every procedure with a length in one module, with its address, in the order the
    /// module's symbols are in: what both reads of a module stream take of its symbols. A
    /// stream that will not read has none, a record of another kind is not parsed
    /// ([`PROCEDURES`]), a procedure that will not parse or whose address will not map is
    /// skipped, and a malformed tail stops the walk where it goes wrong.
    fn procedures_in<'a>(
        &'a self,
        info: &'a pdb2::ModuleInfo<'_>,
    ) -> impl Iterator<Item = (SectionAddress, pdb2::ProcedureSymbol<'a>)> + 'a {
        let symbols = info.symbols().ok().into_iter();
        let symbols = symbols.flat_map(|symbols| symbols.iterator().map_while(Result::ok));
        symbols
            .filter_map(move |symbol| {
                if !PROCEDURES.contains(&symbol.raw_kind()) {
                    return None;
                }
                let Ok(pdb2::SymbolData::Procedure(procedure)) = symbol.parse() else {
                    return None;
                };
                if procedure.len == 0 {
                    return None;
                }
                Some((self.address(procedure.offset)?, procedure))
            })
            .fuse()
    }

    /// A `section:offset` the PDB states, as an address in the image's own space: through
    /// the address map to an RVA and onto the image base, or [`None`] where either fails.
    fn address(&self, offset: PdbInternalSectionOffset) -> Option<SectionAddress> {
        let rva = offset.to_rva(&self.address_map)?;
        self.image_base.checked_add(u64::from(rva.0))
    }

    /// The modules with a contribution overlapping `range`, each once, in index order.
    fn modules_over(&self, range: Range<SectionAddress>) -> Vec<usize> {
        let mut modules: Vec<usize> = self.contributions.over(range).copied().collect();
        modules.sort_unstable();
        modules.dedup();
        modules
    }

    /// The modules `wanted` names (ascending, each once), in that order, skipping any with
    /// nothing to say. Those not yet decoded are decoded in one walk between them. That is
    /// one walk per call, so a pass asking an address at a time decodes every module first
    /// ([`LineBackend::prepare_extents`]). The `modules` lock is taken per module and
    /// released before it is handed over.
    fn decoded<'a>(&'a self, wanted: &'a [usize]) -> impl Iterator<Item = Arc<ModuleLines>> + 'a {
        let missing = wanted.iter().any(|&index| self.remembered(index).is_none());
        if missing && self.every.get().is_none() {
            self.walk(Some(wanted));
        }
        wanted
            .iter()
            .filter_map(|&index| self.remembered(index).flatten())
    }

    /// The module with this index if it has been decoded: the outer [`None`] is "not yet",
    /// the inner one "nothing to say".
    fn remembered(&self, index: usize) -> Option<Option<Arc<ModuleLines>>> {
        let modules = recovered(&self.modules);
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
        let mut count = 0;
        for (index, module) in self.modules() {
            count = index + 1;
            let asked = wanted.is_none_or(|wanted| wanted.binary_search(&index).is_ok());
            if asked && self.remembered(index).is_none() {
                let decoded = self.decode(&module).map(Arc::new);
                let mut modules = recovered(&self.modules);
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
            let mut modules = recovered(&self.modules);
            for &index in wanted {
                modules.entry(index).or_insert(None);
            }
        }
        count
    }

    /// Every module decoded, in the one walk this PDB ever makes of the whole list, and how
    /// many there are.
    fn every_module(&self) -> usize {
        *self.every.get_or_init(|| self.walk(None))
    }

    /// The DBI module list from the front, numbered: the one place it is walked from. A
    /// list that will not read has no modules, and a malformed tail stops the walk where it
    /// goes wrong and keeps what was read.
    fn modules(&self) -> impl Iterator<Item = (usize, pdb2::Module<'_>)> + '_ {
        #[cfg(test)]
        self.walks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let list = self.dbi.modules().ok().into_iter();
        list.flat_map(|list| list.iterator().map_while(Result::ok))
            .enumerate()
            .fuse()
    }

    fn decode(&self, module: &pdb2::Module<'_>) -> Option<ModuleLines> {
        let info = {
            let mut pdb = recovered(&self.pdb);
            pdb.module_info(module).ok()??
        };

        // The module whole, in the image's own addresses: nothing is clipped and nothing
        // comes off, the query being the caller's own when one of these rows reaches it.
        let mut rows = RowCollector::whole();
        if let Ok(program) = info.line_program() {
            // Each file is resolved through the string table once per module, not per row.
            let mut files: HashMap<u32, Option<usize>> = HashMap::new();
            let mut lines = program.lines();
            // A malformed tail stops the walk where it goes wrong and keeps what was read.
            while let Ok(Some(line)) = lines.next() {
                // A row without a length is one whose successor sits *below* it — a shape
                // only assemblers emit — and it is dropped rather than given an end.
                let Some(len) = line.length else {
                    continue;
                };
                let file = *files
                    .entry(line.file_index.0)
                    .or_insert_with(|| self.intern_file(&program, line.file_index, &mut rows));
                // CodeView's line 0 is DWARF's: instructions belonging to no line. Column 0
                // is the "no column" it writes when asked for none, which the collector
                // takes as none.
                let line_number = (line.line_start != 0).then_some(line.line_start);
                let column = line.column_start;
                for range in rebased(&self.address_map, self.image_base, line.offset, len) {
                    rows.push(placed(range), file, line_number, column);
                }
            }
        }

        let mut procedures = HashMap::new();
        for (address, procedure) in self.procedures_in(&info) {
            // Two procedures at one address is a function and its alias; the first one read
            // keeps the address.
            procedures
                .entry(address)
                .or_insert(u64::from(procedure.len));
        }

        let lines = rows.finish();
        if lines.is_none() && procedures.is_empty() {
            return None;
        }
        Some(ModuleLines { lines, procedures })
    }

    /// The file a module's line program names by `index`, resolved through the string table
    /// and interned into `rows` with its checksum, or [`None`] where either lookup fails.
    fn intern_file(
        &self,
        program: &pdb2::LineProgram<'_>,
        index: pdb2::FileIndex,
        rows: &mut RowCollector,
    ) -> Option<usize> {
        let info = program.get_file_info(index).ok()?;
        let name = self.strings.as_ref()?.get(info.name).ok()?.to_string();
        Some(rows.file(&name, source_hash(info.checksum)))
    }
}

impl LineBackend for Pdb {
    /// The rows over `query`, out of every module contributing to it.
    fn line_info(&self, query: Range<PlacedAddress>, rows: &mut RowCollector) {
        let range = own(query);
        let over = self.modules_over(range.clone());
        for module in self.decoded(&over) {
            let Some(lines) = &module.lines else {
                continue;
            };
            for row in lines.rows_over(range.clone()) {
                let file = row
                    .file
                    .and_then(|file| lines.file_with_hash(file))
                    .map(|(name, hash)| rows.file(name, hash));
                // The clip to the query is the collector's, as it is for every backend
                // (`RowCollector::push`).
                rows.push(placed(row.range.clone()), file, row.line, row.column);
            }
        }
    }

    /// The length of the procedure beginning at `address`, or [`None`] when no module
    /// contributes there or none of its procedures begins at that address. Every module
    /// covering the address is decoded, in the one walk, before any is asked.
    fn extent(&self, address: PlacedAddress) -> Option<u64> {
        let address = address.local(Bias::NONE);
        let end = address.checked_add(1)?;
        let over = self.modules_over(address..end);
        // Bound, not returned: the iterator borrows `over`, and a tail expression's
        // temporaries outlive the function's locals.
        let extent = self
            .decoded(&over)
            .find_map(|module| module.procedures.get(&address).copied());
        extent
    }

    /// Every module, in one walk. The extent pass asks one address at a time, and a walk
    /// per module it finds undecoded would cost the square of the module count.
    fn prepare_extents(&self) {
        self.every_module();
    }

    /// Every row of every module that names a file and a line. Every module is decoded in
    /// one walk of the module list and visited from the table after. Each is loaded under
    /// the PDB's lock and visited once it is released; the `modules` lock is held for no
    /// longer than a lookup.
    fn each_row(&self, visit: &mut dyn FnMut(Range<PlacedAddress>, &str, u32)) {
        let count = self.every_module();
        // That walk remembered every module, so each is only looked up here.
        for module in (0..count).filter_map(|index| self.remembered(index).flatten()) {
            let Some(lines) = &module.lines else {
                continue;
            };
            for row in lines.rows() {
                let file = row.file.and_then(|file| lines.file(file));
                let (Some(file), Some(line)) = (file, row.line) else {
                    continue;
                };
                visit(placed(row.range.clone()), file, line);
            }
        }
    }
}

/// A file's checksum as the PDB records it, or [`None`] where it records none or one of the
/// wrong length.
fn source_hash(checksum: pdb2::FileChecksum<'_>) -> Option<SourceHash> {
    match checksum {
        pdb2::FileChecksum::Md5(bytes) => bytes.try_into().ok().map(SourceHash::Md5),
        pdb2::FileChecksum::Sha1(bytes) => bytes.try_into().ok().map(SourceHash::Sha1),
        pdb2::FileChecksum::Sha256(bytes) => bytes.try_into().ok().map(SourceHash::Sha256),
        pdb2::FileChecksum::None => None,
    }
}

/// A range the seam asked about, in the image's own addresses, and [`placed`] is the way
/// back. A PDB describes a **linked image**, which nothing placed, so the two spaces hold
/// the same numbers; these say which of them is meant rather than leave it to be read off a
/// type.
fn own(range: Range<PlacedAddress>) -> Range<SectionAddress> {
    range.start.local(Bias::NONE)..range.end.local(Bias::NONE)
}

/// A range of the image's own addresses in the space the seam reads every backend in; the
/// inverse of [`own`].
fn placed(range: Range<SectionAddress>) -> Range<PlacedAddress> {
    range.start.unplaced()..range.end.unplaced()
}

/// Open the `.pdb` an image's CodeView record describes, trying the paths [`candidates`]
/// names in order. The first that opens as a PDB and **matches** is taken.
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
    candidates(recorded, binary).into_iter().find_map(|path| {
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

/// The `len` bytes the PDB states at `offset`, as the ranges of the image's own address
/// space they lie over: through the address map, which can split them, and onto the image
/// base. Nothing when the offset will not map or its end overflows; a piece that would
/// overflow the address space, or that is empty, is dropped.
fn rebased<'a>(
    address_map: &'a AddressMap<'_>,
    image_base: SectionAddress,
    offset: PdbInternalSectionOffset,
    len: u32,
) -> impl Iterator<Item = Range<SectionAddress>> + 'a {
    let pieces = offset.to_internal_rva(address_map).and_then(|start| {
        let end = start.0.checked_add(len)?;
        Some(address_map.rva_ranges(start..PdbInternalRva(end)))
    });
    pieces.into_iter().flatten().filter_map(move |range| {
        let start = image_base.checked_add(u64::from(range.start.0))?;
        let end = image_base.checked_add(u64::from(range.end.0))?;
        (start < end).then_some(start..end)
    })
}

/// The paths the `.pdb` an image records is looked for at, in order: the recorded file name
/// beside the binary (the build machine's directory is gone, the name is not); the binary's
/// own name with a `.pdb` extension beside it, which is how a `foo.dll` ships as `foo.dll` +
/// `foo.pdb`; and last the recorded path itself, where it is absolute and plain.
///
/// The recorded path is bytes out of the binary, so the **binary** chooses it, and every PE
/// the reader opens is parsed with it. A path beginning with two separators is a UNC share,
/// a device or a verbatim path (`\\host\share\x.pdb`, `\\.\pipe\x`); opening the first makes
/// the machine log in to `host` over SMB, offering the reader's credentials, before a byte
/// comes back. None is tried, on any platform — the string was written by a Windows linker
/// whatever this is running on — and what is left is tried last, the two names beside the
/// binary being the ones that name a file the reader already has.
fn candidates(recorded: &str, binary: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut candidate = |path: PathBuf| {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };

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

    let mut start = recorded.chars();
    let unc_or_device = matches!(
        (start.next(), start.next()),
        (Some('\\' | '/'), Some('\\' | '/'))
    );
    let recorded_path = Path::new(recorded);
    if !unc_or_device && recorded_path.is_absolute() {
        candidate(recorded_path.to_path_buf());
    }

    candidates
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
        // A candidate can name a fifo, and the thread a blocking open would stop is the one
        // parsing the object. `open_regular` does not wait on one, and asks the file it
        // opened rather than the path.
        let Regular { file, len } = open_regular(path, Links::Follow).ok()?;
        Some(BoundedFile { file, len })
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

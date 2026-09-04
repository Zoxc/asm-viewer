use object::{
    read::archive::ArchiveFile, CompressionFormat, Object as _, ObjectKind, ObjectSection,
    ObjectSymbol, Relocation, SectionKind, SymbolKind,
};
use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    hash::Hasher as _,
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

mod demangle;
pub mod disasm;
pub mod guard;
mod line;
mod listing;
mod unwind;

use disasm::Code;
use line::{DebugInfo, Procedure, Public};
use unwind::UnwindEntry;

pub use disasm::{Assembly, BranchEdge, Instruction, SpanKind};
pub use line::{DebugInfoCache, LineInfo, LineRow, Location, SourceDigests, SourceHash};
pub use listing::{CodeListing, DecodedStretch, Gap, GapKind, Listing, Place, Placed, Stretch};
// Re-exported so the viewer needs no `object` dependency of its own.
pub use object::{Architecture, BinaryFormat, SectionIndex, SymbolIndex};

/// [`Object`] is shared as an `Arc` and read from worker threads; the others are what a
/// worker is handed and hands back. Asserted here so a field that stops being shared-safe
/// is a compile error in this crate rather than a borrow error in the UI.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Object>();
    assert_send_sync::<Symbol>();
    assert_send_sync::<Assembly>();
    assert_send_sync::<LineInfo>();
    assert_send_sync::<Listing>();
    assert_send_sync::<CodeListing>();
    assert_send_sync::<DecodedStretch>();
};

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    pub format: BinaryFormat,

    /// The machine the code in here is for, as the file's own header declares it. This is
    /// what picks a disassembler ([`SymbolData::assembly`]) and the only thing that can: a
    /// symbol's bytes say nothing about how to read themselves.
    pub architecture: Architecture,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    /// The same symbols **sorted by name**, byte order. The Symbols list draws it in that
    /// order and a saved place is found in it by binary search, so anything building an
    /// `Object` by hand has to sort it the same way.
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,

    /// This object's debug info, built on the first query — except for a PE whose matching
    /// `.pdb` was opened at parse time for the symbols it names, whose backend is seeded
    /// here so it is not opened twice. Anything building an `Object` by hand writes
    /// `DebugInfoCache::default()`. See [`Object::line_info`].
    pub debug_info: DebugInfoCache,

    /// The same symbols by the address they are **placed** at, built on the first
    /// disassembly and derived from `symbols_sorted`, so anything building an `Object` by
    /// hand writes `AddressIndex::default()` and cannot disagree with it. See
    /// [`Object::symbol_at`].
    pub by_address: AddressIndex,
}

/// [`Object::by_address`]: every text symbol with a section, keyed by its placed address
/// ([`Section::bias`] added to its own) and sorted by it, each address once.
///
/// Built once per object and not per disassembly, behind a `OnceLock` like the debug info:
/// 67 ms for the 115k-symbol sample in an unoptimized build, which is nothing once and
/// everything at every click. Lazy rather than built at parse because an archive's members
/// are parsed all at once and read one at a time.
#[derive(Default)]
pub struct AddressIndex(OnceLock<Vec<(u64, Arc<SymbolData>)>>);

impl Object {
    /// The text symbol that **starts** at `placed`, in the one address space every section
    /// of this object shares ([`Section::bias`]); [`None`] where no symbol does. Two names
    /// for one address answer the first by name — the order `symbols_sorted` holds — so the
    /// answer is the same however the map behind them was iterated.
    ///
    /// The address alone is only a key with the bias in it: in a relocatable object every
    /// code section starts at 0. A caller holding an address in a section's own terms adds
    /// the section's bias first, and one that knows which section the address is in checks
    /// the answer is in it too — the bias makes two sections two places, but a number past
    /// one section's end is still just a number.
    pub fn symbol_at(&self, placed: u64) -> Option<&Arc<SymbolData>> {
        let index = self.by_address.0.get_or_init(|| {
            let mut placed: Vec<(u64, Arc<SymbolData>)> = self
                .symbols_sorted
                .iter()
                .filter_map(|symbol| {
                    let section = symbol.section.as_ref()?;
                    Some((symbol.address.wrapping_add(section.bias), symbol.clone()))
                })
                .collect();
            // Stable, so that of two symbols at one address the first by name is the one
            // `dedup` keeps.
            placed.sort_by_key(|(address, _)| *address);
            placed.dedup_by_key(|(address, _)| *address);
            placed
        });
        let at = index
            .binary_search_by_key(&placed, |(address, _)| *address)
            .ok()?;
        index.get(at).map(|(_, symbol)| symbol)
    }
}

/// A digest of a whole file's bytes: what tells "the same binary" from "one rebuilt
/// underneath the session that named it" (`src/project.rs`). Nothing in this crate reads
/// one; it is computed here because this is where the bytes already are.
///
/// The **content**, not the size and modification time, which are wrong in both directions
/// for the question. xxHash64 because its output is a specified property of the bytes —
/// `std`'s `DefaultHasher` reserves the right to change algorithm between releases, which
/// would declare every saved binary rebuilt after a toolchain upgrade.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileDigest(u64);

impl FileDigest {
    pub fn of(bytes: &[u8]) -> FileDigest {
        // Seed 0, xxHash64's own default: part of the algorithm's identity here, so it is
        // written down rather than chosen per run.
        let mut hasher = twox_hash::XxHash64::with_seed(0);
        hasher.write(bytes);
        FileDigest(hasher.finish())
    }
}

/// Sixteen lowercase hex digits, which is the form the session writes.
impl fmt::Display for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileDigest({self})")
    }
}

/// The bytes an [`Object`] was parsed from, held for as long as the object lives: parsing
/// keeps decompressed bytes only for the sections holding text symbols, and the lazy line
/// info pass needs the rest.
///
/// The bytes are **shared, not copied** — every `Object` out of one file holds a clone of
/// the same `Arc<[u8]>` and differs only in `range` — so an archive costs its bytes once,
/// and one live member keeps the whole archive alive.
#[derive(Clone)]
pub struct ObjectData {
    file: Arc<[u8]>,
    range: Range<usize>,
    /// The digest of the **whole file**, not of `range`: the unit a session names is the
    /// file. [`ObjectData::member`] copies this, so an archive costs one hash and not one
    /// per member.
    digest: FileDigest,
}

impl ObjectData {
    /// The whole file: a plain object file, or the archive file itself. **This is where a
    /// file is hashed**, once, for every object that will come out of it.
    pub fn whole_file(file: Arc<[u8]>) -> Self {
        let range = 0..file.len();
        let digest = FileDigest::of(&file);
        Self {
            file,
            range,
            digest,
        }
    }

    /// One archive member of `file`, as the `(offset, size)` its header declares. [`None`]
    /// when that range does not lie inside the file — the same bounds check
    /// `ArchiveMember::data` does.
    pub fn member(file: &ObjectData, offset: u64, size: u64) -> Option<Self> {
        let start: usize = offset.try_into().ok()?;
        let end = start.checked_add(size.try_into().ok()?)?;
        file.file.get(start..end)?;
        Some(Self {
            file: file.file.clone(),
            range: start..end,
            digest: file.digest,
        })
    }

    /// The object file's own bytes.
    pub fn bytes(&self) -> &[u8] {
        // The range was bounds-checked when it was built.
        &self.file[self.range.clone()]
    }

    /// The digest of the file this object was parsed out of; every object from one file
    /// answers the same thing.
    pub fn digest(&self) -> FileDigest {
        self.digest
    }
}

impl std::fmt::Debug for ObjectData {
    /// Never the bytes themselves: an object file is megabytes of them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectData")
            .field("range", &self.range)
            .field("file_len", &self.file.len())
            .field("digest", &self.digest)
            .finish()
    }
}

/// Copies the bytes into an allocation of their own, for a caller that only has a slice;
/// [`open_files`] shares one allocation per file instead.
impl From<&[u8]> for ObjectData {
    fn from(data: &[u8]) -> Self {
        Self::whole_file(Arc::from(data))
    }
}

impl From<Vec<u8>> for ObjectData {
    fn from(data: Vec<u8>) -> Self {
        Self::whole_file(Arc::from(data))
    }
}

#[derive(Debug)]
pub struct Section {
    /// The section's index in the file it was parsed from, which is what identifies it to a
    /// later pass that re-reads that file — an address on its own is not a key in a
    /// relocatable object where every section starts at 0.
    pub index: SectionIndex,
    pub name: String,
    pub data: Vec<u8>,
    pub address: u64,

    pub relocations: HashMap<u64, Relocation>,

    /// The addresses of this section's text symbols, sorted and **each once**: two symbols
    /// at one address are one entry here. The sync points a listing decodes from.
    pub symbols: Vec<u64>,

    /// The address ranges the file's own unwind table states for the functions in this
    /// section — an x86-64 PE's `.pdata`, an ELF's `.eh_frame`, out of [`unwind::entries`]
    /// — sorted by start, each start once, ends clamped to the section's bytes. Empty for a
    /// file with no table read. What [`SymbolData::extent`] answers from first.
    pub unwind: Vec<Range<u64>>,

    /// Whether the file marks this section as holding code (`SectionKind::Text`). Every
    /// section whose bytes read is kept, since the line info needs the debug ones; this is
    /// what tells the code apart for a listing of all of it.
    pub code: bool,

    /// Where the object's layout puts this section: what is added to an address in it to
    /// place it in the one address space every section of the object shares. 0 for every
    /// section of a linked image, whose addresses are real, and for a section that is not
    /// code; in a relocatable object, where every code section starts at 0, an address of
    /// its own for each. See [`section_biases`].
    pub bias: u64,
}

/// Where each code section is placed in the one address space the object's line info is read
/// in and its code is listed in; what [`Section::bias`] is set from.
///
/// **An address alone is not a key in a relocatable object.** Sections there have no address
/// until linked and rustc emits one `.text.<name>` per function, so every function lands on 0
/// and the line programs pile up. This does what a linker does and gives each code section a
/// place of its own: a bias, added to every address relocated against that section
/// (`line::relocate`) and subtracted again from every row a query returns.
///
/// Two limits, both load-bearing:
///
/// * **Relocatable objects only.** A linked image holds real addresses literally rather than
///   through relocations; moving the few that are relocated would move them away from the
///   rest.
/// * **Code sections only.** An absolute relocation in a debug section is often an offset
///   into another `.debug_*` section (`DW_AT_stmt_list`, `DW_FORM_strp`), which must come out
///   exactly as it went in.
pub(crate) fn section_biases(file: &object::File<'_>) -> HashMap<SectionIndex, u64> {
    let mut biases = HashMap::new();
    if file.kind() != ObjectKind::Relocatable {
        return biases;
    }

    let mut next: u64 = 0;
    for section in file.sections() {
        if section.kind() != SectionKind::Text {
            continue;
        }

        biases.insert(section.index(), next.wrapping_sub(section.address()));

        // Somewhere for the next section to go. A zero-length section still takes an address
        // of its own, so that two of them are two places. An object whose sections do not fit
        // in the address space simply stops being biased past that point.
        let Some(end) = next.checked_add(section.size().max(1)) else {
            break;
        };
        let Some(aligned) = end.checked_next_multiple_of(SECTION_ALIGNMENT) else {
            break;
        };
        next = aligned;
    }

    biases
}

/// What [`section_biases`] rounds each section's placement up to. Nothing depends on the
/// value; the gap it leaves means an off-by-one cannot walk into the next section.
const SECTION_ALIGNMENT: u64 = 16;

/// How far [`SymbolData::estimate_size`]'s next-symbol derivation may reach (1 MiB) before
/// it is treated as having said nothing. Not a claim about how long a function can be — five
/// times the largest in any sample here — but the point past which a sparse export table's
/// derivation is certainly describing something else, at megabytes of decoding per redraw.
/// Where the unwind table states where a function ends — an x86-64 PE, an ELF with an
/// `.eh_frame` — the cap reaches only a symbol no entry covers; it stays for the rest.
const MAX_DERIVED_SIZE: u64 = 1 << 20;

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    pub address: u64,
    pub section: Option<Arc<Section>>,
    pub size: u64,
}

impl SymbolData {
    /// What to call this symbol on screen. The disassembler substitutes this for a relocated
    /// operand, so anything rendering a relocation target has to use the same rule.
    pub fn display(&self) -> &str {
        self.demangled.as_deref().unwrap_or(&self.name)
    }

    /// Object files frequently report a size of 0, so derive the extent from the next symbol
    /// in the section (or the section end). An *upper* bound rather than a measurement: it
    /// includes alignment padding, and a declaration the symbol table never mentioned (an
    /// export, an entry point) has no size of its own. Capped at [`MAX_DERIVED_SIZE`]. See
    /// [`extent`](Self::extent).
    pub fn estimate_size(&self) -> Option<u64> {
        Some(self.derived()?.min(MAX_DERIVED_SIZE))
    }

    /// [`estimate_size`](Self::estimate_size) before its cap: the bytes from this symbol to
    /// the next in the section, or to the section's end.
    fn derived(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        let i = section.symbols.binary_search(&self.address).ok()?;

        // Where the section's bytes stop. [`None`] only for a section placed so near the end
        // of the address space that it does not fit in it.
        let end = section
            .data
            .len()
            .try_into()
            .ok()
            .and_then(|length: u64| section.address.checked_add(length));

        // The next symbol bounds this one; the section bounds them both. One wild address in
        // the symbol table would otherwise be the *previous* symbol's problem: its extent
        // would run past the bytes that were read and `bytes` would answer `None` for a
        // function that is perfectly readable.
        let next = match section.symbols.get(i + 1) {
            Some(&next) => end.map_or(next, |end| next.min(end)),
            None => end?,
        };

        next.checked_sub(self.address)
    }

    /// A length the file states, bounded by the next symbol. A listing is one stretch per
    /// symbol and decodes each as its symbol's extent, so a length reaching past the next
    /// label would draw those rows twice. A zero derivation is no derivation: a symbol
    /// placed exactly at the section's end has nothing to bound it with.
    fn clamped(&self, stated: u64) -> u64 {
        match self.derived().filter(|&size| size != 0) {
            Some(derived) => stated.min(derived),
            None => stated,
        }
    }

    /// The end the file's own unwind table states for the function this symbol is in
    /// ([`Section::unwind`]), as bytes from the symbol's address, or [`None`] where no entry
    /// covers it. Clamped to the next symbol ([`clamped`](Self::clamped)) — and every
    /// entry's own begin is a symbol, which is what stops a parent at the chained entry of
    /// its cold part.
    fn unwind_extent(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        // The last range starting at or before the address: with the starts sorted and
        // each once, the innermost of any that nest.
        let i = section
            .unwind
            .partition_point(|range| range.start <= self.address)
            .checked_sub(1)?;
        let range = &section.unwind[i];
        if !range.contains(&self.address) {
            return None;
        }
        Some(self.clamped(range.end - self.address))
    }

    /// The size the file declares for this symbol ([`size`](Self::size)) where that
    /// declaration is a function's length in bytes, [`clamped`](Self::clamped); [`None`]
    /// where it declares none or the format's size field is something else.
    ///
    /// **ELF only, and that is an allowlist a format joins on evidence.** An ELF `st_size`
    /// is the ABI's own statement of how many bytes the symbol is, and every mainstream
    /// toolchain fills it in: on `librustc_driver.so` it equals the FDE's length for every
    /// one of the 172 169 functions the `.eh_frame` covers. No other format's nonzero size
    /// means that. A COFF function symbol's is the `TotalSize` of an auxiliary
    /// function-definition record, written for COFF's line-number data rather than to
    /// measure code; XCOFF's is a csect's length, and one csect can hold several functions;
    /// Mach-O states no size at all. A declaration that is *wrong* rather than 0 would be
    /// taken as fact here, which is why only the field with the measurement behind it is
    /// read.
    ///
    /// The clamp catches an over-reaching one — hand-written assembly with a `.size` past
    /// the next label. One that is too small is taken as it stands, as an unwind entry's
    /// stated end and a `DW_AT_high_pc` already are.
    fn declared_extent(&self, object: &Object) -> Option<u64> {
        if object.format != BinaryFormat::Elf || self.size == 0 {
            return None;
        }
        Some(self.clamped(self.size))
    }

    /// How many bytes of code this symbol is. Three answers, in order.
    ///
    /// **The end the file's own unwind table states**, where an entry covers the address
    /// ([`unwind_extent`](Self::unwind_extent)): the image's statement, to its loader, of
    /// the very bytes the unwinder covers, so neither the estimate nor its cap bounds it —
    /// only the next symbol does, for the listing's sake — and the debug info is not asked.
    /// On an x86-64 PE or an ELF with an `.eh_frame` that is nearly every function.
    ///
    /// **Then the size the file declares**, where the format makes that a function's length
    /// ([`declared_extent`](Self::declared_extent)): the symbol table's own answer, which
    /// spares the debug info a walk that would only agree with it. On an ELF built without
    /// unwind tables that is every function its symbol table sizes.
    ///
    /// **Else the smaller** of the extent the debug info declares for the function (DWARF's
    /// `DW_AT_low_pc`/`DW_AT_high_pc`, a PDB procedure's length) and
    /// [`estimate_size`](Self::estimate_size), because each bounds the other in a case the
    /// other gets wrong. The estimate over-reaches into padding and over a function with no
    /// symbol; the declared extent over-reaches when two symbols share one function (an
    /// alias, an assembler label, a split cold part), since it describes the *function*.
    ///
    /// A zero estimate is treated as no estimate: a symbol placed exactly at the section's
    /// end has no bytes to derive from, and the debug info may still know its extent.
    pub fn extent(&self, object: &Object) -> Option<u64> {
        if let Some(stated) = self.unwind_extent() {
            return Some(stated);
        }
        if let Some(declared) = self.declared_extent(object) {
            return Some(declared);
        }
        let estimate = self.estimate_size().filter(|&size| size != 0);
        match (self.debug_extent(object), estimate) {
            (Some(declared), Some(estimate)) => Some(declared.min(estimate)),
            (declared, estimate) => declared.or(estimate),
        }
    }

    /// This symbol's bytes, as far as [`estimate_size`](Self::estimate_size) reaches.
    /// Deliberately *not* the debug-info extent: a symbol does not own the file it came from.
    /// Anything with an [`Object`] in hand wants [`data_in`](Self::data_in).
    pub fn data(&self) -> Option<&[u8]> {
        self.bytes(self.estimate_size()?)
    }

    /// This symbol's bytes over [`extent`](Self::extent) — the same range
    /// [`assembly`](Self::assembly) decodes and [`line_info`](Self::line_info) asks about.
    pub fn data_in(&self, object: &Object) -> Option<&[u8]> {
        self.bytes(self.extent(object)?)
    }

    /// `size` bytes of the section starting at this symbol, or [`None`] when that runs off
    /// the end of what was decompressed.
    fn bytes(&self, size: u64) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = size.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.get(offset..end)
    }

    /// This symbol's disassembly, or [`None`] when there are no bytes to decode. An
    /// architecture no backend claims comes back as an [`Assembly`] whose
    /// [`undecodable`](Assembly::undecodable) names it.
    pub fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let bytes = self.data_in(object)?;
        let code = Code::new(bytes, self.address, self.section.as_deref(), object);
        Some(Arc::new(Assembly::decode(object.architecture, &code)))
    }
}

/// A symbol together with the object it came from. Identity is `Arc` pointer identity, never
/// name or index, so duplicate symbol names across objects stay distinct.
#[derive(Clone)]
pub struct Symbol {
    pub object: Arc<Object>,
    pub data: Arc<SymbolData>,
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && Arc::ptr_eq(&self.data, &other.data)
    }
}

/// A hard ceiling (1 GiB) on a single section's decompressed bytes, whatever its header
/// claims. See [`section_data`].
const MAX_SECTION_DATA: u64 = 1 << 30;

/// Read a section's bytes, decompressing it if it says it is compressed, but only after
/// checking that the size it declares is believable.
///
/// `uncompressed_data()` reserves the size in the compression header *before* it looks at a
/// compressed byte, so one flipped `SHF_COMPRESSED` bit turns into a multi-gigabyte
/// allocation and an OOM abort. `compressed_data()` gives the same information without
/// allocating. Two bounds have to hold, and a section failing either is dropped exactly like
/// one whose data will not read: a ratio bound (DEFLATE cannot expand by more than 1032:1
/// nor a zstd frame by more than 32768:1, so a larger declared size is a lie about *these*
/// bytes), and an absolute one, since the ratio bound still scales with the input.
pub(crate) fn section_data<'data, S: ObjectSection<'data>>(section: &S) -> Option<Vec<u8>> {
    let compressed = section.compressed_data().ok()?;

    let max_ratio: u64 = match compressed.format {
        // Not compressed at all: the bytes are already there, nothing to bound.
        CompressionFormat::None => return Some(compressed.data.to_vec()),
        CompressionFormat::Zlib => 1032,
        CompressionFormat::Zstandard => 32768,
        // Any other format is one `decompress()` does not implement; it would fail.
        _ => return None,
    };

    let ratio_bound = (compressed.data.len() as u64).saturating_mul(max_ratio);
    if compressed.uncompressed_size > ratio_bound.min(MAX_SECTION_DATA) {
        return None;
    }

    Some(compressed.decompress().ok()?.into_owned())
}

/// One symbol as the file states it, held while the whole object's names are demangled in
/// one batch (see [`demangle`]).
struct Pending {
    index: SymbolIndex,
    name: String,
    /// Whether the name is the file's own. The entry point's is not — it is
    /// [`ENTRY_POINT_NAME`], which no demangler has anything to say about.
    mangled: bool,
    section: Option<Arc<Section>>,
    address: u64,
    size: u64,
}

/// A function the file **declares** somewhere other than its symbol table; see
/// [`declared_code`].
struct DeclaredCode {
    name: String,
    /// Whether `name` is the file's own and goes through the demangling batch. The two
    /// declarations that carry no name — the entry point and an unwind entry, function or
    /// fragment — are named in the `<…>` convention of [`ENTRY_POINT_NAME`], which no
    /// demangler has anything to say about.
    mangled: bool,
    address: u64,
    /// What the declaration itself said: a dynamic symbol's size, a PDB procedure's length,
    /// an unwind entry's stated end less its begin, and 0 for an export, the entry point and
    /// a PDB public. The extent used comes from [`SymbolData::extent`], which reads this
    /// only where the format makes it a function's length ([`SymbolData::declared_extent`]).
    size: u64,
    /// The code section containing `address` — an export table and an entry point name an
    /// address and nothing else.
    section: SectionIndex,
}

/// The name given to the entry point, one of the two declarations that carry none; the
/// other, an unwind entry, is named by [`unwind_name`] in the same convention. The angle
/// brackets are the point: no assembler, linker or mangling scheme produces them, so neither
/// can collide with a name that was in the file.
const ENTRY_POINT_NAME: &str = "<entry point>";

/// The name given to code only an unwind entry declares: `<function 0x140001000>`, or
/// `<fragment 0x140001000>` for a chained entry — its own address either way, since 20 000
/// of them in one Symbols list have to be told apart and found.
fn unwind_name(entry: &UnwindEntry) -> String {
    let address = entry.range.start;
    if entry.chained {
        format!("<fragment {address:#x}>")
    } else {
        format!("<function {address:#x}>")
    }
}

/// The code a file declares outside its symbol table: its **entry point**, its **exports**,
/// its ELF `.dynsym`, the **procedures** and **publics** of the `.pdb` a PE names, where
/// that was found and matches (`procedures` and `publics`, out of [`DebugInfo::pdb`]), and
/// the **unwind entries** of an x86-64 PE's exception directory or an ELF's `.eh_frame`
/// (`unwind`, out of [`unwind::entries`]). A stripped shared library is otherwise a file with nothing in it,
/// and a `/DEBUG` image has no symbol table at all.
/// Every address here is one the file — or the debug file matched to it by GUID and age —
/// states outright, so the "nothing is scanned for" rule still holds.
///
/// Three decisions the caller depends on:
///
/// **Only in a code section.** An address is looked up in the kept [`SectionKind::Text`]
/// sections and that section becomes the symbol's own; it doubles as the filter keeping
/// exported *data* out.
///
/// **One symbol per address, earliest source winning** (symbol table > dynamic symbol >
/// export > entry point > PDB procedure > PDB public > unwind entry). An export is very
/// often the symbol table's own function under its exported name, and a second `SymbolData`
/// for it would be a second row in the list for one place in the file. The PDB comes after
/// the image so a name the image itself states is never displaced by the debug file's
/// spelling of it, and its publics after its procedures because a procedure carries a
/// display name and a length where a public is a decorated name and an address: the publics
/// name only what nothing else did — a function in a module that shipped without symbols, a
/// thunk, assembler code, or every function of a stripped PDB. The unwind entries come last
/// of all because they carry no name: one at an address anything else named adds nothing,
/// and one nothing named is called `<function 0x…>` by its address — or `<fragment 0x…>`
/// where its unwind info is chained, a second range of some function's rather than a
/// function ([`UnwindEntry`]).
///
/// **Nothing for a relocatable object.** `entry()` answers 0 for an `.o`, and 0 there is a
/// real function's first byte.
fn declared_code(
    file: &object::File<'_>,
    code: &[(Range<u64>, SectionIndex)],
    known: &mut HashSet<u64>,
    procedures: Vec<Procedure>,
    publics: Vec<Public>,
    unwind: &[UnwindEntry],
) -> Vec<DeclaredCode> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    let mut take = |name: String, mangled: bool, address: u64, size: u64| {
        let Some((_, section)) = code.iter().find(|(range, _)| range.contains(&address)) else {
            return;
        };
        if !known.insert(address) {
            return;
        }
        declared.push(DeclaredCode {
            name,
            mangled,
            address,
            size,
            section: *section,
        });
    };

    for symbol in file.dynamic_symbols() {
        if symbol.kind() != SymbolKind::Text {
            continue;
        }
        let Ok(name) = symbol.name_bytes() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        take(
            String::from_utf8_lossy(name).into_owned(),
            true,
            symbol.address(),
            symbol.size(),
        );
    }

    for export in file.exports().unwrap_or_default() {
        if export.name().is_empty() {
            continue;
        }
        take(
            String::from_utf8_lossy(export.name()).into_owned(),
            true,
            export.address(),
            0,
        );
    }

    // 0 is "this image has no entry point", which is how a DLL built without one states it.
    let entry = file.entry();
    if entry != 0 {
        take(ENTRY_POINT_NAME.to_owned(), false, entry, 0);
    }

    // After the image's own names, so they win. The address is already in the image's space
    // and the code-section lookup is what drops a procedure the PDB places in a section the
    // image does not have code in. A procedure's name is the compiler's display name, which
    // no demangler claims and so comes through the batch as it is.
    for procedure in procedures {
        take(procedure.name, true, procedure.address, procedure.len);
    }

    // And the publics behind them: a name for whatever address is still unnamed, and no
    // length, as an export has none. A public in a data section — the flags are the
    // linker's to set, and the section lookup is the rule — is dropped the same way.
    for public in publics {
        take(public.name, true, public.address, 0);
    }

    // Last of all, the unwind entries: an address and a length for whatever is still
    // unnamed, and no name at all.
    for entry in unwind {
        take(
            unwind_name(entry),
            false,
            entry.range.start,
            entry.range.end - entry.range.start,
        );
    }

    declared
}

/// The address ranges code can be in, each with its section: what [`declared_code`] looks a
/// declared address up in, and what places an unwind entry's range in its
/// [`Section::unwind`]. Only sections that were kept — one whose bytes would not decompress
/// has nothing to disassemble either — and only the ones with bytes.
fn code_sections(
    file: &object::File<'_>,
    sections: &HashMap<SectionIndex, Section>,
) -> Vec<(Range<u64>, SectionIndex)> {
    file.sections()
        .filter(|section| section.kind() == SectionKind::Text)
        .filter_map(|section| {
            let kept = sections.get(&section.index())?;
            let length: u64 = kept.data.len().try_into().ok()?;
            let end = kept.address.checked_add(length)?;
            (length > 0).then_some((kept.address..end, section.index()))
        })
        .collect()
}

/// Parse `data` as a single object file. `name` is the display name (an archive member name
/// or the file name) and `path` the file it came from. Anything that fails to parse yields
/// [`None`]. `data` is kept in the returned [`Object`]; see [`ObjectData`].
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let object = object::File::parse(data.bytes())
        .map(|file| {
            // Where each code section goes, decided once here for the line info and the
            // code listing both.
            let biases = section_biases(&file);
            let mut sections: HashMap<SectionIndex, Section> = file
                .sections()
                .filter_map(|section| {
                    let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();
                    let data = section_data(&section)?;
                    let relocations = section.relocations().collect();
                    Some((
                        section.index(),
                        Section {
                            index: section.index(),
                            name,
                            address: section.address(),
                            data,
                            symbols: Vec::new(),
                            unwind: Vec::new(),
                            relocations,
                            code: section.kind() == SectionKind::Text,
                            bias: biases.get(&section.index()).copied().unwrap_or(0),
                        },
                    ))
                })
                .collect();

            // Insert symbol addresses into sections. The addresses are collected as they go,
            // because that set is what tells `declared_code` which of the file's exports are
            // already in the symbol table under their own name.
            let mut known: HashSet<u64> = HashSet::new();
            file.symbols().for_each(|symbol| {
                if symbol.kind() != SymbolKind::Text {
                    return;
                }

                known.insert(symbol.address());
                symbol
                    .section()
                    .index()
                    .and_then(|index| sections.get_mut(&index))
                    .map(|section| section.symbols.push(symbol.address()));
            });

            // A PE's matching `.pdb` is opened here and not on the first line question,
            // because the procedures and publics it names are functions the image itself
            // does not declare. The backend it builds is kept for the line questions later.
            let (debug_info, procedures, publics) = match DebugInfo::pdb(&file, &path) {
                Some((info, procedures, publics)) => {
                    (DebugInfoCache::preloaded(info), procedures, publics)
                }
                None => (DebugInfoCache::default(), Vec::new(), Vec::new()),
            };

            // Declared code goes into the same sorted lists, because that list is what
            // `estimate_size` derives an extent from and a declaration carries none.
            let unwind = unwind::entries(&file);
            let code = code_sections(&file, &sections);
            let declared = declared_code(&file, &code, &mut known, procedures, publics, &unwind);
            for code in &declared {
                if let Some(section) = sections.get_mut(&code.section) {
                    section.symbols.push(code.address);
                }
            }

            // Every unwind entry's range goes to its section, whether or not its begin
            // became a symbol: an export or a procedure at that address takes its extent
            // from the end the entry states. Clamped to the section's bytes, so that end can
            // never reach past what `bytes` can read.
            for UnwindEntry { range, .. } in &unwind {
                let Some((bounds, index)) = code
                    .iter()
                    .find(|(bounds, _)| bounds.contains(&range.start))
                else {
                    continue;
                };
                if let Some(section) = sections.get_mut(index) {
                    section.unwind.push(range.start..range.end.min(bounds.end));
                }
            }

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    // Sorted for the binary searches over it, and each address once: two
                    // symbols at one address (an alias, an assembler label) are one place
                    // in the section, and a repeated entry would make `estimate_size`
                    // answer 0 for whichever of the two the search landed on.
                    section.symbols.sort_unstable();
                    section.symbols.dedup();
                    // The unwind ranges likewise, by start: a table stating one function
                    // twice is one function, and the search over them assumes it.
                    section.unwind.sort_unstable_by_key(|range| range.start);
                    section.unwind.dedup_by_key(|range| range.start);
                    (index, Arc::new(section))
                })
                .collect();

            let sections = section_map.values().cloned().collect();

            let mut pending: Vec<Pending> = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let section = symbol
                        .section()
                        .index()
                        .and_then(|index| section_map.get(&index).cloned());

                    Some(Pending {
                        index: symbol.index(),
                        name: String::from_utf8_lossy(symbol.name_bytes().ok()?).into_owned(),
                        mangled: true,
                        section,
                        address: symbol.address(),
                        size: symbol.size(),
                    })
                })
                .collect();

            // The declared code joins the same map so `symbols_sorted` stays derived from one
            // place. The keys are indices *past* the file's own symbol table, which is the
            // only honest thing they can be; nothing can reach them by relocation, since a
            // file that declares exports is a linked image.
            if !declared.is_empty() {
                let next = file
                    .symbols()
                    .map(|symbol| symbol.index().0)
                    .max()
                    .map_or(0, |index| index + 1);
                for (offset, code) in declared.into_iter().enumerate() {
                    pending.push(Pending {
                        index: SymbolIndex(next + offset),
                        name: code.name,
                        // An export's name is the file's, and on a Windows DLL very often
                        // MSVC-mangled; a PDB public's is the linker's, decorated the same
                        // way; a PDB procedure's is the compiler's display name, which no
                        // demangler claims and so comes through as it is; the entry
                        // point's and an unwind entry's are ours.
                        mangled: code.mangled,
                        section: section_map.get(&code.section).cloned(),
                        address: code.address,
                        size: code.size,
                    });
                }
            }

            // One batch for the whole object, on stacks of their own and on as many cores
            // as the pool has; see `demangle`. The names are *moved* into the batch and
            // moved back out below rather than copied into it: a shared batch is what lets
            // a job outlive this frame, and 115k names is not a copy worth making for it.
            let names: demangle::Names = Arc::new(
                pending
                    .iter_mut()
                    .map(|symbol| symbol.mangled.then(|| std::mem::take(&mut symbol.name)))
                    .collect(),
            );
            let demangled = demangle::batch(&names);
            // Every job is done, so this is the only reference; the clone is unreachable
            // and is there so that a job that somehow outlived its batch costs a copy
            // rather than the names.
            let names = Arc::try_unwrap(names).unwrap_or_else(|names| (*names).clone());

            let symbols: HashMap<_, _> = pending
                .into_iter()
                .zip(names)
                .zip(demangled)
                .map(|((symbol, name), demangled)| {
                    (
                        symbol.index,
                        Arc::new(SymbolData {
                            name: name.unwrap_or(symbol.name),
                            demangled,
                            section: symbol.section,
                            address: symbol.address,
                            size: symbol.size,
                        }),
                    )
                })
                .collect();

            let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
            symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));

            ParsedObject {
                format: file.format(),
                architecture: file.architecture(),
                symbols,
                symbols_sorted,
                sections,
                debug_info,
            }
        })
        .ok()?;

    // Nothing above borrows the file any more -- sections own decompressed copies of
    // their bytes and relocations are owned values -- so the input can be moved in.
    Some(Arc::new(Object {
        name,
        path,
        format: object.format,
        architecture: object.architecture,
        symbols: object.symbols,
        symbols_sorted: object.symbols_sorted,
        sections: object.sections,
        data,
        debug_info: object.debug_info,
        by_address: AddressIndex::default(),
    }))
}

/// Everything [`parse_object`] reads out of the file. It exists only so the borrow of `data`
/// ends before `data` itself is moved into the object.
struct ParsedObject {
    format: BinaryFormat,
    architecture: Architecture,
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
    /// Seeded with the PDB backend where one was opened for its procedures, empty otherwise.
    debug_info: DebugInfoCache,
}

/// What [`open_files_streaming`] has to say as it goes. There is deliberately no *start*:
/// the caller supplied the paths and they are walked in order.
pub enum Progress {
    /// One object, parsed and ready to be read.
    Parsed(Arc<Object>),
    /// Nothing more will come out of this path. Emitted for **every** path the walk reaches,
    /// including one that could not be read and one that yielded nothing at all.
    Finished(PathBuf),
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a plain
/// object file, handing each object to `emit` **as it is parsed** rather than collecting
/// them. Anything that fails to read or parse is silently skipped.
///
/// A callback rather than a channel or an iterator: a channel would make this crate pick a
/// backpressure policy belonging to whoever draws the result, and an iterator would mean
/// self-borrowing the file's bytes across a yield.
///
/// **`emit` answers whether to go on**, which is how work nobody is waiting for stops. The
/// one thing a single answer cannot express is "skip the rest of *this* file but go on to the
/// next", so a multi-file request in which one file is closed goes on parsing that file's
/// remaining members and drops them at the caller.
pub fn open_files_streaming(
    paths: Vec<PathBuf>,
    mut emit: impl FnMut(Progress) -> ControlFlow<()>,
) {
    for path in paths {
        if open_one_file(&path, &mut emit).is_break() {
            return;
        }
    }
}

/// One path's worth of [`open_files_streaming`], split out so `?` on the caller's answer
/// reads as what it is: abandon this file.
fn open_one_file(
    path: &Path,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Ok(file) = fs::read(path) else {
        // Unreadable is still an end: whoever asked for this file is drawing it as pending
        // until told otherwise.
        return emit(Progress::Finished(path.to_path_buf()));
    };

    // One allocation per file, shared by every object parsed out of it and held for as long
    // as they live. Built here rather than where it is used at the bottom, because it is what
    // the members are cut from: the hash it takes is then one pass over the file however many
    // objects come out of it.
    let file = ObjectData::whole_file(Arc::from(file));

    if let Ok(archive) = ArchiveFile::parse(file.bytes()) {
        for member in archive.members() {
            let Ok(member) = member else {
                continue;
            };
            let name = String::from_utf8_lossy(member.name()).into_owned();
            // The same bytes `member.data(..)` would return, addressed as a range into the
            // archive so the member stays reachable without re-scanning the archive.
            let (offset, size) = member.file_range();
            let Some(data) = ObjectData::member(&file, offset, size) else {
                continue;
            };
            if let Some(object) = parse_object(data, name, path.to_path_buf()) {
                emit(Progress::Parsed(object))?;
            }
        }
    }

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
        .into_owned();
    if let Some(object) = parse_object(file, name, path.to_path_buf()) {
        emit(Progress::Parsed(object))?;
    }

    emit(Progress::Finished(path.to_path_buf()))
}

/// [`open_files_streaming`] with the objects collected, for a caller with nowhere to put them
/// one at a time.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();
    open_files_streaming(paths, |progress| {
        if let Progress::Parsed(object) = progress {
            objects.push(object);
        }
        ControlFlow::Continue(())
    });
    objects
}

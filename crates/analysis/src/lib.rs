use object::{
    read::archive::ArchiveFile, CompressionFormat, Object as _, ObjectKind, ObjectSection,
    ObjectSymbol, Relocation, SectionKind, SymbolIndex, SymbolKind,
};
use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    hash::Hasher as _,
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
    sync::Arc,
};
use symbolic_demangle::{Demangle, DemangleOptions};

pub mod disasm;
mod line;

use disasm::Code;

pub use disasm::{Assembly, BranchEdge, Instruction, SpanKind};
pub use line::{DwarfCache, LineInfo, LineRow, Location};
// Re-exported so the viewer needs no `object` dependency of its own.
pub use object::{Architecture, BinaryFormat, SectionIndex};

/// [`Object`] is shared as an `Arc` and read from worker threads; the other three are what
/// a worker is handed and hands back. Asserted here so a field that stops being shared-safe
/// is a compile error in this crate rather than a borrow error in the UI.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Object>();
    assert_send_sync::<Symbol>();
    assert_send_sync::<Assembly>();
    assert_send_sync::<LineInfo>();
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
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,

    /// This object's DWARF, built on the first query and never at parse time. Nothing
    /// constructs it: write `DwarfCache::default()`. See [`Object::line_info`].
    pub dwarf: DwarfCache,
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

    // A sorted list of symbol positions
    pub symbols: Vec<u64>,
}

/// How far [`SymbolData::estimate_size`]'s next-symbol derivation may reach (1 MiB) before
/// it is treated as having said nothing. Not a claim about how long a function can be — five
/// times the largest in any sample here — but the point past which a sparse export table's
/// derivation is certainly describing something else, at megabytes of decoding per redraw.
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
    /// export, an entry point) has no size of its own. See [`extent`](Self::extent).
    pub fn estimate_size(&self) -> Option<u64> {
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

        Some(next.checked_sub(self.address)?.min(MAX_DERIVED_SIZE))
    }

    /// How many bytes of code this symbol is: the **smaller** of DWARF's
    /// `DW_AT_low_pc`/`DW_AT_high_pc` and [`estimate_size`](Self::estimate_size), because
    /// each bounds the other in a case the other gets wrong. The estimate over-reaches into
    /// padding and over a function with no symbol; DWARF over-reaches when two symbols share
    /// one subprogram (an alias, an assembler label, a split cold part), since `high_pc`
    /// describes the *function*.
    ///
    /// A zero estimate is treated as no estimate: two text symbols at one address make the
    /// next-address derivation answer 0.
    pub fn extent(&self, object: &Object) -> Option<u64> {
        let estimate = self.estimate_size().filter(|&size| size != 0);
        match (self.dwarf_extent(object), estimate) {
            (Some(dwarf), Some(estimate)) => Some(dwarf.min(estimate)),
            (dwarf, estimate) => dwarf.or(estimate),
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
        let code = Code::new(
            bytes,
            self.address,
            self.section.as_deref(),
            &object.symbols,
        );
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

/// The longest mangled name this crate will hand to a demangler.
///
/// **A demangler's recursion depth is the file's to choose**, and a stack overflow is an
/// **abort** no `catch_unwind` turns back into "this symbol has no demangled name", so it has
/// to be headed off before the call. `msvc-demangler` 0.11 has no recursion limit at all
/// (one level per `P` byte) and `cpp_demangle`'s is deep enough that reaching it is megabytes
/// of stack. Measured at roughly 10 KiB of stack per byte of name, so this and
/// [`DEMANGLE_STACK`] are one bound. The longest name in any sample in the repo is 1038
/// bytes; a name past the cap is displayed exactly as the file wrote it.
const MAX_MANGLED_NAME: usize = 2048;

/// The stack [`demangled`] runs on; see [`MAX_MANGLED_NAME`]. A *reservation*: pages are
/// committed only as they are touched.
const DEMANGLE_STACK: usize = 64 << 20;

/// A name short enough to demangle on the caller's own stack: measured, the worst-case
/// 64-byte name demangles inside 1 MiB while a 96-byte one overflows it. Below that line the
/// thread is pure cost (~300 µs of create-and-join).
const SHORT_MANGLED_NAME: usize = 64;

/// Demangle one object's symbol names on a stack big enough for the deepest name among them
/// — the caller's own where they are all short ([`SHORT_MANGLED_NAME`]), a thread of this
/// crate's otherwise.
///
/// [`None`] in means a name with nothing to demangle (the entry point's); [`None`] out means
/// no demangler recognised the name, it was longer than the cap, or the demangler panicked.
///
/// The `catch_unwind` is not general defensiveness: a scoped thread that panics makes
/// `std::thread::scope` panic when the scope ends however the handle was joined, so without
/// it one bad name would take out the parse.
fn demangled(names: &[Option<&str>]) -> Vec<Option<String>> {
    let work = || -> Vec<Option<String>> {
        names
            .iter()
            .map(|name| {
                let name = (*name).filter(|name| name.len() <= MAX_MANGLED_NAME)?;
                std::panic::catch_unwind(|| {
                    symbolic_common::Name::from(name).demangle(DemangleOptions::complete())
                })
                .ok()
                .flatten()
            })
            .collect()
    };

    // The deepest any of them can recurse is the longest of them.
    let deepest = names.iter().flatten().map(|name| name.len()).max();
    match deepest {
        None | Some(0) => return vec![None; names.len()],
        Some(deepest) if deepest <= SHORT_MANGLED_NAME => return work(),
        Some(_) => {}
    }

    // A thread that will not start is one more reason for a name to stay as it was written.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DEMANGLE_STACK)
            .spawn_scoped(scope, work)
            .ok()
            .and_then(|handle| handle.join().ok())
    })
    .unwrap_or_else(|| vec![None; names.len()])
}

/// One symbol as the file states it, held while the whole object's names are demangled in
/// one batch ([`demangled`]).
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
    /// [`None`] for the entry point, which is an address the image names no name for.
    name: Option<String>,
    address: u64,
    /// What the declaration itself said, which is 0 for everything but a dynamic symbol.
    /// Kept for display only; the extent used comes from [`SymbolData::extent`].
    size: u64,
    /// The code section containing `address` — an export table and an entry point name an
    /// address and nothing else.
    section: SectionIndex,
}

/// The name given to the entry point, which is the one declaration that carries none. The
/// angle brackets are the point: no assembler, linker or mangling scheme produces them, so
/// this cannot collide with a name that was in the file.
const ENTRY_POINT_NAME: &str = "<entry point>";

/// The code a file declares outside its symbol table: its **entry point**, its **exports**
/// and its ELF `.dynsym`. A stripped shared library is otherwise a file with nothing in it.
/// Every address here is one the file states outright, so the "nothing is scanned for" rule
/// still holds.
///
/// Three decisions the caller depends on:
///
/// **Only in a code section.** An address is looked up in the kept [`SectionKind::Text`]
/// sections and that section becomes the symbol's own; it doubles as the filter keeping
/// exported *data* out.
///
/// **One symbol per address, earliest source winning** (symbol table > dynamic symbol >
/// export > entry point). Not cosmetic: `Section::symbols` is the sorted list
/// [`SymbolData::estimate_size`] binary-searches, and a repeated address makes it answer 0.
///
/// **Nothing for a relocatable object.** `entry()` answers 0 for an `.o`, and 0 there is a
/// real function's first byte.
fn declared_code(
    file: &object::File<'_>,
    sections: &HashMap<SectionIndex, Section>,
    known: &mut HashSet<u64>,
) -> Vec<DeclaredCode> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    // The address ranges code can be in. Only sections that were kept: one whose bytes would
    // not decompress has nothing to disassemble either.
    let code: Vec<(Range<u64>, SectionIndex)> = file
        .sections()
        .filter(|section| section.kind() == SectionKind::Text)
        .filter_map(|section| {
            let kept = sections.get(&section.index())?;
            let length: u64 = kept.data.len().try_into().ok()?;
            let end = kept.address.checked_add(length)?;
            (length > 0).then_some((kept.address..end, section.index()))
        })
        .collect();

    let mut take = |name: Option<String>, address: u64, size: u64| {
        let Some((_, section)) = code.iter().find(|(range, _)| range.contains(&address)) else {
            return;
        };
        if !known.insert(address) {
            return;
        }
        declared.push(DeclaredCode {
            name,
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
            Some(String::from_utf8_lossy(name).into_owned()),
            symbol.address(),
            symbol.size(),
        );
    }

    for export in file.exports().unwrap_or_default() {
        if export.name().is_empty() {
            continue;
        }
        take(
            Some(String::from_utf8_lossy(export.name()).into_owned()),
            export.address(),
            0,
        );
    }

    // 0 is "this image has no entry point", which is how a DLL built without one states it.
    let entry = file.entry();
    if entry != 0 {
        take(None, entry, 0);
    }

    declared
}

/// Parse `data` as a single object file. `name` is the display name (an archive member name
/// or the file name) and `path` the file it came from. Anything that fails to parse yields
/// [`None`]. `data` is kept in the returned [`Object`]; see [`ObjectData`].
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let object = object::File::parse(data.bytes())
        .map(|file| {
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
                            relocations,
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

            // Declared code goes into the same sorted lists, because that list is what
            // `estimate_size` derives an extent from and a declaration carries none.
            let declared = declared_code(&file, &sections, &mut known);
            for code in &declared {
                if let Some(section) = sections.get_mut(&code.section) {
                    section.symbols.push(code.address);
                }
            }

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    section.symbols.sort_unstable();
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
                    let named = code.name.is_some();
                    pending.push(Pending {
                        index: SymbolIndex(next + offset),
                        name: code.name.unwrap_or_else(|| ENTRY_POINT_NAME.to_owned()),
                        // An export's name is the file's, and on a Windows DLL very often
                        // MSVC-mangled; the entry point's is ours.
                        mangled: named,
                        section: section_map.get(&code.section).cloned(),
                        address: code.address,
                        size: code.size,
                    });
                }
            }

            // One batch for the whole object, on a stack of its own; see `demangled`.
            let names: Vec<Option<&str>> = pending
                .iter()
                .map(|symbol| symbol.mangled.then_some(symbol.name.as_str()))
                .collect();
            let demangled = demangled(&names);
            drop(names);

            let symbols: HashMap<_, _> = pending
                .into_iter()
                .zip(demangled)
                .map(|(symbol, demangled)| {
                    (
                        symbol.index,
                        Arc::new(SymbolData {
                            name: symbol.name,
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
        dwarf: DwarfCache::default(),
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

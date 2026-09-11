//! The data model: an [`Object`], its [`Section`]s and its symbols, and the bytes it was
//! parsed from. Built by [`parse_object`](crate::parse_object) and read by everything else.

use crate::disasm::Code;
use crate::{Assembly, DebugInfoCache, ExtentCache};
use object::{Architecture, BinaryFormat, Relocation, SectionIndex, SymbolIndex};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    hash::{Hash, Hasher},
    ops::Range,
    path::PathBuf,
    sync::{Arc, OnceLock},
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
    /// The same symbols **sorted by name**, byte order. The Symbols list draws them in this
    /// order and a saved place is found in it by binary search. [`Object::new`] sorts them.
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,

    /// This object's debug info, built on the first query — except for a PE whose matching
    /// `.pdb` was opened at parse time for the symbols it names, whose backend is seeded
    /// here so it is not opened twice. See [`Object::line_info`].
    pub debug_info: DebugInfoCache,

    /// The code sections' symbols by the address they are **placed** at, built from
    /// `symbols` on first use, so it cannot disagree with them. A symbol left out of
    /// `symbols` has no estimate and no label, and no call is named after it. See
    /// [`PlacedSymbols`].
    pub placed: PlacedSymbols,
}

/// [`Object::placed`]: every symbol inside a code section's bytes
/// ([`SymbolData::code_place`]), as `(placed address, index, symbol)`, sorted by address and
/// then by index. Two names at one address are both kept, side by side in the file's order.
///
/// One index serves the whole object because of the placed layout: a linked image's
/// addresses are real, and each code section of a relocatable object has a place of its own.
/// A section that is not code has no place and would collide, so its symbols are left out.
///
/// What a symbol's extent estimate, a listing's labels, a call's name and the source index
/// all read. Built once per object behind a `OnceLock`, like the debug info, because it
/// sorts every symbol; lazy rather than built at parse because an archive's members are
/// parsed all at once and read one at a time.
#[derive(Default)]
pub struct PlacedSymbols(OnceLock<Vec<(u64, SymbolIndex, Arc<SymbolData>)>>);

impl Object {
    /// An object holding `symbols`, which may come in any order. This is where
    /// [`symbols_sorted`](Self::symbols_sorted) is sorted, and it starts
    /// [`placed`](Self::placed) and `debug_info` empty, each to be built on its first use.
    pub fn new(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
        sections: Vec<Arc<Section>>,
        data: ObjectData,
    ) -> Object {
        let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
        symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Object {
            path,
            name,
            format,
            architecture,
            symbols,
            symbols_sorted,
            sections,
            data,
            debug_info: DebugInfoCache::default(),
            placed: PlacedSymbols::default(),
        }
    }

    /// [`placed`](Self::placed), built on the first ask.
    pub(crate) fn placed_symbols(&self) -> &[(u64, SymbolIndex, Arc<SymbolData>)] {
        self.placed.0.get_or_init(|| {
            let mut placed: Vec<_> = self
                .symbols
                .iter()
                .filter_map(|(&index, symbol)| Some((symbol.code_place()?, index, symbol.clone())))
                .collect();
            // The map's order is the hash seed's; the file's is the symbol index.
            placed.sort_unstable_by_key(|&(address, index, _)| (address, index.0));
            placed
        })
    }

    /// The entries of [`placed`](Self::placed) whose address is inside `range`.
    pub(crate) fn placed_in(&self, range: Range<u64>) -> &[(u64, SymbolIndex, Arc<SymbolData>)] {
        let all = self.placed_symbols();
        let start = all.partition_point(|&(address, ..)| address < range.start);
        let end = all.partition_point(|&(address, ..)| address < range.end);
        &all[start..end.max(start)]
    }

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
        let all = self.placed_symbols();
        let start = all.partition_point(|&(address, ..)| address < placed);
        let end = all.partition_point(|&(address, ..)| address <= placed);
        all[start..end.max(start)]
            .iter()
            .map(|(_, _, symbol)| symbol)
            .min_by(|a, b| a.name.cmp(&b.name))
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
/// keeps decompressed bytes only for the code sections, and whatever reads another one --
/// the line info, the unwind tables -- reads it out of this.
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
/// [`open_files`](crate::open_files) shares one allocation per file instead.
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

    /// The section's bytes, decompressed. [`None`] for every section that is not
    /// [`code`](Self::code): a debug section is read out of the file when a line question
    /// wants it, so a copy here would be a second one held for the object's life.
    pub data: Option<Vec<u8>>,
    pub address: u64,

    /// The section's relocations by the address the bytes each patches sit at, which is
    /// what a disassembly has to ask by. Not always what the file states: see
    /// [`parse_object`](crate::parse_object). Ordered, because the disassembler, the only
    /// reader, asks for the last one in an instruction's bytes. Empty for a section that is
    /// not [`code`](Self::code).
    pub relocations: BTreeMap<u64, Relocation>,

    /// The address ranges the file's own unwind table states for the functions in this
    /// section — an x86-64 PE's `.pdata`, an ELF's `.eh_frame`, out of
    /// [`unwind::entries`](crate::unwind::entries) — each starting in the section's bytes,
    /// sorted by start, each start once, ends clamped to the bytes. Empty for a file with no
    /// table read. What [`SymbolData::extent`] answers from first.
    pub unwind: Vec<Range<u64>>,

    /// Whether the file marks this section as holding code (`SectionKind::Text`). This is
    /// what a listing of all of it lists, and what decides whether the parse read the
    /// section's [`data`](Self::data) and [`relocations`](Self::relocations) at all.
    /// [`text`](Self::text) makes a section that does, and [`other`](Self::other) one that
    /// does not.
    pub code: bool,

    /// Where the object's layout puts this section: what is added to an address in it to
    /// place it in the one address space every section of the object shares. 0 for every
    /// section of a linked image, whose addresses are real, and for a section that is not
    /// code; in a relocatable object, where every code section starts at 0, an address of
    /// its own for each. See [`section_biases`](crate::parse::section_biases).
    pub bias: u64,
}

impl Section {
    /// A section holding code: its bytes, decompressed, the address they start at, the
    /// relocations in them by address, and its [`bias`](Self::bias). No unwind ranges.
    pub fn text(
        index: SectionIndex,
        name: String,
        data: Vec<u8>,
        address: u64,
        relocations: BTreeMap<u64, Relocation>,
        bias: u64,
    ) -> Section {
        Section {
            index,
            name,
            data: Some(data),
            address,
            relocations,
            unwind: Vec::new(),
            code: true,
            bias,
        }
    }

    /// A section holding no code: no bytes, no relocations, no unwind ranges and no bias.
    pub fn other(index: SectionIndex, name: String, address: u64) -> Section {
        Section {
            index,
            name,
            data: None,
            address,
            relocations: BTreeMap::new(),
            unwind: Vec::new(),
            code: false,
            bias: 0,
        }
    }

    /// This section with `unwind` as its [`unwind`](Self::unwind) ranges, made to hold what
    /// that field says: a range not starting in the bytes is dropped, the rest have their
    /// ends clamped to the bytes, and they are sorted by start with each start kept once.
    pub(crate) fn with_unwind(mut self, mut unwind: Vec<Range<u64>>) -> Section {
        let bytes = self.data.as_ref().and_then(|data| {
            let length: u64 = data.len().try_into().ok()?;
            Some(self.address..self.address.checked_add(length)?)
        });
        match bytes {
            Some(bytes) => {
                unwind.retain(|range| bytes.contains(&range.start));
                for range in &mut unwind {
                    range.end = range.end.min(bytes.end);
                }
            }
            None => unwind.clear(),
        }
        // By start, and each start once: a table stating one function twice is one
        // function, and the search over them assumes it.
        unwind.sort_unstable_by_key(|range| range.start);
        unwind.dedup_by_key(|range| range.start);
        self.unwind = unwind;
        self
    }

    /// The placed addresses this section's bytes take up, cut short where the address space
    /// ends. [`None`] for a section that is not [`code`](Self::code), which has no place, and
    /// for one whose place would be past the end of the address space.
    pub(crate) fn placed_range(&self) -> Option<Range<u64>> {
        if !self.code {
            return None;
        }
        let start = self.address.checked_add(self.bias)?;
        let length = self.data.as_ref().map_or(0, Vec::len);
        let length: u64 = length.try_into().unwrap_or(u64::MAX);
        Some(start..start.saturating_add(length))
    }
}

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    pub address: u64,
    pub section: Option<Arc<Section>>,
    pub size: u64,

    /// What [`extent`](Self::extent) answered, once it has been asked; empty until then.
    pub extent: ExtentCache,
}

impl SymbolData {
    /// A symbol as the file states it. Its [`extent`](Self::extent) is worked out on the
    /// first ask.
    pub fn new(
        name: String,
        demangled: Option<String>,
        address: u64,
        section: Option<Arc<Section>>,
        size: u64,
    ) -> SymbolData {
        SymbolData {
            name,
            demangled,
            address,
            section,
            size,
            extent: ExtentCache::default(),
        }
    }

    /// What to call this symbol on screen. The disassembler substitutes this for a relocated
    /// operand, so anything rendering a relocation target has to use the same rule.
    pub fn display(&self) -> &str {
        self.demangled.as_deref().unwrap_or(&self.name)
    }

    /// `address`, one of this symbol's own, in the one address space every section of the
    /// object shares: [`Section::bias`] added, and nothing added for a symbol in no section,
    /// which is in no listing either. That is the space a listing of all the object's code
    /// draws in and the space `symbol_at` answers in, so anything naming a row has to place
    /// an address the same way.
    ///
    /// `wrapping_add` and not `checked_add`, as `line::relocate` adds the same bias:
    /// agreeing with it matters more than an overflow the biases cannot produce, the layout
    /// starting above the highest address the file states. Wrapping is also what keeps this
    /// from panicking on an address a file made up.
    pub fn placed(&self, address: u64) -> u64 {
        let bias = self.section.as_ref().map_or(0, |section| section.bias);
        address.wrapping_add(bias)
    }

    /// Where this symbol is in [`Object::placed`]: its placed address, where its section is
    /// code and the address is inside the section's bytes. [`None`] for every other symbol,
    /// which no listing labels and no estimate is made for.
    pub(crate) fn code_place(&self) -> Option<u64> {
        let range = self.section.as_ref()?.placed_range()?;
        let placed = self.placed(self.address);
        range.contains(&placed).then_some(placed)
    }

    /// This symbol's bytes over [`extent`](Self::extent) — the same range
    /// [`assembly`](Self::assembly) decodes and [`line_info`](Self::line_info) asks about.
    pub fn data_in(&self, object: &Object) -> Option<&[u8]> {
        self.bytes(self.extent(object)?.bytes)
    }

    /// `size` bytes of the section starting at this symbol, or [`None`] when that runs off
    /// the end of what was decompressed.
    fn bytes(&self, size: u64) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = size.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.as_ref()?.get(offset..end)
    }

    /// This symbol's disassembly, or [`None`] when there are no bytes to decode. An
    /// architecture no backend claims comes back as an [`Assembly`] whose
    /// [`undecodable`](Assembly::undecodable) names it.
    ///
    /// The answer **carries the range it was decoded over** and the [`Extent`](crate::Extent)
    /// behind it ([`Assembly::range`]), which is the one place that decision is made for a
    /// symbol the reader is looking at: the line info is asked over that range and the bar
    /// prints its length, neither of them asking [`extent`](Self::extent) again.
    pub fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let extent = self.extent(object)?;
        let bytes = self.bytes(extent.bytes)?;
        // The sum `extent` has already checked. Checked again rather than assumed: every
        // number here came out of a file.
        let end = self.address.checked_add(extent.bytes)?;
        let code = Code::new(bytes, self.address, self.section.as_deref(), object);
        Some(Arc::new(Assembly::decode(
            object.architecture,
            &code,
            self.address..end,
            extent,
        )))
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

impl Eq for Symbol {}

/// The two pointers the equality above compares, so a map keyed by a symbol takes that
/// identity from here rather than spelling it out again.
impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.object).hash(state);
        Arc::as_ptr(&self.data).hash(state);
    }
}

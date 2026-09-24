//! The data model: an [`Object`], its [`Section`]s and its symbols, the bytes it was parsed
//! from, and the [`LoadMessage`]s saying what went wrong while it was read. Built by
//! [`parse_object`](crate::parse_object) and read by everything else.
//! Also [`covering`], the one search the crate looks an address up in a sorted list of
//! ranges with.

use crate::disasm::Code;
use crate::extent::ExtentCache;
use crate::line::{DebugInfo, DebugInfoCache};
use crate::{Assembly, Bias, MadeUp, PlacedAddress, SectionAddress};
use object::{Architecture, BinaryFormat, Endianness, Relocation, SectionIndex, SymbolIndex};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    hash::{Hash, Hasher},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    pub format: BinaryFormat,

    /// The machine the code in here is for, as the file's own header declares it. This is
    /// what picks a disassembler ([`SymbolData::assembly`]) and the only thing that can: a
    /// symbol's bytes say nothing about how to read themselves.
    pub architecture: Architecture,
    /// The byte order the file stores its values in, as its header declares it: what a
    /// run of bytes no instruction claims is read as words in. `Architecture` does not
    /// say it, since MIPS, PowerPC and ARM each come in both. Little for an object made
    /// by [`Object::new`].
    pub endianness: Endianness,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    /// The same symbols **sorted by name**, byte order, and one name's by index, the file's
    /// order. The Symbols list draws them in this order and a saved place is found in it by
    /// [`Object::symbols_named`]. [`Object::new`] sorts them.
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    /// The functions the file calls and does not define, in its symbol table's order and
    /// then its dynamic one's. They have no code here, so they are not among `symbols`: not
    /// a row in the Symbols list, and a relocation against one names nothing
    /// ([`Operand::Placeholder`](crate::Operand::Placeholder)).
    pub imports: Vec<Import>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,
    /// What went wrong while the object was read, in the order it was found. Empty for a
    /// file that read cleanly. See [`LoadMessage`].
    pub messages: Vec<LoadMessage>,

    /// This object's debug info, built on the first query — except for a PE whose matching
    /// `.pdb` was opened at parse time for the symbols it names, whose backend is seeded
    /// here so it is not opened twice. See [`Object::line_info`].
    pub(crate) debug_info: DebugInfoCache,

    /// The code sections' symbols by the address they are **placed** at, built from
    /// `symbols` by [`Object::new`], so it cannot disagree with them. A symbol left out of
    /// `symbols` has no estimate and no label, and no call is named after it. See
    /// `PlacedSymbols`.
    pub(crate) placed: PlacedSymbols,
}

/// Something that went wrong while an object was read, one variant per problem with the
/// data it names. The object is still shown; this says what in it cannot be trusted. How bad
/// it is comes from the variant ([`LoadMessage::severity`]), and so does what the reader is
/// told (its [`Display`](fmt::Display)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadMessage {
    /// The code sections could not all be placed apart, because `section` states `address`,
    /// the highest any code section states, near the top of the address space.
    CodeSectionsOverlap { section: String, address: u64 },
    /// `count` functions or entry points were left out because the descriptor naming their
    /// code could not be read.
    UnreadableDescriptors { count: usize },
    /// `count` sections are called `<section N>` by their index, because their names could
    /// not be read. They are kept, code and all.
    UnreadableSectionNames { count: usize },
    /// An archive's members stopped at the `member`th (from 1), whose header would not
    /// read. Said on the last object shown before it.
    ArchiveCutShort { member: usize },
}

/// How bad a [`LoadMessage`] is. Ordered, so the worst of several is their `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Something odd that leaves what is shown correct.
    Warning,
    /// Something that makes part of what is shown wrong.
    Error,
}

impl LoadMessage {
    /// How bad this is, which only the variant decides.
    pub fn severity(&self) -> Severity {
        match self {
            LoadMessage::CodeSectionsOverlap { .. } => Severity::Error,
            LoadMessage::UnreadableDescriptors { .. } => Severity::Warning,
            // Only the name is wrong, and it says so.
            LoadMessage::UnreadableSectionNames { .. } => Severity::Warning,
            // What is shown is right; only some of it is missing.
            LoadMessage::ArchiveCutShort { .. } => Severity::Warning,
        }
    }
}

/// What the reader is told: one or two plain sentences.
impl fmt::Display for LoadMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadMessage::CodeSectionsOverlap { section, address } => write!(
                f,
                "The code sections could not be placed apart: section `{section}` states the \
                 address {address:#x}, near the top of the address space, so addresses in this \
                 object overlap."
            ),
            LoadMessage::UnreadableDescriptors { count } => write!(
                f,
                "Functions left out because their descriptors could not be read: {count}."
            ),
            LoadMessage::UnreadableSectionNames { count } => write!(
                f,
                "Sections named by their index because their names could not be read: {count}."
            ),
            LoadMessage::ArchiveCutShort { member } => write!(
                f,
                "The archive's member {member} would not read, so it and every member after it \
                 are not shown."
            ),
        }
    }
}

/// [`Object::placed`]: every symbol inside a code section's bytes
/// ([`SymbolData::code_place`]), one [`PlacedSymbol`] each, sorted by address and then by
/// index. Two names at one address are both kept, side by side in the file's order.
///
/// One index serves the whole object because of the placed layout: a linked image's
/// addresses are real, and each code section of a relocatable object has a place of its own.
/// A section that is not code has no place and would collide, so its symbols are left out.
///
/// What a symbol's extent estimate, a listing's labels, a call's name and the source index
/// all read. Built at parse, off the UI thread, because a render asks it too: the history
/// buttons name a saved place with [`Object::symbol_at_placed`], so every ask has to be a
/// binary search and never the sort over every symbol.
pub(crate) struct PlacedSymbols(Vec<PlacedSymbol>);

/// One entry of [`PlacedSymbols`]: a symbol, the index the file names it by, and the address
/// its code is placed at.
pub(crate) struct PlacedSymbol {
    pub(crate) placed: PlacedAddress,
    pub(crate) index: SymbolIndex,
    pub(crate) symbol: Arc<SymbolData>,
}

impl Object {
    /// An object holding `symbols`, which may come in any order, and no imports. This is
    /// where [`symbols_sorted`](Self::symbols_sorted) and [`placed`](Self::placed) are
    /// sorted, and it starts `debug_info` empty, to be built on its first use.
    pub fn new(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
        sections: Vec<Arc<Section>>,
        data: ObjectData,
    ) -> Object {
        Object::preloaded(
            path,
            name,
            format,
            architecture,
            symbols,
            Vec::new(),
            sections,
            data,
            None,
        )
    }

    /// [`new`](Self::new) with `imports`, and with `debug_info` started on `preloaded`, the
    /// backend the parse already built; [`None`] means nothing is loaded yet, and the first
    /// line question loads it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preloaded(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
        imports: Vec<Import>,
        sections: Vec<Arc<Section>>,
        data: ObjectData,
        preloaded: Option<DebugInfo>,
    ) -> Object {
        let mut sorted: Vec<_> = symbols.iter().collect();
        // The map's order is the hash seed's; the file's is the symbol index.
        sorted.sort_unstable_by(|(a_index, a), (b_index, b)| {
            a.name.cmp(&b.name).then(a_index.0.cmp(&b_index.0))
        });
        let symbols_sorted = sorted
            .into_iter()
            .map(|(_, symbol)| symbol.clone())
            .collect();
        let mut placed: Vec<_> = symbols
            .iter()
            .filter_map(|(&index, symbol)| {
                Some(PlacedSymbol {
                    placed: symbol.code_place()?,
                    index,
                    symbol: symbol.clone(),
                })
            })
            .collect();
        // The map's order is the hash seed's; the file's is the symbol index.
        placed.sort_unstable_by_key(|entry| (entry.placed, entry.index.0));
        Object {
            path,
            name,
            format,
            architecture,
            endianness: Endianness::Little,
            symbols,
            symbols_sorted,
            imports,
            sections,
            data,
            messages: Vec::new(),
            debug_info: DebugInfoCache::new(preloaded),
            placed: PlacedSymbols(placed),
        }
    }

    /// The symbols named exactly `name`, in the file's index order; empty where none is.
    ///
    /// Two binary searches over [`symbols_sorted`](Self::symbols_sorted). It depends on
    /// that field's order, so it lives beside the sort that makes it rather than in the
    /// app that asks: a saved place finds its symbol this way.
    pub fn symbols_named(&self, name: &str) -> &[Arc<SymbolData>] {
        let all = &self.symbols_sorted;
        let start = all.partition_point(|data| data.name.as_str() < name);
        let end = all.partition_point(|data| data.name.as_str() <= name);
        &all[start..end.max(start)]
    }

    /// The worst of [`messages`](Self::messages), or [`None`] where there are none.
    pub fn worst(&self) -> Option<Severity> {
        self.messages.iter().map(LoadMessage::severity).max()
    }

    /// [`placed`](Self::placed).
    pub(crate) fn placed_symbols(&self) -> &[PlacedSymbol] {
        &self.placed.0
    }

    /// The entries of [`placed`](Self::placed) whose address is inside `range`.
    pub(crate) fn placed_in(&self, range: Range<PlacedAddress>) -> &[PlacedSymbol] {
        let all = self.placed_symbols();
        let start = all.partition_point(|entry| entry.placed < range.start);
        let end = all.partition_point(|entry| entry.placed < range.end);
        &all[start..end.max(start)]
    }

    /// The text symbol that **starts** at `placed`, in the one address space every section
    /// of this object shares ([`Section::bias`]); [`None`] where no symbol does. Two names
    /// for one address answer the first by name — the order `symbols_sorted` holds — so the
    /// answer is the same however the map behind them was iterated.
    ///
    /// **Named for the space it answers in**, as `Code::symbol_at_local` is for its own:
    /// the address alone is only a key with the bias in it, and in a relocatable object
    /// every code section starts at 0. A caller holding an address in a section's own terms
    /// adds the section's bias first, which is all `Code::symbol_at_local` does, and one
    /// that knows which section the address is in checks the answer is in it too — the bias
    /// makes two sections two places, but a number past one section's end is still just a
    /// number.
    pub fn symbol_at_placed(&self, placed: PlacedAddress) -> Option<&Arc<SymbolData>> {
        let all = self.placed_symbols();
        let start = all.partition_point(|entry| entry.placed < placed);
        let end = all.partition_point(|entry| entry.placed <= placed);
        all[start..end.max(start)]
            .iter()
            .map(|entry| &entry.symbol)
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
    pub address: SectionAddress,

    /// What the parse read of this section, and the one thing that says whether it holds
    /// code: [`Some`] for a section the file marks as code (`SectionKind::Text`) and whose
    /// bytes decompressed, [`None`] for every other. Read through [`code`](Self::code).
    ///
    /// Only a code section's bytes are kept: a debug section is read out of the file when a
    /// line question wants it, so a copy here would be a second one held for the object's
    /// life.
    code: Option<CodeSection>,
}

/// What a section holding code has and no other section does. Reached through
/// [`Section::code`], which is [`Some`] exactly for those.
#[derive(Debug)]
pub struct CodeSection {
    /// The section's bytes, decompressed.
    pub data: Vec<u8>,

    /// The section's relocations by the address the bytes each patches sit at, which is
    /// what a disassembly has to ask by. Not always what the file states: see
    /// [`parse_object`](crate::parse_object). Every one at an address is kept, in the
    /// file's order. Ordered, because the disassembler, the only reader, asks for every
    /// one in an instruction's bytes.
    pub relocations: BTreeMap<SectionAddress, Vec<Relocation>>,

    /// The address ranges the file's own unwind table states for the functions in this
    /// section — an x86-64 PE's `.pdata`, an ELF's `.eh_frame`, out of
    /// [`unwind::entries`](crate::unwind::entries) — each starting in the section's bytes,
    /// sorted by start, each start once, ends clamped to the bytes. Empty for a file with no
    /// table read. What [`SymbolData::extent`] answers from first.
    pub unwind: Vec<Range<SectionAddress>>,

    /// Where the object's layout puts this section: what is added to an address in it to
    /// place it in the one address space every section of the object shares.
    /// [`Bias::NONE`] for every section of a linked image, whose addresses are real; in a
    /// relocatable object, where every code section starts at 0, an address of its own for
    /// each. See [`section_biases`](crate::sections::section_biases).
    pub bias: Bias,
}

impl Section {
    /// A section holding code: its bytes, decompressed, the address they start at, the
    /// relocations in them by address, and its [`bias`](CodeSection::bias). No unwind
    /// ranges.
    pub fn text(
        index: SectionIndex,
        name: String,
        data: Vec<u8>,
        address: SectionAddress,
        relocations: BTreeMap<SectionAddress, Vec<Relocation>>,
        bias: Bias,
    ) -> Section {
        Section {
            index,
            name,
            address,
            code: Some(CodeSection {
                data,
                relocations,
                unwind: Vec::new(),
                bias,
            }),
        }
    }

    /// A section holding no code: no bytes, no relocations, no unwind ranges and no bias.
    pub fn other(index: SectionIndex, name: String, address: SectionAddress) -> Section {
        Section {
            index,
            name,
            address,
            code: None,
        }
    }

    /// What this section holds as code, or [`None`] where it holds none.
    pub fn code(&self) -> Option<&CodeSection> {
        self.code.as_ref()
    }

    /// This section's [`bias`](CodeSection::bias), and [`Bias::NONE`] for a section holding
    /// no code, which has no place in the layout.
    pub fn bias(&self) -> Bias {
        self.code.as_ref().map_or(Bias::NONE, |code| code.bias)
    }

    /// `address`, one of this section's own, in the one address space every section of the
    /// object shares: this section's [`bias`](Self::bias) added. A section holding no code
    /// has no place in the layout, so it answers the same number in the other space. That is the space a
    /// listing of all the object's code draws in and the space
    /// [`Object::symbol_at_placed`] answers in, so anything naming a row places an address
    /// through here.
    ///
    /// Wrapping, and why, is [`SectionAddress::placed`].
    pub fn place(&self, address: SectionAddress) -> PlacedAddress {
        address.placed(self.bias())
    }

    /// [`place`](Self::place) with the overflow said, for a caller that must answer nothing
    /// rather than answer about a different address ([`SectionAddress::placed_checked`]).
    pub(crate) fn place_checked(&self, address: SectionAddress) -> Option<PlacedAddress> {
        address.placed_checked(self.bias())
    }

    /// [`place`](Self::place) saturating, for the ends of a query: an absurd range then asks
    /// about less than it meant to instead of about something else
    /// ([`SectionAddress::placed_saturating`]).
    pub(crate) fn place_saturating(&self, address: SectionAddress) -> PlacedAddress {
        address.placed_saturating(self.bias())
    }

    /// A placed address back in this section's own terms: [`place`](Self::place) undone,
    /// and wrapping for the same reason.
    pub fn local(&self, placed: PlacedAddress) -> SectionAddress {
        placed.local(self.bias())
    }

    /// How many bytes of code this section holds: 0 for one holding none.
    pub(crate) fn len(&self) -> u64 {
        let length = self.code.as_ref().map_or(0, |code| code.data.len());
        // A `usize` is no wider than a `u64` anywhere this builds, so the fallback never
        // answers; it is here so the conversion is not an unwrap.
        length.try_into().unwrap_or(u64::MAX)
    }

    /// Where this section's bytes stop, in the section's own addresses. [`None`] where they
    /// would run past the end of the address space.
    ///
    /// **Checked, and nothing in the crate answers it another way.** A section that does
    /// not fit in the address space names bytes at addresses that do not exist, so it has
    /// no extent, no listing and no place, rather than one of each cut short at
    /// [`u64::MAX`]. The range a symbol is decoded over, the one a listing partitions, the
    /// one an unwind entry is clamped to and the one a declared address is looked up in are
    /// this range, so they cannot say different things.
    pub(crate) fn end(&self) -> Option<SectionAddress> {
        self.address.checked_add(self.len())
    }

    /// The addresses this section's bytes take up, in the section's own terms. [`None`] for
    /// a section with no bytes — one holding no code among them — and for one that does not
    /// fit in the address space ([`end`](Self::end)).
    pub(crate) fn bytes_range(&self) -> Option<Range<SectionAddress>> {
        let end = self.end()?;
        (self.address < end).then_some(self.address..end)
    }

    /// This section with `unwind` as its code's [`unwind`](CodeSection::unwind) ranges, made
    /// to hold what that field says: a range not starting in the bytes is dropped, the rest
    /// have their ends clamped to the bytes, and they are sorted by start with each start
    /// kept once. A section holding no code takes none.
    pub(crate) fn with_unwind(mut self, mut unwind: Vec<Range<SectionAddress>>) -> Section {
        let bytes = self.bytes_range();
        let Some(code) = self.code.as_mut() else {
            return self;
        };
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
        code.unwind = unwind;
        self
    }

    /// The bytes at `range`, which is in this section's own addresses and not placed ones.
    /// [`None`] where the range is not wholly inside the bytes that were kept — a section
    /// holding no code, a range starting before its address, or one running off its end —
    /// and for a range whose end is before its start.
    ///
    /// Every step is checked, these numbers having come out of a file, and this is the one
    /// place a caller slicing a symbol's code or a gap goes through.
    pub fn bytes_in(&self, range: Range<SectionAddress>) -> Option<&[u8]> {
        let length: usize = range.start.bytes_to(range.end)?.try_into().ok()?;
        let offset: usize = self.address.bytes_to(range.start)?.try_into().ok()?;
        let end = offset.checked_add(length)?;
        self.code.as_ref()?.data.get(offset..end)
    }

    /// The same range placed: [`bytes_range`](Self::bytes_range) put through
    /// [`place`](Self::place). [`None`] wherever that answers [`None`] — a section holding
    /// no code has no place either — and where the layout would put the bytes past the end
    /// of the address space, which `section_biases` never does.
    pub(crate) fn placed_range(&self) -> Option<Range<PlacedAddress>> {
        let bytes = self.bytes_range()?;
        Some(self.place_checked(bytes.start)?..self.place_checked(bytes.end)?)
    }
}

/// A function the file calls and does not define: an undefined text symbol. See
/// [`Object::imports`].
#[derive(Debug)]
pub struct Import {
    /// The file's own spelling, not demangled.
    pub name: String,
    /// The address the file states for it, where it states one: a non-PIE executable's ELF
    /// import is at its PLT slot. [`None`] where the file states 0.
    pub address: Option<SectionAddress>,
}

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    /// Which name the app made up, where `name` is one of those and not the file's own.
    pub made_up: Option<MadeUp>,
    pub address: SectionAddress,
    pub section: Option<Arc<Section>>,
    /// The size the file states for the symbol, or [`None`] where it states none: an
    /// export, the entry point, a debug file's public, and a symbol table entry whose size
    /// field is 0. Only an ELF's is a function's length ([`Symbol::extent`]).
    pub size: Option<u64>,

    /// What [`extent`](Self::extent) answered, once it has been asked; empty until then.
    pub(crate) extent: ExtentCache,
}

impl SymbolData {
    /// A symbol as the file states it. Its [`extent`](Self::extent) is worked out on the
    /// first ask.
    pub fn new(
        name: String,
        demangled: Option<String>,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        SymbolData::parsed(name, demangled, None, address, section, size)
    }

    /// A symbol the parse would have named itself, spelled as it spells `made_up`.
    pub fn new_made_up(
        made_up: MadeUp,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        let name = made_up.to_string();
        SymbolData::parsed(name, None, Some(made_up), address, section, size)
    }

    /// [`new`](Self::new) with `made_up` saying which name the parse made up, if it did.
    pub(crate) fn parsed(
        name: String,
        demangled: Option<String>,
        made_up: Option<MadeUp>,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        SymbolData {
            name,
            demangled,
            made_up,
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

    /// `address`, one of this symbol's own, placed by the section it is in
    /// ([`Section::place`]). A symbol in no section is in no listing either, so nothing
    /// placed it ([`SectionAddress::unplaced`]).
    pub fn placed(&self, address: SectionAddress) -> PlacedAddress {
        self.section
            .as_ref()
            .map_or(address.unplaced(), |section| section.place(address))
    }

    /// Where this symbol starts, in the placed space.
    pub fn placed_start(&self) -> PlacedAddress {
        self.placed(self.address)
    }

    /// Where this symbol is in [`Object::placed`]: its placed address, where its section is
    /// code and the address is inside the section's bytes. [`None`] for every other symbol,
    /// which no listing labels and no estimate is made for.
    pub(crate) fn code_place(&self) -> Option<PlacedAddress> {
        self.place_in(&self.section.as_ref()?.placed_range()?)
    }

    /// This symbol's placed address, where `range` — the placed bytes of the section it is
    /// in — covers it. [`code_place`](Self::code_place) is this with the ask for the range,
    /// so a caller holding one already comes here and asks for it once.
    pub(crate) fn place_in(&self, range: &Range<PlacedAddress>) -> Option<PlacedAddress> {
        let placed = self.placed_start();
        range.contains(&placed).then_some(placed)
    }

    /// The addresses this symbol's [`extent`](Self::extent) covers: what
    /// [`data_in`](Self::data_in) slices, [`assembly`](Self::assembly) decodes and
    /// [`line_info`](Self::line_info) asks about. `extent` has already checked the sum;
    /// it is checked again rather than assumed, as every number here came out of a file.
    pub(crate) fn range(&self, object: &Object) -> Option<Range<SectionAddress>> {
        let bytes = self.extent(object)?.bytes;
        Some(self.address..self.address.checked_add(bytes)?)
    }

    /// This symbol's bytes over its [`range`](Self::range), or [`None`] when that runs
    /// off the end of what was decompressed.
    pub fn data_in(&self, object: &Object) -> Option<&[u8]> {
        self.section.as_ref()?.bytes_in(self.range(object)?)
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
        let range = self.range(object)?;
        let bytes = self.section.as_ref()?.bytes_in(range.clone())?;
        let code = Code::new(bytes, self.address, self.section.as_deref(), object);
        Some(Arc::new(Assembly::decode(
            object.architecture,
            &code,
            range,
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

/// Which of `items`, a list sorted by range start, holds `address`: the index of the last
/// one starting at or before it, where that one's range contains it.
///
/// [`None`] in three cases, which is every way an address can miss. `items` is empty, or
/// `address` is below the first start, so there is no candidate at all; or the candidate
/// ends at or before `address`, the gap after a range.
///
/// **Only that one candidate is looked at.** Where ranges nest, an address past an inner
/// range but still inside the outer one answers [`None`] rather than the outer one — this
/// finds the last range starting at or before the address, and nothing else.
pub(crate) fn covering<T, A: Ord>(
    items: &[T],
    range: impl Fn(&T) -> Range<A>,
    address: A,
) -> Option<usize> {
    let index = items
        .partition_point(|item| range(item).start <= address)
        .checked_sub(1)?;
    range(&items[index]).contains(&address).then_some(index)
}

#[cfg(test)]
mod tests;

//! The data model: an [`Object`], its [`Section`]s and its symbols, and the bytes it was
//! parsed from. Built by [`parse_object`](crate::parse_object) and read by everything else.
//! Also [`covering`], the one search the crate looks an address up in a sorted list of
//! ranges with.

use crate::disasm::Code;
use crate::extent::ExtentCache;
use crate::line::{DebugInfo, DebugInfoCache};
use crate::{Assembly, Bias, PlacedAddress, SectionAddress};
use object::{Architecture, BinaryFormat, Relocation, SectionIndex, SymbolIndex};
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
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    /// The same symbols **sorted by name**, byte order, and one name's by index, the file's
    /// order. The Symbols list draws them in this order and a saved place is found in it by
    /// [`Object::symbols_named`]. [`Object::new`] sorts them.
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,

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
    /// An object holding `symbols`, which may come in any order. This is where
    /// [`symbols_sorted`](Self::symbols_sorted) and [`placed`](Self::placed) are sorted,
    /// and it starts `debug_info` empty, to be built on its first use.
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
            sections,
            data,
            None,
        )
    }

    /// [`new`](Self::new) with `debug_info` started on `preloaded`, the backend the parse
    /// already built; [`None`] means nothing is loaded yet, and the first line question
    /// loads it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preloaded(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
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
            symbols,
            symbols_sorted,
            sections,
            data,
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
    /// [`parse_object`](crate::parse_object). Ordered, because the disassembler, the only
    /// reader, asks for the last one in an instruction's bytes.
    pub relocations: BTreeMap<SectionAddress, Relocation>,

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
        relocations: BTreeMap<SectionAddress, Relocation>,
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
    pub fn end(&self) -> Option<SectionAddress> {
        self.address.checked_add(self.len())
    }

    /// The addresses this section's bytes take up, in the section's own terms. [`None`] for
    /// a section with no bytes — one holding no code among them — and for one that does not
    /// fit in the address space ([`end`](Self::end)).
    pub fn bytes_range(&self) -> Option<Range<SectionAddress>> {
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

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    pub address: SectionAddress,
    pub section: Option<Arc<Section>>,
    pub size: u64,

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

    /// `address`, one of this symbol's own, placed by the section it is in
    /// ([`Section::place`]). A symbol in no section is in no listing either, so nothing
    /// placed it ([`SectionAddress::unplaced`]).
    pub fn placed(&self, address: SectionAddress) -> PlacedAddress {
        self.section
            .as_ref()
            .map_or(address.unplaced(), |section| section.place(address))
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
        section.bytes_in(self.address..self.address.checked_add(size)?)
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

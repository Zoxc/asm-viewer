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
// `Section::index` is one of these and `Object::architecture` another, so anything
// building or reading either needs the type; the viewer does not depend on `object`
// itself.
pub use object::{Architecture, BinaryFormat, SectionIndex};

/// `Object` is handed around as an `Arc` and read from worker threads, so everything it
/// holds — the lazily built DWARF context above all, whose `addr2line::Context` is `Send`
/// but not `Sync` on its own — has to be shared-safe. Assert it here rather than find out
/// at a call site.
///
/// The other three are what a worker thread is *handed* and what it hands back: the app
/// analyses a symbol off its UI thread (`src/ui.rs`, `use_analysis`), so a [`Symbol`] has
/// to cross into the worker and an [`Assembly`] and a [`LineInfo`] have to cross back.
/// None of them is anything but plain data and `Arc`s today; asserting it means a field
/// that stops being so is a compile error here rather than a borrow-checker error in the
/// UI, where the fix would be to give up and go back on-thread.
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

    /// The machine the code in here is for, as the file's own header declares it.
    ///
    /// This is what picks a disassembler ([`SymbolData::assembly`]) and the only thing
    /// that can: a symbol's bytes say nothing about how to read themselves, so a decoder
    /// chosen any other way is a decoder that is right by luck. It is on the object
    /// rather than on the symbol because it is a property of the file — every symbol in
    /// one object is for one machine — and `assembly` is handed the object anyway.
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

/// A digest of a whole file's bytes: what tells "the same binary" from "one that has been
/// rebuilt underneath the session that named it" (`src/project.rs`, and `notes/Goals.md`'s
/// "Saves hashes of the binaries"). Nothing in this crate reads one — it is computed here
/// because this is where the bytes already are.
///
/// **Why the content, and not the size and the modification time.** A timestamp is not a
/// property of the bytes, and it is wrong in both directions for the question being asked:
/// a rebuild that lands on identical output still bumps it, so an unchanged binary would
/// come back distrusted, while a file restored by a copy that preserves times, by a
/// checkout, or out of a build cache carries an old one over new bytes — which is exactly
/// the case a saved session must not believe. A length is weaker still; two builds of one
/// crate land on the same one routinely.
///
/// **Why it is affordable.** [`open_files`] has already read the whole file into an
/// `Arc<[u8]>` before anything is parsed, so this is one pass over memory rather than a
/// second read, and it runs on the parse worker thread rather than the UI's. Measured on
/// the repo's own samples, with the file in the page cache: `viewer-sample`, 331 MiB,
/// reads in 334 ms and hashes in 31 ms; `LLVM-24-rust-dev.dll`, 137 MiB, 142 ms and 12 ms;
/// `libanalysis-sample.rlib`, 19 MiB, 20 ms and 1.5 ms. Under a tenth of a read that is
/// happening anyway, and a cold read off a disk is slower while the hash is not.
///
/// **Why xxHash64.** It is already in the tree (`object` -> `ruzstd` -> `twox-hash`), so
/// it costs no new crate, and the algorithm is *specified*: its output is a property of
/// the bytes and not of the build. `std`'s `DefaultHasher` is the tempting free answer and
/// is the wrong one for something written to a file — its documentation reserves the right
/// to change the algorithm between releases, which would quietly declare every saved binary
/// rebuilt after a toolchain upgrade. Non-cryptographic is the right class: this answers
/// "is this the same file", not "is this file trustworthy", and a reader who wants to fool
/// it can point the session at another file anyway. The `crc32fast` also in the tree is a
/// few milliseconds quicker over 331 MiB and half the width; a one-in-four-billion chance
/// of calling a rebuilt binary unchanged is not worth those milliseconds, since missing a
/// rebuild is the whole of what this is for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileDigest(u64);

impl FileDigest {
    /// Hash `bytes`, in one pass.
    pub fn of(bytes: &[u8]) -> FileDigest {
        // Seeded with 0, which is xxHash64's own default: the seed is part of the
        // algorithm's identity here, so it is written down rather than chosen per run.
        let mut hasher = twox_hash::XxHash64::with_seed(0);
        hasher.write(bytes);
        FileDigest(hasher.finish())
    }
}

/// Sixteen lowercase hex digits, which is the form the session writes and the only form
/// anything outside this crate needs: a digest is compared, never counted with.
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

/// The bytes an [`Object`] was parsed from, held for as long as the object lives.
///
/// Parsing keeps decompressed bytes only for the sections that hold text symbols, so an
/// `Object` on its own cannot be asked anything about, say, its `.debug_*` sections after
/// the fact. Keeping the input around lets a later, lazy pass (line info) build what it
/// needs without re-reading the file — and it keeps an archive *member* addressable, which
/// re-reading could not do cheaply: a member is a slice of the archive, so finding it
/// again would mean reading and re-scanning the whole archive.
///
/// The bytes are **shared, not copied**: `open_files` reads each file once and every
/// `Object` it yields from that file — one per archive member *plus* one for the file
/// itself — holds a clone of the same `Arc<[u8]>` and differs only in `range`, which is
/// the extent of *its* object file within it (the whole file for a plain object, the
/// member's slice for an archive member). Copying each member's bytes instead would hold
/// an archive's contents roughly twice over, since the archive file is parsed as a plain
/// object as well.
///
/// **Memory cost:** these are exactly the bytes `open_files` already reads; the only
/// change is that they are now retained for the object's lifetime instead of being dropped
/// when parsing returns. A file costs its own size once however many objects come out of
/// it — 3.5 MiB for the sample `librustc_data_structures-*.rlib`, 137 MiB for the sample
/// `LLVM-24-rust-dev.dll` — and `fs::read` yields a `Vec`, so the conversion to `Arc<[u8]>`
/// copies it once and the peak while opening is briefly twice the file's size. The flip
/// side of sharing is that one live archive member keeps the whole archive's bytes alive,
/// which is the right trade for a viewer that lists every member anyway.
///
/// This allocation is exactly what the `[?]` "Prefer memory mapped files and minimal
/// memory footprint" goal in `notes/Goals.md` would replace: mapping the file instead of
/// reading it turns this into an `Arc` of a mapping, at which point the resident cost is
/// the kernel's page cache and the transient copy disappears too.
#[derive(Clone)]
pub struct ObjectData {
    file: Arc<[u8]>,
    range: Range<usize>,
    /// The digest of the **whole file**, not of `range`.
    ///
    /// The unit a session names is the file — `binaries` is a list of paths, and closing
    /// one closes every object that came out of it — so the file is what a digest has to
    /// answer for. It is also what makes an archive cost one hash rather than one per
    /// member: [`ObjectData::member`] is derived from the file's own `ObjectData` and
    /// copies this, so `libanalysis-sample.rlib`'s 196 members share the single pass
    /// [`ObjectData::whole_file`] made.
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
    /// when that range does not lie inside the file, which is the same bounds check
    /// `ArchiveMember::data` does — such a member is skipped, exactly as before.
    ///
    /// It takes the archive's own [`ObjectData`] rather than its bytes so that the member
    /// inherits the file's [`digest`](ObjectData::digest) instead of prompting a second
    /// pass over the archive: the offsets are into the file, and the file has already been
    /// hashed by the [`whole_file`](ObjectData::whole_file) the caller built to parse the
    /// archive with.
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

    /// The digest of the file this object was parsed out of. See [`FileDigest`], and
    /// note that every object from one file answers the same thing here.
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

/// Copies the bytes into an allocation of their own. Convenient for callers that only
/// have a slice (the tests); [`open_files`] shares one allocation per file instead.
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
    /// The section's index in the file it was parsed from, which is what identifies it
    /// to a later pass that re-reads that file — line info does (see
    /// [`Object::line_info`]), because an address on its own is not a key in a
    /// relocatable object where every section starts at 0.
    pub index: SectionIndex,
    pub name: String,
    pub data: Vec<u8>,
    pub address: u64,

    pub relocations: HashMap<u64, Relocation>,

    // A sorted list of symbol positions
    pub symbols: Vec<u64>,
}

/// How far [`SymbolData::estimate_size`]'s derivation is allowed to reach before it is
/// treated as having said nothing.
///
/// The derivation is "up to the next declaration", which is a tight bound only while the
/// declarations are dense. A symbol table is dense — the largest function in any of the
/// repo's sample objects is 181 KB, and the 99.9th percentile is under 10 KB — but an
/// **export table is not**: `LLVM-24-rust-dev.dll` declares 22 918 functions in 78 MB of
/// `.text`, so the thousands of unexported functions between two exports all land inside
/// the first one's derived extent. Nine of them come out over a megabyte and the worst
/// over three, which is 772 302 instructions to decode and format for a listing whose
/// first screenful is the only part anyone will read.
///
/// 1 MiB is therefore not a claim about how long a function can be; it is the point past
/// which the derivation is certainly describing something other than this function, and
/// past which believing it costs seconds per redraw. It is five times the largest real
/// function measured across every sample in the repo, so nothing with a symbol table
/// notices it at all.
///
/// The honest fix for a stripped PE is more declarations, not a bigger cap: an x86-64
/// image carries a `RUNTIME_FUNCTION` in `.pdata` for every function with unwind info,
/// stating both ends of it. That is `notes/Goals.md`'s separate "Find unwind targets"
/// item; when it lands, the gaps this cap exists for largely stop existing.
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
    /// What to call this symbol on screen: its demangled name where there is one, and
    /// the raw name otherwise. The disassembler substitutes this for a relocated
    /// operand, so anything rendering a relocation target has to use the same rule or
    /// the instruction text and the link over it would disagree.
    pub fn display(&self) -> &str {
        self.demangled.as_deref().unwrap_or(&self.name)
    }

    /// Object files frequently report a size of 0, so derive the extent from the next
    /// symbol in the section (or the section end).
    ///
    /// This is the answer that costs nothing and needs nothing but the object's own
    /// symbol table. It is an *upper* bound rather than a measurement: the bytes between
    /// one function's last instruction and the next function's first are alignment
    /// padding, and a declaration the symbol table never mentioned — an export, an entry
    /// point ([`declared_code`]) — has no size of its own at all. See
    /// [`extent`](Self::extent) for the answer debug info gives when there is any, and
    /// [`MAX_DERIVED_SIZE`] for where the bound stops meaning anything.
    pub fn estimate_size(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        let i = section.symbols.binary_search(&self.address).ok()?;

        // Where the section's bytes stop, which every derivation is clipped to. [`None`]
        // only for a section placed so near the end of the address space that it does not
        // fit in it, which is a file saying something impossible rather than a file this
        // has to have an answer for.
        let end = section
            .data
            .len()
            .try_into()
            .ok()
            .and_then(|length: u64| section.address.checked_add(length));

        // **The next symbol bounds this one; the section bounds them both.** A symbol
        // table is a list of numbers out of the file, and one wild address in it — a
        // corrupt `st_value`, a symbol relocated by nothing — would otherwise be the
        // *previous* symbol's problem: its extent would run to wherever the wild one
        // claims to be, which is past the bytes that were read, and `bytes` would answer
        // [`None`] for a function that is perfectly readable. One unreadable symbol is a
        // row without a listing; it must not cost the row above it one too.
        let next = match section.symbols.get(i + 1) {
            Some(&next) => end.map_or(next, |end| next.min(end)),
            None => end?,
        };

        Some(next.checked_sub(self.address)?.min(MAX_DERIVED_SIZE))
    }

    /// How many bytes of code this symbol is, taking DWARF's word for it where DWARF has
    /// one and falling back on [`estimate_size`](Self::estimate_size) where it does not.
    ///
    /// A `DW_TAG_subprogram` carries `DW_AT_low_pc`/`DW_AT_high_pc`, which is the
    /// compiler stating the function's extent rather than this crate inferring it from
    /// where the *next* function starts. The two differ by the alignment padding between
    /// them — a run of `int3`/`nop` that the estimate hands to the disassembler and DWARF
    /// does not — and they differ by much more wherever a symbol is missing between two
    /// functions, which is the whole reason [`declared_code`] exists.
    ///
    /// **The smaller of the two wins**, rather than DWARF winning outright. Each bounds
    /// the other in a case the other gets wrong, and neither case is exotic:
    ///
    /// * The estimate over-reaches into padding, and into a whole function whenever one
    ///   has no symbol. DWARF is right there.
    /// * DWARF over-reaches when two symbols share one subprogram — an alias, a local
    ///   label the assembler emitted as a text symbol, a cold part split out — because
    ///   `DW_AT_high_pc` describes the *function*, not the symbol that was asked about.
    ///   Running past the next symbol would put another function's instructions in this
    ///   one's listing, which is exactly the confusion the estimate exists to avoid.
    ///
    /// A zero estimate is treated as no estimate: two text symbols at one address (an
    /// alias) make the next-address derivation answer 0, and 0 bytes of code is not an
    /// answer any caller can use.
    ///
    /// **Cost.** [`None`] from the DWARF half is cached per object the same way
    /// [`line_info`](Self::line_info) is, so an object without debug info pays one
    /// section-table scan ever; with debug info it is one DIE walk per compilation unit
    /// visited, and a hash lookup afterwards. See [`Object::subprogram_extent`].
    pub fn extent(&self, object: &Object) -> Option<u64> {
        let estimate = self.estimate_size().filter(|&size| size != 0);
        match (self.dwarf_extent(object), estimate) {
            (Some(dwarf), Some(estimate)) => Some(dwarf.min(estimate)),
            (dwarf, estimate) => dwarf.or(estimate),
        }
    }

    /// This symbol's bytes, as far as [`estimate_size`](Self::estimate_size) reaches.
    ///
    /// Deliberately *not* the debug-info extent: a symbol does not own the file it came
    /// from, so this cannot ask for one. Anything with an [`Object`] in hand should call
    /// [`data_in`](Self::data_in) instead, which is what [`assembly`](Self::assembly)
    /// does. What is left for this one is the caller that only wants a rough size to put
    /// on screen.
    pub fn data(&self) -> Option<&[u8]> {
        self.bytes(self.estimate_size()?)
    }

    /// This symbol's bytes over [`extent`](Self::extent) — the same range
    /// [`assembly`](Self::assembly) decodes and [`line_info`](Self::line_info) asks about.
    pub fn data_in(&self, object: &Object) -> Option<&[u8]> {
        self.bytes(self.extent(object)?)
    }

    /// `size` bytes of the section starting at this symbol, or [`None`] when that runs
    /// off the end of what was decompressed.
    fn bytes(&self, size: u64) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = size.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.get(offset..end)
    }

    /// This symbol's disassembly, or [`None`] when there are no bytes to decode.
    ///
    /// **Which decoder is a property of the object, not of this crate.** The architecture
    /// comes out of the file's own header ([`Object::architecture`]) and picks a backend
    /// through [`disasm::disassembler`]; an architecture no backend claims comes back as
    /// an [`Assembly`] whose [`undecodable`](Assembly::undecodable) names it, which is
    /// the one honest answer for bytes nothing here can read. Decoding them anyway is
    /// what this used to do — the decoder was pinned to 64-bit x86 — and it produced a
    /// full page of plausible, entirely invented instructions for an aarch64 function.
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

/// A symbol together with the object it came from. Identity is `Arc` pointer identity,
/// never name or index, so duplicate symbol names across objects stay distinct.
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

/// A hard ceiling on how large a single section's decompressed bytes may be, whatever
/// its header claims. See [`section_data`].
const MAX_SECTION_DATA: u64 = 1 << 30;

/// Read a section's bytes, decompressing it if it says it is compressed, but only after
/// checking that the size it declares is believable.
///
/// `uncompressed_data()` trusts the size in the compression header and reserves that much
/// *before* it looks at a single compressed byte, so a corrupt or hostile header — one
/// flipped `SHF_COMPRESSED` bit in a 608-byte file is enough — turns into a multi-gigabyte
/// allocation and an OOM abort on a small machine. `compressed_data()` hands us the same
/// information without allocating: the declared `uncompressed_size` plus the compressed
/// bytes themselves, which are a slice of the file and so are already bounded by it.
///
/// Two independent bounds have to hold, and a section failing either is dropped exactly
/// like one whose name or data will not read — no panic, no error, it simply has no data:
///
/// * A ratio bound, which is provable rather than a guess. DEFLATE cannot expand data by
///   more than 1032:1, and a zstd frame not by more than 32768:1 (a 128 KiB block stored
///   as a 4-byte RLE block), so a declared size past that is a lie about *these* bytes no
///   matter what they decode to. As the compressed bytes come from the file, this also
///   caps the allocation at a multiple of the file's own length.
/// * An absolute bound, because the ratio bound alone still scales with the input: a
///   100 MB object could honestly claim 100 GB. 1 GiB is far more than anything this
///   viewer can show — it keeps only sections holding text symbols, and it already holds
///   the whole file in memory besides.
///
/// Both are orders of magnitude above real files: compressed debug sections run at
/// roughly 2:1 to 10:1, so nothing legitimate comes anywhere near either limit.
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
/// **A demangler's recursion depth is the file's to choose.** Every demangler behind
/// `symbolic-demangle` is a recursive-descent parser over the mangled name, and two of
/// them recurse once per *byte* of it: `msvc-demangler` 0.11 has no recursion limit at all
/// (`read_pointee` -> `read_var_type` -> `read_pointee`, one `P` apiece), and while
/// `cpp_demangle` 0.4 does have one, `symbolic` raises it to 160/192 and a frame of that
/// parser is fat enough that reaching the limit is itself several megabytes of stack. A
/// name is bytes out of a string table, so both depths are chosen by whoever wrote the
/// file — and a stack overflow is an **abort**, which no `catch_unwind` can turn back into
/// "this symbol has no demangled name". It has to be headed off before the call.
///
/// Measured in a debug build (which is how this app is run while it is developed, and
/// the profile with the fattest frames): `?f@@YAX` + *n* × `P` + `@Z` overflows a 2 MiB
/// stack — the default for the `std::thread` the viewer parses on — at n ≈ 200, an 8 MiB
/// stack at n ≈ 900 and a 64 MiB stack at n ≈ 6000. So the depth costs roughly 10 KiB of
/// stack per byte of name, and the two constants here are one bound: at most
/// `MAX_MANGLED_NAME` bytes of name, on at least `MAX_MANGLED_NAME` × 10 KiB of stack,
/// with room to spare for a compiler that lays out fatter frames than this one.
///
/// 2048 costs nothing real. The longest symbol name in any sample in the repo is 1038
/// bytes (`LLVM-24-rust-dev.dll`, whose 21 817 MSVC-mangled exports are also the only
/// place MSVC demangling matters here); `viewer-sample`'s longest of 115 577 is 975 and
/// `libanalysis-sample.rlib`'s longest of 4 164 is 806. A name past the cap is not an
/// error — it is displayed as it was written in the file, which is what
/// [`SymbolData::display`] does for every name no demangler recognises anyway.
const MAX_MANGLED_NAME: usize = 2048;

/// The stack [`demangled`] runs on; see [`MAX_MANGLED_NAME`] for where the number comes
/// from. It is a *reservation*: the pages are only committed as they are touched, so the
/// thread costs what a thread costs and not 64 MiB.
const DEMANGLE_STACK: usize = 64 << 20;

/// A name short enough to demangle on the caller's own stack.
///
/// At the ~10 KiB per byte measured for [`MAX_MANGLED_NAME`], 64 bytes of name is under
/// 1 MiB of stack, which is half the 2 MiB a `std::thread` gets by default and an eighth
/// of a main thread's — and it is measured, not extrapolated: the worst-case 64-byte name
/// (`?f@@YAX` + 55 × `P` + `@Z`, and the same shape through `cpp_demangle`) demangles on a
/// 1 MiB stack, while the 96-byte one overflows it. Below that line the thread is pure
/// cost — it is ~300 µs of create-and-join, and every fixture in this crate's tests names
/// its functions `caller` and `target`.
const SHORT_MANGLED_NAME: usize = 64;

/// Demangle one object's symbol names, all of them, on a stack big enough for the
/// deepest name among them — the caller's own where they are all short
/// ([`SHORT_MANGLED_NAME`]), a thread of this crate's otherwise.
///
/// A batch rather than a call per symbol because the thread is the point: spawning one
/// per name would cost more than the demangling does, and the names of one object are
/// known all at once. [`None`] in means a name with nothing to demangle (the entry
/// point's, which is this crate's own invention); [`None`] out means no demangler
/// recognised the name, it was longer than the cap, or the demangler panicked on it.
///
/// The `catch_unwind` is not general defensiveness: a scoped thread that panics makes
/// `std::thread::scope` itself panic when the scope ends, *however* the handle was
/// joined, so without it one bad name would take out the parse instead of taking out
/// its own demangled name. Nothing in this crate's fuzzing has found such a name — 40 000
/// random ones through all four demanglers produced no panic — so it is a net and not a
/// workaround for something known.
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

    // The deepest any of them can recurse is the longest of them. Nothing to demangle at
    // all — an object of unnamed symbols, or one whose only symbol is the entry point —
    // and nothing short enough to be an ordinary stack's business both answer here.
    let deepest = names.iter().flatten().map(|name| name.len()).max();
    match deepest {
        None | Some(0) => return vec![None; names.len()],
        Some(deepest) if deepest <= SHORT_MANGLED_NAME => return work(),
        Some(_) => {}
    }

    // A thread that will not start is one more reason for a name to stay as it was
    // written, exactly like a name the demanglers do not recognise.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DEMANGLE_STACK)
            .spawn_scoped(scope, work)
            .ok()
            .and_then(|handle| handle.join().ok())
    })
    .unwrap_or_else(|| vec![None; names.len()])
}

/// One symbol as the file states it, before its name has been demangled.
///
/// Demangling is one batch per object ([`demangled`]), so every symbol has to be read out
/// of the file before any of them can be finished; this is what is held in between.
struct Pending {
    index: SymbolIndex,
    name: String,
    /// Whether the name is the file's own. The entry point's is not — it is
    /// [`ENTRY_POINT_NAME`], which this crate invented and no demangler has anything to
    /// say about.
    mangled: bool,
    section: Option<Arc<Section>>,
    address: u64,
    size: u64,
}

/// A function the file **declares** somewhere other than its symbol table.
///
/// See [`declared_code`] for where these come from and why they are not a departure from
/// the "only declared functions, nothing is scanned for" rule.
struct DeclaredCode {
    /// [`None`] for the entry point, which is an address the image names no name for.
    name: Option<String>,
    address: u64,
    /// What the declaration itself said the size was, which is 0 for everything but a
    /// dynamic symbol. Kept for display exactly like [`SymbolData::size`]; the extent
    /// actually used comes from [`SymbolData::extent`].
    size: u64,
    /// The code section containing `address`, worked out by [`declared_code`] — an
    /// export table and an entry point name an address and nothing else.
    section: SectionIndex,
}

/// The name given to the entry point, which is the one declaration that carries none.
///
/// The angle brackets are the point: no assembler, linker or mangling scheme produces a
/// name with them, so this cannot be mistaken for something that was in the file and
/// cannot collide with something that was.
const ENTRY_POINT_NAME: &str = "<entry point>";

/// The code a file declares outside its symbol table: its **entry point** and its
/// **exports**.
///
/// A stripped shared library is otherwise a file with nothing in it. `LLVM-24-rust-dev.dll`
/// (a repo sample) has no COFF symbol table at all, so `file.symbols()` is empty and the
/// viewer lists zero functions for 137 MB of code — while the image states in its export
/// directory exactly where several thousand of those functions begin. An ELF `.so` stripped
/// of `.symtab` still has `.dynsym`, which says the same thing.
///
/// This does **not** loosen the standing rule that only declared functions are
/// disassembled and nothing is scanned for (`notes/Goals.md`, *Binary inspection design*).
/// Every address here is one the file states outright, in a table a loader reads:
///
/// * `dynamic_symbols()` — the ELF `.dynsym`, which declares a kind ([`SymbolKind::Text`]
///   is required here) and a size, so it is read like the symbol table it is rather than
///   through `exports()`, which flattens it to name-and-address and keeps the data
///   definitions too.
/// * `exports()` — the PE export directory (and the Mach-O export trie), which is
///   name-and-address only. Forwarders are already dropped by `object`.
/// * `entry()` — the one address the image header names.
///
/// Three decisions, each of which the caller depends on:
///
/// **Only in a code section.** An address is looked up in the sections that were kept and
/// are [`SectionKind::Text`], and that section becomes the symbol's own — an export table
/// gives no section and neither does an entry point, and a symbol with no section has no
/// bytes, no size and no line info. It doubles as the filter that keeps exported *data*
/// out: a PE exports its globals from the same table as its functions and an ELF
/// `.dynsym` definition may be an object, and neither lands in `.text`.
///
/// **One symbol per address, and the earliest source wins.** A file can declare the same
/// function in several of these places at once — the symbol table *and* the export table,
/// an export *and* the entry point — and the order above is the order of how much each
/// says. The symbol table wins over everything (it carries a size and the internal name),
/// a named export wins over the unnamed entry point. This is not cosmetic: `Section::symbols`
/// is a sorted list of addresses that [`SymbolData::estimate_size`] binary-searches, and a
/// repeated address there makes the next-address derivation answer 0.
///
/// **Nothing for a relocatable object.** An `.o` has no exports and no entry point, but
/// `entry()` answers 0 for one all the same — and 0 is a perfectly ordinary address there,
/// the first byte of the first section, so believing it would invent an `<entry point>`
/// symbol on top of a real function in every object file the viewer opens.
fn declared_code(
    file: &object::File<'_>,
    sections: &HashMap<SectionIndex, Section>,
    known: &mut HashSet<u64>,
) -> Vec<DeclaredCode> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    // The address ranges code can be in, as `(range, index)`. Only sections that were
    // kept: one whose bytes would not decompress has nothing to disassemble either.
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

    // 0 is "this image has no entry point", which is how a DLL built without one states
    // it; it is also what a file too damaged to have one reads as.
    let entry = file.entry();
    if entry != 0 {
        take(None, entry, 0);
    }

    declared
}

/// Parse `data` as a single object file. `name` is the display name (an archive member
/// name or the file name) and `path` the file it came from. Anything that fails to
/// parse yields [`None`].
///
/// `data` is kept in the returned [`Object`]; see [`ObjectData`]. A caller with nothing
/// but bytes (the tests) can pass `bytes.into()`, which gives them an allocation of their
/// own; [`open_files`] shares one per file.
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

            // Insert symbol addresses into sections. The addresses are collected as they
            // go, because that set is what tells `declared_code` which of the file's
            // exports are already in the symbol table under their own name.
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

            // What the file declares elsewhere: exports and the entry point. Their
            // addresses go into the sections' sorted lists alongside the symbol table's,
            // because that list is what `SymbolData::estimate_size` derives an extent
            // from and a declaration carries none.
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

            // The declared code joins the same map rather than living in a list beside
            // it, so `symbols_sorted` stays derived from one place and the object's
            // symbol count is the number of functions it can show. The keys are indices
            // *past* the file's own symbol table, which is the only honest thing they can
            // be: these symbols are not in it. Nothing can reach them by relocation —
            // relocation targets are indices the file itself wrote, and a file that
            // declares exports is a linked image whose text sections carry no symbol
            // relocations at all.
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
                        // An export's name is the file's, and on a Windows DLL it is very
                        // often MSVC-mangled; the entry point's is ours.
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

/// Everything [`parse_object`] reads out of the file, i.e. an [`Object`] minus the fields
/// that do not come from parsing. It exists only so the borrow of `data` ends before
/// `data` itself is moved into the object.
struct ParsedObject {
    format: BinaryFormat,
    architecture: Architecture,
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
}

/// What [`open_files_streaming`] has to say as it goes.
///
/// Two variants and not three: a *start* would be the caller telling itself something it
/// already knows, since it supplied the paths and they are walked in order. What it
/// cannot know is when one is done with — a file contributes anything from zero objects
/// to an archive's two hundred — and that is [`Progress::Finished`].
pub enum Progress {
    /// One object, parsed and ready to be read. Every object between two
    /// [`Finished`](Progress::Finished)es came out of the path the second of them names.
    Parsed(Arc<Object>),
    /// Nothing more will come out of this path.
    ///
    /// Emitted for **every** path the walk reaches, including one that could not be read
    /// and one that yielded nothing at all: it is what says "stop waiting", and a caller
    /// drawing a file it has asked for needs that said whether or not anything came of
    /// it.
    Finished(PathBuf),
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a
/// plain object file, handing each object to `emit` **as it is parsed** rather than
/// collecting them. Anything that fails to read or parse is silently skipped.
///
/// This is the shape the app opens binaries in, and the streaming is the whole point of
/// it: a 196-member archive is 196 answers arriving over the couple of hundred
/// milliseconds it takes, so the reader can be reading the first member's symbols while
/// the last one is still being parsed. A `Vec` returned at the end is the same work with
/// the first 195 answers withheld.
///
/// **A callback, not a channel or an iterator.** The crate stays framework-free and this
/// is the least it can know about its caller: a channel would make the crate choose one
/// (and choose bounded or unbounded, which is a backpressure policy belonging to whoever
/// is drawing the result), and an iterator would mean either self-borrowing the file's
/// bytes across a yield or a generator. `open_files` is that same callback closing over a
/// `Vec`.
///
/// **`emit` answers whether to go on**, which is how work nobody is waiting for stops:
/// the app returns [`ControlFlow::Break`] when the reader has closed the file being
/// parsed or left the project it belongs to, and the walk stops where it stands rather
/// than parsing another 300 MB into a value that will be dropped. The one thing a single
/// answer cannot express is "skip the rest of *this* file but go on to the next", so a
/// multi-file request in which one file is closed goes on parsing that file's remaining
/// members and drops them at the caller. That is deliberate: it costs a worker thread
/// some work in the rarest case rather than a third answer every call site has to have an
/// opinion about.
///
/// **The digest is one pass per file**, not per object: [`ObjectData::whole_file`] is
/// built once at the top of each path and every member is cut from it
/// ([`ObjectData::member`]), which is what streaming must not quietly turn into 196
/// hashes of the same 20 MB.
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

/// One path's worth of [`open_files_streaming`]. Split out so that `?` on the caller's
/// answer reads as what it is — abandon this file — rather than as a flag threaded
/// through two nested loops.
fn open_one_file(
    path: &Path,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let Ok(file) = fs::read(path) else {
        // Unreadable is still an end. Whoever asked for this file is drawing it as
        // pending until told otherwise, and "it was never there" is told exactly here.
        return emit(Progress::Finished(path.to_path_buf()));
    };

    // One allocation per file, shared by every object parsed out of it and held for
    // as long as they live; see `ObjectData`. `fs::read` gives a `Vec`, so this
    // copies the bytes once more before dropping the original.
    //
    // The file's own `ObjectData` is built here rather than at the bottom where it is
    // used, because it is what the members are cut from: the hash it takes is then one
    // pass over the file however many objects come out of it.
    let file = ObjectData::whole_file(Arc::from(file));

    if let Ok(archive) = ArchiveFile::parse(file.bytes()) {
        for member in archive.members() {
            let Ok(member) = member else {
                continue;
            };
            let name = String::from_utf8_lossy(member.name()).into_owned();
            // The same bytes `member.data(..)` would return, addressed as a range into
            // the archive so the member stays reachable from the object without the
            // archive having to be scanned again.
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

/// [`open_files_streaming`] with the objects collected, for a caller with nowhere to put
/// them one at a time — the crate's own tests, and anything that has no window to draw
/// them in.
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

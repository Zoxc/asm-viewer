use iced_x86::Formatter;
use object::{
    read::archive::ArchiveFile, CompressionFormat, Object as _, ObjectSection, ObjectSymbol,
    Relocation, RelocationTarget, SymbolIndex, SymbolKind,
};
use std::{cell::RefCell, collections::HashMap, fs, ops::Range, path::PathBuf, rc::Rc, sync::Arc};
use symbolic_demangle::{Demangle, DemangleOptions};

mod line;

pub use line::{DwarfCache, LineInfo, LineRow, Location};
// `Section::index` is one of these, so anything building or reading a `Section` needs the
// type; the viewer does not depend on `object` itself.
pub use object::{BinaryFormat, SectionIndex};

/// `Object` is handed around as an `Arc` and read from worker threads, so everything it
/// holds — the lazily built DWARF context above all, whose `addr2line::Context` is `Send`
/// but not `Sync` on its own — has to be shared-safe. Assert it here rather than find out
/// at a call site.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Object>();
};

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    pub format: BinaryFormat,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,

    /// This object's DWARF, built on the first query and never at parse time. Nothing
    /// constructs it: write `DwarfCache::default()`. See [`Object::line_info`].
    pub dwarf: DwarfCache,
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
}

impl ObjectData {
    /// The whole file: a plain object file, or the archive file itself.
    pub fn whole_file(file: Arc<[u8]>) -> Self {
        let range = 0..file.len();
        Self { file, range }
    }

    /// One archive member, as the `(offset, size)` its header declares. [`None`] when
    /// that range does not lie inside the file, which is the same bounds check
    /// `ArchiveMember::data` does — such a member is skipped, exactly as before.
    pub fn member(file: &Arc<[u8]>, offset: u64, size: u64) -> Option<Self> {
        let start: usize = offset.try_into().ok()?;
        let end = start.checked_add(size.try_into().ok()?)?;
        file.get(start..end)?;
        Some(Self {
            file: file.clone(),
            range: start..end,
        })
    }

    /// The object file's own bytes.
    pub fn bytes(&self) -> &[u8] {
        // The range was bounds-checked when it was built.
        &self.file[self.range.clone()]
    }
}

impl std::fmt::Debug for ObjectData {
    /// Never the bytes themselves: an object file is megabytes of them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectData")
            .field("range", &self.range)
            .field("file_len", &self.file.len())
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
    pub fn estimate_size(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        let i = section.symbols.binary_search(&self.address).ok()?;
        if i + 1 == section.symbols.len() {
            section
                .address
                .checked_add(section.data.len().try_into().ok()?)?
                .checked_sub(self.address)
        } else {
            section.symbols[i + 1].checked_sub(self.address)
        }
    }

    pub fn data(&self) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = self.estimate_size()?.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.get(offset..end)
    }

    pub fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let bytes = self.data()?;
        let mut decoder =
            iced_x86::Decoder::with_ip(64, bytes, self.address, iced_x86::DecoderOptions::NONE);

        // The name the next `format` call should substitute for the placeholder operand,
        // handed to the formatter's symbol resolver through a cell the loop below writes.
        // See `RelocationResolver`.
        let pending = Rc::new(RefCell::new(None));
        let mut formatter = iced_x86::IntelFormatter::with_options(
            Some(Box::new(RelocationResolver {
                pending: pending.clone(),
            })),
            None,
        );

        formatter.options_mut().set_first_operand_char_index(10);
        formatter
            .options_mut()
            .set_space_after_operand_separator(true);
        // A branch target is padded to sixteen digits by default -- `jle short
        // 000000000000004Bh` for a jump a few bytes up the same function -- which is the
        // width of a 64-bit address spent on a number that is nowhere near one. The
        // leading zeros carry nothing a reader wants: the target is read for *where* it
        // is relative to the instructions around it, and the addresses in the column to
        // the left are already padded to a fixed width for exactly that comparison.
        // Displacements and immediates are a separate option and are left alone.
        formatter.options_mut().set_branch_leading_zeros(false);

        let mut instruction = iced_x86::Instruction::default();

        let mut assembly = Assembly {
            instructions: Vec::new(),
            edges: Vec::new(),
        };

        // Every branch this symbol takes that names an address of its own, as (the index
        // of the branching instruction, the address it names). Collected while decoding
        // and turned into [`BranchEdge`]s afterwards, because a forward branch names an
        // address no instruction has been decoded at yet.
        let mut branches: Vec<(usize, u64)> = Vec::new();

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            let start_index = (instruction.ip() - self.address) as usize;

            let mut relocation = None;

            self.section.as_ref().map(|section| {
                for i in 0..instruction.len() {
                    section
                        .relocations
                        .get(&(instruction.ip() + i as u64))
                        .map(|r| {
                            relocation = Some(r.target().clone());
                        });
                }
            });

            // Whether *any* relocation covers these bytes, which is not the same question
            // as whether one resolved to something navigable below: a branch relocated
            // against a section or a data symbol resolves to `None` and its displacement
            // is a placeholder all the same. Only the first question tells `branch_target`
            // whether the encoded target means anything.
            let relocated = relocation.is_some();

            let relocation = relocation.and_then(|r| match r {
                RelocationTarget::Symbol(i) => object.symbols.get(&i).cloned(),
                _ => None,
            });

            // Resolved to instruction indices below, once every address in the symbol is
            // known: a branch can point forwards as easily as backwards.
            if !relocated {
                if let Some(target) = branch_target(&instruction) {
                    branches.push((assembly.instructions.len(), target));
                }
            }

            let mut inst = Instruction {
                address: instruction.ip(),
                bytes: bytes[start_index..start_index + instruction.len()].to_vec(),
                format: Vec::new(),
                relocation,
                relocation_span: None,
            };

            // Arm the resolver for exactly this instruction. It takes the name, so at
            // most one operand is substituted however many the formatter asks about,
            // and anything left over is cleared before the next instruction.
            *pending.borrow_mut() = inst
                .relocation
                .as_ref()
                .map(|target| target.display().to_owned());

            // Keep the `rip+` visible when — and only when — the name that is about to
            // replace the displacement is one the user can navigate to. See
            // `rip_relative`.
            formatter
                .options_mut()
                .set_rip_relative_addresses(inst.relocation.is_some() && rip_relative(&instruction));

            formatter.format(&instruction, &mut inst);

            assembly.instructions.push(inst);
        }

        // The decoder walks the symbol's bytes from the front, so these are ascending and
        // a target is one binary search away. An address that is not in the list is a
        // branch this symbol has no row for — out of its extent, or into the middle of one
        // of its instructions — and is dropped; see [`Assembly::edges`].
        let addresses: Vec<u64> = assembly
            .instructions
            .iter()
            .map(|instruction| instruction.address)
            .collect();
        assembly.edges = branches
            .into_iter()
            .filter_map(|(from, target)| {
                let to = addresses.binary_search(&target).ok()?;
                (to != from).then_some(BranchEdge { from, to })
            })
            .collect();

        Some(Arc::new(assembly))
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

/// The kind of a formatted assembly text span, as far as the UI is concerned.
///
/// This is the disassembler-independent stand-in for `iced_x86::FormatterTextKind`,
/// so that nothing outside this crate has to depend on a particular backend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanKind {
    Mnemonic,
    Prefix,
    Register,
    Number,
    /// A branch target: the address operand of a `call`, `jmp` or `jcc`.
    Address,
    Other,
}

impl From<iced_x86::FormatterTextKind> for SpanKind {
    fn from(kind: iced_x86::FormatterTextKind) -> Self {
        match kind {
            iced_x86::FormatterTextKind::Mnemonic => SpanKind::Mnemonic,
            iced_x86::FormatterTextKind::Prefix => SpanKind::Prefix,
            iced_x86::FormatterTextKind::Register => SpanKind::Register,
            iced_x86::FormatterTextKind::Number => SpanKind::Number,
            // A near-branch target comes through `write_number` as one of these two:
            // `FunctionAddress` when the branch is a call, `LabelAddress` for a plain
            // jump or a conditional one. iced-x86 has no `BranchTarget` kind.
            iced_x86::FormatterTextKind::LabelAddress
            | iced_x86::FormatterTextKind::FunctionAddress => SpanKind::Address,
            _ => SpanKind::Other,
        }
    }
}

#[derive(Clone)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub format: Vec<(String, SpanKind)>,
    pub relocation: Option<Arc<SymbolData>>,

    /// Where in [`format`](Self::format) the relocation target's name was substituted
    /// for the operand's placeholder value, when it was. That is one whole span, so a
    /// renderer can lift it out and draw it as a link to `relocation` without having to
    /// find it in the text.
    ///
    /// [`None`] with a `relocation` present means the formatter never offered an operand
    /// to substitute into — the relocation covers a byte of this instruction but none of
    /// its operands is an address or an immediate — and the target can only be named
    /// beside the instruction rather than inside it.
    pub relocation_span: Option<usize>,
}

impl iced_x86::FormatterOutput for Instruction {
    fn write(&mut self, text: &str, kind: iced_x86::FormatterTextKind) {
        self.format.push((text.to_owned(), kind.into()));
    }

    /// The formatter asked [`RelocationResolver`] about an operand and got the
    /// relocation target's name back, so this is the placeholder's replacement. Record
    /// where it lands: it is the span the UI makes clickable.
    fn write_symbol(
        &mut self,
        _instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        _address: u64,
        symbol: &iced_x86::SymbolResult<'_>,
    ) {
        fn part<'a>(
            part: &'a iced_x86::SymResTextPart<'a>,
        ) -> (&'a str, iced_x86::FormatterTextKind) {
            let text = match &part.text {
                iced_x86::SymResString::Str(text) => text,
                iced_x86::SymResString::String(text) => text.as_str(),
            };
            (text, part.color)
        }

        let start = self.format.len();
        match &symbol.text {
            iced_x86::SymResTextInfo::Text(one) => {
                let (text, kind) = part(one);
                self.write(text, kind);
            }
            // Our resolver never builds one of these, but the trait allows it.
            iced_x86::SymResTextInfo::TextVec(many) => {
                for one in *many {
                    let (text, kind) = part(one);
                    self.write(text, kind);
                }
            }
        }

        // Only point at a name that is a single span; anything else has no one span to
        // make clickable and falls back to being named beside the instruction.
        if self.format.len() == start + 1 {
            self.relocation_span = Some(start);
        }
    }
}

/// Whether this instruction's memory operand is addressed relative to the instruction
/// pointer, i.e. whether printing it as `[rip+…]` is the truth about the encoding.
///
/// iced-x86 hides that form by default: `rip_relative_addresses` is off, so
/// `IntelFormatter` drops the base register and prints the absolute address the
/// displacement resolves to (`[6]`). That is the more useful answer for an operand whose
/// displacement is real, and it is what an *unrelocated* rip-relative operand keeps. It
/// is the wrong answer when the displacement is a relocation placeholder that we replace
/// with a symbol name, because then `[name]` claims an absolute address the encoding does
/// not have; `[rip+name]` is both true and the form every disassembler prints.
///
/// `memory_base` is [`Register::None`](iced_x86::Register::None) for an instruction with
/// no memory operand at all, so this answers `false` for a direct `call rel32` or a
/// `mov eax, imm32` and their rendering is untouched. `EIP` is included because 64-bit
/// code can address relative to it with a `67h` address-size override, and the formatter
/// treats the two the same.
fn rip_relative(instruction: &iced_x86::Instruction) -> bool {
    matches!(
        instruction.memory_base(),
        iced_x86::Register::RIP | iced_x86::Register::EIP
    )
}

/// Hands the formatter the relocation target's name in place of a relocated operand's
/// value.
///
/// The value encoded in a relocated operand is a placeholder — a zero, or an addend — so
/// printing it is worse than useless. iced-x86's [`SymbolResolver`](iced_x86::SymbolResolver)
/// is the hook for replacing it, and it is asked at the point the operand is being
/// written, so the name lands *inside* whatever syntax surrounds it: a memory operand
/// reads `[name]` rather than the `[]` that dropping the number outright left behind.
///
/// The relocation itself is found by byte range, not by operand — a relocation records
/// where in the instruction it applies, and nothing maps that back to an operand number —
/// so `pending` is armed with the name once per instruction and *taken* by the first
/// operand the formatter asks about. An instruction with a second numeric operand
/// therefore keeps that operand's real value instead of losing it too.
struct RelocationResolver {
    pending: Rc<RefCell<Option<String>>>,
}

impl iced_x86::SymbolResolver for RelocationResolver {
    fn symbol(
        &mut self,
        _instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        address: u64,
        _address_size: u32,
    ) -> Option<iced_x86::SymbolResult<'_>> {
        let name = self.pending.borrow_mut().take()?;
        // The symbol's address has to be the one asked about: the formatter prints the
        // difference between the two after the name, and any other value would append a
        // meaningless displacement to it.
        Some(iced_x86::SymbolResult::with_string_kind(
            address,
            name,
            iced_x86::FormatterTextKind::FunctionAddress,
        ))
    }
}

pub struct Assembly {
    pub instructions: Vec<Instruction>,

    /// The branches that stay inside this symbol, for a renderer to draw as arrows down
    /// the side of the listing.
    ///
    /// Both ends are indices into [`instructions`](Self::instructions) rather than
    /// addresses, because that is what a row can be asked about: an arrow gutter is drawn
    /// per row and has to know which edges start, end and pass through the row it is
    /// building. `from` ascends and no instruction branches twice, so this is at most one
    /// entry per instruction and is already in listing order.
    ///
    /// Three kinds of branch are deliberately *not* in here, and each of them would be a
    /// line drawn to a place that is not where it points:
    ///
    /// * One that leaves the symbol. Nothing on screen is at the other end of it — the
    ///   listing is one symbol's own bytes — and the instruction already names where it
    ///   goes.
    /// * One whose displacement is a relocation placeholder. The encoded value is a zero
    ///   or an addend that a linker will overwrite, so read literally it is very often a
    ///   plausible-looking address a few bytes away, i.e. an edge to a row of this very
    ///   symbol that the branch has nothing to do with.
    /// * One landing inside an instruction rather than on one. Either the bytes are not
    ///   code, or the real instruction stream is not the one a linear decode from the
    ///   symbol's first byte produced; either way there is no row to point an arrowhead
    ///   at, and inventing the nearest one would be a lie about where control goes.
    ///
    /// Nor is a branch to itself (`jmp $`), whose two ends are the same row: it would take
    /// a lane to draw a line of no length, and the instruction's own operand already says
    /// it goes nowhere.
    pub edges: Vec<BranchEdge>,
}

/// A branch from one instruction of a symbol to another instruction of the same symbol,
/// both named by their index in [`Assembly::instructions`]. See [`Assembly::edges`] for
/// what is and is not one of these.
///
/// `from` and `to` are in execution order, not in listing order: a backward branch — the
/// bottom of a loop — has `from` greater than `to`. Anything laying edges out in a gutter
/// wants [`first`](Self::first) and [`last`](Self::last) instead, which are the rows the
/// line is drawn between whichever way it runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BranchEdge {
    /// The instruction that branches.
    pub from: usize,
    /// The instruction it branches to.
    pub to: usize,
}

impl BranchEdge {
    /// The topmost of the two rows this edge spans.
    pub fn first(&self) -> usize {
        self.from.min(self.to)
    }

    /// The bottommost of the two rows this edge spans.
    pub fn last(&self) -> usize {
        self.from.max(self.to)
    }

    /// Whether this branch runs back up the listing, which is what the bottom of a loop
    /// looks like.
    pub fn is_backward(&self) -> bool {
        self.to < self.from
    }
}

/// The address `instruction` branches to, when it names one in its own encoding and that
/// is somewhere a reader would follow it to.
///
/// Which flow-control kinds count is a judgement about what an arrow *means* in the
/// gutter: it means control leaves this row and carries on at that one. So an
/// unconditional jump and a conditional one are edges, and so are `loop`, `loopcc` and
/// `jrcxz`, which iced-x86 already classifies as conditional branches. `xbegin` is one
/// too — its operand is the address execution resumes at when the transaction aborts,
/// which is a real second exit from the row and usually a handler a few instructions
/// further down.
///
/// A **call** is not, even when it lands inside this same symbol — a recursive one, or the
/// `call $+5` a position-independent thunk uses to read its own address. Control comes
/// straight back to the row underneath, so an arrow leading the eye away from it would say
/// the opposite of what happens; and a call is the one branch that already renders as a
/// navigable name whenever it resolves to a symbol, which is the better answer for the
/// question a call raises. An indirect branch or call names no address at all, so there is
/// nothing to draw either way.
///
/// The operand kind is checked as well as the flow control because `near_branch_target`
/// answers 0 for anything that is not a near branch, and 0 is a perfectly ordinary address
/// in a relocatable object — it is the symbol's own first byte. `xabort imm8` is the
/// instruction that makes that reachable: it shares `xbegin`'s flow-control kind, its
/// operand is an immediate and not an address at all, and without this check every one of
/// them would draw an arrow to the top of the function.
fn branch_target(instruction: &iced_x86::Instruction) -> Option<u64> {
    match instruction.flow_control() {
        iced_x86::FlowControl::UnconditionalBranch
        | iced_x86::FlowControl::ConditionalBranch
        | iced_x86::FlowControl::XbeginXabortXend => {}
        _ => return None,
    }

    matches!(
        instruction.op0_kind(),
        iced_x86::OpKind::NearBranch16
            | iced_x86::OpKind::NearBranch32
            | iced_x86::OpKind::NearBranch64
    )
    .then(|| instruction.near_branch_target())
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

            // Insert symbol addresses into sections
            file.symbols().for_each(|symbol| {
                if symbol.kind() != SymbolKind::Text {
                    return;
                }

                symbol
                    .section()
                    .index()
                    .and_then(|index| sections.get_mut(&index))
                    .map(|section| section.symbols.push(symbol.address()));
            });

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    section.symbols.sort_unstable();
                    (index, Arc::new(section))
                })
                .collect();

            let sections = section_map.values().cloned().collect();

            let symbols: HashMap<_, _> = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let name = String::from_utf8_lossy(symbol.name_bytes().ok()?).into_owned();
                    let demangled =
                        symbolic_common::Name::from(&name).demangle(DemangleOptions::complete());

                    let section = symbol
                        .section()
                        .index()
                        .and_then(|index| section_map.get(&index).cloned());

                    Some((
                        symbol.index(),
                        Arc::new(SymbolData {
                            name,
                            demangled,
                            section,
                            address: symbol.address(),
                            size: symbol.size(),
                        }),
                    ))
                })
                .collect();

            let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
            symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));

            ParsedObject {
                format: file.format(),
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
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
}

fn open_object(out: &mut Vec<Arc<Object>>, data: ObjectData, name: String, path: PathBuf) {
    out.extend(parse_object(data, name, path));
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a
/// plain object file. Anything that fails to read or parse is silently skipped.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();

    for path in paths {
        let Ok(file) = fs::read(&path) else {
            continue;
        };

        // One allocation per file, shared by every object parsed out of it and held for
        // as long as they live; see `ObjectData`. `fs::read` gives a `Vec`, so this
        // copies the bytes once more before dropping the original.
        let file: Arc<[u8]> = Arc::from(file);

        if let Ok(archive) = ArchiveFile::parse(&*file) {
            for member in archive.members() {
                member
                    .map(|member| {
                        let name = String::from_utf8_lossy(member.name()).into_owned();
                        // The same bytes `member.data(..)` would return, addressed as a
                        // range into the archive so the member stays reachable from the
                        // object without the archive having to be scanned again.
                        let (offset, size) = member.file_range();
                        if let Some(data) = ObjectData::member(&file, offset, size) {
                            open_object(&mut objects, data, name, path.clone());
                        }
                    })
                    .ok();
            }
        }

        open_object(
            &mut objects,
            ObjectData::whole_file(file),
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
                .into_owned(),
            path.clone(),
        );
    }

    objects
}

//! The seam between this crate and whatever library actually decodes instructions.
//!
//! Everything a caller sees of a disassembly — [`Assembly`], [`Instruction`], [`SpanKind`],
//! [`BranchEdge`] — is defined here and names no backend. [`x86`] is the only module in the
//! crate that mentions `iced-x86`; a second architecture is a second [`Disassembler`] impl
//! plus an arm in [`Assembly::decode`].
//!
//! The dispatch is generic and not dynamic: the architecture is matched on once, each arm
//! naming a concrete backend, and the decode path is monomorphised for it. Nothing is boxed
//! and no signature here says `dyn`, so a backend's formatting and span-mapping can inline
//! into the per-instruction decode loop.

use crate::{Extent, Object, Section, SymbolData};
use object::{Architecture, RelocationTarget};
use std::{ops::Range, sync::Arc};

mod x86;

/// What to call `architecture` in a sentence explaining that nothing decoded it. [`None`]
/// for one with no spelling here, which falls back to the `Debug` name through the caller.
fn architecture_name(architecture: Architecture) -> Option<&'static str> {
    Some(match architecture {
        Architecture::Unknown => "an unknown architecture",
        Architecture::Aarch64 | Architecture::Aarch64_Ilp32 => "aarch64",
        Architecture::Arm => "32-bit ARM",
        Architecture::I386 => "32-bit x86",
        Architecture::X86_64 | Architecture::X86_64_X32 => "x86-64",
        Architecture::Riscv32 => "32-bit RISC-V",
        Architecture::Riscv64 => "64-bit RISC-V",
        Architecture::Mips => "MIPS",
        Architecture::Mips64 => "64-bit MIPS",
        Architecture::PowerPc => "PowerPC",
        Architecture::PowerPc64 => "64-bit PowerPC",
        Architecture::LoongArch64 => "LoongArch64",
        Architecture::S390x => "s390x",
        Architecture::Wasm32 | Architecture::Wasm64 => "WebAssembly",
        _ => return None,
    })
}

/// The bytes of one symbol, handed to a backend to decode.
///
/// The relocations come with them because a relocated operand's encoded value is a
/// placeholder a linker will overwrite. Asking is a method rather than a field so a backend
/// gets the answer *per instruction*, which is the only granularity a relocation has — it
/// records a byte range, never an operand number.
pub struct Code<'a> {
    /// The symbol's own bytes, from its first to its last.
    pub bytes: &'a [u8],

    /// The address `bytes[0]` sits at. In a relocatable object this is an offset into the
    /// section and typically 0; nothing here depends on it being either.
    pub address: u64,

    /// The section the bytes came from, for its relocations. [`None`] for a symbol with no
    /// section.
    section: Option<&'a Section>,

    /// The object the bytes belong to, for its symbols: by index, for turning a
    /// relocation's target into something a reader can click, and by address, for a call
    /// whose relocation a linker has already applied.
    object: &'a Object,
}

impl<'a> Code<'a> {
    pub(crate) fn new(
        bytes: &'a [u8],
        address: u64,
        section: Option<&'a Section>,
        object: &'a Object,
    ) -> Self {
        Self {
            bytes,
            address,
            section,
            object,
        }
    }

    /// The relocation covering any of the `len` bytes at `address`, if there is one; the
    /// *last* hit wins.
    ///
    /// Both halves of the answer are different questions: [`Some`] means the encoded operand
    /// is a placeholder, while [`target`](Relocated::target) is [`None`] whenever the
    /// relocation points at something this object has no text symbol for (a section, a data
    /// symbol, an undefined import).
    pub fn relocation(&self, address: u64, len: usize) -> Option<Relocated> {
        let section = self.section?;
        let mut found = None;
        for offset in 0..len as u64 {
            // Checked because the address is the file's number plus a byte offset, and a
            // section placed at the very end of the address space wraps it.
            if let Some(relocation) = address
                .checked_add(offset)
                .and_then(|address| section.relocations.get(&address))
            {
                found = Some(relocation);
            }
        }

        let target = match found?.target() {
            RelocationTarget::Symbol(index) => self.object.symbols.get(&index).cloned(),
            _ => None,
        };
        Some(Relocated { target })
    }

    /// The text symbol starting at `address`, an address in this code's own section — the
    /// one a branch's encoding names — where one does. The other way a target gets a name:
    /// in a linked image the relocations are gone and the displacement is the answer.
    ///
    /// The object's index is by *placed* address, so the section's bias goes on first, and
    /// the hit has to be in this section: with every code section of a relocatable object
    /// at 0, a displacement past this section's end lands on some other section's function
    /// in the placed space, and that is not where the call goes. A target nothing starts at
    /// is [`None`], and the operand stays the number it is.
    pub fn symbol_at(&self, address: u64) -> Option<Arc<SymbolData>> {
        let section = self.section?;
        let symbol = self.object.symbol_at(address.wrapping_add(section.bias))?;
        let home = symbol.section.as_ref()?;
        std::ptr::eq(Arc::as_ptr(home), section).then(|| symbol.clone())
    }
}

/// A relocation covering an instruction's bytes. See [`Code::relocation`].
pub struct Relocated {
    /// The text symbol the relocation names, where it names one this object kept.
    pub target: Option<Arc<SymbolData>>,
}

/// One disassembler backend: given a symbol's bytes, the rows to draw for them, in address
/// order.
///
/// A backend decodes from the first byte to the last and stops early rather than erroring —
/// the bytes are whatever was in the file. A branch's target stays an *address* on its row
/// ([`Instruction::branch`]), because a forward branch names one no instruction has been
/// decoded at yet; [`Assembly`] turns those into row indices once the whole symbol is decoded.
///
/// Implementors are named concretely by `Assembly::decode` and never made into an object, so
/// the trait is the shape a backend is written to and not a way to hold one.
pub trait Disassembler {
    fn disassemble(&self, code: &Code<'_>) -> Vec<Instruction>;
}

/// The kind of a formatted assembly text span: the disassembler-independent stand-in for a
/// backend's own text-kind enumeration, so nothing outside this crate depends on one.
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

/// What an instruction's operand names, where it names anything: the one link a row of it
/// can draw, and the whole of what a backend decided about it.
///
/// **The four cases split by how the address was arrived at, not by what the instruction
/// is.** Any operand a relocation covers is [`SymbolName`](Self::SymbolName) or
/// [`Placeholder`](Self::Placeholder), whatever the opcode and whichever operand it is: an
/// immediate, a memory displacement and a branch's own rel32 are all one question, which is
/// whether the relocation named a text symbol this object kept. A `lea rdi, [rip+0x0]`
/// relocated against a function is a [`SymbolName`](Self::SymbolName) exactly as a `call`
/// is; the same `lea` against a data symbol is a [`Placeholder`](Self::Placeholder).
/// [`Branch`](Self::Branch) and [`Call`](Self::Call) are the *unrelocated* cases alone,
/// where the encoding's own displacement is the answer.
///
/// So the cases are exclusive by construction and not by convention. A relocated operand
/// carries a placeholder a linker will overwrite, so a row that names a symbol names no
/// address of its own and a row that names an address was not relocated; and a call the
/// backend found a symbol for goes to that symbol's address rather than to one of its own.
/// A caller matches on this; nothing outside a backend works out which case a row is in.
#[derive(Clone)]
pub enum Operand {
    /// A text symbol the operand names, whatever kind of operand it is: the target of a
    /// relocation covering the instruction's bytes, or, with no relocation, the function a
    /// direct `call` reaches ([`Code::symbol_at`]) — which is what a linked image's calls
    /// are, the linker having applied theirs.
    ///
    /// Spelt out rather than called `Symbol`, which the crate already exports for the
    /// object-and-[`SymbolData`] pair, and named after what it holds rather than after what
    /// happened to the operand.
    SymbolName {
        symbol: Arc<SymbolData>,

        /// Where in [`format`](Instruction::format) the name was substituted for the
        /// operand's placeholder value — one whole span, so a renderer can lift it out and
        /// draw it as a link. An index into `format` and never an offset into the symbol.
        ///
        /// [`None`] means the formatter offered no operand to substitute into, and the
        /// symbol can only be named *beside* the instruction.
        span: Option<usize>,
    },

    /// A relocation covered the bytes and named nothing this object kept — a section, a
    /// data symbol, an undefined import. What is printed is the placeholder itself, so the
    /// row names neither a symbol nor an address and has no link. Kept apart from having no
    /// operand at all because it is the one case where the number means nothing.
    ///
    /// [`SymbolName`](Self::SymbolName)'s other half, and reached on the same rule: a
    /// relocation covers these bytes. Nothing about the opcode enters into it.
    Placeholder,

    /// The address this instruction's own encoding branches to, no relocation covering it —
    /// a `jmp`, a `jcc`, a `loop`, an `xbegin`; never a `call`, since control comes straight
    /// back.
    ///
    /// The **address-keyed** answer, kept beside the index-keyed [`Assembly::edges`]: a
    /// listing that is not one symbol's — a whole section's — cannot say up front whether
    /// the target has a row, only where it is, and finds the row when it decodes there.
    /// Nothing is judged here: a branch out of the symbol, into the middle of an
    /// instruction, or `jmp $` all keep their address, and the four kinds
    /// [`Assembly::edges`] drops keep their span. A caller that wants to *follow* one pairs
    /// this with [`Assembly::edge_from`], which is what says the target has a row.
    Branch {
        address: u64,

        /// Where in [`format`](Instruction::format) the displacement was printed — an index
        /// into `format`, as every `span` here is. It says where the number is and not that
        /// there is anywhere to go.
        span: usize,
    },

    /// The address a direct near `call` goes to, its displacement real — no relocation
    /// covers its bytes — and no text symbol starting there, since a call one does is a
    /// [`SymbolName`](Self::SymbolName) and its address is the symbol's.
    ///
    /// [`Branch`](Self::Branch)'s counterpart for the one kind of branch it leaves out, and
    /// kept apart from it because the two answer different questions: a branch is a line
    /// the gutter draws and a row this listing may have, while this is only *where the
    /// instruction goes* — into the middle of a function, a function no symbol names, a
    /// stretch of a linked image nothing claims — for a reader to be taken there in a
    /// listing of the whole object. Nothing is judged here either.
    Call {
        address: u64,

        /// Where in [`format`](Instruction::format) the address was printed.
        span: usize,
    },
}

#[derive(Clone)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub format: Vec<(String, SpanKind)>,

    /// What this instruction's operand names, where it names anything. [`None`] for a row
    /// with nothing to point at: a `ret`, a register move, a number that is only a number.
    pub operand: Option<Operand>,
}

impl Instruction {
    /// The text symbol this instruction's operand names, where it names one. See
    /// [`Operand::SymbolName`].
    pub fn symbol(&self) -> Option<&Arc<SymbolData>> {
        match &self.operand {
            Some(Operand::SymbolName { symbol, .. }) => Some(symbol),
            _ => None,
        }
    }

    /// The address this instruction's own encoding branches to. See [`Operand::Branch`],
    /// which is the only case this is [`Some`] for.
    pub fn branch(&self) -> Option<u64> {
        match self.operand {
            Some(Operand::Branch { address, .. }) => Some(address),
            _ => None,
        }
    }

    /// The address this instruction goes to and nothing here has named: a branch's own, or
    /// an unnamed call's. See [`Operand::Branch`] and [`Operand::Call`].
    pub fn target(&self) -> Option<u64> {
        match self.operand {
            Some(Operand::Branch { address, .. } | Operand::Call { address, .. }) => Some(address),
            _ => None,
        }
    }
}

pub struct Assembly {
    pub instructions: Vec<Instruction>,

    /// The branches that stay inside this symbol, for an arrow gutter. Both ends are indices
    /// into [`instructions`](Self::instructions) rather than addresses, because that is what
    /// a row can be asked about.
    ///
    /// Four kinds are deliberately dropped, each being a line to a place it does not point
    /// at: one leaving the symbol; one whose displacement is a relocation placeholder (the
    /// encoded value is a zero or an addend, very often a plausible-looking address inside
    /// this symbol); one landing mid-instruction; and `jmp $`, whose two ends are one row.
    /// A call is not an edge either — control comes straight back.
    pub edges: Vec<BranchEdge>,

    /// Set when no backend in this crate decodes the architecture the object declared, and
    /// names that architecture. Distinct from an empty listing, which means a symbol holding
    /// no instructions.
    pub undecodable: Option<&'static str>,

    /// The bytes these rows were decoded from: the symbol's address to its extent, in the
    /// section's own addresses. Carried so that nobody has to ask for the extent a second
    /// time — [`SymbolData::extent`] is the most expensive answer in the crate, and this is
    /// what it came to.
    pub range: Range<u64>,

    /// The extent [`range`](Self::range) is, so that a caller can tell an end the file
    /// states from the derivation's cap without asking again.
    pub extent: Extent,
}

impl Assembly {
    /// The listing for `code`, decoded by whichever backend claims `architecture`, over
    /// `range` — the bytes `code` holds — and the `extent` that range came from. Both are
    /// the caller's, [`SymbolData::assembly`] having them in hand already.
    ///
    /// The only place an architecture is dispatched on. The set is closed at compile time,
    /// so each arm names its backend concretely and `decoded` is compiled once per backend
    /// rather than called through a vtable; an architecture no arm claims is the third
    /// answer, `unsupported`.
    ///
    /// **Bitness comes from the architecture and not from `is_64()`.** `X86_64_X32` — the
    /// x32 ABI — is 64-bit *code* with 32-bit pointers, so a file whose class says 32 still
    /// decodes as 64 there.
    pub(crate) fn decode(
        architecture: Architecture,
        code: &Code<'_>,
        range: Range<u64>,
        extent: Extent,
    ) -> Self {
        match architecture {
            Architecture::X86_64 | Architecture::X86_64_X32 => {
                Self::decoded(x86::X86 { bitness: 64 }, code, range, extent)
            }
            Architecture::I386 => Self::decoded(x86::X86 { bitness: 32 }, code, range, extent),
            _ => Self::unsupported(architecture, range, extent),
        }
    }

    /// The listing `backend` reads out of `code`, with its branch addresses resolved to row
    /// indices.
    ///
    /// Generic over the backend rather than taking one behind a pointer: this is the whole
    /// decode path, so monomorphising it is what lets a backend's per-instruction work
    /// inline into it.
    fn decoded<D: Disassembler>(
        backend: D,
        code: &Code<'_>,
        range: Range<u64>,
        extent: Extent,
    ) -> Self {
        let instructions = backend.disassemble(code);

        // A backend decodes from the front, so these ascend and a target is one binary search
        // away. An address that is not in the list is a branch this symbol has no row for and
        // is dropped; see `edges`.
        let addresses: Vec<u64> = instructions
            .iter()
            .map(|instruction| instruction.address)
            .collect();
        let edges = instructions
            .iter()
            .enumerate()
            .filter_map(|(from, instruction)| {
                let to = addresses.binary_search(&instruction.branch()?).ok()?;
                (to != from).then_some(BranchEdge { from, to })
            })
            .collect();

        Self {
            instructions,
            edges,
            undecodable: None,
            range,
            extent,
        }
    }

    /// The edge the instruction at `index` branches along, if it is a branch this symbol
    /// has both ends of.
    ///
    /// A binary search, not a scan: an instruction names at most one branch target and a
    /// backend decodes from the front, so `from` ascends strictly across
    /// [`edges`](Self::edges).
    pub fn edge_from(&self, index: usize) -> Option<BranchEdge> {
        let at = self
            .edges
            .binary_search_by_key(&index, |edge| edge.from)
            .ok()?;
        self.edges.get(at).copied()
    }

    /// The answer for an architecture no arm of `decode` claims: no rows, the architecture's
    /// name to say why, and the bytes that would have been decoded — an undecodable symbol
    /// still states its extent.
    fn unsupported(architecture: Architecture, range: Range<u64>, extent: Extent) -> Self {
        Self {
            instructions: Vec::new(),
            edges: Vec::new(),
            undecodable: Some(
                architecture_name(architecture).unwrap_or("an unsupported architecture"),
            ),
            range,
            extent,
        }
    }
}

/// A branch from one instruction of a symbol to another instruction of the same symbol.
///
/// `from` and `to` are in execution order, not listing order: a backward branch — the bottom
/// of a loop — has `from` greater than `to`.
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

    pub fn is_backward(&self) -> bool {
        self.to < self.from
    }
}

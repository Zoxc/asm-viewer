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

use crate::{Section, SymbolData};
use object::{Architecture, RelocationTarget, SymbolIndex};
use std::{collections::HashMap, sync::Arc};

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

    /// The object's text symbols by index, for turning a relocation's target into something
    /// a reader can click.
    symbols: &'a HashMap<SymbolIndex, Arc<SymbolData>>,
}

impl<'a> Code<'a> {
    pub(crate) fn new(
        bytes: &'a [u8],
        address: u64,
        section: Option<&'a Section>,
        symbols: &'a HashMap<SymbolIndex, Arc<SymbolData>>,
    ) -> Self {
        Self {
            bytes,
            address,
            section,
            symbols,
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
            RelocationTarget::Symbol(index) => self.symbols.get(&index).cloned(),
            _ => None,
        };
        Some(Relocated { target })
    }
}

/// A relocation covering an instruction's bytes. See [`Code::relocation`].
pub struct Relocated {
    /// The text symbol the relocation names, where it names one this object kept.
    pub target: Option<Arc<SymbolData>>,
}

/// What a backend hands back: one entry per instruction it read, plus every branch that
/// named an address of its own.
///
/// The branches are `(index of the branching instruction, the address it names)` and stay
/// addresses here, because a forward branch names an address no instruction has been decoded
/// at yet. [`Assembly`] turns them into row indices once the whole symbol is decoded.
pub struct Decoded {
    pub instructions: Vec<Instruction>,
    pub branches: Vec<(usize, u64)>,
}

/// One disassembler backend: given a symbol's bytes, the rows to draw for them.
///
/// A backend decodes from the first byte to the last and stops early rather than erroring —
/// the bytes are whatever was in the file.
///
/// Implementors are named concretely by `Assembly::decode` and never made into an object, so
/// the trait is the shape a backend is written to and not a way to hold one.
pub trait Disassembler {
    fn disassemble(&self, code: &Code<'_>) -> Decoded;
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

#[derive(Clone)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub format: Vec<(String, SpanKind)>,
    pub relocation: Option<Arc<SymbolData>>,

    /// Where in [`format`](Self::format) the relocation target's name was substituted for
    /// the operand's placeholder value, when it was — one whole span, so a renderer can lift
    /// it out and draw it as a link.
    ///
    /// [`None`] with a `relocation` present means the formatter never offered an operand to
    /// substitute into, and the target can only be named beside the instruction.
    pub relocation_span: Option<usize>,
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
}

impl Assembly {
    /// The listing for `code`, decoded by whichever backend claims `architecture`.
    ///
    /// The only place an architecture is dispatched on. The set is closed at compile time,
    /// so each arm names its backend concretely and `decoded` is compiled once per backend
    /// rather than called through a vtable; an architecture no arm claims is the third
    /// answer, `unsupported`.
    ///
    /// **Bitness comes from the architecture and not from `is_64()`.** `X86_64_X32` — the
    /// x32 ABI — is 64-bit *code* with 32-bit pointers, so a file whose class says 32 still
    /// decodes as 64 there.
    pub(crate) fn decode(architecture: Architecture, code: &Code<'_>) -> Self {
        match architecture {
            Architecture::X86_64 | Architecture::X86_64_X32 => {
                Self::decoded(x86::X86 { bitness: 64 }, code)
            }
            Architecture::I386 => Self::decoded(x86::X86 { bitness: 32 }, code),
            _ => Self::unsupported(architecture),
        }
    }

    /// The listing `backend` reads out of `code`, with its branch addresses resolved to row
    /// indices.
    ///
    /// Generic over the backend rather than taking one behind a pointer: this is the whole
    /// decode path, so monomorphising it is what lets a backend's per-instruction work
    /// inline into it.
    fn decoded<D: Disassembler>(backend: D, code: &Code<'_>) -> Self {
        let decoded = backend.disassemble(code);

        // A backend decodes from the front, so these ascend and a target is one binary search
        // away. An address that is not in the list is a branch this symbol has no row for and
        // is dropped; see `edges`.
        let addresses: Vec<u64> = decoded
            .instructions
            .iter()
            .map(|instruction| instruction.address)
            .collect();
        let edges = decoded
            .branches
            .into_iter()
            .filter_map(|(from, target)| {
                let to = addresses.binary_search(&target).ok()?;
                (to != from).then_some(BranchEdge { from, to })
            })
            .collect();

        Self {
            instructions: decoded.instructions,
            edges,
            undecodable: None,
        }
    }

    /// The answer for an architecture no arm of `decode` claims: no rows, and the
    /// architecture's name to say why.
    fn unsupported(architecture: Architecture) -> Self {
        Self {
            instructions: Vec::new(),
            edges: Vec::new(),
            undecodable: Some(
                architecture_name(architecture).unwrap_or("an unsupported architecture"),
            ),
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

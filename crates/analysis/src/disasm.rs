//! The seam between this crate and whatever library actually decodes instructions.
//!
//! Everything a caller sees of a disassembly — [`Assembly`], [`Instruction`],
//! [`SpanKind`], [`BranchEdge`] — is defined here and names no backend. Behind
//! [`Disassembler`] sits one implementation today, [`x86`], which is the only module in
//! the crate that mentions `iced-x86`. A second architecture is a second impl plus an arm
//! in [`disassembler`], not a change to any of the types above.
//!
//! The trait is deliberately one call wide. A disassembler library offers far more than
//! this — instruction semantics, register liveness, an encoder — and a seam shaped after
//! *that* would be a second API to keep in step with a backend that never arrives. What
//! is behind this one is exactly what [`Assembly`] is: rows of formatted text, the bytes
//! they were decoded from, the relocation target substituted into each row's operand, and
//! the branches that name an address of their own. Everything after that is architecture
//! independent and stays out here — resolving a branch's address to a row index is a
//! binary search over what was decoded, so the four rules in [`Assembly::edges`] hold for
//! every backend rather than once per backend.
//!
//! What *is* per-backend, and belongs behind the trait rather than in front of it, is
//! every rule about how a relocated operand prints: which operand takes the substituted
//! name, and whether the operand keeps its `rip+`. Both are x86 spellings of a general
//! problem, and neither has an answer the seam could state for an architecture that has
//! not been written yet.

use crate::{Section, SymbolData};
use object::{Architecture, RelocationTarget, SymbolIndex};
use std::{collections::HashMap, sync::Arc};

mod x86;

/// The backend that decodes `architecture`, or [`None`] when nothing here does.
///
/// The architecture comes out of the object file ([`Object::architecture`](crate::Object)),
/// which is the only honest source for it: the bytes of a symbol say nothing about how to
/// read themselves, and decoding ARM as x86 yields a full page of confident nonsense
/// rather than an error.
///
/// **Bitness comes from the architecture and not from `is_64()`.** `object` splits x86
/// into `I386` and `X86_64` already, and the third form — `X86_64_X32`, the x32 ABI — is
/// 64-bit *code* with 32-bit pointers, so a file whose class says 32 still decodes as 64
/// there. Asking the header's class instead would get that one case backwards.
pub fn disassembler(architecture: Architecture) -> Option<Box<dyn Disassembler>> {
    match architecture {
        Architecture::X86_64 | Architecture::X86_64_X32 => Some(Box::new(x86::X86::bits(64))),
        Architecture::I386 => Some(Box::new(x86::X86::bits(32))),
        _ => None,
    }
}

/// What to call `architecture` in a sentence explaining that nothing decoded it.
///
/// `object::Architecture` is `Debug` and nothing else, and `Riscv64` is not what a reader
/// calls it. Only the architectures a real object file in the wild declares are spelled
/// out; the rest fall back to the `Debug` name through the caller, which is why this
/// answers [`None`] rather than a placeholder.
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
/// The relocations come with them because a relocated operand cannot be printed without
/// them: the value encoded there is a placeholder a linker will overwrite, so the bytes
/// alone are not enough to say what the instruction refers to. Asking is a method rather
/// than a field so that a backend gets the answer *per instruction*, which is the only
/// granularity a relocation has — it records a byte range, never an operand number.
pub struct Code<'a> {
    /// The symbol's own bytes, from its first to its last.
    pub bytes: &'a [u8],

    /// The address `bytes[0]` sits at, which is where a backend starts its instruction
    /// pointer. In a relocatable object this is an offset into the section and typically
    /// 0; nothing here depends on it being either.
    pub address: u64,

    /// The section the bytes came from, for its relocations. [`None`] for a symbol with
    /// no section, which is a symbol nothing can be looked up for.
    section: Option<&'a Section>,

    /// The object's text symbols by index, for turning a relocation's target into
    /// something a reader can click.
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

    /// The relocation covering any of the `len` bytes at `address`, if there is one.
    ///
    /// A relocation applies to a byte range inside an instruction — the four displacement
    /// bytes of a `call rel32`, say — so the whole instruction is asked about and the
    /// *last* hit wins, which is the only instruction with two of them that could be
    /// drawn at all. Both halves of the answer matter and they are different questions:
    /// [`Some`] means the encoded operand is a placeholder, while
    /// [`target`](Relocated::target) is [`None`] whenever the relocation points at
    /// something this object has no text symbol for (a section, a data symbol, an
    /// undefined import). See [`Assembly::edges`] for why the first question is the one a
    /// branch is judged on.
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
/// addresses here, because a forward branch names an address no instruction has been
/// decoded at yet. [`Assembly`] turns them into row indices once the whole symbol is
/// decoded, so which of them survive is decided in one place for every backend.
pub struct Decoded {
    pub instructions: Vec<Instruction>,
    pub branches: Vec<(usize, u64)>,
}

/// One disassembler backend: given a symbol's bytes, the rows to draw for them.
///
/// A backend decodes from the first byte to the last and stops early rather than
/// erroring — the bytes are whatever was in the file, and a listing that ends where the
/// bytes stopped making sense is the honest answer for a symbol whose extent was derived
/// rather than declared.
pub trait Disassembler {
    fn disassemble(&self, code: &Code<'_>) -> Decoded;
}

/// The kind of a formatted assembly text span, as far as the UI is concerned.
///
/// This is the disassembler-independent stand-in for a backend's own text-kind
/// enumeration (`iced_x86::FormatterTextKind` for the one backend there is), so that
/// nothing outside this crate has to depend on a particular one. It is deliberately
/// coarse: these are the distinctions a reader gets a colour for, not the distinctions a
/// formatter draws.
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

    /// Why there is nothing above, when the reason is that no backend in this crate
    /// decodes the architecture the object declared — and the name of that architecture,
    /// so a caller can say which one it was without depending on `object` itself.
    ///
    /// This is the third answer, and it exists because the other two are both wrong for a
    /// file the app can perfectly well open. Decoding the bytes anyway is how this crate
    /// used to behave — a page of plausible x86 for an aarch64 function, confident and
    /// entirely invented — and an empty listing with nothing said would be
    /// indistinguishable from a symbol that genuinely holds no instructions. A caller
    /// with this in hand can say *why* the pane is empty, which is the whole difference
    /// between a viewer that does not support an architecture and one that is broken.
    pub undecodable: Option<&'static str>,
}

impl Assembly {
    /// The listing for `code`, decoded by whichever backend claims `architecture`.
    pub(crate) fn decode(architecture: Architecture, code: &Code<'_>) -> Self {
        let Some(backend) = disassembler(architecture) else {
            return Self {
                instructions: Vec::new(),
                edges: Vec::new(),
                undecodable: Some(
                    architecture_name(architecture).unwrap_or("an unsupported architecture"),
                ),
            };
        };

        let decoded = backend.disassemble(code);

        // A backend decodes from the front, so these ascend and a target is one binary
        // search away. An address that is not in the list is a branch this symbol has no
        // row for — out of its extent, or into the middle of one of its instructions —
        // and is dropped; see [`Assembly::edges`].
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

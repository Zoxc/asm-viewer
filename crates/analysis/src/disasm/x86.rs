//! The x86 backend: the only module in the crate that mentions `iced-x86`.
//!
//! Everything here is a rule about how *x86* is read and printed — that a relocated
//! operand takes the target's name in place of its placeholder, that a rip-relative
//! operand keeps its `rip+` only when a name is going into it, which flow-control kinds
//! name an address a reader would follow. None of it generalises, which is exactly why it
//! sits behind [`Disassembler`] rather than in the code that calls one.

use super::{Code, Decoded, Disassembler, Instruction, SpanKind};
use iced_x86::Formatter;
use std::{cell::RefCell, rc::Rc};

/// x86 at one of its two decodable widths.
///
/// A bitness is not a property of the bytes — the same encoding means different
/// instructions at 16, 32 and 64 bits — so it comes from the object's architecture and is
/// fixed for the whole file. 16-bit is not reachable: `object` has no architecture for
/// it, a `.o` full of real-mode code declares itself `I386`, and nothing this app opens
/// contains any.
pub(super) struct X86 {
    bitness: u32,
}

impl X86 {
    pub(super) fn bits(bitness: u32) -> Self {
        Self { bitness }
    }
}

impl Disassembler for X86 {
    fn disassemble(&self, code: &Code<'_>) -> Decoded {
        let mut decoder = iced_x86::Decoder::with_ip(
            self.bitness,
            code.bytes,
            code.address,
            iced_x86::DecoderOptions::NONE,
        );

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

        let mut decoded = Decoded {
            instructions: Vec::new(),
            branches: Vec::new(),
        };

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            // Where this instruction is within the symbol. **Every step is checked**,
            // because the instruction pointer is the symbol's address plus what has been
            // decoded so far and both halves are numbers out of the file: a section
            // placed at the end of the address space, with an extent reaching past it,
            // wraps the pointer, and the offset derived from it went straight into a
            // slice index. No input reaches it *today* — an extent that long can only
            // come from debug info, and `addr2line` declines a `DW_TAG_subprogram` whose
            // `low_pc + high_pc` overflows before this crate ever sees it (see
            // `a_function_at_the_end_of_the_address_space_does_not_panic`) — but that is
            // four separate accidents holding the line, one of them a bug in a
            // dependency, and `.pdata` extents (`notes/Goals.md`) would add a fifth
            // source. The listing stops where the arithmetic does: what was decoded
            // before the wrap is still this symbol, and nothing after it is.
            let Some(start_index) = instruction
                .ip()
                .checked_sub(code.address)
                .and_then(|offset| usize::try_from(offset).ok())
            else {
                break;
            };
            let Some(end_index) = start_index.checked_add(instruction.len()) else {
                break;
            };
            let Some(encoded) = code.bytes.get(start_index..end_index) else {
                break;
            };

            let relocation = code.relocation(instruction.ip(), instruction.len());

            // Whether *any* relocation covers these bytes, which is not the same question
            // as whether one resolved to something navigable: a branch relocated against
            // a section or a data symbol resolves to `None` and its displacement is a
            // placeholder all the same. Only the first question tells `branch_target`
            // whether the encoded target means anything.
            let relocated = relocation.is_some();
            let relocation = relocation.and_then(|relocation| relocation.target);

            // Resolved to instruction indices by the caller, once every address in the
            // symbol is known: a branch can point forwards as easily as backwards.
            if !relocated {
                if let Some(target) = branch_target(&instruction) {
                    decoded.branches.push((decoded.instructions.len(), target));
                }
            }

            let mut inst = Instruction {
                address: instruction.ip(),
                bytes: encoded.to_vec(),
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
            formatter.options_mut().set_rip_relative_addresses(
                inst.relocation.is_some() && rip_relative(&instruction),
            );

            formatter.format(&instruction, &mut inst);

            decoded.instructions.push(inst);
        }

        decoded
    }
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

/// The spans of an [`Instruction`] are written by the formatter straight into it, which
/// is what makes `write_symbol` below able to say *where* the substituted name landed.
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
/// The option is global to the formatter and read by `format_memory`, so it cannot be set
/// per operand — which is why it is flipped per *instruction*, on exactly those with both
/// a resolved relocation and a rip-relative memory operand.
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

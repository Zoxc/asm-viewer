//! The x86 backend: the only module in the crate that mentions `iced-x86`.

use super::{Code, Decoded, Disassembler, Instruction, SpanKind};
use iced_x86::Formatter;
use std::{cell::RefCell, rc::Rc};

/// x86 at one of its two decodable widths (32 or 64). 16-bit is not reachable: `object`
/// has no architecture for it.
pub(super) struct X86 {
    pub(super) bitness: u32,
}

impl Disassembler for X86 {
    fn disassemble(&self, code: &Code<'_>) -> Decoded {
        let mut decoder = iced_x86::Decoder::with_ip(
            self.bitness,
            code.bytes,
            code.address,
            iced_x86::DecoderOptions::NONE,
        );

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
        // Branch targets are otherwise padded to sixteen digits. Displacements and
        // immediates are a separate option and are left alone.
        formatter.options_mut().set_branch_leading_zeros(false);

        let mut instruction = iced_x86::Instruction::default();

        let mut decoded = Decoded {
            instructions: Vec::new(),
            branches: Vec::new(),
        };

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            // Checked: the instruction pointer is the symbol's address plus what has been
            // decoded, both of them numbers out of the file, so a section at the end of
            // the address space wraps it and the offset would index the slice. The listing
            // stops at the wrap.
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

            // Whether *any* relocation covers these bytes, which is not whether one
            // resolved to something navigable: a branch relocated against a section
            // resolves to `None` and its displacement is a placeholder all the same. Only
            // this question says whether the encoded branch target means anything.
            let relocated = relocation.is_some();
            let relocation = relocation.and_then(|relocation| relocation.target);

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

            // The resolver takes the name, so at most one operand is substituted however
            // many the formatter asks about, and anything left over is cleared here before
            // the next instruction.
            *pending.borrow_mut() = inst
                .relocation
                .as_ref()
                .map(|target| target.display().to_owned());

            // `rip_relative_addresses` is global to the formatter (`format_memory` reads
            // it), so it is flipped per instruction: the `rip+` is kept only when a name is
            // replacing the displacement, since `[name]` would claim an absolute address
            // the encoding does not have. `EIP` counts too — 64-bit code can address
            // relative to it with a `67h` override.
            formatter.options_mut().set_rip_relative_addresses(
                inst.relocation.is_some()
                    && matches!(
                        instruction.memory_base(),
                        iced_x86::Register::RIP | iced_x86::Register::EIP
                    ),
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
            // A near-branch target comes through `write_number` as one of these two;
            // iced-x86 has no `BranchTarget` kind.
            iced_x86::FormatterTextKind::LabelAddress
            | iced_x86::FormatterTextKind::FunctionAddress => SpanKind::Address,
            _ => SpanKind::Other,
        }
    }
}

impl iced_x86::FormatterOutput for Instruction {
    fn write(&mut self, text: &str, kind: iced_x86::FormatterTextKind) {
        self.format.push((text.to_owned(), kind.into()));
    }

    /// The formatter got a name back from [`RelocationResolver`], so this is the
    /// placeholder's replacement. Record where it lands: it is the span the UI makes
    /// clickable.
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

        // Only point at a name that is a single span; anything else falls back to being
        // named beside the instruction.
        if self.format.len() == start + 1 {
            self.relocation_span = Some(start);
        }
    }
}

/// Hands the formatter the relocation target's name in place of a relocated operand's
/// placeholder value, at the point the operand is written — so the name lands inside
/// whatever syntax surrounds it (`[name]` rather than the `[]` dropping the number left).
///
/// A relocation records a byte range and never an operand number, so `pending` is armed
/// once per instruction and *taken* by the first operand asked about; a second numeric
/// operand keeps its real value.
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
        // difference between the two after the name.
        Some(iced_x86::SymbolResult::with_string_kind(
            address,
            name,
            iced_x86::FormatterTextKind::FunctionAddress,
        ))
    }
}

/// The address `instruction` branches to, when it names one in its own encoding.
///
/// A **call** is deliberately not one: control comes straight back to the row underneath.
/// The operand kind is checked as well as the flow control because `near_branch_target`
/// answers 0 for anything that is not a near branch, and 0 is an ordinary address in a
/// relocatable object — `xabort imm8` shares `xbegin`'s flow-control kind and would
/// otherwise draw an arrow to the top of the function.
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

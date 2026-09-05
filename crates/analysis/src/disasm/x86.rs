//! The x86 backend: the only module in the crate that mentions `iced-x86`.

use super::{Code, Disassembler, Instruction, SpanKind};
use iced_x86::Formatter;
use std::{cell::RefCell, rc::Rc};

/// x86 at one of its two decodable widths (32 or 64). 16-bit is not reachable: `object`
/// has no architecture for it.
pub(super) struct X86 {
    pub(super) bitness: u32,
}

impl Disassembler for X86 {
    fn disassemble(&self, code: &Code<'_>) -> Vec<Instruction> {
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

        let mut instructions = Vec::new();

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
            let relocation = match relocation {
                Some(relocation) => relocation.target,
                // No relocation, so the displacement is real: in a linked image it is the
                // function a call reaches, and a symbol starting exactly there is its name.
                None => call_target(&instruction).and_then(|target| code.symbol_at(target)),
            };

            let branch = if relocated {
                None
            } else {
                branch_target(&instruction)
            };
            // Where the instruction goes, for the operand to be a door there: a branch's
            // own address, or a call's where no symbol has named it -- a name is the
            // door, and a relocation placeholder names nowhere.
            let target = if relocated {
                None
            } else {
                branch.or_else(|| call_target(&instruction).filter(|_| relocation.is_none()))
            };

            let mut inst = Instruction {
                address: instruction.ip(),
                bytes: encoded.to_vec(),
                format: Vec::new(),
                relocation,
                relocation_span: None,
                branch_span: None,
                branch,
                target,
                target_span: None,
            };

            // The resolver takes the name, so at most one operand is substituted however
            // many the formatter asks about, and anything left over is cleared here before
            // the next instruction.
            *pending.borrow_mut() = inst
                .relocation
                .as_ref()
                .map(|target| target.display().to_owned());

            // `rip_relative_addresses` is global to the formatter (`format_memory` reads
            // it), so it is flipped per instruction: the `rip+` is kept wherever a
            // relocation covers the operand, since without it `format_memory` folds the
            // displacement into an absolute address the encoding does not have — and a
            // relocated displacement is a placeholder, whether a name is going into it or
            // not. `EIP` counts too — 64-bit code can address relative to it with a `67h`
            // override.
            formatter.options_mut().set_rip_relative_addresses(
                relocated
                    && matches!(
                        instruction.memory_base(),
                        iced_x86::Register::RIP | iced_x86::Register::EIP
                    ),
            );

            formatter.format(&instruction, &mut inst);

            // `write_number` marks every branch-target operand the formatter writes, and a
            // far branch's selector-and-offset are written the same way. Only the
            // instructions that named an address of their own above keep the mark, and a
            // branch's is its `branch_span` too: those are the ones a row can be pointed
            // at.
            if target.is_none() {
                inst.target_span = None;
            }
            inst.branch_span = inst.target_span.filter(|_| branch.is_some());

            instructions.push(inst);
        }

        instructions
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

    /// Every number the formatter prints comes through here, and the branch target is the
    /// one written with a branch's own text kind — a displacement or an immediate is a
    /// plain `Number`. Record where it lands, the way [`write_symbol`](Self::write_symbol)
    /// records a substituted name: it is the span the UI makes clickable.
    ///
    /// The *first* such span, since a far branch writes its selector and its offset both
    /// this way; the decode loop discards the mark for anything that is not a near branch
    /// or call naming an address of its own, and copies it to `branch_span` for a branch.
    fn write_number(
        &mut self,
        _instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        text: &str,
        _value: u64,
        _number_kind: iced_x86::NumberKind,
        kind: iced_x86::FormatterTextKind,
    ) {
        if SpanKind::from(kind) == SpanKind::Address && self.target_span.is_none() {
            self.target_span = Some(self.format.len());
        }
        self.write(text, kind);
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
    near_target(instruction)
}

/// The address `instruction` calls, when it is a direct near `call`: [`branch_target`]'s
/// counterpart for the one kind of branch it leaves out, asked so the function there can be
/// named — never so the gutter draws it. A `jmp` out of the symbol is a tail call and could
/// be named the same way, but its displacement is a branch's own span (`branch_span`) and
/// making it a link to a function is a decision of its own.
fn call_target(instruction: &iced_x86::Instruction) -> Option<u64> {
    (instruction.flow_control() == iced_x86::FlowControl::Call)
        .then(|| near_target(instruction))
        .flatten()
}

/// `near_branch_target` for exactly the operands it means something for.
fn near_target(instruction: &iced_x86::Instruction) -> Option<u64> {
    matches!(
        instruction.op0_kind(),
        iced_x86::OpKind::NearBranch16
            | iced_x86::OpKind::NearBranch32
            | iced_x86::OpKind::NearBranch64
    )
    .then(|| instruction.near_branch_target())
}

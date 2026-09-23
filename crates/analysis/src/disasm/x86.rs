//! The x86 backend: the only module in the crate that mentions `iced-x86`.

use super::{Code, Disassembler, Instruction, Operand, SpanKind, SymbolName};
use crate::{SectionAddress, SymbolData};
use iced_x86::Formatter;
use std::{cell::RefCell, rc::Rc, sync::Arc};

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
            code.address.get(),
            iced_x86::DecoderOptions::NONE,
        );

        let armed = Rc::new(RefCell::new(Armed::default()));
        let mut formatter = iced_x86::IntelFormatter::with_options(
            Some(Box::new(RelocationResolver {
                armed: armed.clone(),
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

            // The decoder counts in plain integers; the address it is at is one of the
            // section's, since that is what it was started from.
            let ip = SectionAddress::new(instruction.ip());

            // Checked: the instruction pointer is the symbol's address plus what has been
            // decoded, both of them numbers out of the file, so a section at the end of
            // the address space wraps it and the offset would index the slice. The listing
            // stops at the wrap.
            let Some(start_index) = code
                .address
                .bytes_to(ip)
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

            // Every relocation in these bytes, with the field each is in, so each name goes
            // to the operand its field encodes and not to whichever the formatter asks
            // about first. An x86 operand has one field a relocation can be in, so two
            // relocations are two operands' (`mov dword ptr [g], f`).
            let relocations: Vec<(Option<Field>, Option<Arc<SymbolData>>)> = code
                .relocations(ip, instruction.len())
                .map(|relocation| {
                    let field = ip.bytes_to(relocation.address).and_then(|offset| {
                        field_at(&decoder.get_constant_offsets(&instruction), offset)
                    });
                    (field, relocation.target)
                })
                .collect();

            // Whether *any* relocation covers these bytes, which is not whether one
            // resolved to something navigable: a branch relocated against a section
            // resolves to `None` and its displacement is a placeholder all the same. Only
            // this question says whether the encoded branch target means anything.
            let relocated = !relocations.is_empty();

            // `rip_relative_addresses` is global to the formatter (`format_memory` reads
            // it), so it is flipped per instruction: the `rip+` is kept wherever a
            // relocation covers the operand, since without it `format_memory` folds the
            // displacement into an absolute address the encoding does not have — and a
            // relocated displacement is a placeholder, whether a name is going into it or
            // not. A relocation in the immediate leaves the displacement real, so that one
            // is folded as though nothing were relocated. `EIP` counts too — 64-bit code can
            // address relative to it with a `67h` override.
            formatter.options_mut().set_rip_relative_addresses(
                relocations
                    .iter()
                    .any(|(field, _)| *field != Some(Field::Immediate))
                    && matches!(
                        instruction.memory_base(),
                        iced_x86::Register::RIP | iced_x86::Register::EIP
                    ),
            );

            // The names the resolver hands out, one per relocation that named something.
            // With no relocation the displacement is real: in a linked image it is the
            // function a call reaches, and a symbol starting exactly there is its name.
            let pending = if relocated {
                relocations
                    .into_iter()
                    .filter_map(|(field, target)| {
                        Some(Pending {
                            symbol: target?,
                            field,
                        })
                    })
                    .collect()
            } else {
                call_target(&instruction)
                    .and_then(|target| code.symbol_at_local(target))
                    .map(|symbol| Pending {
                        symbol,
                        field: None,
                    })
                    .into_iter()
                    .collect()
            };
            *armed.borrow_mut() = Armed {
                pending,
                taken: Vec::new(),
            };

            let mut formatted = Formatted::default();
            formatter.format(&instruction, &mut formatted);

            // Each name the formatter took, with the span `write_symbol` put it in -- the
            // two in the same order, one write per name taken -- and then any name no
            // operand took, to be named beside the row.
            let Armed { pending, taken } = std::mem::take(&mut *armed.borrow_mut());
            let names: Vec<SymbolName> = taken
                .into_iter()
                .enumerate()
                .map(|(at, symbol)| SymbolName {
                    symbol,
                    span: formatted.names.get(at).copied().flatten(),
                })
                .chain(pending.into_iter().map(|pending| SymbolName {
                    symbol: pending.symbol,
                    span: None,
                }))
                .collect();

            // The four kinds of operand, decided once and only here, and by how the
            // address was arrived at rather than by what the instruction is. A relocation
            // settles it whichever operand it covers: a name where it named a text symbol
            // this object kept, and the placeholder itself where none named anything, since
            // a relocated number is a linker's fill whatever it happens to spell. Only what
            // no relocation covers names an address of its own, and only for the rows the
            // formatter printed one for — the mark `write_number` left.
            let operand = if !names.is_empty() {
                Some(Operand::Names(names))
            } else if relocated {
                Some(Operand::Placeholder)
            } else {
                match (
                    branch_target(&instruction),
                    call_target(&instruction),
                    formatted.target,
                ) {
                    (Some(address), _, Some(span)) => Some(Operand::Branch { address, span }),
                    (None, Some(address), Some(span)) => Some(Operand::Call { address, span }),
                    _ => None,
                }
            };

            instructions.push(Instruction {
                address: ip,
                bytes: encoded.to_vec(),
                format: formatted.format,
                operand,
            });
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

/// What the formatter writes one instruction as: its spans, and where the spans a link
/// could be made of landed.
///
/// The formatter's output rather than [`Instruction`] itself, so the crate's own type holds
/// no scratch state and implements no `iced-x86` trait. Which [`Operand`] the spans belong
/// to is the decode loop's decision, made once formatting is done.
#[derive(Default)]
struct Formatted {
    format: Vec<(String, SpanKind)>,

    /// Where each name was substituted ([`write_symbol`](Self::write_symbol)), in the order
    /// they were written: [`None`] for a name written as more than one span.
    names: Vec<Option<usize>>,

    /// The first branch target the formatter printed ([`write_number`](Self::write_number)).
    target: Option<usize>,
}

impl iced_x86::FormatterOutput for Formatted {
    fn write(&mut self, text: &str, kind: iced_x86::FormatterTextKind) {
        self.format.push((text.to_owned(), kind.into()));
    }

    /// Every number the formatter prints comes through here, and the branch target is the
    /// one written with a branch's own text kind — a displacement or an immediate is a
    /// plain `Number`. Record where it lands, the way [`write_symbol`](Self::write_symbol)
    /// records a substituted name: it is the span the UI makes clickable.
    ///
    /// The *first* such span, since a far branch writes its selector and its offset both
    /// this way; the decode loop keeps the mark only for a row whose operand names an
    /// address of its own.
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
        if SpanKind::from(kind) == SpanKind::Address && self.target.is_none() {
            self.target = Some(self.format.len());
        }
        self.write(text, kind);
    }

    /// The formatter got a name back from [`RelocationResolver`], so this is the
    /// placeholder's replacement. Record where it lands: it is a span the UI makes
    /// clickable.
    ///
    /// Only a name that is a single span can be pointed at; anything else falls back to
    /// being named beside the instruction, which is [`SymbolName`]'s `span: None`.
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

        self.names
            .push((self.format.len() == start + 1).then_some(start));
    }
}

/// Hands the formatter a relocation target's name in place of a relocated operand's
/// placeholder value, at the point the operand is written — so the name lands inside
/// whatever syntax surrounds it (`[name]` rather than the `[]` dropping the number left).
///
/// A relocation records a byte range and never an operand number, so the resolver is armed
/// once per instruction with a name per relocation and the field each is in, and a name is
/// *taken* by the first operand asked about that its field encodes: a memory operand for a
/// displacement, an immediate or a branch for an immediate. A name whose relocation is in
/// neither field goes to the first operand asked about that no name's field claims. Any
/// other numeric operand keeps its real value.
struct RelocationResolver {
    armed: Rc<RefCell<Armed>>,
}

/// What the resolver is armed with for one instruction, and what it has handed out.
#[derive(Default)]
struct Armed {
    /// The names no operand has taken yet.
    pending: Vec<Pending>,

    /// The names operands took, in the order the formatter asked for them.
    taken: Vec<Arc<SymbolData>>,
}

/// A name the resolver is armed with, and the field of the instruction its relocation is in.
struct Pending {
    symbol: Arc<SymbolData>,

    /// [`None`] where the relocation is in neither field.
    field: Option<Field>,
}

/// The two fields of an x86 encoding a relocation can be in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    /// A memory operand's displacement.
    Displacement,

    /// An immediate, or a branch's own displacement, which iced counts as one.
    Immediate,
}

/// The field `offset` bytes into the instruction is in, by where the decoder found each.
fn field_at(offsets: &iced_x86::ConstantOffsets, offset: u64) -> Option<Field> {
    let offset = usize::try_from(offset).ok()?;
    let within = |start: usize, size: usize| (start..start.saturating_add(size)).contains(&offset);
    if offsets.has_displacement()
        && within(offsets.displacement_offset(), offsets.displacement_size())
    {
        Some(Field::Displacement)
    } else if (offsets.has_immediate()
        && within(offsets.immediate_offset(), offsets.immediate_size()))
        || (offsets.has_immediate2()
            && within(offsets.immediate_offset2(), offsets.immediate_size2()))
    {
        Some(Field::Immediate)
    } else {
        None
    }
}

/// The field an operand of kind `kind` is encoded in, where it is one a relocation can be in.
fn field_of(kind: iced_x86::OpKind) -> Option<Field> {
    use iced_x86::OpKind::*;
    match kind {
        Memory => Some(Field::Displacement),
        Immediate8 | Immediate8_2nd | Immediate16 | Immediate32 | Immediate64 | Immediate8to16
        | Immediate8to32 | Immediate8to64 | Immediate32to64 | NearBranch16 | NearBranch32
        | NearBranch64 | FarBranch16 | FarBranch32 => Some(Field::Immediate),
        _ => None,
    }
}

impl iced_x86::SymbolResolver for RelocationResolver {
    fn symbol(
        &mut self,
        instruction: &iced_x86::Instruction,
        _operand: u32,
        instruction_operand: Option<u32>,
        address: u64,
        _address_size: u32,
    ) -> Option<iced_x86::SymbolResult<'_>> {
        let mut armed = self.armed.borrow_mut();
        let asked = instruction_operand.and_then(|operand| field_of(instruction.op_kind(operand)));
        // The name for the field this operand is encoded in, or else one whose field is
        // not known.
        let at = armed
            .pending
            .iter()
            .position(|pending| pending.field.is_some() && pending.field == asked)
            .or_else(|| {
                armed
                    .pending
                    .iter()
                    .position(|pending| pending.field.is_none())
            })?;
        let symbol = armed.pending.remove(at).symbol;
        let name = symbol.display().to_owned();
        armed.taken.push(symbol);
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
fn branch_target(instruction: &iced_x86::Instruction) -> Option<SectionAddress> {
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
/// be named the same way, but it is an [`Operand::Branch`] and making that a link to a
/// function is a decision of its own.
fn call_target(instruction: &iced_x86::Instruction) -> Option<SectionAddress> {
    (instruction.flow_control() == iced_x86::FlowControl::Call)
        .then(|| near_target(instruction))
        .flatten()
}

/// `near_branch_target` for exactly the operands it means something for.
fn near_target(instruction: &iced_x86::Instruction) -> Option<SectionAddress> {
    matches!(
        instruction.op0_kind(),
        iced_x86::OpKind::NearBranch16
            | iced_x86::OpKind::NearBranch32
            | iced_x86::OpKind::NearBranch64
    )
    .then(|| SectionAddress::new(instruction.near_branch_target()))
}

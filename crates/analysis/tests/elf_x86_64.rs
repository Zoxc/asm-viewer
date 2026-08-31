//! Parsing, size estimation, disassembly and relocation resolution on a minimal
//! hand-built x86-64 ELF relocatable object.

mod common;

use analysis::{parse_object, Object, SpanKind, SymbolData};
use common::{
    caller_and_target, elf_x86_64, indirect_caller_and_target, rip_relative_store_to_data,
    TextRelocation, TextSymbol,
};
use std::path::PathBuf;
use std::sync::Arc;

fn parse(data: &[u8]) -> Arc<Object> {
    parse_object(data.into(), "fixture.o".into(), PathBuf::from("/fixture.o"))
        .expect("fixture parses")
}

fn symbol(object: &Object, name: &str) -> Arc<SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol named {name}; got {:?}", names(object)))
        .clone()
}

fn names(object: &Object) -> Vec<&str> {
    object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect()
}

#[test]
fn both_text_symbols_parse() {
    let object = parse(&caller_and_target());

    assert_eq!(names(&object), ["caller", "target"]);
    assert_eq!(object.symbols.len(), 2);
    assert_eq!(object.format, analysis::BinaryFormat::Elf);

    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");

    let section = caller.section.as_ref().expect("caller has a section");
    assert_eq!(section.name, ".text");
    // A relocatable object's `.text` starts at 0, so symbol addresses are its offsets.
    assert_eq!(section.address, 0);
    assert_eq!(caller.address, 0);
    assert_eq!(target.address, 6);
    assert_eq!(section.symbols, vec![0, 6]);
}

#[test]
fn estimate_size_is_derived_from_the_next_symbol() {
    let object = parse(&caller_and_target());
    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");

    // Nothing to derive from: the fixture declares no sizes at all.
    assert_eq!(caller.size, 0);
    assert_eq!(target.size, 0);

    // `caller` ends where `target` begins ...
    assert_eq!(caller.estimate_size(), Some(6));
    assert_eq!(
        caller.data(),
        Some(&[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3][..])
    );

    // ... and the last symbol in the section ends at the section end.
    assert_eq!(target.estimate_size(), Some(1));
    assert_eq!(target.data(), Some(&[0xC3][..]));
}

#[test]
fn assembly_decodes_both_instructions() {
    let object = parse(&caller_and_target());
    let caller = symbol(&object, "caller");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    assert_eq!(assembly.instructions.len(), 2);

    let call = &assembly.instructions[0];
    assert_eq!(call.address, 0);
    assert_eq!(call.bytes, [0xE8, 0x00, 0x00, 0x00, 0x00]);
    assert!(
        text(call).starts_with("call"),
        "expected a call, got {:?}",
        text(call)
    );
    assert_eq!(
        call.format.first().map(|(_, kind)| *kind),
        Some(SpanKind::Mnemonic)
    );

    let ret = &assembly.instructions[1];
    assert_eq!(ret.address, 5);
    assert_eq!(ret.bytes, [0xC3]);
    assert!(text(ret).starts_with("ret"), "got {:?}", text(ret));
}

#[test]
fn the_call_resolves_to_the_target_symbol() {
    let object = parse(&caller_and_target());
    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    let resolved = call.relocation.as_ref().expect("the call has a relocation");
    // Identity is `Arc` pointer identity everywhere, so assert that and not the name.
    assert!(Arc::ptr_eq(resolved, &target));

    // The relocation covers only the call; the trailing `ret` has none.
    assert!(assembly.instructions[1].relocation.is_none());
}

#[test]
fn the_placeholder_operand_is_replaced_when_a_relocation_applies() {
    let object = parse(&caller_and_target());
    let caller = symbol(&object, "caller");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    // The encoded displacement is a meaningless placeholder (0), so it must never be
    // printed: the target's name stands in its place, in the operand's own position.
    assert!(
        !call
            .format
            .iter()
            .any(|(_, kind)| *kind == SpanKind::Number),
        "the relocated call kept a number span: {:?}",
        call.format
    );
    assert_eq!(text(call).trim_end(), "call      target");
    assert_eq!(relocation_span(call), Some(("target", SpanKind::Address)));
}

#[test]
fn an_unrelocated_branch_keeps_its_target() {
    // Control for the test above: the same `call rel32` with no relocation on it still
    // prints the encoded (here meaningless) displacement.
    let data = elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[],
    );
    let object = parse(&data);
    let caller = symbol(&object, "caller");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    assert!(call.relocation.is_none());
    // The branch target is a `FunctionAddress`, i.e. `SpanKind::Address` — not a number
    // span — so it is `write_number` being suppressed that removes it above, and the
    // text is what has to be asserted on.
    assert_eq!(text(call).trim_end(), "call      5");
    assert_eq!(
        spans_of(call, SpanKind::Address),
        ["5"],
        "the call target should be an address span: {:?}",
        call.format
    );
}

#[test]
fn jump_targets_are_address_spans_too() {
    // The same rule for a plain jump, whose target iced-x86 emits as a `LabelAddress`
    // rather than the `FunctionAddress` a call gets: a short `EB xx` and a near
    // `E9 xx xx xx xx`, both unrelocated so the encoded displacement is printed.
    let data = elf_x86_64(
        &[TextSymbol {
            name: "jumper",
            bytes: &[0xEB, 0x00, 0xE9, 0x00, 0x00, 0x00, 0x00, 0xC3],
        }],
        &[],
    );
    let object = parse(&data);
    let jumper = symbol(&object, "jumper");

    let assembly = jumper.assembly(&object).expect("jumper disassembles");
    assert_eq!(assembly.instructions.len(), 3);

    let short = &assembly.instructions[0];
    assert_eq!(text(short).trim_end(), "jmp       short 2");
    assert_eq!(spans_of(short, SpanKind::Address), ["2"]);

    let near = &assembly.instructions[1];
    assert_eq!(text(near).trim_end(), "jmp       7");
    assert_eq!(spans_of(near, SpanKind::Address), ["7"]);

    // The `ret` that follows has no operand at all, so nothing else is coloured as one.
    assert!(spans_of(&assembly.instructions[2], SpanKind::Address).is_empty());
}

#[test]
fn a_relocation_drops_an_immediate_number_span() {
    // `mov eax, imm32` does produce a `SpanKind::Number` operand, so this is where the
    // dropped-placeholder rule is visible as a missing number span.
    let symbols = [
        TextSymbol {
            name: "loader",
            bytes: &[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3],
        },
        TextSymbol {
            name: "target",
            bytes: &[0xC3],
        },
    ];

    let plain = parse(&elf_x86_64(&symbols, &[]));
    let mov = symbol(&plain, "loader")
        .assembly(&plain)
        .expect("disassembles")
        .instructions[0]
        .clone();
    assert!(
        mov.format.iter().any(|(_, kind)| *kind == SpanKind::Number),
        "expected a number span for the immediate: {:?}",
        mov.format
    );
    assert_eq!(text(&mov).trim_end(), "mov       eax, 1");

    let relocated = parse(&elf_x86_64(
        &symbols,
        &[TextRelocation {
            in_symbol: 0,
            offset: 1,
            target: 1,
        }],
    ));
    let mov = symbol(&relocated, "loader")
        .assembly(&relocated)
        .expect("disassembles")
        .instructions[0]
        .clone();
    assert!(mov.relocation.is_some());
    assert!(
        !mov.format.iter().any(|(_, kind)| *kind == SpanKind::Number),
        "the relocated immediate kept a number span: {:?}",
        mov.format
    );
    assert_eq!(text(&mov).trim_end(), "mov       eax, target");
    assert_eq!(relocation_span(&mov), Some(("target", SpanKind::Address)));
}

#[test]
fn a_relocation_anywhere_in_the_instruction_counts() {
    // The relocation sits at offset 2, in the middle of the call's displacement.
    let data = elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 2,
            target: 1,
        }],
    );
    let object = parse(&data);
    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let resolved = assembly.instructions[0]
        .relocation
        .as_ref()
        .expect("the call has a relocation");
    assert!(Arc::ptr_eq(resolved, &target));
}

#[test]
fn an_unrelocated_indirect_call_keeps_its_displacement() {
    // The control for the test below: the same `call qword ptr [rip+0x0]` with nothing
    // relocating it prints the encoded displacement, which iced-x86 resolves to the
    // absolute address it points at (the instruction is 6 bytes long and starts at 0).
    let object = parse(&indirect_caller_and_target(false));
    let caller = symbol(&object, "caller");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    assert!(call.relocation.is_none());
    assert_eq!(call.relocation_span, None);
    assert_eq!(text(call).trim_end(), "call      qword ptr [6]");
    assert_eq!(spans_of(call, SpanKind::Number), ["6"]);
}

#[test]
fn a_relocated_indirect_call_names_its_target_inside_the_brackets() {
    // The regression: the relocation applies to a *memory* operand, so the number it
    // replaces sits inside brackets the formatter has already opened. Dropping it left
    // `call qword ptr []`; the target's name has to take its place instead.
    //
    // The operand is rip-relative, and naming it does not make it any less so: the `rip+`
    // stays, because `[target]` would claim an absolute address the encoding does not
    // have. (The control above, which has no name to show, keeps iced-x86's default and
    // prints the absolute address the displacement resolves to.)
    let object = parse(&indirect_caller_and_target(true));
    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    let resolved = call.relocation.as_ref().expect("the call has a relocation");
    assert!(Arc::ptr_eq(resolved, &target));

    assert_eq!(text(call).trim_end(), "call      qword ptr [rip+target]");
    // The placeholder displacement is gone, not merely hidden ...
    assert!(
        spans_of(call, SpanKind::Number).is_empty(),
        "the relocated displacement kept a number span: {:?}",
        call.format
    );
    // ... and the name is one span of its own, which is what the UI turns into the
    // clickable link.
    assert_eq!(relocation_span(call), Some(("target", SpanKind::Address)));
}

#[test]
fn the_relocation_span_is_the_only_one_replaced() {
    // An instruction with a relocated memory operand *and* an unrelated immediate:
    // `mov dword ptr [rip+0x0], 7`, relocated at offset 2. Only the memory operand is
    // named; the immediate is a real value and must survive.
    let data = elf_x86_64(
        &[
            TextSymbol {
                name: "storer",
                bytes: &[
                    0xC7, 0x05, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xC3,
                ],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 2,
            target: 1,
        }],
    );
    let object = parse(&data);
    let storer = symbol(&object, "storer");

    let assembly = storer.assembly(&object).expect("storer disassembles");
    let mov = &assembly.instructions[0];

    assert_eq!(text(mov).trim_end(), "mov       dword ptr [rip+target], 7");
    assert_eq!(spans_of(mov, SpanKind::Number), ["7"]);
    // The `rip+` is *not* part of the link: `relocation_span` still isolates the name.
    assert_eq!(relocation_span(mov), Some(("target", SpanKind::Address)));
    assert_eq!(
        mov.format[mov.relocation_span.unwrap() - 1].0,
        "+",
        "the name should follow the rip and its operator: {:?}",
        mov.format
    );
}

#[test]
fn an_unresolvable_relocation_keeps_the_plain_displacement() {
    // The same `mov dword ptr [rip+0x0], 7`, but relocated against a *data* symbol, which
    // parsing drops along with everything else that is not a text symbol. There is a
    // relocation on the instruction and nothing to navigate to, so there must be no link
    // — and with no name to put after it, no `rip+` either: the operand prints exactly
    // what an unrelocated one prints, the absolute address the displacement resolves to.
    let object = parse(&rip_relative_store_to_data());
    let storer = symbol(&object, "storer");
    assert_eq!(names(&object), ["storer"]);

    let assembly = storer.assembly(&object).expect("storer disassembles");
    let mov = &assembly.instructions[0];

    assert!(mov.relocation.is_none());
    assert_eq!(mov.relocation_span, None);
    assert_eq!(text(mov).trim_end(), "mov       dword ptr [0Ah], 7");
    assert_eq!(spans_of(mov, SpanKind::Number), ["0Ah", "7"]);
}

#[test]
fn the_rip_form_is_per_instruction() {
    // Two identical `call qword ptr [rip+0x0]`, only the first relocated. The rip-relative
    // form is turned on for the instruction being named and must not leak into the next
    // one, which still prints its own absolute address (it starts at 6 and is 6 long).
    let data = elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[
                    0xFF, 0x15, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x15, 0x00, 0x00, 0x00, 0x00, 0xC3,
                ],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 2,
            target: 1,
        }],
    );
    let object = parse(&data);
    let caller = symbol(&object, "caller");

    let assembly = caller.assembly(&object).expect("caller disassembles");

    assert_eq!(
        text(&assembly.instructions[0]).trim_end(),
        "call      qword ptr [rip+target]"
    );
    assert_eq!(
        text(&assembly.instructions[1]).trim_end(),
        "call      qword ptr [0Ch]"
    );
}

#[test]
fn a_direct_relocated_operand_never_gains_a_rip() {
    // The two direct forms, neither of which has a memory operand at all: the rip-relative
    // rule keys off `memory_base`, so both must read exactly as they did before it.
    let object = parse(&caller_and_target());
    let call = symbol(&object, "caller")
        .assembly(&object)
        .expect("disassembles")
        .instructions[0]
        .clone();
    assert_eq!(text(&call).trim_end(), "call      target");
    assert_eq!(relocation_span(&call), Some(("target", SpanKind::Address)));

    let object = parse(&elf_x86_64(
        &[
            TextSymbol {
                name: "loader",
                bytes: &[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 1,
            target: 1,
        }],
    ));
    let mov = symbol(&object, "loader")
        .assembly(&object)
        .expect("disassembles")
        .instructions[0]
        .clone();
    assert_eq!(text(&mov).trim_end(), "mov       eax, target");
    assert_eq!(relocation_span(&mov), Some(("target", SpanKind::Address)));
}

/// The span [`analysis::Instruction::relocation_span`] points at, with its kind.
fn relocation_span(instruction: &analysis::Instruction) -> Option<(&str, SpanKind)> {
    let index = instruction.relocation_span?;
    let (text, kind) = instruction.format.get(index)?;
    Some((text.as_str(), *kind))
}

/// The text of every span in `instruction` that carries `kind`.
fn spans_of(instruction: &analysis::Instruction, kind: SpanKind) -> Vec<&str> {
    instruction
        .format
        .iter()
        .filter(|(_, span)| *span == kind)
        .map(|(text, _)| text.as_str())
        .collect()
}

fn text(instruction: &analysis::Instruction) -> String {
    instruction
        .format
        .iter()
        .map(|(text, _)| text.as_str())
        .collect()
}

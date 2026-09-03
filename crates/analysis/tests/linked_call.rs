//! A direct `call` in a **linked** image, where the linker has consumed the relocation and
//! the displacement in the instruction is the target itself: it still has to name the
//! function it calls, or the only calls that link are the ones no linker has seen.

mod common;

use analysis::SpanKind;
use common::{
    elf_shared_object, parse, pe_dll, symbol, text, ExportedSymbol, SharedObject, TEXT_ADDRESS,
};
use object::{
    write, Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind,
    SymbolScope,
};
use std::sync::Arc;

/// `f` = `call rel32; ret` with the displacement already resolved to `g` = `ret`, six bytes
/// on: `E8 01 00 00 00` reaches the byte after itself plus one.
const TEXT: &[u8] = &[0xE8, 0x01, 0x00, 0x00, 0x00, 0xC3, 0xC3];

const FUNCTIONS: &[ExportedSymbol] = &[
    ExportedSymbol {
        name: "f",
        offset: 0,
        size: 6,
        code: true,
    },
    ExportedSymbol {
        name: "g",
        offset: 6,
        size: 1,
        code: true,
    },
];

/// The span [`analysis::Instruction::relocation_span`] points at, with its kind.
fn relocation_span(instruction: &analysis::Instruction) -> Option<(&str, SpanKind)> {
    let index = instruction.relocation_span?;
    let (text, kind) = instruction.format.get(index)?;
    Some((text.as_str(), *kind))
}

/// What both linked images have to answer: the call names `g`, in the same clickable shape a
/// relocated call's target has, and the `ret` after it names nothing.
fn the_call_names_g(object: &analysis::Object) {
    let f = symbol(object, "f");
    let g = symbol(object, "g");
    assert_eq!(g.address, TEXT_ADDRESS + 6);

    let assembly = f.assembly(object).expect("f disassembles");
    assert_eq!(assembly.instructions.len(), 2);
    let call = &assembly.instructions[0];

    let resolved = call
        .relocation
        .as_ref()
        .expect("the call resolves to a symbol");
    assert!(Arc::ptr_eq(resolved, &g));
    assert_eq!(text(call).trim_end(), "call      g");
    assert_eq!(relocation_span(call), Some(("g", SpanKind::Address)));

    assert!(assembly.instructions[1].relocation.is_none());
}

#[test]
fn a_linked_elf_calls_its_target_by_name() {
    let object = parse(&elf_shared_object(SharedObject {
        text: TEXT,
        dynamic: FUNCTIONS,
        static_symbols: &[],
        entry: None,
        eh_frame: &[],
    }));
    the_call_names_g(&object);
}

#[test]
fn a_linked_pe_calls_its_target_by_name() {
    let object = parse(&pe_dll(TEXT, FUNCTIONS, None));
    the_call_names_g(&object);
}

#[test]
fn a_call_landing_inside_a_function_keeps_its_number() {
    // The same image with the displacement one byte short: `f`'s own `ret` at 5 is where
    // it lands, which is no function's start.
    let bytes = &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3, 0xC3];
    let object = parse(&elf_shared_object(SharedObject {
        text: bytes,
        dynamic: FUNCTIONS,
        static_symbols: &[],
        entry: None,
        eh_frame: &[],
    }));
    let f = symbol(&object, "f");

    let assembly = f.assembly(&object).expect("f disassembles");
    let call = &assembly.instructions[0];
    assert!(call.relocation.is_none());
    assert!(call.relocation_span.is_none());
    assert_eq!(
        text(call).trim_end(),
        format!("call      {:X}h", TEXT_ADDRESS + 5)
    );
}

#[test]
fn an_unrelocated_call_never_reaches_across_sections() {
    // A relocatable object with two code sections, both at 0, so `.text.b`'s `g` sits at
    // the address `.text.a`'s call reaches. In `.text.a` that address is past the section's
    // last byte and nothing's start: the address alone is not a key here, and resolving it
    // through the wrong section would name a function the call cannot reach.
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let a = obj.add_section(Vec::new(), b".text.a".to_vec(), SectionKind::Text);
    let b = obj.add_section(Vec::new(), b".text.b".to_vec(), SectionKind::Text);
    obj.append_section_data(a, &[0xE8, 0x01, 0x00, 0x00, 0x00, 0xC3], 1);
    obj.append_section_data(b, &[0x90; 6], 1);
    obj.append_section_data(b, &[0xC3], 1);
    for (name, section, value) in [("f", a, 0), ("filler", b, 0), ("g", b, 6)] {
        obj.add_symbol(write::Symbol {
            name: name.as_bytes().to_vec(),
            value,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(section),
            flags: SymbolFlags::None,
        });
    }
    let object = parse(&obj.write().expect("writing the fixture object"));

    let f = symbol(&object, "f");
    assert_eq!(symbol(&object, "g").address, 6);

    let assembly = f.assembly(&object).expect("f disassembles");
    let call = &assembly.instructions[0];
    assert!(
        call.relocation.is_none(),
        "resolved across sections to {:?}",
        call.relocation.as_ref().map(|symbol| &symbol.name)
    );
    assert_eq!(text(call).trim_end(), "call      6");
}

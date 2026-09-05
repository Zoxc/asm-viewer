//! A Mach-O object records a relocation as an offset from the start of its section, while
//! its sections are laid out one after another and its symbols are absolute in that space.
//! A code section that is not at 0 is where the two differ, and where a lookup by address
//! finds nothing unless the parse has converted the keys.
//!
//! `object` 0.32 calls only `__TEXT,__text` code, so the object that reaches a disassembly
//! with this to answer is one whose `__text` is not its first section -- what an assembler
//! writes for a file that names a data section before its first instruction.

mod common;

use common::{parse, symbol, text};
use object::{
    write, Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationKind, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};
use std::sync::Arc;

/// Where `__text` starts once 16 bytes of `__const` are in front of it.
const TEXT_ADDRESS: u64 = 16;

/// `__TEXT,__const` and then `__TEXT,__text`, the layout an assembler gives a file that
/// names a data section before its first instruction. The code is `call rel32; ret; ret`,
/// the displacement left at 0 for a linker to fill in, with `caller` on the call, `other`
/// on the first `ret` -- the address that placeholder spells -- and `target`, what the
/// relocation actually names, on the second.
fn text_after_const() -> Vec<u8> {
    let mut obj = write::Object::new(
        BinaryFormat::MachO,
        Architecture::X86_64,
        Endianness::Little,
    );
    // Plain names, so the fixture is read as it is written; Mach-O's own mangling would
    // put an underscore in front of each.
    obj.mangling = write::Mangling::None;

    let constants = obj.add_section(
        b"__TEXT".to_vec(),
        b"__const".to_vec(),
        SectionKind::ReadOnlyData,
    );
    obj.append_section_data(constants, &[0; TEXT_ADDRESS as usize], 1);

    let text = obj.add_section(b"__TEXT".to_vec(), b"__text".to_vec(), SectionKind::Text);
    obj.append_section_data(text, &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3, 0xC3], 1);

    let mut target = None;
    for (name, value) in [("caller", 0), ("other", 5), ("target", 6)] {
        let symbol = obj.add_symbol(write::Symbol {
            name: name.as_bytes().to_vec(),
            value,
            size: 1,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        });
        if name == "target" {
            target = Some(symbol);
        }
    }

    obj.add_relocation(
        text,
        write::Relocation {
            offset: 1,
            size: 32,
            kind: RelocationKind::Relative,
            encoding: RelocationEncoding::X86Branch,
            symbol: target.expect("the fixture declares target"),
            addend: -4,
        },
    )
    .expect("adding the branch relocation");

    obj.write().expect("writing the fixture object")
}

#[test]
fn a_relocation_is_keyed_by_the_address_its_bytes_are_at() {
    let object = parse(&text_after_const());
    let caller = symbol(&object, "caller");
    let section = caller.section.as_ref().expect("caller has a section");
    assert_eq!(section.name, "__text");
    assert_eq!(section.address, TEXT_ADDRESS);
    assert_eq!(
        section.relocations.keys().copied().collect::<Vec<_>>(),
        vec![TEXT_ADDRESS + 1]
    );
}

#[test]
fn a_call_in_a_section_that_is_not_at_zero_resolves_through_its_relocation() {
    let object = parse(&text_after_const());
    let caller = symbol(&object, "caller");
    let target = symbol(&object, "target");
    assert_eq!(caller.address, TEXT_ADDRESS);
    // What the placeholder displacement spells, and so what a missed relocation names.
    assert_eq!(symbol(&object, "other").address, TEXT_ADDRESS + 5);

    let assembly = caller.assembly(&object).expect("caller disassembles");
    let call = &assembly.instructions[0];

    // Miss the relocation and that placeholder stands as a real target, so the reader is
    // shown a call to `other`.
    assert!(Arc::ptr_eq(
        call.relocation.as_ref().expect("the call is relocated"),
        &target
    ));
    assert_eq!(text(call).trim_end(), "call      target");
    // A placeholder names nowhere, so the row is no door and the gutter draws no arrow.
    assert_eq!(call.target, None);
    assert_eq!(call.branch, None);
}

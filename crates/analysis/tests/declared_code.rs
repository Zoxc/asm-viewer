//! Entry points and exported functions, which a stripped shared library declares in
//! places that are not its symbol table.
//!
//! The two fixtures are assembled byte by byte rather than with `object`'s writer, which
//! emits relocatable objects: neither a `.dynsym` nor a PE export directory exists in one.

mod common;

use common::{
    at, elf_shared_object, macho_executable, named, parse, pe_dll, ExportedSymbol, SharedObject,
    MACHO_CODE_OFFSET, TEXT_ADDRESS,
};

/// Four functions back to back, each `nop`s then a `ret`, so every offset below is a real
/// instruction boundary and a listing decoded from it terminates.
const TEXT: &[u8] = &[
    0x90, 0x90, 0x90, 0xC3, // 0: three nops and a ret
    0x90, 0xC3, // 4
    0xC3, // 6
    0x90, 0x90, 0xC3, // 7
];

/// Two functions and one exported *global*, which must not become a function however it
/// is declared.
const EXPORTS: &[ExportedSymbol] = &[
    ExportedSymbol {
        name: "first",
        offset: 0,
        size: 4,
        code: true,
    },
    ExportedSymbol {
        name: "second",
        offset: 4,
        size: 2,
        code: true,
    },
    ExportedSymbol {
        name: "a_global",
        offset: 0,
        size: 8,
        code: false,
    },
];

/// The stripped shape: `.dynsym` holds the exports and `.symtab` is not there at all.
fn stripped(entry: Option<u64>) -> SharedObject<'static> {
    SharedObject {
        text: TEXT,
        dynamic: EXPORTS,
        static_symbols: &[],
        entry,
        eh_frame: &[],
    }
}

#[test]
fn a_shared_object_with_no_symbol_table_still_lists_its_exports() {
    let object = parse(&elf_shared_object(stripped(Some(6))));

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    // `a_global` is an `STT_OBJECT` in `.data`: declared, exported, and not code.
    assert_eq!(names, ["<entry point>", "first", "second"]);

    assert_eq!(named(&object, "first").address, at(TEXT_ADDRESS));
    assert_eq!(named(&object, "second").address, at(TEXT_ADDRESS + 4));
    assert_eq!(
        named(&object, "<entry point>").address,
        at(TEXT_ADDRESS + 6)
    );

    // Each of them landed in `.text`, which is what gives them bytes at all.
    for symbol in &object.symbols_sorted {
        assert_eq!(
            symbol.section.as_ref().map(|section| section.name.as_str()),
            Some(".text"),
        );
    }
}

#[test]
fn a_dll_with_no_coff_symbol_table_still_lists_its_exports() {
    let object = parse(&pe_dll(TEXT, EXPORTS, Some(6)));

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    // The PE export table says nothing about kind, so `a_global` is dropped purely
    // because its address is in `.rdata` rather than in a code section.
    assert_eq!(names, ["<entry point>", "first", "second"]);

    assert_eq!(named(&object, "first").address, at(TEXT_ADDRESS));
    assert_eq!(named(&object, "second").address, at(TEXT_ADDRESS + 4));
    assert_eq!(
        named(&object, "<entry point>").address,
        at(TEXT_ADDRESS + 6)
    );
}

#[test]
fn an_image_declaring_no_entry_point_grows_no_entry_symbol() {
    for object in [
        parse(&elf_shared_object(stripped(None))),
        parse(&pe_dll(TEXT, EXPORTS, None)),
    ] {
        let names: Vec<&str> = object
            .symbols_sorted
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert!(
            !names.contains(&"<entry point>"),
            "entry point invented from an AddressOfEntryPoint of 0: {names:?}",
        );
    }
}

#[test]
fn an_entry_point_on_an_exported_function_is_one_symbol_not_two() {
    // The entry point *is* `second`, which the export table already names.
    let object = parse(&pe_dll(TEXT, EXPORTS, Some(4)));

    let names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(!names.contains(&"<entry point>"), "{names:?}");
}

#[test]
fn a_declaration_carries_no_size_so_the_extent_comes_from_the_next_one() {
    let object = parse(&pe_dll(TEXT, EXPORTS, Some(7)));

    // A PE export table carries no size at all, so the extent is the next declaration's
    // address, exactly as it is for a symbol-table entry declaring none.
    assert_eq!(named(&object, "first").size, None);
    assert_eq!(
        named(&object, "first")
            .estimate_size(&object)
            .map(|extent| extent.bytes),
        Some(4)
    );
    assert_eq!(
        named(&object, "second")
            .estimate_size(&object)
            .map(|extent| extent.bytes),
        Some(3)
    );
    // The last one runs to the end of the section's bytes.
    assert_eq!(
        named(&object, "<entry point>")
            .estimate_size(&object)
            .map(|extent| extent.bytes),
        Some(3)
    );

    // An ELF `.dynsym` does carry one, and there it is a size to trust: the extent is the
    // declaration rather than the derivation, which here agree.
    let elf = parse(&elf_shared_object(stripped(Some(7))));
    assert_eq!(named(&elf, "first").size, Some(4));
    assert_eq!(
        named(&elf, "first")
            .estimate_size(&elf)
            .map(|extent| extent.bytes),
        Some(4)
    );
    assert_eq!(
        named(&elf, "first").extent(&elf).map(|extent| extent.bytes),
        Some(4)
    );
}

#[test]
fn an_exported_function_disassembles() {
    for object in [
        parse(&elf_shared_object(stripped(Some(6)))),
        parse(&pe_dll(TEXT, EXPORTS, Some(6))),
    ] {
        let first = named(&object, "first").clone();
        let assembly = first.assembly(&object).expect("a listing for `first`");
        let text: Vec<String> = assembly
            .instructions
            .iter()
            .map(|instruction| {
                instruction
                    .format
                    .iter()
                    .map(|(span, _)| span.as_str())
                    .collect()
            })
            .collect();
        assert_eq!(text, ["nop", "nop", "nop", "ret"]);
        assert_eq!(assembly.instructions[0].address, at(TEXT_ADDRESS));
    }
}

#[test]
fn a_relocatable_object_declares_no_entry_point_however_the_header_reads() {
    // `Object::entry()` answers 0 for an `.o`, and 0 there is the first byte of the first
    // section — a real function, which must not also become `<entry point>`.
    let object = parse(&common::caller_and_target());

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["caller", "target"]);
}

#[test]
fn an_export_that_is_already_a_symbol_table_entry_is_not_listed_twice() {
    // A library that was *not* stripped declares `first` three times at one address: in
    // `.symtab` as `first_internal`, in `.dynsym` under its exported name, and as the
    // entry point. The symbol table wins.
    let object = parse(&elf_shared_object(SharedObject {
        text: TEXT,
        dynamic: EXPORTS,
        static_symbols: &[ExportedSymbol {
            name: "first_internal",
            offset: 0,
            size: 4,
            code: true,
        }],
        entry: Some(0),
        eh_frame: &[],
    }));

    let at_first = object
        .symbols_sorted
        .iter()
        .filter(|symbol| symbol.address == at(TEXT_ADDRESS))
        .count();
    assert_eq!(at_first, 1);
    assert_eq!(named(&object, "first_internal").address, at(TEXT_ADDRESS));

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["first_internal", "second"]);
}

#[test]
fn a_macho_entry_point_from_lc_main_is_placed_through_its_segment() {
    // `LC_MAIN` states a file offset, not an address. At `__TEXT`'s usual 0x100000000 the
    // offset as it stands is in no section, so the entry point was lost.
    const TEXT: u64 = 0x1_0000_0000;
    let entry = MACHO_CODE_OFFSET + 0x180;
    let object = parse(&macho_executable(TEXT, entry, false));
    assert_eq!(named(&object, "<entry point>").address, at(TEXT + entry));
}

#[test]
fn a_macho_entry_offset_is_never_taken_for_an_address_inside_another_function() {
    // With `__TEXT` at 0x100, the offset as it stands is 0x80 bytes into `_first`.
    const TEXT: u64 = 0x100;
    let entry = MACHO_CODE_OFFSET + 0x180;
    let object = parse(&macho_executable(TEXT, entry, false));
    assert_eq!(named(&object, "<entry point>").address, at(TEXT + entry));
    assert!(
        object
            .symbols_sorted
            .iter()
            .all(|symbol| symbol.address != at(entry)),
        "{:?}",
        common::names(&object),
    );
}

#[test]
fn a_macho_thread_state_without_a_pc_leaves_the_entry_point_to_lc_main() {
    // The `LC_UNIXTHREAD` comes first but stops short of its PC, so the `LC_MAIN` after it
    // is the entry point, and its offset is placed like any other.
    const TEXT: u64 = 0x1_0000_0000;
    let entry = MACHO_CODE_OFFSET + 0x180;
    let object = parse(&macho_executable(TEXT, entry, true));
    assert_eq!(named(&object, "<entry point>").address, at(TEXT + entry));
}

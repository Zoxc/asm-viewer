//! Entry points and exported functions, which a stripped shared library declares in
//! places that are not its symbol table.
//!
//! Both fixtures are built in memory like every other one, but by hand rather than with
//! `object`'s writer: it emits relocatable objects, and neither a `.dynsym` nor a PE
//! export directory exists in one of those. Assembling the two images byte by byte is
//! what lets the suite cover the case at all without committing a `.so` and a `.dll`.

mod common;

use analysis::{parse_object, Object, SymbolData};
use common::{elf_shared_object, pe_dll, ExportedSymbol, SharedObject, TEXT_ADDRESS};
use std::path::PathBuf;
use std::sync::Arc;

/// Four functions back to back, each `nop`s then a `ret`, so every offset below is a
/// real instruction boundary and a listing decoded from it terminates.
const TEXT: &[u8] = &[
    0x90, 0x90, 0x90, 0xC3, // 0: three nops and a ret
    0x90, 0xC3, // 4
    0xC3, // 6
    0x90, 0x90, 0xC3, // 7
];

fn parse(bytes: Vec<u8>) -> Arc<Object> {
    parse_object(bytes.as_slice().into(), "fixture".into(), PathBuf::from("/f"))
        .expect("the fixture should parse")
}

fn named<'a>(object: &'a Object, name: &str) -> &'a Arc<SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no symbol {name:?} among {:?}",
                object
                    .symbols_sorted
                    .iter()
                    .map(|symbol| &symbol.name)
                    .collect::<Vec<_>>()
            )
        })
}

/// Three declarations: two functions, and one exported *global* which must not become a
/// function however it is declared.
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
    }
}

#[test]
fn a_shared_object_with_no_symbol_table_still_lists_its_exports() {
    let object = parse(elf_shared_object(stripped(Some(6))));

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    // `a_global` is an `STT_OBJECT` in `.data`: declared, exported, and not code.
    assert_eq!(names, ["<entry point>", "first", "second"]);

    assert_eq!(named(&object, "first").address, TEXT_ADDRESS);
    assert_eq!(named(&object, "second").address, TEXT_ADDRESS + 4);
    assert_eq!(named(&object, "<entry point>").address, TEXT_ADDRESS + 6);

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
    let object = parse(pe_dll(TEXT, EXPORTS, Some(6)));

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    // The PE export table says nothing about kind, so `a_global` is dropped purely
    // because its address is in `.rdata` rather than in a code section.
    assert_eq!(names, ["<entry point>", "first", "second"]);

    assert_eq!(named(&object, "first").address, TEXT_ADDRESS);
    assert_eq!(named(&object, "second").address, TEXT_ADDRESS + 4);
    assert_eq!(named(&object, "<entry point>").address, TEXT_ADDRESS + 6);
}

#[test]
fn an_image_declaring_no_entry_point_grows_no_entry_symbol() {
    for object in [
        parse(elf_shared_object(stripped(None))),
        parse(pe_dll(TEXT, EXPORTS, None)),
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
    let object = parse(pe_dll(TEXT, EXPORTS, Some(4)));

    let names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(!names.contains(&"<entry point>"), "{names:?}");

    // And the section's address list is still strictly ascending, which is what
    // `estimate_size` binary-searches.
    let section = named(&object, "second").section.clone().unwrap();
    assert_eq!(section.symbols, [TEXT_ADDRESS, TEXT_ADDRESS + 4]);
}

#[test]
fn a_declaration_carries_no_size_so_the_extent_comes_from_the_next_one() {
    let object = parse(pe_dll(TEXT, EXPORTS, Some(7)));

    // The declared size a PE export table can carry is none at all.
    assert_eq!(named(&object, "first").size, 0);

    // ... so the extent is the next declaration's address, exactly as it is for a
    // symbol-table entry declaring 0.
    assert_eq!(named(&object, "first").estimate_size(), Some(4));
    assert_eq!(named(&object, "second").estimate_size(), Some(3));
    // The last one runs to the end of the section's bytes.
    assert_eq!(named(&object, "<entry point>").estimate_size(), Some(3));

    // An ELF `.dynsym` does carry one, and it is kept for display without displacing
    // the derived extent.
    let elf = parse(elf_shared_object(stripped(Some(7))));
    assert_eq!(named(&elf, "first").size, 4);
    assert_eq!(named(&elf, "first").estimate_size(), Some(4));
}

#[test]
fn an_exported_function_disassembles() {
    for object in [
        parse(elf_shared_object(stripped(Some(6)))),
        parse(pe_dll(TEXT, EXPORTS, Some(6))),
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
        assert_eq!(assembly.instructions[0].address, TEXT_ADDRESS);
    }
}

#[test]
fn a_relocatable_object_declares_no_entry_point_however_the_header_reads() {
    // `Object::entry()` answers 0 for an `.o`, and 0 there is the first byte of the
    // first section — a real function, which must not also become `<entry point>`.
    let object = parse(common::caller_and_target());

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
    // A library that was *not* stripped declares `first` twice: once in `.symtab`,
    // where it is called `first_internal`, and once in `.dynsym` under its exported
    // name. Both are declarations of the same address, and the symbol table wins —
    // it is the table that carries a size and the name the code was compiled under.
    let object = parse(elf_shared_object(SharedObject {
        text: TEXT,
        dynamic: EXPORTS,
        static_symbols: &[ExportedSymbol {
            name: "first_internal",
            offset: 0,
            size: 4,
            code: true,
        }],
        // And the entry point is that same address a third time.
        entry: Some(0),
    }));

    let at_first = object
        .symbols_sorted
        .iter()
        .filter(|symbol| symbol.address == TEXT_ADDRESS)
        .count();
    assert_eq!(at_first, 1);
    assert_eq!(named(&object, "first_internal").address, TEXT_ADDRESS);

    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["first_internal", "second"]);

    // The addresses the section holds are still strictly ascending, which is what
    // `estimate_size` binary-searches — a repeat would make it answer 0.
    let section = named(&object, "second").section.clone().unwrap();
    assert_eq!(section.symbols, [TEXT_ADDRESS, TEXT_ADDRESS + 4]);
}

//! The formats whose stated function addresses are not the addresses of the code: a Thumb,
//! MIPS16 or microMIPS function's has bit 0 set, and a PPC64 ELFv1 or XCOFF one names a
//! function descriptor whose first word is the code's address. Each is read through to the
//! code. So are the other code addresses that carry the bit: an import's PLT entry and an
//! FDE's ends.

mod common;

use object::pe;

use analysis::LoadMessage;
use common::{
    arm_pe_dll, arm_thumb_image, at, macho_arm_executable, mips_compressed_image, named, parse,
    ppc64_elfv1_image, ppc64_elfv1_object, xcoff_image, ARMNT_TEXT, ARM_TEXT, MACHO_ARM_TEXT,
    MIPS_TEXT, PPC64_TEXT, XCOFF_DATA, XCOFF_TEXT,
};

fn sorted_names(object: &analysis::Object) -> Vec<&str> {
    let mut names: Vec<&str> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn a_thumb_function_is_at_its_code_and_a_data_symbol_is_left_alone() {
    let object = parse(&arm_thumb_image());

    // `a_datum` is data in `.data`, and not listed. `odd_datum` is data in `.text`, which
    // an export in a code section is listed as, at the address it states.
    assert_eq!(
        sorted_names(&object),
        [
            "<entry point>",
            "<function 0x8014>",
            "arm_fn",
            "odd_datum",
            "thumb_export",
            "thumb_fn"
        ]
    );
    assert_eq!(named(&object, "thumb_fn").address, at(ARM_TEXT));
    assert_eq!(named(&object, "arm_fn").address, at(ARM_TEXT + 4));
    assert_eq!(named(&object, "thumb_export").address, at(ARM_TEXT + 8));
    assert_eq!(named(&object, "odd_datum").address, at(ARM_TEXT + 0xd));
    assert_eq!(named(&object, "<entry point>").address, at(ARM_TEXT + 0x10));
    assert_eq!(named(&object, "thumb_fn").size, Some(4));
    assert_eq!(object.messages, []);
}

#[test]
fn a_mips16_or_micromips_function_is_at_its_code_and_a_data_symbol_is_left_alone() {
    for is_64 in [false, true] {
        let object = parse(&mips_compressed_image(is_64));

        assert_eq!(
            sorted_names(&object),
            [
                "<entry point>",
                "<function 0x400014>",
                "micromips_export",
                "mips16_fn",
                "mips_fn",
                "odd_datum"
            ],
            "64-bit: {is_64}"
        );
        assert_eq!(named(&object, "mips16_fn").address, at(MIPS_TEXT));
        assert_eq!(named(&object, "mips_fn").address, at(MIPS_TEXT + 4));
        assert_eq!(
            named(&object, "micromips_export").address,
            at(MIPS_TEXT + 8)
        );
        assert_eq!(named(&object, "odd_datum").address, at(MIPS_TEXT + 0xd));
        assert_eq!(
            named(&object, "<entry point>").address,
            at(MIPS_TEXT + 0x10)
        );
        assert_eq!(object.messages, [], "64-bit: {is_64}");
    }
}

#[test]
fn an_imports_tagged_plt_entry_is_cleared_in_either_symbol_table() {
    for (image, text) in [
        (arm_thumb_image(), ARM_TEXT),
        (mips_compressed_image(false), MIPS_TEXT),
        (mips_compressed_image(true), MIPS_TEXT),
    ] {
        let object = parse(&image);
        let imports: Vec<_> = object
            .imports
            .iter()
            .map(|import| (import.name.as_str(), import.address))
            .collect();
        assert_eq!(
            imports,
            [
                ("symtab_import", Some(at(text + 0x20))),
                ("plt_import", Some(at(text + 0x30)))
            ]
        );
    }
}

#[test]
fn an_fde_with_tagged_ends_is_the_function_they_bound() {
    for (image, text) in [
        (arm_thumb_image(), ARM_TEXT),
        (mips_compressed_image(false), MIPS_TEXT),
        (mips_compressed_image(true), MIPS_TEXT),
    ] {
        let object = parse(&image);
        let function = object
            .symbols_sorted
            .iter()
            .find(|symbol| symbol.name.starts_with("<function"))
            .expect("the FDE's function is listed");
        assert_eq!(function.address, at(text + 0x14));
        assert_eq!(function.extent(&object).map(|extent| extent.bytes), Some(4));
    }
}

#[test]
fn a_32_bit_arm_pe_export_and_entry_point_are_at_their_code() {
    for machine in [
        pe::IMAGE_FILE_MACHINE_ARMNT,
        pe::IMAGE_FILE_MACHINE_ARM,
        pe::IMAGE_FILE_MACHINE_THUMB,
    ] {
        let object = parse(&arm_pe_dll(machine));

        // `odd_datum` is in `.rdata`, and not listed.
        assert_eq!(sorted_names(&object), ["<entry point>", "thumb_fn"]);
        assert_eq!(
            named(&object, "thumb_fn").address,
            at(ARMNT_TEXT),
            "{:#x}",
            machine.0
        );
        assert_eq!(
            named(&object, "<entry point>").address,
            at(ARMNT_TEXT + 4),
            "{:#x}",
            machine.0
        );
        assert_eq!(object.messages, []);
    }
}

#[test]
fn an_armv7_mach_o_export_and_entry_point_are_at_their_code() {
    for thread in [false, true] {
        let object = parse(&macho_arm_executable(thread));

        // `_thumb_fn` once: the export trie's bit 0 does not make it a second place.
        assert_eq!(
            sorted_names(&object),
            ["<entry point>", "_only_exported", "_thumb_fn"],
            "LC_UNIXTHREAD: {thread}"
        );
        assert_eq!(named(&object, "_thumb_fn").address, at(MACHO_ARM_TEXT));
        assert_eq!(
            named(&object, "_only_exported").address,
            at(MACHO_ARM_TEXT + 8)
        );
        assert_eq!(
            named(&object, "<entry point>").address,
            at(MACHO_ARM_TEXT + 4),
            "LC_UNIXTHREAD: {thread}"
        );
        assert_eq!(object.messages, [], "LC_UNIXTHREAD: {thread}");
    }
}

#[test]
fn a_ppc64_elfv1_function_is_at_the_code_its_descriptor_names() {
    let object = parse(&ppc64_elfv1_image());

    // `.foo` is `foo` again, and `broken`'s descriptor cannot be read.
    assert_eq!(sorted_names(&object), ["<entry point>", "bar", "foo"]);
    let foo = named(&object, "foo");
    assert_eq!(foo.address, at(PPC64_TEXT));
    // The code's size, from `.foo`, and not the descriptor's.
    assert_eq!(foo.size, Some(8));
    assert_eq!(named(&object, "bar").address, at(PPC64_TEXT + 8));
    assert_eq!(named(&object, "bar").size, None);
    assert_eq!(named(&object, "<entry point>").address, at(PPC64_TEXT + 12));
    for symbol in &object.symbols_sorted {
        assert_eq!(
            symbol.section.as_ref().map(|section| section.name.as_str()),
            Some(".text"),
            "{}",
            symbol.name
        );
    }

    assert_eq!(
        object.messages,
        [LoadMessage::UnreadableDescriptors { count: 1 }]
    );
}

#[test]
fn a_ppc64_elfv1_object_reads_its_descriptor_through_the_relocation_that_fills_it() {
    let object = parse(&ppc64_elfv1_object());

    let foo = named(&object, "foo");
    assert_eq!(foo.address, at(8));
    assert_eq!(
        foo.section.as_ref().map(|section| section.name.as_str()),
        Some(".text")
    );
    assert_eq!(object.messages, []);
}

#[test]
fn an_xcoff_entry_point_is_at_the_code_its_descriptor_names() {
    for is_64 in [false, true] {
        let object = parse(&xcoff_image(is_64, XCOFF_DATA));
        assert_eq!(sorted_names(&object), ["<entry point>"], "64-bit: {is_64}");
        assert_eq!(
            named(&object, "<entry point>").address,
            at(XCOFF_TEXT + 8),
            "64-bit: {is_64}"
        );
        assert_eq!(object.messages, [], "64-bit: {is_64}");
    }
}

/// `o_entry` is all ones where a module has no entry point, as in a shared object built
/// without `-e`. That is no descriptor, so nothing is said about one.
#[test]
fn an_xcoff_module_with_no_entry_point_says_nothing() {
    for is_64 in [false, true] {
        let object = parse(&xcoff_image(is_64, u64::MAX));
        assert!(object.symbols_sorted.is_empty(), "64-bit: {is_64}");
        assert_eq!(object.messages, [], "64-bit: {is_64}");
    }
}

#[test]
fn an_xcoff_entry_point_whose_descriptor_no_section_holds_is_left_out() {
    let object = parse(&xcoff_image(false, 0x3000_0000));

    assert!(object.symbols_sorted.is_empty());
    assert_eq!(
        object.messages,
        [LoadMessage::UnreadableDescriptors { count: 1 }]
    );
}

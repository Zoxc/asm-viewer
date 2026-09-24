//! The formats whose stated function addresses are not the addresses of the code: a Thumb
//! function's has bit 0 set, and a PPC64 ELFv1 or XCOFF one names a function descriptor
//! whose first word is the code's address. Each is read through to the code.

mod common;

use analysis::LoadMessage;
use common::{
    arm_thumb_image, at, named, parse, ppc64_elfv1_image, ppc64_elfv1_object, xcoff_image,
    ARM_TEXT, PPC64_TEXT, XCOFF_DATA, XCOFF_TEXT,
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

#[test]
fn an_xcoff_entry_point_whose_descriptor_no_section_holds_is_left_out() {
    let object = parse(&xcoff_image(false, 0x3000_0000));

    assert!(object.symbols_sorted.is_empty());
    assert_eq!(
        object.messages,
        [LoadMessage::UnreadableDescriptors { count: 1 }]
    );
}

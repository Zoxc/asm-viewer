use super::MadeUp;
use crate::SectionAddress;

/// An address in a section's own terms, written as the number a test means by it.
fn at(address: u64) -> SectionAddress {
    SectionAddress::new(address)
}

/// The three spellings, exactly as they are written into `project.toml` and the session
/// file. A bookmark on one of these resolves by its name alone once the binary has been
/// rebuilt, so a changed spelling loses it silently.
#[test]
fn the_three_names_are_spelled_as_they_are_saved() {
    assert_eq!(MadeUp::EntryPoint.to_string(), "<entry point>");
    assert_eq!(
        MadeUp::Function(at(0x140001000)).to_string(),
        "<function 0x140001000>"
    );
    assert_eq!(
        MadeUp::Fragment(at(0x140001000)).to_string(),
        "<fragment 0x140001000>"
    );
    // An address is written `0x0` and not `0`.
    assert_eq!(MadeUp::Function(at(0)).to_string(), "<function 0x0>");
}

/// A name read back, which is what the app saves the structure of instead of the spelling.
/// The address is part of the question: a file may call a symbol anything at all, and one
/// that reads like a made-up name for somewhere else is the file's own name.
#[test]
fn a_name_is_read_back_only_at_the_address_it_spells() {
    assert_eq!(
        MadeUp::of("<entry point>", at(0x10)),
        Some(MadeUp::EntryPoint)
    );
    assert_eq!(
        MadeUp::of("<function 0x10>", at(0x10)),
        Some(MadeUp::Function(at(0x10)))
    );
    assert_eq!(
        MadeUp::of("<fragment 0x10>", at(0x10)),
        Some(MadeUp::Fragment(at(0x10)))
    );
    assert_eq!(MadeUp::of("<function 0x10>", at(0x20)), None);
    assert_eq!(MadeUp::of("main", at(0x10)), None);
    assert_eq!(MadeUp::of("<T as Trait>::f", at(0x10)), None);
}

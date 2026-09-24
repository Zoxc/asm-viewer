use super::{
    covering, Bias, LoadMessage, Object, ObjectData, Section, SectionAddress, Severity, SymbolData,
};
use object::{Architecture, BinaryFormat, SectionIndex, SymbolIndex};
use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::Arc,
};

/// An address in a section's own terms, written as the number a test means by it.
fn at(address: u64) -> SectionAddress {
    SectionAddress::new(address)
}

/// An object whose symbols are `(index, name, address)`, handed over in the order given —
/// which is not the order `Object::new` sorts them into.
fn object(symbols: &[(u32, &str, u64)]) -> Object {
    let symbols: HashMap<_, _> = symbols
        .iter()
        .map(|&(index, name, address)| {
            let data = SymbolData::new(name.to_string(), None, at(address), None, None);
            (SymbolIndex(index as usize), Arc::new(data))
        })
        .collect();
    Object::new(
        "object.o".into(),
        "object.o".to_string(),
        BinaryFormat::Elf,
        Architecture::X86_64,
        symbols,
        Vec::new(),
        ObjectData::from(Vec::new()),
    )
}

/// The lookup is the sort's contract: the whole run of one name, in the file's index
/// order, and nothing of the names either side of it.
#[test]
fn a_name_answers_its_whole_run_in_index_order() {
    let object = object(&[
        (3, "shared", 0x30),
        (1, "before", 0x10),
        (2, "shared", 0x20),
        (4, "zzz", 0x40),
    ]);

    let named = object.symbols_named("shared");
    let places: Vec<_> = named.iter().map(|data| data.address.get()).collect();
    // Index 2 before index 3, whatever order the map iterated them in.
    assert_eq!(places, [0x20, 0x30]);

    assert_eq!(object.symbols_named("before").len(), 1);
    assert!(object.symbols_named("shar").is_empty());
    assert!(object.symbols_named("shared2").is_empty());
    assert!(object.symbols_named("").is_empty());
}

/// Every way a range can miss the bytes answers [`None`] rather than panicking: the numbers
/// in one came out of a file.
#[test]
fn bytes_in_answers_only_for_a_range_inside_the_bytes() {
    let section = Section::text(
        SectionIndex(1),
        ".text".to_string(),
        vec![0, 1, 2, 3, 4, 5, 6, 7],
        at(0x1000),
        BTreeMap::new(),
        Bias::NONE,
    );

    assert_eq!(
        section.bytes_in(at(0x1002)..at(0x1005)),
        Some(&[2, 3, 4][..])
    );
    assert_eq!(
        section.bytes_in(at(0x1000)..at(0x1008)),
        Some(&section.code().unwrap().data[..])
    );
    // An empty range inside the bytes is an empty slice, not a miss.
    assert_eq!(section.bytes_in(at(0x1004)..at(0x1004)), Some(&[][..]));

    // Before the section, past its end, and end before start.
    assert_eq!(section.bytes_in(at(0x0FFF)..at(0x1002)), None);
    assert_eq!(section.bytes_in(at(0x1004)..at(0x1009)), None);
    assert_eq!(section.bytes_in(at(0x1005)..at(0x1002)), None);
    // A length far past the bytes, whether or not it fits a `usize`.
    assert_eq!(section.bytes_in(at(0x1000)..at(u64::MAX)), None);

    // A section with no bytes answers for nothing.
    let empty = Section::other(SectionIndex(2), ".debug_info".to_string(), at(0x1000));
    assert_eq!(empty.bytes_in(at(0x1000)..at(0x1000)), None);
}

/// The three ways an address misses, and the one nesting answer the callers depend on.
#[test]
fn covering_answers_only_for_the_last_start_at_or_before() {
    let ranges = [0x10..0x20, 0x30..0x38, 0x40..0x48];
    let covers = |address| covering(&ranges, Range::clone, address);

    assert_eq!(covers(0x10), Some(0));
    assert_eq!(covers(0x1F), Some(0));
    assert_eq!(covers(0x30), Some(1));
    assert_eq!(covers(0x47), Some(2));

    // Nothing at all: no range starts at or before the address.
    assert_eq!(covers(0x0F), None);
    assert!(covering(&[] as &[Range<u64>], Range::clone, 0x10).is_none());
    // The gap after a range, and past the last one.
    assert_eq!(covers(0x20), None);
    assert_eq!(covers(0x3F), None);
    assert_eq!(covers(0x48), None);

    // Only the last start at or before is looked at, so an address past an inner range is
    // not answered with the outer one that still contains it.
    let nested = [0x10..0x40, 0x20..0x28];
    assert_eq!(covering(&nested, Range::clone, 0x30), None);
}

/// The one test of what a load message says; every other test matches on the variant.
#[test]
fn a_load_message_says_what_went_wrong_and_names_what_it_carries() {
    let overlap = LoadMessage::CodeSectionsOverlap {
        section: Some(".text.high".to_owned()),
        address: 0xffff_fffb,
    };
    assert_eq!(overlap.severity(), Severity::Error);
    assert_eq!(
        overlap.to_string(),
        "The code sections could not be placed apart: section `.text.high` states the address \
         0xfffffffb, near the top of the address space, so addresses in this object overlap."
    );
    let unnamed = LoadMessage::CodeSectionsOverlap {
        section: None,
        address: 0xffff_fffb,
    };
    assert!(unnamed.to_string().contains("section states the address"));

    let unread = LoadMessage::UnreadableDescriptors { count: 2 };
    assert_eq!(unread.severity(), Severity::Warning);
    assert_eq!(
        unread.to_string(),
        "Functions left out because their descriptors could not be read: 2."
    );
}

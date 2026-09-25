use super::{
    covering, Bias, FirstCovering, LoadMessage, Object, ObjectData, Section, SectionAddress,
    Severity, SymbolData,
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

/// The first range in the order given wins where two overlap, whichever starts first, and
/// an address answers exactly what `find` over the list would.
#[test]
fn first_covering_answers_what_find_would() {
    let ranges = [
        (0x20..0x30, 'a'),
        (0x10..0x40, 'b'),
        (0x28..0x50, 'c'),
        (0x60..0x60, 'd'),
        (
            Range {
                start: 0x70,
                end: 0x68,
            },
            'e',
        ),
        (0x80..0x90, 'f'),
        (0x90..0x98, 'g'),
    ];
    let lookup = FirstCovering::new(ranges.iter().cloned());
    for address in 0..0xA0 {
        let found = ranges
            .iter()
            .find(|(range, _)| range.contains(&address))
            .map(|&(_, value)| value);
        assert_eq!(lookup.get(address), found, "at {address:#x}");
    }
    assert_eq!(lookup.get(0x2C), Some('a'));
    assert_eq!(lookup.get(0x18), Some('b'));
    assert_eq!(lookup.get(0x3C), Some('b'));
    assert_eq!(lookup.get(0x48), Some('c'));
    assert_eq!(lookup.get(0x90), Some('g'));

    // Ends at the top of the type are ends like any other.
    let top = FirstCovering::new([(u64::MAX - 1..u64::MAX, 1), (0..u64::MAX, 2)]);
    assert_eq!(top.get(u64::MAX - 1), Some(1));
    assert_eq!(top.get(0), Some(2));
    assert_eq!(top.get(u64::MAX), None);
}

/// The one test of what a load message says; every other test matches on the variant.
#[test]
fn a_load_message_says_what_went_wrong_and_names_what_it_carries() {
    let overlap = LoadMessage::CodeSectionsOverlap {
        section: ".text.high".to_owned(),
        address: 0xffff_fffb,
    };
    assert_eq!(overlap.severity(), Severity::Error);
    assert_eq!(
        overlap.to_string(),
        "The code sections could not be placed apart: section `.text.high` states the address \
         0xfffffffb, near the top of the address space, so addresses in this object overlap."
    );
    let unread = LoadMessage::UnreadableDescriptors { count: 2 };
    assert_eq!(unread.severity(), Severity::Warning);
    assert_eq!(
        unread.to_string(),
        "Functions left out because their descriptors could not be read: 2."
    );

    let unnamed = LoadMessage::UnreadableSectionNames { count: 1 };
    assert_eq!(unnamed.severity(), Severity::Warning);
    assert_eq!(
        unnamed.to_string(),
        "Sections named by their index because their names could not be read: 1."
    );

    let cut = LoadMessage::ArchiveCutShort { member: 2 };
    assert_eq!(cut.severity(), Severity::Warning);
    assert_eq!(
        cut.to_string(),
        "The archive's member 2 would not read, so it and every member after it are not shown."
    );

    let thin = LoadMessage::ThinArchive { members: 3 };
    assert_eq!(thin.severity(), Severity::Warning);
    assert_eq!(
        thin.to_string(),
        "The archive is thin: its members are in other files, which are not opened. \
         Members not shown: 3."
    );

    let unreadable = LoadMessage::UnreadableMembers { count: 2 };
    assert_eq!(unreadable.severity(), Severity::Warning);
    assert_eq!(
        unreadable.to_string(),
        "Archive members left out because they are not object files this reader can read: 2."
    );

    let skipped = LoadMessage::DebugInfoSkipped { count: 1 };
    assert_eq!(skipped.severity(), Severity::Warning);
    assert_eq!(
        skipped.to_string(),
        "Parts of the debug info that would not read, so some code has no source lines: 1."
    );

    let whole = [
        (
            LoadMessage::EmptyArchive,
            "The archive holds no object files.",
        ),
        (
            LoadMessage::NotAnObject,
            "This file is not an object file or an archive.",
        ),
        (
            LoadMessage::Malformed {
                error: "Invalid ELF header".to_owned(),
            },
            "This file would not parse: Invalid ELF header.",
        ),
        (
            LoadMessage::CouldNotRead {
                error: "not a regular file".to_owned(),
            },
            "This file could not be read: not a regular file.",
        ),
    ];
    for (message, words) in whole {
        assert_eq!(message.severity(), Severity::Warning);
        assert_eq!(message.to_string(), words);
    }
}

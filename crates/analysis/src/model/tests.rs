use super::{Object, ObjectData, Section, SymbolData};
use object::{Architecture, BinaryFormat, SectionIndex, SymbolIndex};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

/// An object whose symbols are `(index, name, address)`, handed over in the order given —
/// which is not the order `Object::new` sorts them into.
fn object(symbols: &[(u32, &str, u64)]) -> Object {
    let symbols: HashMap<_, _> = symbols
        .iter()
        .map(|&(index, name, address)| {
            let data = SymbolData::new(name.to_string(), None, address, None, 0);
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
    let places: Vec<_> = named.iter().map(|data| data.address).collect();
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
        0x1000,
        BTreeMap::new(),
        0,
    );

    assert_eq!(section.bytes_in(0x1002..0x1005), Some(&[2, 3, 4][..]));
    assert_eq!(
        section.bytes_in(0x1000..0x1008),
        Some(&section.data.as_ref().unwrap()[..])
    );
    // An empty range inside the bytes is an empty slice, not a miss.
    assert_eq!(section.bytes_in(0x1004..0x1004), Some(&[][..]));

    // Before the section, past its end, and end before start.
    assert_eq!(section.bytes_in(0x0FFF..0x1002), None);
    assert_eq!(section.bytes_in(0x1004..0x1009), None);
    assert_eq!(section.bytes_in(0x1005..0x1002), None);
    // A length far past the bytes, whether or not it fits a `usize`.
    assert_eq!(section.bytes_in(0x1000..u64::MAX), None);

    // A section with no bytes answers for nothing.
    let empty = Section::other(SectionIndex(2), ".debug_info".to_string(), 0x1000);
    assert_eq!(empty.bytes_in(0x1000..0x1000), None);
}

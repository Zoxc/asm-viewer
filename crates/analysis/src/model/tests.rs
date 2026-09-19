use super::{Object, ObjectData, SymbolData};
use object::{Architecture, BinaryFormat, SymbolIndex};
use std::{collections::HashMap, sync::Arc};

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

use std::collections::HashMap;
use std::path::PathBuf;

use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolData};

use super::*;

/// A bare `Object` with the given text symbols — only the fields [`pick`] compares, which
/// is the `Arc`s themselves.
fn object(name: &str, symbols: &[&str]) -> Arc<Object> {
    let section = Arc::new(Section {
        index: SectionIndex(0),
        name: ".text".into(),
        data: vec![0xC3; symbols.len()],
        address: 0,
        relocations: HashMap::new(),
        symbols: (0..symbols.len() as u64).collect(),
        code: true,
        bias: 0,
    });

    let symbols_sorted: Vec<Arc<SymbolData>> = symbols
        .iter()
        .enumerate()
        .map(|(address, name)| {
            Arc::new(SymbolData {
                name: (*name).to_owned(),
                demangled: None,
                address: address as u64,
                section: Some(section.clone()),
                size: 0,
            })
        })
        .collect();

    Arc::new(Object {
        path: PathBuf::from("/tmp/lib.a"),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted,
        sections: vec![section],
        data: ObjectData::from(b"bytes".as_slice()),
        dwarf: Default::default(),
    })
}

/// Every symbol of `object`, in its own order — what a query would answer with if the
/// whole object held the line.
fn all(object: &Arc<Object>) -> Vec<Symbol> {
    object
        .symbols_sorted
        .iter()
        .map(|data| Symbol {
            object: object.clone(),
            data: data.clone(),
        })
        .collect()
}

#[test]
fn nothing_compiled_from_it_is_nothing_to_pick() {
    assert!(pick(&[], &[]).is_none());
}

#[test]
fn with_nowhere_visited_the_first_wins() {
    let object = object("a.o", &["one", "two", "three"]);
    let candidates = all(&object);

    let picked = pick(&candidates, &[]).expect("three candidates");
    assert!(picked == candidates[0]);
}

#[test]
fn the_most_recently_visited_candidate_wins() {
    let object = object("a.o", &["one", "two", "three"]);
    let candidates = all(&object);

    // Newest first, so `three` is where the reader has just been and `two` is older.
    let recent = vec![candidates[2].clone(), candidates[1].clone()];

    let picked = pick(&candidates, &recent).expect("three candidates");
    assert!(picked == candidates[2], "the older visit won");
}

/// The head of `recent` is the symbol already on screen, and it is what keeps reading down
/// one instantiation from walking across them.
#[test]
fn the_symbol_on_screen_beats_an_older_visit() {
    let object = object("a.o", &["one", "two", "three"]);
    let candidates = all(&object);

    let shown = candidates[1].clone();
    let recent = vec![shown.clone(), candidates[2].clone()];

    let picked = pick(&candidates, &recent).expect("three candidates");
    assert!(picked == shown);
}

/// A history is mostly symbols this line has nothing to do with, so the walk has to skip
/// them rather than stop at them.
#[test]
fn somewhere_visited_that_is_not_a_candidate_is_skipped() {
    let here = object("a.o", &["one", "two"]);
    let elsewhere = object("b.o", &["other"]);
    let candidates = all(&here);

    let recent = vec![all(&elsewhere)[0].clone(), candidates[1].clone()];

    let picked = pick(&candidates, &recent).expect("two candidates");
    assert!(picked == candidates[1]);
}

/// Two objects can hold symbols of the same name at the same address — an archive holding
/// one function once per member — and they are different answers.
#[test]
fn one_name_in_two_objects_stays_two_candidates() {
    let first = object("a.o", &["shared"]);
    let second = object("b.o", &["shared"]);
    let candidates = vec![all(&first)[0].clone(), all(&second)[0].clone()];

    let picked = pick(&candidates, &[candidates[1].clone()]).expect("two candidates");
    assert!(picked == candidates[1]);
    assert!(picked != candidates[0]);
}

use std::collections::BTreeMap;
use std::path::PathBuf;

use analysis::{
    Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolData, SymbolIndex,
};

use super::*;

/// A bare `Object` with the given text symbols — only the fields [`pick`] compares, which
/// is the `Arc`s themselves.
fn object(name: &str, symbols: &[&str]) -> Arc<Object> {
    let bytes = vec![0xC3; symbols.len()];
    let section = Arc::new(Section::text(
        SectionIndex(0),
        ".text".into(),
        bytes,
        0,
        BTreeMap::new(),
        0,
    ));

    let symbols = symbols
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let address = index as u64;
            let symbol =
                SymbolData::new((*name).to_owned(), None, address, Some(section.clone()), 0);
            (SymbolIndex(index), Arc::new(symbol))
        })
        .collect();

    Arc::new(Object::new(
        PathBuf::from("/tmp/lib.a"),
        name.to_owned(),
        BinaryFormat::Elf,
        Architecture::X86_64,
        symbols,
        vec![section],
        ObjectData::from(b"bytes".as_slice()),
    ))
}

/// Every symbol of `object`, in its name order — what a query would answer with if the
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

    // Newest first, so the third is where the reader has just been and the second is older.
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

/// A symbol in a section that was placed somewhere: what the section view draws it at.
fn placed(name: &str, address: u64, bias: u64) -> Arc<SymbolData> {
    let section = Section::text(
        SectionIndex(0),
        ".text".into(),
        Vec::new(),
        0,
        BTreeMap::new(),
        bias,
    );
    Arc::new(SymbolData::new(
        name.to_owned(),
        None,
        address,
        Some(Arc::new(section)),
        0,
    ))
}

/// The place a listing opens at is the lowest **placed** address, which is not the lowest
/// raw one: with two code sections, the symbol at the lower raw address can be drawn later.
/// Nor is it the first of a slice that is not in placed order.
#[test]
fn the_lowest_placed_address_is_not_the_first() {
    // Raw order: `early` at 0x10 comes first, but its section sits above the other's.
    let symbols = [placed("early", 0x10, 0x2000), placed("late", 0x40, 0x1000)];
    assert_eq!(lowest_placed(&symbols), Some(0x1040));

    // One section is the ordinary case, and there the two agree.
    let one = [placed("a", 0x40, 0x1000), placed("b", 0x10, 0x1000)];
    assert_eq!(lowest_placed(&one), Some(0x1010));
}

/// A symbol in no section is in no listing either, and nothing at all is no answer.
#[test]
fn a_symbol_with_no_section_is_nowhere_to_open() {
    let loose = Arc::new(SymbolData::new("absolute".to_owned(), None, 0x10, None, 0));
    assert_eq!(lowest_placed(&[loose.clone()]), None);
    // And it is stepped over rather than taken as the lowest.
    assert_eq!(lowest_placed(&[loose, placed("a", 0x40, 0)]), Some(0x40));
    assert_eq!(lowest_placed(&[]), None);
}

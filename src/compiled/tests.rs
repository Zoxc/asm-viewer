use std::collections::BTreeMap;
use std::path::PathBuf;

use analysis::{
    Architecture, Bias, BinaryFormat, ObjectData, Section, SectionAddress, SectionIndex,
    SymbolData, SymbolIndex,
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
        SectionAddress::new(0),
        BTreeMap::new(),
        Bias::NONE,
    ));

    let symbols = symbols
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let address = index as u64;
            let symbol = SymbolData::new(
                (*name).to_owned(),
                None,
                SectionAddress::new(address),
                Some(section.clone()),
                None,
            );
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

/// An object of code sections, each 0x80 bytes from 0 and placed by its bias, holding
/// `symbols` -- a name, which section and an address -- which come back in that order.
fn laid_out(biases: &[u64], symbols: &[(&str, usize, u64)]) -> (Object, Vec<Arc<SymbolData>>) {
    let sections: Vec<_> = biases
        .iter()
        .enumerate()
        .map(|(index, &bias)| {
            Arc::new(Section::text(
                SectionIndex(index),
                format!(".text.{index}"),
                vec![0xC3; 0x80],
                SectionAddress::new(0),
                BTreeMap::new(),
                Bias::new(bias),
            ))
        })
        .collect();
    let symbols: Vec<_> = symbols
        .iter()
        .map(|&(name, section, address)| {
            Arc::new(SymbolData::new(
                name.to_owned(),
                None,
                SectionAddress::new(address),
                Some(sections[section].clone()),
                None,
            ))
        })
        .collect();
    let object = Object::new(
        PathBuf::from("/tmp/image"),
        "image".to_owned(),
        BinaryFormat::Elf,
        Architecture::X86_64,
        (symbols.iter().cloned().enumerate())
            .map(|(index, symbol)| (SymbolIndex(index), symbol))
            .collect(),
        sections,
        ObjectData::from(b"bytes".as_slice()),
    );
    (object, symbols)
}

/// The place a listing opens at is the lowest **placed** address, which is not the lowest
/// raw one: with two code sections, the symbol at the lower raw address can be drawn later.
/// Nor is it the first of a slice that is not in placed order.
#[test]
fn the_lowest_placed_address_is_not_the_first() {
    // Raw order: `early` at 0x10 comes first, but its section sits above the other's.
    let (object, symbols) = laid_out(&[0x2000, 0x1000], &[("early", 0, 0x10), ("late", 1, 0x40)]);
    assert_eq!(
        lowest_placed(&object, &symbols),
        Some(PlacedAddress::new(0x1040))
    );

    // One section is the ordinary case, and there the two agree.
    let (object, one) = laid_out(&[0x1000], &[("a", 0, 0x40), ("b", 0, 0x10)]);
    assert_eq!(
        lowest_placed(&object, &one),
        Some(PlacedAddress::new(0x1010))
    );
}

/// A symbol in no section is in no listing either, and nothing at all is no answer.
#[test]
fn a_symbol_with_no_section_is_nowhere_to_open() {
    let (object, placed) = laid_out(&[0], &[("a", 0, 0x40)]);
    let loose = Arc::new(SymbolData::new(
        "absolute".to_owned(),
        None,
        SectionAddress::new(0x10),
        None,
        None,
    ));
    assert_eq!(lowest_placed(&object, std::slice::from_ref(&loose)), None);
    // And it is stepped over rather than taken as the lowest.
    assert_eq!(
        lowest_placed(&object, &[loose, placed[0].clone()]),
        Some(PlacedAddress::new(0x40))
    );
    assert_eq!(lowest_placed(&object, &[]), None);
}

/// Two sections placed on top of each other: the listing of all the code draws only the
/// first, so `b`, lower but in the second, is nowhere in it. Its placed address would land
/// on the first section's bytes.
#[test]
fn a_symbol_in_a_section_the_listing_leaves_out_is_nowhere_to_open() {
    let (object, symbols) = laid_out(&[0x1000, 0x1000], &[("b", 1, 0x10), ("a", 0, 0x40)]);
    assert_eq!(
        lowest_placed(&object, &symbols),
        Some(PlacedAddress::new(0x1040))
    );
    assert_eq!(lowest_placed(&object, &symbols[..1]), None);
}

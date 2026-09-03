use std::collections::HashMap;
use std::path::PathBuf;

use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, Symbol, SymbolData};

use super::*;
use crate::project::Selection;

/// A bare `Object` with the given text symbols, sorted by name as the parser leaves them.
fn object(name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
    let section = Arc::new(Section {
        index: SectionIndex(0),
        name: ".text".into(),
        data: vec![0xC3; symbols.len()],
        address: 0,
        relocations: HashMap::new(),
        symbols: symbols.iter().map(|(_, address)| *address).collect(),
        unwind: Vec::new(),
        code: true,
        bias: 0,
    });
    let mut symbols_sorted: Vec<Arc<SymbolData>> = symbols
        .iter()
        .map(|(name, address)| {
            Arc::new(SymbolData {
                name: (*name).to_owned(),
                demangled: Some(format!("demangled::{name}")),
                address: *address,
                section: Some(section.clone()),
                size: 0,
            })
        })
        .collect();
    symbols_sorted.sort_by(|a, b| a.name.cmp(&b.name));
    Arc::new(Object {
        path: PathBuf::from("/tmp/lib.a"),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted,
        sections: vec![section],
        data: ObjectData::from(&b"a build"[..]),
        debug_info: Default::default(),
    })
}

fn symbol(object: &Arc<Object>, index: usize) -> Document {
    Document::Assembly(Selection::Symbol(Symbol {
        object: object.clone(),
        data: object.symbols_sorted[index].clone(),
    }))
}

fn names(bookmarks: &Bookmarks) -> Vec<&str> {
    bookmarks
        .entries()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect()
}

/// Adding is appending, in the reader's order; toggling the same place again removes it and
/// leaves the others where they were.
#[test]
fn toggling_adds_at_the_end_and_removes_in_place() {
    let object = object("a.o", &[("first", 0), ("second", 8), ("third", 16)]);
    let objects = [object.clone()];
    let mut bookmarks = Bookmarks::default();

    assert!(bookmarks.toggle(&symbol(&object, 2), "third", &objects));
    assert!(bookmarks.toggle(&symbol(&object, 0), "first", &objects));
    let file = Document::Source(Arc::from("/src/main.rs"));
    assert!(bookmarks.toggle(&file, "main.rs", &objects));
    assert_eq!(names(&bookmarks), ["third", "first", "main.rs"]);

    assert!(!bookmarks.toggle(&symbol(&object, 2), "third", &objects));
    assert_eq!(names(&bookmarks), ["first", "main.rs"]);
    assert_eq!(bookmarks.matching(&symbol(&object, 0), &objects), Some(0));
    assert_eq!(bookmarks.matching(&file, &objects), Some(1));
    assert_eq!(bookmarks.matching(&symbol(&object, 2), &objects), None);
}

/// Membership is asked by resolution and not of the saved form: a rebuild that moved the
/// symbol leaves the entry at its old address, and the entry still answers for the symbol
/// where it is now.
#[test]
fn a_moved_symbol_still_matches_its_bookmark() {
    let before = object("a.o", &[("caller", 0), ("target", 6)]);
    let mut bookmarks = Bookmarks::default();
    assert!(bookmarks.toggle(&symbol(&before, 1), "target", &[before.clone()]));

    let after = object("a.o", &[("caller", 0), ("target", 96)]);
    let moved = symbol(&after, 1);
    assert_ne!(
        SavedDocument::from_document(&moved),
        bookmarks.entries()[0].document,
        "the saved forms disagree about the address"
    );
    assert_eq!(bookmarks.matching(&moved, &[after.clone()]), Some(0));
    // And so toggling from the moved symbol removes rather than duplicates.
    assert!(!bookmarks.toggle(&moved, "target", &[after.clone()]));
    assert!(bookmarks.entries().is_empty());
}

/// A bookmark whose object is gone matches nothing and is kept; `remove` is how it goes,
/// and an index past the end is nothing to remove.
#[test]
fn a_dead_bookmark_is_kept_until_removed_by_index() {
    let object = object("a.o", &[("target", 6)]);
    let mut bookmarks = Bookmarks::default();
    bookmarks.toggle(&symbol(&object, 0), "target", &[object.clone()]);

    assert_eq!(bookmarks.matching(&symbol(&object, 0), &[]), None);
    assert_eq!(bookmarks.entries().len(), 1);
    bookmarks.remove(5);
    assert_eq!(bookmarks.entries().len(), 1);
    bookmarks.remove(0);
    assert!(bookmarks.entries().is_empty());
}

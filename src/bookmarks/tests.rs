use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use analysis::{
    Architecture, Bias, BinaryFormat, ObjectData, Section, SectionAddress, SectionIndex, Symbol,
    SymbolData, SymbolIndex,
};

use super::*;

/// A bare `Object` with the given text symbols, which `symbols_sorted` holds by name.
fn object(name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
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
        .map(|(index, (name, address))| {
            let demangled = Some(format!("demangled::{name}"));
            let symbol = SymbolData::new(
                (*name).to_owned(),
                demangled,
                SectionAddress::new(*address),
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
        ObjectData::from(&b"a build"[..]),
    ))
}

fn symbol(object: &Arc<Object>, index: usize) -> Document {
    Document::Symbol(Symbol {
        object: object.clone(),
        data: object.symbols_sorted[index].clone(),
    })
}

fn names(bookmarks: &Bookmarks) -> Vec<Cow<'_, str>> {
    bookmarks
        .entries()
        .iter()
        .map(|entry| entry.label())
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
    let file = Document::Source(Arc::from(Path::new("/src/main.rs")));
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
    assert!(bookmarks.toggle(&symbol(&before, 1), "target", std::slice::from_ref(&before)));

    let after = object("a.o", &[("caller", 0), ("target", 96)]);
    let moved = symbol(&after, 1);
    assert_ne!(
        SavedDocument::from_document(&moved),
        bookmarks.entries()[0].document,
        "the saved forms disagree about the address"
    );
    assert_eq!(
        bookmarks.matching(&moved, std::slice::from_ref(&after)),
        Some(0)
    );
    // And so toggling from the moved symbol removes rather than duplicates.
    assert!(!bookmarks.toggle(&moved, "target", std::slice::from_ref(&after)));
    assert!(bookmarks.entries().is_empty());
}

/// A bookmark whose object is gone matches nothing and is kept; `remove` is how it goes,
/// and a bookmark not in the list is nothing to remove.
#[test]
fn a_dead_bookmark_is_kept_until_removed() {
    let object = object("a.o", &[("target", 6)]);
    let mut bookmarks = Bookmarks::default();
    bookmarks.toggle(&symbol(&object, 0), "target", std::slice::from_ref(&object));
    let dead = bookmarks.entries()[0].clone();

    assert_eq!(bookmarks.matching(&symbol(&object, 0), &[]), None);
    assert_eq!(bookmarks.entries().len(), 1);
    let file = Document::Source(Arc::from(Path::new("/src/main.rs")));
    bookmarks.remove(&Bookmark::new(
        SavedDocument::from_document(&file),
        "main.rs",
    ));
    assert_eq!(bookmarks.entries().len(), 1);
    bookmarks.remove(&dead);
    assert!(bookmarks.entries().is_empty());
}

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use analysis::{Architecture, BinaryFormat, Object, ObjectData};

use super::*;
use crate::document::Selection;

/// A distinct document: two calls with the same `name` still produce different `Arc`s, and
/// so entries that do not compare equal.
fn place(name: &str) -> Document {
    Document::Assembly(Selection::Object(Arc::new(Object {
        path: PathBuf::from("/tmp/lib.a"),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted: Vec::new(),
        sections: Vec::new(),
        data: ObjectData::from(&b""[..]),
        debug_info: Default::default(),
        by_address: Default::default(),
    })))
}

/// The panel draws the record newest first, and a place visited again moves to the top
/// rather than appearing twice.
#[test]
fn recording_puts_the_newest_place_first() {
    let (a, b) = (place("a"), place("b"));
    let mut visits = Visits::default();
    assert!(visits.entries().is_empty());

    visits.record(a.clone());
    visits.record(b.clone());
    visits.record(a.clone());
    assert!(visits.entries() == [a, b]);
}

#[test]
fn recording_past_the_cap_drops_the_oldest_places() {
    let mut visits = Visits::default();
    let places: Vec<Document> = (0..MAX_VISITS + 3).map(|i| place(&i.to_string())).collect();
    for entry in &places {
        visits.record(entry.clone());
    }
    assert_eq!(visits.entries().len(), MAX_VISITS);
    assert!(visits.entries().first() == places.last());
    assert!(visits.entries().last() == places.get(3));
}

/// A saved list is not trusted: duplicates collapse before the cap is applied, so a file
/// with many revisits of few places keeps all of them.
#[test]
fn restoring_collapses_duplicates_before_capping() {
    let (a, b) = (place("a"), place("b"));
    let mut saved = Vec::new();
    for _ in 0..MAX_VISITS {
        saved.push(a.clone());
        saved.push(b.clone());
    }

    let visits = Visits::restored(saved);
    assert!(visits.entries() == [a, b]);
}

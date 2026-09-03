use std::{collections::HashMap, path::PathBuf, sync::Arc};

use analysis::{Architecture, BinaryFormat, Object, ObjectData};

use super::*;
use crate::project::Selection;

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

fn newest_first(visits: &Visits) -> Vec<Document> {
    visits.recent().cloned().collect()
}

#[test]
fn recording_puts_the_newest_place_first() {
    let (a, b) = (place("a"), place("b"));
    let mut visits = Visits::default();
    assert_eq!(visits.recent().len(), 0);

    visits.record(a.clone());
    visits.record(b.clone());
    assert!(newest_first(&visits) == [b.clone(), a.clone()]);
    assert!(visits.entries() == [a, b]);
}

/// A place visited again moves to the top rather than appearing twice, and the top place
/// visited again changes nothing -- which `would_record` says in advance, so a caller can
/// skip a write that would wake the panel for nothing.
#[test]
fn a_place_visited_again_moves_to_the_top_once() {
    let (a, b, c) = (place("a"), place("b"), place("c"));
    let mut visits = Visits::default();
    for entry in [&a, &b, &c] {
        visits.record(entry.clone());
    }

    assert!(visits.would_record(&a));
    visits.record(a.clone());
    assert!(newest_first(&visits) == [a.clone(), c.clone(), b.clone()]);

    assert!(!visits.would_record(&a));
    visits.record(a.clone());
    assert!(newest_first(&visits) == [a, c, b]);
}

#[test]
fn recording_past_the_cap_drops_the_oldest_places() {
    let mut visits = Visits::default();
    let places: Vec<Document> = (0..MAX_VISITS + 3).map(|i| place(&i.to_string())).collect();
    for entry in &places {
        visits.record(entry.clone());
    }
    assert_eq!(visits.entries().len(), MAX_VISITS);
    assert!(visits.entries().first() == places.get(3));
    assert!(visits.entries().last() == places.last());
}

/// A saved list is not trusted: duplicates collapse onto their newest occurrence before
/// the cap is applied, so a file with many revisits of few places keeps all of them.
#[test]
fn restoring_collapses_duplicates_before_capping() {
    let (a, b) = (place("a"), place("b"));
    let mut saved = Vec::new();
    for _ in 0..MAX_VISITS {
        saved.push(a.clone());
        saved.push(b.clone());
    }
    saved.push(a.clone());

    let visits = Visits::restored(saved);
    assert!(newest_first(&visits) == [a, b]);
}

#[test]
fn retaining_keeps_the_order_of_what_it_keeps() {
    let (a, b, c) = (place("a"), place("b"), place("c"));
    let mut visits = Visits::default();
    for entry in [&a, &b, &c] {
        visits.record(entry.clone());
    }

    let kept = visits.retaining(|entry| *entry != b);
    assert!(newest_first(&kept) == [c, a]);
    assert!(
        visits.entries().len() == 3,
        "retaining changed the original"
    );
}

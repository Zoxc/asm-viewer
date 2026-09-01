use super::*;
use std::sync::Arc;

fn file(name: &str) -> Document {
    Document::Source(Arc::from(name))
}

#[test]
fn an_opened_document_comes_back_by_its_id() {
    let mut docs = Docs::default();
    let id = docs.open(file("/src/main.rs"));
    assert!(docs.get(id) == Some(&file("/src/main.rs")));
    assert_eq!(docs.id_of(&file("/src/main.rs")), Some(id));
    assert_eq!(docs.id_of(&file("/src/other.rs")), None);
}

#[test]
fn a_closed_id_stands_for_nothing() {
    let mut docs = Docs::default();
    let id = docs.open(file("/src/main.rs"));
    docs.close(id);
    assert!(docs.get(id).is_none());
    assert_eq!(docs.id_of(&file("/src/main.rs")), None);
    assert_eq!(docs.len(), 0);
}

/// The rule the whole type exists to keep. A tab header is keyed by its id and a drag
/// carries one, so an id handed out twice would land a closed document's header state
/// — or a drag begun before it was closed — on whichever document took its number.
#[test]
fn an_id_is_never_reused() {
    let mut docs = Docs::default();
    let first = docs.open(file("a.rs"));
    docs.close(first);
    let second = docs.open(file("b.rs"));
    assert_ne!(first, second);

    // And not even for the same document opened again.
    docs.close(second);
    let third = docs.open(file("a.rs"));
    assert_ne!(first, third);
    assert_ne!(second, third);
}

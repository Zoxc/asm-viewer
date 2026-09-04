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
    assert_eq!(docs.showing(&file("/src/main.rs")), Some(id));
    assert_eq!(docs.showing(&file("/src/other.rs")), None);
    assert!(docs.contains(id, &Stop::whole(file("/src/main.rs"))));
}

#[test]
fn a_closed_id_stands_for_nothing() {
    let mut docs = Docs::default();
    let id = docs.open(file("/src/main.rs"));
    docs.close(id);
    assert!(docs.get(id).is_none());
    assert!(docs.trail(id).is_none());
    assert_eq!(docs.showing(&file("/src/main.rs")), None);
    assert!(!docs.contains(id, &Stop::whole(file("/src/main.rs"))));
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

/// A tab shows the entry under its trail's cursor: pushing moves what it shows, going
/// back moves it back, and the places it has been stay on the trail for the positions
/// kept by them.
#[test]
fn a_tab_shows_the_current_entry_of_its_trail() {
    let mut docs = Docs::default();
    let id = docs.open(file("a.rs"));
    docs.trail_mut(id).expect("open").push(file("b.rs"));
    assert!(docs.get(id) == Some(&file("b.rs")));
    assert_eq!(docs.showing(&file("a.rs")), None);
    assert!(docs.contains(id, &Stop::whole(file("a.rs"))));
    assert!(docs.contains(id, &Stop::whole(file("b.rs"))));

    docs.trail_mut(id).expect("open").back();
    assert!(docs.get(id) == Some(&file("a.rs")));
    assert_eq!(docs.showing(&file("a.rs")), Some(id));
    assert!(docs.contains(id, &Stop::whole(file("b.rs"))));
}

/// Two tabs can show one place. Which one answers must not depend on the order a
/// `HashMap` walks, so it is the lowest id.
#[test]
fn showing_answers_the_lowest_of_several_tabs_on_one_place() {
    let mut docs = Docs::default();
    let first = docs.open(file("a.rs"));
    let second = docs.open(file("a.rs"));
    let third = docs.open(file("a.rs"));
    assert_eq!(docs.showing(&file("a.rs")), Some(first));
    docs.close(first);
    assert_eq!(docs.showing(&file("a.rs")), Some(second));
    docs.close(second);
    assert_eq!(docs.showing(&file("a.rs")), Some(third));
}

/// There is at most one temporal tab; marking another takes the mark off the first, and
/// closing it or promoting it leaves none.
#[test]
fn the_temporal_tab_is_one_tab_until_it_is_closed_or_promoted() {
    let mut docs = Docs::default();
    let first = docs.open(file("a.rs"));
    let second = docs.open(file("b.rs"));
    assert_eq!(docs.temporal(), None);

    docs.mark_temporal(first);
    assert_eq!(docs.temporal(), Some(first));
    docs.mark_temporal(second);
    assert_eq!(docs.temporal(), Some(second));

    docs.promote(first);
    assert_eq!(
        docs.temporal(),
        Some(second),
        "promoting another tab took the mark"
    );
    docs.promote(second);
    assert_eq!(docs.temporal(), None);
    assert!(docs.get(second).is_some(), "promoting closed the tab");

    docs.mark_temporal(second);
    docs.close(second);
    assert_eq!(docs.temporal(), None);

    // A closed id cannot be made the temporal tab.
    docs.mark_temporal(second);
    assert_eq!(docs.temporal(), None);
}

#[test]
fn a_restored_trail_opens_whole_and_an_empty_one_opens_nothing() {
    let mut docs = Docs::default();
    let mut trail = History::default();
    trail.push(file("a.rs"));
    trail.push(file("b.rs"));
    trail.back();

    let id = docs.open_trail(trail, true).expect("a trail with entries");
    assert!(docs.get(id) == Some(&file("a.rs")));
    assert!(docs.trail(id).is_some_and(|trail| trail.can_forward()));
    assert_eq!(docs.temporal(), Some(id));

    assert!(docs.open_trail(History::default(), false).is_none());
    assert_eq!(docs.len(), 1);
    // And no id was spent on it.
    let next = docs.open(file("c.rs"));
    assert_eq!(next, DocId(id.0 + 1));
}

/// A closing binary takes its entries off every surviving trail, each cursor carried to
/// the nearest older survivor, and leaves what the tab shows when that is not in the file.
#[test]
fn retaining_entries_thins_every_trail_and_carries_the_cursors() {
    let mut docs = Docs::default();
    let id = docs.open(file("a.rs"));
    let trail = docs.trail_mut(id).expect("open");
    trail.push(file("gone.rs"));
    trail.push(file("b.rs"));
    trail.back();
    assert!(docs.get(id) == Some(&file("gone.rs")));

    // The tab is left on `b.rs` by the caller before the walk; here it is moved by hand.
    docs.trail_mut(id).expect("open").forward();
    docs.retain_entries(|document| *document != file("gone.rs"));

    let trail = docs.trail(id).expect("open");
    assert!(trail.entries() == [Stop::whole(file("a.rs")), Stop::whole(file("b.rs"))]);
    assert!(trail.current().map(|stop| &stop.document) == Some(&file("b.rs")));
    assert!(!docs.contains(id, &Stop::whole(file("gone.rs"))));
}

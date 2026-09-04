use std::{collections::HashMap, path::PathBuf, sync::Arc};

use analysis::{Architecture, BinaryFormat, Object, ObjectData};

use super::*;
use crate::project::Selection;

/// A distinct stop: two calls with the same `name` still produce different `Arc`s, and so
/// entries that do not compare equal.
fn selection(name: &str) -> Stop {
    Stop::whole(document(name))
}

/// The document inside one of those.
fn document(name: &str) -> Document {
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

#[test]
fn an_empty_history_goes_nowhere() {
    let mut history = History::default();
    assert!(history.current().is_none());
    assert!(history.cursor().is_none());
    assert!(!history.can_back());
    assert!(!history.can_forward());
    assert!(history.back().is_none());
    assert!(history.forward().is_none());
}

#[test]
fn pushing_records_and_moves_the_cursor() {
    let (a, b) = (selection("a"), selection("b"));
    let mut history = History::default();

    history.push(a.clone());
    assert!(history.current() == Some(&a));
    assert!(!history.can_back());

    history.push(b.clone());
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(1));
    assert!(history.can_back());
    assert!(!history.can_forward());
}

#[test]
fn back_and_forward_move_the_cursor_without_pushing() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    assert!(history.back() == Some(b.clone()));
    assert!(history.back() == Some(a.clone()));
    assert!(!history.can_back());
    assert!(history.back().is_none());

    assert!(history.forward() == Some(b));
    assert!(history.forward() == Some(c.clone()));
    assert!(!history.can_forward());
    assert!(history.forward().is_none());
    assert!(history.current() == Some(&c));
}

#[test]
fn navigating_back_does_not_re_record_where_it_landed() {
    let (a, b) = (selection("a"), selection("b"));
    let mut history = History::default();
    history.push(a.clone());
    history.push(b);

    let landed = history.back().expect("an older entry");
    assert!(!history.would_push(&landed));
    history.push(landed);

    assert!(history.current() == Some(&a));
    assert!(history.can_forward());
    assert!(!history.can_back());
}

#[test]
fn the_entries_are_kept_oldest_first_whatever_the_cursor_does() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    assert!(newest_first(&history) == vec![c.clone(), b, a]);

    history.back();
    assert!(history.entries().len() == 3);
    assert!(newest_first(&history).first() == Some(&c));
}

/// The entries newest first.
fn newest_first(history: &History) -> Vec<Stop> {
    history.entries().iter().rev().cloned().collect()
}

#[test]
fn revisiting_bumps_an_entry_out_of_the_middle() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    history.push(b.clone());

    // One copy of `b`, now the newest, and the entries behind it closed up.
    assert!(newest_first(&history) == vec![b.clone(), c.clone(), a.clone()]);
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(2));
    assert!(!history.can_forward());
    assert!(history.back() == Some(c));
    assert!(history.back() == Some(a));
    assert!(!history.can_back());
}

#[test]
fn pushing_the_entry_under_the_cursor_is_still_a_no_op() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    // The no-op comes first: bumping what the cursor is already on would drop the
    // forward entry with it.
    history.back();
    history.push(b.clone());

    assert!(newest_first(&history) == vec![c, b.clone(), a]);
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(1));
    assert!(history.can_forward());
}

#[test]
fn a_bump_after_going_back_still_drops_the_forward_entries() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    // Back to `b`, then away to `a`: `c` is abandoned and the `a` behind the cursor is
    // bumped rather than copied.
    history.back();
    history.push(a.clone());

    assert!(newest_first(&history) == vec![a.clone(), b.clone()]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(1));
    assert!(!history.can_forward());
    assert!(history.back() == Some(b));
    assert!(!history.can_back());
}

#[test]
fn restoring_collapses_duplicates_onto_the_newest_occurrence() {
    let (a, b) = (selection("a"), selection("b"));

    let history = History::restored(vec![a.clone(), b.clone(), a.clone()], 2);

    assert!(history.entries() == [b.clone(), a.clone()]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(1));
    assert!(!history.can_forward());
    assert!(!history.would_push(&a));

    // Every occurrence collapses, not just the last pair.
    let history = History::restored(vec![a.clone(), b.clone(), a.clone(), b.clone()], 3);
    assert!(history.entries() == [a, b]);
    assert!(history.cursor() == Some(1));
}

#[test]
fn a_restored_cursor_follows_the_entry_it_was_on() {
    let (a, b) = (selection("a"), selection("b"));

    // The cursor is on the middle entry, which the collapse leaves at index 0.
    let history = History::restored(vec![a.clone(), b.clone(), a.clone()], 1);
    assert!(history.entries() == [b.clone(), a.clone()]);
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(0));
    assert!(history.can_forward());
    assert!(!history.can_back());

    // And on the *first* of two equal entries, where the collapse moves the entry itself
    // to the end: the cursor goes with it.
    let history = History::restored(vec![a.clone(), b.clone(), a.clone()], 0);
    assert!(history.entries() == [b, a.clone()]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(1));
    assert!(!history.can_forward());
}

/// `count` distinct selections, oldest first, pushed onto a fresh history — the caller
/// keeps them to say which ones the cap should have dropped.
fn filled(count: usize) -> (History, Vec<Stop>) {
    let entries: Vec<Stop> = (0..count).map(|i| selection(&format!("e{i}"))).collect();
    let mut history = History::default();
    for entry in &entries {
        history.push(entry.clone());
    }
    (history, entries)
}

#[test]
fn pushing_past_the_cap_drops_the_oldest_entries() {
    let over = MAX_ENTRIES + 50;
    let (history, entries) = filled(over);

    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.entries()[0] == entries[over - MAX_ENTRIES]);
    assert!(history.entries().last() == entries.last());

    // The cursor is still on the entry the last push appended, at its new index.
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.current() == entries.last());
    assert!(!history.can_forward());

    // And walking all the way back reaches the oldest survivor rather than running off an
    // index the drop left naming nothing.
    let mut history = history;
    let mut steps = 0;
    while let Some(entry) = history.back() {
        assert!(entries.contains(&entry));
        steps += 1;
    }
    assert!(steps == MAX_ENTRIES - 1);
    assert!(history.cursor() == Some(0));
    assert!(history.current() == Some(&entries[over - MAX_ENTRIES]));
}

#[test]
fn pushing_past_the_cap_after_going_back_truncates_first() {
    let over = MAX_ENTRIES + 50;
    let (mut history, entries) = filled(over);

    // Back five, so a push has forward entries to truncate as well as a cap to enforce.
    for _ in 0..5 {
        history.back();
    }
    let fresh: Vec<Stop> = (0..10).map(|i| selection(&format!("n{i}"))).collect();
    history.push(fresh[0].clone());
    assert!(history.entries().len() == MAX_ENTRIES - 4);
    assert!(!history.can_forward());

    for entry in &fresh[1..] {
        history.push(entry.clone());
    }
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.current() == fresh.last());
    assert!(!history.can_forward());

    // Ten pushes onto a list the truncation left five short of the cap, so five more
    // entries went off the front on top of the fifty the fill had already dropped.
    assert!(history.entries()[0] == entries[over - MAX_ENTRIES + 5]);
    for abandoned in &entries[over - 5..] {
        assert!(!history.entries().contains(abandoned));
    }
}

#[test]
fn restoring_keeps_the_newest_entries_and_carries_the_cursor() {
    let over = MAX_ENTRIES + 50;
    let entries: Vec<Stop> = (0..over).map(|i| selection(&format!("e{i}"))).collect();

    // A cursor near the newest entry: its entry survives the trim and comes down with it.
    let cursor = over - 10;
    let history = History::restored(entries.clone(), cursor);
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.entries()[0] == entries[over - MAX_ENTRIES]);
    assert!(history.current() == Some(&entries[cursor]));
    assert!(history.cursor() == Some(cursor - (over - MAX_ENTRIES)));
    assert!(history.can_forward());
    assert!(!history.would_push(&entries[cursor]));

    // A cursor so deep in the back stack that the trim drops its entry: it lands on the
    // oldest survivor rather than out of range.
    let history = History::restored(entries.clone(), 10);
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.cursor() == Some(0));
    assert!(history.current() == Some(&entries[over - MAX_ENTRIES]));
    assert!(!history.can_back());
    assert!(history.can_forward());

    // A cursor past the end is clamped before any of that happens.
    let history = History::restored(entries.clone(), over + 100);
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.current() == entries.last());
}

#[test]
fn restoring_collapses_duplicates_before_capping() {
    // Twice the cap saved, ten under the cap distinct: the collapse runs first, so all
    // of them fit. Capping first would have thrown away half of them.
    let distinct = MAX_ENTRIES - 10;
    let unique: Vec<Stop> = (0..distinct).map(|i| selection(&format!("e{i}"))).collect();
    let saved: Vec<Stop> = unique.iter().flat_map(|e| [e.clone(), e.clone()]).collect();

    let history = History::restored(saved, 2 * distinct - 1);
    assert!(history.entries().len() == distinct);
    assert!(history.entries() == unique);
    assert!(history.cursor() == Some(distinct - 1));
}

/// What an entry is called, so the tests below can name the file that is closing without
/// [`Document::in_file`], which is `project.rs`'s to test.
fn named(entry: &Document) -> &str {
    match entry {
        Document::Assembly(Selection::Object(object)) => &object.name,
        _ => unreachable!("the entries here are all objects"),
    }
}

#[test]
fn retaining_drops_what_it_rejects_and_leaves_the_cursor_where_it_was() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    let mut history = history.retaining(|entry| named(entry) != "b");
    assert!(history.entries() == [a.clone(), c.clone()]);
    assert!(history.current() == Some(&c));
    assert!(history.back() == Some(a));
    assert!(!history.can_back());
}

/// Two places in one document are two entries, which is the whole of what a stop is for:
/// following a link inside an object's code is a place to come back to, where opening the
/// same listing twice is not.
#[test]
fn two_places_in_one_document_are_two_entries() {
    let code = document("code");
    let mut history = History::default();
    history.push(Stop::whole(code.clone()));
    history.push(Stop::at(code.clone(), 0x10));
    history.push(Stop::at(code.clone(), 0x40));

    assert_eq!(history.entries().len(), 3);
    assert!(history.current() == Some(&Stop::at(code.clone(), 0x40)));
    assert!(history.back() == Some(Stop::at(code.clone(), 0x10)));
    assert!(history.back() == Some(Stop::whole(code.clone())));
    assert!(!history.can_back());

    // And a place still behind the cursor is bumped to the end rather than doubled, as a
    // revisited document is: going back to 0x10 the long way leaves one entry for it.
    history.forward();
    history.forward();
    history.push(Stop::at(code.clone(), 0x10));
    assert!(
        history.entries()
            == [
                Stop::whole(code.clone()),
                Stop::at(code.clone(), 0x40),
                Stop::at(code, 0x10)
            ],
        "the place was recorded twice"
    );
}

/// The entry the cursor was on is one of the dropped ones, so it falls back to the nearest
/// older survivor.
#[test]
fn retaining_falls_back_to_the_nearest_older_survivor() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }
    history.back();

    let history = history.retaining(|entry| named(entry) != "b");
    assert!(history.current() == Some(&a));
    assert!(!history.can_back());
    assert!(history.can_forward());
}

/// Nothing older than the cursor survived either, so it lands on the oldest entry left
/// rather than out of range.
#[test]
fn retaining_with_no_older_survivor_lands_on_the_oldest_left() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }
    history.back();
    history.back();

    let history = history.retaining(|entry| named(entry) != "a");
    assert!(history.current() == Some(&b));
    assert!(!history.can_back());
    assert!(history.can_forward());
}

#[test]
fn retaining_everything_changes_nothing() {
    let (a, b) = (selection("a"), selection("b"));
    let mut history = History::default();
    for entry in [&a, &b] {
        history.push(entry.clone());
    }
    history.back();

    let history = history.retaining(|_| true);
    assert!(history.entries() == [a.clone(), b]);
    assert!(history.current() == Some(&a));
    assert!(history.can_forward());
}

#[test]
fn retaining_nothing_is_the_empty_history() {
    let mut history = History::default();
    history.push(selection("a"));
    history.push(selection("b"));

    let history = history.retaining(|_| false);
    assert!(history.entries().is_empty());
    assert!(history.current().is_none());
    assert!(!history.can_back());
    assert!(!history.can_forward());
}

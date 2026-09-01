use std::{collections::HashMap, path::PathBuf, sync::Arc};

use analysis::{Architecture, BinaryFormat, Object, ObjectData};

use super::*;
use crate::project::Selection;

/// A distinct document. Two calls with the same `name` still produce different
/// `Arc`s, and so entries that do not compare equal — identity here is the pointer,
/// never the name.
fn selection(name: &str) -> Document {
    Document::Assembly(Selection::Object(Arc::new(Object {
        path: PathBuf::from("/tmp/lib.a"),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted: Vec::new(),
        sections: Vec::new(),
        data: ObjectData::from(&b""[..]),
        dwarf: Default::default(),
    })))
}

#[test]
fn an_empty_history_goes_nowhere() {
    let mut history = History::default();
    assert!(history.current().is_none());
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
    assert!(history.can_back());
    assert!(!history.can_forward());
}

#[test]
fn consecutive_duplicates_are_ignored() {
    let a = selection("a");
    let mut history = History::default();

    history.push(a.clone());
    history.push(a.clone());
    history.push(a);
    assert!(!history.can_back());
    assert!(!history.can_forward());
}

#[test]
fn the_same_name_is_still_a_different_entry() {
    let mut history = History::default();
    history.push(selection("a"));
    history.push(selection("a"));
    assert!(history.can_back());
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

    // What the choke point does once back() has set the selection: it sees the
    // change like any other and offers it to the history, which already has the
    // cursor on it.
    let landed = history.back().expect("an older entry");
    assert!(!history.would_push(&landed));
    history.push(landed);

    // Still one step back from where it was, with the forward entry intact.
    assert!(history.current() == Some(&a));
    assert!(history.can_forward());
    assert!(!history.can_back());
}

#[test]
fn the_cursor_is_the_index_of_the_current_entry() {
    let mut history = History::default();
    assert!(history.cursor().is_none());

    history.push(selection("a"));
    history.push(selection("b"));
    assert!(history.cursor() == Some(1));

    history.back();
    assert!(history.cursor() == Some(0));
}

#[test]
fn recent_lists_every_entry_newest_first() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    let recent: Vec<_> = history
        .recent()
        .map(|(i, entry)| (i, entry.clone()))
        .collect();
    assert!(recent == vec![(2, c), (1, b), (0, a)]);

    // Going back changes which entry is current, not what the list holds.
    history.back();
    assert!(history.recent().len() == 3);
    assert!(history.recent().next().map(|(i, _)| i) == Some(2));
}

#[test]
fn jumping_moves_the_cursor_without_pushing() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    // The entry under the cursor and one past the end are both no-ops, and must
    // stay so: the panel's rows call this for every click, including the current
    // row's, and a no-op has to leave the cursor where it is.
    assert!(!history.can_jump(2));
    assert!(history.jump(2).is_none());
    assert!(!history.can_jump(3));
    assert!(history.jump(3).is_none());
    assert!(history.cursor() == Some(2));

    assert!(history.can_jump(0));
    let landed = history.jump(0).expect("the oldest entry");
    assert!(landed == a);
    assert!(history.current() == Some(&a));

    // Nothing in front was dropped, so the newer entries are still reachable.
    assert!(history.can_forward());
    assert!(history.recent().len() == 3);

    // And, as for back/forward, the choke point that observes the resulting
    // selection change finds nothing left to record.
    assert!(!history.would_push(&landed));
}

#[test]
fn an_empty_history_jumps_nowhere() {
    let mut history = History::default();
    assert!(!history.can_jump(0));
    assert!(history.jump(0).is_none());
    assert!(history.recent().len() == 0);
}

/// The entries newest first, which is the order the history panel shows and the one
/// bumping is about.
fn newest_first(history: &History) -> Vec<Document> {
    history.recent().map(|(_, entry)| entry.clone()).collect()
}

#[test]
fn revisiting_bumps_an_entry_out_of_the_middle() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    history.push(b.clone());

    // One copy of `b`, now the newest, and everything else in the order it was.
    assert!(newest_first(&history) == vec![b.clone(), c.clone(), a.clone()]);
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(2));
    assert!(!history.can_forward());

    // And the entries behind it closed up rather than leaving a hole.
    assert!(history.back() == Some(c));
    assert!(history.back() == Some(a));
    assert!(!history.can_back());
}

#[test]
fn revisiting_bumps_the_oldest_entry() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    history.push(a.clone());

    assert!(newest_first(&history) == vec![a.clone(), c, b]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(2));
}

#[test]
fn a_bump_leaves_the_cursor_on_the_last_entry() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c, &a, &b] {
        history.push(entry.clone());
    }

    // Every push either appended or bumped, so the cursor is the last index and
    // there is never anything to go forward to.
    assert!(history.cursor() == Some(history.recent().len() - 1));
    assert!(!history.can_forward());
    assert!(newest_first(&history) == vec![b, a, c]);
}

#[test]
fn pushing_the_entry_under_the_cursor_is_still_a_no_op() {
    let (a, b, c) = (selection("a"), selection("b"), selection("c"));
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    // The rule that stops back/forward from re-recording comes first: pushing what
    // the cursor is already on must not bump it to the newest position, which would
    // drop the forward entry with it.
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

    // Back to `b`, then away to `a`: `c` is abandoned as it always was, and the
    // `a` behind the cursor is bumped rather than copied.
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

    // What a `session.toml` written before entries were bumped can hold. The cursor
    // is on the newest `a`, the usual case.
    let history = History::restored(vec![a.clone(), b.clone(), a.clone()], 2);

    assert!(history.entries() == [b.clone(), a.clone()]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(1));
    assert!(!history.can_forward());
    // And the effect that observes the restored selection finds nothing to record.
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

    // And on the *first* of two equal entries, where the collapse moves the entry
    // itself to the end: the cursor goes with it rather than staying on an index
    // that now names something else.
    let history = History::restored(vec![a.clone(), b.clone(), a.clone()], 0);
    assert!(history.entries() == [b, a.clone()]);
    assert!(history.current() == Some(&a));
    assert!(history.cursor() == Some(1));
    assert!(!history.can_forward());
}

#[test]
fn a_restored_history_holds_the_no_duplicates_invariant() {
    let (a, b) = (selection("a"), selection("b"));
    let mut history = History::restored(vec![a.clone(), b.clone(), a.clone()], 2);

    // Pushing on top of a restored history behaves as on a built one: `b` is bumped
    // out of the list rather than copied.
    history.push(b.clone());
    assert!(history.entries() == [a, b.clone()]);
    assert!(history.current() == Some(&b));
    assert!(history.cursor() == Some(1));
}

/// `count` distinct selections, oldest first, pushed onto a fresh history — the
/// caller keeps them to say which ones the cap should have dropped.
fn filled(count: usize) -> (History, Vec<Document>) {
    let entries: Vec<Document> = (0..count).map(|i| selection(&format!("e{i}"))).collect();
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

    // The length holds at the cap, and it is the front that went.
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.entries()[0] == entries[over - MAX_ENTRIES]);
    assert!(history.entries().last() == entries.last());

    // The cursor is still on the entry the last push appended, at its new index.
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.current() == entries.last());
    assert!(!history.can_forward());

    // And walking all the way back reaches the oldest survivor rather than running
    // off an index the drop left naming nothing.
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
fn the_cursor_follows_its_entry_across_a_drop() {
    let over = MAX_ENTRIES + 50;
    let entries: Vec<Document> = (0..over).map(|i| selection(&format!("e{i}"))).collect();

    // A cursor part-way back through an over-long history: what the cap moves it to
    // is wherever its *entry* ended up, not the index that entry used to have.
    let cursor = over - 25;
    let history = History::restored(entries.clone(), cursor);
    let landed = history.cursor().expect("an entry");
    assert!(landed != cursor);
    assert!(history.entries()[landed] == entries[cursor]);
    // Which is to say it kept its distance from the newest end.
    assert!(history.entries().len() - landed == over - cursor);
    assert!(history.can_back());
    assert!(history.can_forward());

    // A push that trips the cap likewise leaves the cursor on the entry it appended,
    // at the index the drop moved that entry to rather than the count of pushes.
    let (history, entries) = filled(over);
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.entries()[MAX_ENTRIES - 1] == entries[over - 1]);
}

#[test]
fn pushing_past_the_cap_after_going_back_truncates_first() {
    let over = MAX_ENTRIES + 50;
    let (mut history, entries) = filled(over);

    // Back five, so a push has forward entries to truncate as well as a cap to
    // enforce. The truncation takes the list under the cap again...
    for _ in 0..5 {
        history.back();
    }
    let fresh: Vec<Document> = (0..10).map(|i| selection(&format!("n{i}"))).collect();
    history.push(fresh[0].clone());
    assert!(history.entries().len() == MAX_ENTRIES - 4);
    assert!(!history.can_forward());

    // ...and the pushes after it fill it back up and start dropping the front again.
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
    // The abandoned branch is gone rather than merely capped away.
    for abandoned in &entries[over - 5..] {
        assert!(!history.entries().contains(abandoned));
    }
}

#[test]
fn restoring_keeps_the_newest_entries_and_carries_the_cursor() {
    let over = MAX_ENTRIES + 50;
    let entries: Vec<Document> = (0..over).map(|i| selection(&format!("e{i}"))).collect();

    // A cursor near the newest entry, the ordinary case: its entry survives the trim
    // and the cursor comes down with it by however many entries were dropped.
    let cursor = over - 10;
    let history = History::restored(entries.clone(), cursor);
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.entries()[0] == entries[over - MAX_ENTRIES]);
    assert!(history.current() == Some(&entries[cursor]));
    assert!(history.cursor() == Some(cursor - (over - MAX_ENTRIES)));
    assert!(history.can_forward());
    assert!(!history.would_push(&entries[cursor]));

    // A cursor so deep in the back stack that the trim drops its entry: it lands on
    // the oldest survivor rather than out of range.
    let history = History::restored(entries.clone(), 10);
    assert!(history.entries().len() == MAX_ENTRIES);
    assert!(history.cursor() == Some(0));
    assert!(history.current() == Some(&entries[over - MAX_ENTRIES]));
    assert!(!history.can_back());
    assert!(history.can_forward());

    // A cursor past the end is still clamped before any of that happens.
    let history = History::restored(entries.clone(), over + 100);
    assert!(history.cursor() == Some(MAX_ENTRIES - 1));
    assert!(history.current() == entries.last());
}

#[test]
fn restoring_collapses_duplicates_before_capping() {
    // Every entry saved twice: 380 saved entries, 190 destinations. The collapse runs
    // first, so all 190 fit and the cap drops nothing — capping first would have
    // thrown away half of them to keep 200 saved *entries*.
    let unique: Vec<Document> = (0..190).map(|i| selection(&format!("e{i}"))).collect();
    let saved: Vec<Document> = unique.iter().flat_map(|e| [e.clone(), e.clone()]).collect();

    let history = History::restored(saved, 379);
    assert!(history.entries().len() == 190);
    assert!(history.entries() == unique);
    assert!(history.cursor() == Some(189));
}

/// What an entry is called, so that the retaining tests below can name the file
/// that is closing without [`Document::in_file`], which is `project.rs`'s to test.
fn named(entry: &Document) -> &str {
    match entry {
        Document::Assembly(Selection::Object(object)) => &object.name,
        _ => unreachable!("the entries here are all objects"),
    }
}

/// Closing a file drops the entries that pointed into it, and the cursor stays on
/// the entry it was on when that entry was not one of them.
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

/// The entry the cursor was on is one of the dropped ones, so the reader is left on
/// the nearest older place they can still reach — the same answer a restore gives.
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

/// Nothing older than the cursor survived either, so it lands on the oldest entry
/// left rather than out of range.
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

/// Closing a file nothing in the history points at leaves it exactly as it was,
/// cursor included.
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

/// Closing the only file open: the history goes with it rather than keeping a list
/// of places nothing can reach.
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

#[test]
fn pushing_after_going_back_drops_the_forward_entries() {
    let (a, b, c, d) = (
        selection("a"),
        selection("b"),
        selection("c"),
        selection("d"),
    );
    let mut history = History::default();
    for entry in [&a, &b, &c] {
        history.push(entry.clone());
    }

    history.back();
    history.back();
    history.push(d.clone());

    assert!(history.current() == Some(&d));
    assert!(!history.can_forward());
    assert!(history.back() == Some(a));
    assert!(!history.can_back());
}

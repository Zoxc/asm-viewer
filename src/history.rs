//! Where the selection has been: a browser-style back/forward history.
//!
//! Framework-free, like [`crate::project`] — this is plain data over [`Selection`], so
//! the whole navigation model is unit-testable without a UI. Only the `State<History>`
//! that shares it through context lives in `ui.rs`.
//!
//! An entry is a [`Selection`], i.e. `Arc`s compared by pointer, so recording one costs
//! a refcount bump and comparing two is a pointer test. The flip side of pointer
//! identity is that parsing the same file twice yields different pointers: entries made
//! before a re-parse never compare equal to the ones made after it, and they keep the
//! superseded `Object` alive for as long as the history holds them. Within a session
//! objects are only ever added, so that only comes up if the user reopens a file that
//! is already open.
//!
//! The history *is* persisted, as [`crate::project::SavedHistory`] inside `project.json`:
//! [`History::entries`] and [`History::cursor`] are what goes out and
//! [`History::restored`] is what comes back. Entries whose object or symbol no longer
//! resolves are dropped on the way in, which is why `restored` takes an already-built
//! list rather than replaying pushes.

use crate::project::Selection;

/// A list of visited selections plus a cursor into it.
///
/// The cursor is the entry currently on screen. [`History::push`] records a new entry
/// after it and drops whatever was in front; [`History::back`] and
/// [`History::forward`] only move the cursor.
///
/// No two entries are ever equal: pushing somewhere that is already in the list bumps
/// that entry to the newest position instead of appending a copy. [`History::push`] and
/// [`History::restored`] are the only two ways entries get in, and both enforce it.
#[derive(Clone, Default)]
pub struct History {
    entries: Vec<Selection>,
    /// The index of the current entry. In range whenever `entries` is non-empty, and
    /// `0` — meaning nothing — while it is empty.
    cursor: usize,
}

impl History {
    /// A history rebuilt from a saved session: `entries` oldest first, with the cursor on
    /// `entries[cursor]`.
    ///
    /// The cursor is clamped into range, so neither a hand-edited `project.json` nor a
    /// restore that dropped entries can leave it past the end. An empty `entries` gives
    /// back the empty history, cursor and all.
    ///
    /// The no-duplicates invariant [`History::push`] keeps is *enforced* here rather than
    /// assumed, because the entries come from outside: a `project.json` written before
    /// entries were bumped rather than appended can name the same destination twice, two
    /// saved entries can resolve to the same `Arc`, and the file can be hand-edited.
    /// Duplicates are collapsed the way `push` would have left them — the newest
    /// occurrence is the one that survives — and the cursor follows the *entry* it was
    /// on to wherever the collapse put it, rather than staying on an index that now names
    /// something else. That is what keeps the restore's property that the cursor entry is
    /// the restored selection true even when collapsing moved it.
    pub fn restored(entries: Vec<Selection>, cursor: usize) -> History {
        let current = entries
            .get(cursor.min(entries.len().saturating_sub(1)))
            .cloned();

        let mut deduplicated: Vec<Selection> = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(position) = deduplicated.iter().position(|existing| *existing == entry) {
                deduplicated.remove(position);
            }
            deduplicated.push(entry);
        }

        // `unwrap_or(0)` is only ever the empty history: the entry the cursor was on is
        // still in the list, since collapsing moves an occurrence rather than dropping
        // the destination.
        let cursor = current
            .and_then(|current| deduplicated.iter().position(|entry| *entry == current))
            .unwrap_or(0);

        History {
            entries: deduplicated,
            cursor,
        }
    }

    /// Every entry, oldest first — what persistence saves. The history panel wants
    /// [`History::recent`] instead, which numbers them and hands them back newest first.
    pub fn entries(&self) -> &[Selection] {
        &self.entries
    }

    /// The entry the cursor is on, or `None` before anything has been recorded.
    pub fn current(&self) -> Option<&Selection> {
        self.entries.get(self.cursor)
    }

    /// Whether [`History::push`] would record `selection` as a new entry.
    ///
    /// It would not for [`Selection::None`], which is the state the app boots into
    /// rather than a place to come back to, nor for a selection that is already the
    /// entry at the cursor. That second rule is the one that matters: it is what stops
    /// back/forward navigation — which sets the selection, and so is observed like any
    /// other selection change — from re-recording where it has just moved the cursor.
    pub fn would_push(&self, selection: &Selection) -> bool {
        !matches!(selection, Selection::None) && self.current() != Some(selection)
    }

    /// Record `selection` as the newest entry and put the cursor on it. A no-op when
    /// [`History::would_push`] is false.
    ///
    /// Anything in front of the cursor is discarded first, so going back and then
    /// somewhere new forgets the abandoned branch, exactly as a browser does. An entry
    /// equal to `selection` that is still there afterwards is then *bumped* rather than
    /// duplicated: it is removed from where it was and appended, so revisiting somewhere
    /// moves it to the newest position instead of adding a second copy. No two entries
    /// are ever equal, and the list reads as each destination once, in the order it was
    /// last visited.
    ///
    /// Only entries *behind* the cursor can match — the truncation dropped everything in
    /// front of it and `would_push` has already ruled out the one under it — and removing
    /// one shifts the rest down, which is why the cursor is taken from the final length
    /// rather than stepped.
    pub fn push(&mut self, selection: Selection) {
        if !self.would_push(&selection) {
            return;
        }

        self.entries.truncate(self.cursor + 1);
        self.entries.retain(|entry| *entry != selection);
        self.entries.push(selection);
        self.cursor = self.entries.len() - 1;
    }

    /// The index of the entry the cursor is on, or `None` before anything has been
    /// recorded. Paired with [`History::recent`] this is all a list of the entries needs
    /// to mark the one that is current, without the entries themselves being reachable.
    pub fn cursor(&self) -> Option<usize> {
        (self.cursor < self.entries.len()).then_some(self.cursor)
    }

    /// Every entry with its index, newest first — the order a history panel shows them
    /// in. The index is what [`History::jump`] takes, so a row can carry the one it was
    /// built from and hand it straight back.
    pub fn recent(&self) -> impl ExactSizeIterator<Item = (usize, &Selection)> + '_ {
        self.entries.iter().enumerate().rev()
    }

    /// Whether there is an older entry to go back to.
    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    /// Whether there is a newer entry to go forward to.
    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Step the cursor back one entry and hand back what is now current, or `None` at
    /// the oldest entry. Nothing is recorded: the caller sets the selection to what
    /// comes back, and the push that observes that change dedups against the entry the
    /// cursor has just landed on.
    pub fn back(&mut self) -> Option<Selection> {
        self.can_back().then(|| {
            self.cursor -= 1;
            self.entries[self.cursor].clone()
        })
    }

    /// Step the cursor forward one entry, or `None` at the newest one. The mirror of
    /// [`History::back`], and equally not a push.
    pub fn forward(&mut self) -> Option<Selection> {
        self.can_forward().then(|| {
            self.cursor += 1;
            self.entries[self.cursor].clone()
        })
    }

    /// Whether [`History::jump`] would move the cursor: `index` has to name an entry,
    /// and not the one already under the cursor.
    pub fn can_jump(&self, index: usize) -> bool {
        index < self.entries.len() && index != self.cursor
    }

    /// Put the cursor straight on the entry at `index` and hand it back, or `None` when
    /// [`History::can_jump`] is false. Like [`History::back`] and [`History::forward`]
    /// this only moves the cursor: nothing is recorded and nothing in front of it is
    /// dropped, so jumping to an older entry leaves the newer ones to come back to.
    pub fn jump(&mut self, index: usize) -> Option<Selection> {
        self.can_jump(index).then(|| {
            self.cursor = index;
            self.entries[index].clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf, sync::Arc};

    use analysis::{BinaryFormat, Object};

    use super::*;

    /// A distinct selection. Two calls with the same `name` still produce different
    /// `Arc`s, and so entries that do not compare equal — identity here is the pointer,
    /// never the name.
    fn selection(name: &str) -> Selection {
        Selection::Object(Arc::new(Object {
            path: PathBuf::from("/tmp/lib.a"),
            name: name.to_owned(),
            format: BinaryFormat::Elf,
            symbols: HashMap::new(),
            symbols_sorted: Vec::new(),
            sections: Vec::new(),
        }))
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
    fn nothing_selected_is_not_recorded() {
        let mut history = History::default();
        history.push(Selection::None);
        assert!(history.current().is_none());
        assert!(!history.can_back());
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
    fn newest_first(history: &History) -> Vec<Selection> {
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

        // What a `project.json` written before entries were bumped can hold. The cursor
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
}

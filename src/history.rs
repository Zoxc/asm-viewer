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
//! Nothing here is persisted: the history is per-session state, and `project.json`
//! keeps only the binaries and the selection.

use crate::project::Selection;

/// A list of visited selections plus a cursor into it.
///
/// The cursor is the entry currently on screen. [`History::push`] records a new entry
/// after it and drops whatever was in front; [`History::back`] and
/// [`History::forward`] only move the cursor.
#[derive(Clone, Default)]
pub struct History {
    entries: Vec<Selection>,
    /// The index of the current entry. In range whenever `entries` is non-empty, and
    /// `0` — meaning nothing — while it is empty.
    cursor: usize,
}

impl History {
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
    /// somewhere new forgets the abandoned branch, exactly as a browser does.
    pub fn push(&mut self, selection: Selection) {
        if !self.would_push(&selection) {
            return;
        }

        self.entries.truncate(self.cursor + 1);
        self.entries.push(selection);
        self.cursor = self.entries.len() - 1;
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

//! Where the reader has been: a browser-style back/forward history.
//!
//! Framework-free, like [`crate::project`] — this is plain data over [`Document`], so
//! the whole navigation model is unit-testable without a UI. Only the `State<History>`
//! that shares it through context lives in `ui.rs`.
//!
//! An entry is a [`Document`], i.e. `Arc`s compared by pointer, so recording one costs
//! a refcount bump and comparing two is a pointer test. The flip side of pointer
//! identity is that parsing the same file twice yields different pointers: entries made
//! before a re-parse never compare equal to the ones made after it, and they keep the
//! superseded `Object` alive for as long as the history holds them. Within a session
//! objects are only ever added, so that only comes up if the user reopens a file that
//! is already open. That, together with the whole list going out to disk on every
//! flush, is why the history is capped at [`MAX_ENTRIES`] rather than growing for the
//! life of the session.
//!
//! The history *is* persisted, as [`crate::project::SavedHistory`] inside `session.toml`:
//! [`History::entries`] and [`History::cursor`] are what goes out and
//! [`History::restored`] is what comes back. Entries whose object or symbol no longer
//! resolves are dropped on the way in, which is why `restored` takes an already-built
//! list rather than replaying pushes.

use crate::project::Document;

/// The most entries a history ever holds. A push that would take it past this drops the
/// oldest ones off the front, and [`History::restored`] keeps the newest this many.
///
/// The cap exists because an entry is not free. Each one holds an `Arc<Object>` or an
/// `Arc<SymbolData>`, so an uncapped history pins every object the user has visited —
/// including the superseded parse of a file that was reopened, which nothing else keeps
/// alive — for the rest of the session, and the entries are all serialized out again on
/// every flush. 200 is far more back/forward reach than a session ever uses while
/// bounding both.
const MAX_ENTRIES: usize = 200;

/// A list of visited documents plus a cursor into it.
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
    entries: Vec<Document>,
    /// The index of the current entry. In range whenever `entries` is non-empty, and
    /// `0` — meaning nothing — while it is empty.
    cursor: usize,
}

impl History {
    /// A history rebuilt from a saved session: `entries` oldest first, with the cursor on
    /// `entries[cursor]`.
    ///
    /// The cursor is clamped into range, so neither a hand-edited `session.toml` nor a
    /// restore that dropped entries can leave it past the end. An empty `entries` gives
    /// back the empty history, cursor and all.
    ///
    /// The no-duplicates invariant [`History::push`] keeps is *enforced* here rather than
    /// assumed, because the entries come from outside: a `session.toml` written before
    /// entries were bumped rather than appended can name the same destination twice, two
    /// saved entries can resolve to the same `Arc`, and the file can be hand-edited.
    /// Duplicates are collapsed the way `push` would have left them — the newest
    /// occurrence is the one that survives — and the cursor follows the *entry* it was
    /// on to wherever the collapse put it, rather than staying on an index that now names
    /// something else. That is what keeps the restore's property that the cursor entry is
    /// the restored selection true even when collapsing moved it.
    ///
    /// [`MAX_ENTRIES`] is enforced here too, after the collapse, so an older or
    /// hand-edited file cannot load a history longer than one the app would build. The
    /// window kept is the *newest* `MAX_ENTRIES` entries rather than the ones around the
    /// cursor: that is exactly what a session of pushes would have left, so a restore can
    /// only produce a history `push` could have, and it keeps the destinations the user
    /// visited most recently instead of trimming them to preserve a deep back stack. The
    /// cursor is almost always at or near the newest entry — it is where the user was, and
    /// every push puts it last — so its entry survives the trim in every ordinary case and
    /// the cursor follows it down; in the pathological one, a saved cursor with more than
    /// `MAX_ENTRIES` entries in front of it, its entry goes and the cursor lands on the
    /// oldest survivor.
    pub fn restored(entries: Vec<Document>, cursor: usize) -> History {
        let current = entries
            .get(cursor.min(entries.len().saturating_sub(1)))
            .cloned();

        let mut deduplicated: Vec<Document> = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(position) = deduplicated.iter().position(|existing| *existing == entry) {
                deduplicated.remove(position);
            }
            deduplicated.push(entry);
        }

        // `unwrap_or(0)` is only ever the empty history: the entry the cursor was on is
        // still in the list, since collapsing moves an occurrence rather than dropping
        // the destination. Trimming to `MAX_ENTRIES` can drop it, but that happens after
        // this, in `cap`, which carries the cursor with it.
        let cursor = current
            .and_then(|current| deduplicated.iter().position(|entry| *entry == current))
            .unwrap_or(0);

        let mut history = History {
            entries: deduplicated,
            cursor,
        };
        history.cap();
        history
    }

    /// A history rebuilt from entries that may no longer point anywhere: one [`Option`]
    /// per entry of the list it came from, oldest first and `None` where the entry is
    /// gone, with `cursor` an index into *that* list rather than into what survives.
    ///
    /// The one walk both ways of losing entries go through — a restore whose binaries
    /// have changed ([`crate::project::Session::resolve_history`]) and a file the reader
    /// closed ([`History::retaining`]) — because the rule is the same in both, and it is
    /// the *cursor* that makes it worth having in one place. Walking the entries up to
    /// and including `cursor`, it is left on the last one that survived: so it stays on
    /// the entry it was on when that entry survived, falls back to the nearest older
    /// survivor when it did not, and to the oldest surviving entry when nothing older
    /// survived either.
    ///
    /// Dropping rather than degrading is the decision this encodes, and it is the same
    /// one in both callers: a history entry is one of many, and a list of places the
    /// reader cannot get back to is worse than a short list. The *selection* is the one
    /// thing that degrades instead, having to be somewhere.
    ///
    /// Duplicates are [`History::restored`]'s business rather than this loop's, which is
    /// why the survivors go out through it.
    pub fn rebuilt(entries: impl IntoIterator<Item = Option<Document>>, cursor: usize) -> History {
        let mut kept = Vec::new();
        let mut moved = 0;

        for (index, entry) in entries.into_iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            if index <= cursor {
                moved = kept.len();
            }
            kept.push(entry);
        }

        History::restored(kept, moved)
    }

    /// The same history with only the entries `keep` accepts, the cursor carried the way
    /// [`History::rebuilt`] carries it.
    ///
    /// This is what closing a file does to the history: every entry pointing into an
    /// object that is going away is dropped, and the reader is left on the nearest place
    /// they can still reach. The predicate is over the whole [`Document`] rather than
    /// over an object, because an entry names a symbol as often as an object and both
    /// answer for the file they came out of.
    pub fn retaining(&self, keep: impl Fn(&Document) -> bool) -> History {
        History::rebuilt(
            self.entries
                .iter()
                .map(|entry| keep(entry).then(|| entry.clone())),
            self.cursor,
        )
    }

    /// Every entry, oldest first — what persistence saves. The history panel wants
    /// [`History::recent`] instead, which numbers them and hands them back newest first.
    pub fn entries(&self) -> &[Document] {
        &self.entries
    }

    /// The entry the cursor is on, or `None` before anything has been recorded.
    pub fn current(&self) -> Option<&Document> {
        self.entries.get(self.cursor)
    }

    /// Whether [`History::push`] would record `document` as a new entry.
    ///
    /// It would not for a document that is already the entry at the cursor, which is
    /// the whole of the rule now that "nothing selected" is an absent selection rather
    /// than a value this could be handed: it is what stops back/forward navigation —
    /// which sets the selection, and so is observed like any other selection change —
    /// from re-recording where it has just moved the cursor.
    pub fn would_push(&self, document: &Document) -> bool {
        self.current() != Some(document)
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
    ///
    /// A push that takes the list past [`MAX_ENTRIES`] then drops the oldest entries, so
    /// the newest destination is always recorded and it is the far end of the back stack
    /// that is forgotten.
    pub fn push(&mut self, document: Document) {
        if !self.would_push(&document) {
            return;
        }

        self.entries.truncate(self.cursor + 1);
        self.entries.retain(|entry| *entry != document);
        self.entries.push(document);
        self.cursor = self.entries.len() - 1;
        self.cap();
    }

    /// Drop entries off the front until at most [`MAX_ENTRIES`] are left, keeping the
    /// cursor on the entry it was on.
    ///
    /// Dropping `excess` entries shifts every index that survives down by that much, so
    /// the cursor moves with them. `saturating_sub` is what keeps it in range when the
    /// entry it was on is itself one of the dropped ones: it lands on the oldest
    /// survivor, which is the same place [`crate::project::Session::resolve_history`]
    /// puts it when nothing older than the saved cursor survived. That case cannot arise
    /// from [`History::push`], which caps with the cursor on the entry it has just
    /// appended, only from [`History::restored`].
    fn cap(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        if excess == 0 {
            return;
        }

        self.entries.drain(..excess);
        self.cursor = self.cursor.saturating_sub(excess);
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
    pub fn recent(&self) -> impl ExactSizeIterator<Item = (usize, &Document)> + '_ {
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
    pub fn back(&mut self) -> Option<Document> {
        self.can_back().then(|| {
            self.cursor -= 1;
            self.entries[self.cursor].clone()
        })
    }

    /// Step the cursor forward one entry, or `None` at the newest one. The mirror of
    /// [`History::back`], and equally not a push.
    pub fn forward(&mut self) -> Option<Document> {
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
    pub fn jump(&mut self, index: usize) -> Option<Document> {
        self.can_jump(index).then(|| {
            self.cursor = index;
            self.entries[index].clone()
        })
    }
}

#[cfg(test)]
mod tests;

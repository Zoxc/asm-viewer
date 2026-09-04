//! Where one tab has been: a browser-style back/forward trail over [`Stop`], one per tab.
//! Everywhere the reader has been across every tab is [`crate::visits::Visits`].
//!
//! Entries are compared by `Arc` pointer, so entries made before a re-parse never compare
//! equal to ones made after it. Persisted as [`crate::project::SavedTab`].

use crate::project::Document;

/// The most entries a trail ever holds; the oldest are dropped past it. Per tab, so it is
/// modest: every entry is saved with the session, rows and all.
const MAX_ENTRIES: usize = 50;

/// One place on a tab's trail: a document, and where in it the tab was.
///
/// The address is `Some` only for an object's code, which is the one document a tab
/// visits at more than one place -- following a link there moves the listing rather than
/// opening anything, so the trail is the only record that the reader was somewhere else
/// in it a moment ago. Every other document *is* the place, and carries none.
#[derive(Clone, PartialEq)]
pub struct Stop {
    pub document: Document,
    /// The placed address the tab was at, for a stop in an object's code.
    pub address: Option<u64>,
}

impl From<Document> for Stop {
    fn from(document: Document) -> Stop {
        Stop::whole(document)
    }
}

impl Stop {
    /// A whole document, at no place in particular: what everything but a jump inside an
    /// object's code makes.
    pub fn whole(document: Document) -> Stop {
        Stop {
            document,
            address: None,
        }
    }

    /// A place in an object's code.
    pub fn at(document: Document, address: u64) -> Stop {
        Stop {
            document,
            address: Some(address),
        }
    }
}

/// A list of visited places plus a cursor into it.
///
/// No two entries are ever equal: [`History::push`] and [`History::restored`] are the only
/// ways in, and both bump a revisited stop rather than appending a copy. A document can
/// therefore be on one trail more than once, at one address each, which is what lets Back
/// walk the places a reader followed inside an object's code; two stops at one place are
/// still one entry, which is what lets a tab and a document name the position its panes
/// are kept by.
#[derive(Clone, Default, PartialEq)]
pub struct History {
    entries: Vec<Stop>,
    /// In range whenever `entries` is non-empty, and `0` — meaning nothing — while empty.
    cursor: usize,
}

impl History {
    /// A history rebuilt from a saved session: `entries` oldest first, cursor on
    /// `entries[cursor]`.
    ///
    /// The entries come from outside, so the invariants are enforced rather than assumed:
    /// the cursor is clamped into range, duplicates are collapsed onto their newest
    /// occurrence with the cursor following the *entry* it was on, and the list is then
    /// trimmed to the newest [`MAX_ENTRIES`].
    pub fn restored(entries: Vec<Stop>, cursor: usize) -> History {
        let current = entries
            .get(cursor.min(entries.len().saturating_sub(1)))
            .cloned();

        let mut deduplicated: Vec<Stop> = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(position) = deduplicated.iter().position(|existing| *existing == entry) {
                deduplicated.remove(position);
            }
            deduplicated.push(entry);
        }

        // `unwrap_or(0)` is only ever the empty history: collapsing moves an occurrence
        // rather than dropping the destination. The trim can drop it, and that happens
        // afterwards in `cap`, which carries the cursor with it.
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
    /// per entry, oldest first and `None` where the entry is gone, with `cursor` an index
    /// into *that* list rather than into what survives.
    ///
    /// The cursor is left on the last survivor at or before it, falling back to the oldest
    /// survivor when nothing older survived. The one walk both a restore and a file close
    /// go through.
    pub fn rebuilt(entries: impl IntoIterator<Item = Option<Stop>>, cursor: usize) -> History {
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
    pub fn retaining(&self, keep: impl Fn(&Document) -> bool) -> History {
        History::rebuilt(
            self.entries
                .iter()
                .map(|entry| keep(&entry.document).then(|| entry.clone())),
            self.cursor,
        )
    }

    /// Every entry, oldest first — what persistence saves.
    pub fn entries(&self) -> &[Stop] {
        &self.entries
    }

    /// The entry the cursor is on, or `None` before anything has been recorded.
    pub fn current(&self) -> Option<&Stop> {
        self.entries.get(self.cursor)
    }

    /// Whether [`History::push`] would record `stop`. False for the entry already under
    /// the cursor, which is what stops back/forward from re-recording where they have
    /// just moved it.
    pub fn would_push(&self, stop: &Stop) -> bool {
        self.current() != Some(stop)
    }

    /// Record `stop` as the newest entry and put the cursor on it. A no-op when
    /// [`History::would_push`] is false.
    ///
    /// Anything in front of the cursor is discarded first, then an equal entry still
    /// behind it is bumped to the end rather than duplicated, then the cap is enforced.
    pub fn push(&mut self, stop: impl Into<Stop>) {
        let stop = stop.into();
        if !self.would_push(&stop) {
            return;
        }

        self.entries.truncate(self.cursor + 1);
        self.entries.retain(|entry| *entry != stop);
        self.entries.push(stop);
        self.cursor = self.entries.len() - 1;
        self.cap();
    }

    /// Drop entries off the front until at most [`MAX_ENTRIES`] are left, moving the
    /// cursor down with them. `saturating_sub` lands it on the oldest survivor when the
    /// entry it was on was itself dropped, which only [`History::restored`] can cause.
    fn cap(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        if excess == 0 {
            return;
        }

        self.entries.drain(..excess);
        self.cursor = self.cursor.saturating_sub(excess);
    }

    /// The index of the entry the cursor is on, or `None` before anything has been
    /// recorded.
    pub fn cursor(&self) -> Option<usize> {
        (self.cursor < self.entries.len()).then_some(self.cursor)
    }

    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Step the cursor back one entry and hand back what is now current, or `None` at the
    /// oldest entry. Nothing is recorded.
    pub fn back(&mut self) -> Option<Stop> {
        self.can_back().then(|| {
            self.cursor -= 1;
            self.entries[self.cursor].clone()
        })
    }

    /// Step the cursor forward one entry, or `None` at the newest one. Equally not a push.
    pub fn forward(&mut self) -> Option<Stop> {
        self.can_forward().then(|| {
            self.cursor += 1;
            self.entries[self.cursor].clone()
        })
    }
}

#[cfg(test)]
mod tests;

//! Where one tab has been: a browser-style back/forward trail over [`Stop`], one per tab.
//! Everywhere the reader has been across every tab is [`crate::visits::Visits`].
//!
//! The places are an [`Order`], as the visits are; what a trail adds is the cursor, and
//! the rule that going somewhere new abandons whatever was in front of it. Entries are
//! compared by `Arc` pointer, so entries made before a re-parse never compare equal to
//! ones made after it. Persisted as [`crate::project::SavedTab`].

use std::sync::Arc;

use analysis::Object;

use crate::order::Order;
use crate::project::Document;

/// The most entries a trail ever holds; the oldest are dropped past it. Per tab, so it is
/// modest: every entry is saved with the session, rows and all.
const MAX_ENTRIES: usize = 50;

/// One place on a tab's trail: a document, and where in it the tab was.
///
/// Two documents are visited at more than one place, and each says where in its own
/// terms: an object's code by the address, a source file by the line. Following a link
/// into either moves what is drawn rather than opening anything, so the trail is the
/// only record that the reader was somewhere else in it a moment ago. A symbol *is* the
/// place, and carries neither, nor does a document opened at no place in particular -- a
/// file a reader asked for by name is the file and not a line of it.
///
/// **Where a stop is goes with the kind of document it is in**, so the two are written
/// together: [`Stop::at`] takes the object whose code the address is in, [`Stop::on`] the
/// file the line is of, and [`Stop::whole`] neither. Those three are the only ways to
/// make one, and the place itself is private, so a line of an object's code and an
/// address in a file are states no caller can build -- which is what lets [`Stop::place`]
/// hand back the pair and spares every reader an arm for a place that cannot happen.
#[derive(Clone, PartialEq)]
pub struct Stop {
    pub document: Document,
    /// Where in `document`, and [`None`] for the document itself.
    place: Option<Inside>,
}

/// The half of a place a [`Stop`] stores; the `document` beside it is the other half.
#[derive(Clone, Copy, PartialEq)]
enum Inside {
    /// A placed address, only ever beside a [`Document::Code`].
    Address(u64),
    /// A line, only ever beside a [`Document::Source`].
    Line(u32),
}

/// Where a stop is inside its document: what [`Stop::place`] hands back, so that nothing
/// reading a stop has to say what an address in a source file or a line of an object's
/// code would mean.
///
/// The document is [`Stop::document`] and is not repeated -- except the **object**, which
/// a reader of an address wants and could otherwise take off the document only through an
/// arm for the object that is not there.
#[derive(Clone, Copy)]
pub enum Place<'a> {
    /// The document itself, at no place in particular.
    Whole,
    /// A placed address in an object's code, and the object whose code it is.
    Code(&'a Arc<Object>, u64),
    /// A line of a source file. 1-based, as DWARF's are.
    Source(u32),
}

impl From<Document> for Stop {
    fn from(document: Document) -> Stop {
        Stop::whole(document)
    }
}

impl Stop {
    /// A whole document, at no place in particular: what a door that names nowhere in
    /// particular makes.
    pub fn whole(document: Document) -> Stop {
        Stop {
            document,
            place: None,
        }
    }

    /// A place in `object`'s code, at a placed address.
    pub fn at(object: Arc<Object>, address: u64) -> Stop {
        Stop {
            document: Document::Code(object),
            place: Some(Inside::Address(address)),
        }
    }

    /// A place in the source file `file`, on a line. 1-based, as DWARF's are.
    pub fn on(file: Arc<str>, line: u32) -> Stop {
        Stop {
            document: Document::Source(file),
            place: Some(Inside::Line(line)),
        }
    }

    /// Where this is inside its document.
    pub fn place(&self) -> Place<'_> {
        match (&self.document, self.place) {
            (Document::Code(object), Some(Inside::Address(address))) => {
                Place::Code(object, address)
            }
            (Document::Source(_), Some(Inside::Line(line))) => Place::Source(line),
            // The constructors write the place beside the document it belongs to and
            // nothing else writes it, so what is left is the documents that carry none.
            _ => Place::Whole,
        }
    }

    /// The line of the file this is, for a stop in a source file, and [`None`] for every
    /// other place.
    pub fn line(&self) -> Option<u32> {
        match self.place() {
            Place::Source(line) => Some(line),
            Place::Whole | Place::Code(..) => None,
        }
    }

    /// Whether this names a place *inside* its document rather than the document itself.
    /// What decides whether arriving at it is a move worth putting on the trail: a door
    /// that names only a document has nothing to come back to that the document is not.
    pub fn inside(&self) -> bool {
        self.place.is_some()
    }
}

/// A list of visited places plus a cursor into it.
///
/// The places are an [`Order`], newest first, so no two entries are ever equal: a
/// revisited place is bumped to the front rather than appended a second time. A document
/// can therefore be on one trail more than once, at one address each, which is what lets
/// Back walk the places a reader followed inside an object's code; two stops at one place
/// are still one entry, which is what lets a tab and a document name the position its
/// panes are kept by.
#[derive(Clone, Default, PartialEq)]
pub struct History {
    entries: Order<Stop>,
    /// In range whenever `entries` is non-empty, and `0` — meaning nothing — while empty.
    cursor: usize,
}

impl History {
    /// A history rebuilt from a saved session: `entries` newest first, cursor on
    /// `entries[cursor]`.
    ///
    /// The entries come from outside, so the invariants are enforced rather than assumed:
    /// the cursor is clamped into range, duplicates are collapsed onto their newest
    /// occurrence with the cursor following the *entry* it was on, and the list is then
    /// trimmed to the newest [`MAX_ENTRIES`]. A trim that drops the entry the cursor was
    /// on leaves it on the oldest survivor.
    pub fn restored(entries: Vec<Stop>, cursor: usize) -> History {
        let current = entries
            .get(cursor.min(entries.len().saturating_sub(1)))
            .cloned();

        // Collecting collapses the duplicates, keeping the newest of each; the cursor
        // then follows its own entry, which is still in there. `unwrap_or(0)` is only
        // ever the empty history.
        let mut entries: Order<Stop> = entries.into_iter().collect();
        let cursor = current
            .and_then(|current| entries.position(&current))
            .unwrap_or(0);

        entries.truncate(MAX_ENTRIES);
        History {
            cursor: cursor.min(entries.len().saturating_sub(1)),
            entries,
        }
    }

    /// A history rebuilt from entries that may no longer point anywhere: one [`Option`]
    /// per entry, newest first and `None` where the entry is gone, with `cursor` an index
    /// into *that* list rather than into what survives.
    ///
    /// The cursor is left on the newest survivor at or older than it, falling back to the
    /// oldest survivor when nothing older survived. The one walk both a restore and a
    /// file close go through.
    pub fn rebuilt(entries: impl IntoIterator<Item = Option<Stop>>, cursor: usize) -> History {
        let mut kept = Vec::new();
        let mut moved = None;

        for (index, entry) in entries.into_iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            if index >= cursor && moved.is_none() {
                moved = Some(kept.len());
            }
            kept.push(entry);
        }

        let moved = moved.unwrap_or_else(|| kept.len().saturating_sub(1));
        History::restored(kept, moved)
    }

    /// The same history with only the entries `keep` accepts, the cursor carried the way
    /// [`History::rebuilt`] carries it.
    pub fn retaining(&self, keep: impl Fn(&Document) -> bool) -> History {
        History::rebuilt(
            self.entries()
                .iter()
                .map(|entry| keep(&entry.document).then(|| entry.clone())),
            self.cursor,
        )
    }

    /// Every entry, newest first — what persistence saves.
    pub fn entries(&self) -> &[Stop] {
        self.entries.entries()
    }

    /// The entry the cursor is on, or `None` before anything has been recorded.
    pub fn current(&self) -> Option<&Stop> {
        self.entries.get(self.cursor)
    }

    /// Whether [`History::push`] would record `stop`. False for the entry already under
    /// the cursor, which is what stops back/forward from re-recording where they have
    /// just moved it. Not [`Order::would_touch`], which asks about the front: the cursor
    /// is where the reader is, and the front is only where they were last.
    pub fn would_push(&self, stop: &Stop) -> bool {
        self.current() != Some(stop)
    }

    /// Record `stop` as the newest entry and put the cursor on it. A no-op when
    /// [`History::would_push`] is false.
    ///
    /// Anything in front of the cursor is abandoned first, so the cursor's own entry is
    /// the newest before `stop` goes in front of it and the cursor is `0` however the
    /// list was arranged. An equal entry still behind it is bumped rather than
    /// duplicated, and the cap then drops the oldest.
    pub fn push(&mut self, stop: impl Into<Stop>) {
        let stop = stop.into();
        if !self.would_push(&stop) {
            return;
        }

        self.entries.drop_newer_than(self.cursor);
        self.entries.touch(stop);
        self.cursor = 0;
        self.entries.truncate(MAX_ENTRIES);
    }

    /// The index of the entry the cursor is on, or `None` before anything has been
    /// recorded.
    pub fn cursor(&self) -> Option<usize> {
        (self.cursor < self.entries.len()).then_some(self.cursor)
    }

    pub fn can_back(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn can_forward(&self) -> bool {
        self.cursor > 0
    }

    /// Step the cursor back one entry and hand back what is now current, or `None` at the
    /// oldest entry. Nothing is recorded.
    pub fn back(&mut self) -> Option<Stop> {
        self.can_back().then(|| {
            self.cursor += 1;
            self.entries()[self.cursor].clone()
        })
    }

    /// Step the cursor forward one entry, or `None` at the newest one. Equally not a push.
    pub fn forward(&mut self) -> Option<Stop> {
        self.can_forward().then(|| {
            self.cursor -= 1;
            self.entries()[self.cursor].clone()
        })
    }
}

#[cfg(test)]
mod tests;

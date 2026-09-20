//! Where one tab has been: a browser-style back/forward trail over [`Stop`], one per tab.
//! Everywhere the reader has been across every tab is [`crate::visits::Visits`].
//!
//! The places are an [`Order`], as the visits are; what a trail adds is the cursor, and
//! the rule that going somewhere new abandons whatever was in front of it. Entries are
//! compared by `Arc` pointer, so entries made before a re-parse never compare equal to
//! ones made after it. Persisted as [`crate::project::SavedTab`].

use std::sync::Arc;

use analysis::{Object, PlacedAddress, SectionAddress, Symbol};

use crate::document::{Address, Document};
use crate::order::Order;

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
/// file a reader asked for by name is the file and not a line of it. The one exception
/// is a symbol's call to itself: following it moves nothing either, so the place it lands
/// on is an instruction of the symbol ([`Stop::in_symbol`]), and Back comes back to the
/// call.
///
/// **Where a stop is goes with the kind of document it is in**, so the two are written
/// together: [`Stop::at`] takes the object whose code the address is in,
/// [`Stop::in_symbol`] the symbol, [`Stop::on`] the file the line is of, and
/// [`Stop::whole`] none. Those four are the only ways to make one, and the place itself
/// is private, so a line of an object's code and an address in a file are states no
/// caller can build -- which is what lets [`Stop::place`] hand back the pair and spares
/// every reader an arm for a place that cannot happen.
#[derive(Clone, PartialEq)]
pub struct Stop {
    pub document: Document,
    /// Where in `document`, and [`None`] for the document itself.
    place: Option<Inside>,
}

/// The half of a place a [`Stop`] stores; the `document` beside it is the other half.
#[derive(Clone, Copy, PartialEq)]
enum Inside {
    /// An address in whichever space its document is in ([`Address`]): placed beside a
    /// [`Document::Code`], the symbol's own beside a [`Document::Symbol`].
    Address(Address),
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
    Code(&'a Arc<Object>, PlacedAddress),
    /// An instruction of a symbol, at the symbol's own address for it.
    Instruction(SectionAddress),
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
    pub fn at(object: Arc<Object>, address: PlacedAddress) -> Stop {
        Stop {
            document: Document::Code(object),
            place: Some(Inside::Address(Address::Placed(address))),
        }
    }

    /// An instruction of `symbol`, at the symbol's own address for it: where following a
    /// call it makes to itself lands.
    pub fn in_symbol(symbol: Symbol, address: SectionAddress) -> Stop {
        Stop {
            document: Document::Symbol(symbol),
            place: Some(Inside::Address(Address::Local(address))),
        }
    }

    /// A place in the source file `file`, on a line. 1-based, as DWARF's are.
    pub fn on(file: Arc<str>, line: u32) -> Stop {
        Stop {
            document: Document::Source(file),
            place: Some(Inside::Line(line)),
        }
    }

    /// `document` at whichever of the two halves belongs to it, and the document itself
    /// where neither does.
    ///
    /// **The one place a document is paired with halves stated apart.** A saved place
    /// and a landing each state them loose -- an address, a line and a document, each
    /// its own value -- and can therefore state a pairing that means nothing, so a place
    /// whose half does not belong to its document is the document itself and not a
    /// guess. The **space** is half of belonging: a placed address beside a symbol, or a
    /// symbol's own beside an object's code, is as much a pairing that means nothing as
    /// a line beside either, and falls through here the same way.
    pub fn paired(document: Document, address: Option<Address>, line: Option<u32>) -> Stop {
        match (document, address, line) {
            (Document::Code(object), Some(Address::Placed(address)), _) => {
                Stop::at(object, address)
            }
            (Document::Symbol(symbol), Some(Address::Local(address)), _) => {
                Stop::in_symbol(symbol, address)
            }
            (Document::Source(file), _, Some(line)) => Stop::on(file, line),
            (document, _, _) => Stop::whole(document),
        }
    }

    /// Where this is inside its document.
    pub fn place(&self) -> Place<'_> {
        match (&self.document, self.place) {
            (Document::Code(object), Some(Inside::Address(Address::Placed(address)))) => {
                Place::Code(object, address)
            }
            (Document::Symbol(_), Some(Inside::Address(Address::Local(address)))) => {
                Place::Instruction(address)
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
            Place::Whole | Place::Code(..) | Place::Instruction(_) => None,
        }
    }

    /// The address this is at, for a stop in an object's code -- placed -- or at an
    /// instruction of a symbol -- the symbol's own -- and [`None`] for every other place.
    pub fn address(&self) -> Option<Address> {
        match self.place() {
            Place::Code(_, address) => Some(Address::Placed(address)),
            Place::Instruction(address) => Some(Address::Local(address)),
            Place::Whole | Place::Source(_) => None,
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

        // Restoring collapses the duplicates, keeping the newest of each, and then cuts
        // to the cap; the cursor follows its own entry through both. An entry that is gone
        // was cut, since the collapse keeps one of every entry -- so it was among the
        // oldest, and the oldest survivor is where the cursor lands. The empty history's
        // answer is the same.
        let entries = Order::restored_within(entries, MAX_ENTRIES);
        let cursor = current
            .and_then(|current| entries.position(&current))
            .unwrap_or_else(|| entries.len().saturating_sub(1));

        History { cursor, entries }
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

    /// Record `stop` as the newest entry and put the cursor on it, and say whether it
    /// was recorded. Not for the entry already under the cursor, which is what stops
    /// back/forward from re-recording where they have just moved it. Not
    /// [`Order::touch`]'s test, which asks about the front: the cursor is where the reader
    /// is, and the front is only where they were last.
    ///
    /// Anything in front of the cursor is abandoned first, so the cursor's own entry is
    /// the newest before `stop` goes in front of it and the cursor is `0` however the
    /// list was arranged. An equal entry still behind it is bumped rather than
    /// duplicated, and the cap then drops the oldest.
    pub fn push(&mut self, stop: impl Into<Stop>) -> bool {
        let stop = stop.into();
        if self.current() == Some(&stop) {
            return false;
        }

        self.entries.drop_newer_than(self.cursor);
        self.entries.touch_within(stop, MAX_ENTRIES);
        self.cursor = 0;
        true
    }

    /// The index of the entry the cursor is on, or `None` before anything has been
    /// recorded.
    pub fn cursor(&self) -> Option<usize> {
        (self.cursor < self.entries.len()).then_some(self.cursor)
    }

    /// The entry a step back would land on, or `None` at the oldest one. The list is
    /// newest first, so a step back is a step *up* the indices.
    ///
    /// **The one place a step's destination is worked out**, and the one place "can it be
    /// taken" is answered: they are the same question, and a second spelling of either is
    /// a second rule to keep in step. [`History::back`] moves by this, and so does the
    /// toolbar's tooltip through `Nav::destination` (`src/ui/documents.rs`), so a live
    /// button and a step that does something cannot disagree.
    pub fn behind(&self) -> Option<&Stop> {
        self.entries.get(self.cursor + 1)
    }

    /// The entry a step forward would land on, or `None` at the newest one. The other
    /// half of [`History::behind`], asked by the same callers.
    pub fn ahead(&self) -> Option<&Stop> {
        self.entries.get(self.cursor.checked_sub(1)?)
    }

    /// Step the cursor back one entry and hand back what is now current, or `None` at the
    /// oldest entry. Nothing is recorded.
    pub fn back(&mut self) -> Option<Stop> {
        let stop = self.behind()?.clone();
        self.cursor += 1;
        Some(stop)
    }

    /// Step the cursor forward one entry, or `None` at the newest one. Equally not a push.
    pub fn forward(&mut self) -> Option<Stop> {
        let stop = self.ahead()?.clone();
        self.cursor -= 1;
        Some(stop)
    }
}

#[cfg(test)]
mod tests;

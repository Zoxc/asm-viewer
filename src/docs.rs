//! The documents that are open, and the handle each one is known by.
//!
//! Framework-free, like [`crate::tabs`] and [`crate::history`] — a table with two rules
//! on it and no pixels, so the rules are unit-tested without mounting a UI.
//!
//! It exists because of a type bound and not because of a design preference. An open
//! document is a tab in the dock, and freya's `DockingModel::TabId` is
//! `Copy + PartialEq + Hash + 'static`. A [`crate::project::Document`] is none of those:
//! it holds an `Arc<Object>`, an `Arc<SymbolData>` or an `Arc<str>`, it compares by
//! pointer identity where it is a place in a binary and by text where it is a file, and
//! it hashes by nothing at all. So the dock holds a [`DocId`], which is a `u32`, and this
//! is what turns one back into the document it stands for.
//!
//! **This is a side table and not the list of open documents.** The order the reader put
//! their tabs in is the dock panel's own `tabs` vec, and there is deliberately no second
//! copy of it here — two lists that have to agree is the thing this app refuses
//! everywhere else, and it is the same reason [`crate::tabs::Tabs`] holds no cursor. What
//! this answers is only "which document is this id", and "does this document already have
//! one". Membership is the one thing the two share: an entry exists here exactly while a
//! tab holds its id, which is an invariant the closing functions keep and the tests
//! assert.

use crate::project::Document;
use std::collections::HashMap;

/// The handle a dock tab holds in place of the document it shows.
///
/// A `u32` newtype rather than a bare `u32` so it cannot be confused with the panel ids
/// the dock also numbers from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocId(u32);

/// Every open document, by the id its tab is known by.
#[derive(Default)]
pub struct Docs {
    open: HashMap<DocId, Document>,
    /// The next id to hand out. Only ever counts up — see [`Docs::open`].
    next: u32,
}

impl Docs {
    /// Take an id for `document` and remember it under that id.
    ///
    /// **Ids are never reused**, which is the whole reason this counts up rather than
    /// filling gaps, and it is load-bearing twice. freya keys a tab's header element by
    /// its id, so a reused id would have a closed document's header state carried onto
    /// whichever document took its number. And a drag is a value the pointer is carrying:
    /// a drag begun before its document was closed would otherwise be dropped onto a live
    /// document it was never picked up from. At one id per tab ever opened, `u32` is not
    /// a number a reader reaches.
    pub fn open(&mut self, document: Document) -> DocId {
        let id = DocId(self.next);
        self.next += 1;
        self.open.insert(id, document);
        id
    }

    /// Forget `id` and the document it stood for. Closing an id that is not open is a
    /// no-op, the way closing a tab that is not open is.
    pub fn close(&mut self, id: DocId) {
        self.open.remove(&id);
    }

    /// The document `id` stands for, or `None` when it stands for nothing — a tab that
    /// has been closed, or an id from a drag that outlived its document.
    pub fn get(&self, id: DocId) -> Option<&Document> {
        self.open.get(&id)
    }

    /// The id `document` is already open under, or `None` when it is not open.
    ///
    /// A scan, the way [`crate::tabs::Tabs::find`] is one, and for the same reason: a
    /// `Document` hashes by nothing, and there are dozens of these rather than thousands.
    pub fn id_of(&self, document: &Document) -> Option<DocId> {
        self.open
            .iter()
            .find(|(_, open)| *open == document)
            .map(|(id, _)| *id)
    }

    /// How many documents are open. The dock panel is what says in which *order*; this
    /// is what a test asserts the two hold the same set with.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.open.len()
    }
}

#[cfg(test)]
mod tests;

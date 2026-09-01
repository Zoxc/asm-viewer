//! The documents that are open, and the handle each one is known by.
//!
//! The dock holds a [`DocId`] rather than a [`crate::project::Document`] because freya's
//! `DockingModel::TabId` is `Copy + PartialEq + Hash + 'static` and a `Document` is none
//! of those. This is a side table, not the list of open documents: the order is the dock
//! panel's own `tabs` vec. An entry exists here exactly while a tab holds its id.

use crate::project::Document;
use std::collections::HashMap;

/// The handle a dock tab holds in place of the document it shows. A newtype so it cannot
/// be confused with the panel ids the dock also numbers from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DocId(u32);

/// Every open document, by the id its tab is known by.
#[derive(Default)]
pub struct Docs {
    open: HashMap<DocId, Document>,
    next: u32,
}

impl Docs {
    /// Take an id for `document` and remember it under that id.
    ///
    /// **Ids are never reused**: freya keys a tab's header element by its id, and a drag
    /// carries one, so a reused id would land a closed document's header state — or a
    /// drag begun before it was closed — on whichever document took its number.
    pub fn open(&mut self, document: Document) -> DocId {
        let id = DocId(self.next);
        self.next += 1;
        self.open.insert(id, document);
        id
    }

    /// Forget `id` and the document it stood for.
    pub fn close(&mut self, id: DocId) {
        self.open.remove(&id);
    }

    /// The document `id` stands for, or `None` for a closed tab or an id from a drag that
    /// outlived its document.
    pub fn get(&self, id: DocId) -> Option<&Document> {
        self.open.get(&id)
    }

    /// The id `document` is already open under, or `None` when it is not open. A scan,
    /// since a `Document` hashes by nothing and there are dozens of these.
    pub fn id_of(&self, document: &Document) -> Option<DocId> {
        self.open
            .iter()
            .find(|(_, open)| *open == document)
            .map(|(id, _)| *id)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.open.len()
    }
}

#[cfg(test)]
mod tests;

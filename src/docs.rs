//! The documents that are open, and the handle each one is known by.
//!
//! The dock holds a [`DocId`] rather than a [`crate::project::Document`] because freya's
//! `DockingModel::TabId` is `Copy + PartialEq + Hash + 'static` and a `Document` is none
//! of those. This is a side table, not the list of open documents: the order is the dock
//! panel's own `tabs` vec. An entry exists here exactly while a tab holds its id.
//!
//! What a tab stands for is a **trail** and not one document: every place the tab has
//! shown, with a cursor on the one it shows now ([`History`]). Back and Forward walk the
//! trail of the tab on screen and no other. One tab may be **temporal**: the preview tab a
//! click from outside the panes -- a sidebar row -- opens its place in, and the next such
//! click reuses, so walking down a list leaves no tab behind per row.

use std::collections::HashMap;

use crate::history::History;
use crate::project::Document;

/// The handle a dock tab holds in place of the document it shows. A newtype so it cannot
/// be confused with the panel ids the dock also numbers from zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(u32);

/// One place on one tab's trail: the tab, and a document it has shown. What a tab's
/// viewing positions and driven line are kept by, so going back restores the rows the
/// reader left on both sides. A trail holds no two equal documents, so the pair names one
/// entry.
pub type Entry = (DocId, Document);

/// Every open tab's trail, by the id the tab is known by, and which tab is the temporal
/// one.
#[derive(Default)]
pub struct Docs {
    open: HashMap<DocId, History>,
    temporal: Option<DocId>,
    next: u32,
}

impl Docs {
    /// Take an id for a tab showing `document` alone, and remember it under that id.
    ///
    /// **Ids are never reused**: freya keys a tab's header element by its id, and a drag
    /// carries one, so a reused id would land a closed document's header state — or a
    /// drag begun before it was closed — on whichever document took its number.
    pub fn open(&mut self, document: Document) -> DocId {
        let mut trail = History::default();
        trail.push(document);
        self.open_trail(trail, false)
            .expect("a trail of one entry has a current entry")
    }

    /// Take an id for a tab with the whole of `trail` behind it, marked temporal or not:
    /// what a restored session opens. `None` for a trail with nothing on it, which is no
    /// tab at all; nothing is taken for it.
    pub fn open_trail(&mut self, trail: History, temporal: bool) -> Option<DocId> {
        trail.current()?;
        let id = DocId(self.next);
        self.next += 1;
        self.open.insert(id, trail);
        if temporal {
            self.temporal = Some(id);
        }
        Some(id)
    }

    /// Forget `id` and the trail it stood for. A temporal tab closed is no longer the
    /// temporal one, so the next preview opens a tab of its own.
    pub fn close(&mut self, id: DocId) {
        self.open.remove(&id);
        if self.temporal == Some(id) {
            self.temporal = None;
        }
    }

    /// The document `id` shows now -- the entry under its trail's cursor -- or `None` for
    /// a closed tab or an id from a drag that outlived its document.
    pub fn get(&self, id: DocId) -> Option<&Document> {
        self.open.get(&id)?.current()
    }

    /// The trail behind `id`, or `None` for a closed tab.
    pub fn trail(&self, id: DocId) -> Option<&History> {
        self.open.get(&id)
    }

    /// The same trail, to move along or push onto.
    pub fn trail_mut(&mut self, id: DocId) -> Option<&mut History> {
        self.open.get_mut(&id)
    }

    /// The tab showing `document` now, or `None` when no tab does. The lowest id where
    /// several do: two tabs can show one place, and the answer must not depend on the
    /// order a `HashMap` happens to walk. A scan, since a `Document` hashes by nothing and
    /// there are dozens of these.
    pub fn showing(&self, document: &Document) -> Option<DocId> {
        self.open
            .iter()
            .filter(|(_, trail)| trail.current() == Some(document))
            .map(|(id, _)| *id)
            .min()
    }

    /// Whether `document` is anywhere on the trail of `id` -- what a pane asks before
    /// writing a viewing position down, so a row of a listing that has just been closed
    /// is not put straight back.
    pub fn contains(&self, id: DocId, document: &Document) -> bool {
        self.open
            .get(&id)
            .is_some_and(|trail| trail.entries().contains(document))
    }

    /// The temporal tab, while there is one.
    pub fn temporal(&self) -> Option<DocId> {
        self.temporal.filter(|id| self.open.contains_key(id))
    }

    /// Make `id` the temporal tab, in place of whichever was. Only an open tab can be.
    pub fn mark_temporal(&mut self, id: DocId) {
        if self.open.contains_key(&id) {
            self.temporal = Some(id);
        }
    }

    /// Make `id` a tab that stays, if it was the temporal one; nothing else changes, the
    /// tab keeping its slot and its trail.
    pub fn promote(&mut self, id: DocId) {
        if self.temporal == Some(id) {
            self.temporal = None;
        }
    }

    /// Take every entry `keep` rejects off every trail, each cursor carried the way
    /// [`History::retaining`] carries it: what a closing binary does to the tabs it does
    /// not close. The caller has already closed every tab whose *current* entry `keep`
    /// rejects, so no trail is left with nothing on it.
    pub fn retain_entries(&mut self, keep: impl Fn(&Document) -> bool) {
        for trail in self.open.values_mut() {
            *trail = trail.retaining(&keep);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.open.len()
    }
}

#[cfg(test)]
impl DocId {
    /// An id no tab has ever had, for a pane a test mounts with no tab behind it.
    pub fn stray() -> DocId {
        DocId(u32::MAX)
    }
}

#[cfg(test)]
mod tests;

//! The reader's own list of places: bookmarks.
//!
//! A bookmark is a [`SavedDocument`] and the name it was made under, kept in the order the
//! reader added them and saved with the project (`crate::project`). It holds no `Arc`: a
//! bookmark outlives the binary it points into, so whether it is *live* is a question asked
//! of what is loaded now and never a thing the list remembers.

use std::sync::Arc;

use analysis::Object;
use serde::{Deserialize, Serialize};

use crate::project::{Document, SavedDocument};

/// One bookmark: where it points and what to call it.
///
/// The name is stored because a saved symbol carries only its mangled name, and a bookmark
/// whose binary is closed has nothing else to be drawn by. It is the whole name and not the
/// shortened one a row draws, so what a filter matches is what the tooltip says. Field
/// order is load-bearing (`crate::project`): the name is a plain value and has to reach the
/// file before the table its document is written as.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub document: SavedDocument,
}

/// The bookmarks, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    entries: Vec<Bookmark>,
}

impl Bookmarks {
    pub fn from_entries(entries: Vec<Bookmark>) -> Bookmarks {
        Bookmarks { entries }
    }

    pub fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    /// The bookmark that points at `document`, as the index of its entry.
    ///
    /// Asked by **resolving** each entry against `objects` rather than by comparing saved
    /// forms: a rebuild moves a symbol, and the entry keeps the address it was made at
    /// while the live symbol carries the new one, so the two saved forms would never agree
    /// again about a bookmark the panel is drawing live.
    pub fn matching(&self, document: &Document, objects: &[Arc<Object>]) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.document.resolve_by_name(objects).as_ref() == Some(document))
    }

    /// Remove the bookmark pointing at `document`, or add one called `name` at the end.
    /// Answers whether one was added.
    pub fn toggle(
        &mut self,
        document: &Document,
        name: impl Into<String>,
        objects: &[Arc<Object>],
    ) -> bool {
        if let Some(index) = self.matching(document, objects) {
            self.entries.remove(index);
            return false;
        }
        self.entries.push(Bookmark {
            name: name.into(),
            document: SavedDocument::from_document(document),
        });
        true
    }

    /// Remove the entry at `index`, which is how a bookmark that no longer resolves — and
    /// so names no document to be asked about — is let go of.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }
}

#[cfg(test)]
mod tests;

//! The reader's own list of places: bookmarks.
//!
//! A bookmark is a [`SavedDocument`] and the name it was made under, kept in the order the
//! reader added them and saved with the project (`crate::project`). It holds no `Arc`: a
//! bookmark outlives the binary it points into, so whether it is *live* is a question asked
//! of what is loaded now and never a thing the list remembers.

use std::{borrow::Cow, sync::Arc};

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
///
/// It is **absent** where the place spells its own name — a symbol the app named rather
/// than the file, which is saved as which name it is and an address
/// ([`SavedDocument::made_up_name`]). Storing the spelling here would put it back in the
/// file, and a spelling in the file is the one thing that would stop the app changing it.
/// [`Bookmark::label`] is what a row draws either way.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Bookmark {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub document: SavedDocument,
}

impl Bookmark {
    /// A bookmark of `document`, called `name` where the place cannot say what it is
    /// called itself. The one place that rule is applied.
    pub fn new(document: SavedDocument, name: impl Into<String>) -> Bookmark {
        Bookmark {
            name: document.made_up_name().is_none().then(|| name.into()),
            document,
        }
    }

    /// What to call this: the name it was made under, or the place's own, which is
    /// rendered afresh and so is never a spelling the app has since stopped using.
    pub fn label(&self) -> Cow<'_, str> {
        match self.document.made_up_name() {
            Some(name) => Cow::Owned(name),
            None => Cow::Borrowed(self.name.as_deref().unwrap_or_default()),
        }
    }
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
        self.entries
            .push(Bookmark::new(SavedDocument::from_document(document), name));
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

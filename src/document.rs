//! What the reader has open: a place in a binary, or a source file. Framework-free.
//!
//! [`Document`] is what every tab, trail, visit and bookmark is keyed by, and
//! [`Selection`] is the assembly-driven half of one: an object, or a symbol in it. Both
//! compare by `Arc` pointer identity, the app's rule everywhere; a source file compares
//! as text, so the same file reached two ways is one tab.
//!
//! [`Pane`] is the two sides every tab has, and [`Document::driven_from`] which of them
//! the reader came for.
//!
//! Where a document was *left* is [`Positions`](crate::positions::Positions); how one is
//! written to a file is [`SavedDocument`](crate::project::SavedDocument).

use std::{path::Path, sync::Arc};

use analysis::{Object, Symbol};

/// What is currently selected in the UI. There is no "nothing" variant: having none is an
/// absent one, `Option<Selection>`.
#[derive(Clone)]
pub enum Selection {
    Object(Arc<Object>),
    Symbol(Symbol),
}

impl Selection {
    /// Whether this points into the file at `path`. A symbol answers for the file its
    /// *object* came out of, and `path` is [`Object::path`] and never an object's name,
    /// so an archive closes members and all.
    pub fn in_file(&self, path: &Path) -> bool {
        self.file() == path
    }

    /// The file it came out of: an archive for a member, and never an object's name.
    pub fn file(&self) -> &Path {
        match self {
            Selection::Object(object) => &object.path,
            Selection::Symbol(symbol) => &symbol.object.path,
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Selection::Object(a), Selection::Object(b)) => Arc::ptr_eq(a, b),
            (Selection::Symbol(a), Selection::Symbol(b)) => a == b,
            _ => false,
        }
    }
}

/// One of the two panes that show code.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Assembly,
    Source,
}

/// One of the places the reader has open: a place in a binary, or a file.
///
/// A tab holds one of these and has two sides — assembly and source — and the variant
/// says which side the tab is *about* and therefore which one drives the other. A file is
/// a string and not a `PathBuf`: the spelling the debug info said, or the project directory
/// joined with a Files row's entries, which is deliberately the same spelling and is never
/// canonicalised, since the two are compared as text and a file reached both ways is one tab.
///
/// [`Code`](Document::Code) is a third kind: **all of one object's code** as one listing,
/// the symbols drawn as labels inside it where they start. It is assembly-driven like a
/// symbol's tab, and one per object rather than one per place in it — where the reader
/// was in it is the tab's position, not its identity.
#[derive(Clone)]
pub enum Document {
    Assembly(Selection),
    Source(Arc<str>),
    Code(Arc<Object>),
}

impl Document {
    /// Whether this points into the file at `path`. A source-driven document answers
    /// **false** whatever the path: a file chip outlives the binary that led the reader
    /// to it.
    pub fn in_file(&self, path: &Path) -> bool {
        !matches!(self, Document::Source(_)) && self.file() == path
    }

    /// The file on disk this is a place in: the binary for the two assembly-driven
    /// kinds, and the source file itself for a file. Spelled the way the document is,
    /// so a relative one stays relative.
    pub fn file(&self) -> &Path {
        match self {
            Document::Assembly(selection) => selection.file(),
            Document::Source(file) => Path::new(&**file),
            Document::Code(object) => &object.path,
        }
    }

    /// The side this is driven from: the pane the reader came here to read, which the
    /// other one follows. A file is read for itself and the assembly follows it; an
    /// object and a symbol are read as code and the source follows.
    pub fn driven_from(&self) -> Pane {
        match self {
            Document::Source(_) => Pane::Source,
            Document::Assembly(_) | Document::Code(_) => Pane::Assembly,
        }
    }

    /// The symbol this is about — a document that is a function, and not one that is an
    /// object or a file. What the analysis worker is asked for.
    pub fn symbol(&self) -> Option<&Symbol> {
        match self {
            Document::Assembly(Selection::Symbol(symbol)) => Some(symbol),
            _ => None,
        }
    }
}

impl PartialEq for Document {
    /// Each variant by its own rule — `Arc` pointer identity for a selection and for an
    /// object's code, text for a file — and never across the kinds: an object's code and
    /// the object itself are two documents.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Document::Assembly(a), Document::Assembly(b)) => a == b,
            (Document::Source(a), Document::Source(b)) => a == b,
            (Document::Code(a), Document::Code(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;

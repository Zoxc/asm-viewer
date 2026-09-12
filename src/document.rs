//! What the reader has open: a place in a binary, or a source file. Framework-free.
//!
//! [`Document`] is what every tab, trail, visit and bookmark is keyed by. The three
//! assembly-driven kinds compare by `Arc` pointer identity, the app's rule everywhere;
//! a source file compares as text, so the same file reached two ways is one tab.
//!
//! [`Pane`] is the two sides every tab has, and [`Document::driven_from`] which of them
//! the reader came for.
//!
//! Where a document was *left* is [`Positions`](crate::positions::Positions); how one is
//! written to a file is [`SavedDocument`](crate::project::SavedDocument).

use std::{path::Path, sync::Arc};

use analysis::{Object, Symbol};

/// One of the two panes that show code.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pane {
    Assembly,
    Source,
}

/// Which of the three kinds of place a document is, with nothing of what it points at.
///
/// It is what [`Document`] and [`SavedDocument`](crate::project::SavedDocument) agree
/// about: a saved place is drawn with the glyph its live tab wears, so the two answer one
/// question and neither can grow a kind the other forgets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A place in a binary: an object's symbols, or one symbol's code.
    Binary,
    /// A source file.
    Source,
    /// The whole of an object's code, as one listing.
    Code,
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
    /// An object's symbols.
    Object(Arc<Object>),
    /// One symbol's code.
    Symbol(Symbol),
    /// A source file.
    Source(Arc<str>),
    /// The whole of an object's code, as one listing.
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
            Document::Object(object) | Document::Code(object) => &object.path,
            Document::Symbol(symbol) => &symbol.object.path,
            Document::Source(file) => Path::new(&**file),
        }
    }

    /// Which of the three kinds of place this is: what a list draws it with, and all a
    /// glyph needs of it.
    pub fn kind(&self) -> Kind {
        match self {
            Document::Object(_) | Document::Symbol(_) => Kind::Binary,
            Document::Source(_) => Kind::Source,
            Document::Code(_) => Kind::Code,
        }
    }

    /// The side this is driven from: the pane the reader came here to read, which the
    /// other one follows. A file is read for itself and the assembly follows it; an
    /// object and a symbol are read as code and the source follows.
    pub fn driven_from(&self) -> Pane {
        match self {
            Document::Source(_) => Pane::Source,
            Document::Object(_) | Document::Symbol(_) | Document::Code(_) => Pane::Assembly,
        }
    }

    /// The object whose code this is — a document that is a whole binary, and not one
    /// that is a symbol or a file. What the section view's rows are of.
    pub fn code(&self) -> Option<&Arc<Object>> {
        match self {
            Document::Code(object) => Some(object),
            _ => None,
        }
    }

    /// The symbol this is about — a document that is a function, and not one that is an
    /// object or a file. What the analysis worker is asked for.
    pub fn symbol(&self) -> Option<&Symbol> {
        match self {
            Document::Symbol(symbol) => Some(symbol),
            _ => None,
        }
    }
}

impl PartialEq for Document {
    /// Each variant by its own rule — `Arc` pointer identity for an object and for an
    /// object's code, text for a file — and never across the kinds: an object's code and
    /// the object itself are two documents.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Document::Object(a), Document::Object(b)) => Arc::ptr_eq(a, b),
            (Document::Symbol(a), Document::Symbol(b)) => a == b,
            (Document::Source(a), Document::Source(b)) => a == b,
            (Document::Code(a), Document::Code(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;

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

use std::{fmt, path::Path, sync::Arc};

use analysis::{Object, PlacedAddress, SectionAddress, Symbol};

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

/// An address **in whichever of the two spaces its document is in**: the object's one
/// space for the whole of its code, one section's own for a symbol read alone.
///
/// The crate's two address types each name a space outright
/// ([`PlacedAddress`], [`SectionAddress`]); this is the one that has not committed to
/// either, and a caller must ask which it is before it can do anything with the number.
/// It is for the three places that hold an address **apart from** the document it belongs
/// to -- a landing, a planting, a place on a trail -- where neither type fits, because
/// which one is right is a fact about the document and not about the number.
///
/// [`in_document`](Self::in_document) is where a document says which, and
/// [`Stop::paired`](crate::history::Stop::paired) is where one is put back beside a
/// document: a pairing that means nothing -- a placed address beside a symbol -- is
/// dropped there rather than guessed at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Address {
    /// In the one space an object's sections share: what [`Object::symbol_at_placed`]
    /// answers in, and what a listing of the whole of its code is keyed by.
    ///
    /// [`Object::symbol_at_placed`]: analysis::Object::symbol_at_placed
    Placed(PlacedAddress),
    /// In one section's own terms, as [`Section::local`] answers: the addresses the file
    /// itself states, which are what a symbol read alone is listed in.
    ///
    /// [`Section::local`]: analysis::Section::local
    Local(SectionAddress),
}

impl Address {
    /// `address` in whichever space `document` is in, and [`None`] for a document that is
    /// in neither -- a source file, whose assembly side is whichever symbol its line was
    /// compiled into, and an object's symbol list, which is no place in any code.
    /// **The one place a loose number is given a space**, which is what the session file
    /// needs: it states the number and the document apart, and only the document can say
    /// which of the two the number was.
    pub fn in_document(document: &Document, address: u64) -> Option<Address> {
        match document {
            Document::Code(_) => Some(Address::Placed(PlacedAddress::new(address))),
            Document::Symbol(_) => Some(Address::Local(SectionAddress::new(address))),
            Document::Object(_) | Document::Source(_) => None,
        }
    }

    /// The placed address this is, and [`None`] where it is a symbol's own: what a reader
    /// of an object's code asks, rather than taking the number and hoping.
    pub fn placed(self) -> Option<PlacedAddress> {
        match self {
            Address::Placed(address) => Some(address),
            Address::Local(_) => None,
        }
    }

    /// The section's own address this is, and [`None`] where it is a placed one.
    pub fn local(self) -> Option<SectionAddress> {
        match self {
            Address::Local(address) => Some(address),
            Address::Placed(_) => None,
        }
    }

    /// The plain number, for what is written to a file.
    pub fn get(self) -> u64 {
        match self {
            Address::Placed(address) => address.get(),
            Address::Local(address) => address.get(),
        }
    }
}

/// Hex as either address prints, so a row draws one without first asking which it is.
impl fmt::UpperHex for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Address::Placed(address) => fmt::UpperHex::fmt(address, f),
            Address::Local(address) => fmt::UpperHex::fmt(address, f),
        }
    }
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

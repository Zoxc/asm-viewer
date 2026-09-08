//! What a document is called and drawn as wherever a list names one: a tab's chip, a
//! sidebar row, a bookmark, a place on a trail.
//!
//! Nothing here opens or closes anything. [`entry_text`] is the short spelling a chip and
//! a row draw and [`entry_name`] the whole one, which is what a tooltip says and what a
//! filter reads -- so a generic argument no tab draws is still something to search for.
//! [`entry_key`] is the identity a row or a chip is keyed by.

use super::*;

/// What a stop on a trail is called: the document's own name, except for a place inside
/// one, which says where. A place in an object's code is named by the symbol
/// **starting** at that address -- so stepping back through a listing says which function
/// each step goes to and not the object's name three times over -- and a place no symbol
/// starts at, which is what a call into the middle of a function makes, has no name to
/// give and is the object's. A place in a file is the file and the line, which is all
/// that tells two of them apart.
pub(crate) fn stop_text(stop: &Stop) -> String {
    match stop.place() {
        Place::Code(object, address) => match object.symbol_at(address) {
            Some(symbol) => short_name(symbol.display()),
            None => entry_text(&stop.document),
        },
        Place::Source(line) => format!("{}:{line}", entry_text(&stop.document)),
        Place::Whole => entry_text(&stop.document),
    }
}

/// What a document is called where it is named in a list. A source file's *name* only and
/// a symbol's `module::fn_name` only ([`short_name`]); the whole of either is in
/// [`entry_tooltip`].
pub(crate) fn entry_text(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Symbol(_)) => short_name(&entry_name(entry)),
        entry => entry_name(entry),
    }
}

/// The whole of what a document is called: the demangled symbol name, the object's name,
/// or the source file's path. What a filter reads, so that a generic argument is still
/// something a reader can search for after the tab stopped drawing it.
pub(crate) fn entry_name(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Object(object)) | Document::Code(object) => {
            object.name.clone()
        }
        Document::Assembly(Selection::Symbol(symbol)) => symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone(),
        Document::Source(file) => source::name_of(Path::new(&**file)),
    }
}

/// What hovering a document's tab or row says: the whole path for a file and for an
/// object's code, whose name says nothing about where it came from; the whole name for
/// everything else -- which is where the rest of a shortened symbol name is.
pub(crate) fn entry_tooltip(entry: &Document) -> String {
    match entry {
        Document::Source(file) => file.to_string(),
        Document::Code(object) => object.path.display().to_string(),
        entry => entry_name(entry),
    }
}

/// Which kind of tab this is, as the one glyph that tells the three apart.
pub(crate) fn entry_icon(entry: &Document) -> Element {
    kind_icon(entry.kind())
}

/// **The one table of the three glyphs.** A place is drawn the same open or saved, so
/// [`entry_icon`] and a bookmark row both come here, and a fourth [`Kind`] cannot reach
/// one of them and miss the other.
pub(crate) fn kind_icon(kind: Kind) -> Element {
    glyph(match kind {
        Kind::Binary => ("binary", lucide::binary()),
        Kind::Source => ("file-code", lucide::file_code()),
        Kind::Code => ("scroll-text", lucide::scroll_text()),
    })
}

/// The identity of what a document points at, for keying the row or tab that names it.
/// The variant is part of the key so a pointer and a path cannot hash into one key for
/// two tabs of different kinds.
#[derive(Hash)]
pub(crate) enum EntryKey<'a> {
    Object(usize),
    Symbol(usize),
    Source(&'a str),
    Code(usize),
}

pub(crate) fn entry_key(entry: &Document) -> EntryKey<'_> {
    match entry {
        Document::Assembly(Selection::Object(object)) => {
            EntryKey::Object(Arc::as_ptr(object).addr())
        }
        Document::Assembly(Selection::Symbol(symbol)) => {
            EntryKey::Symbol(Arc::as_ptr(&symbol.data).addr())
        }
        Document::Source(file) => EntryKey::Source(file),
        Document::Code(object) => EntryKey::Code(Arc::as_ptr(object).addr()),
    }
}

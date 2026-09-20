//! What a document is called and drawn as wherever a list names one: a tab's chip, a
//! sidebar row, a bookmark, a place on a trail.
//!
//! Nothing here opens or closes anything. [`Names`] is the whole of what a place is called
//! -- the short spelling a row draws, the whole one a filter reads, and what hovering says
//! -- built in one pass, and [`entry_key`] is the identity a row or a chip is keyed by.

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
        Place::Code(object, address) => {
            match object.symbol_at_placed(PlacedAddress::new(address)) {
                Some(symbol) => short_name(symbol.display()),
                None => Names::of(&stop.document).text,
            }
        }
        Place::Source(line) => format!("{}:{line}", Names::of(&stop.document).text),
        Place::Whole | Place::Instruction(_) => Names::of(&stop.document).text,
    }
}

/// What a document is called, every way a list or a chip says it.
///
/// **One value and not a function each.** A symbol's three spellings are one demangled
/// name -- a hundred and fifty characters on average -- cut two ways, so a caller wanting
/// both the drawn name and the whole one used to have to know which of five functions
/// built it once. Here every caller gets all three for the price of the longest.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Names {
    /// The short spelling a row or a chip draws: a symbol's `module::fn_name`
    /// ([`short_name`]), a source file's name without its path.
    pub(crate) text: String,
    /// The whole of it, which is what a filter reads -- so a generic argument no tab draws
    /// is still something a reader can search for.
    pub(crate) whole: String,
    /// What hovering says: the path for a source file and for an object's code, whose name
    /// says nothing about where it came from, and the whole name for everything else,
    /// which is where the rest of a shortened symbol name is.
    pub(crate) tooltip: String,
}

impl Names {
    /// What an open document is called: one arm per kind of place, and the symbol's
    /// demangled name built once.
    pub(crate) fn of(entry: &Document) -> Names {
        match entry {
            Document::Symbol(symbol) => {
                let whole = symbol.data.display().to_owned();
                Names {
                    text: short_name(&whole),
                    tooltip: whole.clone(),
                    whole,
                }
            }
            Document::Object(object) => Names::whole_of(object.name.clone(), object.name.clone()),
            Document::Code(object) => {
                Names::whole_of(object.name.clone(), object.path.display().to_string())
            }
            Document::Source(file) => {
                Names::whole_of(source::name_of(Path::new(&**file)), file.to_string())
            }
        }
    }

    /// The same for a **saved** place, which has the name it was bookmarked under and no
    /// object behind it. Drawn from the bookmark whether or not the place resolves, so a
    /// row does not change its spelling when its binary is closed.
    ///
    /// Beside [`Names::of`] and not in the Bookmarks view, because it is the same three
    /// rules over the saved spelling of the same four kinds: apart, the two drifted.
    pub(crate) fn of_saved(bookmark: &Bookmark) -> Names {
        let label = bookmark.label().into_owned();
        match &bookmark.document {
            SavedDocument::Symbol { .. } => Names {
                text: short_name(&label),
                tooltip: label.clone(),
                whole: label,
            },
            SavedDocument::Source { path } => Names::whole_of(label, path.clone()),
            SavedDocument::Object {
                path,
                shown: SavedShown::Code,
                ..
            } => Names::whole_of(label, path.display().to_string()),
            SavedDocument::Object { .. } => Names::whole_of(label.clone(), label),
        }
    }

    /// A name a page is called by, which is one word either way.
    pub(crate) fn page(title: &str) -> Names {
        Names::whole_of(title.to_owned(), title.to_owned())
    }

    /// A name that is not cut down: the one string is both what is drawn and what a filter
    /// reads, with whatever hovering says beside it.
    fn whole_of(name: String, tooltip: String) -> Names {
        Names {
            text: name.clone(),
            whole: name,
            tooltip,
        }
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
        Document::Object(object) => EntryKey::Object(Arc::as_ptr(object).addr()),
        Document::Symbol(symbol) => EntryKey::Symbol(Arc::as_ptr(&symbol.data).addr()),
        Document::Source(file) => EntryKey::Source(file),
        Document::Code(object) => EntryKey::Code(Arc::as_ptr(object).addr()),
    }
}

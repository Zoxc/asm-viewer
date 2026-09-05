//! The names the app gives code the file names nothing: an image's entry point, a function
//! only an unwind entry declares, and a fragment of one. This is where each is spelled.

use crate::unwind::UnwindEntry;
use std::fmt;

/// A name the app made up, its [`Display`](fmt::Display) the one place the spelling lives.
///
/// The angle brackets are the point: no assembler, linker or mangling scheme produces them,
/// so none of these can collide with a name that was in the file. The address is in the
/// name because it is all that tells one from the next — 20 000 `<function 0x…>`s in one
/// Symbols list have to be told apart and found.
///
/// **The spellings are persisted.** A bookmark and a saved place hold their symbol's name
/// as a string in `project.toml`, and once the binary has been rebuilt that string is all a
/// bookmark resolves by (the app's `SavedDocument::resolve_by_name`). A changed spelling
/// drops a reader's bookmarks without a word, so `tests.rs` pins the three exactly.
#[derive(Clone, Copy)]
pub(crate) enum MadeUp {
    /// The image's entry point, which is an address and no name.
    EntryPoint,
    /// A function at an address: code only an unwind entry declares, or a text symbol whose
    /// own name will not read out of the string table.
    Function(u64),
    /// A second range of some function's rather than a function: an unwind entry whose
    /// unwind info is chained.
    Fragment(u64),
}

impl MadeUp {
    /// What to call the code an unwind entry declares.
    pub(crate) fn unwind(entry: &UnwindEntry) -> MadeUp {
        let address = entry.range.start;
        if entry.chained {
            MadeUp::Fragment(address)
        } else {
            MadeUp::Function(address)
        }
    }

    /// The name, with the `mangled` flag it is kept under: `false`, for every one of these.
    /// A made-up name is not the file's own, and no demangler has anything to say about it.
    pub(crate) fn unmangled(self) -> (String, bool) {
        (self.to_string(), false)
    }
}

impl fmt::Display for MadeUp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MadeUp::EntryPoint => f.write_str("<entry point>"),
            MadeUp::Function(address) => write!(f, "<function {address:#x}>"),
            MadeUp::Fragment(address) => write!(f, "<fragment {address:#x}>"),
        }
    }
}

#[cfg(test)]
mod tests;

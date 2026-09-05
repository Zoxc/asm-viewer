//! The names the app gives code the file names nothing: an image's entry point, a function
//! only an unwind entry declares, and a fragment of one. This is where each is spelled.

use crate::unwind::UnwindEntry;
use std::fmt;

/// A name the app made up, its [`Display`](fmt::Display) the one place the spelling lives.
/// [`MadeUp::of`] reads one back.
///
/// The angle brackets are the point: no assembler, linker or mangling scheme produces them,
/// so none of these can collide with a name that was in the file. The address is in the
/// name because it is all that tells one from the next — 20 000 `<function 0x…>`s in one
/// Symbols list have to be told apart and found.
///
/// **A spelling is never saved.** The app writes one of these to `project.toml` as which
/// name it is and the symbol's address, and renders it again on the way back (the app's
/// `SavedName`), so a bookmark on a made-up name outlives a decision to spell it some other
/// way. `tests.rs` pins today's three all the same: they are what a reader reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadeUp {
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
    /// Which of these `name` is for a symbol at `address`, or [`None`] where the name is the
    /// file's own. The candidates are rendered and compared, so [`Display`](fmt::Display)
    /// stays the one place the spelling lives; and it is the address that decides, so a name
    /// out of a file that reads like one of these but sits somewhere else is not taken for
    /// one.
    pub fn of(name: &str, address: u64) -> Option<MadeUp> {
        [
            MadeUp::EntryPoint,
            MadeUp::Function(address),
            MadeUp::Fragment(address),
        ]
        .into_iter()
        .find(|made_up| made_up.to_string() == name)
    }

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

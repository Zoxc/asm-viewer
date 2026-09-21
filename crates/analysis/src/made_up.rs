//! The names the app gives code the file names nothing: an image's entry point, a function
//! only an unwind entry declares, and a fragment of one. This is where each is spelled.

use crate::unwind::UnwindEntry;
use crate::SectionAddress;
use std::fmt;

/// A name the app made up, its [`Display`](fmt::Display) the one place the spelling lives.
/// A symbol so named keeps which one it is ([`SymbolData::made_up`](crate::SymbolData::made_up)).
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
    Function(SectionAddress),
    /// A second range of some function's rather than a function: an unwind entry whose
    /// unwind info is chained.
    Fragment(SectionAddress),
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

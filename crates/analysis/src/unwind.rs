//! The unwind tables a linked image states its functions' bounds in, read for what they
//! declare: an x86-64 PE's exception directory (`.pdata`), one `RUNTIME_FUNCTION` per
//! function with unwind info. Each entry states **both ends** of a function — the loader's
//! word, not a debugger's — which is what makes it worth reading past the export table, and
//! why reading it keeps the "nothing is scanned for" rule. What the entries become — a
//! symbol where nothing else names the address, and the stated end as the extent — is
//! `declared_code`'s and `SymbolData::extent`'s business in `lib.rs`; this module only reads.

use object::{read::pe::PeFile64, Architecture, Object as _};
use std::ops::Range;

/// The entries the file's unwind table states, in file order, placed on the image base and
/// not yet placed in any section — that is `declared_code`'s lookup, which is also what
/// drops one whose begin is not in code. Empty for a file with no table this reads.
pub(crate) fn entries(file: &object::File<'_>) -> Vec<UnwindEntry> {
    match file {
        object::File::Pe64(pe) => self::pe(pe),
        _ => Vec::new(),
    }
}

/// One `RUNTIME_FUNCTION` of an x86-64 PE's exception directory, out of
/// [`entries`]: the range it states, and whether its `UNWIND_INFO` is **chained**
/// (`UNW_FLAG_CHAININFO`) — a second range of a function that has a primary entry elsewhere,
/// a cold part or the piece after a mid-body stack adjustment, which Microsoft calls a
/// *function fragment* — rather than a function's own.
pub(crate) struct UnwindEntry {
    pub(crate) range: Range<u64>,
    pub(crate) chained: bool,
}

/// The entries an x86-64 PE's exception directory states: one `RUNTIME_FUNCTION` per function
/// with unwind info, its begin and end RVAs read and placed on the image base, and one byte
/// of the `UNWIND_INFO` its third word names, for the chained flag. Each is a **declaration
/// of both ends** of a function — the loader's, not a debugger's — which is what makes it
/// worth reading past the export table: a stripped image exports a handful of its functions,
/// and every function between two exports is otherwise nameless and of no known length. In
/// file order, an entry whose end is not past its begin dropped, and not yet placed in any
/// section — that is `declared_code`'s lookup, which is also what drops one whose begin is
/// not in code. An `UNWIND_INFO` that cannot be read, or is of a version other than the one
/// there is, makes its entry a plain function: the range is still stated.
///
/// x86-64 only: ARM64's `.pdata` record is another shape, 8 bytes, and a PE32 has none. A
/// COFF `.obj` carries a relocatable `.pdata` section and no data directory; it is not a
/// `Pe64` and so is skipped, which is `declared_code`'s rule for a relocatable object too.
fn pe(pe: &PeFile64<'_>) -> Vec<UnwindEntry> {
    if pe.architecture() != Architecture::X86_64 {
        return Vec::new();
    }
    let Some(directory) = pe
        .data_directories()
        .get(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
    else {
        return Vec::new();
    };
    let sections = pe.section_table();
    let Ok(data) = directory.data(pe.data(), &sections) else {
        return Vec::new();
    };

    let base = pe.relative_address_base();
    data.chunks_exact(12)
        .filter_map(|entry| {
            let word = |at: usize| entry[at..at + 4].try_into().ok().map(u32::from_le_bytes);
            let begin = base.checked_add(u64::from(word(0)?))?;
            let end = base.checked_add(u64::from(word(4)?))?;
            if begin >= end {
                return None;
            }
            // `UNWIND_INFO`'s first byte: the version in its low three bits, the flags
            // above them.
            let chained = sections
                .pe_data_at(pe.data(), word(8)?)
                .and_then(|info| info.first())
                .is_some_and(|&first| first & 7 == 1 && (first >> 3) & 4 != 0);
            Some(UnwindEntry {
                range: begin..end,
                chained,
            })
        })
        .collect()
}

//! The unwind tables a linked image states its functions' bounds in, read for what they
//! declare: an x86-64 PE's exception directory (`.pdata`), one `RUNTIME_FUNCTION` per
//! function with unwind info, and an ELF's `.eh_frame`, one FDE per function with any.
//! Each entry states **both ends** of a function — the loader's or the unwinder's word, not
//! a debugger's — which is what makes it worth reading past the export table, and why
//! reading it keeps the "nothing is scanned for" rule. What the entries become — a symbol
//! where nothing else names the address, and the stated end as the extent — is
//! `declared_code`'s business in `parse.rs` and `SymbolData::extent`'s in `extent.rs`; this
//! module only reads. It is the one part of the crate that reads call-frame information;
//! `line/dwarf.rs` is still the only one that knows DWARF's debug sections and `addr2line`.

use crate::sections::runtime_endian;
use crate::SectionAddress;
use gimli::{BaseAddresses, CieOrFde, EhFrame, EhFrameOffset, UnwindSection as _};
use object::pe::ImageRuntimeFunctionEntry;
use object::{
    read::pe::PeFile64, Architecture, LittleEndian, Object as _, ObjectKind, ObjectSection as _,
};
use std::{collections::HashMap, ops::Range};

/// The entries the file's unwind table states, in file order, placed on the image base and
/// not yet placed in any section — that is `declared_code`'s lookup, which is also what
/// drops one whose begin is not in code. Empty for a file with no table this reads: a
/// relocatable object among them, whose `.eh_frame` is written before its addresses are —
/// see [`elf`].
pub(crate) fn entries(file: &object::File<'_>) -> Vec<UnwindEntry> {
    match file {
        object::File::Pe64(pe) => self::pe(pe),
        object::File::Elf32(_) | object::File::Elf64(_)
            if file.kind() != ObjectKind::Relocatable =>
        {
            elf(file)
        }
        _ => Vec::new(),
    }
}

/// The FDEs an ELF's `.eh_frame` states, each a function's start and length, on any
/// architecture: the format is DWARF's call-frame information, the same everywhere, and on
/// x86-64 every function has one by default (`-fasynchronous-unwind-tables`), leaves
/// included, which is more than `.pdata` covers. No fragment flag exists here, so a `.cold`
/// part a stripped image no longer names is a function of its own; `chained` is never set.
///
/// **Linked images only.** A relocatable object's `.eh_frame` is written before its
/// addresses are: the FDEs' address fields are zero with a `R_X86_64_PC32` each, and read
/// as they lie they decode to ranges that happen to fall inside `.text` — in the committed
/// `line_fixture.o`, `0x20..0x34` for a function at 0 — which would hand `sum_to` at 0x30
/// an extent of 4 instead of 62. `declared_code` refuses a relocatable object anyway; the
/// ranges reaching `CodeSection::unwind` is what this gate is for.
///
/// Read once, front to back: `.eh_frame_hdr` is the unwinder's lookup table over the same
/// records and says nothing more, and `.debug_frame` is the same format in an object built
/// without unwind tables, whose extents DWARF's own `DW_AT_high_pc` already gives. The walk
/// ends at the section's end, at the zero-length terminator, or at the first record that
/// will not parse — a bad record's length is exactly what cannot be trusted to find the
/// next — keeping what was read; an FDE whose own parse fails is skipped. Every CIE comes
/// before the FDEs that use it, so they are kept as they go by and re-read only on a miss.
fn elf(file: &object::File<'_>) -> Vec<UnwindEntry> {
    let Some(section) = file.section_by_name(".eh_frame") else {
        return Vec::new();
    };
    let Ok(data) = section.data() else {
        return Vec::new();
    };

    let mut eh_frame = EhFrame::new(data, runtime_endian(file));
    if let Some(size) = file.architecture().address_size() {
        eh_frame.set_address_size(size.bytes());
    }

    // Where the pc-relative pointers are relative to; `.text` and `.got` for the rarer
    // text- and data-relative ones, which a CIE's personality pointer can be, and a CIE
    // that cannot be read ends the walk.
    let mut bases = BaseAddresses::default().set_eh_frame(section.address());
    if let Some(text) = file.section_by_name(".text") {
        bases = bases.set_text(text.address());
    }
    if let Some(got) = file.section_by_name(".got") {
        bases = bases.set_got(got.address());
    }

    let mut cies: HashMap<EhFrameOffset<usize>, gimli::CommonInformationEntry<_>> = HashMap::new();
    let mut entries = Vec::new();
    let mut records = eh_frame.entries(&bases);
    while let Ok(Some(record)) = records.next() {
        let partial = match record {
            CieOrFde::Cie(cie) => {
                cies.insert(EhFrameOffset(cie.offset()), cie);
                continue;
            }
            CieOrFde::Fde(partial) => partial,
        };
        let fde = partial.parse(|section, bases, offset| match cies.get(&offset) {
            Some(cie) => Ok(cie.clone()),
            None => section.cie_from_offset(bases, offset),
        });
        let Ok(fde) = fde else {
            continue;
        };
        let begin = SectionAddress::new(fde.initial_address());
        let Some(end) = begin.checked_add(fde.len()) else {
            continue;
        };
        if fde.len() == 0 {
            continue;
        }
        entries.push(UnwindEntry {
            range: begin..end,
            chained: false,
        });
    }
    entries
}

/// One `RUNTIME_FUNCTION` of an x86-64 PE's exception directory, out of
/// [`entries`]: the range it states, and whether its `UNWIND_INFO` is **chained**
/// (`UNW_FLAG_CHAININFO`) — a second range of a function that has a primary entry elsewhere,
/// a cold part or the piece after a mid-body stack adjustment, which Microsoft calls a
/// *function fragment* — rather than a function's own.
pub(crate) struct UnwindEntry {
    pub(crate) range: Range<SectionAddress>,
    pub(crate) chained: bool,
}

impl UnwindEntry {
    /// How many bytes of code the entry declares.
    ///
    /// The subtraction cannot underflow today: both readers in this module drop an entry
    /// whose end is not past its begin — [`pe`] by `begin >= end`, [`elf`] by skipping
    /// a zero-length FDE. That rule is theirs, so it saturates here beside them rather than
    /// have a caller elsewhere rest on it unsaid.
    pub(crate) fn len(&self) -> u64 {
        self.range.start.bytes_to_saturating(self.range.end)
    }
}

/// The entries an x86-64 PE's exception directory states: one `ImageRuntimeFunctionEntry` per
/// function with unwind info, its begin and end RVAs read and placed on the image base, and one
/// byte of the `UNWIND_INFO` its third field names, for the chained flag; where that field
/// is odd it names another entry instead, and the entry is chained. A trailing partial
/// record is dropped. Each is a **declaration of both ends** of a function — the loader's, not
/// a debugger's — which is what makes it worth reading past the export table: a stripped image
/// exports a handful of its functions, and every function between two exports is otherwise
/// nameless and of no known length. In
/// file order, an entry whose end is not past its begin dropped, and not yet placed in any
/// section — that is `declared_code`'s lookup, which is also what drops one whose begin is
/// not in code. An `UNWIND_INFO` that cannot be read, or is of a version other than 1 or 2,
/// makes its entry a plain function: the range is still stated.
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

    let count = data.len() / size_of::<ImageRuntimeFunctionEntry>();
    let Ok((entries, _)) = object::slice_from_bytes::<ImageRuntimeFunctionEntry>(data, count)
    else {
        return Vec::new();
    };

    let base = SectionAddress::new(pe.relative_address_base());
    entries
        .iter()
        .filter_map(|entry| {
            let begin = base.checked_add(u64::from(entry.begin_address.get(LittleEndian)))?;
            let end = base.checked_add(u64::from(entry.end_address.get(LittleEndian)))?;
            if begin >= end {
                return None;
            }
            // An odd RVA (`RUNTIME_FUNCTION_INDIRECT`) names another `RUNTIME_FUNCTION`,
            // whose unwind info this range shares, so it is chained without a byte read.
            // Otherwise it names an `UNWIND_INFO`, whose first byte holds the version in
            // its low three bits and the flags above them. Versions 1 and 2 share that
            // header; 2 only adds epilog codes.
            let rva = entry.unwind_info_address_or_data.get(LittleEndian);
            let chained = rva & 1 != 0
                || sections
                    .pe_data_at(pe.data(), rva)
                    .and_then(|info| info.first())
                    .is_some_and(|&first| matches!(first & 7, 1 | 2) && (first >> 3) & 4 != 0);
            Some(UnwindEntry {
                range: begin..end,
                chained,
            })
        })
        .collect()
}

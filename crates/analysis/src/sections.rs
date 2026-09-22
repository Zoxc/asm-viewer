//! Rules about an object's sections that more than one reader follows: where each code
//! section is placed and how a section's bytes are read with a believable size, which the
//! parse and the DWARF loader share, and the byte order `gimli` reads them in, which the DWARF
//! loader and the unwind reader share.

use crate::Bias;
use object::{
    CompressionFormat, Object as _, ObjectKind, ObjectSection, SectionIndex, SectionKind,
};
use std::collections::HashMap;

/// The file's byte order as `gimli` takes it.
pub(crate) fn runtime_endian(file: &object::File<'_>) -> gimli::RunTimeEndian {
    if file.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    }
}

/// Where each code section is placed in the one address space the object's line info is read
/// in and its code is listed in; what [`CodeSection::bias`](crate::model::CodeSection::bias) is
/// set from.
///
/// **An address alone is not a key in a relocatable object.** Sections there have no address
/// until linked and rustc emits one `.text.<name>` per function, so every function lands on 0
/// and the line programs pile up. This does what a linker does and gives each code section a
/// place of its own, as long as the bytes it decompresses to: a bias, added to every address
/// relocated against that section (`line::relocate`) and subtracted again from every row a
/// query returns.
///
/// **A bias is never a wrapped value.** The layout starts above the highest address the file
/// states, so a section is placed at or above where the file put it: a query can add a bias
/// with checked arithmetic and mean what `line::relocate`'s wrapping add means.
///
/// Two limits, both load-bearing:
///
/// * **Relocatable objects only.** A linked image holds real addresses literally rather than
///   through relocations; moving the few that are relocated would move them away from the
///   rest.
/// * **Code sections only.** An absolute relocation in a debug section is often an offset
///   into another `.debug_*` section (`DW_AT_stmt_list`, `DW_FORM_strp`), which must come out
///   exactly as it went in.
pub(crate) fn section_biases(file: &object::File<'_>) -> HashMap<SectionIndex, Bias> {
    let mut biases = HashMap::new();
    if file.kind() != ObjectKind::Relocatable {
        return biases;
    }

    let text = || {
        file.sections()
            .filter(|section| section.kind() == SectionKind::Text)
    };

    // Everything is placed at or above the highest address the file states, so a section is
    // never moved *down* and a bias is never a wrapped subtraction. Nothing moves for the
    // usual relocatable object, whose text sections all state 0; a Mach-O `.o` lays its
    // sections out with addresses of their own and does state more.
    let mut next: u64 = text().map(|section| section.address()).max().unwrap_or(0);

    for section in text() {
        // `next` starts at or above every text address and only grows, so this is the plain
        // difference. `wrapping_sub` and not `-` so that a proof going wrong is not a panic.
        biases.insert(
            section.index(),
            Bias::new(next.wrapping_sub(section.address())),
        );

        // Somewhere for the next section to go, past the bytes `section_data` keeps: for a
        // compressed section the size its header says it decompresses to, not the `size()` it
        // takes in the file. A zero-length section still takes an address of its own, so that
        // two of them are two places. An object whose sections do not fit in the address space
        // simply stops being biased past that point.
        // FIXME: warn the reader where the two sizes disagree -- a compressed loadable section,
        // which the ELF spec forbids.
        let length = match section.compressed_file_range() {
            Ok(range) if range.format != CompressionFormat::None => range.uncompressed_size,
            _ => section.size(),
        };
        let Some(end) = next.checked_add(length.max(1)) else {
            break;
        };
        let Some(aligned) = end.checked_next_multiple_of(SECTION_ALIGNMENT) else {
            break;
        };
        next = aligned;
    }

    biases
}

/// The bias of the section `index` names in a map [`section_biases`] made. A section with no
/// entry, or no section at all, was not moved: a linked image's map is empty.
pub(crate) fn bias_of(biases: &HashMap<SectionIndex, Bias>, index: Option<SectionIndex>) -> Bias {
    index
        .and_then(|index| biases.get(&index))
        .copied()
        .unwrap_or(Bias::NONE)
}

/// What [`section_biases`] rounds each section's placement up to. Nothing depends on the
/// value; the gap it leaves means an off-by-one cannot walk into the next section.
const SECTION_ALIGNMENT: u64 = 16;

/// A hard ceiling (1 GiB) on a single section's decompressed bytes, whatever its header
/// claims. See [`section_data`].
const MAX_SECTION_DATA: u64 = 1 << 30;

/// Read a section's bytes, decompressing it if it says it is compressed, but only after
/// checking that the size it declares is believable.
///
/// `uncompressed_data()` reserves the size in the compression header *before* it looks at a
/// compressed byte, so one flipped `SHF_COMPRESSED` bit turns into a multi-gigabyte
/// allocation and an OOM abort. `compressed_data()` gives the same information without
/// allocating. Two bounds have to hold, and a section failing either is dropped exactly like
/// one whose data will not read: a ratio bound (DEFLATE cannot expand by more than 1032:1
/// nor a zstd frame by more than 32768:1, so a larger declared size is a lie about *these*
/// bytes), and an absolute one, since the ratio bound still scales with the input. The
/// declared size then bounds what zlib produces on its own, `decompress()` inflating it into
/// a vector it never grows; what zstd produces is bounded by [`zstd_data`] instead.
pub(crate) fn section_data<'data, S: ObjectSection<'data>>(section: &S) -> Option<Vec<u8>> {
    let compressed = section.compressed_data().ok()?;

    let max_ratio: u64 = match compressed.format {
        // Not compressed at all: the bytes are already there, nothing to bound.
        CompressionFormat::None => return Some(compressed.data.to_vec()),
        CompressionFormat::Zlib => 1032,
        CompressionFormat::Zstandard => 32768,
        // Any other format is one `decompress()` does not implement; it would fail.
        _ => return None,
    };

    let ratio_bound = (compressed.data.len() as u64).saturating_mul(max_ratio);
    if compressed.uncompressed_size > ratio_bound.min(MAX_SECTION_DATA) {
        return None;
    }

    if compressed.format == CompressionFormat::Zstandard {
        return zstd_data(compressed.data, compressed.uncompressed_size);
    }

    Some(compressed.decompress().ok()?.into_owned())
}

/// A zstd section inflated here rather than by `decompress()`, which takes the declared size
/// as a hint only: it reserves that much and then reads the frame to its end, whatever that
/// produces, so the bounds above bound nothing (`notes/upstream/object.md`). The read stops
/// one byte past `size`, and a frame producing any other number of bytes is a lie about
/// these ones and is dropped like a declared size the ratio bound rejects.
fn zstd_data(data: &[u8], size: u64) -> Option<Vec<u8>> {
    use std::io::Read as _;

    let capacity: usize = size.try_into().ok()?;
    let mut out = Vec::with_capacity(capacity);
    let decoder = ruzstd::decoding::StreamingDecoder::new(data).ok()?;
    decoder
        .take(size.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 == size).then_some(out)
}

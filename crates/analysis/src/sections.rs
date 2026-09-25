//! Rules about an object's sections that more than one reader follows: where each code
//! section is placed and how a section's bytes are read with a believable size, which the
//! parse and the DWARF loader share, and the byte order `gimli` reads them in, which the DWARF
//! loader and the unwind reader share.

use crate::made_up::UnnamedSection;
use crate::{Bias, LoadMessage};
use object::{
    CompressedData, CompressionFormat, Object as _, ObjectKind, ObjectSection, SectionIndex,
    SectionKind,
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
/// **Running out of address space is reported, not worked around.** A section stating an
/// address near the top leaves no room after it, so the sections not placed by then stay
/// where the file put them, and [`Placement::message`] says so.
///
/// Two limits, both load-bearing:
///
/// * **Relocatable objects only.** A linked image holds real addresses literally rather than
///   through relocations; moving the few that are relocated would move them away from the
///   rest.
/// * **Code sections only.** An absolute relocation in a debug section is often an offset
///   into another `.debug_*` section (`DW_AT_stmt_list`, `DW_FORM_strp`), which must come out
///   exactly as it went in.
pub(crate) fn section_biases(file: &object::File<'_>) -> Placement {
    let mut placement = Placement {
        biases: HashMap::new(),
        message: None,
    };
    if file.kind() != ObjectKind::Relocatable {
        return placement;
    }

    let text = || {
        file.sections()
            .filter(|section| section.kind() == SectionKind::Text)
    };

    // Everything is placed at or above the highest address the file states, so a section is
    // never moved *down* and a bias is never a wrapped subtraction. Nothing moves for the
    // usual relocatable object, whose text sections all state 0; a Mach-O `.o` lays its
    // sections out with addresses of their own and does state more.
    let highest = text().max_by_key(|section| section.address());
    // Where the next section goes: `None` once the last slot's round-up ran past the end of
    // the address space.
    let mut next = Some(highest.as_ref().map_or(0, |section| section.address()));

    for section in text() {
        // How long a slot is: the bytes `section_data` keeps, which for a compressed section
        // is the size its header says it decompresses to, not the `size()` it takes in the
        // file. A section `section_data` drops takes no more room than an empty one, and a
        // zero-length section still takes an address of its own, so that two of them are two
        // places. Each slot is then at most `MAX_SECTION_DATA`, so the layout runs out of
        // address space only for a file stating an address near the top of it.
        // FIXME: warn the reader where the two sizes disagree -- a compressed loadable section,
        // which the ELF spec forbids.
        let length = section
            .compressed_data()
            .ok()
            .and_then(|compressed| kept_size(&compressed))
            .unwrap_or(0);
        let slot = next.and_then(|start| Some((start, start.checked_add(length.max(1))?)));
        let Some((start, end)) = slot else {
            // No room left. This section and the ones after it stay where the file put them,
            // on top of each other, and the reader is told so rather than the layout moved.
            placement.message = highest.as_ref().map(out_of_room);
            break;
        };

        // `start` is at or above every text address, so this is the plain difference.
        // `wrapping_sub` and not `-` so that a proof going wrong is not a panic.
        placement.biases.insert(
            section.index(),
            Bias::new(start.wrapping_sub(section.address())),
        );
        next = end.checked_next_multiple_of(SECTION_ALIGNMENT);
    }

    placement
}

/// What [`section_biases`] worked out: each code section's bias, and what went wrong.
pub(crate) struct Placement {
    pub(crate) biases: HashMap<SectionIndex, Bias>,
    /// Said when the layout ran out of address space, which leaves some sections unplaced.
    pub(crate) message: Option<LoadMessage>,
}

/// The error for a layout that ran out of address space. Only a stated address near the top
/// can do that (see [`section_biases`]), so `highest`, the section stating the highest, is
/// the one named.
fn out_of_room(highest: &object::Section<'_, '_>) -> LoadMessage {
    LoadMessage::CodeSectionsOverlap {
        section: section_name(highest).unwrap_or_else(|made_up| made_up),
        address: highest.address(),
    }
}

/// What `section` is called: the file's own name, or, as the `Err`, the one made up for it
/// where that will not read ([`UnnamedSection`]).
pub(crate) fn section_name(section: &object::Section<'_, '_>) -> Result<String, String> {
    match section.name_bytes() {
        Ok(name) => Ok(String::from_utf8_lossy(name).into_owned()),
        Err(_) => Err(UnnamedSection(section.index()).to_string()),
    }
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
    let size = kept_size(&compressed)?;

    match compressed.format {
        CompressionFormat::None => Some(compressed.data.to_vec()),
        CompressionFormat::Zstandard => zstd_data(compressed.data, size),
        _ => Some(compressed.decompress().ok()?.into_owned()),
    }
}

/// How many bytes [`section_data`] keeps of a section, worked out without reading them, or
/// `None` for a section it drops. [`section_biases`] sizes each section's slot by this, so
/// the two cannot disagree.
fn kept_size(compressed: &CompressedData<'_>) -> Option<u64> {
    let max_ratio: u64 = match compressed.format {
        // Not compressed at all: the bytes are already there, nothing to bound.
        CompressionFormat::None => return Some(compressed.data.len() as u64),
        CompressionFormat::Zlib => 1032,
        CompressionFormat::Zstandard => 32768,
        // Any other format is one `decompress()` does not implement; it would fail.
        _ => return None,
    };

    let ratio_bound = (compressed.data.len() as u64).saturating_mul(max_ratio);
    (compressed.uncompressed_size <= ratio_bound.min(MAX_SECTION_DATA))
        .then_some(compressed.uncompressed_size)
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

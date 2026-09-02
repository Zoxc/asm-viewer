//! DWARF line-number information, read lazily out of the bytes an [`Object`] was parsed
//! from. Nothing here runs at parse time; the first query builds the context, and an object
//! with no DWARF caches that answer too.
//!
//! The readers are [`gimli::EndianArcSlice`] — an `Arc<[u8]>` per DWARF section — rather
//! than borrows of [`ObjectData`](crate::ObjectData), so the context owns its data and
//! [`Object`] does not become self-referential. Those `Arc`s are separate allocations: a
//! section may be compressed, and every one is relocated in place (see [`relocate`]).
//!
//! This file is the forward direction — an address range in, source rows out. The reverse —
//! a file and a line, out to the symbols compiled from them — is [`source`], a file of its
//! own because it is a whole-object index rather than a query, sharing this one's [`Dwarf`],
//! its biases and its guard.

use crate::{section_data, Object, Section, SymbolData};
use gimli::{EndianArcSlice, RunTimeEndian};
use object::{
    Object as _, ObjectKind, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget,
    SectionIndex, SectionKind,
};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

mod source;

use source::SourceIndex;

type Reader = EndianArcSlice<RunTimeEndian>;

/// An [`Object`]'s DWARF, or the fact that it has none, worked out at most once. Caching the
/// *absence* is what keeps a stripped binary from re-scanning its section table per query.
#[derive(Default)]
pub struct DwarfCache(OnceLock<Option<Dwarf>>);

/// One object's DWARF, parsed once and kept for the object's lifetime.
pub(crate) struct Dwarf {
    /// The [`Mutex`] is about `Sync`, not contention: `addr2line::Context` caches each
    /// unit's parsed line program in an `UnsafeCell` behind `&self`, so it is `Send` but not
    /// `Sync`, and [`Object`] is shared across threads as an `Arc`.
    context: Mutex<addr2line::Context<Reader>>,

    /// Where each code section was placed in the address space the context reads in. See
    /// [`section_biases`]; empty for a linked image, which needs none.
    biases: HashMap<SectionIndex, u64>,

    /// Every compilation unit that has been asked about, and the extent of each
    /// `DW_TAG_subprogram` in it, keyed by the unit's `.debug_info` offset and then by the
    /// subprogram's `DW_AT_low_pc`. Both keys are in the biased address space.
    extents: Mutex<HashMap<u64, HashMap<u64, u64>>>,

    /// The line info inverted — file and line to the symbols compiled from it — built whole
    /// on the first source question and never before one. A `OnceLock` and not a `Mutex`
    /// like the two above, because unlike them it is not filled in a unit at a time: see
    /// [`source`].
    index: OnceLock<SourceIndex>,
}

impl Dwarf {
    /// Build the context for one object file, or [`None`] when it has no DWARF. Never an
    /// error: foreign debug info and corrupt debug info are both simply "no line info".
    pub(crate) fn load(data: &crate::ObjectData) -> Option<Dwarf> {
        without_panicking(|| Dwarf::load_inner(data)).flatten()
    }

    fn load_inner(data: &crate::ObjectData) -> Option<Dwarf> {
        let file = object::File::parse(data.bytes()).ok()?;

        // The cheap test first, so an object with no DWARF costs one section-table scan.
        // `section_by_name` already knows ELF's `.zdebug_info` and Mach-O's `__debug_info`.
        file.section_by_name(gimli::SectionId::DebugInfo.name())?;

        let endian = if file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let biases = section_biases(&file);

        let dwarf =
            gimli::Dwarf::load::<_, ()>(|id| Ok(load_section(&file, id, endian, &biases, &[])))
                .ok()?;

        // Read once more, without the range lists that were left behind by the bias. Rare
        // enough — nothing in the tree emits the shape — that reading twice is cheaper than
        // holding the section data aside for a patch that almost never comes.
        let stale = stale_range_lists(&file, &biases, &dwarf);
        let dwarf = if stale.is_empty() {
            dwarf
        } else {
            gimli::Dwarf::load::<_, ()>(|id| Ok(load_section(&file, id, endian, &biases, &stale)))
                .ok()?
        };

        Some(Dwarf {
            context: Mutex::new(addr2line::Context::from_dwarf(dwarf).ok()?),
            biases,
            extents: Mutex::default(),
            index: OnceLock::new(),
        })
    }

    /// How far the section with this index was moved by [`section_biases`]; 0 for a section
    /// that was not moved, and for every section of a linked image.
    fn bias(&self, section: SectionIndex) -> u64 {
        self.biases.get(&section).copied().unwrap_or(0)
    }

    /// Resolve a whole address range in one pass. `bias` is what the range's section was
    /// moved by, so the query and the rows it produces are translated in and out of the
    /// address space the context was built in.
    fn line_info(&self, bias: u64, range: Range<u64>) -> Option<Arc<LineInfo>> {
        without_panicking(|| self.line_info_inner(bias, range)).flatten()
    }

    fn line_info_inner(&self, bias: u64, range: Range<u64>) -> Option<Arc<LineInfo>> {
        // A poisoned lock means a previous query panicked. Nothing here is left half-written
        // by one (the context is only ever read), so recover rather than propagate.
        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());

        // Saturating rather than wrapping, so an absurd range asks about less than it meant
        // to instead of about something else.
        let query = range.start.saturating_add(bias)..range.end.saturating_add(bias);

        let mut files: Vec<Arc<str>> = Vec::new();
        let mut file_indices: HashMap<Arc<str>, usize> = HashMap::new();
        let mut rows: Vec<LineRow> = Vec::new();

        // One call for the symbol's whole extent: this walks each covering unit's line table
        // once, where per-instruction lookups would binary-search it per address.
        for (address, length, location) in
            context.find_location_range(query.start, query.end).ok()?
        {
            let Some(end) = address.checked_add(length) else {
                continue;
            };
            // addr2line hands back the row *containing* the start of the query, which may
            // begin before it, and clips nothing at the top either.
            let start = address.max(query.start).wrapping_sub(bias);
            let end = end.min(query.end).wrapping_sub(bias);
            if start >= end {
                continue;
            }

            let file = location.file.map(|file| match file_indices.get(file) {
                Some(index) => *index,
                None => {
                    let file: Arc<str> = Arc::from(file);
                    files.push(file.clone());
                    file_indices.insert(file, files.len() - 1);
                    files.len() - 1
                }
            });

            rows.push(LineRow {
                range: start..end,
                file,
                line: location.line,
                column: location.column,
            });
        }

        drop(context);

        // Units are visited in range order and rows within a unit ascend, but two units may
        // cover overlapping addresses, so sort rather than assume.
        rows.sort_by_key(|row| (row.range.start, row.range.end));

        // Then clip so the rows genuinely do not overlap, which [`LineInfo::row_at`] needs to
        // binary-search them: it looks for the last row starting at or before an address, and
        // a row nested inside a longer one makes that answer arbitrary. The row that starts
        // first keeps the addresses it covers, and one left with nothing goes.
        let mut covered = 0;
        rows.retain_mut(|row| {
            row.range.start = row.range.start.max(covered);
            if row.range.start >= row.range.end {
                return false;
            }
            covered = row.range.end;
            true
        });

        // Coalesce runs that say the same thing: a line program emits a row per
        // is_stmt/discriminator change as well as per source position.
        rows.dedup_by(|next, row| {
            let same = row.range.end == next.range.start
                && row.file == next.file
                && row.line == next.line
                && row.column == next.column;
            if same {
                row.range.end = next.range.end;
            }
            same
        });

        // "There is DWARF but it says nothing about this symbol" and "there is no DWARF" are
        // the same answer to a caller.
        (!rows.is_empty()).then(|| Arc::new(LineInfo { rows, files }))
    }

    /// The extent of the `DW_TAG_subprogram` beginning at `address`, or [`None`] when no unit
    /// covers the address or the subprogram that does begins elsewhere.
    fn extent(&self, bias: u64, address: u64) -> Option<u64> {
        without_panicking(|| self.extent_inner(bias, address)).flatten()
    }

    fn extent_inner(&self, bias: u64, address: u64) -> Option<u64> {
        let probe = address.checked_add(bias)?;
        // `addr2line` 0.21's `Context::find_units` asks its range index about `probe + 1`
        // with a plain addition, so the very last address in the space panics. Declined here
        // rather than left to `without_panicking`: this one is ours to see coming.
        if probe == u64::MAX {
            return None;
        }

        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());

        // `skip_all_loads` declines to fetch split DWARF, which this crate does not read
        // anywhere else either.
        let (sections, unit) = context.find_dwarf_and_unit(probe).skip_all_loads()?;
        let key = unit.header.offset().as_debug_info_offset()?.0 as u64;

        // Nested under the context's lock, and only ever in that order — this is the one
        // place either is taken.
        let mut extents = self.extents.lock().unwrap_or_else(|e| e.into_inner());
        let extents = extents
            .entry(key)
            .or_insert_with(|| subprogram_extents(sections, unit));

        extents.get(&probe).copied()
    }
}

/// Every `DW_TAG_subprogram` in one unit that states where it begins and ends, as
/// `low_pc -> size`. A whole-unit walk rather than a search, because the answer is cached
/// per unit and a reader asks about symbol after symbol out of the same unit.
///
/// Skipped: a subprogram with no `DW_AT_low_pc` (a declaration, not code), one with
/// `DW_AT_ranges` instead of `DW_AT_high_pc` (discontiguous, so no single extent), and one
/// claiming zero bytes. `DW_AT_high_pc` is an *end address* when its form is an address and a
/// *length* when its form is a constant; both spellings are in the wild. Abstract origins are
/// not followed: only the DIE carrying `low_pc` knows where the bytes are.
fn subprogram_extents(
    sections: &gimli::Dwarf<Reader>,
    unit: &gimli::Unit<Reader>,
) -> HashMap<u64, u64> {
    let mut extents = HashMap::new();

    let mut entries = unit.entries();
    // A malformed unit stops the walk where it goes wrong rather than discarding what was
    // read before it.
    while let Ok(Some((_, entry))) = entries.next_dfs() {
        if entry.tag() != gimli::DW_TAG_subprogram {
            continue;
        }

        let low = match entry.attr_value(gimli::DW_AT_low_pc) {
            Ok(Some(value)) => match sections.attr_address(unit, value) {
                Ok(Some(low)) => low,
                _ => continue,
            },
            _ => continue,
        };

        let size = match entry.attr_value(gimli::DW_AT_high_pc) {
            Ok(Some(gimli::AttributeValue::Udata(length))) => length,
            Ok(Some(value)) => match sections.attr_address(unit, value) {
                Ok(Some(high)) => match high.checked_sub(low) {
                    Some(size) => size,
                    None => continue,
                },
                _ => continue,
            },
            _ => continue,
        };

        if size == 0 {
            continue;
        }

        // Two subprograms at one address is a concrete instance beside its abstract root, or
        // a unit included twice; the first one read keeps the address.
        extents.entry(low).or_insert(size);
    }

    extents
}

/// Run DWARF parsing with a net under it, turning a panic into "no line info".
///
/// Not general defensiveness: two known, reachable bugs in `addr2line` 0.21, both unchecked
/// arithmetic on numbers a `.debug_*` section states. A line-table row's length is
/// `next.address - row.address`, and nothing stops a line program from moving its address
/// backwards; and a range is `low_pc + high_pc` wherever `high_pc` is a length, which
/// overflows for a length running off the end of the address space — that one while the
/// context is being *built*, which is why the guard is around [`Dwarf::load`] too.
///
/// Sound because a panic leaves nothing half-written: the context is only ever read, and the
/// lock a panic poisons is recovered explicitly.
fn without_panicking<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Where each code section is placed in the address space the DWARF is read in.
///
/// **An address alone is not a key in a relocatable object.** Sections there have no address
/// until linked and rustc emits one `.text.<name>` per function, so every function lands on 0
/// and the line programs pile up. This does what a linker does and gives each code section a
/// place of its own: a bias, added to every address relocated against that section
/// ([`relocate`]) and subtracted again from every row a query returns.
///
/// Two limits, both load-bearing:
///
/// * **Relocatable objects only.** A linked image holds real addresses literally rather than
///   through relocations; moving the few that are relocated would move them away from the
///   rest.
/// * **Code sections only.** An absolute relocation in a debug section is often an offset
///   into another `.debug_*` section (`DW_AT_stmt_list`, `DW_FORM_strp`), which must come out
///   exactly as it went in.
fn section_biases(file: &object::File<'_>) -> HashMap<SectionIndex, u64> {
    let mut biases = HashMap::new();
    if file.kind() != ObjectKind::Relocatable {
        return biases;
    }

    let mut next: u64 = 0;
    for section in file.sections() {
        if section.kind() != SectionKind::Text {
            continue;
        }

        biases.insert(section.index(), next.wrapping_sub(section.address()));

        // Somewhere for the next section to go. A zero-length section still takes an address
        // of its own, so that two of them are two places. An object whose sections do not fit
        // in the address space simply stops being biased past that point.
        let Some(end) = next.checked_add(section.size().max(1)) else {
            break;
        };
        let Some(aligned) = end.checked_next_multiple_of(SECTION_ALIGNMENT) else {
            break;
        };
        next = aligned;
    }

    biases
}

/// What [`section_biases`] rounds each section's placement up to. Nothing depends on the
/// value; the gap it leaves means an off-by-one cannot walk into the next section.
const SECTION_ALIGNMENT: u64 = 16;

/// One unit range list that [`section_biases`] left behind, and the bytes that make it read
/// as a list of no ranges at all.
struct StaleRangeList {
    section: gimli::SectionId,
    offset: usize,
    /// A DWARF 4 list ends on a pair of zero addresses and a DWARF 5 one on a single
    /// `DW_RLE_end_of_list` byte, so this is what has to be written over the list's first
    /// entry for the whole list to end where it begins.
    length: usize,
}

/// Every unit whose `DW_AT_ranges` list did not move when the bias moved its code.
///
/// The bias moves exactly what [`relocate`] moves. A line program's `DW_LNE_set_address` is
/// always relocated in a relocatable object, so a sequence always follows its section; a
/// unit's range list usually does too, since DWARF 4 states a range as a pair of addresses and
/// DWARF 5 has `DW_RLE_start_length`. But neither obliges a producer to. A range can also be
/// two offsets from a base address — `DW_RLE_offset_pair`, and DWARF 4's equivalent — and a
/// unit is free to give that base as a `DW_AT_low_pc` of 0 it does not relocate. Those offsets
/// follow nothing. The unit is then left declaring a range its own code is no longer in, and
/// `addr2line` does not look inside a unit whose ranges miss the probe: a silent "no line
/// info" for every section the bias moved, rather than a wrong answer.
///
/// A list is judged by its **first entry**, which is where a relocation lands in every form
/// that states an address — `DW_RLE_base_address` included, so a list of offset pairs from a
/// relocated base is correctly left alone, which is the shape rustc emits for the ranges of an
/// inlined subroutine. Only a unit's own list is examined and only that list is dropped: the
/// lists a unit's children hold are read by nobody here and are not ours to rewrite.
///
/// One shape is declined rather than judged: DWARF 5's `DW_RLE_startx_*` state their addresses
/// as indices into `.debug_addr`, which is relocated like any other section, so a relocation
/// there means the ranges moved without any of them saying so.
fn stale_range_lists(
    file: &object::File<'_>,
    biases: &HashMap<SectionIndex, u64>,
    dwarf: &gimli::Dwarf<Reader>,
) -> Vec<StaleRangeList> {
    let mut stale = Vec::new();
    if !biases.values().any(|&bias| bias != 0) {
        return stale;
    }

    let relocations = |id: gimli::SectionId| match file.section_by_name(id.name()) {
        Some(section) => section
            .relocations()
            .filter(|(_, relocation)| relocation.kind() == RelocationKind::Absolute)
            .map(|(offset, _)| offset)
            .collect(),
        None => Vec::new(),
    };
    if !relocations(gimli::SectionId::DebugAddr).is_empty() {
        return stale;
    }
    let ranges = relocations(gimli::SectionId::DebugRanges);
    let rnglists = relocations(gimli::SectionId::DebugRngLists);

    let mut headers = dwarf.units();
    // A malformed unit stops the walk where it goes wrong, as everywhere else here.
    while let Ok(Some(header)) = headers.next() {
        let encoding = header.encoding();
        let Ok(abbreviations) = dwarf.abbreviations(&header) else {
            continue;
        };
        let mut entries = header.entries(&abbreviations);
        let Ok(Some((_, root))) = entries.next_dfs() else {
            continue;
        };
        // An index into the section's offset table (`DW_FORM_rnglistx`) is declined rather
        // than resolved: the table itself would have to be trusted to find the list.
        let offset = match root.attr_value(gimli::DW_AT_ranges) {
            Ok(Some(gimli::AttributeValue::SecOffset(offset))) => offset,
            Ok(Some(gimli::AttributeValue::RangeListsRef(offset))) => offset.0,
            _ => continue,
        };

        let address_size = usize::from(encoding.address_size);
        let (section, relocations, length) = if encoding.version >= 5 {
            (gimli::SectionId::DebugRngLists, &rnglists, 1)
        } else {
            (gimli::SectionId::DebugRanges, &ranges, 2 * address_size)
        };

        // The first entry, plus the opcode byte DWARF 5 puts in front of it. Reaching into
        // the second entry only makes the list look relocated when it partly is, which is
        // the answer that leaves it alone.
        let first = offset..offset.saturating_add(2 * address_size + 1);
        if relocations
            .iter()
            .any(|&relocation| first.contains(&(relocation as usize)))
        {
            continue;
        }

        stale.push(StaleRangeList {
            section,
            offset,
            length,
        });
    }

    stale
}

/// Read one DWARF section, decompressing and relocating it. A section that is missing or
/// unreadable becomes an empty reader, which is what `gimli` expects for "not present".
fn load_section(
    file: &object::File<'_>,
    id: gimli::SectionId,
    endian: RunTimeEndian,
    biases: &HashMap<SectionIndex, u64>,
    stale: &[StaleRangeList],
) -> Reader {
    let data = file
        .section_by_name(id.name())
        .and_then(|section| {
            // `section_data`'s guard: a header that lies about how much it decompresses to is
            // dropped, not believed.
            let mut data = section_data(&section)?;
            relocate(&mut data, file, &section, endian, biases);

            // A stale list ([`stale_range_lists`]) is ended where it begins rather than
            // believed, and after the relocation pass so that nothing writes the addresses
            // back. Zeroed in place rather than removed, so every offset a unit holds into
            // the section is still one this section has. A unit left declaring no ranges is
            // one `addr2line` takes the ranges of from its line program's sequences, which
            // did move with the code.
            for list in stale.iter().filter(|list| list.section == id) {
                let end = list.offset.saturating_add(list.length);
                if let Some(bytes) = data.get_mut(list.offset..end) {
                    bytes.fill(0);
                }
            }

            Some(data)
        })
        .unwrap_or_default();

    EndianArcSlice::new(Arc::from(data), endian)
}

/// Apply a debug section's relocations to a copy of its bytes.
///
/// In a relocatable object `DW_AT_low_pc` and `DW_LNE_set_address` are written as zero with a
/// relocation against the function's symbol, so line info read without relocating maps every
/// function to address 0. The bytes are already a private copy, so they are patched in place.
///
/// Only `Absolute` relocations are applied, which is what DWARF's address and offset forms
/// use; anything else (a COFF `SECREL`, say) is left alone rather than guessed at.
///
/// The value written is `symbol/section address + addend`, plus the bytes already there when
/// the format keeps the addend in the section (ELF `REL`, COFF) rather than in the relocation
/// (ELF `RELA`), plus the target section's bias. Every step wraps and every write is
/// bounds-checked, so no relocation table, however corrupt, can do more than scribble on this
/// copy.
fn relocate<'data, 'file>(
    data: &mut [u8],
    file: &object::File<'data>,
    section: &object::Section<'data, 'file>,
    endian: RunTimeEndian,
    biases: &HashMap<SectionIndex, u64>,
) {
    for (offset, relocation) in section.relocations() {
        if relocation.kind() != RelocationKind::Absolute {
            continue;
        }

        // A target's address is its section's address plus its offset in it, and in a
        // relocatable object that section address is the bias rather than the 0 the file
        // states.
        let bias = |index: Option<SectionIndex>| {
            index
                .and_then(|index| biases.get(&index))
                .copied()
                .unwrap_or(0)
        };
        let target = match relocation.target() {
            RelocationTarget::Symbol(index) => file
                .symbol_by_index(index)
                .ok()
                .map(|s| s.address().wrapping_add(bias(s.section_index()))),
            RelocationTarget::Section(index) => file
                .section_by_index(index)
                .ok()
                .map(|s| s.address().wrapping_add(bias(Some(index)))),
            _ => None,
        };
        let Some(target) = target else { continue };

        let Ok(offset) = usize::try_from(offset) else {
            continue;
        };
        let size = usize::from(relocation.size()) / 8;
        let Some(bytes) = data.get_mut(offset..offset.wrapping_add(size)) else {
            continue;
        };

        let implicit = if relocation.has_implicit_addend() {
            read_uint(bytes, endian)
        } else {
            0
        };
        let value = implicit
            .wrapping_add(target)
            .wrapping_add(relocation.addend() as u64);

        write_uint(bytes, endian, value);
    }
}

/// The 4- or 8-byte unsigned at `bytes`. Any other width is not a DWARF address or offset and
/// answers 0.
fn read_uint(bytes: &[u8], endian: RunTimeEndian) -> u64 {
    let mut buffer = [0u8; 8];
    match (bytes.len(), endian) {
        (4, RunTimeEndian::Little) | (8, RunTimeEndian::Little) => {
            buffer[..bytes.len()].copy_from_slice(bytes);
            u64::from_le_bytes(buffer)
        }
        (4, _) | (8, _) => {
            buffer[8 - bytes.len()..].copy_from_slice(bytes);
            u64::from_be_bytes(buffer)
        }
        _ => 0,
    }
}

/// The inverse of [`read_uint`]; a width it does not understand is left untouched.
fn write_uint(bytes: &mut [u8], endian: RunTimeEndian, value: u64) {
    let len = bytes.len();
    match (len, endian) {
        (4, RunTimeEndian::Little) | (8, RunTimeEndian::Little) => {
            bytes.copy_from_slice(&value.to_le_bytes()[..len]);
        }
        (4, _) | (8, _) => {
            bytes.copy_from_slice(&value.to_be_bytes()[8 - len..]);
        }
        _ => {}
    }
}

/// One run of instructions and the source position DWARF gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRow {
    /// The instruction addresses this row covers, clipped to the range that was asked about
    /// and in the same address space as [`SymbolData::address`].
    pub range: Range<u64>,
    /// An index into [`LineInfo::files`], or [`None`] when the row names no file.
    pub file: Option<usize>,
    /// DWARF's line number. Genuinely optional: line 0 means "these instructions belong to no
    /// source line", which is neither line 0 nor line 1.
    pub line: Option<u32>,
    /// DWARF's column number, [`None`] both when the producer emitted no column at all and
    /// when it emitted `DW_LNS_set_column 0`, the "left edge of the line" marker.
    pub column: Option<u32>,
}

/// A source position, for callers that want one address answered rather than a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// The line info covering one address range, resolved in a single pass, so a caller holding a
/// symbol's instructions asks once and then answers each of them locally with
/// [`row_at`](Self::row_at).
///
/// The rows are ascending, non-overlapping and coalesced, but *not* contiguous —
/// compiler-generated instructions belonging to no source line leave gaps, and
/// [`row_at`](Self::row_at) returns [`None`] there rather than inventing a position.
/// Non-overlapping is an invariant of this type, established by scoping the query to a
/// section ([`section_biases`]) and by the clipping in [`Dwarf::line_info_inner`]; where two
/// rows genuinely covered one address, the one that starts first keeps it.
pub struct LineInfo {
    rows: Vec<LineRow>,
    files: Vec<Arc<str>>,
}

impl LineInfo {
    /// Every row, ascending by address and non-overlapping.
    pub fn rows(&self) -> &[LineRow] {
        &self.rows
    }

    /// The source files these rows touch, deduplicated, in the order they were first seen.
    /// [`LineRow::file`] indexes into this.
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    /// The row covering `address`, or [`None`] when no row does. The last row starting at or
    /// before `address` is the only candidate *because* the rows do not overlap.
    pub fn row_at(&self, address: u64) -> Option<&LineRow> {
        let index = match self
            .rows
            .binary_search_by_key(&address, |row| row.range.start)
        {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let row = &self.rows[index];
        (address < row.range.end).then_some(row)
    }

    /// The file a row names.
    pub fn file_of(&self, row: &LineRow) -> Option<&str> {
        Some(&*self.files[row.file?])
    }

    /// `(file, line, column)` for a single instruction address.
    pub fn location(&self, address: u64) -> Option<Location<'_>> {
        let row = self.row_at(address)?;
        Some(Location {
            file: self.file_of(row),
            line: row.line,
            column: row.column,
        })
    }
}

impl Object {
    /// The line info for an address range **within one section**, building this object's
    /// DWARF context on the first call and reusing it afterwards.
    ///
    /// The section is not decoration: in a relocatable object every section starts at 0, so
    /// `range` on its own does not say which code it means. See [`section_biases`].
    ///
    /// [`None`] means "no line info" for every reason at once: no DWARF, debug info in a
    /// format this does not read (CodeView), DWARF that will not parse, or DWARF that says
    /// nothing about this range.
    ///
    /// Worker-thread work by construction: the first call parses `.debug_abbrev` and every
    /// unit's header and range list, and each call parses the line program of every unit
    /// covering the range, once per unit for the object's lifetime.
    pub fn line_info(&self, section: &Section, range: Range<u64>) -> Option<Arc<LineInfo>> {
        let dwarf = self.dwarf()?;
        dwarf.line_info(dwarf.bias(section.index), range)
    }

    /// How many bytes of code the debug info says the function starting at `address` **within
    /// one section** is, or [`None`] when it does not say. One DIE walk per compilation unit
    /// visited, cached; see [`SymbolData::extent`] for how it and the next-symbol estimate
    /// bound each other.
    pub fn subprogram_extent(&self, section: &Section, address: u64) -> Option<u64> {
        let dwarf = self.dwarf()?;
        dwarf.extent(dwarf.bias(section.index), address)
    }

    /// This object's DWARF context, built at most once — including the "there is none"
    /// answer.
    fn dwarf(&self) -> Option<&Dwarf> {
        self.dwarf
            .0
            .get_or_init(|| Dwarf::load(&self.data))
            .as_ref()
    }
}

impl SymbolData {
    /// The line info for this symbol's instructions, over the same extent
    /// [`assembly`](Self::assembly) decodes.
    pub fn line_info(&self, object: &Object) -> Option<Arc<LineInfo>> {
        let section = self.section.as_ref()?;
        let end = self.address.checked_add(self.extent(object)?)?;
        object.line_info(section, self.address..end)
    }

    /// What DWARF says this symbol's extent is, [`None`] when it says nothing.
    pub fn dwarf_extent(&self, object: &Object) -> Option<u64> {
        object.subprogram_extent(self.section.as_ref()?, self.address)
    }
}

//! The DWARF backend of [`super`]: `addr2line` over sections copied out of the object, the
//! one module in the crate that knows `gimli` and `addr2line`.
//!
//! The readers are [`gimli::EndianArcSlice`] — an `Arc<[u8]>` per DWARF section — rather
//! than borrows of [`ObjectData`](crate::ObjectData), so the context owns its data and
//! [`Object`](crate::Object) does not become self-referential. Those `Arc`s are separate
//! allocations: a section may be compressed, and every one is relocated in place (see
//! [`relocate`]).
//!
//! Nothing here catches a panic: the guard is [`super::DebugInfo`]'s, one net around every
//! question whichever backend answers it.

use super::{LineInfo, RowCollector};
use crate::{section_data, Section};
use gimli::{EndianArcSlice, RunTimeEndian};
use object::{
    Object as _, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget, SectionIndex,
};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

type Reader = EndianArcSlice<RunTimeEndian>;

/// One object's DWARF, parsed once and kept for the object's lifetime.
pub(super) struct Dwarf {
    /// The [`Mutex`] is about `Sync`, not contention: `addr2line::Context` caches each
    /// unit's parsed line program in an `UnsafeCell` behind `&self`, so it is `Send` but not
    /// `Sync`, and [`Object`](crate::Object) is shared across threads as an `Arc`.
    context: Mutex<addr2line::Context<Reader>>,

    /// Where each code section was placed in the address space the context reads in: the
    /// sections' own [`Section::bias`], see [`crate::section_biases`]. Empty for a linked
    /// image, which needs none.
    biases: HashMap<SectionIndex, u64>,

    /// Every compilation unit that has been asked about, and the extent of each
    /// `DW_TAG_subprogram` in it, keyed by the unit's `.debug_info` offset and then by the
    /// subprogram's `DW_AT_low_pc`. Both keys are in the biased address space.
    extents: Mutex<HashMap<u64, HashMap<u64, u64>>>,
}

impl Dwarf {
    /// Build the context for one object file, or [`None`] when it has no DWARF. Never an
    /// error: corrupt debug info is simply "no line info".
    ///
    /// `sections` are the object's own, for where the parse placed each of them
    /// ([`Section::bias`]): the same layout the code listing reads, so a row's address and a
    /// listing's agree by construction.
    pub(super) fn load(file: &object::File<'_>, sections: &[Arc<Section>]) -> Option<Dwarf> {
        // The cheap test first, so an object with no DWARF costs one section-table scan.
        if !Dwarf::present(file) {
            return None;
        }

        let endian = if file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let biases: HashMap<SectionIndex, u64> = sections
            .iter()
            .filter(|section| section.bias != 0)
            .map(|section| (section.index, section.bias))
            .collect();

        let dwarf =
            gimli::Dwarf::load::<_, ()>(|id| Ok(load_section(file, id, endian, &biases, &[])))
                .ok()?;

        // Read once more, without the range lists that were left behind by the bias. Rare
        // enough — nothing in the tree emits the shape — that reading twice is cheaper than
        // holding the section data aside for a patch that almost never comes.
        let stale = stale_range_lists(file, &biases, &dwarf);
        let dwarf = if stale.is_empty() {
            dwarf
        } else {
            gimli::Dwarf::load::<_, ()>(|id| Ok(load_section(file, id, endian, &biases, &stale)))
                .ok()?
        };

        Some(Dwarf {
            context: Mutex::new(addr2line::Context::from_dwarf(dwarf).ok()?),
            biases,
            extents: Mutex::default(),
        })
    }

    /// Whether the object carries DWARF at all: one section-table scan for `.debug_info`.
    /// `section_by_name` already knows ELF's `.zdebug_info` and Mach-O's `__debug_info`.
    /// The same test [`load`](Self::load) starts with, so the seam's "DWARF first" rule can
    /// be applied before a `.pdb` is opened without building the DWARF backend to ask.
    pub(super) fn present(file: &object::File<'_>) -> bool {
        file.section_by_name(gimli::SectionId::DebugInfo.name())
            .is_some()
    }

    /// How far the section with this index was moved by [`crate::section_biases`]; 0 for a
    /// section that was not moved, and for every section of a linked image.
    pub(super) fn bias(&self, section: SectionIndex) -> u64 {
        self.biases.get(&section).copied().unwrap_or(0)
    }

    /// Resolve a whole address range in one pass. `bias` is what the range's section was
    /// moved by, so the query and the rows it produces are translated in and out of the
    /// address space the context was built in.
    pub(super) fn line_info(&self, bias: u64, range: Range<u64>) -> Option<LineInfo> {
        // A poisoned lock means a previous query panicked. Nothing here is left half-written
        // by one (the context is only ever read), so recover rather than propagate.
        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());

        // Saturating rather than wrapping, so an absurd range asks about less than it meant
        // to instead of about something else.
        let query = range.start.saturating_add(bias)..range.end.saturating_add(bias);

        let mut rows = RowCollector::default();

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

            // DWARF 5 can record a file's MD5 too, but `addr2line` 0.21 renders the name
            // without handing the entry back, so no hash travels with it for now.
            let file = location.file.map(|file| rows.file(file, None));
            rows.push(start..end, file, location.line, location.column);
        }

        drop(context);

        rows.finish()
    }

    /// The extent of the `DW_TAG_subprogram` beginning at `address`, or [`None`] when no unit
    /// covers the address or the subprogram that does begins elsewhere.
    pub(super) fn extent(&self, bias: u64, address: u64) -> Option<u64> {
        let probe = address.checked_add(bias)?;
        // `addr2line` 0.21's `Context::find_units` asks its range index about `probe + 1`
        // with a plain addition, so the very last address in the space panics. Declined here
        // rather than left to the guard: this one is ours to see coming.
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

    /// Every row of every line program that names a file and a line, in the **biased**
    /// address space, handed to `visit` under the context's lock — so `visit` must not ask
    /// this object anything.
    pub(super) fn each_row(&self, visit: &mut dyn FnMut(Range<u64>, &str, u32)) {
        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());

        // The whole address space in one pass. Safe where `extent` had to decline `u64::MAX`:
        // that unchecked `probe + 1` is in `find_units`, and this goes through
        // `find_units_range`, which takes the bound as given.
        let Ok(rows) = context.find_location_range(0, u64::MAX) else {
            return;
        };

        for (address, length, location) in rows {
            // A row naming no file or no line points at nothing a reader could ask for.
            // DWARF line 0 is already `None` by the time `addr2line` has spoken.
            let (Some(file), Some(line)) = (location.file, location.line) else {
                continue;
            };
            let Some(end) = address.checked_add(length) else {
                continue;
            };
            if address >= end {
                continue;
            }
            visit(address..end, file, line);
        }
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

/// One unit range list that [`crate::section_biases`] left behind, and the bytes that make it read
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

//! One object file read into an [`Object`]: its sections and where each is placed, its
//! symbols, the code it declares outside its symbol table, and the names demangled.

use crate::demangle;
use crate::line::{DebugInfo, DebugInfoCache, Procedure, Public};
use crate::unwind::{self, UnwindEntry};
use crate::{MadeUp, Object, ObjectData, Section, SymbolData};
use object::{
    BinaryFormat, CompressionFormat, ExportTarget, Object as _, ObjectKind, ObjectSection,
    ObjectSymbol, SectionIndex, SectionKind, SymbolIndex, SymbolKind,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Where each code section is placed in the one address space the object's line info is read
/// in and its code is listed in; what [`CodeSection::bias`](crate::CodeSection::bias) is
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
pub(crate) fn section_biases(file: &object::File<'_>) -> HashMap<SectionIndex, u64> {
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
        biases.insert(section.index(), next.wrapping_sub(section.address()));

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

/// One symbol as the file states it, in its symbol table or elsewhere ([`declared_code`]),
/// held until the whole object's names are demangled in one batch ([`symbol_data`]).
struct Pending {
    index: SymbolIndex,
    name: String,
    /// Whether the name is the file's own and goes through the demangling batch. One that is
    /// not is a [`MadeUp`] name, which no demangler has anything to say about.
    mangled: bool,
    address: u64,
    /// What the file said: a symbol's size, a PDB procedure's length, an unwind entry's
    /// stated end less its begin, and 0 for an export, the entry point and a PDB public. The
    /// extent used comes from [`SymbolData::extent`], which reads this only where the format
    /// makes it a function's length ([`SymbolData::declared_extent`]).
    size: u64,
    /// The section the symbol is in, looked up when the symbol is built. For declared code it
    /// is the code section containing `address`: an export table and an entry point name an
    /// address and nothing else.
    section: Option<SectionIndex>,
}

/// The text symbols of a file's symbol table, out of [`symbol_table`].
struct SymbolTable {
    /// Each one whose name reads, in table order.
    named: Vec<Pending>,
    /// Each one whose name will not read, in table order, called by its address. One claims
    /// its placed address only where nothing else named it (the table, [`declared_code`], or
    /// one of these before it), so an export, a PDB procedure or public, or an unwind entry
    /// can still give it a real name, and a name at the same offset of another section of a
    /// relocatable object does not drop it.
    unnamed: Vec<Pending>,
    /// The first index past the table, which declared code is numbered from.
    next: usize,
}

/// The code a file declares outside its symbol table: its **entry point**, its **exports**,
/// its ELF `.dynsym`, the **procedures** and **publics** of the `.pdb` a PE names, where
/// that was found and matches (`procedures` and `publics`, out of [`DebugInfo::pdb`]), and
/// the **unwind entries** of an x86-64 PE's exception directory or an ELF's `.eh_frame`
/// (`unwind`, out of [`unwind::entries`]). A stripped shared library is otherwise a file with nothing in it,
/// and a `/DEBUG` image has no symbol table at all.
/// Every address here is one the file — or the debug file matched to it by GUID and age —
/// states outright, so the "nothing is scanned for" rule still holds.
///
/// Three decisions the caller depends on:
///
/// **Only in a code section.** An address is looked up in the kept [`SectionKind::Text`]
/// sections and that section becomes the symbol's own; it doubles as the filter keeping
/// exported *data* out.
///
/// **One symbol per address, earliest source winning** (symbol table > dynamic symbol >
/// export > entry point > PDB procedure > PDB public > unwind entry). An export is very
/// often the symbol table's own function under its exported name, and a second `SymbolData`
/// for it would be a second row in the list for one place in the file. The PDB comes after
/// the image so a name the image itself states is never displaced by the debug file's
/// spelling of it, and its publics after its procedures because a procedure carries a
/// display name and a length where a public is a decorated name and an address: the publics
/// name only what nothing else did — a function in a module that shipped without symbols, a
/// thunk, assembler code, or every function of a stripped PDB. The unwind entries come last
/// of all because they carry no name: one at an address anything else named adds nothing,
/// and one nothing named is called `<function 0x…>` by its address — or `<fragment 0x…>`
/// where its unwind info is chained, a second range of some function's rather than a
/// function ([`UnwindEntry`]).
///
/// **Nothing for a relocatable object.** `entry()` answers 0 for an `.o`, and 0 there is a
/// real function's first byte.
///
/// The indices start at `next`, *past* the file's own symbol table, which is the only honest
/// thing they can be. Nothing can reach them by relocation, since a file that declares
/// exports is a linked image.
fn declared_code(
    file: &object::File<'_>,
    code: &[(Range<u64>, SectionIndex)],
    known: &mut HashSet<u64>,
    next: usize,
    procedures: Vec<Procedure>,
    publics: Vec<Public>,
    unwind: &[UnwindEntry],
) -> Vec<Pending> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    // Takes the name and whether it is the file's own, which is what decides whether the
    // name is offered to the demanglers. `MadeUp::unmangled` is that pair for the names
    // that are ours.
    let mut take = |(name, mangled): (String, bool), address: u64, size: u64| {
        let Some((_, section)) = code.iter().find(|(range, _)| range.contains(&address)) else {
            return;
        };
        if !known.insert(address) {
            return;
        }
        declared.push(Pending {
            index: SymbolIndex(next + declared.len()),
            name,
            mangled,
            address,
            size,
            section: Some(*section),
        });
    };

    for symbol in file.dynamic_symbols() {
        if symbol.kind() != SymbolKind::Text {
            continue;
        }
        let Ok(name) = symbol.name_bytes() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        take(
            (String::from_utf8_lossy(name).into_owned(), true),
            symbol.address(),
            symbol.size(),
        );
    }

    // `exports` reports one entry at a time, so a malformed one is skipped rather than
    // taken as the end of the table. An export names a place in this image only when it
    // has a name and an address: one identified by ordinal has nothing to draw, and a
    // forwarder or a re-export names a place in another image. The name is the file's, and
    // on a Windows DLL very often MSVC-mangled.
    for export in file.exports().into_iter().flatten().flatten() {
        let ExportTarget::Address { address } = export.target() else {
            continue;
        };
        let Some(name) = export.name().into_name() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        take(
            (String::from_utf8_lossy(name).into_owned(), true),
            address,
            0,
        );
    }

    // 0 is "this image has no entry point", which is how a DLL built without one states it.
    let entry = file.entry();
    if entry != 0 {
        take(MadeUp::EntryPoint.unmangled(), entry, 0);
    }

    // After the image's own names, so they win. The address is already in the image's space
    // and the code-section lookup is what drops a procedure the PDB places in a section the
    // image does not have code in. A procedure's name is the compiler's display name, which
    // no demangler claims and so comes through the batch as it is.
    for procedure in procedures {
        take((procedure.name, true), procedure.address, procedure.len);
    }

    // And the publics behind them: a name for whatever address is still unnamed, and no
    // length, as an export has none. A public's name is the linker's, decorated as an
    // export's is. A public in a data section — the flags are the linker's to set, and the
    // section lookup is the rule — is dropped the same way.
    for public in publics {
        take((public.name, true), public.address, 0);
    }

    // Last of all, the unwind entries: an address and a length for whatever is still
    // unnamed, and no name at all.
    for entry in unwind {
        take(
            MadeUp::unwind(entry).unmangled(),
            entry.range.start,
            entry.len(),
        );
    }

    declared
}

/// The address ranges code can be in, each with its section: what [`declared_code`] looks a
/// declared address up in, and what places an unwind entry's range in its
/// [`CodeSection::unwind`](crate::CodeSection::unwind). Only the sections that hold code —
/// one whose bytes would not decompress was dropped, having nothing to disassemble either —
/// and only the ones with bytes.
///
/// In the file's own section order, which is what decides the section an address in two
/// overlapping ranges is taken to be in.
fn code_sections(sections: &HashMap<SectionIndex, Section>) -> Vec<(Range<u64>, SectionIndex)> {
    let mut ranges: Vec<(Range<u64>, SectionIndex)> = sections
        .values()
        .filter_map(|section| {
            let length: u64 = section.code()?.data.len().try_into().ok()?;
            let end = section.address.checked_add(length)?;
            (length > 0).then_some((section.address..end, section.index))
        })
        .collect();
    ranges.sort_unstable_by_key(|&(_, index)| index.0);
    ranges
}

/// Parse `data` as a single object file. `name` is the display name (an archive member name
/// or the file name) and `path` the file it came from. Anything that fails to parse yields
/// [`None`]. `data` is kept in the returned [`Object`]; see [`ObjectData`].
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let file = object::File::parse(data.bytes()).ok()?;

    let sections = read_sections(&file);
    let SymbolTable {
        named: mut symbols,
        unnamed,
        next,
    } = symbol_table(&file);
    // Keyed by placed address: in a relocatable object every section starts at 0, so an
    // address alone does not say which code it is ([`section_biases`]).
    let place = |symbol: &Pending| {
        let section = symbol.section.and_then(|index| sections.get(&index));
        symbol
            .address
            .wrapping_add(section.map_or(0, Section::bias))
    };
    let mut known: HashSet<u64> = symbols.iter().map(place).collect();

    let (debug_info, procedures, publics) = open_pdb(&file, &path);
    let unwind = unwind::entries(&file);
    let code = code_sections(&sections);
    let declared = declared_code(&file, &code, &mut known, next, procedures, publics, &unwind);
    let ranges = place_unwind(&code, &unwind);

    // After `declared_code`, so `known` holds every address anything named.
    symbols.extend(
        unnamed
            .into_iter()
            .filter(|symbol| known.insert(place(symbol))),
    );
    symbols.extend(declared);

    let sections = freeze_sections(sections, ranges);
    let symbols = symbol_data(symbols, &sections);

    let format = file.format();
    let architecture = file.architecture();
    let mut object = Object::new(
        path,
        name,
        format,
        architecture,
        symbols,
        sections.into_values().collect(),
        data,
    );
    object.debug_info = debug_info;
    Some(Arc::new(object))
}

/// Every section the file states, by index. Each code section's place is decided here, once,
/// for the line info and the code listing both ([`section_biases`]).
fn read_sections(file: &object::File<'_>) -> HashMap<SectionIndex, Section> {
    let biases = section_biases(file);
    let format = file.format();
    file.sections()
        .filter_map(|section| {
            let index = section.index();
            let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();

            // Only a code section's bytes are read here. Whatever reads another -- the line
            // info, the unwind tables -- reads it out of the file again, so a copy here would
            // be a second one held for the object's life. A code section whose bytes will not
            // decompress is dropped outright: there is nothing to disassemble in it and
            // nothing else to keep it for.
            if section.kind() != SectionKind::Text {
                return Some((index, Section::other(index, name, section.address())));
            }
            let data = section_data(&section)?;

            // Mach-O states a relocation's place as an offset from the start of its section,
            // and lays its sections out one after another, so that offset is not the address
            // for any section but the first. Every lookup here is by address, so the
            // conversion is done once, where the map is built. ELF and COFF need none: a
            // relocatable object's sections are all at 0, and a linked ELF's `r_offset` is
            // already an address. Only a code section's are collected, because the only
            // reader is the disassembler's operand lookup; the DWARF backend takes a debug
            // section's from the file.
            let base = match format {
                BinaryFormat::MachO => section.address(),
                _ => 0,
            };
            let relocations = section
                .relocations()
                .filter_map(|(offset, relocation)| Some((base.checked_add(offset)?, relocation)))
                .collect();
            let bias = biases.get(&index).copied().unwrap_or(0);
            Some((
                index,
                Section::text(index, name, data, section.address(), relocations, bias),
            ))
        })
        .collect()
}

/// The file's text symbols.
///
/// A symbol whose name will not read is a place in the file all the same. It is set aside
/// until the rest have claimed their addresses ([`SymbolTable::unnamed`]).
fn symbol_table(file: &object::File<'_>) -> SymbolTable {
    let mut table = SymbolTable {
        named: Vec::new(),
        unnamed: Vec::new(),
        next: 0,
    };
    for symbol in file.symbols() {
        table.next = table.next.max(symbol.index().0 + 1);
        if symbol.kind() != SymbolKind::Text {
            continue;
        }

        let address = symbol.address();
        let section = symbol.section().index();

        let pending = |(name, mangled)| Pending {
            index: symbol.index(),
            name,
            mangled,
            address,
            size: symbol.size(),
            section,
        };
        match symbol.name_bytes() {
            Ok(name) => table
                .named
                .push(pending((String::from_utf8_lossy(name).into_owned(), true))),
            Err(_) => table
                .unnamed
                .push(pending(MadeUp::Function(address).unmangled())),
        }
    }
    table
}

/// A PE's matching `.pdb`, with the procedures and publics it names; an empty cache and none
/// for any other file. Opened here and not on the first line question, because those are
/// functions the image itself does not declare. The backend it builds is kept for the line
/// questions later.
fn open_pdb(file: &object::File<'_>, path: &Path) -> (DebugInfoCache, Vec<Procedure>, Vec<Public>) {
    match DebugInfo::pdb(file, path) {
        Some((info, procedures, publics)) => (DebugInfoCache::preloaded(info), procedures, publics),
        None => (DebugInfoCache::default(), Vec::new(), Vec::new()),
    }
}

/// Every unwind entry's range, by the section it starts in, whether or not its begin became a
/// symbol: an export or a procedure at that address takes its extent from the end the entry
/// states. The first section holding the start takes it.
fn place_unwind(
    code: &[(Range<u64>, SectionIndex)],
    unwind: &[UnwindEntry],
) -> HashMap<SectionIndex, Vec<Range<u64>>> {
    let mut ranges: HashMap<SectionIndex, Vec<Range<u64>>> = HashMap::new();
    for UnwindEntry { range, .. } in unwind {
        let Some((_, index)) = code
            .iter()
            .find(|(bounds, _)| bounds.contains(&range.start))
        else {
            continue;
        };
        ranges.entry(*index).or_default().push(range.clone());
    }
    ranges
}

/// The sections, done with: each given the unwind ranges that start in it, then shared.
/// [`Section::with_unwind`] clamps each range's end to the section's bytes, so that end can
/// never reach past what `bytes` can read.
fn freeze_sections(
    sections: HashMap<SectionIndex, Section>,
    mut ranges: HashMap<SectionIndex, Vec<Range<u64>>>,
) -> HashMap<SectionIndex, Arc<Section>> {
    sections
        .into_iter()
        .map(|(index, section)| {
            let unwind = ranges.remove(&index).unwrap_or_default();
            (index, Arc::new(section.with_unwind(unwind)))
        })
        .collect()
}

/// Every symbol built, by index, with its section looked up and its name demangled.
///
/// One batch for the whole object, on stacks of their own and on as many cores as the pool
/// has; see [`demangle`]. The file's own names are *moved* into the batch and come back out
/// of it rather than being copied: 115k names is not a copy worth making.
fn symbol_data(
    mut symbols: Vec<Pending>,
    sections: &HashMap<SectionIndex, Arc<Section>>,
) -> HashMap<SymbolIndex, Arc<SymbolData>> {
    let (names, demangled) = demangle::batch(
        symbols
            .iter_mut()
            .map(|symbol| symbol.mangled.then(|| std::mem::take(&mut symbol.name)))
            .collect(),
    );

    symbols
        .into_iter()
        .zip(names)
        .zip(demangled)
        .map(|((symbol, name), demangled)| {
            (
                symbol.index,
                Arc::new(SymbolData::new(
                    // `None` for a name that was not offered, and so is still the symbol's.
                    name.unwrap_or(symbol.name),
                    demangled,
                    symbol.address,
                    symbol
                        .section
                        .and_then(|index| sections.get(&index).cloned()),
                    symbol.size,
                )),
            )
        })
        .collect()
}

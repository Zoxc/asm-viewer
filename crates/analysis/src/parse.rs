//! One object file read into an [`Object`]: its sections and where each is placed, its
//! symbols, the code it declares outside its symbol table, and the names demangled.

use crate::demangle;
use crate::line::{DebugInfo, Declared};
use crate::unwind::{self, UnwindEntry};
use crate::{Bias, MadeUp, Object, ObjectData, PlacedAddress, Section, SectionAddress, SymbolData};
use object::{
    BinaryFormat, CompressionFormat, ExportTarget, Object as _, ObjectKind, ObjectSection,
    ObjectSymbol, SectionIndex, SectionKind, SymbolIndex, SymbolKind,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
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

/// A symbol's name as the parse holds it, until [`symbol_data`] builds the symbol. Only a
/// [`Name::Symbol`] is offered to the demanglers.
pub(crate) enum Name {
    /// The file's own spelling, or a debug file's decorated one: offered to the demanglers.
    Symbol(String),
    /// A debug file's name that is already fit to show, which no demangler has anything to
    /// say about.
    Informative(String),
    /// One of ours, rendered to a `String` only when the symbol is built.
    MadeUp(MadeUp),
}

/// One symbol as the file states it, in its symbol table or elsewhere ([`declared_code`]),
/// held until the whole object's names are demangled in one batch ([`symbol_data`]).
struct Pending {
    index: SymbolIndex,
    name: Name,
    address: SectionAddress,
    /// What the file said: a symbol's size, the length a debug file's record states, an
    /// unwind entry's stated end less its begin, and 0 for an export, the entry point and a
    /// record with no length of its own. The
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
    /// one of these before it), so an export, a name out of the debug file, or an unwind
    /// entry can still give it a real name, and a name at the same offset of another section
    /// of a relocatable object does not drop it.
    unnamed: Vec<Pending>,
    /// The first index past the table, which declared code is numbered from.
    next: usize,
}

/// The code a file declares outside its symbol table: its **entry point**, its **exports**,
/// its ELF `.dynsym`, the functions its **debug file** names where there is one (`named`, out
/// of [`DebugInfo::declared`]), and the **unwind entries** of an x86-64 PE's exception
/// directory or an ELF's `.eh_frame` (`unwind`, out of [`unwind::entries`]). A stripped
/// shared library is otherwise a file with nothing in it, and a `/DEBUG` image has no symbol
/// table at all.
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
/// export > entry point > debug file > unwind entry). An export is very often the symbol
/// table's own function under its exported name, and a second `SymbolData` for it would be a
/// second row in the list for one place in the file. The debug file comes after the image so
/// a name the image itself states is never displaced by the debug file's spelling of it, and
/// its own records are taken in the order it hands them over, which is the order it wants
/// them believed. The unwind entries come last of all because they carry no name: one at an
/// address anything else named adds nothing, and one nothing named is called
/// `<function 0x…>` by its address — or `<fragment 0x…>` where its unwind info is chained, a
/// second range of some function's rather than a function ([`UnwindEntry`]).
///
/// **Nothing for a relocatable object.** `entry()` answers 0 for an `.o`, and 0 there is a
/// real function's first byte.
///
/// The indices start at `next`, *past* the file's own symbol table, which is the only honest
/// thing they can be. Nothing can reach them by relocation, since a file that declares
/// exports is a linked image.
fn declared_code(
    file: &object::File<'_>,
    code: &[(Range<SectionAddress>, SectionIndex)],
    known: &mut HashSet<PlacedAddress>,
    next: usize,
    named: Vec<Declared>,
    unwind: &[UnwindEntry],
) -> Vec<Pending> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    // Which kind of name it is decides whether it is offered to the demanglers.
    let mut take = |name: Name, address: SectionAddress, size: u64| {
        let Some((_, section)) = code.iter().find(|(range, _)| range.contains(&address)) else {
            return;
        };
        // `known` is keyed by placed address, and nothing here is a relocatable object's
        // ([`section_biases`]), so nothing placed this one.
        if !known.insert(address.unplaced()) {
            return;
        }
        declared.push(Pending {
            index: SymbolIndex(next + declared.len()),
            name,
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
            Name::Symbol(String::from_utf8_lossy(name).into_owned()),
            SectionAddress::new(symbol.address()),
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
            Name::Symbol(String::from_utf8_lossy(name).into_owned()),
            SectionAddress::new(address),
            0,
        );
    }

    // 0 is "this image has no entry point", which is how a DLL built without one states it.
    let entry = file.entry();
    if entry != 0 {
        take(
            Name::MadeUp(MadeUp::EntryPoint),
            SectionAddress::new(entry),
            0,
        );
    }

    // After the image's own names, so they win, and among themselves in the order the debug
    // file handed them over. The addresses are already in the image's space, and the
    // code-section lookup is what drops a record the debug file places in a section the
    // image does not have code in — a public the linker flagged as code that is not, say.
    for function in named {
        take(function.name, function.address, function.len);
    }

    // Last of all, the unwind entries: an address and a length for whatever is still
    // unnamed, and no name at all.
    for entry in unwind {
        take(
            Name::MadeUp(MadeUp::unwind(entry)),
            entry.range.start,
            entry.len(),
        );
    }

    declared
}

/// The address ranges code can be in, each with its section: what [`declared_code`] looks a
/// declared address up in, and what places an unwind entry's range in its
/// [`CodeSection::unwind`](crate::CodeSection::unwind). Each is the section's own
/// [`bytes_range`](Section::bytes_range), so only the sections that hold code are here — one
/// whose bytes would not decompress was dropped, having nothing to disassemble either — and
/// only the ones whose bytes have addresses to sit at.
///
/// In the file's own section order, which is what decides the section an address in two
/// overlapping ranges is taken to be in.
fn code_sections(
    sections: &HashMap<SectionIndex, Section>,
) -> Vec<(Range<SectionAddress>, SectionIndex)> {
    let mut ranges: Vec<(Range<SectionAddress>, SectionIndex)> = sections
        .values()
        .filter_map(|section| Some((section.bytes_range()?, section.index)))
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
        section.map_or(symbol.address.unplaced(), |section| {
            section.place(symbol.address)
        })
    };
    let mut known: HashSet<PlacedAddress> = symbols.iter().map(place).collect();

    // The debug file is opened here and not on the first line question, because the
    // functions it names are ones the image itself does not declare. The backend it builds
    // is kept for the line questions later.
    let (preloaded, named) = match DebugInfo::declared(&file, &path) {
        Some((info, named)) => (Some(info), named),
        None => (None, Vec::new()),
    };
    let unwind = unwind::entries(&file);
    let code = code_sections(&sections);
    let declared = declared_code(&file, &code, &mut known, next, named, &unwind);
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
    let object = Object::preloaded(
        path,
        name,
        format,
        architecture,
        symbols,
        sections.into_values().collect(),
        data,
        preloaded,
    );
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
                return Some((
                    index,
                    Section::other(index, name, SectionAddress::new(section.address())),
                ));
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
                .filter_map(|(offset, relocation)| {
                    Some((SectionAddress::new(base).checked_add(offset)?, relocation))
                })
                .collect();
            let bias = biases.get(&index).copied().unwrap_or(Bias::NONE);
            Some((
                index,
                Section::text(
                    index,
                    name,
                    data,
                    SectionAddress::new(section.address()),
                    relocations,
                    bias,
                ),
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

        let address = SectionAddress::new(symbol.address());
        let section = symbol.section().index();

        let pending = |name: Name| Pending {
            index: symbol.index(),
            name,
            address,
            size: symbol.size(),
            section,
        };
        match symbol.name_bytes() {
            Ok(name) => table.named.push(pending(Name::Symbol(
                String::from_utf8_lossy(name).into_owned(),
            ))),
            Err(_) => table
                .unnamed
                .push(pending(Name::MadeUp(MadeUp::Function(address)))),
        }
    }
    table
}

/// Every unwind entry's range, by the section it starts in, whether or not its begin became a
/// symbol: an export or a procedure at that address takes its extent from the end the entry
/// states. The first section holding the start takes it.
fn place_unwind(
    code: &[(Range<SectionAddress>, SectionIndex)],
    unwind: &[UnwindEntry],
) -> HashMap<SectionIndex, Vec<Range<SectionAddress>>> {
    let mut ranges: HashMap<SectionIndex, Vec<Range<SectionAddress>>> = HashMap::new();
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
    mut ranges: HashMap<SectionIndex, Vec<Range<SectionAddress>>>,
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
/// has; see [`demangle`]. Only the [`Name::Symbol`]s are offered: each is *moved* into the
/// batch and comes back out beside the rest of its symbol, in the order it went in. 115k
/// names is not a copy worth making.
fn symbol_data(
    symbols: Vec<Pending>,
    sections: &HashMap<SectionIndex, Arc<Section>>,
) -> HashMap<SymbolIndex, Arc<SymbolData>> {
    let mut built = HashMap::with_capacity(symbols.len());
    let mut build = |index, name, demangled, address, size, section: Option<SectionIndex>| {
        let section = section.and_then(|index| sections.get(&index).cloned());
        let symbol = SymbolData::new(name, demangled, address, section, size);
        built.insert(index, Arc::new(symbol));
    };

    // `waiting[i]` is the rest of the symbol `offered[i]` names.
    let mut offered = Vec::new();
    let mut waiting = Vec::new();
    for Pending {
        index,
        name,
        address,
        size,
        section,
    } in symbols
    {
        match name {
            Name::Symbol(name) => {
                offered.push(name);
                waiting.push((index, address, size, section));
            }
            Name::Informative(name) => build(index, name, None, address, size, section),
            Name::MadeUp(made_up) => {
                build(index, made_up.to_string(), None, address, size, section)
            }
        }
    }

    let (names, demangled) = demangle::batch(offered);
    for ((index, address, size, section), (name, demangled)) in
        waiting.into_iter().zip(names.into_iter().zip(demangled))
    {
        build(index, name, demangled, address, size, section);
    }
    built
}

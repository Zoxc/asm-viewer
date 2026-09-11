//! One object file read into an [`Object`]: its sections and where each is placed, its
//! symbols, the code it declares outside its symbol table, and the names demangled.

use crate::demangle;
use crate::line::{DebugInfo, Procedure, Public};
use crate::unwind::{self, UnwindEntry};
use crate::{
    AddressIndex, DebugInfoCache, ExtentCache, MadeUp, Object, ObjectData, Section, SymbolData,
};
use object::{
    Architecture, BinaryFormat, CompressionFormat, ExportTarget, Object as _, ObjectKind,
    ObjectSection, ObjectSymbol, SectionIndex, SectionKind, SymbolIndex, SymbolKind,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

/// Where each code section is placed in the one address space the object's line info is read
/// in and its code is listed in; what [`Section::bias`] is set from.
///
/// **An address alone is not a key in a relocatable object.** Sections there have no address
/// until linked and rustc emits one `.text.<name>` per function, so every function lands on 0
/// and the line programs pile up. This does what a linker does and gives each code section a
/// place of its own: a bias, added to every address relocated against that section
/// (`line::relocate`) and subtracted again from every row a query returns.
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

/// One symbol as the file states it, held while the whole object's names are demangled in
/// one batch (see [`demangle`]).
struct Pending {
    index: SymbolIndex,
    name: String,
    /// Whether the name is the file's own. One that is not is a [`MadeUp`] name, and no
    /// demangler has anything to say about one of those.
    mangled: bool,
    section: Option<Arc<Section>>,
    address: u64,
    size: u64,
}

/// A function the file **declares** somewhere other than its symbol table; see
/// [`declared_code`].
struct DeclaredCode {
    name: String,
    /// Whether `name` is the file's own and goes through the demangling batch. The two
    /// declarations that carry no name — the entry point and an unwind entry, function or
    /// fragment — are [`MadeUp`] instead, which no demangler has anything to say about.
    mangled: bool,
    address: u64,
    /// What the declaration itself said: a dynamic symbol's size, a PDB procedure's length,
    /// an unwind entry's stated end less its begin, and 0 for an export, the entry point and
    /// a PDB public. The extent used comes from [`SymbolData::extent`], which reads this
    /// only where the format makes it a function's length ([`SymbolData::declared_extent`]).
    size: u64,
    /// The code section containing `address` — an export table and an entry point name an
    /// address and nothing else.
    section: SectionIndex,
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
fn declared_code(
    file: &object::File<'_>,
    code: &[(Range<u64>, SectionIndex)],
    known: &mut HashSet<u64>,
    procedures: Vec<Procedure>,
    publics: Vec<Public>,
    unwind: &[UnwindEntry],
) -> Vec<DeclaredCode> {
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
        declared.push(DeclaredCode {
            name,
            mangled,
            address,
            size,
            section: *section,
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
    // forwarder or a re-export names a place in another image.
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
    // length, as an export has none. A public in a data section — the flags are the
    // linker's to set, and the section lookup is the rule — is dropped the same way.
    for public in publics {
        take((public.name, true), public.address, 0);
    }

    // Last of all, the unwind entries: an address and a length for whatever is still
    // unnamed, and no name at all.
    for entry in unwind {
        take(
            MadeUp::unwind(entry).unmangled(),
            entry.range.start,
            entry.range.end - entry.range.start,
        );
    }

    declared
}

/// The address ranges code can be in, each with its section: what [`declared_code`] looks a
/// declared address up in, and what places an unwind entry's range in its
/// [`Section::unwind`]. Only sections that were kept — one whose bytes would not decompress
/// has nothing to disassemble either — and only the ones with bytes.
fn code_sections(
    file: &object::File<'_>,
    sections: &HashMap<SectionIndex, Section>,
) -> Vec<(Range<u64>, SectionIndex)> {
    file.sections()
        .filter(|section| section.kind() == SectionKind::Text)
        .filter_map(|section| {
            let kept = sections.get(&section.index())?;
            let length: u64 = kept.data.as_ref()?.len().try_into().ok()?;
            let end = kept.address.checked_add(length)?;
            (length > 0).then_some((kept.address..end, section.index()))
        })
        .collect()
}

/// Parse `data` as a single object file. `name` is the display name (an archive member name
/// or the file name) and `path` the file it came from. Anything that fails to parse yields
/// [`None`]. `data` is kept in the returned [`Object`]; see [`ObjectData`].
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let object = object::File::parse(data.bytes())
        .map(|file| {
            // Where each code section goes, decided once here for the line info and the
            // code listing both.
            let biases = section_biases(&file);
            let format = file.format();
            let mut sections: HashMap<SectionIndex, Section> = file
                .sections()
                .filter_map(|section| {
                    let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();

                    // Only a code section's bytes are read here. Whatever reads another
                    // -- the line info, the unwind tables -- reads it out of the file
                    // again, so a copy here would be a second one held for the object's
                    // life. A code section whose bytes will not decompress is dropped
                    // outright: there is nothing to disassemble in it and nothing else to
                    // keep it for.
                    let code = section.kind() == SectionKind::Text;
                    let data = if code {
                        Some(section_data(&section)?)
                    } else {
                        None
                    };

                    // Mach-O states a relocation's place as an offset from the start of
                    // its section, and lays its sections out one after another, so that
                    // offset is not the address for any section but the first. Every
                    // lookup here is by address, so the conversion is done once, where
                    // the map is built. ELF and COFF need none: a relocatable object's
                    // sections are all at 0, and a linked ELF's `r_offset` is already an
                    // address. Only a code section's are collected, because the only
                    // reader is the disassembler's operand lookup; the DWARF backend
                    // takes a debug section's from the file.
                    let base = match format {
                        BinaryFormat::MachO => section.address(),
                        _ => 0,
                    };
                    let relocations = if code {
                        section
                            .relocations()
                            .filter_map(|(offset, relocation)| {
                                Some((base.checked_add(offset)?, relocation))
                            })
                            .collect()
                    } else {
                        HashMap::new()
                    };
                    Some((
                        section.index(),
                        Section {
                            index: section.index(),
                            name,
                            address: section.address(),
                            data,
                            symbols: Vec::new(),
                            unwind: Vec::new(),
                            relocations,
                            code,
                            bias: biases.get(&section.index()).copied().unwrap_or(0),
                        },
                    ))
                })
                .collect();

            // Insert symbol addresses into sections. The addresses are collected as they go,
            // because that set is what tells `declared_code` which of the file's exports are
            // already in the symbol table under their own name.
            //
            // A symbol whose name will not read out of the string table is a place in the
            // file all the same, so its address goes into the section and the symbol below
            // it keeps its extent. It stays out of `known`, so an export, a PDB procedure or
            // public, or an unwind entry can still claim that address and give it a real
            // name; only where none does is the symbol listed by its address, below.
            let mut known: HashSet<u64> = HashSet::new();
            file.symbols().for_each(|symbol| {
                if symbol.kind() != SymbolKind::Text {
                    return;
                }

                if symbol.name_bytes().is_ok() {
                    known.insert(symbol.address());
                }
                symbol
                    .section()
                    .index()
                    .and_then(|index| sections.get_mut(&index))
                    .map(|section| section.symbols.push(symbol.address()));
            });

            // A PE's matching `.pdb` is opened here and not on the first line question,
            // because the procedures and publics it names are functions the image itself
            // does not declare. The backend it builds is kept for the line questions later.
            let (debug_info, procedures, publics) = match DebugInfo::pdb(&file, &path) {
                Some((info, procedures, publics)) => {
                    (DebugInfoCache::preloaded(info), procedures, publics)
                }
                None => (DebugInfoCache::default(), Vec::new(), Vec::new()),
            };

            // Declared code goes into the same sorted lists, because that list is what
            // `estimate_size` derives an extent from and a declaration carries none.
            let unwind = unwind::entries(&file);
            let code = code_sections(&file, &sections);
            let declared = declared_code(&file, &code, &mut known, procedures, publics, &unwind);
            for code in &declared {
                if let Some(section) = sections.get_mut(&code.section) {
                    section.symbols.push(code.address);
                }
            }

            // Every unwind entry's range goes to its section, whether or not its begin
            // became a symbol: an export or a procedure at that address takes its extent
            // from the end the entry states. Clamped to the section's bytes, so that end can
            // never reach past what `bytes` can read.
            for UnwindEntry { range, .. } in &unwind {
                let Some((bounds, index)) = code
                    .iter()
                    .find(|(bounds, _)| bounds.contains(&range.start))
                else {
                    continue;
                };
                if let Some(section) = sections.get_mut(index) {
                    section.unwind.push(range.start..range.end.min(bounds.end));
                }
            }

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    // Sorted for the binary searches over it, and each address once: two
                    // symbols at one address (an alias, an assembler label) are one place
                    // in the section, and a repeated entry would make `estimate_size`
                    // answer 0 for whichever of the two the search landed on.
                    section.symbols.sort_unstable();
                    section.symbols.dedup();
                    // The unwind ranges likewise, by start: a table stating one function
                    // twice is one function, and the search over them assumes it.
                    section.unwind.sort_unstable_by_key(|range| range.start);
                    section.unwind.dedup_by_key(|range| range.start);
                    (index, Arc::new(section))
                })
                .collect();

            let sections = section_map.values().cloned().collect();

            let mut pending: Vec<Pending> = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let section = symbol
                        .section()
                        .index()
                        .and_then(|index| section_map.get(&index).cloned());

                    // A name that will not read leaves the address as the only thing to
                    // call the symbol by. `known` now holds every address the table named
                    // and every one `declared_code` claimed, so this walk runs after it
                    // and takes only what nothing else named: a second symbol at an
                    // address an export or an unwind entry already named would be a second
                    // row for one place in the file.
                    let (name, mangled) = match symbol.name_bytes() {
                        Ok(name) => (String::from_utf8_lossy(name).into_owned(), true),
                        Err(_) => {
                            let address = symbol.address();
                            known
                                .insert(address)
                                .then(|| MadeUp::Function(address).unmangled())?
                        }
                    };

                    Some(Pending {
                        index: symbol.index(),
                        name,
                        mangled,
                        section,
                        address: symbol.address(),
                        size: symbol.size(),
                    })
                })
                .collect();

            // The declared code joins the same map so `symbols_sorted` stays derived from one
            // place. The keys are indices *past* the file's own symbol table, which is the
            // only honest thing they can be; nothing can reach them by relocation, since a
            // file that declares exports is a linked image.
            if !declared.is_empty() {
                let next = file
                    .symbols()
                    .map(|symbol| symbol.index().0)
                    .max()
                    .map_or(0, |index| index + 1);
                for (offset, code) in declared.into_iter().enumerate() {
                    pending.push(Pending {
                        index: SymbolIndex(next + offset),
                        name: code.name,
                        // An export's name is the file's, and on a Windows DLL very often
                        // MSVC-mangled; a PDB public's is the linker's, decorated the same
                        // way; a PDB procedure's is the compiler's display name, which no
                        // demangler claims and so comes through as it is; the entry
                        // point's and an unwind entry's are ours.
                        mangled: code.mangled,
                        section: section_map.get(&code.section).cloned(),
                        address: code.address,
                        size: code.size,
                    });
                }
            }

            // One batch for the whole object, on stacks of their own and on as many cores
            // as the pool has; see `demangle`. The names are *moved* into the batch and
            // moved back out below rather than copied into it: a shared batch is what lets
            // a job outlive this frame, and 115k names is not a copy worth making for it.
            let names: demangle::Names = Arc::new(
                pending
                    .iter_mut()
                    .map(|symbol| symbol.mangled.then(|| std::mem::take(&mut symbol.name)))
                    .collect(),
            );
            let demangled = demangle::batch(&names);
            // Every job is done, so this is the only reference; the clone is unreachable
            // and is there so that a job that somehow outlived its batch costs a copy
            // rather than the names.
            let names = Arc::try_unwrap(names).unwrap_or_else(|names| (*names).clone());

            let symbols: HashMap<_, _> = pending
                .into_iter()
                .zip(names)
                .zip(demangled)
                .map(|((symbol, name), demangled)| {
                    (
                        symbol.index,
                        Arc::new(SymbolData {
                            name: name.unwrap_or(symbol.name),
                            demangled,
                            section: symbol.section,
                            address: symbol.address,
                            size: symbol.size,
                            extent: ExtentCache::default(),
                        }),
                    )
                })
                .collect();

            let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
            symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));

            ParsedObject {
                format: file.format(),
                architecture: file.architecture(),
                symbols,
                symbols_sorted,
                sections,
                debug_info,
            }
        })
        .ok()?;

    // Nothing above borrows the file any more -- sections own decompressed copies of
    // their bytes and relocations are owned values -- so the input can be moved in.
    Some(Arc::new(Object {
        name,
        path,
        format: object.format,
        architecture: object.architecture,
        symbols: object.symbols,
        symbols_sorted: object.symbols_sorted,
        sections: object.sections,
        data,
        debug_info: object.debug_info,
        by_address: AddressIndex::default(),
    }))
}

/// Everything [`parse_object`] reads out of the file. It exists only so the borrow of `data`
/// ends before `data` itself is moved into the object.
struct ParsedObject {
    format: BinaryFormat,
    architecture: Architecture,
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
    /// Seeded with the PDB backend where one was opened for its procedures, empty otherwise.
    debug_info: DebugInfoCache,
}

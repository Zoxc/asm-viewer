//! In-memory fixture builders: every test object is assembled with the `object` and
//! `gimli` writers rather than read off disk.

#![allow(dead_code)]

use analysis::{parse_object, CodeListing, Instruction, Listing, Object, Place, SymbolData};
use object::write;
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationKind, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Parse a fixture built by one of the writers below. The name and path are the same for
/// every one of them: nothing asserts on either, and a fixture is identified by what it
/// holds rather than by what it is called.
pub fn parse(data: &[u8]) -> Arc<Object> {
    parse_object(data.into(), "fixture.o".into(), PathBuf::from("/fixture.o"))
        .expect("the fixture parses")
}

/// Every text symbol's name, in the sorted order the object lists them.
pub fn names(object: &Object) -> Vec<&str> {
    object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect()
}

/// The symbol a fixture was built to have. The panic lists what the object actually
/// holds, which is the only thing that helps when a fixture stops declaring it.
pub fn named<'a>(object: &'a Object, name: &str) -> &'a Arc<SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol named {name}; got {:?}", names(object)))
}

/// [`named`] for a caller that wants the symbol on its own rather than borrowed from the
/// object it came out of.
pub fn symbol(object: &Object, name: &str) -> Arc<SymbolData> {
    named(object, name).clone()
}

/// One instruction's formatted text, the spans it was captured in run back together.
pub fn text(instruction: &Instruction) -> String {
    instruction
        .format
        .iter()
        .map(|(text, _)| text.as_str())
        .collect()
}

/// Where one of the committed, compiler-produced fixtures (`tests/fixtures/`) sits on disk.
/// The path itself matters to one of them: a PE's `.pdb` is looked for **beside the
/// binary**, so a test that wants the pair found has to parse the DLL under its real path.
pub fn committed_fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// One of the committed, compiler-produced fixtures (`tests/fixtures/`) — the only inputs
/// in the suite a real toolchain wrote. A missing one is a broken checkout, not a reason
/// to skip: fail loudly and say how to put it back.
pub fn committed_fixture(name: &str) -> Vec<u8> {
    let path = committed_fixture_path(name);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}\n\
             This fixture is committed to the repository, not generated. Restore it from \
             git, or rebuild it with the command in tests/fixtures/line_fixture.c (the gcc \
             objects), tests/pdb.rs (the DLLs and their PDBs) or tests/unwind.rs (the \
             shared object).",
            path.display()
        )
    })
}

/// How many symbols of one object [`parse_and_walk`] asks a source question about. The index
/// behind them is built once, so the rest is a binary search each — but the sweep runs this
/// thousands of times and every symbol of every mutation is a line-info query apiece.
const MAX_SOURCE_QUERIES: usize = 4;

/// How many stretches of each section's listing [`parse_and_walk`] decodes. The skeleton is
/// built whole, since it is the cheap half; a decode is the symbol's own disassembly again,
/// and `Section` has no kind, so this walks a `.debug_info`'s listing as readily as a
/// `.text`'s.
const MAX_LISTING_STRETCHES: usize = 4;

/// Parse, then walk everything a parsed object exposes, so a panic anywhere past
/// `parse_object` is caught too. The object is placed at a path nothing sits beside.
pub fn parse_and_walk(data: &[u8]) -> Option<Arc<Object>> {
    parse_and_walk_at(data, PathBuf::from("/fuzz"))
}

/// [`parse_and_walk`] with the object placed at `path`, which is where a PE's `.pdb` is
/// looked for: the one way the walk reaches the PDB backend.
pub fn parse_and_walk_at(data: &[u8], path: PathBuf) -> Option<Arc<Object>> {
    let object = parse_object(data.into(), "fuzz".into(), path)?;

    for symbol in &object.symbols_sorted {
        let _ = symbol.estimate_size();
        let _ = symbol.data();
        let _ = symbol.extent(&object);
        let _ = symbol.data_in(&object);
        if let Some(assembly) = symbol.assembly(&object) {
            for instruction in &assembly.instructions {
                let _: String = instruction.format.iter().map(|(t, _)| t.as_str()).collect();
            }
            // Both ends index `instructions`, so a renderer must never be handed a row
            // that is not there.
            let mut previous = None;
            for edge in &assembly.edges {
                assert!(edge.from < assembly.instructions.len());
                assert!(edge.to < assembly.instructions.len());
                assert_ne!(edge.from, edge.to);
                // One edge per instruction at most, in listing order.
                assert!(previous < Some(edge.from));
                previous = Some(edge.from);
            }
        }
        // Rows are ascending and non-overlapping for *any* input, however corrupt — they
        // are clipped to make it so — hence `previous` is the last row's end.
        if let Some(info) = symbol.line_info(&object) {
            let mut previous = 0;
            for row in info.rows() {
                assert!(row.range.start >= previous && row.range.start < row.range.end);
                previous = row.range.end;
                assert_eq!(
                    info.row_at(row.range.start)
                        .map(|found| found.range.clone()),
                    Some(row.range.clone())
                );
                let _ = info.file_of(row);
                let _ = info.location(row.range.start);
            }
            let _ = info.location(u64::MAX);
        }
    }
    // Build the DWARF context even for an object whose symbols were all dropped.
    for section in &object.sections {
        let _ = object.line_info(section, 0..u64::MAX);
    }

    // Every section's listing: the skeleton whole, the first few stretches decoded. What is
    // asserted is what holds for any input — the stretches partition the section's bytes
    // in order and a gap lies inside its stretch; the agreement with the symbol's own
    // listing is a claim tested where the objects are honest, in `listing.rs`.
    for section in &object.sections {
        let listing = Listing::new(&object, section.clone());
        let stretches = listing.stretches();
        let end = section
            .address
            .saturating_add(section.data.len().try_into().unwrap_or(u64::MAX));
        // No bytes, or none with room in the address space: nothing to list.
        if section.address >= end {
            assert!(stretches.is_empty());
        } else {
            assert_eq!(
                stretches.first().map(|s| s.range.start),
                Some(section.address)
            );
            assert_eq!(stretches.last().map(|s| s.range.end), Some(end));
        }
        for (index, stretch) in stretches.iter().enumerate() {
            assert!(stretch.range.start < stretch.range.end);
            if let Some(next) = stretches.get(index + 1) {
                assert_eq!(stretch.range.end, next.range.start);
            }
            assert!(index == 0 || !stretch.symbols.is_empty());
            assert_eq!(listing.stretch_at(stretch.range.start), Some(index));
            if index < MAX_LISTING_STRETCHES {
                let decoded = listing.decode(&object, index).expect("a stretch decodes");
                if let Some(gap) = decoded.gap {
                    assert!(gap.range.start >= stretch.range.start);
                    assert!(gap.range.start < gap.range.end);
                    assert_eq!(gap.range.end, stretch.range.end);
                }
            }
        }
        assert_eq!(listing.stretch_at(end), None);
        assert_eq!(listing.decode(&object, stretches.len()).is_some(), false);
    }

    // And all of the code as one listing: the sections placed in order without overlap,
    // every stretch found again at its placed address. Nothing decoded — the per-section
    // walk above did that.
    let code = CodeListing::new(&object);
    let mut placed_end = None;
    for (index, placed) in code.sections().iter().enumerate() {
        let range = placed.range();
        assert!(range.start < range.end);
        assert!(placed_end.is_none_or(|end| end <= range.start));
        placed_end = Some(range.end);
        assert_eq!(code.section_of(placed.listing.section()), Some(index));
        for (stretch, s) in placed.listing.stretches().iter().enumerate() {
            let at = placed.place(s.range.start);
            assert!(range.contains(&at));
            assert_eq!(
                code.at(at),
                Some(Place {
                    section: index,
                    stretch
                })
            );
        }
        assert_ne!(code.at(range.end).map(|place| place.section), Some(index));
    }

    // The reverse direction, which builds a whole-object index the first time it is asked.
    // Every symbol's own file and line, so the lookup path is walked and not only the build,
    // plus a name no object can hold. What is asserted is only what holds for *any* input:
    // that the answer is made of this object's own symbols. The round trip — every line a
    // symbol names finding that symbol again — is a claim about honest DWARF and is asserted
    // where the DWARF is honest, in `source_index.rs` and `real_object.rs`.
    for symbol in &object.symbols_sorted {
        let Some(info) = symbol.line_info(&object) else {
            continue;
        };
        let Some((file, line)) = info.rows().iter().find_map(|row| {
            let file = info.file_of(row)?;
            Some((file.to_owned(), row.line?))
        }) else {
            continue;
        };

        for found in object.symbols_at_line(&file, line) {
            assert!(
                object
                    .symbols_sorted
                    .iter()
                    .any(|known| Arc::ptr_eq(known, &found)),
                "{file}:{line} answered with a symbol this object does not have"
            );
        }
        // A range and the line inside it ask the same thing.
        assert_eq!(
            object
                .symbols_from_source(&file, line..line.saturating_add(1))
                .len(),
            object.symbols_at_line(&file, line).len()
        );
    }
    assert!(object.symbols_at_line("\u{0}no such file", 1).is_empty());

    Some(object)
}

/// Run `parse_and_walk` on every input, returning the labels of the ones that panicked.
pub fn survivors<'a>(inputs: impl IntoIterator<Item = (String, &'a [u8])>) -> Vec<String> {
    inputs
        .into_iter()
        .filter_map(|(label, data)| {
            catch_unwind(AssertUnwindSafe(|| parse_and_walk(data)))
                .err()
                .map(|_| label)
        })
        .collect()
}

pub struct TextSymbol<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// A relocation inside the generated `.text`, at `offset` within `in_symbol`.
pub struct TextRelocation {
    pub in_symbol: usize,
    pub offset: u64,
    pub target: usize,
}

/// A minimal x86-64 ELF relocatable object whose `.text` holds `symbols` back to back.
pub fn elf_x86_64(symbols: &[TextSymbol], relocations: &[TextRelocation]) -> Vec<u8> {
    elf_text(Architecture::X86_64, symbols, relocations)
}

/// The same fixture for any architecture: the only difference between an object that
/// decodes as 32-bit x86 and one that decodes as aarch64 is the `e_machine` in its header.
/// `relocations` are written with x86's branch encoding, so anything else must pass none.
pub fn elf_text(
    architecture: Architecture,
    symbols: &[TextSymbol],
    relocations: &[TextRelocation],
) -> Vec<u8> {
    elf_text_padded(architecture, &[], symbols, relocations)
}

/// [`elf_text`] with `leading` bytes at the start of `.text` that no symbol names: the one
/// shape the symbol-by-symbol builder cannot make, since every byte it appends is a symbol's.
pub fn elf_text_padded(
    architecture: Architecture,
    leading: &[u8],
    symbols: &[TextSymbol],
    relocations: &[TextRelocation],
) -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, architecture, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    if !leading.is_empty() {
        obj.append_section_data(text, leading, 1);
    }

    let mut offsets = Vec::new();
    let mut ids = Vec::new();

    for symbol in symbols {
        let offset = obj.append_section_data(text, symbol.bytes, 1);
        offsets.push(offset);
        ids.push(obj.add_symbol(write::Symbol {
            name: symbol.name.as_bytes().to_vec(),
            value: offset,
            // Deliberately 0: object files frequently report no size at all.
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        }));
    }

    for relocation in relocations {
        obj.add_relocation(
            text,
            write::Relocation {
                offset: offsets[relocation.in_symbol] + relocation.offset,
                size: 32,
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                symbol: ids[relocation.target],
                addend: -4,
            },
        )
        .expect("adding a relocation to .text");
    }

    obj.write().expect("writing the fixture object")
}

/// An x86-64 **COFF** relocatable object whose `.text` holds `symbols` back to back, each an
/// `IMAGE_SYM_CLASS_EXTERNAL` function whose auxiliary function-definition record declares
/// the given `TotalSize` — the one nonzero size `object` reads out of a COFF symbol.
/// Assembled byte by byte because `object`'s COFF writer emits no auxiliary function
/// records, which is the whole point of the fixture. Names have to fit the 8 bytes a symbol
/// entry holds inline, so nothing here needs a string table.
pub fn coff_x86_64(symbols: &[(TextSymbol, u32)]) -> Vec<u8> {
    const HEADER: usize = 20;
    const SECTION_HEADER: usize = 40;
    /// One symbol table entry, and one auxiliary record.
    const SYMBOL: usize = 18;

    let text: Vec<u8> = symbols
        .iter()
        .flat_map(|(symbol, _)| symbol.bytes)
        .copied()
        .collect();
    let symtab = HEADER + SECTION_HEADER + text.len();

    let mut file = Vec::new();
    file.extend_from_slice(&0x8664u16.to_le_bytes()); // Machine
    file.extend_from_slice(&1u16.to_le_bytes()); // NumberOfSections
    file.extend_from_slice(&0u32.to_le_bytes()); // TimeDateStamp
    file.extend_from_slice(&(symtab as u32).to_le_bytes()); // PointerToSymbolTable
    let entries = (symbols.len() * 2) as u32; // Each symbol plus its auxiliary record.
    file.extend_from_slice(&entries.to_le_bytes()); // NumberOfSymbols
    file.extend_from_slice(&0u16.to_le_bytes()); // SizeOfOptionalHeader
    file.extend_from_slice(&0u16.to_le_bytes()); // Characteristics

    file.extend_from_slice(b".text\0\0\0"); // Name
    file.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
    file.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    file.extend_from_slice(&(text.len() as u32).to_le_bytes()); // SizeOfRawData
    file.extend_from_slice(&((HEADER + SECTION_HEADER) as u32).to_le_bytes()); // PointerToRawData
    file.extend_from_slice(&0u32.to_le_bytes()); // PointerToRelocations
    file.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumbers
    file.extend_from_slice(&0u16.to_le_bytes()); // NumberOfRelocations
    file.extend_from_slice(&0u16.to_le_bytes()); // NumberOfLinenumbers

    // Characteristics: IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ.
    file.extend_from_slice(&0x6000_0020u32.to_le_bytes());

    file.extend_from_slice(&text);

    let mut offset = 0u32;
    for (symbol, total_size) in symbols {
        let bytes = symbol.name.as_bytes();
        assert!(bytes.len() <= 8, "`{}` needs a string table", symbol.name);
        let mut name = [0u8; 8];
        name[..bytes.len()].copy_from_slice(bytes);

        file.extend_from_slice(&name); // Name
        file.extend_from_slice(&offset.to_le_bytes()); // Value
        file.extend_from_slice(&1i16.to_le_bytes()); // SectionNumber, one-based
        file.extend_from_slice(&0x20u16.to_le_bytes()); // IMAGE_SYM_DTYPE_FUNCTION << 4
        file.push(2); // IMAGE_SYM_CLASS_EXTERNAL
        file.push(1); // NumberOfAuxSymbols

        file.extend_from_slice(&0u32.to_le_bytes()); // TagIndex
        file.extend_from_slice(&total_size.to_le_bytes()); // TotalSize
        file.extend_from_slice(&0u32.to_le_bytes()); // PointerToLinenumber
        file.extend_from_slice(&0u32.to_le_bytes()); // PointerToNextFunction
        file.extend_from_slice(&0u16.to_le_bytes()); // Unused

        offset += symbol.bytes.len() as u32;
    }

    // A string table of its own length alone, which is what "empty" is written as.
    file.extend_from_slice(&4u32.to_le_bytes());
    debug_assert_eq!(symtab + symbols.len() * 2 * SYMBOL + 4, file.len());
    file
}

/// `caller` = `call rel32; ret`, relocated at offset 1 against `target` = `ret`.
pub fn caller_and_target() -> Vec<u8> {
    elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 1,
            target: 1,
        }],
    )
}

/// `caller` = `call qword ptr [rip+0x0]; ret`, where the relocation applies to a
/// rip-relative **memory** operand rather than to the whole branch target. The
/// displacement starts at offset 2. `relocated` unset is the control: the same bytes with
/// no relocation on them.
pub fn indirect_caller_and_target(relocated: bool) -> Vec<u8> {
    elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xFF, 0x15, 0x00, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        if relocated {
            &[TextRelocation {
                in_symbol: 0,
                offset: 2,
                target: 1,
            }]
        } else {
            &[]
        },
    )
}

/// `jumper` = `jmp rel32; ret`, with the branch relocated against a **data** symbol —
/// which parsing drops, so the instruction's `relocation` is [`None`] while its
/// displacement is still a placeholder. Read literally the jump lands on address 5, this
/// symbol's own `ret`.
pub fn branch_to_data() -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    let text = obj.section_id(write::StandardSection::Text);
    let offset = obj.append_section_data(text, &[0xE9, 0x00, 0x00, 0x00, 0x00, 0xC3], 1);
    obj.add_symbol(write::Symbol {
        name: b"jumper".to_vec(),
        value: offset,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    let data = obj.section_id(write::StandardSection::Data);
    let value = obj.append_section_data(data, &[0; 4], 4);
    let counter = obj.add_symbol(write::Symbol {
        name: b"counter".to_vec(),
        value,
        size: 4,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(data),
        flags: SymbolFlags::None,
    });

    obj.add_relocation(
        text,
        write::Relocation {
            offset: offset + 1,
            size: 32,
            kind: RelocationKind::Relative,
            encoding: RelocationEncoding::X86Branch,
            symbol: counter,
            addend: -4,
        },
    )
    .expect("adding a relocation to .text");

    obj.write().expect("writing the fixture object")
}

/// Deterministic pseudo-random bytes (xorshift64*), so a failure is reproducible from its
/// seed alone — never `rand`, never the clock.
pub fn garbage(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// One row of a fixture's line program.
pub struct DwarfRow {
    pub address: u64,
    /// An index into [`DwarfFixture::files`].
    pub file: usize,
    /// 0 means "no source line", which is what DWARF's line 0 says.
    pub line: u64,
    /// 0 means the "left edge" of the line, i.e. no column.
    pub column: u64,
}

/// One code section of a fixture, its symbols laid out back to back, and the one line
/// program sequence describing them.
///
/// Several of these is the shape rustc emits: one `.text.<name>` per function, **every one
/// at address 0**, since a section in a relocatable object has no address until it is
/// linked — the case an address alone cannot key.
pub struct DwarfSection<'a> {
    /// [`None`] for the standard `.text`; [`Some`] for a section of its own (`.text.first`).
    pub name: Option<&'a str>,
    pub symbols: &'a [TextSymbol<'a>],
    /// Rows of this section's sequence, addressed from the section's own start.
    pub rows: &'a [DwarfRow],
    /// Where this section's sequence ends, as an offset into the section.
    pub length: u64,
    /// One `DW_TAG_subprogram` per entry, as `(index into `symbols`, extent in bytes)`:
    /// a stated `DW_AT_low_pc`/`DW_AT_high_pc` rather than a derived extent.
    pub subprograms: &'a [(usize, u64)],
    /// When set, an index into this section's `symbols`: addresses are written as zero
    /// with an absolute relocation against it, the way a compiler emits a relocatable
    /// object. When unset they are constants, as in a linked binary.
    pub base_symbol: Option<usize>,
}

pub struct DwarfFixture<'a> {
    pub comp_dir: &'a str,
    /// The source files the line programs can name; `DwarfRow::file` indexes this.
    pub files: &'a [&'a str],
    /// One section gives the unit a `DW_AT_low_pc`/`DW_AT_high_pc`; several give it a
    /// `DW_AT_ranges` list — see [`UnitRanges`] for how that list is written.
    pub sections: &'a [DwarfSection<'a>],
    /// How a multi-section unit states where its code is.
    pub unit_ranges: UnitRanges,
}

/// How [`DwarfFixture`] writes the `DW_AT_ranges` list of a unit spanning several sections.
/// Ignored by a single-section fixture, which states `DW_AT_low_pc`/`DW_AT_high_pc` instead.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UnitRanges {
    /// One entry per section, each an address and a length, and each address relocated
    /// against that section's own symbol — what gcc and rustc emit.
    Relocated,
    /// One entry per section, each a pair of **offsets** from the unit's `DW_AT_low_pc`,
    /// which is written as a literal 0 and carries no relocation. Nothing in the tree emits
    /// this, but DWARF permits it: `DW_RLE_offset_pair` beside an unrelocated base leaves the
    /// unit declaring a range that does not move when the line program's addresses do.
    OffsetPairs,
}

/// [`elf_x86_64`] plus a DWARF compilation unit and line program describing its code
/// sections. Addresses a compiler would relocate go through [`RelocWriter`], which records
/// where each landed, so the ELF carries the same relocations against the same symbols —
/// no byte pattern is searched for. Every symbol declares an `st_size` of 0, as
/// [`TextSymbol`] does everywhere else.
pub fn elf_x86_64_with_dwarf(fixture: DwarfFixture) -> Vec<u8> {
    elf_x86_64_with_dwarf_declaring(fixture, &[])
}

/// [`elf_x86_64_with_dwarf`] with an `st_size` per symbol, in the order the sections list
/// them and running on across section boundaries; a symbol past the end of `declared`
/// declares 0. The one thing [`TextSymbol`] cannot say, and the case where the symbol table
/// answers what DWARF would have been walked for.
pub fn elf_x86_64_with_dwarf_declaring(fixture: DwarfFixture, declared: &[u64]) -> Vec<u8> {
    use gimli::write::{
        Address, AttributeValue, DwarfUnit, LineProgram, LineString, Range, RangeList, Sections,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    // `Address::Symbol` indexes into this, and so does the relocation pass at the bottom.
    let mut symbols: Vec<write::SymbolId> = Vec::new();
    let mut bases: Vec<Option<usize>> = Vec::new();

    for section in fixture.sections {
        let id = match section.name {
            None => obj.section_id(write::StandardSection::Text),
            Some(name) => obj.add_section(Vec::new(), name.as_bytes().to_vec(), SectionKind::Text),
        };
        let first = symbols.len();
        for symbol in section.symbols {
            let offset = obj.append_section_data(id, symbol.bytes, 1);
            symbols.push(obj.add_symbol(write::Symbol {
                name: symbol.name.as_bytes().to_vec(),
                value: offset,
                size: declared.get(symbols.len()).copied().unwrap_or(0),
                kind: SymbolKind::Text,
                scope: SymbolScope::Linkage,
                weak: false,
                section: write::SymbolSection::Section(id),
                flags: SymbolFlags::None,
            }));
        }
        bases.push(section.base_symbol.map(|index| first + index));
    }

    // A relocation against one of the section's symbols, or a literal 0 as a linked image
    // has it.
    let address = |section: usize| match bases[section] {
        Some(symbol) => Address::Symbol { symbol, addend: 0 },
        None => Address::Constant(0),
    };

    let encoding = gimli::Encoding {
        format: gimli::Format::Dwarf32,
        version: 4,
        address_size: 8,
    };

    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        gimli::LineEncoding::default(),
        LineString::String(fixture.comp_dir.as_bytes().to_vec()),
        LineString::String(fixture.files[0].as_bytes().to_vec()),
        None,
    );
    let directory = program.default_directory();
    let files: Vec<_> = fixture
        .files
        .iter()
        .map(|file| {
            program.add_file(
                LineString::String(file.as_bytes().to_vec()),
                directory,
                None,
            )
        })
        .collect();

    let mut ranges = Vec::new();
    for (index, section) in fixture.sections.iter().enumerate() {
        program.begin_sequence(Some(address(index)));
        for row in section.rows {
            let current = program.row();
            current.address_offset = row.address;
            current.file = files[row.file];
            current.line = row.line;
            current.column = row.column;
            program.generate_row();
        }
        program.end_sequence(section.length);
        ranges.push(match fixture.unit_ranges {
            UnitRanges::Relocated => Range::StartLength {
                begin: address(index),
                length: section.length,
            },
            UnitRanges::OffsetPairs => Range::OffsetPair {
                begin: 0,
                end: section.length,
            },
        });
    }
    dwarf.unit.line_program = program;

    let root = dwarf.unit.root();
    let mut first = 0;
    for section in fixture.sections {
        for &(symbol, extent) in section.subprograms {
            let die = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
            let entry = dwarf.unit.get_mut(die);
            entry.set(
                gimli::DW_AT_name,
                AttributeValue::String(section.symbols[symbol].name.as_bytes().to_vec()),
            );
            entry.set(
                gimli::DW_AT_low_pc,
                AttributeValue::Address(Address::Symbol {
                    symbol: first + symbol,
                    addend: 0,
                }),
            );
            // The DWARF 4 spelling: a constant form on `DW_AT_high_pc` is a *length*.
            entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(extent));
        }
        first += section.symbols.len();
    }

    // Without a range on the unit, nothing will look inside it for an address.
    let range_list = (fixture.sections.len() > 1).then(|| dwarf.unit.ranges.add(RangeList(ranges)));
    let entry = dwarf.unit.get_mut(root);
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(fixture.comp_dir.as_bytes().to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(fixture.files[0].as_bytes().to_vec()),
    );
    match range_list {
        Some(list) => {
            entry.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(list));
            // A DWARF 4 range list holds offsets from the unit's base address, so
            // [`UnitRanges::Relocated`] must not also declare a `DW_AT_low_pc`: its entries
            // are the absolute addresses already, each one relocated on its own.
            // [`UnitRanges::OffsetPairs`] is the other spelling and needs the base, which it
            // states as an unrelocated 0.
            if fixture.unit_ranges == UnitRanges::OffsetPairs {
                entry.set(
                    gimli::DW_AT_low_pc,
                    AttributeValue::Address(Address::Constant(0)),
                );
            }
        }
        None => {
            entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(address(0)));
            entry.set(
                gimli::DW_AT_high_pc,
                AttributeValue::Udata(fixture.sections[0].length),
            );
        }
    }

    let mut sections = Sections::new(RelocWriter::default());
    dwarf.write(&mut sections).expect("writing the DWARF");

    sections
        .for_each(|id, writer| {
            if writer.slice().is_empty() {
                return Ok::<_, ()>(());
            }
            let section = obj.add_section(
                Vec::new(),
                id.name().as_bytes().to_vec(),
                SectionKind::Debug,
            );
            obj.append_section_data(section, writer.slice(), 1);

            for relocation in &writer.relocations {
                obj.add_relocation(
                    section,
                    write::Relocation {
                        offset: relocation.offset,
                        size: relocation.size * 8,
                        kind: RelocationKind::Absolute,
                        encoding: RelocationEncoding::Generic,
                        symbol: symbols[relocation.symbol],
                        addend: relocation.addend,
                    },
                )
                .expect("adding a relocation to a debug section");
            }
            Ok(())
        })
        .expect("laying out the DWARF sections");

    obj.write().expect("writing the fixture object")
}

/// The written half of the DWARF corpus: DWARF 4 with its strings inline, two functions
/// in one `.text` and three line-program rows over them. `subprograms` is what a caller
/// wanting stated `DW_AT_low_pc`/`DW_AT_high_pc` extents passes; empty is line info alone.
pub fn dwarf_fixture(subprograms: &[(usize, u64)]) -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: "/src",
        files: &["main.c", "other.c"],
        sections: &[DwarfSection {
            name: None,
            symbols: &[
                TextSymbol {
                    name: "first",
                    bytes: &[0x90, 0x90, 0x90, 0x90, 0x90, 0xC3],
                },
                TextSymbol {
                    name: "second",
                    bytes: &[0x90, 0xC3],
                },
            ],
            rows: &[
                DwarfRow {
                    address: 0,
                    file: 0,
                    line: 10,
                    column: 3,
                },
                DwarfRow {
                    address: 3,
                    file: 0,
                    line: 11,
                    column: 0,
                },
                DwarfRow {
                    address: 6,
                    file: 1,
                    line: 42,
                    column: 7,
                },
            ],
            length: 8,
            subprograms,
            base_symbol: Some(1),
        }],
        unit_ranges: UnitRanges::Relocated,
    })
}

#[derive(Clone)]
struct DebugRelocation {
    offset: u64,
    /// An index into the fixture's symbol table.
    symbol: usize,
    addend: i64,
    /// In bytes, as `gimli` writes it; ELF wants bits.
    size: u8,
}

/// A `gimli::write::Writer` that records relocations instead of refusing them: `EndianVec`
/// alone answers `Address::Symbol` with `Error::InvalidAddress`, which is exactly the form
/// a compiler emits into a relocatable object's `.debug_line` and `.debug_ranges`.
#[derive(Clone)]
struct RelocWriter {
    inner: gimli::write::EndianVec<gimli::LittleEndian>,
    relocations: Vec<DebugRelocation>,
}

impl Default for RelocWriter {
    fn default() -> Self {
        Self {
            inner: gimli::write::EndianVec::new(gimli::LittleEndian),
            relocations: Vec::new(),
        }
    }
}

impl RelocWriter {
    fn slice(&self) -> &[u8] {
        self.inner.slice()
    }
}

impl gimli::write::Writer for RelocWriter {
    type Endian = gimli::LittleEndian;

    fn endian(&self) -> Self::Endian {
        gimli::LittleEndian
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn write(&mut self, bytes: &[u8]) -> gimli::write::Result<()> {
        self.inner.write(bytes)
    }

    fn write_at(&mut self, offset: usize, bytes: &[u8]) -> gimli::write::Result<()> {
        self.inner.write_at(offset, bytes)
    }

    fn write_address(
        &mut self,
        address: gimli::write::Address,
        size: u8,
    ) -> gimli::write::Result<()> {
        match address {
            gimli::write::Address::Constant(value) => self.write_udata(value, size),
            gimli::write::Address::Symbol { symbol, addend } => {
                self.relocations.push(DebugRelocation {
                    offset: self.len() as u64,
                    symbol,
                    addend,
                    size,
                });
                self.write_udata(0, size)
            }
        }
    }
}

/// `storer` = `mov dword ptr [rip+0x0], 7; ret`, relocated at offset 2 against a **data**
/// symbol — which parsing drops, so the relocation is on the instruction and yet resolves
/// to nothing navigable.
pub fn rip_relative_store_to_data() -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    let text = obj.section_id(write::StandardSection::Text);
    let offset = obj.append_section_data(
        text,
        &[
            0xC7, 0x05, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xC3,
        ],
        1,
    );
    obj.add_symbol(write::Symbol {
        name: b"storer".to_vec(),
        value: offset,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    let data = obj.section_id(write::StandardSection::Data);
    let value = obj.append_section_data(data, &[0; 4], 4);
    let counter = obj.add_symbol(write::Symbol {
        name: b"counter".to_vec(),
        value,
        size: 4,
        kind: SymbolKind::Data,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(data),
        flags: SymbolFlags::None,
    });

    obj.add_relocation(
        text,
        write::Relocation {
            offset: offset + 2,
            size: 32,
            kind: RelocationKind::Relative,
            encoding: RelocationEncoding::Generic,
            symbol: counter,
            addend: -4,
        },
    )
    .expect("adding a relocation to .text");

    obj.write().expect("writing the fixture object")
}

/// One entry of a hand-built image's export or dynamic symbol table.
pub struct ExportedSymbol<'a> {
    pub name: &'a str,
    /// An offset into the fixture's `.text`, not a virtual address.
    pub offset: u64,
    /// What the declaration itself claims — always 0 for a PE export, whose table has no
    /// room for a size.
    pub size: u64,
    /// When false the symbol is written as data (ELF `STT_OBJECT`, or a PE export whose
    /// address is in `.rdata`), which must **not** come out as a text symbol.
    pub code: bool,
}

/// Where a hand-built image puts its code: a page in, at a non-zero image base.
const IMAGE_BASE: u64 = 0x1_4000_0000;
const TEXT_RVA: u64 = 0x1000;
/// What an exported `offset` is relative to.
pub const TEXT_ADDRESS: u64 = IMAGE_BASE + TEXT_RVA;

pub struct SharedObject<'a> {
    pub text: &'a [u8],
    /// Written to `.dynsym`, the table a stripped library still has.
    pub dynamic: &'a [ExportedSymbol<'a>],
    /// Written to `.symtab`, the table `strip` removes. Empty is the stripped case;
    /// filling both is how a file declaring one function twice is built.
    pub static_symbols: &'a [ExportedSymbol<'a>],
    /// An offset into `.text`, or [`None`] for an image that declares no entry point.
    pub entry: Option<u64>,
    /// Written to `.eh_frame` as one FDE each, `(begin, end)` offsets into `.text` — allowed
    /// past its end, so a test can state a function where there is no code — and empty for
    /// an image without an unwind table.
    pub eh_frame: &'a [(u64, u64)],
}

/// The `.eh_frame` an ELF image carries, as `gcc` writes one: a `zR` CIE whose FDE addresses
/// are `pcrel|sdata4`, an FDE per range, and the zero-length terminator. Through `gimli`'s
/// writer, whose pc-relative encoding subtracts only its own offset into the section, so
/// the address handed to it is made relative to the section's `address` first; the
/// terminator is appended by hand, since the writer leaves it out.
fn eh_frame_section(address: u64, ranges: &[(u64, u64)]) -> Vec<u8> {
    use gimli::write::{
        Address, CommonInformationEntry, EhFrame, EndianVec, FrameDescriptionEntry, FrameTable,
    };
    use gimli::{Encoding, Format, LittleEndian, Register};

    let mut table = FrameTable::default();
    let mut cie = CommonInformationEntry::new(
        Encoding {
            format: Format::Dwarf32,
            version: 1,
            address_size: 8,
        },
        1,
        -8,
        Register(16),
    );
    cie.fde_address_encoding = gimli::DwEhPe(gimli::DW_EH_PE_pcrel.0 | gimli::DW_EH_PE_sdata4.0);
    let cie = table.add_cie(cie);
    for &(begin, end) in ranges {
        let function = TEXT_ADDRESS + begin;
        table.add_fde(
            cie,
            FrameDescriptionEntry::new(
                Address::Constant(function.wrapping_sub(address)),
                (end - begin) as u32,
            ),
        );
    }
    let mut section = EhFrame(EndianVec::new(LittleEndian));
    table
        .write_eh_frame(&mut section)
        .expect("writing the fixture's .eh_frame");
    let mut bytes = section.0.into_vec();
    bytes.extend_from_slice(&[0; 4]);
    bytes
}

/// An x86-64 ELF **shared object** (`ET_DYN`), assembled byte by byte because `object`'s
/// writer emits `ET_REL` relocatable objects and cannot write a dynamic symbol table —
/// which is the shape being tested: a stripped `.so` has no `.symtab` at all. With
/// `eh_frame` ranges, an `.eh_frame` section too ([`eh_frame_section`]), last, so an image
/// without one is the eight-section one byte for byte.
pub fn elf_shared_object(fixture: SharedObject) -> Vec<u8> {
    const SHDR: usize = 64;
    const EHDR: usize = 64;
    const SYM: usize = 24;

    // Section indices, in the order they are written below; `.eh_frame`, when there is
    // one, is the ninth.
    const TEXT: u16 = 1;
    const DATA: u16 = 2;
    const SHSTRTAB: u16 = 7;
    const SECTIONS: u16 = 8;

    let SharedObject {
        text,
        dynamic,
        static_symbols,
        entry,
        eh_frame,
    } = fixture;

    // `.data` exists only so a data symbol has somewhere to be that is not code.
    let data = [0u8; 8];
    let data_rva = TEXT_RVA + text.len() as u64 + 0x1000;
    // A page past `.data`: decided up front, since the FDEs are written relative to it.
    let eh_frame_rva = data_rva + 0x1000;
    let eh_frame_bytes = if eh_frame.is_empty() {
        Vec::new()
    } else {
        eh_frame_section(IMAGE_BASE + eh_frame_rva, eh_frame)
    };

    // The entries start with the null entry every ELF symbol table has.
    let table = |symbols: &[ExportedSymbol]| {
        let mut strings = vec![0u8];
        let mut entries = vec![0u8; SYM];
        for symbol in symbols {
            let name = strings.len() as u32;
            strings.extend_from_slice(symbol.name.as_bytes());
            strings.push(0);

            let (info, shndx, value) = if symbol.code {
                // STB_GLOBAL << 4 | STT_FUNC
                (0x12u8, TEXT, IMAGE_BASE + TEXT_RVA + symbol.offset)
            } else {
                // STB_GLOBAL << 4 | STT_OBJECT
                (0x11u8, DATA, IMAGE_BASE + data_rva + symbol.offset)
            };
            entries.extend_from_slice(&name.to_le_bytes());
            entries.push(info);
            entries.push(0); // st_other
            entries.extend_from_slice(&shndx.to_le_bytes());
            entries.extend_from_slice(&value.to_le_bytes());
            entries.extend_from_slice(&symbol.size.to_le_bytes());
        }
        (entries, strings)
    };
    let (dynsym, dynstr) = table(dynamic);
    let (symtab, strtab) = table(static_symbols);

    let mut shstrtab = vec![0u8];
    let mut section_name = |name: &str| {
        let offset = shstrtab.len() as u32;
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
        offset
    };
    let names = [
        section_name(".text"),
        section_name(".data"),
        section_name(".dynsym"),
        section_name(".dynstr"),
        section_name(".symtab"),
        section_name(".strtab"),
        section_name(".shstrtab"),
    ];
    let eh_frame_name = (!eh_frame.is_empty()).then(|| section_name(".eh_frame"));

    let mut out = vec![0u8; EHDR];
    let place = |out: &mut Vec<u8>, bytes: &[u8]| {
        let offset = out.len() as u64;
        out.extend_from_slice(bytes);
        (offset, bytes.len() as u64)
    };
    let text_at = place(&mut out, text);
    let data_at = place(&mut out, &data);
    let dynsym_at = place(&mut out, &dynsym);
    let dynstr_at = place(&mut out, &dynstr);
    let symtab_at = place(&mut out, &symtab);
    let strtab_at = place(&mut out, &strtab);
    let shstrtab_at = place(&mut out, &shstrtab);
    let eh_frame_at = (!eh_frame_bytes.is_empty()).then(|| place(&mut out, &eh_frame_bytes));
    let shoff = out.len() as u64;

    // sh_name, sh_type, sh_flags, sh_addr, (sh_offset, sh_size), sh_link, sh_entsize.
    // sh_info is 1 for a symbol table (one local symbol, the null entry) and 0
    // otherwise; sh_addralign is always 1 here.
    let shdr =
        |name: u32, kind: u32, flags: u64, addr: u64, at: (u64, u64), link: u32, entsize: u64| {
            let mut bytes = Vec::with_capacity(SHDR);
            bytes.extend_from_slice(&name.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&flags.to_le_bytes());
            bytes.extend_from_slice(&addr.to_le_bytes());
            bytes.extend_from_slice(&at.0.to_le_bytes());
            bytes.extend_from_slice(&at.1.to_le_bytes());
            bytes.extend_from_slice(&link.to_le_bytes());
            bytes.extend_from_slice(&u32::from(entsize != 0).to_le_bytes());
            bytes.extend_from_slice(&1u64.to_le_bytes());
            bytes.extend_from_slice(&entsize.to_le_bytes());
            bytes
        };

    // SHT_PROGBITS = 1, SHT_SYMTAB = 2, SHT_STRTAB = 3, SHT_DYNSYM = 11.
    // SHF_WRITE = 1, SHF_ALLOC = 2, SHF_EXECINSTR = 4.
    out.extend_from_slice(&shdr(0, 0, 0, 0, (0, 0), 0, 0));
    out.extend_from_slice(&shdr(
        names[0],
        1,
        2 | 4,
        IMAGE_BASE + TEXT_RVA,
        text_at,
        0,
        0,
    ));
    out.extend_from_slice(&shdr(
        names[1],
        1,
        2 | 1,
        IMAGE_BASE + data_rva,
        data_at,
        0,
        0,
    ));
    out.extend_from_slice(&shdr(names[2], 11, 2, 0, dynsym_at, 4, SYM as u64));
    out.extend_from_slice(&shdr(names[3], 3, 2, 0, dynstr_at, 0, 0));
    out.extend_from_slice(&shdr(names[4], 2, 0, 0, symtab_at, 6, SYM as u64));
    out.extend_from_slice(&shdr(names[5], 3, 0, 0, strtab_at, 0, 0));
    out.extend_from_slice(&shdr(names[6], 3, 0, 0, shstrtab_at, 0, 0));
    if let (Some(name), Some(at)) = (eh_frame_name, eh_frame_at) {
        out.extend_from_slice(&shdr(name, 1, 2, IMAGE_BASE + eh_frame_rva, at, 0, 0));
    }
    let sections = SECTIONS + u16::from(eh_frame_at.is_some());

    // And the header, now that every offset is known. ET_DYN = 3, EM_X86_64 = 62.
    let header = &mut out[..EHDR];
    header[..4].copy_from_slice(b"\x7fELF");
    header[4] = 2; // ELFCLASS64
    header[5] = 1; // ELFDATA2LSB
    header[6] = 1; // EV_CURRENT
    header[16..18].copy_from_slice(&3u16.to_le_bytes()); // e_type
    header[18..20].copy_from_slice(&62u16.to_le_bytes()); // e_machine
    header[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    let entry = entry.map_or(0, |offset| IMAGE_BASE + TEXT_RVA + offset);
    header[24..32].copy_from_slice(&entry.to_le_bytes());
    header[40..48].copy_from_slice(&shoff.to_le_bytes());
    header[52..54].copy_from_slice(&(EHDR as u16).to_le_bytes()); // e_ehsize
    header[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
    header[58..60].copy_from_slice(&(SHDR as u16).to_le_bytes()); // e_shentsize
    header[60..62].copy_from_slice(&sections.to_le_bytes());
    header[62..64].copy_from_slice(&SHSTRTAB.to_le_bytes());

    out
}

/// The CodeView record a linker leaves in a PE's debug directory, naming the `.pdb` it wrote
/// beside the image and the identity (`guid`, `age`) that `.pdb` has to answer with.
pub struct CodeViewRecord<'a> {
    /// The 16 GUID bytes exactly as they sit in the file (Windows' mixed-endian layout).
    pub guid: [u8; 16],
    pub age: u32,
    /// The recorded path, as the linker wrote it: the build machine's, or a bare name.
    pub path: &'a str,
}

/// What [`pe_image`] is asked for. `entry` is an offset into `.text`, or [`None`] as in a
/// resource-only DLL; `codeview` is the debug directory's one record, or [`None`] for an
/// image built without `/DEBUG`; `unwind` is the exception directory's `RUNTIME_FUNCTION`s,
/// each a `(begin, end)` pair of offsets into `.text` — allowed past its end, so a test can
/// state a function where there is no code — and empty for an image without one; and
/// `fragments` are more of them whose `UNWIND_INFO` is **chained** (`UNW_FLAG_CHAININFO`),
/// the shape a cold part or a second prologue's range has, written after `unwind`'s.
pub struct PeDll<'a> {
    pub text: &'a [u8],
    pub symbols: &'a [ExportedSymbol<'a>],
    pub entry: Option<u64>,
    pub codeview: Option<CodeViewRecord<'a>>,
    pub unwind: &'a [(u64, u64)],
    pub fragments: &'a [(u64, u64)],
}

/// [`pe_image`] without a debug directory or unwind info, which is what every test before
/// the PDB backend asked for.
pub fn pe_dll(text: &[u8], symbols: &[ExportedSymbol], entry: Option<u64>) -> Vec<u8> {
    pe_image(PeDll {
        text,
        symbols,
        entry,
        codeview: None,
        unwind: &[],
        fragments: &[],
    })
}

/// An x86-64 PE **DLL** with an export directory and **no COFF symbol table**: the export
/// table is then the only thing naming any code. Hand-assembled for the reason the ELF
/// above is. With a [`CodeViewRecord`], `.rdata` also carries a debug directory of one
/// `IMAGE_DEBUG_TYPE_CODEVIEW` entry pointing at an `RSDS` record — the shape `object`'s
/// `pdb_info` reads — so a test can name any `.pdb` on disk from an image built in memory.
/// With `unwind` entries, a third section `.pdata` holds one 12-byte `RUNTIME_FUNCTION`
/// each and the exception data directory points at it, as `link.exe` lays an x86-64 image
/// out — a plain entry's unwind info pointing at zeroes, a fragment's at a chained
/// `UNWIND_INFO` in `.rdata`; without, the image is the two-section one byte for byte.
pub fn pe_image(dll: PeDll) -> Vec<u8> {
    let PeDll {
        text,
        symbols,
        entry,
        codeview,
        unwind,
        fragments,
    } = dll;
    const FILE_ALIGNMENT: usize = 0x200;
    const SECTION_ALIGNMENT: u64 = 0x1000;
    /// DOS stub, `PE\0\0`, COFF header, PE32+ optional header and up to three section
    /// headers, rounded up to one file-alignment unit — which everything below assumes fits.
    const HEADERS: usize = FILE_ALIGNMENT;

    let text_size = text.len();
    let text_raw = text_size.next_multiple_of(FILE_ALIGNMENT);
    let rdata_rva = TEXT_RVA + (text_size as u64).next_multiple_of(SECTION_ALIGNMENT);

    // The export directory, the three parallel arrays it points at, then the name
    // strings — all inside `.rdata`, in that order.
    let named: Vec<&ExportedSymbol> = symbols.iter().collect();
    let count = named.len() as u32;
    const DIRECTORY: u64 = 40;
    let functions_rva = rdata_rva + DIRECTORY;
    let names_rva = functions_rva + 4 * count as u64;
    let ordinals_rva = names_rva + 4 * count as u64;
    let strings_rva = ordinals_rva + 2 * count as u64;

    let mut strings = Vec::new();
    // The library's own name comes first, as a linker writes it.
    let mut string_rvas = Vec::new();
    let library_rva = strings_rva;
    strings.extend_from_slice(b"fixture.dll\0");
    for symbol in &named {
        string_rvas.push(strings_rva + strings.len() as u64);
        strings.extend_from_slice(symbol.name.as_bytes());
        strings.push(0);
    }

    // Past everything the export table occupies, so a data export is an address in a
    // section that is not code.
    let data_rva = strings_rva + strings.len() as u64 + 0x10;

    let mut rdata = Vec::new();
    let put32 = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
    put32(&mut rdata, 0); // Characteristics
    put32(&mut rdata, 0); // TimeDateStamp
    put32(&mut rdata, 0); // Major/MinorVersion
    put32(&mut rdata, library_rva as u32); // Name
    put32(&mut rdata, 1); // Base (the first ordinal)
    put32(&mut rdata, count); // NumberOfFunctions
    put32(&mut rdata, count); // NumberOfNames
    put32(&mut rdata, functions_rva as u32);
    put32(&mut rdata, names_rva as u32);
    put32(&mut rdata, ordinals_rva as u32);
    assert_eq!(rdata.len() as u64, DIRECTORY);

    for symbol in &named {
        let rva = if symbol.code {
            TEXT_RVA + symbol.offset
        } else {
            data_rva + symbol.offset
        };
        put32(&mut rdata, rva as u32);
    }
    for rva in &string_rvas {
        put32(&mut rdata, *rva as u32);
    }
    for index in 0..count as u16 {
        rdata.extend_from_slice(&index.to_le_bytes());
    }
    rdata.extend_from_slice(&strings);
    // Room for the data export to point at. A plain entry's unwind-info RVA points here
    // too: zeroes, which is an `UNWIND_INFO` of version 0 with no flags.
    rdata.resize(rdata.len() + 0x20, 0);

    // The `UNWIND_INFO` every fragment points at: version 1, `UNW_FLAG_CHAININFO`, no
    // codes, and the primary `RUNTIME_FUNCTION` a chained one ends with — the first plain
    // entry's, which nothing reads.
    let chained_rva = rdata_rva + rdata.len() as u64;
    if !fragments.is_empty() {
        rdata.extend_from_slice(&[0x21, 0, 0, 0]);
        let (begin, end) = unwind.first().copied().unwrap_or((0, 0));
        put32(&mut rdata, (TEXT_RVA + begin) as u32);
        put32(&mut rdata, (TEXT_RVA + end) as u32);
        put32(&mut rdata, data_rva as u32);
    }

    // The debug directory — one 28-byte `IMAGE_DEBUG_DIRECTORY` — and the CodeView record it
    // points at, after everything the export table occupies. `object` reads the record by
    // its *file* offset (`PointerToRawData`), so both that and the RVA are filled in.
    let mut debug_directory = None;
    if let Some(record) = codeview {
        rdata.resize(rdata.len().next_multiple_of(4), 0);
        let directory = rdata.len();
        rdata.resize(directory + 28, 0);
        let cv = rdata.len();
        rdata.extend_from_slice(b"RSDS");
        rdata.extend_from_slice(&record.guid);
        put32(&mut rdata, record.age);
        rdata.extend_from_slice(record.path.as_bytes());
        rdata.push(0);
        let cv_size = (rdata.len() - cv) as u32;

        let entry = &mut rdata[directory..directory + 28];
        entry[12..16].copy_from_slice(&2u32.to_le_bytes()); // IMAGE_DEBUG_TYPE_CODEVIEW
        entry[16..20].copy_from_slice(&cv_size.to_le_bytes()); // SizeOfData
        entry[20..24].copy_from_slice(&((rdata_rva as usize + cv) as u32).to_le_bytes());
        entry[24..28].copy_from_slice(&((HEADERS + text_raw + cv) as u32).to_le_bytes());
        debug_directory = Some(rdata_rva as usize + directory);
    }

    let rdata_size = rdata.len();
    let rdata_raw = rdata_size.next_multiple_of(FILE_ALIGNMENT);

    // The unwind table: one `RUNTIME_FUNCTION` per entry — begin RVA, end RVA, and the RVA
    // of its `UNWIND_INFO`, which nothing here reads and so points at the hole `.rdata`
    // keeps for the data export. In a `.pdata` of its own after `.rdata`, as a linker
    // places it.
    let pdata_rva = rdata_rva + (rdata_size as u64).next_multiple_of(SECTION_ALIGNMENT);
    let mut pdata = Vec::new();
    for (entries, info_rva) in [(unwind, data_rva), (fragments, chained_rva)] {
        for &(begin, end) in entries {
            put32(&mut pdata, (TEXT_RVA + begin) as u32);
            put32(&mut pdata, (TEXT_RVA + end) as u32);
            put32(&mut pdata, info_rva as u32);
        }
    }
    let pdata_size = pdata.len();
    let pdata_raw = pdata_size.next_multiple_of(FILE_ALIGNMENT);
    let section_count: u16 = if pdata.is_empty() { 2 } else { 3 };
    let image_end = if pdata.is_empty() {
        rdata_rva + rdata_size as u64
    } else {
        pdata_rva + pdata_size as u64
    };
    let image_size = image_end.next_multiple_of(SECTION_ALIGNMENT);

    let mut out = vec![0u8; HEADERS + text_raw + rdata_raw + pdata_raw];
    out[HEADERS..HEADERS + text_size].copy_from_slice(text);
    out[HEADERS + text_raw..HEADERS + text_raw + rdata_size].copy_from_slice(&rdata);
    let pdata_pointer = HEADERS + text_raw + rdata_raw;
    out[pdata_pointer..pdata_pointer + pdata_size].copy_from_slice(&pdata);

    out[..2].copy_from_slice(b"MZ");
    out[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes()); // e_lfanew
    out[0x40..0x44].copy_from_slice(b"PE\0\0");

    // COFF header at 0x44: Machine, NumberOfSections, TimeDateStamp,
    // PointerToSymbolTable, NumberOfSymbols, SizeOfOptionalHeader, Characteristics.
    let coff = &mut out[0x44..0x58];
    coff[0..2].copy_from_slice(&0x8664u16.to_le_bytes());
    coff[2..4].copy_from_slice(&section_count.to_le_bytes());
    // PointerToSymbolTable and NumberOfSymbols stay 0: the point of the fixture.
    coff[16..18].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
    coff[18..20].copy_from_slice(&0x2022u16.to_le_bytes()); // EXECUTABLE | LARGE_ADDRESS | DLL

    // PE32+ optional header at 0x58.
    let opt = &mut out[0x58..0x58 + 240];
    opt[0..2].copy_from_slice(&0x20bu16.to_le_bytes()); // PE32+
    opt[16..20].copy_from_slice(&(entry.map_or(0, |o| TEXT_RVA + o) as u32).to_le_bytes());
    opt[20..24].copy_from_slice(&(TEXT_RVA as u32).to_le_bytes()); // BaseOfCode
    opt[24..32].copy_from_slice(&IMAGE_BASE.to_le_bytes());
    opt[32..36].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
    opt[36..40].copy_from_slice(&(FILE_ALIGNMENT as u32).to_le_bytes());
    opt[56..60].copy_from_slice(&(image_size as u32).to_le_bytes());
    opt[60..64].copy_from_slice(&(HEADERS as u32).to_le_bytes()); // SizeOfHeaders
    opt[108..112].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes
                                                         // Data directory 0 is the export table.
    opt[112..116].copy_from_slice(&(rdata_rva as u32).to_le_bytes());
    opt[116..120].copy_from_slice(&(rdata_size as u32).to_le_bytes());
    // Data directory 3 is the exception directory: the whole of `.pdata`, when there is one.
    if !pdata.is_empty() {
        opt[136..140].copy_from_slice(&(pdata_rva as u32).to_le_bytes());
        opt[140..144].copy_from_slice(&(pdata_size as u32).to_le_bytes());
    }
    // Data directory 6 is the debug directory: one entry, when there is one.
    if let Some(directory) = debug_directory {
        opt[160..164].copy_from_slice(&(directory as u32).to_le_bytes());
        opt[164..168].copy_from_slice(&28u32.to_le_bytes());
    }

    // Section headers at 0x58 + 240 = 0x148.
    let headers = 0x148;
    let section = |name: &[u8],
                   rva: u64,
                   virtual_size: usize,
                   pointer: usize,
                   raw: usize,
                   characteristics: u32| {
        let mut bytes = vec![0u8; 40];
        bytes[..name.len()].copy_from_slice(name);
        bytes[8..12].copy_from_slice(&(virtual_size as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&(rva as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(raw as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(pointer as u32).to_le_bytes());
        bytes[36..40].copy_from_slice(&characteristics.to_le_bytes());
        bytes
    };
    // CNT_CODE | MEM_EXECUTE | MEM_READ, and CNT_INITIALIZED_DATA | MEM_READ.
    let text_header = section(
        b".text",
        TEXT_RVA,
        text_size,
        HEADERS,
        text_raw,
        0x6000_0020,
    );
    let rdata_header = section(
        b".rdata",
        rdata_rva,
        rdata_size,
        HEADERS + text_raw,
        rdata_raw,
        0x4000_0040,
    );
    out[headers..headers + 40].copy_from_slice(&text_header);
    out[headers + 40..headers + 80].copy_from_slice(&rdata_header);
    if !pdata.is_empty() {
        let pdata_header = section(
            b".pdata",
            pdata_rva,
            pdata_size,
            pdata_pointer,
            pdata_raw,
            0x4000_0040,
        );
        out[headers + 80..headers + 120].copy_from_slice(&pdata_header);
    }

    out
}

/// The images `declared_code` reads, each with the number of symbols it declares: a stripped
/// ELF `.so` whose only symbol table is `.dynsym`, and a PE DLL whose declarations are its
/// export directory, its entry point and its unwind table — three entries in either table,
/// two of them on the export and the entry point and the third on a function nothing names.
/// An `.o` declares none of these, so a corpus of relocatable objects leaves the export,
/// entry-point and unwind paths unexercised entirely.
pub fn declared_code_images() -> Vec<(&'static str, Vec<u8>, usize)> {
    const TEXT: &[u8] = &[0x90, 0x90, 0x90, 0xC3, 0x90, 0xC3, 0xC3];
    const UNWIND: &[(u64, u64)] = &[(0, 4), (4, 6), (6, 7)];
    const SYMBOLS: &[ExportedSymbol] = &[
        ExportedSymbol {
            name: "first",
            offset: 0,
            size: 4,
            code: true,
        },
        ExportedSymbol {
            name: "a_global",
            offset: 0,
            size: 8,
            code: false,
        },
    ];

    vec![
        (
            "elf .so",
            elf_shared_object(SharedObject {
                text: TEXT,
                dynamic: SYMBOLS,
                static_symbols: &[],
                entry: Some(4),
                eh_frame: UNWIND,
            }),
            3,
        ),
        (
            "pe dll",
            pe_image(PeDll {
                text: TEXT,
                symbols: SYMBOLS,
                entry: Some(4),
                codeview: None,
                unwind: UNWIND,
                fragments: &[],
            }),
            3,
        ),
        // The same DLL naming a `.pdb` that is nowhere on disk: the debug directory is read
        // and the search comes back empty, which is the common case for a stripped image.
        (
            "pe dll naming a pdb",
            pe_image(PeDll {
                text: TEXT,
                symbols: SYMBOLS,
                entry: Some(4),
                codeview: Some(CodeViewRecord {
                    guid: *b"0123456789abcdef",
                    age: 3,
                    path: "C:\\build\\fixture.pdb",
                }),
                unwind: UNWIND,
                fragments: &[],
            }),
            3,
        ),
    ]
}

/// A GNU `ar` archive holding `members`: the `object` writer cannot produce one.
pub fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut file = b"!<arch>\n".to_vec();
    for (name, data) in members {
        file.extend_from_slice(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                format!("{name}/"),
                0,
                0,
                0,
                644,
                data.len()
            )
            .as_bytes(),
        );
        file.extend_from_slice(data);
        // Members are two-byte aligned.
        if data.len() % 2 == 1 {
            file.push(b'\n');
        }
    }
    file
}

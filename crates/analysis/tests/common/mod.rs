//! In-memory fixture builders: every test object is assembled with the `object` and
//! `gimli` writers rather than read off disk.

#![allow(dead_code)]

use analysis::{
    parse_object, CodeListing, Instruction, LineInfo, LineRow, Listing, Object, Operand, Placed,
    PlacedAddress, Section, SectionAddress, SymbolData,
};
use object::write;
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SectionKind, SymbolFlags, SymbolKind, SymbolScope,
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

/// An address in a section's own terms, written as the number a test means by it: what a
/// fixture's sections, symbols and listings are all laid out in.
pub fn at(address: u64) -> SectionAddress {
    SectionAddress::new(address)
}

/// The same in the one space an object's code sections share, which is where a fixture's
/// biases put them.
pub fn placed_at(address: u64) -> PlacedAddress {
    PlacedAddress::new(address)
}

/// The file a row of `info` names, as a string to compare against.
pub fn file_of<'a>(info: &'a LineInfo, row: &LineRow) -> Option<&'a str> {
    info.file(row.file?).map(|name| &**name)
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

/// The address an instruction goes to that nothing has named: a branch's own, or an unnamed
/// call's.
pub fn goes_to(instruction: &Instruction) -> Option<SectionAddress> {
    match instruction.operand {
        Some(Operand::Branch { address, .. } | Operand::Call { address, .. }) => Some(address),
        _ => None,
    }
}

/// One section's listing, reached as the crate hands it out: through the object's
/// [`CodeListing`]. Derefs to the [`Listing`].
pub struct SectionListing {
    code: CodeListing,
    index: usize,
}

impl SectionListing {
    /// The section as the whole listing placed it.
    pub fn placed(&self) -> &Placed {
        &self.code.sections()[self.index]
    }

    /// Where the section's bytes stop, in its own addresses.
    pub fn end(&self) -> SectionAddress {
        let placed = self.placed();
        placed.local(placed.range().end)
    }
}

impl std::ops::Deref for SectionListing {
    type Target = Listing;

    fn deref(&self) -> &Listing {
        &self.placed().listing
    }
}

/// `section`'s listing. Panics for a section the [`CodeListing`] leaves out.
pub fn listing_of(object: &Object, section: &Section) -> SectionListing {
    let code = CodeListing::new(object);
    let index = code
        .section_of(section)
        .unwrap_or_else(|| panic!("section {} is listed", section.name));
    SectionListing { code, index }
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
        let _ = symbol.estimate_size(&object);
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
            let mut previous = at(0);
            for row in info.rows() {
                assert!(row.range.start >= previous && row.range.start < row.range.end);
                previous = row.range.end;
                assert_eq!(
                    info.row_at(row.range.start)
                        .map(|found| found.range.clone()),
                    Some(row.range.clone())
                );
                let _ = row.file.and_then(|file| info.file(file));
            }
            let _ = info
                .row_at(at(u64::MAX))
                .and_then(|row| info.file(row.file?));
        }
    }
    // Build the DWARF context even for an object whose symbols were all dropped.
    for section in &object.sections {
        let _ = object.line_info(section, at(0)..at(u64::MAX));
    }

    // All of the code as one listing: the sections placed in order without overlap, each
    // section's stretches partitioning its bytes in order, every stretch found again at its
    // placed address, and the first few of each decoded with a gap inside its stretch. What
    // is asserted holds for any input; the agreement with the symbol's own listing is a
    // claim tested where the objects are honest, in `listing.rs`.
    let code = CodeListing::new(&object);
    let mut placed_end = None;
    let mut flat = 0;
    for (index, placed) in code.sections().iter().enumerate() {
        let range = placed.range();
        assert!(range.start < range.end);
        assert!(placed_end.is_none_or(|end| end <= range.start));
        placed_end = Some(range.end);
        let listing = &placed.listing;
        assert_eq!(code.section_of(listing.section()), Some(index));

        let stretches = listing.stretches();
        let end = placed.local(range.end);
        assert_eq!(
            stretches.first().map(|s| s.range.start),
            Some(listing.section().address)
        );
        assert_eq!(stretches.last().map(|s| s.range.end), Some(end));
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
            let at = placed.place(stretch.range.start);
            assert!(range.contains(&at));
            assert_eq!(code.at(at), Some(flat));
            assert!(std::ptr::eq(
                code.stretch(flat).expect("the stretch").1,
                stretch
            ));
            flat += 1;
        }
        assert_eq!(listing.stretch_at(end), None);
        assert!(listing.decode(&object, stretches.len()).is_none());
        // The air past the section's last byte is in no stretch of it.
        assert!(code.at(range.end).is_none_or(|at| at >= flat));
    }
    assert_eq!(code.stretch_count(), flat);

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
            let file = info.file(row.file?)?;
            Some((file.clone(), row.line?))
        }) else {
            continue;
        };

        for found in object.symbols_from_lines(&file, line..=line) {
            assert!(
                object
                    .symbols_sorted
                    .iter()
                    .any(|known| Arc::ptr_eq(known, &found)),
                "{file}:{line} answered with a symbol this object does not have"
            );
        }
        // A range holding the line answers with everything the line does.
        let range = object.symbols_from_lines(&file, line..=line.saturating_add(1));
        for found in object.symbols_from_lines(&file, line..=line) {
            assert!(range.iter().any(|known| Arc::ptr_eq(known, &found)));
        }
    }
    assert!(object
        .symbols_from_lines("\u{0}no such file", 1..=1)
        .is_empty());

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
                symbol: ids[relocation.target],
                addend: -4,
                flags: RelocationFlags::Generic {
                    kind: RelocationKind::Relative,
                    encoding: RelocationEncoding::X86Branch,
                    size: 32,
                },
            },
        )
        .expect("adding a relocation to .text");
    }

    obj.write().expect("writing the fixture object")
}

/// One section's header in a written 64-bit little-endian ELF, as `(offset of the header in
/// the file, index of the section)`. Byte surgery, because `object`'s writer states no
/// address for a relocatable object's sections and offers no way to ask for one.
fn elf_section_header(data: &[u8], name: &str) -> (usize, u16) {
    let half = |at: usize| u16::from_le_bytes(data[at..at + 2].try_into().unwrap()) as usize;
    let word = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
    let addr = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap()) as usize;

    // `e_shoff`, `e_shentsize`, `e_shnum` and `e_shstrndx` of the ELF header, then the
    // section name table's own `sh_offset`.
    let table = addr(0x28);
    let entry = half(0x3a);
    let count = half(0x3c);
    let names = addr(table + half(0x3e) * entry + 0x18);

    for index in 0..count {
        let header = table + index * entry;
        let start = names + word(header);
        let end = start + data[start..].iter().position(|&byte| byte == 0).unwrap();
        if &data[start..end] == name.as_bytes() {
            return (header, index as u16);
        }
    }
    panic!("no section named {name}");
}

/// Give one section of a written ELF an address of its own, moving every symbol defined in
/// it to match: `st_value` in a relocatable object is an offset from the section's start.
///
/// The shape a Mach-O `.o` has naturally and `ld -r --section-start` produces — a
/// relocatable object that states where a section goes — and the one the writers cannot
/// build.
pub fn elf_place_section(data: &mut [u8], name: &str, address: u64) {
    let (header, index) = elf_section_header(data, name);
    data[header + 0x10..header + 0x18].copy_from_slice(&address.to_le_bytes());

    let (symtab, _) = elf_section_header(data, ".symtab");
    let field = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap()) as usize;
    let offset = field(symtab + 0x18);
    let size = field(symtab + 0x20);
    let entry = field(symtab + 0x38);

    for symbol in (offset..offset + size).step_by(entry) {
        // `st_shndx`, then `st_value`.
        if u16::from_le_bytes(data[symbol + 6..symbol + 8].try_into().unwrap()) != index {
            continue;
        }
        let value = u64::from_le_bytes(data[symbol + 8..symbol + 16].try_into().unwrap());
        data[symbol + 8..symbol + 16].copy_from_slice(&(value + address).to_le_bytes());
    }
}

/// Point one section's bytes off the end of a written ELF, so nothing can read them. The
/// parse keeps no section whose bytes it could not read, which is what makes such a section
/// one the line info has to place without being able to see it.
pub fn elf_unreadable_section(data: &mut [u8], name: &str) {
    let (header, _) = elf_section_header(data, name);
    let past = data.len() as u64 + 0x1000;
    data[header + 0x18..header + 0x20].copy_from_slice(&past.to_le_bytes());
}

/// The ELF in `data` with `name`'s `st_name` pointed past its string table, so `object`
/// answers `Err` for that symbol's name and nothing else about the file changes: the
/// corrupt or mutated object where a text symbol is in the table but cannot be read out
/// of it. Written here rather than by a writer because no writer emits an unreadable name.
pub fn elf_with_unreadable_name(data: &[u8], name: &str) -> Vec<u8> {
    use object::read::{Object as _, ObjectSection as _, ObjectSymbol as _};

    /// `Elf64_Sym`, whose `st_name` is its first field.
    const SYMBOL: usize = 24;

    let mut data = data.to_vec();
    let (index, table) = {
        let file = object::File::parse(&data[..]).expect("the fixture parses");
        let index = file
            .symbols()
            .find(|symbol| symbol.name() == Ok(name))
            .unwrap_or_else(|| panic!("no symbol named {name} in the fixture"))
            .index()
            .0;
        let table = file
            .section_by_name(".symtab")
            .and_then(|section| section.file_range())
            .expect("the fixture has a symbol table")
            .0;
        (index, table as usize)
    };

    let at = table + index * SYMBOL;
    data[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    data
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
            symbol: counter,
            addend: -4,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                size: 32,
            },
        },
    )
    .expect("adding a relocation to .text");

    obj.write().expect("writing the fixture object")
}

/// `caller` = `call rel32; ret`, relocated against `printf`, an undefined `STT_FUNC`: an
/// import, which the file calls and does not define.
pub fn call_to_import() -> Vec<u8> {
    call_to(
        BinaryFormat::Elf,
        write::Symbol {
            name: b"printf".to_vec(),
            value: 0,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Undefined,
            // The writer would make it `STT_NOTYPE`, and a linker writes `STB_GLOBAL` `STT_FUNC`.
            flags: SymbolFlags::Elf {
                st_info: object::elf::SymbolInfo::new(
                    object::elf::STB_GLOBAL,
                    object::elf::STT_FUNC,
                ),
                st_other: object::elf::SymbolOther(0),
            },
        },
    )
}

/// The same call in a COFF object, relocated against `hook`, a weak external of function
/// type with section number 0. The writer puts the weak external's default, a data symbol
/// at an absolute 0, just before it.
pub fn call_to_weak_external() -> Vec<u8> {
    call_to(
        BinaryFormat::Coff,
        write::Symbol {
            name: b"hook".to_vec(),
            value: 0,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: true,
            section: write::SymbolSection::Undefined,
            flags: SymbolFlags::None,
        },
    )
}

/// `caller` = `call rel32; ret` in an x86-64 object of `format`, relocated against `callee`.
fn call_to(format: BinaryFormat, callee: write::Symbol) -> Vec<u8> {
    let mut obj = write::Object::new(format, Architecture::X86_64, Endianness::Little);

    let text = obj.section_id(write::StandardSection::Text);
    let offset = obj.append_section_data(text, &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3], 1);
    obj.add_symbol(write::Symbol {
        name: b"caller".to_vec(),
        value: offset,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    let callee = obj.add_symbol(callee);

    obj.add_relocation(
        text,
        write::Relocation {
            offset: offset + 1,
            symbol: callee,
            addend: -4,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                size: 32,
            },
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
        None,
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
    // The base the offset pairs below are from, stated in the list itself as well as on the
    // unit. `gimli`'s writer does not count a `DW_AT_low_pc` of 0 as a base and rejects a
    // pair without one, and a base-address entry is the other spelling of the same thing —
    // both say 0 and neither is relocated, which is what leaves the list behind when the
    // bias moves the code.
    if fixture.unit_ranges == UnitRanges::OffsetPairs {
        ranges.push(Range::BaseAddress {
            address: Address::Constant(0),
        });
    }
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
                        symbol: symbols[relocation.symbol],
                        addend: relocation.addend,
                        flags: RelocationFlags::Generic {
                            kind: RelocationKind::Absolute,
                            encoding: RelocationEncoding::Generic,
                            size: relocation.size * 8,
                        },
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

/// A **linked** 32-bit x86 image that kept its debug sections' relocations, the shape
/// `ld --emit-relocs` leaves. An i386 relocation is `REL`: the addend sits in the bytes being
/// patched, where the linker has already written the address it resolved, so applying one
/// again would add the symbol's address to it a second time.
///
/// One function, `only`, 0x10 bytes at 0x100 in `.text`, with one line-program row over its
/// first byte and a `DW_AT_low_pc` to match. Both addresses are written as the linker left
/// them: resolved in the bytes, with the relocation that resolved them still naming `only`.
pub fn elf_i386_linked_with_relocations() -> Vec<u8> {
    use gimli::write::{Address, AttributeValue, DwarfUnit, LineProgram, LineString, Sections};

    // Where `only` sits in `.text`, and its address too: the section's own is 0.
    const START: u64 = 0x100;
    const LENGTH: u64 = 0x10;

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::I386, Endianness::Little);

    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0x90; START as usize], 1);
    let value = obj.append_section_data(text, &[0x90; LENGTH as usize], 1);
    let only = obj.add_symbol(write::Symbol {
        name: b"only".to_vec(),
        value,
        size: LENGTH,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    let encoding = gimli::Encoding {
        format: gimli::Format::Dwarf32,
        version: 4,
        address_size: 4,
    };
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        gimli::LineEncoding::default(),
        LineString::String(b"/src".to_vec()),
        None,
        LineString::String(b"main.c".to_vec()),
        None,
    );
    let directory = program.default_directory();
    let file = program.add_file(LineString::String(b"main.c".to_vec()), directory, None);

    // `Address::Symbol` is what [`RelocWriter`] records a relocation for; the zero it writes
    // is overwritten below with the address the linker resolved.
    let address = Address::Symbol {
        symbol: 0,
        addend: 0,
    };
    program.begin_sequence(Some(address));
    let row = program.row();
    row.address_offset = 0;
    row.file = file;
    row.line = 7;
    program.generate_row();
    program.end_sequence(LENGTH);
    dwarf.unit.line_program = program;

    let root = dwarf.unit.root();
    let entry = dwarf.unit.get_mut(root);
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(b"/src".to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(b"main.c".to_vec()),
    );
    entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(address));
    entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(LENGTH));

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
                // An i386 relocation keeps its addend in the section, so `object` writes this
                // one into the bytes and states none in the relocation itself — which is the
                // linked image's shape: the resolved address in the section, the relocation
                // beside it.
                obj.add_relocation(
                    section,
                    write::Relocation {
                        offset: relocation.offset,
                        symbol: only,
                        addend: START as i64 + relocation.addend,
                        flags: RelocationFlags::Generic {
                            kind: RelocationKind::Absolute,
                            encoding: RelocationEncoding::Generic,
                            size: relocation.size * 8,
                        },
                    },
                )
                .expect("adding a relocation to a debug section");
            }
            Ok(())
        })
        .expect("laying out the DWARF sections");

    let mut data = obj.write().expect("writing the fixture object");
    // `write::Object` writes `ET_REL` and nothing else; this file is linked.
    data[16..18].copy_from_slice(&object::elf::ET_EXEC.0.to_le_bytes());
    data
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

/// A relocatable object whose one compilation unit holds **two line-program sequences** with
/// a gap between them, and a symbol that begins in that gap.
///
/// `.text` is 0x16 bytes. `before` is the first 6, which the first sequence covers
/// (`main.c:10`); `middle` is the rest, and only its last 6 bytes are covered, by the second
/// sequence (`other.c:42`). The unit declares the lot, `DW_AT_low_pc` 0 to `DW_AT_high_pc`
/// 0x16, so a question about any of it reaches the unit. Both sequences begin at a relocated
/// address, as a compiler writes them.
///
/// The shape a producer emitting one sequence per section per unit never makes, and the one
/// `addr2line` 0.21 cannot answer: see `notes/upstream/addr2line.md`.
pub fn elf_x86_64_two_sequences() -> Vec<u8> {
    use gimli::write::{Address, AttributeValue, DwarfUnit, LineProgram, LineString, Sections};

    const BEFORE: u64 = 6;
    const MIDDLE: u64 = 0x10;

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);

    let mut symbols = Vec::new();
    for (name, length) in [("before", BEFORE), ("middle", MIDDLE)] {
        let value = obj.append_section_data(text, &vec![0x90; length as usize], 1);
        symbols.push(obj.add_symbol(write::Symbol {
            name: name.as_bytes().to_vec(),
            value,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        }));
    }

    // Every address the DWARF states is written against `before`, the section's first
    // symbol, the way a compiler relocates one.
    let at = |addend: i64| Address::Symbol { symbol: 0, addend };

    let encoding = gimli::Encoding {
        format: gimli::Format::Dwarf32,
        version: 4,
        address_size: 8,
    };
    let mut dwarf = DwarfUnit::new(encoding);
    let mut program = LineProgram::new(
        encoding,
        gimli::LineEncoding::default(),
        LineString::String(b"/src".to_vec()),
        None,
        LineString::String(b"main.c".to_vec()),
        None,
    );
    let directory = program.default_directory();
    let files = [
        program.add_file(LineString::String(b"main.c".to_vec()), directory, None),
        program.add_file(LineString::String(b"other.c".to_vec()), directory, None),
    ];

    for (start, length, file, line) in [(0, BEFORE, 0, 10), (0x10, BEFORE, 1, 42)] {
        program.begin_sequence(Some(at(start)));
        let row = program.row();
        row.address_offset = 0;
        row.file = files[file];
        row.line = line;
        program.generate_row();
        program.end_sequence(length);
    }
    dwarf.unit.line_program = program;

    let root = dwarf.unit.root();
    let entry = dwarf.unit.get_mut(root);
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(b"/src".to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(b"main.c".to_vec()),
    );
    entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(at(0)));
    entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(BEFORE + MIDDLE));

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
                        symbol: symbols[relocation.symbol],
                        addend: relocation.addend,
                        flags: RelocationFlags::Generic {
                            kind: RelocationKind::Absolute,
                            encoding: RelocationEncoding::Generic,
                            size: relocation.size * 8,
                        },
                    },
                )
                .expect("adding a relocation to a debug section");
            }
            Ok(())
        })
        .expect("laying out the DWARF sections");

    obj.write().expect("writing the fixture object")
}

/// `storer` = `mov dword ptr [rip+displacement], 7; ret`, relocated at offset 2 against a
/// **data** symbol — which parsing drops, so the relocation is on the instruction and yet
/// resolves to nothing navigable. `displacement` is what the four placeholder bytes hold:
/// zero is what an ELF RELA leaves them, since its addend is in the relocation entry, and
/// anything else is what a format storing the addend in the operand writes there.
pub fn rip_relative_store_to_data(displacement: i32) -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    let placeholder = displacement.to_le_bytes();
    let text = obj.section_id(write::StandardSection::Text);
    let offset = obj.append_section_data(
        text,
        &[
            0xC7,
            0x05,
            placeholder[0],
            placeholder[1],
            placeholder[2],
            placeholder[3],
            0x07,
            0x00,
            0x00,
            0x00,
            0xC3,
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
            symbol: counter,
            addend: -4,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::Generic,
                size: 32,
            },
        },
    )
    .expect("adding a relocation to .text");

    obj.write().expect("writing the fixture object")
}

/// `probe` = `code`, with one 32-bit **absolute** relocation at `offset` naming the text
/// symbol `g` that follows it — the shape of a relocated operand that is neither a branch
/// nor rip-relative. What the four bytes under the relocation hold is `code`'s business: a
/// format keeping its addend in the operand leaves it there.
pub fn elf_x86_64_absolute(code: &[u8], offset: u64) -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    let text = obj.section_id(write::StandardSection::Text);
    let probe = obj.append_section_data(text, code, 1);
    obj.add_symbol(write::Symbol {
        name: b"probe".to_vec(),
        value: probe,
        size: code.len() as u64,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    let value = obj.append_section_data(text, &[0xC3], 1);
    let g = obj.add_symbol(write::Symbol {
        name: b"g".to_vec(),
        value,
        size: 1,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    obj.add_relocation(
        text,
        write::Relocation {
            offset: probe + offset,
            symbol: g,
            addend: 0,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Absolute,
                encoding: RelocationEncoding::Generic,
                size: 32,
            },
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
/// are `pcrel|sdata4`, an FDE per range of offsets into the `.text` at `text`, and the
/// zero-length terminator. Through `gimli`'s writer, whose pc-relative encoding subtracts
/// only its own offset into the section, so the address handed to it is made relative to
/// the section's `address` first; the terminator is appended by hand, since the writer
/// leaves it out.
fn eh_frame_section(address: u64, text: u64, ranges: &[(u64, u64)], big_endian: bool) -> Vec<u8> {
    use gimli::write::{
        Address, CommonInformationEntry, EhFrame, EndianVec, FrameDescriptionEntry, FrameTable,
    };
    use gimli::{Encoding, Format, Register, RunTimeEndian};

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
        let function = text + begin;
        table.add_fde(
            cie,
            FrameDescriptionEntry::new(
                Address::Constant(function.wrapping_sub(address)),
                (end - begin) as u32,
            ),
        );
    }
    let endian = if big_endian {
        RunTimeEndian::Big
    } else {
        RunTimeEndian::Little
    };
    let mut section = EhFrame(EndianVec::new(endian));
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
        eh_frame_section(IMAGE_BASE + eh_frame_rva, TEXT_ADDRESS, eh_frame, false)
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
/// two of them on the export and the entry point and the third on a function nothing names;
/// a Mach-O executable whose entry point is its `LC_MAIN`, beside the one function its
/// symbol table names; and the images whose functions' stated addresses are not their code's:
/// an ARM ELF, two MIPS ones, three 32-bit ARM DLLs and two armv7 Mach-O ones whose
/// addresses carry a mode bit, and a PPC64 ELFv1 one and two XCOFF ones with descriptors.
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
        (
            "mach-o executable",
            macho_executable(0x1_0000_0000, MACHO_CODE_OFFSET + 0x180, false),
            2,
        ),
        ("arm thumb", arm_thumb_image(), 6),
        ("mips32 compressed", mips_compressed_image(false), 6),
        ("mips64 compressed", mips_compressed_image(true), 6),
        ("armnt dll", armnt_dll(), 2),
        (
            "windows ce arm dll",
            arm_pe_dll(object::pe::IMAGE_FILE_MACHINE_ARM),
            2,
        ),
        (
            "windows ce thumb dll",
            arm_pe_dll(object::pe::IMAGE_FILE_MACHINE_THUMB),
            2,
        ),
        ("armv7 mach-o, LC_MAIN", macho_arm_executable(false), 3),
        ("armv7 mach-o, LC_UNIXTHREAD", macho_arm_executable(true), 3),
        ("ppc64 elfv1", ppc64_elfv1_image(), 3),
        ("xcoff32", xcoff_image(false, XCOFF_DATA), 1),
        ("xcoff64", xcoff_image(true, XCOFF_DATA), 1),
    ]
}

/// Where [`macho_executable`] puts its code in the file: past the load commands, which is
/// where `__text` sits in an image `ld64` links.
pub const MACHO_CODE_OFFSET: u64 = 0x200;

/// One section of an [`elf_image`]: its bytes at `address`, code or data.
pub struct ImageSection<'a> {
    pub name: &'a str,
    pub address: u64,
    pub code: bool,
    pub bytes: &'a [u8],
}

/// One global symbol of an [`elf_image`]. `section` indexes [`ElfImage::sections`], and
/// [`None`] is an undefined symbol: an import.
pub struct ImageSymbol<'a> {
    pub name: &'a str,
    pub value: u64,
    pub size: u64,
    pub kind: object::elf::SymbolType,
    pub section: Option<usize>,
}

/// A linked ELF (`ET_EXEC`) of either class and either byte order, for the machines the
/// other builders here cannot write an image for.
pub struct ElfImage<'a> {
    pub is_64: bool,
    pub big_endian: bool,
    pub machine: object::elf::Machine,
    pub flags: object::elf::FileFlags,
    pub entry: u64,
    pub sections: &'a [ImageSection<'a>],
    /// Written to `.symtab`.
    pub symbols: &'a [ImageSymbol<'a>],
    /// Written to `.dynsym`, which is what `object` reads exports from.
    pub dynamic: &'a [ImageSymbol<'a>],
}

/// [`ElfImage`] written with `object`'s ELF writer, which lays out an image of any class,
/// byte order and machine where [`write::Object`] only writes a relocatable object: the
/// ELF header, each section's bytes, the two symbol tables with their strings, the section
/// names, then the section headers. No program headers, which the parse does not read.
pub fn elf_image(image: ElfImage) -> Vec<u8> {
    use object::elf;
    use object::write::elf::{FileHeader, SectionHeader, Sym, Writer};

    let ElfImage {
        is_64,
        big_endian,
        machine,
        flags,
        entry,
        sections,
        symbols,
        dynamic,
    } = image;
    let endian = if big_endian {
        Endianness::Big
    } else {
        Endianness::Little
    };
    let mut out = Vec::new();
    let mut writer = Writer::new(endian, is_64, &mut out);

    // Everything is reserved first, in the order it is then written.
    writer.reserve_file_header();
    let headers: Vec<_> = sections
        .iter()
        .map(|section| {
            let name = writer.add_section_name(section.name.as_bytes());
            (name, writer.reserve_section_index())
        })
        .collect();
    let names: Vec<_> = symbols
        .iter()
        .map(|symbol| writer.add_string(symbol.name.as_bytes()))
        .collect();
    let dynamic_names: Vec<_> = dynamic
        .iter()
        .map(|symbol| writer.add_dynamic_string(symbol.name.as_bytes()))
        .collect();
    for symbol in symbols {
        writer.reserve_symbol_index(symbol.section.map(|section| headers[section].1));
    }
    for _ in dynamic {
        writer.reserve_dynamic_symbol_index();
    }
    writer.reserve_symtab_section_index();
    writer.reserve_strtab_section_index();
    writer.reserve_dynsym_section_index();
    writer.reserve_dynstr_section_index();
    writer.reserve_shstrtab_section_index();
    let offsets: Vec<u64> = sections
        .iter()
        .map(|section| writer.reserve(section.bytes.len() as u64, 8))
        .collect();
    writer.reserve_symtab();
    writer.reserve_strtab().expect("reserving .strtab");
    writer.reserve_dynsym();
    writer.reserve_dynstr().expect("reserving .dynstr");
    writer.reserve_shstrtab().expect("reserving .shstrtab");
    writer.reserve_section_headers();

    writer
        .write_file_header(&FileHeader {
            os_abi: elf::ELFOSABI_NONE,
            abi_version: 0,
            e_type: elf::ET_EXEC,
            e_machine: machine,
            e_entry: entry,
            e_flags: flags,
        })
        .expect("writing the ELF header");
    for section in sections {
        writer.write_align(8);
        writer.write(section.bytes);
    }
    let sym = |symbol: &ImageSymbol, name| Sym {
        section: symbol.section.map(|section| headers[section].1 .0),
        st_name: name,
        st_info: elf::SymbolInfo::new(elf::STB_GLOBAL, symbol.kind),
        st_other: elf::SymbolOther(0),
        st_shndx: elf::SymbolSection(0),
        st_value: symbol.value,
        st_size: symbol.size,
    };
    writer.write_null_symbol();
    for (symbol, name) in symbols.iter().zip(names) {
        writer.write_symbol(&sym(symbol, writer.string_offset(Some(name))));
    }
    writer.write_strtab();
    writer.write_null_dynamic_symbol();
    for (symbol, name) in dynamic.iter().zip(dynamic_names) {
        writer.write_dynamic_symbol(&sym(symbol, writer.dynamic_string_offset(Some(name))));
    }
    writer.write_dynstr();
    writer.write_shstrtab();

    writer.write_null_section_header();
    for ((section, (name, _)), offset) in sections.iter().zip(&headers).zip(offsets) {
        let flags = if section.code {
            elf::SHF_ALLOC.0 | elf::SHF_EXECINSTR.0
        } else {
            elf::SHF_ALLOC.0 | elf::SHF_WRITE.0
        };
        writer.write_section_header(&SectionHeader {
            sh_name: writer.section_name_offset(Some(*name)),
            sh_type: elf::SHT_PROGBITS,
            sh_flags: elf::SectionFlags(flags),
            sh_addr: section.address,
            sh_offset: offset,
            sh_size: section.bytes.len() as u64,
            sh_addralign: 1,
            ..SectionHeader::default()
        });
    }
    // One local symbol in each table: the null entry.
    writer.write_symtab_section_header(1);
    writer.write_strtab_section_header();
    writer.write_dynsym_section_header(0, 1);
    writer.write_dynstr_section_header(0);
    writer.write_shstrtab_section_header();
    out
}

/// Where [`arm_thumb_image`] puts its code.
pub const ARM_TEXT: u64 = 0x8000;

/// A 32-bit ARM executable whose Thumb functions state their addresses as the ARM ELF ABI
/// has them, with bit 0 set: `thumb_fn` at `.text + 1` in `.symtab`, `thumb_export` at
/// `.text + 9` in `.dynsym`, and the entry point at `.text + 0x11`. Beside them, an ARM
/// function at an even address, and a data symbol at an odd one in `.text`, which is
/// exported and not a function, so its address is not tagged.
pub fn arm_thumb_image() -> Vec<u8> {
    tagged_elf_image(
        ["thumb_fn", "arm_fn", "thumb_export"],
        ElfImage {
            is_64: false,
            big_endian: false,
            machine: object::elf::EM_ARM,
            flags: object::elf::EF_ARM_EABI_VER5,
            entry: 0,
            sections: &[],
            symbols: &[],
            dynamic: &[],
        },
        ARM_TEXT,
    )
}

/// Where [`mips_compressed_image`] puts its code.
pub const MIPS_TEXT: u64 = 0x40_0000;

/// A MIPS executable, 32-bit big-endian or 64-bit little-endian, whose MIPS16 and microMIPS
/// functions state their addresses with bit 0 set: `mips16_fn` at `.text + 1` in `.symtab`,
/// an odd `STT_FUNC` that binutils reads as compressed; `micromips_export` at `.text + 9`
/// in `.dynsym`, where GNU ld and lld both keep the bit; and the entry point at
/// `.text + 0x11`, as both linkers write it. Beside them, a MIPS function at an even
/// address, and a data symbol at an odd one in `.text`, which is not tagged.
pub fn mips_compressed_image(is_64: bool) -> Vec<u8> {
    tagged_elf_image(
        ["mips16_fn", "mips_fn", "micromips_export"],
        ElfImage {
            is_64,
            big_endian: !is_64,
            machine: object::elf::EM_MIPS,
            flags: if is_64 {
                object::elf::EF_MIPS_ARCH_64
            } else {
                object::elf::EF_MIPS_ARCH_32
            },
            entry: 0,
            sections: &[],
            symbols: &[],
            dynamic: &[],
        },
        MIPS_TEXT,
    )
}

/// The image [`arm_thumb_image`] and [`mips_compressed_image`] both are, for `machine`'s
/// header and with `names` for the tagged function, the untagged one and the tagged
/// export: `.text` at `text`, `.data` a page after it, and every address stated as the
/// linker writes it.
///
/// Also stated with bit 0 set: two imports' PLT entries past `.text`, `plt_import` at
/// `.text + 0x31` in `.dynsym`, as GNU ld and lld write a microMIPS one, and `symtab_import`
/// at `.text + 0x21` in `.symtab`; and the one FDE in `.eh_frame`, for a function nothing
/// else names at `.text + 0x14`, which states `0x15..0x19` as an FDE written against labels
/// gas marks as compressed code does.
fn tagged_elf_image(names: [&str; 3], machine: ElfImage, text: u64) -> Vec<u8> {
    let [tagged, untagged, export] = names;
    let eh_frame_address = text + 0x2000;
    let eh_frame = eh_frame_section(eh_frame_address, text, &[(0x15, 0x19)], machine.big_endian);
    elf_image(ElfImage {
        entry: text + 0x11,
        sections: &[
            ImageSection {
                name: ".text",
                address: text,
                code: true,
                bytes: &[0; 0x1c],
            },
            ImageSection {
                name: ".eh_frame",
                address: eh_frame_address,
                code: false,
                bytes: &eh_frame,
            },
            ImageSection {
                name: ".data",
                address: text + 0x1000,
                code: false,
                bytes: &[0; 8],
            },
        ],
        symbols: &[
            ImageSymbol {
                name: tagged,
                value: text + 1,
                size: 4,
                kind: object::elf::STT_FUNC,
                section: Some(0),
            },
            ImageSymbol {
                name: untagged,
                value: text + 4,
                size: 4,
                kind: object::elf::STT_FUNC,
                section: Some(0),
            },
            ImageSymbol {
                name: "a_datum",
                value: text + 0x1001,
                size: 1,
                kind: object::elf::STT_OBJECT,
                section: Some(2),
            },
            ImageSymbol {
                name: "symtab_import",
                value: text + 0x21,
                size: 0,
                kind: object::elf::STT_FUNC,
                section: None,
            },
        ],
        dynamic: &[
            ImageSymbol {
                name: export,
                value: text + 9,
                size: 4,
                kind: object::elf::STT_FUNC,
                section: Some(0),
            },
            ImageSymbol {
                name: "odd_datum",
                value: text + 0xd,
                size: 1,
                kind: object::elf::STT_OBJECT,
                section: Some(0),
            },
            ImageSymbol {
                name: "plt_import",
                value: text + 0x31,
                size: 0,
                kind: object::elf::STT_FUNC,
                section: None,
            },
        ],
        ..machine
    })
}

/// Where [`armnt_dll`] is loaded and puts its code.
pub const ARMNT_BASE: u64 = 0x1000_0000;
pub const ARMNT_TEXT: u64 = ARMNT_BASE + 0x1000;

/// An ARMNT (32-bit ARM Windows) PE32 DLL whose export table and entry point state Thumb
/// code with bit 0 set, as `link.exe` and `lld-link` write them: `thumb_fn` at `.text + 1`
/// and the entry point at `.text + 5`. `odd_datum` is exported at an odd address in
/// `.rdata`. No symbol table, as such a DLL ships.
pub fn armnt_dll() -> Vec<u8> {
    arm_pe_dll(object::pe::IMAGE_FILE_MACHINE_ARMNT)
}

/// [`armnt_dll`] for another 32-bit ARM `machine`: Windows CE's `IMAGE_FILE_MACHINE_ARM` or
/// `IMAGE_FILE_MACHINE_THUMB`, which `object` calls an unknown architecture.
pub fn arm_pe_dll(machine: object::pe::Machine) -> Vec<u8> {
    use object::pe;
    use object::write::pe::{NtHeaders, Writer};

    const TEXT_RVA: u32 = 0x1000;
    const RDATA_RVA: u32 = 0x2000;
    let put16 = |out: &mut [u8], at: usize, value: u16| {
        out[at..at + 2].copy_from_slice(&value.to_le_bytes());
    };
    let put32 = |out: &mut [u8], at: usize, value: u32| {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };

    // Thumb `nop`s, then `bx lr`.
    let mut text = [0x00, 0xbf].repeat(8);
    text.extend_from_slice(&[0x70, 0x47]);
    text.resize(0x100, 0);

    // The export directory, its three arrays, then the names; each name's ordinal is its
    // function's index, and the names are in sorted order as a linker writes them. The
    // writer has no export table, so this is laid out by hand.
    let mut rdata = vec![0u8; 0x100];
    let functions = 40;
    let names = functions + 8;
    let ordinals = names + 8;
    let strings = ordinals + 4;
    let text_names = b"fixture.dll\0odd_datum\0thumb_fn\0";
    rdata[strings..strings + text_names.len()].copy_from_slice(text_names);
    let string = |offset: usize| RDATA_RVA + (strings + offset) as u32;
    put32(&mut rdata, 12, string(0)); // Name
    put32(&mut rdata, 16, 1); // Base
    put32(&mut rdata, 20, 2); // NumberOfFunctions
    put32(&mut rdata, 24, 2); // NumberOfNames
    put32(&mut rdata, 28, RDATA_RVA + functions as u32);
    put32(&mut rdata, 32, RDATA_RVA + names as u32);
    put32(&mut rdata, 36, RDATA_RVA + ordinals as u32);
    put32(&mut rdata, functions, TEXT_RVA + 1); // thumb_fn
    put32(&mut rdata, functions + 4, RDATA_RVA + 0xc1); // odd_datum
    put32(&mut rdata, names, string(12));
    put32(&mut rdata, names + 4, string(22));
    put16(&mut rdata, ordinals, 1);
    put16(&mut rdata, ordinals + 2, 0);

    let mut out = Vec::new();
    let mut writer = Writer::new(false, 0x1000, 0x200, &mut out);
    writer.reserve_dos_header();
    writer.reserve_nt_headers(16);
    writer.set_data_directory(
        pe::IMAGE_DIRECTORY_ENTRY_EXPORT,
        RDATA_RVA,
        rdata.len() as u32,
    );
    writer.reserve_section_headers(2);
    let text_at = writer.reserve_text_section(text.len() as u32);
    let rdata_at = writer.reserve_rdata_section(rdata.len() as u32);
    // The export table above was written for these.
    assert_eq!(text_at.virtual_address, TEXT_RVA);
    assert_eq!(rdata_at.virtual_address, RDATA_RVA);

    writer
        .write_empty_dos_header()
        .expect("writing the DOS header");
    writer.write_nt_headers(NtHeaders {
        machine,
        time_date_stamp: 0,
        characteristics: pe::IMAGE_FILE_EXECUTABLE_IMAGE
            | pe::IMAGE_FILE_32BIT_MACHINE
            | pe::IMAGE_FILE_DLL,
        major_linker_version: 0,
        minor_linker_version: 0,
        address_of_entry_point: TEXT_RVA + 5,
        image_base: ARMNT_BASE,
        major_operating_system_version: 0,
        minor_operating_system_version: 0,
        major_image_version: 0,
        minor_image_version: 0,
        major_subsystem_version: 0,
        minor_subsystem_version: 0,
        subsystem: pe::IMAGE_SUBSYSTEM_UNKNOWN,
        dll_characteristics: pe::DllFlags(0),
        size_of_stack_reserve: 0,
        size_of_stack_commit: 0,
        size_of_heap_reserve: 0,
        size_of_heap_commit: 0,
    });
    writer.write_section_headers();
    writer.write_section(text_at.file_offset, &text);
    writer.write_section(rdata_at.file_offset, &rdata);
    out
}

/// Where [`macho_arm_executable`] puts its code: `__TEXT` at [`MACHO_ARM_BASE`], holding
/// the file from its first byte, and `__text` at [`MACHO_CODE_OFFSET`] into it.
pub const MACHO_ARM_BASE: u64 = 0x4000;
pub const MACHO_ARM_TEXT: u64 = MACHO_ARM_BASE + MACHO_CODE_OFFSET;

/// An armv7 Mach-O executable, stated as `ld64` writes one with Thumb code. `_thumb_fn`
/// at `__text`'s first byte is in the symbol table at its even address, flagged
/// `N_ARM_THUMB_DEF`, and in the export trie with bit 0 set; `_only_exported`, at 8, is in
/// the trie alone, also with bit 0 set. The entry point is at 4, with bit 0 set: in
/// `LC_MAIN`'s `entryoff`, or, with `thread`, in the PC of an `LC_UNIXTHREAD` instead.
pub fn macho_arm_executable(thread: bool) -> Vec<u8> {
    use object::macho;
    use object::write::macho::{
        Encoder, MachHeader, Nlist, SectionHeader, SegmentCommand, SymtabCommand,
    };

    const CODE_LEN: u64 = 0x20;
    const TRIE: u64 = MACHO_CODE_OFFSET + CODE_LEN;
    const NAMES: &[u8] = b"\0_thumb_fn\0";
    let name = |name: &[u8]| {
        let mut padded = [0; 16];
        padded[..name.len()].copy_from_slice(name);
        padded
    };
    // Thumb `nop`s.
    let code = [0x00, 0xbf].repeat(CODE_LEN as usize / 2);

    // The export trie: a root with an edge to each name, and a terminal node under each
    // holding its flags (0, a regular export) and its offset from `__TEXT` as a ULEB128.
    let mut trie = vec![0, 2];
    let edges = [&b"_thumb_fn\0"[..], &b"_only_exported\0"[..]];
    let first = trie.len() + edges.iter().map(|edge| edge.len() + 1).sum::<usize>();
    for (index, edge) in edges.iter().enumerate() {
        trie.extend_from_slice(edge);
        trie.push((first + 5 * index) as u8);
    }
    for offset in [MACHO_CODE_OFFSET + 1, MACHO_CODE_OFFSET + 9] {
        trie.extend_from_slice(&[3, 0, (offset & 0x7f) as u8 | 0x80, (offset >> 7) as u8, 0]);
    }
    let symbols = TRIE + trie.len().next_multiple_of(4) as u64;

    let encoder = Encoder::new(Endianness::Little, false);
    let mut commands = Vec::new();
    let segment = |commands: &mut Vec<u8>, segname, vmaddr, vmsize, filesize, nsects| {
        let prot = if nsects == 0 {
            macho::VmProt(0)
        } else {
            macho::VM_PROT_READ | macho::VM_PROT_EXECUTE
        };
        encoder.segment_command(
            commands,
            &SegmentCommand {
                segname: name(segname),
                vmaddr,
                vmsize,
                fileoff: 0,
                filesize,
                maxprot: prot,
                initprot: prot,
                nsects,
                flags: macho::SegmentFlags(0),
            },
        );
    };
    segment(&mut commands, b"__PAGEZERO", 0, MACHO_ARM_BASE, 0, 0);
    segment(&mut commands, b"__TEXT", MACHO_ARM_BASE, TRIE, TRIE, 1);
    encoder.section_header(
        &mut commands,
        &SectionHeader {
            sectname: name(b"__text"),
            segname: name(b"__TEXT"),
            addr: MACHO_ARM_TEXT,
            size: CODE_LEN,
            offset: MACHO_CODE_OFFSET as u32,
            align: 1,
            reloff: 0,
            nreloc: 0,
            flags: macho::S_ATTR_PURE_INSTRUCTIONS | macho::S_ATTR_SOME_INSTRUCTIONS,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
        },
    );
    if thread {
        // ARM_THREAD_STATE and its 17 words: r0 to r12, sp, lr, pc and cpsr.
        let mut state = [1u32, 17].to_vec();
        state.extend([0; 17]);
        state[2 + 15] = (MACHO_ARM_TEXT + 5) as u32;
        let state: Vec<u8> = state.iter().flat_map(|word| word.to_le_bytes()).collect();
        encoder.load_command(&mut commands, macho::LC_UNIXTHREAD, &state);
    } else {
        let mut main = (MACHO_CODE_OFFSET + 5).to_le_bytes().to_vec();
        main.extend_from_slice(&[0; 8]);
        encoder.load_command(&mut commands, macho::LC_MAIN, &main);
    }
    let mut exports = (TRIE as u32).to_le_bytes().to_vec();
    exports.extend_from_slice(&(trie.len() as u32).to_le_bytes());
    encoder.load_command(&mut commands, macho::LC_DYLD_EXPORTS_TRIE, &exports);
    encoder.symtab_command(
        &mut commands,
        &SymtabCommand {
            symoff: symbols as u32,
            nsyms: 1,
            stroff: symbols as u32 + encoder.nlist_size() as u32,
            strsize: NAMES.len() as u32,
        },
    );

    let mut file = Vec::new();
    encoder.mach_header(
        &mut file,
        &MachHeader {
            cputype: macho::CPU_TYPE_ARM,
            cpusubtype: macho::CPU_SUBTYPE_ARM_V7.into(),
            filetype: macho::MH_EXECUTE,
            ncmds: load_commands(&commands),
            sizeofcmds: commands.len() as u32,
            flags: macho::FileFlags(0),
        },
    );
    file.extend_from_slice(&commands);
    file.resize(MACHO_CODE_OFFSET as usize, 0);
    file.extend_from_slice(&code);
    file.extend_from_slice(&trie);
    file.resize(symbols as usize, 0);
    encoder.nlist(
        &mut file,
        &Nlist {
            n_strx: 1,
            n_type: macho::N_SECT | macho::N_EXT,
            n_sect: 1,
            n_desc: macho::N_ARM_THUMB_DEF,
            n_value: MACHO_ARM_TEXT,
        },
    );
    file.extend_from_slice(NAMES);
    file
}

/// How many little-endian Mach-O load commands `commands` holds, each as long as its
/// `cmdsize` says: what a header's `ncmds` is, counted from what was written.
fn load_commands(commands: &[u8]) -> u32 {
    let mut count = 0;
    let mut rest = commands;
    while !rest.is_empty() {
        let size = u32::from_le_bytes(rest[4..8].try_into().unwrap()) as usize;
        assert!(size >= 8, "a load command's size covers its own header");
        rest = &rest[size..];
        count += 1;
    }
    count
}

/// Where [`ppc64_elfv1_image`] puts its code and its descriptors.
pub const PPC64_TEXT: u64 = 0x1000_0000;
pub const PPC64_OPD: u64 = 0x1002_0000;

/// A big-endian PPC64 ELFv1 executable. Its functions' symbols and its entry point name
/// descriptors in `.opd`, 24 bytes each, whose first doubleword is the code's address:
/// `foo` names `.text`'s first byte, `bar` its ninth, and the entry point its thirteenth.
/// `.foo` names `foo`'s code directly, as older toolchains wrote it, with the code's size.
/// `broken` names a descriptor that runs past the end of `.opd`. `bar` is in `.dynsym`
/// too, as a shared object's exported functions are.
pub fn ppc64_elfv1_image() -> Vec<u8> {
    let mut opd = Vec::new();
    for code in [PPC64_TEXT, PPC64_TEXT + 8, PPC64_TEXT + 12] {
        opd.extend_from_slice(&code.to_be_bytes());
        opd.extend_from_slice(&0x1003_8000u64.to_be_bytes()); // the TOC
        opd.extend_from_slice(&0u64.to_be_bytes());
    }
    opd.extend_from_slice(&[0; 4]);
    let bar = ImageSymbol {
        name: "bar",
        value: PPC64_OPD + 0x18,
        size: 24,
        kind: object::elf::STT_FUNC,
        section: Some(1),
    };
    elf_image(ElfImage {
        is_64: true,
        big_endian: true,
        machine: object::elf::EM_PPC64,
        flags: object::elf::FileFlags(1), // ELFv1
        entry: PPC64_OPD + 0x30,
        sections: &[
            ImageSection {
                name: ".text",
                address: PPC64_TEXT,
                code: true,
                // Four `nop`s.
                bytes: &[0x60, 0, 0, 0, 0x60, 0, 0, 0, 0x60, 0, 0, 0, 0x60, 0, 0, 0],
            },
            ImageSection {
                name: ".opd",
                address: PPC64_OPD,
                code: false,
                bytes: &opd,
            },
        ],
        symbols: &[
            ImageSymbol {
                name: "foo",
                value: PPC64_OPD,
                size: 24,
                kind: object::elf::STT_FUNC,
                section: Some(1),
            },
            ImageSymbol {
                name: ".foo",
                value: PPC64_TEXT,
                size: 8,
                kind: object::elf::STT_FUNC,
                section: Some(0),
            },
            ImageSymbol { ..bar },
            ImageSymbol {
                name: "broken",
                value: PPC64_OPD + 0x48,
                size: 24,
                kind: object::elf::STT_FUNC,
                section: Some(1),
            },
        ],
        dynamic: &[bar],
    })
}

/// A big-endian PPC64 ELFv1 relocatable object: `foo`'s descriptor in `.opd` is zeros, and
/// the `R_PPC64_ADDR64` against `.text` plus 8 that the linker fills its first doubleword
/// from is the only place the code's address is stated.
pub fn ppc64_elfv1_object() -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::PowerPc64, Endianness::Big);
    obj.flags = object::FileFlags::Elf {
        os_abi: object::elf::ELFOSABI_NONE,
        abi_version: 0,
        e_flags: object::elf::FileFlags(1),
    };
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0x60, 0, 0, 0].repeat(4), 4);
    let opd = obj.add_section(Vec::new(), b".opd".to_vec(), SectionKind::Data);
    obj.append_section_data(opd, &[0; 24], 8);
    obj.add_symbol(write::Symbol {
        name: b"foo".to_vec(),
        value: 0,
        size: 24,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(opd),
        flags: SymbolFlags::None,
    });
    let text_symbol = obj.section_symbol(text);
    obj.add_relocation(
        opd,
        write::Relocation {
            offset: 0,
            symbol: text_symbol,
            addend: 8,
            flags: RelocationFlags::Elf {
                r_type: object::elf::R_PPC64_ADDR64,
            },
        },
    )
    .expect("adding the descriptor's relocation");
    obj.write().expect("writing the fixture object")
}

/// Where [`xcoff_image`] puts its code and its entry point's descriptor.
pub const XCOFF_TEXT: u64 = 0x1000_0000;
pub const XCOFF_DATA: u64 = 0x2000_0000;

/// An XCOFF executable, 32- or 64-bit, with no symbol table: `.text`, and `.data` holding
/// one function descriptor whose first word is `.text + 8`. `entry` is the auxiliary
/// header's `o_entry`, which a linker points at that descriptor. Written byte by byte:
/// `object` writes no XCOFF image (`notes/upstream/object.md`).
pub fn xcoff_image(is_64: bool, entry: u64) -> Vec<u8> {
    let word = if is_64 { 8 } else { 4 };
    let put = |out: &mut Vec<u8>, value: u64, width: usize| {
        out.extend_from_slice(&value.to_be_bytes()[8 - width..]);
    };
    let (file_header, aux_header, section_header) =
        if is_64 { (24, 120, 72) } else { (20, 72, 40) };
    let text = [0x60, 0, 0, 0].repeat(4);
    let mut descriptor = Vec::new();
    for value in [XCOFF_TEXT + 8, XCOFF_DATA + 0x100, 0] {
        put(&mut descriptor, value, word);
    }
    let text_at = (file_header + aux_header + 2 * section_header) as u64;
    let data_at = text_at + text.len() as u64;

    let mut out = Vec::new();
    // f_magic, f_nscns, f_timdat, then f_symptr, f_opthdr, f_flags and f_nsyms in the
    // class's order. F_EXEC = 2.
    put(&mut out, if is_64 { 0x01F7 } else { 0x01DF }, 2);
    put(&mut out, 2, 2);
    put(&mut out, 0, 4);
    if is_64 {
        put(&mut out, 0, 8);
        put(&mut out, aux_header as u64, 2);
        put(&mut out, 2, 2);
        put(&mut out, 0, 4);
    } else {
        put(&mut out, 0, 4);
        put(&mut out, 0, 4);
        put(&mut out, aux_header as u64, 2);
        put(&mut out, 2, 2);
    }

    // Only `o_entry` is read: after o_mflag, o_vstamp and three sizes in XCOFF32; after
    // o_debugger, three addresses, nine halves, six bytes and three sizes in XCOFF64.
    let entry_at = if is_64 { 80 } else { 16 };
    let mut aux = vec![0u8; aux_header];
    aux[entry_at..entry_at + word].copy_from_slice(&entry.to_be_bytes()[8 - word..]);
    out.extend_from_slice(&aux);

    // s_name, s_paddr, s_vaddr, s_size, s_scnptr, s_relptr, s_lnnoptr, s_nreloc, s_nlnno,
    // s_flags and, in XCOFF64, s_reserve. STYP_TEXT = 0x20, STYP_DATA = 0x40.
    for (name, address, bytes, at, kind) in [
        (b".text\0\0\0", XCOFF_TEXT, &text, text_at, 0x20),
        (b".data\0\0\0", XCOFF_DATA, &descriptor, data_at, 0x40),
    ] {
        out.extend_from_slice(name);
        put(&mut out, address, word);
        put(&mut out, address, word);
        put(&mut out, bytes.len() as u64, word);
        put(&mut out, at, word);
        put(&mut out, 0, word);
        put(&mut out, 0, word);
        let count = if is_64 { 4 } else { 2 };
        put(&mut out, 0, count);
        put(&mut out, 0, count);
        put(&mut out, kind, 4);
        if is_64 {
            put(&mut out, 0, 4);
        }
    }
    out.extend_from_slice(&text);
    out.extend_from_slice(&descriptor);
    out
}

/// An x86-64 Mach-O **executable** (`MH_EXECUTE`), assembled with `object`'s encoder because
/// its writer emits `MH_OBJECT` only, with neither segments nor `LC_MAIN`. `__PAGEZERO`, then
/// `__TEXT` at `text_vmaddr` holding the file from its first byte, as `ld64` lays it out, with
/// one `__text` section at [`MACHO_CODE_OFFSET`]: 0x200 bytes of two functions, `_first` at
/// offset 0, which the symbol table names, and one at 0x180, which nothing names. `entryoff`
/// is `LC_MAIN`'s, a file offset. With `unreadable_thread`, an `LC_UNIXTHREAD` comes first
/// whose state stops before its PC, which leaves the entry to the `LC_MAIN`.
pub fn macho_executable(text_vmaddr: u64, entryoff: u64, unreadable_thread: bool) -> Vec<u8> {
    use object::macho;
    use object::write::macho::{
        Encoder, MachHeader, Nlist, SectionHeader, SegmentCommand, SymtabCommand,
    };

    const CODE_LEN: u64 = 0x200;
    const SYMBOLS: u64 = MACHO_CODE_OFFSET + CODE_LEN;
    const NAMES: &[u8] = b"\0_first\0";
    let name = |name: &[u8]| {
        let mut padded = [0; 16];
        padded[..name.len()].copy_from_slice(name);
        padded
    };
    let mut code = vec![0x90; CODE_LEN as usize];
    code[0x17f] = 0xC3;
    code[0x1ff] = 0xC3;

    let encoder = Encoder::new(Endianness::Little, true);
    let mut commands = Vec::new();
    encoder.segment_command(
        &mut commands,
        &SegmentCommand {
            segname: name(b"__PAGEZERO"),
            vmaddr: 0,
            vmsize: text_vmaddr,
            fileoff: 0,
            filesize: 0,
            maxprot: macho::VmProt(0),
            initprot: macho::VmProt(0),
            nsects: 0,
            flags: macho::SegmentFlags(0),
        },
    );
    encoder.segment_command(
        &mut commands,
        &SegmentCommand {
            segname: name(b"__TEXT"),
            vmaddr: text_vmaddr,
            vmsize: SYMBOLS,
            fileoff: 0,
            filesize: SYMBOLS,
            maxprot: macho::VM_PROT_READ | macho::VM_PROT_EXECUTE,
            initprot: macho::VM_PROT_READ | macho::VM_PROT_EXECUTE,
            nsects: 1,
            flags: macho::SegmentFlags(0),
        },
    );
    encoder.section_header(
        &mut commands,
        &SectionHeader {
            sectname: name(b"__text"),
            segname: name(b"__TEXT"),
            addr: text_vmaddr + MACHO_CODE_OFFSET,
            size: CODE_LEN,
            offset: MACHO_CODE_OFFSET as u32,
            align: 4,
            reloff: 0,
            nreloc: 0,
            flags: macho::S_ATTR_PURE_INSTRUCTIONS | macho::S_ATTR_SOME_INSTRUCTIONS,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
        },
    );
    if unreadable_thread {
        // x86_THREAD_STATE64 and its count of 42 words, and none of the words.
        let mut state = 4u32.to_le_bytes().to_vec();
        state.extend_from_slice(&42u32.to_le_bytes());
        encoder.load_command(&mut commands, macho::LC_UNIXTHREAD, &state);
    }
    let mut main = entryoff.to_le_bytes().to_vec();
    main.extend_from_slice(&[0; 8]);
    encoder.load_command(&mut commands, macho::LC_MAIN, &main);
    encoder.symtab_command(
        &mut commands,
        &SymtabCommand {
            symoff: SYMBOLS as u32,
            nsyms: 1,
            stroff: SYMBOLS as u32 + encoder.nlist_size() as u32,
            strsize: NAMES.len() as u32,
        },
    );

    let mut file = Vec::new();
    encoder.mach_header(
        &mut file,
        &MachHeader {
            cputype: macho::CPU_TYPE_X86_64,
            cpusubtype: macho::CPU_SUBTYPE_X86_64_ALL.into(),
            filetype: macho::MH_EXECUTE,
            ncmds: load_commands(&commands),
            sizeofcmds: commands.len() as u32,
            flags: macho::FileFlags(0),
        },
    );
    file.extend_from_slice(&commands);
    file.resize(MACHO_CODE_OFFSET as usize, 0);
    file.extend_from_slice(&code);
    encoder.nlist(
        &mut file,
        &Nlist {
            n_strx: 1,
            n_type: macho::N_SECT | macho::N_EXT,
            n_sect: 1,
            n_desc: macho::SymbolDesc(0),
            n_value: text_vmaddr + MACHO_CODE_OFFSET,
        },
    );
    file.extend_from_slice(NAMES);
    file
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

/// A directory of one test's own under the system temporary directory, removed when the
/// test ends -- when it panics included, which is the case a removal at the foot of the
/// body misses. The fixtures that have to be on disk go here, and `/tmp` is memory on many
/// systems.
pub struct Scratch(pub PathBuf);

impl Scratch {
    /// Named `analysis-{what}-{pid}-{n}`, with `n` counted across the process, so no two
    /// calls share one.
    pub fn new(what: &str) -> Scratch {
        static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("analysis-{what}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("creating the test directory");
        Scratch(directory)
    }

    /// A fixture written into it, at the path it was written to.
    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("writing a fixture");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

//! In-memory fixture builders. Nothing here touches the filesystem or commits a blob:
//! every test object is assembled with the `object` crate's writer.

#![allow(dead_code)]

use analysis::{parse_object, Object};
use object::write;
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationKind, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;

/// Parse, then walk everything a parsed object exposes, so a panic in size estimation or
/// disassembly is caught too and not just one in `parse_object`.
pub fn parse_and_walk(data: &[u8]) -> Option<Arc<Object>> {
    let object = parse_object(data.into(), "fuzz".into(), PathBuf::from("/fuzz"))?;

    for symbol in &object.symbols_sorted {
        let _ = symbol.estimate_size();
        let _ = symbol.data();
        // The debug-info extent goes down the same DWARF path `line_info` does, and it
        // is what `assembly` slices the symbol's bytes with.
        let _ = symbol.extent(&object);
        let _ = symbol.data_in(&object);
        if let Some(assembly) = symbol.assembly(&object) {
            for instruction in &assembly.instructions {
                let _: String = instruction.format.iter().map(|(t, _)| t.as_str()).collect();
            }
            // A branch edge is a pair of row indices a renderer will index the listing
            // with, so both ends have to be rows that exist however corrupt the bytes
            // they were decoded from — an edge naming a row that is not there would be a
            // panic in the gutter rather than here.
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
        // The DWARF path, on every input the sweeps below produce. Reading the rows back
        // matters as much as building them: `LineInfo`'s own invariants (ascending,
        // non-overlapping, in-range file indices) are what `row_at` and `file_of` rely on.
        // Non-overlapping holds for *any* input, however corrupt — the rows are clipped
        // to make it hold — so `previous` is the last row's **end**, not its start.
        if let Some(info) = symbol.line_info(&object) {
            let mut previous = 0;
            for row in info.rows() {
                assert!(row.range.start >= previous && row.range.start < row.range.end);
                previous = row.range.end;
                // Which is exactly what makes `row_at` well defined: an address inside a
                // row is answered by that row and no other.
                assert_eq!(
                    info.row_at(row.range.start).map(|found| found.range.clone()),
                    Some(row.range.clone())
                );
                let _ = info.file_of(row);
                let _ = info.location(row.range.start);
            }
            let _ = info.location(u64::MAX);
        }
    }
    // Also ask each section about a range no symbol covers, so the context is built even
    // for an object whose symbols were all dropped.
    for section in &object.sections {
        let _ = object.line_info(section, 0..u64::MAX);
    }

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

/// A relocation inside the generated `.text`: `(symbol index, offset within that
/// symbol, index of the symbol it targets)`.
pub struct TextRelocation {
    pub in_symbol: usize,
    pub offset: u64,
    pub target: usize,
}

/// Build a minimal x86-64 ELF relocatable object whose `.text` holds `symbols` laid out
/// back to back, each declared with `st_size == 0` — the common case that
/// `SymbolData::estimate_size` exists to work around.
pub fn elf_x86_64(symbols: &[TextSymbol], relocations: &[TextRelocation]) -> Vec<u8> {
    elf_text(Architecture::X86_64, symbols, relocations)
}

/// The same fixture for any architecture, which is what makes "the decoder comes from the
/// file" testable: the *bytes* of a `.text` section say nothing about how to read
/// themselves, so the only difference between an object that decodes as 32-bit x86 and
/// one that decodes as aarch64 is the `e_machine` in its header.
///
/// `relocations` are written with x86's branch encoding, so anything but x86 has to pass
/// none — which is no loss, since what a non-x86 fixture is for is the decode itself.
pub fn elf_text(
    architecture: Architecture,
    symbols: &[TextSymbol],
    relocations: &[TextRelocation],
) -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, architecture, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);

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

/// The fixture from the plan: `caller` = `call rel32; ret` with a relocation at offset 1
/// pointing at `target` = `ret`.
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

/// `caller` = `call qword ptr [rip+0x0]; ret` — an *indirect* call, where the operand the
/// relocation applies to is a rip-relative **memory** operand rather than the whole branch
/// target. `FF 15` is the opcode and the four displacement bytes start at offset 2, which
/// is where a linker puts the relocation. `target` = `ret`, as in [`caller_and_target`].
///
/// With `relocated` unset the same bytes carry no relocation at all, which is the control
/// for what the relocated form should print.
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

/// `jumper` = `jmp rel32; ret`, with the branch relocated against a **data** symbol.
///
/// Parsing keeps only `SymbolKind::Text` symbols, so the instruction's `relocation` comes
/// out as [`None`] even though the four displacement bytes are every bit as much a
/// placeholder as a resolvable branch's are. Read literally the jump lands on address 5,
/// which is this symbol's own `ret`: the fixture where "did a relocation resolve?" and
/// "is this displacement real?" give different answers.
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

/// Deterministic pseudo-random bytes (xorshift64*), so a failure is always reproducible.
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

/// One row of a fixture's line program: an offset from the start of the section it
/// belongs to, and the source position at it.
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
/// A fixture with several of these is the shape rustc emits: one `.text.<name>` section
/// per function, **every one of them at address 0**, because a section in a relocatable
/// object has no address until it is linked. That is the case an address alone cannot
/// key, and the one the single-`.text` fixtures cannot show.
pub struct DwarfSection<'a> {
    /// [`None`] for the standard `.text`; [`Some`] for a section of its own, named the
    /// way a compiler names them (`.text.first`).
    pub name: Option<&'a str>,
    pub symbols: &'a [TextSymbol<'a>],
    /// Rows of this section's sequence, addressed from the section's own start.
    pub rows: &'a [DwarfRow],
    /// Where this section's sequence ends, as an offset into the section.
    pub length: u64,
    /// One `DW_TAG_subprogram` per entry, as `(index into `symbols`, extent in bytes)`
    /// — the compiler stating a function's `DW_AT_low_pc`/`DW_AT_high_pc` rather than
    /// leaving `SymbolData::extent` to derive it from where the next symbol starts.
    /// Empty is the shape every line-info fixture had before extents were read.
    pub subprograms: &'a [(usize, u64)],
    /// When set, an index into this section's `symbols`: the sequence's
    /// `DW_LNE_set_address` (and the unit's range entry, where there is one) is written
    /// as zero with an absolute relocation against that symbol, exactly the way a
    /// compiler emits it for a relocatable object. When unset, the address is a
    /// constant, as in a linked binary.
    pub base_symbol: Option<usize>,
}

pub struct DwarfFixture<'a> {
    pub comp_dir: &'a str,
    /// The source files the line programs can name; `DwarfRow::file` indexes this.
    pub files: &'a [&'a str],
    /// One or more code sections. One keeps the linked shape — a unit with
    /// `DW_AT_low_pc`/`DW_AT_high_pc`; several give the unit a `DW_AT_ranges` list with
    /// one relocated entry apiece, which is what a discontiguous unit looks like.
    pub sections: &'a [DwarfSection<'a>],
}

/// Build an x86-64 ELF like [`elf_x86_64`], with a DWARF compilation unit and line
/// program describing its code sections.
///
/// This is real DWARF written by `gimli::write`, not bytes copied out of a compiler, so
/// the test that reads it back is a round trip through the same formats a compiler emits
/// without needing one installed. Addresses that a compiler would relocate are written
/// through [`RelocWriter`], which records where each one landed, so the ELF carries the
/// same relocations against the same symbols — no byte pattern is searched for.
pub fn elf_x86_64_with_dwarf(fixture: DwarfFixture) -> Vec<u8> {
    use gimli::write::{
        Address, AttributeValue, DwarfUnit, LineProgram, LineString, Range, RangeList, Sections,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);

    // One symbol table for the whole fixture: `Address::Symbol` indexes into this, and so
    // does the relocation pass at the bottom.
    let mut symbols: Vec<write::SymbolId> = Vec::new();
    // The flat index of each section's base symbol, where it has one.
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
                size: 0,
                kind: SymbolKind::Text,
                scope: SymbolScope::Linkage,
                weak: false,
                section: write::SymbolSection::Section(id),
                flags: SymbolFlags::None,
            }));
        }
        bases.push(section.base_symbol.map(|index| first + index));
    }

    // The address a section's line program starts at: a relocation against one of its
    // symbols, or a literal 0 the way a linked image has it.
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
        .map(|file| program.add_file(LineString::String(file.as_bytes().to_vec()), directory, None))
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
        ranges.push(Range::StartLength {
            begin: address(index),
            length: section.length,
        });
    }
    dwarf.unit.line_program = program;

    // The subprogram DIEs, as children of the unit's root. Their `low_pc` is written
    // through the same relocated `Address::Symbol` a compiler uses, so a relocatable
    // fixture's extents are keyed the same way its line rows are.
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
    let unit_ranges = (fixture.sections.len() > 1)
        .then(|| dwarf.unit.ranges.add(RangeList(ranges)));
    let entry = dwarf.unit.get_mut(root);
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(fixture.comp_dir.as_bytes().to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(fixture.files[0].as_bytes().to_vec()),
    );
    match unit_ranges {
        // A DWARF 4 range list holds offsets from the unit's base address, so the unit
        // must not also declare a `DW_AT_low_pc`: the entries are the absolute addresses
        // already, each one relocated on its own.
        Some(list) => entry.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(list)),
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
            let section =
                obj.add_section(Vec::new(), id.name().as_bytes().to_vec(), SectionKind::Debug);
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

/// Where one address in a debug section is, and what it is relative to.
#[derive(Clone)]
pub struct DebugRelocation {
    offset: u64,
    /// An index into the fixture's symbol table.
    symbol: usize,
    addend: i64,
    /// In bytes, as `gimli` writes it; ELF wants bits.
    size: u8,
}

/// A `gimli::write::Writer` that records relocations instead of refusing them.
///
/// `EndianVec` alone answers `Address::Symbol` with `Error::InvalidAddress`, which is
/// exactly the address form a compiler emits into a relocatable object's `.debug_line`
/// and `.debug_ranges`. Recording each one — where it landed, which symbol it is against
/// and with what addend — is what lets the fixture carry real relocations without
/// searching the written bytes for the opcode that produced them.
#[derive(Clone)]
pub struct RelocWriter {
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
                // The placeholder the relocation replaces, as a compiler writes it.
                self.write_udata(0, size)
            }
        }
    }
}

/// `storer` = `mov dword ptr [rip+0x0], 7; ret`, with the relocation at offset 2 pointing
/// at a **data** symbol in `.data` — a global variable, which is what a rip-relative store
/// like this one actually writes to.
///
/// Parsing keeps only `SymbolKind::Text` symbols, so the relocation is present on the
/// instruction and yet resolves to nothing the viewer can navigate to. That is the case
/// where the operand has to keep its plain displacement rather than gain a link that goes
/// nowhere.
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

/// One entry of a hand-built shared library's export/dynamic symbol table.
pub struct ExportedSymbol<'a> {
    pub name: &'a str,
    /// An offset into the fixture's `.text`, not a virtual address: the builders below
    /// place `.text` themselves, so a test never has to know where.
    pub offset: u64,
    /// What the declaration itself claims, which is 0 for a PE export — its table has no
    /// room for a size — and whatever is asked for in an ELF `.dynsym`.
    pub size: u64,
    /// When false the symbol is written as data (ELF `STT_OBJECT`, or a PE export whose
    /// address is in `.rdata`), which must **not** come out as a text symbol.
    pub code: bool,
}

/// Where a hand-built image puts its code, chosen to look like something a linker
/// produced rather than to be memorable: a page in, at a non-zero image base.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;
pub const TEXT_RVA: u64 = 0x1000;
/// The virtual address of the fixtures' `.text`, i.e. what an exported `offset` is
/// relative to.
pub const TEXT_ADDRESS: u64 = IMAGE_BASE + TEXT_RVA;

/// A hand-built ELF shared object: what is in `.text`, what each of the two symbol
/// tables declares, and whether the header names an entry point.
pub struct SharedObject<'a> {
    pub text: &'a [u8],
    /// Written to `.dynsym`, which is the table a stripped library still has.
    pub dynamic: &'a [ExportedSymbol<'a>],
    /// Written to `.symtab`, which is the table `strip` removes. Leave it empty for the
    /// stripped case; filling both is how a file declaring one function twice is built.
    pub static_symbols: &'a [ExportedSymbol<'a>],
    /// An offset into `.text`, or [`None`] for an image that declares no entry point.
    pub entry: Option<u64>,
}

/// Build an x86-64 ELF **shared object** (`ET_DYN`) with the symbol tables asked for.
///
/// This one is assembled byte by byte rather than with `object`'s writer, which emits
/// `ET_REL` relocatable objects and has no way to write a dynamic symbol table at all.
/// That is precisely the shape being tested: a stripped `.so` has no `.symtab`, so
/// `Object::symbols()` is empty and everything the file says about its own code it says
/// through `.dynsym`.
pub fn elf_shared_object(fixture: SharedObject) -> Vec<u8> {
    const SHDR: usize = 64;
    const EHDR: usize = 64;
    const SYM: usize = 24;

    // Section indices, in the order they are written below.
    const TEXT: u16 = 1;
    const DATA: u16 = 2;
    const SHSTRTAB: u16 = 7;
    const SECTIONS: u16 = 8;

    let SharedObject {
        text,
        dynamic,
        static_symbols,
        entry,
    } = fixture;

    // `.data` exists only so a data symbol has somewhere to be that is not code.
    let data = [0u8; 8];
    let data_rva = TEXT_RVA + text.len() as u64 + 0x1000;

    // One symbol table: the string table it names, and the entries themselves, which
    // start with the null entry every ELF symbol table has.
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

    // Contents laid out back to back after the header; addresses are their own thing.
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
    let shoff = out.len() as u64;

    // sh_name, sh_type, sh_flags, sh_addr, (sh_offset, sh_size), sh_link, sh_entsize.
    // sh_info is 1 for a symbol table (one local symbol, the null entry) and 0
    // otherwise; sh_addralign is always 1 here.
    let shdr = |name: u32, kind: u32, flags: u64, addr: u64, at: (u64, u64), link: u32, entsize: u64| {
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
    out.extend_from_slice(&shdr(names[0], 1, 2 | 4, IMAGE_BASE + TEXT_RVA, text_at, 0, 0));
    out.extend_from_slice(&shdr(names[1], 1, 2 | 1, IMAGE_BASE + data_rva, data_at, 0, 0));
    out.extend_from_slice(&shdr(names[2], 11, 2, 0, dynsym_at, 4, SYM as u64));
    out.extend_from_slice(&shdr(names[3], 3, 2, 0, dynstr_at, 0, 0));
    out.extend_from_slice(&shdr(names[4], 2, 0, 0, symtab_at, 6, SYM as u64));
    out.extend_from_slice(&shdr(names[5], 3, 0, 0, strtab_at, 0, 0));
    out.extend_from_slice(&shdr(names[6], 3, 0, 0, shstrtab_at, 0, 0));

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
    header[60..62].copy_from_slice(&SECTIONS.to_le_bytes());
    header[62..64].copy_from_slice(&SHSTRTAB.to_le_bytes());

    out
}

/// Build an x86-64 PE **DLL** with an export directory and **no COFF symbol table**,
/// which is the shape of the repo's `LLVM-24-rust-dev.dll` sample: `Object::symbols()`
/// is empty and the export table is the only thing that names any code.
///
/// Hand-assembled for the same reason the ELF above is — `object`'s writer emits COFF
/// object files, not images, and has no export directory in it.
///
/// `entry` is an offset into `.text`, or [`None`] for a DLL with no entry point (which
/// is a real thing: `AddressOfEntryPoint` is 0 in a resource-only DLL).
pub fn pe_dll(text: &[u8], symbols: &[ExportedSymbol], entry: Option<u64>) -> Vec<u8> {
    const FILE_ALIGNMENT: usize = 0x200;
    const SECTION_ALIGNMENT: u64 = 0x1000;
    /// DOS stub, `PE\0\0`, COFF header, PE32+ optional header, two section headers,
    /// rounded up to one file-alignment unit — which everything below assumes fits.
    const HEADERS: usize = FILE_ALIGNMENT;

    let text_size = text.len();
    let text_raw = text_size.next_multiple_of(FILE_ALIGNMENT);
    let rdata_rva = TEXT_RVA + (text_size as u64).next_multiple_of(SECTION_ALIGNMENT);

    // The export directory, then the three parallel arrays it points at, then the
    // name strings — all inside `.rdata`, laid out in that order.
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

    // A data export points into `.rdata` — past everything the export table itself
    // occupies — so it is an address in a section that is not code.
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
    // Room for the data export to point at.
    rdata.resize(rdata.len() + 0x20, 0);

    let rdata_size = rdata.len();
    let rdata_raw = rdata_size.next_multiple_of(FILE_ALIGNMENT);
    let image_size = (rdata_rva + rdata_size as u64).next_multiple_of(SECTION_ALIGNMENT);

    let mut out = vec![0u8; HEADERS + text_raw + rdata_raw];
    out[HEADERS..HEADERS + text_size].copy_from_slice(text);
    out[HEADERS + text_raw..HEADERS + text_raw + rdata_size].copy_from_slice(&rdata);

    out[..2].copy_from_slice(b"MZ");
    out[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes()); // e_lfanew
    out[0x40..0x44].copy_from_slice(b"PE\0\0");

    // COFF header at 0x44: Machine, NumberOfSections, TimeDateStamp,
    // PointerToSymbolTable, NumberOfSymbols, SizeOfOptionalHeader, Characteristics.
    let coff = &mut out[0x44..0x58];
    coff[0..2].copy_from_slice(&0x8664u16.to_le_bytes());
    coff[2..4].copy_from_slice(&2u16.to_le_bytes());
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

    // Section headers at 0x58 + 240 = 0x148.
    let headers = 0x148;
    let section = |name: &[u8], rva: u64, virtual_size: usize, pointer: usize, raw: usize, characteristics: u32| {
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
    let text_header = section(b".text", TEXT_RVA, text_size, HEADERS, text_raw, 0x6000_0020);
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

    out
}

//! In-memory fixture builders. Nothing here touches the filesystem or commits a blob:
//! every test object is assembled with the `object` crate's writer.

#![allow(dead_code)]

use object::write;
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationKind, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};

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
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
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

    let root = dwarf.unit.root();
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

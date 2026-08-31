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

/// One row of a fixture's line program: an offset from the start of the generated
/// `.text`, and the source position at it.
pub struct DwarfRow {
    pub address: u64,
    /// An index into [`DwarfFixture::files`].
    pub file: usize,
    /// 0 means "no source line", which is what DWARF's line 0 says.
    pub line: u64,
    /// 0 means the "left edge" of the line, i.e. no column.
    pub column: u64,
}

pub struct DwarfFixture<'a> {
    pub symbols: &'a [TextSymbol<'a>],
    pub comp_dir: &'a str,
    /// The source files the line program can name; `DwarfRow::file` indexes this.
    pub files: &'a [&'a str],
    pub rows: &'a [DwarfRow],
    /// Where the one sequence ends, as an offset into `.text`.
    pub length: u64,
    /// When set, the sequence's `DW_LNE_set_address` operand is written as zero with an
    /// absolute relocation against this symbol, exactly the way a compiler emits it for a
    /// relocatable object. When unset, the address is a constant, as in a linked binary.
    pub base_symbol: Option<usize>,
}

/// Build an x86-64 ELF like [`elf_x86_64`], with a DWARF compilation unit and line
/// program describing its `.text`.
///
/// This is real DWARF written by `gimli::write`, not bytes copied out of a compiler, so
/// the test that reads it back is a round trip through the same formats a compiler emits
/// without needing one installed.
pub fn elf_x86_64_with_dwarf(fixture: DwarfFixture) -> Vec<u8> {
    use gimli::write::{
        Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Sections,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);

    let mut ids = Vec::new();
    for symbol in fixture.symbols {
        let offset = obj.append_section_data(text, symbol.bytes, 1);
        ids.push(obj.add_symbol(write::Symbol {
            name: symbol.name.as_bytes().to_vec(),
            value: offset,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        }));
    }

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

    program.begin_sequence(Some(Address::Constant(0)));
    for row in fixture.rows {
        let current = program.row();
        current.address_offset = row.address;
        current.file = files[row.file];
        current.line = row.line;
        current.column = row.column;
        program.generate_row();
    }
    program.end_sequence(fixture.length);
    dwarf.unit.line_program = program;

    let root = dwarf.unit.root();
    let entry = dwarf.unit.get_mut(root);
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(fixture.comp_dir.as_bytes().to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(fixture.files[0].as_bytes().to_vec()),
    );
    // Without a range on the unit, nothing will look inside it for an address.
    entry.set(gimli::DW_AT_low_pc, AttributeValue::Address(Address::Constant(0)));
    entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(fixture.length));

    let mut sections = Sections::new(EndianVec::new(gimli::LittleEndian));
    dwarf.write(&mut sections).expect("writing the DWARF");

    sections
        .for_each(|id, data| {
            if data.slice().is_empty() {
                return Ok::<_, ()>(());
            }
            let section =
                obj.add_section(Vec::new(), id.name().as_bytes().to_vec(), SectionKind::Debug);
            obj.append_section_data(section, data.slice(), 1);

            if id == gimli::SectionId::DebugLine {
                if let Some(base) = fixture.base_symbol {
                    obj.add_relocation(
                        section,
                        write::Relocation {
                            offset: set_address_operand(data.slice()),
                            size: 64,
                            kind: RelocationKind::Absolute,
                            encoding: RelocationEncoding::Generic,
                            symbol: ids[base],
                            addend: 0,
                        },
                    )
                    .expect("adding a relocation to .debug_line");
                }
            }
            Ok(())
        })
        .expect("laying out the DWARF sections");

    obj.write().expect("writing the fixture object")
}

/// The offset of the 8-byte operand of the one `DW_LNE_set_address` in a `.debug_line`
/// section: the extended-opcode escape `00`, a length of 9, then `DW_LNE_set_address`
/// (`0x02`). A compiler puts a relocation exactly here.
fn set_address_operand(debug_line: &[u8]) -> u64 {
    let pattern = [0x00u8, 0x09, 0x02];
    let mut found = debug_line
        .windows(pattern.len())
        .enumerate()
        .filter(|(_, window)| *window == pattern);
    let (offset, _) = found.next().expect("a DW_LNE_set_address in .debug_line");
    assert!(found.next().is_none(), "more than one DW_LNE_set_address");
    (offset + pattern.len()) as u64
}

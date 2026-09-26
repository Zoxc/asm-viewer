use super::{read_uint, relocate, write_uint};
use crate::sections::{bias_of, section_biases};
use crate::SectionAddress;
use gimli::RunTimeEndian::{Big, Little};
use object::{
    write, Architecture, BinaryFormat, Endianness, Object as _, ObjectSection as _,
    RelocationEncoding, RelocationFlags, RelocationKind, RelocationTarget, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};
use std::cell::Cell;

/// Both widths in both byte orders, the way a relocation's field is read and patched. A
/// 4-byte field takes the low word of what is written.
#[test]
fn relocation_fields_read_and_write_in_either_byte_order() {
    let mut field = [0u8; 4];
    write_uint(&mut field, Little, 0xaaaa_bbbb_0102_0304);
    assert_eq!(field, [4, 3, 2, 1]);
    assert_eq!(read_uint(&field, Little), 0x0102_0304);

    write_uint(&mut field, Big, 0xaaaa_bbbb_0102_0304);
    assert_eq!(field, [1, 2, 3, 4]);
    assert_eq!(read_uint(&field, Big), 0x0102_0304);

    let mut field = [0u8; 8];
    write_uint(&mut field, Little, 0x0102_0304_0506_0708);
    assert_eq!(field, [8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(read_uint(&field, Little), 0x0102_0304_0506_0708);

    write_uint(&mut field, Big, 0x0102_0304_0506_0708);
    assert_eq!(field, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(read_uint(&field, Big), 0x0102_0304_0506_0708);
}

/// A narrower field takes the low bytes of what is written, in either byte order.
#[test]
fn narrow_fields_read_and_write_in_either_byte_order() {
    let mut field = [0u8; 2];
    write_uint(&mut field, Little, 0xaaaa_0102);
    assert_eq!(field, [2, 1]);
    assert_eq!(read_uint(&field, Little), 0x0102);

    write_uint(&mut field, Big, 0xaaaa_0102);
    assert_eq!(field, [1, 2]);
    assert_eq!(read_uint(&field, Big), 0x0102);

    let mut field = [0u8; 1];
    write_uint(&mut field, Big, 0x1ff);
    assert_eq!(field, [0xff]);
    assert_eq!(read_uint(&field, Little), 0xff);
}

/// Any other width reads as 0 and is left as it was, rather than panicking.
#[test]
fn other_widths_are_neither_read_nor_written() {
    for len in [0, 9, 16] {
        let mut field = vec![0xffu8; len];
        assert_eq!(read_uint(&field, Little), 0);
        assert_eq!(read_uint(&field, Big), 0);
        write_uint(&mut field, Little, 0);
        write_uint(&mut field, Big, 0);
        assert!(field.iter().all(|&b| b == 0xff));
    }
}

/// A Mach-O `SUBTRACTOR` pair writes the difference of two symbols, which `object` hands over
/// as one `Absolute` relocation with a subtractor. Here `end - start` in a debug section, with
/// `__text` after 16 bytes of `__const` so neither symbol is at 0: without the subtraction
/// the field reads as `end`'s address, 20, and not 4.
#[test]
fn a_mach_o_subtractor_pair_writes_a_difference() {
    let mut obj = write::Object::new(
        BinaryFormat::MachO,
        Architecture::X86_64,
        Endianness::Little,
    );
    let constants = obj.add_section(
        b"__TEXT".to_vec(),
        b"__const".to_vec(),
        SectionKind::ReadOnlyData,
    );
    obj.append_section_data(constants, &[0; 16], 1);
    let text = obj.add_section(b"__TEXT".to_vec(), b"__text".to_vec(), SectionKind::Text);
    obj.append_section_data(text, &[0x90, 0x90, 0x90, 0xC3], 1);
    let mut symbol = |name: &[u8], value| {
        obj.add_symbol(write::Symbol {
            name: name.to_vec(),
            value,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        })
    };
    let start = symbol(b"start", 0);
    let end = symbol(b"end", 4);
    let debug = obj.add_section(
        b"__DWARF".to_vec(),
        b"__debug_info".to_vec(),
        SectionKind::Debug,
    );
    obj.append_section_data(debug, &[0; 8], 1);
    obj.add_relocation_with_subtractor(
        debug,
        write::Relocation {
            offset: 0,
            symbol: end,
            addend: 0,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Absolute,
                encoding: RelocationEncoding::Generic,
                size: 64,
            },
        },
        Some(start),
    )
    .expect("adding the subtractor pair");
    let bytes = obj.write().expect("writing the fixture object");

    let file = object::File::parse(&*bytes).expect("parsing the fixture object");
    let section = file
        .section_by_name("__debug_info")
        .expect("the fixture has a debug section");
    let mut data = section.data().expect("the debug section reads").to_vec();
    relocate(
        &mut data,
        &file,
        &section,
        Little,
        &section_biases(&file).biases,
        &Cell::new(false),
    );
    assert_eq!(read_uint(&data, Little), 4);
}

/// A Mach-O relocation against a section, rather than a symbol, keeps the whole target
/// address in the bytes, the section's own address included, as an assembler writes it. Here
/// a debug field names 2 bytes into `__StaticInit`, which the file puts at 16 after `__text`:
/// the field comes out as that section's placed start plus 2, and not with its address added
/// twice.
#[test]
fn a_mach_o_section_relocation_counts_the_section_address_once() {
    let mut obj = write::Object::new(
        BinaryFormat::MachO,
        Architecture::X86_64,
        Endianness::Little,
    );
    let text = obj.add_section(b"__TEXT".to_vec(), b"__text".to_vec(), SectionKind::Text);
    obj.append_section_data(text, &[0x90, 0x90, 0x90, 0xC3], 1);
    let init = obj.add_section(
        b"__TEXT".to_vec(),
        b"__StaticInit".to_vec(),
        SectionKind::Text,
    );
    obj.append_section_data(init, &[0x90, 0x90, 0x90, 0xC3], 16);
    let init_symbol = obj.section_symbol(init);
    let debug = obj.add_section(
        b"__DWARF".to_vec(),
        b"__debug_info".to_vec(),
        SectionKind::Debug,
    );
    obj.append_section_data(debug, &[0; 8], 1);
    obj.add_relocation(
        debug,
        write::Relocation {
            offset: 0,
            symbol: init_symbol,
            addend: 0,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Absolute,
                encoding: RelocationEncoding::Generic,
                size: 64,
            },
        },
    )
    .expect("adding the relocation");
    let bytes = obj.write().expect("writing the fixture object");

    let file = object::File::parse(&*bytes).expect("parsing the fixture object");
    let init = file
        .section_by_name("__StaticInit")
        .expect("the fixture has a second code section");
    assert_eq!(init.address(), 16);
    let section = file
        .section_by_name("__debug_info")
        .expect("the fixture has a debug section");
    let (_, relocation) = section.relocations().next().expect("one relocation");
    assert_eq!(relocation.target(), RelocationTarget::Section(init.index()));

    // The `object` writer leaves the offset alone in the bytes; an assembler writes the address.
    let mut data = section.data().expect("the debug section reads").to_vec();
    write_uint(&mut data, Little, init.address() + 2);
    let biases = section_biases(&file).biases;
    relocate(
        &mut data,
        &file,
        &section,
        Little,
        &biases,
        &Cell::new(false),
    );
    let placed = SectionAddress::new(init.address() + 2).placed(biases[&init.index()]);
    assert_eq!(read_uint(&data, Little), placed.get());
}

/// A 32-bit ARM relocatable object states a Thumb function's value with bit 0 set, so a
/// debug field relocated against it would read one byte past the function the parse puts at
/// the even address. Here `thumb_fn` at 1 in `.text` and `odd_datum`, data at 1 in `.data`:
/// the first field comes out at `.text`'s placed start, and the second, whose symbol is no
/// function, a byte into `.data`.
#[test]
fn a_relocation_against_a_thumb_function_resolves_to_its_code() {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::Arm, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0; 8], 4);
    let data = obj.section_id(write::StandardSection::Data);
    obj.append_section_data(data, &[0; 8], 4);
    let mut symbol = |name: &[u8], kind, section| {
        obj.add_symbol(write::Symbol {
            name: name.to_vec(),
            value: 1,
            size: 4,
            kind,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(section),
            flags: SymbolFlags::None,
        })
    };
    let thumb_fn = symbol(b"thumb_fn", SymbolKind::Text, text);
    let odd_datum = symbol(b"odd_datum", SymbolKind::Data, data);
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &[0; 8], 1);
    for (offset, symbol) in [(0, thumb_fn), (4, odd_datum)] {
        obj.add_relocation(
            debug,
            write::Relocation {
                offset,
                symbol,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: object::elf::R_ARM_ABS32,
                },
            },
        )
        .expect("adding a relocation");
    }
    let bytes = obj.write().expect("writing the fixture object");

    let file = object::File::parse(&*bytes).expect("parsing the fixture object");
    let section = file
        .section_by_name(".debug_info")
        .expect("the fixture has a debug section");
    let mut data = section.data().expect("the debug section reads").to_vec();
    let biases = section_biases(&file).biases;
    relocate(
        &mut data,
        &file,
        &section,
        Little,
        &biases,
        &Cell::new(false),
    );
    let placed = |name, offset| {
        let section = file.section_by_name(name).expect("the fixture's section");
        SectionAddress::new(offset)
            .placed(bias_of(&biases, Some(section.index())))
            .get()
    };
    assert_eq!(read_uint(&data[..4], Little), placed(".text", 0));
    assert_eq!(read_uint(&data[4..], Little), placed(".data", 1));
}

/// An ELF relocation with symbol index 0 names no symbol, whose value the ELF spec makes 0,
/// so the field comes out as the addend. `object` calls its target `Absolute`. Skipped, the
/// field kept the 0 the compiler wrote where a `RELA` addend belongs.
#[test]
fn a_relocation_with_no_symbol_writes_its_addend() {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0xC3], 1);
    let function = obj.add_symbol(write::Symbol {
        name: b"function".to_vec(),
        value: 0,
        size: 1,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &[0; 8], 1);
    obj.add_relocation(
        debug,
        write::Relocation {
            offset: 0,
            symbol: function,
            addend: 0x1234,
            flags: RelocationFlags::Elf {
                r_type: object::elf::R_X86_64_64,
            },
        },
    )
    .expect("adding the relocation");
    let mut bytes = obj.write().expect("writing the fixture object");

    // The one `RELA` entry's symbol index, the high half of its second word, set to 0.
    let (start, _) = object::File::parse(&*bytes)
        .expect("parsing the fixture object")
        .section_by_name(".rela.debug_info")
        .and_then(|section| section.file_range())
        .expect("the relocation section");
    let info = start as usize + 8;
    bytes[info + 4..info + 8].fill(0);

    let file = object::File::parse(&*bytes).expect("parsing the patched object");
    let section = file
        .section_by_name(".debug_info")
        .expect("the fixture has a debug section");
    let (_, relocation) = section.relocations().next().expect("one relocation");
    assert_eq!(relocation.target(), RelocationTarget::Absolute);
    let mut data = section.data().expect("the debug section reads").to_vec();
    let lost = Cell::new(false);
    relocate(
        &mut data,
        &file,
        &section,
        Little,
        &section_biases(&file).biases,
        &lost,
    );
    assert_eq!(read_uint(&data, Little), 0x1234);
    assert!(!lost.get());
}

/// A 2-byte field is relocated like a wider one: DWARF for a 16-bit target states its
/// addresses in two bytes. A relocation past the end of the section loses the object's DWARF,
/// as nothing can be written there.
#[test]
fn a_two_byte_field_is_relocated_and_one_outside_the_section_is_lost() {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0xC3], 1);
    let function = obj.add_symbol(write::Symbol {
        name: b"function".to_vec(),
        value: 0,
        size: 1,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &[0; 2], 1);
    for offset in [0, 2] {
        obj.add_relocation(
            debug,
            write::Relocation {
                offset,
                symbol: function,
                addend: 0x1234,
                flags: RelocationFlags::Elf {
                    r_type: object::elf::R_X86_64_16,
                },
            },
        )
        .expect("adding a relocation");
    }
    let bytes = obj.write().expect("writing the fixture object");

    let file = object::File::parse(&*bytes).expect("parsing the fixture object");
    let section = file
        .section_by_name(".debug_info")
        .expect("the fixture has a debug section");
    let mut data = section.data().expect("the debug section reads").to_vec();
    let lost = Cell::new(false);
    relocate(
        &mut data,
        &file,
        &section,
        Little,
        &section_biases(&file).biases,
        &lost,
    );
    assert_eq!(read_uint(&data, Little), 0x1234);
    assert!(lost.get());
}

/// Relocate `.debug_info`, the one debug section `obj` has, as the load does, and whether a
/// relocation would not apply.
fn relocated(obj: write::Object, endian: gimli::RunTimeEndian) -> (Vec<u8>, bool) {
    let bytes = obj.write().expect("writing the fixture object");
    let file = object::File::parse(&*bytes).expect("parsing the fixture object");
    let section = file
        .section_by_name(".debug_info")
        .expect("the fixture has a debug section");
    let mut data = section.data().expect("the debug section reads").to_vec();
    let lost = Cell::new(false);
    relocate(
        &mut data,
        &file,
        &section,
        endian,
        &section_biases(&file).biases,
        &lost,
    );
    (data, lost.get())
}

/// RISC-V states a length in a debug section as two labels' difference, a pair of
/// relocations at one field: `ADD` the end, `SUB` the start. Every width a compiler writes
/// one in comes out as `end - start`, 0x90 here: 4 and 2 bytes, the low six bits of a
/// byte (its top two kept), and a ULEB128 of the length it was written in.
#[test]
fn a_risc_v_pair_writes_the_difference_of_its_labels() {
    use object::elf::*;

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::Riscv64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0; 0x100], 4);
    let mut label = |name: &[u8], value| {
        obj.add_symbol(write::Symbol {
            name: name.to_vec(),
            value,
            size: 0,
            kind: SymbolKind::Label,
            scope: SymbolScope::Compilation,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        })
    };
    let start = label(b".Lstart", 0x10);
    let end = label(b".Lend", 0xa0);
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &[0, 0, 0, 0, 0, 0, 0x40, 0x80, 0], 1);
    let pairs = [
        (0, R_RISCV_ADD32, R_RISCV_SUB32),
        (4, R_RISCV_ADD16, R_RISCV_SUB16),
        (6, R_RISCV_SET6, R_RISCV_SUB6),
        (7, R_RISCV_SET_ULEB128, R_RISCV_SUB_ULEB128),
    ];
    for (offset, first, second) in pairs {
        for (symbol, r_type) in [(end, first), (start, second)] {
            obj.add_relocation(
                debug,
                write::Relocation {
                    offset,
                    symbol,
                    addend: 0,
                    flags: RelocationFlags::Elf { r_type },
                },
            )
            .expect("adding a relocation");
        }
    }

    let (data, lost) = relocated(obj, Little);
    assert_eq!(read_uint(&data[0..4], Little), 0x90);
    assert_eq!(read_uint(&data[4..6], Little), 0x90);
    assert_eq!(data[6], 0x40 | 0x10);
    assert_eq!(data[7..9], [0x90, 0x01]);
    assert!(!lost);
}

/// A relocation of a kind not applied loses the object's DWARF, where it was left holding the
/// compiler's 0. A thread-local variable's offset does not: nothing is read from it.
#[test]
fn a_relocation_of_a_kind_not_applied_loses_the_dwarf() {
    let one = |r_type| {
        let mut obj =
            write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
        let text = obj.section_id(write::StandardSection::Text);
        obj.append_section_data(text, &[0xC3], 1);
        let function = obj.add_symbol(write::Symbol {
            name: b"function".to_vec(),
            value: 0,
            size: 1,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        });
        let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
        obj.append_section_data(debug, &[0; 4], 1);
        obj.add_relocation(
            debug,
            write::Relocation {
                offset: 0,
                symbol: function,
                addend: 0,
                flags: RelocationFlags::Elf { r_type },
            },
        )
        .expect("adding the relocation");
        relocated(obj, Little).1
    };

    assert!(one(object::elf::R_X86_64_PC32));
    assert!(!one(object::elf::R_X86_64_DTPOFF32));
}

/// A COFF `SECREL` states where its symbol is in the symbol's own section, which is how a
/// MinGW object points one debug section into another. Skipped, the field kept only the
/// addend, 4, and lost the symbol's 8.
#[test]
fn a_coff_secrel_writes_the_offset_in_its_section() {
    let mut obj = write::Object::new(BinaryFormat::Coff, Architecture::X86_64, Endianness::Little);
    let line = obj.add_section(Vec::new(), b".debug_line".to_vec(), SectionKind::Debug);
    obj.append_section_data(line, &[0; 16], 1);
    let unit = obj.add_symbol(write::Symbol {
        name: b"unit".to_vec(),
        value: 8,
        size: 0,
        kind: SymbolKind::Data,
        scope: SymbolScope::Compilation,
        weak: false,
        section: write::SymbolSection::Section(line),
        flags: SymbolFlags::None,
    });
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &[0; 4], 1);
    obj.add_relocation(
        debug,
        write::Relocation {
            offset: 0,
            symbol: unit,
            addend: 4,
            flags: RelocationFlags::Coff {
                typ: object::pe::IMAGE_REL_AMD64_SECREL,
            },
        },
    )
    .expect("adding the relocation");

    let (data, lost) = relocated(obj, Little);
    assert_eq!(read_uint(&data, Little), 12);
    assert!(!lost);
}

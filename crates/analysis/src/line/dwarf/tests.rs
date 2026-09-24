use super::{read_uint, relocate, write_uint};
use crate::sections::{bias_of, section_biases};
use crate::SectionAddress;
use gimli::RunTimeEndian::{Big, Little};
use object::{
    write, Architecture, BinaryFormat, Endianness, Object as _, ObjectSection as _,
    RelocationEncoding, RelocationFlags, RelocationKind, RelocationTarget, SectionKind,
    SymbolFlags, SymbolKind, SymbolScope,
};

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

/// Any other width reads as 0 and is left as it was, rather than panicking.
#[test]
fn other_widths_are_neither_read_nor_written() {
    for len in [0, 1, 2, 3, 5, 7, 9] {
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
    relocate(&mut data, &file, &section, Little, &biases);
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
    relocate(&mut data, &file, &section, Little, &biases);
    let placed = |name, offset| {
        let section = file.section_by_name(name).expect("the fixture's section");
        SectionAddress::new(offset)
            .placed(bias_of(&biases, Some(section.index())))
            .get()
    };
    assert_eq!(read_uint(&data[..4], Little), placed(".text", 0));
    assert_eq!(read_uint(&data[4..], Little), placed(".data", 1));
}

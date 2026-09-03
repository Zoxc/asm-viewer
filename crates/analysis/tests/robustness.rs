//! One named, minimal fixture per defect actually found by `mutations.rs`' sweep, plus the
//! small sweeps that came first. Every note below says which defect its test pins.

mod common;

use analysis::{parse_object, Object};
use common::{
    caller_and_target, committed_fixture, declared_code_images, dwarf_fixture, elf_x86_64, garbage,
    parse_and_walk, survivors, TextRelocation, TextSymbol,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

#[test]
fn garbage_is_rejected_without_panicking() {
    for seed in 1..64u64 {
        for len in [0usize, 1, 4, 16, 64, 1024] {
            let data = garbage(seed, len);
            let parsed = catch_unwind(AssertUnwindSafe(|| parse_and_walk(&data)))
                .unwrap_or_else(|_| panic!("panicked on garbage(seed = {seed}, len = {len})"));
            assert!(
                parsed.is_none(),
                "garbage(seed = {seed}, len = {len}) parsed as an object"
            );
        }
    }
}

#[test]
fn plausible_looking_headers_are_rejected_without_panicking() {
    // Right magic, wrong everything else — the case a length check is most likely to miss.
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for magic in [
        &b"\x7fELF"[..],
        &b"MZ"[..],
        &b"\xfe\xed\xfa\xcf"[..],
        &b"\xcf\xfa\xed\xfe"[..],
        &b"\0asm"[..],
        &b"!<arch>\n"[..],
    ] {
        for len in [0usize, 8, 64, 512] {
            let mut data = magic.to_vec();
            data.extend(garbage(0xC0FFEE, len));
            cases.push((format!("{magic:?} + {len} garbage bytes"), data));
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

#[test]
fn truncated_objects_do_not_panic() {
    let valid = caller_and_target();
    let failures =
        survivors((0..valid.len()).map(|len| (format!("truncated to {len} bytes"), &valid[..len])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");

    // Sanity check that the fixture itself is the thing being truncated.
    assert!(parse_and_walk(&valid).is_some());
}

/// Byte 9 of an ELF64 section header is the second byte of `sh_flags`, which holds
/// `SHF_COMPRESSED` (0x800).
fn section_flag_bytes(elf: &[u8]) -> Vec<usize> {
    let shoff = u64::from_le_bytes(elf[0x28..0x30].try_into().unwrap()) as usize;
    let shentsize = u16::from_le_bytes(elf[0x3A..0x3C].try_into().unwrap()) as usize;
    let shnum = u16::from_le_bytes(elf[0x3C..0x3E].try_into().unwrap()) as usize;
    (0..shnum).map(|i| shoff + i * shentsize + 9).collect()
}

#[test]
fn corrupted_objects_do_not_panic() {
    let valid = caller_and_target();

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for offset in 0..valid.len() {
        for mask in [0xFFu8, 0x01, 0x80] {
            let mut data = valid.clone();
            data[offset] ^= mask;
            cases.push((format!("byte {offset} ^ {mask:#04x}"), data));
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// Defect: one flipped bit turning on `SHF_COMPRESSED` in `.rela.text`'s header made the
/// bytes after it read as an `Elf64_Chdr` announcing 8.6 GB of output, and
/// `uncompressed_data()` reserved all of it before reading a compressed byte — never a
/// panic, so the sweep above stayed green, but an OOM abort. The declared size is now
/// weighed against the compressed bytes and the section is dropped instead.
#[test]
fn a_lying_compressed_size_in_a_section_header_costs_nothing() {
    let valid = caller_and_target();
    let baseline = parse_and_walk(&valid).expect("the fixture parses");
    assert!(section_names(&baseline).contains(&".rela.text".to_owned()));

    let offset = section_flag_bytes(&valid)[2];
    let mut data = valid.clone();
    data[offset] ^= 0xFF;

    let object = parse_and_walk(&data).expect("the object still parses");

    // The section that claimed 8.6 GB is gone, not decompressed.
    assert!(!section_names(&object).contains(&".rela.text".to_owned()));

    // Nothing else grew: no section holds more bytes than the whole file has.
    for section in &object.sections {
        assert!(
            section.data.len() <= data.len(),
            "section {} holds {} bytes of a {}-byte file",
            section.name,
            section.data.len(),
            data.len()
        );
    }

    // And the rest of the object is untouched: `.text` and its symbols still decode.
    assert_eq!(
        section_names(&object).len(),
        section_names(&baseline).len() - 1
    );
    for symbol in &object.symbols_sorted {
        assert!(symbol.data().is_some(), "{} lost its data", symbol.name);
    }
}

/// The same guard, on a *valid* zlib stream whose header lies about its output size: it
/// has to fire on the declared size alone, before decompressing. Both bounds are exercised
/// — past the 1 GiB cap, and past what DEFLATE could produce from these bytes.
#[test]
fn a_valid_zlib_stream_is_only_decompressed_when_its_declared_size_is_believable() {
    let payload = b"decompressed section contents";

    let honest = parse_and_walk(&elf_with_compressed_section(payload, payload.len() as u64))
        .expect("parses");
    let section = honest
        .sections
        .iter()
        .find(|section| section.name == ".debug_info")
        .expect("an honestly sized compressed section is kept");
    assert_eq!(section.data, payload);

    for declared in [
        1u64 << 33, // past the absolute cap.
        1 << 20,    // under the cap, but ~36000:1 from 29 bytes: past what DEFLATE can do.
    ] {
        let data = elf_with_compressed_section(payload, declared);
        let object = parse_and_walk(&data).expect("the object still parses");
        assert!(
            !section_names(&object).contains(&".debug_info".to_owned()),
            "a section declaring {declared} bytes was decompressed anyway"
        );
    }
}

fn section_names(object: &Object) -> Vec<String> {
    object
        .sections
        .iter()
        .map(|section| section.name.clone())
        .collect()
}

/// An ELF holding one `SHF_COMPRESSED` `.debug_info` whose zlib stream really does decode
/// to `payload`, but whose compression header declares `declared_size` bytes of output.
fn elf_with_compressed_section(payload: &[u8], declared_size: u64) -> Vec<u8> {
    use object::{write, Architecture, BinaryFormat, Endianness, SectionFlags, SectionKind};

    let mut contents = Vec::new();
    // Elf64_Chdr: ch_type = ELFCOMPRESS_ZLIB, ch_reserved, ch_size, ch_addralign.
    contents.extend_from_slice(&1u32.to_le_bytes());
    contents.extend_from_slice(&0u32.to_le_bytes());
    contents.extend_from_slice(&declared_size.to_le_bytes());
    contents.extend_from_slice(&1u64.to_le_bytes());
    contents.extend_from_slice(&zlib_stored(payload));

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let id = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(id, &contents, 1);
    obj.section_mut(id).flags = SectionFlags::Elf {
        sh_flags: object::elf::SHF_COMPRESSED.into(),
    };
    obj.write().expect("writing the fixture object")
}

/// `payload` as a valid zlib stream: one final DEFLATE block, stored rather than
/// compressed, so no compressor is needed to build the fixture.
fn zlib_stored(payload: &[u8]) -> Vec<u8> {
    let len: u16 = payload.len().try_into().expect("a small payload");

    let mut out = vec![0x78, 0x01]; // CMF/FLG for a 32 KiB window, no dictionary.
    out.push(0x01); // BFINAL = 1, BTYPE = 00 (stored).
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(payload);

    let (mut a, mut b) = (1u32, 0u32);
    for byte in payload {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

#[test]
fn assembly_of_data_ending_mid_instruction_is_partial_not_a_panic() {
    // Defect: a symbol whose bytes stop inside an instruction, because the next symbol
    // starts there. `caller` is three bytes of a five-byte `call rel32`.
    let data = elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xE8, 0x00, 0x00],
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
    );

    let object =
        parse_object(data[..].into(), "trunc.o".into(), PathBuf::from("/trunc.o")).expect("parses");
    let caller = object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == "caller")
        .expect("caller parses")
        .clone();

    assert_eq!(caller.estimate_size(), Some(3));
    assert_eq!(caller.data(), Some(&[0xE8, 0x00, 0x00][..]));

    let assembly = caller
        .assembly(&object)
        .expect("a partial decode still yields output");
    assert!(!assembly.instructions.is_empty());

    // Whatever came out must stay inside the symbol's own bytes.
    let decoded: usize = assembly
        .instructions
        .iter()
        .map(|instruction| instruction.bytes.len())
        .sum();
    assert_eq!(decoded, 3);
}

#[test]
fn every_truncation_of_a_symbols_bytes_decodes_without_panicking() {
    // The decoder running out of bytes at every position inside an instruction.
    for len in 1..=6usize {
        let bytes = &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3][..len];
        let data = elf_x86_64(
            &[
                TextSymbol {
                    name: "caller",
                    bytes,
                },
                TextSymbol {
                    name: "target",
                    bytes: &[0xC3],
                },
            ],
            &[TextRelocation {
                in_symbol: 0,
                offset: 0,
                target: 1,
            }],
        );

        catch_unwind(AssertUnwindSafe(|| parse_and_walk(&data)))
            .unwrap_or_else(|_| panic!("panicked decoding the first {len} bytes"));
    }
}

#[test]
fn a_symbol_outside_any_section_yields_no_data() {
    use object::{
        write, Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
    };

    // Defect: an absolute symbol is still `SymbolKind::Text` but belongs to no section, so
    // every size/data/disassembly path has to bail out instead of indexing something.
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    obj.add_symbol(write::Symbol {
        name: b"absolute_fn".to_vec(),
        value: 0x1000,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: write::SymbolSection::Absolute,
        flags: SymbolFlags::None,
    });
    let data = obj.write().expect("writing the fixture object");

    let object =
        parse_object(data[..].into(), "abs.o".into(), PathBuf::from("/abs.o")).expect("parses");
    let symbol = object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == "absolute_fn")
        .expect("the absolute symbol parses")
        .clone();

    assert!(symbol.section.is_none());
    assert_eq!(symbol.estimate_size(), None);
    assert_eq!(symbol.data(), None);
    assert!(symbol.assembly(&object).is_none());
}

/// The byte ranges of every `.debug_*` section in an ELF, from its section table.
fn debug_section_ranges(elf: &[u8]) -> Vec<std::ops::Range<usize>> {
    let object = object::File::parse(elf).expect("the fixture parses");
    use object::{Object as _, ObjectSection};
    object
        .sections()
        .filter(|section| {
            section
                .name()
                .map(|name| name.starts_with(".debug_"))
                .unwrap_or(false)
        })
        .filter_map(|section| {
            let (offset, size) = section.file_range()?;
            let start = usize::try_from(offset).ok()?;
            Some(start..start + usize::try_from(size).ok()?)
        })
        .collect()
}

/// Garbage where DWARF should be must degrade to "no line info", never abort. Flipping
/// every byte of every `.debug_*` section keeps the ELF valid and aims the damage at
/// `gimli`.
#[test]
fn corrupted_debug_sections_do_not_panic() {
    let valid = dwarf_fixture(&[]);
    let ranges = debug_section_ranges(&valid);
    assert!(!ranges.is_empty(), "the fixture has .debug_* sections");
    assert!(
        parse_and_walk(&valid).is_some(),
        "the fixture itself parses and resolves"
    );

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for range in &ranges {
        for offset in range.clone() {
            for mask in [0xFFu8, 0x01, 0x80] {
                let mut data = valid.clone();
                data[offset] ^= mask;
                cases.push((format!("debug byte {offset} ^ {mask:#04x}"), data));
            }
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The same sweep with whole runs of random bytes rather than single flips, so a mutation
/// is not limited to one field of one record.
#[test]
fn debug_sections_full_of_garbage_do_not_panic() {
    let valid = dwarf_fixture(&[]);
    let ranges = debug_section_ranges(&valid);

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for seed in 1..48u64 {
        for range in &ranges {
            let mut data = valid.clone();
            let noise = garbage(seed, range.len());
            data[range.clone()].copy_from_slice(&noise);
            cases.push((
                format!("section {range:?} replaced with garbage({seed})"),
                data,
            ));
        }

        // And every section at once: a file that is not really DWARF but says it is.
        let mut data = valid.clone();
        for range in &ranges {
            let noise = garbage(seed.wrapping_mul(31), range.len());
            data[range.clone()].copy_from_slice(&noise);
        }
        cases.push((format!("every debug section garbage({seed})"), data));
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The compressed-section defect again on the DWARF loader's path — the one that is
/// *expected* to meet compressed sections, `.debug_*` being what compilers compress. The
/// loader goes through the same `section_data` guard, so the section reads as absent.
#[test]
fn a_lying_compressed_debug_section_costs_nothing() {
    let payload = b"not really DWARF, but it is not read either";

    for declared in [1u64 << 33, 1 << 20] {
        let data = elf_with_compressed_section(payload, declared);
        let object = parse_and_walk(&data).expect("the object still parses");
        assert!(
            !section_names(&object).contains(&".debug_info".to_owned()),
            "a .debug_info declaring {declared} bytes was decompressed anyway"
        );
        for section in &object.sections {
            assert!(object.line_info(section, 0..u64::MAX).is_none());
        }
    }
}

/// Defect: `addr2line` 0.21 computes a row's length as `next.address - row.address`
/// unchecked, and a line program may legally move its address backwards — a
/// subtract-with-overflow panic on a file the app merely opened. Caught by
/// `without_panicking` in `src/line.rs`, so the panic message the run prints is expected.
#[test]
fn a_line_program_that_runs_backwards_does_not_panic() {
    let data = elf_with_backwards_line_program();

    let object = catch_unwind(AssertUnwindSafe(|| parse_and_walk(&data)))
        .expect("a backwards line program is caught, not propagated")
        .expect("the object still parses");

    for section in &object.sections {
        assert!(object.line_info(section, 0..0x400).is_none());
    }
    for symbol in &object.symbols_sorted {
        assert!(symbol.line_info(&object).is_none());
    }
}

/// An ELF whose DWARF is written by hand, because no writer will produce this: both
/// `gimli::write` and every real compiler assert that a sequence's addresses ascend.
fn elf_with_backwards_line_program() -> Vec<u8> {
    use object::{write, Architecture, BinaryFormat, Endianness, SectionKind};

    fn uleb(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return;
            }
        }
    }

    // .debug_abbrev: one abbreviation, a compile unit with low_pc/high_pc/stmt_list.
    let mut abbrev = Vec::new();
    uleb(&mut abbrev, 1);
    uleb(&mut abbrev, 0x11); // DW_TAG_compile_unit
    abbrev.push(0); // no children
    for (attribute, form) in [
        (0x11u64, 0x01u64), // DW_AT_low_pc,    DW_FORM_addr
        (0x12, 0x07),       // DW_AT_high_pc,   DW_FORM_data8
        (0x10, 0x17),       // DW_AT_stmt_list, DW_FORM_sec_offset
    ] {
        uleb(&mut abbrev, attribute);
        uleb(&mut abbrev, form);
    }
    uleb(&mut abbrev, 0);
    uleb(&mut abbrev, 0);
    uleb(&mut abbrev, 0); // end of the abbreviation table

    // .debug_info: that one unit, covering 0..0x400 and pointing at the line program.
    let mut die = vec![1u8];
    die.extend_from_slice(&0u64.to_le_bytes()); // low_pc
    die.extend_from_slice(&0x400u64.to_le_bytes()); // high_pc
    die.extend_from_slice(&0u32.to_le_bytes()); // stmt_list
    die.push(0); // end of children

    let mut info = Vec::new();
    info.extend_from_slice(&((2 + 4 + 1 + die.len()) as u32).to_le_bytes()); // unit_length
    info.extend_from_slice(&4u16.to_le_bytes()); // version
    info.extend_from_slice(&0u32.to_le_bytes()); // debug_abbrev offset
    info.push(8); // address size
    info.extend_from_slice(&die);

    // .debug_line: a DWARF 4 header, then the sequence that walks backwards.
    let mut header = vec![
        1,    // minimum_instruction_length
        1,    // maximum_operations_per_instruction
        1,    // default_is_stmt
        0xFB, // line_base = -5
        14,   // line_range
        13,   // opcode_base
    ];
    header.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]); // standard_opcode_lengths
    header.push(0); // no include_directories
    header.extend_from_slice(b"a.c\0");
    uleb(&mut header, 0); // directory index
    uleb(&mut header, 0); // mtime
    uleb(&mut header, 0); // length
    header.push(0); // end of file_names

    let mut program = Vec::new();
    program.extend_from_slice(&[0x00, 0x09, 0x02]); // DW_LNE_set_address
    program.extend_from_slice(&0x100u64.to_le_bytes());
    program.push(0x01); // DW_LNS_copy: a row at 0x100
    program.extend_from_slice(&[0x00, 0x09, 0x02]); // DW_LNE_set_address, backwards
    program.extend_from_slice(&0u64.to_le_bytes());
    program.extend_from_slice(&[0x00, 0x01, 0x01]); // DW_LNE_end_sequence, at 0

    let mut line = Vec::new();
    line.extend_from_slice(&((2 + 4 + header.len() + program.len()) as u32).to_le_bytes());
    line.extend_from_slice(&4u16.to_le_bytes()); // version
    line.extend_from_slice(&(header.len() as u32).to_le_bytes()); // header_length
    line.extend_from_slice(&header);
    line.extend_from_slice(&program);

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0xC3], 1);
    for (name, contents) in [
        (".debug_abbrev", abbrev),
        (".debug_info", info),
        (".debug_line", line),
    ] {
        let id = obj.add_section(Vec::new(), name.as_bytes().to_vec(), SectionKind::Debug);
        obj.append_section_data(id, &contents, 1);
    }
    obj.write().expect("writing the fixture object")
}

/// The committed, compiler-produced objects (`tests/fixtures/`). These are DWARF 5 — a
/// `.debug_line_str`, a version 5 line-program header, a `.debug_rnglists` — where the
/// written fixture above is DWARF 4 with its strings inline, so mutating them aims at
/// `gimli` paths the synthesized one never reaches.
fn committed_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    ["line_fixture.o", "line_fixture_split.o"]
        .into_iter()
        .map(|name| (name, committed_fixture(name)))
        .collect()
}

/// Both sweeps again over real DWARF 5 rather than the written DWARF 4 above.
#[test]
fn corrupted_debug_sections_of_a_real_object_do_not_panic() {
    for (name, valid) in committed_fixtures() {
        let ranges = debug_section_ranges(&valid);
        assert!(!ranges.is_empty(), "{name} has .debug_* sections");
        assert!(
            parse_and_walk(&valid).is_some(),
            "{name} itself parses and resolves"
        );

        let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
        for range in &ranges {
            for offset in range.clone() {
                for mask in [0xFFu8, 0x01, 0x80] {
                    let mut data = valid.clone();
                    data[offset] ^= mask;
                    cases.push((format!("{name}: byte {offset} ^ {mask:#04x}"), data));
                }
            }

            for seed in 1..16u64 {
                let mut data = valid.clone();
                let noise = garbage(seed, range.len());
                data[range.clone()].copy_from_slice(&noise);
                cases.push((format!("{name}: {range:?} = garbage({seed})"), data));
            }
        }

        let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
        assert!(failures.is_empty(), "panicked on: {failures:?}");
    }
}

#[test]
fn truncated_images_do_not_panic() {
    for (kind, valid, functions) in declared_code_images() {
        // Sanity check that the fixture is an image the parse can read.
        let object = parse_and_walk(&valid).expect("the image parses");
        assert_eq!(object.symbols_sorted.len(), functions, "{kind}");

        let failures = survivors(
            (0..valid.len()).map(|len| (format!("{kind} truncated to {len}"), &valid[..len])),
        );
        assert!(failures.is_empty(), "panicked on: {failures:?}");
    }
}

#[test]
fn corrupted_images_do_not_panic() {
    for (kind, valid, _) in declared_code_images() {
        let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
        for offset in 0..valid.len() {
            for mask in [0xFFu8, 0x01, 0x80] {
                let mut data = valid.clone();
                data[offset] ^= mask;
                cases.push((format!("{kind} byte {offset} ^ {mask:#04x}"), data));
            }
        }

        let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
        assert!(failures.is_empty(), "panicked on: {failures:?}");
    }
}

/// Defect: a mangled name is bytes out of a string table and says how deep the demangler
/// reading it recurses — one level per byte for `msvc-demangler` 0.11, which has no limit
/// at all — so a long name was a **stack overflow**, an abort no `catch_unwind` catches.
/// Before the fix this did not fail, it killed the test binary. Headed off before the call
/// by `MAX_MANGLED_NAME` and a stack sized for the rest (`demangled` in `src/lib.rs`).
#[test]
fn a_deeply_nested_name_does_not_overflow_the_stack() {
    // 1000 levels overflows the 8 MiB this test's own thread has, let alone the 2 MiB the
    // viewer's parse thread gets; 4000 is past the cap as well. The last name is the
    // control: an ordinary one, in the same batch.
    let names: Vec<Vec<u8>> = vec![
        format!("?f@@YAX{}@Z", "P".repeat(1000)).into_bytes(),
        format!("?g@@YAX{}@Z", "P".repeat(4000)).into_bytes(),
        format!("_Z1f{}v", "P".repeat(1000)).into_bytes(),
        b"_ZN4core3fmt9Formatter12pad_integral17h0123456789abcdefE".to_vec(),
    ];

    let data = elf_with_names(&names);
    let object = parse_object(data[..].into(), "deep.o".into(), PathBuf::from("/deep.o"))
        .expect("the object parses");

    assert_eq!(object.symbols_sorted.len(), names.len());
    // A name no demangler could take is displayed exactly as the file wrote it.
    for symbol in &object.symbols_sorted {
        assert!(!symbol.display().is_empty());
    }

    let named = |name: &str| {
        object
            .symbols_sorted
            .iter()
            .find(|symbol| symbol.name.starts_with(name))
            .unwrap_or_else(|| panic!("{name} is listed"))
            .clone()
    };
    // Past the cap: not demangled, and that is the whole cost of the guard.
    assert_eq!(named("?g@@").demangled, None);
    // Under it: demangled on the thread `demangled` spawns, whatever it makes of it.
    let _ = named("?f@@").demangled;
    assert_eq!(
        named("_ZN4core").demangled.as_deref(),
        Some("core::fmt::Formatter::pad_integral")
    );
}

/// An ELF whose `.text` holds one one-byte function per name.
fn elf_with_names(names: &[Vec<u8>]) -> Vec<u8> {
    use object::{
        write, Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    for name in names {
        let offset = obj.append_section_data(text, &[0xC3], 1);
        obj.add_symbol(write::Symbol {
            name: name.clone(),
            value: offset,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        });
    }
    obj.write().expect("writing the fixture object")
}

/// Defect: a section header may say its bytes live at the end of the address space, and
/// the debug info may then claim a function running off the end of it. Four independent
/// bounds have to hold at once, and this is the one input in the suite that walks all of
/// them: `estimate_size`'s and the DWARF backend's checked adds, the caught panic from
/// `addr2line` 0.21's unchecked `low_pc + high_pc`, and `assembly` declining to decode a
/// symbol with no extent rather than one decoded from an address that wrapped.
#[test]
fn a_function_at_the_end_of_the_address_space_does_not_panic() {
    // Six single-byte instructions from three below the top of the address space: the
    // fourth is at `u64::MAX`, and the debug info claims two bytes more than that.
    const BASE: u64 = u64::MAX - 3;

    let data = elf_at_the_end_of_the_address_space(BASE);
    let object = parse_and_walk(&data).expect("the object parses");
    let symbol = object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == "edge")
        .expect("the symbol is listed")
        .clone();

    assert_eq!(symbol.address, BASE);
    assert_eq!(symbol.estimate_size(), None);
    assert_eq!(symbol.extent(&object), None);
    assert_eq!(symbol.data_in(&object), None);
    assert!(symbol.assembly(&object).is_none());
    assert!(symbol.line_info(&object).is_none());
}

/// An ELF **image** (`ET_EXEC`, so nothing is biased and DWARF addresses are read
/// literally) whose `.text` sits at `base` and whose debug info claims six bytes for the
/// function there. Written with the two writers and then patched, since the `object`
/// writer only emits relocatable objects and only three numbers have to change.
fn elf_at_the_end_of_the_address_space(base: u64) -> Vec<u8> {
    use gimli::write::{
        Address, AttributeValue, DwarfUnit, EndianVec, LineProgram, LineString, Range, RangeList,
        Sections,
    };
    use object::{
        write, Architecture, BinaryFormat, Endianness, ObjectSymbol as _, SectionKind, SymbolFlags,
        SymbolKind, SymbolScope,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    obj.append_section_data(text, &[0x90, 0x90, 0x90, 0x90, 0x90, 0xC3], 1);
    obj.add_symbol(write::Symbol {
        name: b"edge".to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: write::SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

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
        LineString::String(b"edge.c".to_vec()),
        None,
    );
    program.begin_sequence(Some(Address::Constant(base)));
    let row = program.row();
    row.address_offset = 0;
    row.line = 1;
    program.generate_row();
    // Three bytes, so the sequence ends exactly at `u64::MAX` rather than past it.
    program.end_sequence(3);
    dwarf.unit.line_program = program;

    let root = dwarf.unit.root();
    let subprogram = dwarf.unit.add(root, gimli::DW_TAG_subprogram);
    let entry = dwarf.unit.get_mut(subprogram);
    entry.set(gimli::DW_AT_name, AttributeValue::String(b"edge".to_vec()));
    entry.set(
        gimli::DW_AT_low_pc,
        AttributeValue::Address(Address::Constant(base)),
    );
    // Six: two bytes more than there is address space, which is the point of the fixture.
    entry.set(gimli::DW_AT_high_pc, AttributeValue::Udata(6));

    // A range list rather than the unit's own `low_pc`/`high_pc`, so the unit stops at
    // the top of the address space while the subprogram inside it over-reaches.
    let ranges = dwarf.unit.ranges.add(RangeList(vec![Range::StartLength {
        begin: Address::Constant(base),
        length: 3,
    }]));
    let entry = dwarf.unit.get_mut(root);
    entry.set(gimli::DW_AT_ranges, AttributeValue::RangeListRef(ranges));
    entry.set(
        gimli::DW_AT_comp_dir,
        AttributeValue::String(b"/src".to_vec()),
    );
    entry.set(
        gimli::DW_AT_name,
        AttributeValue::String(b"edge.c".to_vec()),
    );

    let mut sections = Sections::new(EndianVec::new(gimli::LittleEndian));
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
            Ok(())
        })
        .expect("laying out the DWARF sections");

    let mut data = obj.write().expect("writing the fixture object");

    // ET_REL -> ET_EXEC: an image, so `section_biases` leaves its addresses alone.
    data[16..18].copy_from_slice(&2u16.to_le_bytes());

    // And the two numbers that put the function where it is.
    let (text_header, symbol_entry) = {
        use object::{Object as _, ObjectSection as _};
        let file = object::File::parse(&data[..]).expect("the fixture parses");
        let shoff = u64::from_le_bytes(data[0x28..0x30].try_into().unwrap()) as usize;
        let shentsize = u16::from_le_bytes(data[0x3A..0x3C].try_into().unwrap()) as usize;
        let text = file
            .section_by_name(".text")
            .expect(".text is there")
            .index();
        let symtab = file
            .section_by_name(".symtab")
            .expect(".symtab is there")
            .file_range()
            .expect(".symtab is in the file")
            .0 as usize;
        let index = file
            .symbols()
            .find(|symbol| symbol.name() == Ok("edge"))
            .expect("the symbol is there")
            .index()
            .0;
        // sh_addr is 16 bytes into a section header; st_value 8 into a 24-byte symbol.
        (shoff + text.0 * shentsize + 16, symtab + index * 24 + 8)
    };
    data[text_header..text_header + 8].copy_from_slice(&base.to_le_bytes());
    data[symbol_entry..symbol_entry + 8].copy_from_slice(&base.to_le_bytes());

    data
}

/// Defect: a single wild `st_value` — a symbol whose address is nowhere in its own section
/// — used to spread, because `estimate_size` derived the *previous* symbol's extent from
/// it and so left the readable function above it with no listing. The derivation is
/// clipped to the section's own bytes now.
///
/// The fixture puts that beside three failures that are already scoped to one symbol (an
/// absolute symbol, a name past `MAX_MANGLED_NAME`, and bytes that stop mid-instruction),
/// which is what makes the point: one symbol that cannot be analysed is one row without a
/// listing, not an object without one.
#[test]
fn one_symbol_that_cannot_be_analysed_does_not_take_out_the_others() {
    use object::{
        write, Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope,
    };

    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);
    let add = |obj: &mut write::Object, name: Vec<u8>, bytes: &[u8], section| {
        let value = match section {
            write::SymbolSection::Section(_) => obj.append_section_data(text, bytes, 1),
            _ => 0x1000,
        };
        obj.add_symbol(write::Symbol {
            name,
            value,
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section,
            flags: SymbolFlags::None,
        });
    };

    let in_text = write::SymbolSection::Section(text);
    // `xor eax, eax; ret`, twice, around everything that cannot be read.
    add(
        &mut obj,
        b"first_good".to_vec(),
        &[0x31, 0xC0, 0xC3],
        in_text,
    );
    add(
        &mut obj,
        b"absolute".to_vec(),
        &[],
        write::SymbolSection::Absolute,
    );
    add(
        &mut obj,
        format!("?deep@@YAX{}@Z", "P".repeat(3000)).into_bytes(),
        // Three bytes of a five-byte `call rel32`: the next symbol starts inside it.
        &[0xE8, 0x00, 0x00],
        in_text,
    );
    add(&mut obj, b"cut_short".to_vec(), &[0xE8, 0x00], in_text);
    add(
        &mut obj,
        b"second_good".to_vec(),
        &[0x31, 0xC0, 0xC3],
        in_text,
    );
    let mut data = obj.write().expect("writing the fixture object");

    // And one symbol pointed clean out of its own section, which the writer will not emit.
    {
        use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
        let file = object::File::parse(&data[..]).expect("the fixture parses");
        let symtab = file
            .section_by_name(".symtab")
            .expect(".symtab is there")
            .file_range()
            .expect(".symtab is in the file")
            .0 as usize;
        let index = file
            .symbols()
            .find(|symbol| symbol.name() == Ok("cut_short"))
            .expect("the symbol is there")
            .index()
            .0;
        data[symtab + index * 24 + 8..symtab + index * 24 + 16]
            .copy_from_slice(&0xDEAD_BEEFu64.to_le_bytes());
    }

    let object = parse_and_walk(&data).expect("the object parses");

    // Every one of them is listed. Nothing was dropped because something else failed.
    assert_eq!(object.symbols_sorted.len(), 5);
    let named = |name: &str| {
        object
            .symbols_sorted
            .iter()
            .find(|symbol| symbol.name.starts_with(name))
            .unwrap_or_else(|| panic!("{name} is listed"))
            .clone()
    };

    // The two ordinary functions are exactly as they would be on their own.
    for name in ["first_good", "second_good"] {
        let good = named(name);
        let assembly = good.assembly(&object).expect("it decodes");
        assert_eq!(assembly.instructions.len(), 2, "{name}");
        assert_eq!(assembly.instructions[1].format[0].0, "ret", "{name}");
    }

    // And each of the others fails on its own terms, which is a row without a listing.
    assert!(named("absolute").assembly(&object).is_none());
    assert!(named("cut_short").data_in(&object).is_none());
    // The deep name is not demangled, and that is all it costs: it still decodes.
    let deep = named("?deep@@");
    assert_eq!(deep.demangled, None);
    assert!(deep.assembly(&object).is_some());
}

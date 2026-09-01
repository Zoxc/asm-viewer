//! "Should never panic on any file input": malformed, truncated and mutated inputs must
//! come back as `None` or as a partial object, never as a panic.

mod common;

use analysis::{parse_object, Object};
use common::{
    caller_and_target, elf_shared_object, elf_x86_64, elf_x86_64_with_dwarf, garbage,
    parse_and_walk, pe_dll, survivors, DwarfFixture, DwarfRow, DwarfSection, ExportedSymbol,
    SharedObject, TextRelocation, TextSymbol,
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
/// `SHF_COMPRESSED` (0x800). Setting it sends the parse down the decompression path with
/// a bogus `ch_size`; see `a_lying_compressed_size_in_a_section_header_costs_nothing`.
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

/// A single flipped bit in a section header used to cost seconds of CPU and roughly 8 GB
/// of resident memory. Byte 9 of `.rela.text`'s header turns on `SHF_COMPRESSED`, and the
/// relocation bytes that follow read as an `Elf64_Chdr` announcing a zlib stream that
/// unpacks to 8.6 GB; `uncompressed_data()` reserved all of it before looking at a single
/// compressed byte. Never a panic, so the sweep above stayed green, but an OOM abort on a
/// machine with less RAM, which defeats "should never fall over on any file input".
///
/// `analysis` now weighs the declared size against the compressed bytes before
/// decompressing, so the section is dropped the way any unreadable section is: the parse
/// still succeeds, `.rela.text` simply is not among the sections, and nothing ever
/// allocates more than the file itself holds.
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

/// The guard has to fire on the declared size alone, before any decompression: a section
/// whose zlib stream is perfectly valid but whose header lies about how much it unpacks to
/// is dropped rather than believed. Both bounds are exercised — a size past the 1 GiB cap,
/// and a smaller one that is still past what DEFLATE could possibly produce from these
/// bytes — while the same section with an honest size decompresses as it always did.
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
    // `caller` is three bytes of a five-byte `call rel32`: the next symbol starts inside
    // the instruction, so `estimate_size` cuts it in half.
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
    // `call rel32; ret` cut at every length, so the decoder runs out of bytes at each
    // position inside an instruction.
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

    // An absolute symbol: still `SymbolKind::Text`, but it belongs to no section, so
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

/// The DWARF fixture the line-info tests read, for the sweeps below to corrupt.
fn dwarf_fixture() -> Vec<u8> {
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
            subprograms: &[],
            base_symbol: Some(1),
        }],
    })
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

/// Garbage where DWARF should be must degrade to "no line info", never abort. Every byte
/// of every `.debug_*` section is flipped in turn, which keeps the ELF itself valid and
/// aims the damage squarely at `gimli` — a corrupt unit header, a line program that runs
/// off the end, a file index pointing at nothing, an abbreviation that does not exist.
#[test]
fn corrupted_debug_sections_do_not_panic() {
    let valid = dwarf_fixture();
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
    let valid = dwarf_fixture();
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

        // And every section at once, which is the "a file that is not really DWARF at
        // all but says it is" case.
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

/// The DWARF loader goes through the same `section_data` guard the rest of the crate
/// does, so a `.debug_info` whose compression header lies about its size costs nothing:
/// the section reads as absent and the object simply has no line info. Without the guard
/// this is the compressed-section bug again, only on the path that is *expected* to meet
/// compressed sections — `.debug_*` are the ones compilers actually compress.
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

/// `addr2line` 0.21 computes a line-table row's length as `next.address - row.address`
/// with an unchecked subtraction, and a line program may legally move its address
/// backwards: `DW_LNE_set_address` takes any address at all. This hand-written program
/// sets the address to 0x100, emits a row there and then ends the sequence back at 0,
/// so the only row's "next address" is below it — a subtract-with-overflow panic on a
/// debug build, on a file the app merely opened.
///
/// `analysis` catches it (see `without_panicking` in `src/line.rs`), so the object still
/// parses and simply has no line info. Note that the panic message the run prints comes
/// from that caught panic and is expected.
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

/// The committed, compiler-produced objects (`tests/fixtures/`, read back by
/// `tests/real_object.rs`), for the sweeps below to corrupt as well.
///
/// The written fixture above is DWARF 4 with its strings inline; these are DWARF 5, with a
/// `.debug_line_str`, a version 5 line-program header and — in the `-ffunction-sections`
/// build — a `.debug_rnglists`. Mutating them therefore aims at `gimli` parsing paths the
/// synthesized fixture never reaches at all.
fn committed_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    ["line_fixture.o", "line_fixture_split.o"]
        .into_iter()
        .map(|name| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join(name);
            let data = std::fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "{}: {error}\n\
                     This fixture is committed to the repository, not generated. Restore it \
                     from git, or rebuild it with the command in \
                     tests/fixtures/line_fixture.c.",
                    path.display()
                )
            });
            (name, data)
        })
        .collect()
}

/// Both sweeps again — every byte flipped, then whole sections replaced with noise — over
/// real DWARF 5 rather than the written DWARF 4 above.
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

/// The images `declared_code` reads: a stripped ELF `.so` whose only symbol table is
/// `.dynsym`, and a PE DLL whose only declaration of anything is its export directory.
///
/// The relocatable fixtures above never reach that code at all — an `.o` declares no
/// exports and no entry point, so the pass returns before it looks at anything — which
/// means everything the export and entry paths do with a file's own numbers (an address
/// looked up in a section's range, a name read out of a string table, an ordinal
/// indexing an address array) is unexercised by every other sweep in this file.
fn images() -> Vec<(&'static str, Vec<u8>)> {
    const TEXT: &[u8] = &[0x90, 0x90, 0x90, 0xC3, 0x90, 0xC3];
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
            }),
        ),
        ("pe dll", pe_dll(TEXT, SYMBOLS, Some(4))),
    ]
}

#[test]
fn truncated_images_do_not_panic() {
    for (kind, valid) in images() {
        // Sanity check that the fixture is a fixture: an image the parse can read, with
        // the declared code in it.
        let object = parse_and_walk(&valid).expect("the image parses");
        assert_eq!(object.symbols_sorted.len(), 2, "{kind}");

        let failures = survivors(
            (0..valid.len()).map(|len| (format!("{kind} truncated to {len}"), &valid[..len])),
        );
        assert!(failures.is_empty(), "panicked on: {failures:?}");
    }
}

#[test]
fn corrupted_images_do_not_panic() {
    for (kind, valid) in images() {
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

/// A mangled name says how deep the demangler that reads it will recurse, and two of the
/// demanglers behind `symbolic-demangle` recurse once per byte: `msvc-demangler` 0.11 has
/// no recursion limit at all, and `cpp_demangle`'s (raised to 160/192 by `symbolic`) is
/// deep enough that reaching it is megabytes of stack in a debug build. A symbol name is
/// bytes out of a string table, so this is a **stack overflow** — an abort, which no
/// `catch_unwind` can catch — on a file the user merely opened. Before the fix this test
/// did not fail, it killed the test binary: `fatal runtime error: stack overflow`.
///
/// `analysis` heads it off before the call: names past `MAX_MANGLED_NAME` are not
/// demangled at all, and the rest are demangled on a stack sized for the deepest of them
/// (`demangled` in `src/lib.rs`). Every symbol is still listed, under the name the file
/// gave it, which is what every unrecognised name already does.
#[test]
fn a_deeply_nested_name_does_not_overflow_the_stack() {
    // 1000 levels overflows the 8 MiB this test's own thread has, let alone the 2 MiB the
    // viewer's parse thread gets; 4000 is also past the cap, so it is not demangled at
    // all. `hot` is the control: an ordinary name, in the same batch, still demangles.
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
    for symbol in &object.symbols_sorted {
        // Named, listed, and readable either way: a name no demangler could take is
        // displayed exactly as the file wrote it.
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
    // And the ordinary name in the same batch is untouched by any of it.
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

/// A section header states where its bytes live, and nothing stops it from saying they
/// live at the end of the address space. A file that does — and whose debug info then
/// claims a function running off the end of it — must still parse, still list its
/// symbols, and answer "I do not know" everywhere it does not know, rather than falling
/// over anywhere along the way.
///
/// Four independent bounds are what makes that happen here, and this fixture is the one
/// input in the suite that walks all of them at once:
///
/// * `estimate_size` derives the last symbol's extent as `section.address +
///   section.data.len()`, which does not fit, so it answers [`None`] instead of wrapping.
/// * `Dwarf::extent` adds the section bias with a checked add, for the same reason.
/// * `addr2line` 0.21 adds `low_pc + high_pc` unchecked while parsing a unit's functions,
///   so the over-reaching `DW_TAG_subprogram` is a panic inside the dependency — caught,
///   and the answer is "no debug-info extent" (see `without_panicking` in `src/line.rs`).
/// * With no extent at all there are no bytes to decode, so the symbol lists with no
///   disassembly rather than with a listing decoded from an address that wrapped.
///
/// `SymbolData::assembly`'s own arithmetic is checked as well, so a future extent that
/// *does* reach past the end (`.pdata`, in `notes/Goals.md`) stops the listing at the
/// wrap instead of indexing past the symbol's bytes.
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
/// function there.
///
/// Written with the two writers and then patched, rather than assembled by hand: the
/// `object` writer only emits relocatable objects, and only three numbers have to change
/// to turn one into an image at an address of its own — `e_type`, the section's `sh_addr`
/// and the symbol's `st_value`. The DWARF is written with real addresses (never a
/// relocation), which is exactly what a linked image carries.
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

    // ET_REL -> ET_EXEC: an image, so `section_biases` leaves its addresses alone and
    // `declared_code` reads its (absent) exports and its zero entry point.
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

/// The other half of the goal (`notes/Goals.md`, *Binary inspection design*): "errors
/// doing analysis should allow inspecting functions without errors". A file the viewer
/// can only partly make sense of must still show everything in it that it *can* make
/// sense of — one symbol that cannot be sized, decoded or demangled is one symbol
/// without a listing, not an object without a listing.
///
/// The fixture puts four different kinds of failure in one object, in the middle of two
/// perfectly ordinary functions:
///
/// * a symbol in no section at all (an absolute one), which has no bytes to size or
///   decode;
/// * a name deep enough to be past the demangler cap (`MAX_MANGLED_NAME`), which is the
///   whole batch's business, since demangling is done for the object at once;
/// * a symbol whose bytes stop in the middle of an instruction, because the next symbol
///   starts there;
/// * a symbol whose address is nowhere in its own section, which is the one that used to
///   spread: `estimate_size` derived the *previous* symbol's extent from it, so a single
///   wild `st_value` left the perfectly readable function above it with no listing. The
///   derivation is clipped to the section's own bytes now.
///
/// What this establishes is structural: sizing, decoding and demangling are each per
/// symbol, so a failure in one is scoped to that symbol's row. The two that fail
/// *together* are the DWARF context (one per object, and its absence is one answer for
/// all of them) and, inside it, a unit's function extents — `addr2line` parses a whole
/// unit's subprograms at once — which is why an extent is only ever an improvement on
/// the next-symbol estimate and never the only answer.
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

    // And one symbol pointed clean out of its own section, which the writer will not
    // emit: `st_value` is patched afterwards, the way an unreasonable file would have it.
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

    // The two ordinary functions are exactly as they would be on their own: two
    // instructions each, the second of them a `ret`.
    for name in ["first_good", "second_good"] {
        let good = named(name);
        let assembly = good.assembly(&object).expect("it decodes");
        assert_eq!(assembly.instructions.len(), 2, "{name}");
        assert_eq!(assembly.instructions[1].format[0].0, "ret", "{name}");
    }

    // And each of the others fails on its own terms, which is a row without a listing.
    assert!(named("absolute").assembly(&object).is_none());
    assert!(named("cut_short").data_in(&object).is_none());
    // The deep name is not demangled, and that is all it costs: the symbol is listed
    // under the name the file gave it and still decodes.
    let deep = named("?deep@@");
    assert_eq!(deep.demangled, None);
    assert!(deep.assembly(&object).is_some());
}

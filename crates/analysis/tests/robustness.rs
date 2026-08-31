//! "Should never panic on any file input": malformed, truncated and mutated inputs must
//! come back as `None` or as a partial object, never as a panic.

mod common;

use analysis::{parse_object, Object};
use common::{caller_and_target, elf_x86_64, garbage, TextRelocation, TextSymbol};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Arc;

/// Parse, then walk everything a parsed object exposes, so a panic in size estimation or
/// disassembly is caught too and not just one in `parse_object`.
fn parse_and_walk(data: &[u8]) -> Option<Arc<Object>> {
    let object = parse_object(data.into(), "fuzz".into(), PathBuf::from("/fuzz"))?;

    for symbol in &object.symbols_sorted {
        let _ = symbol.estimate_size();
        let _ = symbol.data();
        if let Some(assembly) = symbol.assembly(&object) {
            for instruction in &assembly.instructions {
                let _: String = instruction.format.iter().map(|(t, _)| t.as_str()).collect();
            }
        }
    }

    Some(object)
}

/// Run `parse_and_walk` on every input, returning the labels of the ones that panicked.
fn survivors<'a>(inputs: impl IntoIterator<Item = (String, &'a [u8])>) -> Vec<String> {
    inputs
        .into_iter()
        .filter_map(|(label, data)| {
            catch_unwind(AssertUnwindSafe(|| parse_and_walk(data)))
                .err()
                .map(|_| label)
        })
        .collect()
}

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

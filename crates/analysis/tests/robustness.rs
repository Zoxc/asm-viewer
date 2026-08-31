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
    let object = parse_object(data, "fuzz".into(), PathBuf::from("/fuzz"))?;

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
/// a bogus `ch_size`; see `compressed_flag_makes_the_parse_allocate_gigabytes` below.
fn section_flag_bytes(elf: &[u8]) -> Vec<usize> {
    let shoff = u64::from_le_bytes(elf[0x28..0x30].try_into().unwrap()) as usize;
    let shentsize = u16::from_le_bytes(elf[0x3A..0x3C].try_into().unwrap()) as usize;
    let shnum = u16::from_le_bytes(elf[0x3C..0x3E].try_into().unwrap()) as usize;
    (0..shnum).map(|i| shoff + i * shentsize + 9).collect()
}

#[test]
fn corrupted_objects_do_not_panic() {
    let valid = caller_and_target();
    let skip = section_flag_bytes(&valid);

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for offset in 0..valid.len() {
        // Excluded only because it is slow and enormous, not because it is safe --
        // see `compressed_flag_makes_the_parse_allocate_gigabytes`.
        if skip.contains(&offset) {
            continue;
        }
        for mask in [0xFFu8, 0x01, 0x80] {
            let mut data = valid.clone();
            data[offset] ^= mask;
            cases.push((format!("byte {offset} ^ {mask:#04x}"), data));
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// A single flipped bit in a section header costs seconds of CPU and roughly 8 GB of
/// resident memory: `SHF_COMPRESSED` makes `object`'s `uncompressed_data()` read a
/// `ch_size` out of whatever bytes follow and reserve that much up front, before any of
/// it is validated against the size of the file.
///
/// Not a panic, so the no-panic sweep above stays green, but it does defeat "should never
/// fall over on any file input" on a machine with less RAM than that. Ignored by default
/// precisely because running it allocates those gigabytes; run with
/// `cargo test -p analysis -- --ignored --exact compressed_flag_makes_the_parse_allocate_gigabytes`.
#[test]
#[ignore = "allocates several GB and takes seconds; documents an open robustness issue"]
fn compressed_flag_makes_the_parse_allocate_gigabytes() {
    let mut data = caller_and_target();
    let offset = section_flag_bytes(&data)[2];
    data[offset] ^= 0xFF;

    assert!(parse_and_walk(&data).is_some());
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

    let object = parse_object(&data, "trunc.o".into(), PathBuf::from("/trunc.o")).expect("parses");
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

    let object = parse_object(&data, "abs.o".into(), PathBuf::from("/abs.o")).expect("parses");
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

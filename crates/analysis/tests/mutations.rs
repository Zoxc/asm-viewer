//! The search behind `robustness.rs`: every fixture the suite builds, mutated three ways
//! — truncated, poisoned field by field, and splatted with random bytes — with the whole
//! pipeline run over each result (`common::parse_and_walk`). A mutated file may parse into
//! anything at all; the only failure is a panic.
//!
//! **Everything here is deterministic and bounded**, so a failure is reproducible from its
//! label alone and the suite stays in single-digit seconds: the pseudo-random bytes are
//! `common::garbage` over a fixed seed, never `rand` and never the clock, and where the
//! full product would be large the sweep is sampled by an even stride from the front
//! rather than by picking — at most [`MAX_FIELDS`] numeric fields per file, and every
//! [`TRUNCATION_STRIDE`]-th length past [`WHOLE_TRUNCATION`] bytes. Which cases run is
//! therefore fixed, and a sampled table is still represented end to end.

mod common;

use common::{
    caller_and_target, committed_fixture, declared_code_images, dwarf_fixture, garbage, survivors,
};

/// At most this many numeric fields are poisoned per file. The committed objects have 371
/// and 471 of them, and a stride over the whole table finds the same classes of defect.
const MAX_FIELDS: usize = 128;

/// Every length up to here is truncated to; past it, every [`TRUNCATION_STRIDE`]-th.
/// Everything a header parser reads is in the first few hundred bytes.
const WHOLE_TRUNCATION: usize = 1024;

/// See [`WHOLE_TRUNCATION`]. An odd stride on purpose: a power of two would land on the
/// same alignment inside every structure it walked past.
const TRUNCATION_STRIDE: usize = 7;

/// Every shape the crate can be asked about: relocatable objects with and without DWARF,
/// real compiler output in DWARF 5, the two linked images, whose export and entry-point
/// paths (`declared_code`) no `.o` reaches at all, and a linker's real DLL with a debug
/// directory naming a `.pdb`.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut corpus = vec![
        ("caller_and_target".to_owned(), caller_and_target()),
        ("dwarf".to_owned(), dwarf_fixture(&[(0, 6), (1, 2)])),
        (
            "line_fixture.o".to_owned(),
            committed_fixture("line_fixture.o"),
        ),
        (
            "line_fixture_split.o".to_owned(),
            committed_fixture("line_fixture_split.o"),
        ),
        (
            "line_fixture.dll".to_owned(),
            committed_fixture("line_fixture.dll"),
        ),
    ];
    corpus.extend(
        declared_code_images()
            .into_iter()
            .map(|(label, data)| (label.to_owned(), data)),
    );
    corpus
}

/// A file that stops part-way through is the commonest malformed file there is. Every
/// prefix has to come back as an object or as nothing.
#[test]
fn truncation_at_every_length_does_not_panic() {
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, valid) in corpus() {
        let lengths =
            (0..valid.len()).filter(|len| *len <= WHOLE_TRUNCATION || len % TRUNCATION_STRIDE == 0);
        for len in lengths {
            cases.push((format!("{name} truncated to {len}"), valid[..len].to_vec()));
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The sweep that finds the arithmetic: a flipped bit rarely turns a count into something
/// interesting, where writing `u64::MAX` into it always does. Every field a parser reads as
/// a count, an offset or a size takes each of [`poisons`] in turn.
///
/// This is the sweep that reaches `addr2line` 0.21's two unchecked additions, caught by
/// `without_panicking` in `src/line.rs` — this test is green *because* they are.
#[test]
fn field_targeted_corruption_does_not_panic() {
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, valid) in corpus() {
        let mut fields = elf_fields(&valid);
        fields.extend(pe_fields(&valid));
        // Sampled by an even stride, never by picking; see the module docs.
        let stride = 1 + fields.len() / MAX_FIELDS;
        for (offset, width) in fields.into_iter().step_by(stride) {
            for value in poisons(valid.len()) {
                let mut data = valid.clone();
                let bytes = value.to_le_bytes();
                data[offset..offset + width].copy_from_slice(&bytes[..width]);
                cases.push((format!("{name}: [{offset}+{width}] = {value:#x}"), data));
            }
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The sweep that knows nothing about the formats: runs of pseudo-random bytes over a
/// valid file at pseudo-random places, for what a field-targeted sweep never names.
#[test]
fn random_splats_do_not_panic() {
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, valid) in corpus() {
        for seed in 1..200u64 {
            // Everything about a case comes out of its seed, so its label reproduces it.
            let mut state = seed | 1;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };

            let mut data = valid.clone();
            for _ in 0..=(next() % 4) {
                let at = next() as usize % data.len();
                let end = (at + 1 + next() as usize % 16).min(data.len());
                data[at..end].copy_from_slice(&garbage(next(), end - at));
            }
            cases.push((format!("{name} splat seed {seed}"), data));
        }
    }

    let failures = survivors(cases.iter().map(|(label, data)| (label.clone(), &data[..])));
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The values written into a field, chosen for the boundaries a length check is written
/// against: nothing, everything, each width's own limit, and the file's own length on
/// either side of it.
fn poisons(len: usize) -> [u64; 8] {
    [
        0,
        1,
        u16::MAX as u64,
        u32::MAX as u64,
        u64::MAX,
        1 << 63,
        len as u64,
        len as u64 + 1,
    ]
}

/// Every `(offset, width)` in an ELF64 that a parser reads as a count, an offset or a
/// size: the file header, every section header, and every entry of every symbol or
/// relocation table those headers point at.
fn elf_fields(data: &[u8]) -> Vec<(usize, usize)> {
    let mut fields = Vec::new();
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 {
        return fields;
    }

    // e_type, e_machine, e_entry, e_phoff, e_shoff, e_ehsize, e_phentsize, e_phnum,
    // e_shentsize, e_shnum, e_shstrndx.
    for (offset, width) in [
        (16usize, 2usize),
        (18, 2),
        (24, 8),
        (32, 8),
        (40, 8),
        (52, 2),
        (54, 2),
        (56, 2),
        (58, 2),
        (60, 2),
        (62, 2),
    ] {
        fields.push((offset, width));
    }

    let shoff = u64::from_le_bytes(data[0x28..0x30].try_into().unwrap()) as usize;
    let shentsize = u16::from_le_bytes(data[0x3A..0x3C].try_into().unwrap()) as usize;
    let shnum = u16::from_le_bytes(data[0x3C..0x3E].try_into().unwrap()) as usize;
    if shentsize != 64 {
        return fields;
    }

    // The tables the section headers point at. Every entry of each is 24 bytes, whether it
    // is a symbol or a RELA.
    let mut tables: Vec<(usize, usize)> = Vec::new();
    for i in 0..shnum {
        let base = shoff + i * shentsize;
        if base + shentsize > data.len() {
            break;
        }
        // sh_name, sh_type, sh_flags, sh_addr, sh_offset, sh_size, sh_link, sh_info,
        // sh_addralign, sh_entsize.
        for (offset, width) in [
            (0usize, 4usize),
            (4, 4),
            (8, 8),
            (16, 8),
            (24, 8),
            (32, 8),
            (40, 4),
            (44, 4),
            (48, 8),
            (56, 8),
        ] {
            fields.push((base + offset, width));
        }

        // SHT_SYMTAB, SHT_RELA and SHT_DYNSYM: the three with 24-byte entries.
        let kind = u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap());
        if matches!(kind, 2 | 4 | 11) {
            let offset =
                u64::from_le_bytes(data[base + 24..base + 32].try_into().unwrap()) as usize;
            let size = u64::from_le_bytes(data[base + 32..base + 40].try_into().unwrap()) as usize;
            tables.push((offset, size));
        }
    }

    for (offset, size) in tables {
        if !offset
            .checked_add(size)
            .is_some_and(|end| end <= data.len())
        {
            continue;
        }
        for entry in 0..size / 24 {
            let base = offset + entry * 24;
            // st_name, st_info + st_other + st_shndx, st_value, st_size — and the same
            // four are a RELA's r_offset, r_info and r_addend.
            for (offset, width) in [(0usize, 4usize), (4, 4), (8, 8), (16, 8)] {
                fields.push((base + offset, width));
            }
        }
    }

    fields
}

/// The same for a PE image: the COFF header, the PE32+ optional header, the export and
/// debug data directories, the section table, the export directory it points at — what
/// `declared_code` walks in an image with no symbol table — and the debug directory's
/// entry and the CodeView record behind it, which is how a `.pdb` is named and matched.
fn pe_fields(data: &[u8]) -> Vec<(usize, usize)> {
    let mut fields = Vec::new();
    if data.len() < 0x40 || &data[..2] != b"MZ" {
        return fields;
    }
    let pe = u32::from_le_bytes(data[0x3c..0x40].try_into().unwrap()) as usize;
    if pe + 24 > data.len() || &data[pe..pe + 4] != b"PE\0\0" {
        return fields;
    }

    // Machine, NumberOfSections, PointerToSymbolTable, NumberOfSymbols,
    // SizeOfOptionalHeader, Characteristics.
    let coff = pe + 4;
    for (offset, width) in [(0usize, 2usize), (2, 2), (8, 4), (12, 4), (16, 2), (18, 2)] {
        fields.push((coff + offset, width));
    }

    // Magic, AddressOfEntryPoint, BaseOfCode, ImageBase, SectionAlignment, FileAlignment,
    // SizeOfImage, SizeOfHeaders, NumberOfRvaAndSizes, the export data directory and the
    // debug data directory.
    let opt = coff + 20;
    for (offset, width) in [
        (0usize, 2usize),
        (16, 4),
        (20, 4),
        (24, 8),
        (32, 4),
        (36, 4),
        (56, 4),
        (60, 4),
        (108, 4),
        (112, 4),
        (116, 4),
        (160, 4),
        (164, 4),
    ] {
        fields.push((opt + offset, width));
    }
    if opt + 168 > data.len() {
        return fields;
    }

    let sections = u16::from_le_bytes(data[coff + 2..coff + 4].try_into().unwrap()) as usize;
    let optional = u16::from_le_bytes(data[coff + 16..coff + 18].try_into().unwrap()) as usize;
    let table = opt + optional;
    let export_rva = u32::from_le_bytes(data[opt + 112..opt + 116].try_into().unwrap());
    let debug_rva = u32::from_le_bytes(data[opt + 160..opt + 164].try_into().unwrap());
    let mut export = None;
    let mut debug = None;

    for i in 0..sections {
        let base = table + i * 40;
        if base + 40 > data.len() {
            break;
        }
        // VirtualSize, VirtualAddress, SizeOfRawData, PointerToRawData, Characteristics.
        for (offset, width) in [(8usize, 4usize), (12, 4), (16, 4), (20, 4), (36, 4)] {
            fields.push((base + offset, width));
        }

        // Where in the file the export directory and the debug directory are, if they are in
        // this section.
        let size = u32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap());
        let rva = u32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap());
        let pointer = u32::from_le_bytes(data[base + 20..base + 24].try_into().unwrap()) as usize;
        if export_rva >= rva && export_rva < rva.saturating_add(size) {
            export = Some(pointer + (export_rva - rva) as usize);
        }
        if debug_rva != 0 && debug_rva >= rva && debug_rva < rva.saturating_add(size) {
            debug = Some(pointer + (debug_rva - rva) as usize);
        }
    }

    // Name, Base, NumberOfFunctions, NumberOfNames, AddressOfFunctions, AddressOfNames,
    // AddressOfNameOrdinals.
    if let Some(base) = export.filter(|base| base + 40 <= data.len()) {
        for offset in [12usize, 16, 20, 24, 28, 32, 36] {
            fields.push((base + offset, 4));
        }
    }

    // The debug directory's one entry — Type, SizeOfData, AddressOfRawData, PointerToRawData —
    // and, through the last of those, the CodeView record: its `RSDS` signature, the four
    // words of the GUID, and the age.
    if let Some(base) = debug.filter(|base| base + 28 <= data.len()) {
        for offset in [12usize, 16, 20, 24] {
            fields.push((base + offset, 4));
        }
        let pointer = u32::from_le_bytes(data[base + 24..base + 28].try_into().unwrap()) as usize;
        if pointer + 24 <= data.len() {
            for offset in [0usize, 4, 8, 12, 16, 20] {
                fields.push((pointer + offset, 4));
            }
        }
    }

    fields
}

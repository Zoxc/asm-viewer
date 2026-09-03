//! The search behind `robustness.rs`: every fixture the suite builds, mutated three ways
//! — truncated, poisoned field by field, and splatted with random bytes — with the whole
//! pipeline run over each result (`common::parse_and_walk_at`). A mutated file may parse
//! into anything at all; the only failure is a panic.
//!
//! **Everything here is deterministic and bounded**, so a failure is reproducible from its
//! label alone and the suite stays in single-digit seconds: the pseudo-random bytes are
//! `common::garbage` over a fixed seed, never `rand` and never the clock, and where the
//! full product would be large the sweep is sampled by an even stride from the front
//! rather than by picking — at most [`MAX_FIELDS`] numeric fields per file, and every
//! [`TRUNCATION_STRIDE`]-th length past [`WHOLE_TRUNCATION`] bytes. Which cases run is
//! therefore fixed, and a sampled table is still represented end to end.
//!
//! **Six of the inputs are files on disk**, because a `.pdb` is a second file found beside
//! its binary: each of the linker's three DLLs is parsed at a path with its pristine PDB
//! beside it, so a mutation of the DLL that leaves the CodeView record intact goes on to
//! open and match the PDB — at parse time now, for the procedures and publics it names; and
//! each PDB is mutated in turn, every mutation written beside its pristine DLL before the
//! DLL is parsed. The second and third pairs, the images that name nothing, are the ones
//! whose every name is the PDB's, so a mutated PDB there is what the procedure walk is
//! swept with — and the third's has a function only its publics name, so the publics walk
//! too. Each test writes under a directory of its own in the target directory, since the
//! three run at once.

mod common;

use common::{
    caller_and_target, committed_fixture, declared_code_images, dwarf_fixture, garbage,
    parse_and_walk_at,
};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

/// At most this many numeric fields are poisoned per file. The committed objects have 371
/// and 471 of them, and a stride over the whole table finds the same classes of defect.
const MAX_FIELDS: usize = 128;

/// Every length up to here is truncated to; past it, every [`TRUNCATION_STRIDE`]-th.
/// Everything a header parser reads is in the first few hundred bytes.
const WHOLE_TRUNCATION: usize = 1024;

/// See [`WHOLE_TRUNCATION`]. An odd stride on purpose: a power of two would land on the
/// same alignment inside every structure it walked past.
const TRUNCATION_STRIDE: usize = 7;

/// The PDB's own truncation stride: it is 72 KB and every case is a file written, so every
/// length of its 56-byte superblock and then every this-many-th. Odd, for the reason above,
/// and prime to 4096 so the cuts drift across the page boundaries an MSF is laid out on.
const PDB_TRUNCATION_STRIDE: usize = 509;

/// The three committed DLL + PDB pairs (`tests/pdb.rs`): the one whose three functions are
/// exported, the one that exports nothing and is named only by its PDB, and the one with a
/// fourth function only that PDB's publics name.
const PAIRS: [(&str, &str); 3] = [
    ("line_fixture.dll", "line_fixture.pdb"),
    ("line_fixture_noexport.dll", "line_fixture_noexport.pdb"),
    ("line_fixture_public.dll", "line_fixture_public.pdb"),
];

/// One input to the pipeline: the bytes, the path they are said to be at, and a file to put
/// beside them first — the mutated PDB a pristine DLL is parsed next to.
struct Case {
    label: String,
    data: Vec<u8>,
    path: PathBuf,
    beside: Option<(PathBuf, Vec<u8>)>,
}

impl Case {
    /// Bytes at a path nothing sits beside, which is every fixture built in memory.
    fn in_memory(label: String, data: Vec<u8>) -> Case {
        Case {
            label,
            data,
            path: PathBuf::from("/fuzz"),
            beside: None,
        }
    }

    /// The same bytes-and-place, mutated: what every sweep below builds from a corpus entry.
    fn mutated(&self, label: String, data: Vec<u8>) -> Case {
        Case {
            label,
            data,
            path: self.path.clone(),
            beside: None,
        }
    }
}

/// A directory of this test's own under the target directory. Not emptied: every file a
/// case needs is written before the case runs, over whatever an earlier run left.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("mutations")
        .join(test);
    fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Every shape the crate can be asked about: relocatable objects with and without DWARF,
/// real compiler output in DWARF 5, the two linked images, whose export and entry-point
/// paths (`declared_code`) no `.o` reaches at all, one of them naming a `.pdb` that is
/// nowhere, and the linker's three real DLLs each **beside its PDB**.
fn corpus(test: &str) -> Vec<Case> {
    let mut corpus = vec![
        Case::in_memory("caller_and_target".to_owned(), caller_and_target()),
        Case::in_memory("dwarf".to_owned(), dwarf_fixture(&[(0, 6), (1, 2)])),
        Case::in_memory(
            "line_fixture.o".to_owned(),
            committed_fixture("line_fixture.o"),
        ),
        Case::in_memory(
            "line_fixture_split.o".to_owned(),
            committed_fixture("line_fixture_split.o"),
        ),
    ];
    corpus.extend(
        declared_code_images()
            .into_iter()
            .map(|(label, data, _)| Case::in_memory(label.to_owned(), data)),
    );

    let dir = scratch(test).join("dll");
    fs::create_dir_all(&dir).unwrap();
    for (dll, pdb) in PAIRS {
        fs::write(dir.join(pdb), committed_fixture(pdb)).unwrap();
        let data = committed_fixture(dll);
        // The pair is found where the sweep put it, so the mutations reach the PDB backend
        // rather than a search that comes back empty — and for the images that name
        // nothing, a symbol with lines is the PDB having been read at parse; their `.pdata`
        // gives them symbols on their own, but no lines. Any symbol with lines, not the
        // first: the third pair's first by name is the public, which has none.
        let intact = parse_and_walk_at(&data, dir.join(dll)).expect("the DLL parses");
        assert!(
            intact
                .symbols_sorted
                .iter()
                .any(|symbol| symbol.line_info(&intact).is_some()),
            "the PDB beside {dll} was not read"
        );
        corpus.push(Case {
            label: dll.to_owned(),
            data,
            path: dir.join(dll),
            beside: None,
        });
    }
    corpus
}

/// Each pristine DLL parsed beside every mutation `mutate` makes of its PDB.
fn pdb_cases(test: &str, mutate: impl Fn(&[u8]) -> Vec<(String, Vec<u8>)>) -> Vec<Case> {
    let dir = scratch(test).join("pdb");
    fs::create_dir_all(&dir).unwrap();
    let mut cases = Vec::new();
    for (dll, pdb) in PAIRS {
        let data = committed_fixture(dll);
        let mutations = mutate(&committed_fixture(pdb));
        cases.extend(mutations.into_iter().map(|(label, bytes)| Case {
            label: format!("{pdb} {label}"),
            data: data.clone(),
            path: dir.join(dll),
            beside: Some((dir.join(pdb), bytes)),
        }));
    }
    cases
}

/// Run the pipeline over every case, returning the labels of the ones that panicked.
fn failures(cases: Vec<Case>) -> Vec<String> {
    cases
        .into_iter()
        .filter_map(|case| {
            if let Some((path, bytes)) = &case.beside {
                fs::write(path, bytes).expect("writing the file beside the binary");
            }
            catch_unwind(AssertUnwindSafe(|| {
                parse_and_walk_at(&case.data, case.path.clone())
            }))
            .err()
            .map(|_| case.label)
        })
        .collect()
}

/// A file that stops part-way through is the commonest malformed file there is. Every
/// prefix has to come back as an object or as nothing.
#[test]
fn truncation_at_every_length_does_not_panic() {
    const TEST: &str = "truncation";
    let mut cases = Vec::new();
    for valid in corpus(TEST) {
        let lengths = (0..valid.data.len())
            .filter(|len| *len <= WHOLE_TRUNCATION || len % TRUNCATION_STRIDE == 0);
        for len in lengths {
            cases.push(valid.mutated(
                format!("{} truncated to {len}", valid.label),
                valid.data[..len].to_vec(),
            ));
        }
    }

    cases.extend(pdb_cases(TEST, |pdb| {
        (0..pdb.len())
            .filter(|len| *len < 56 || len % PDB_TRUNCATION_STRIDE == 0)
            .map(|len| (format!("truncated to {len}"), pdb[..len].to_vec()))
            .collect()
    }));

    let failures = failures(cases);
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The sweep that finds the arithmetic: a flipped bit rarely turns a count into something
/// interesting, where writing `u64::MAX` into it always does. Every field a parser reads as
/// a count, an offset or a size takes each of [`poisons`] in turn.
///
/// This is the sweep that reaches `addr2line` 0.21's two unchecked additions and `pdb2`
/// 0.10's, caught by `without_panicking` in `src/line.rs` — this test is green *because*
/// they are.
#[test]
fn field_targeted_corruption_does_not_panic() {
    const TEST: &str = "fields";
    let poisoned = |valid: &[u8], fields: Vec<(usize, usize)>| -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        // Sampled by an even stride, never by picking; see the module docs.
        let stride = 1 + fields.len() / MAX_FIELDS;
        for (offset, width) in fields.into_iter().step_by(stride) {
            for value in poisons(valid.len()) {
                let mut data = valid.to_vec();
                let bytes = value.to_le_bytes();
                data[offset..offset + width].copy_from_slice(&bytes[..width]);
                out.push((format!("[{offset}+{width}] = {value:#x}"), data));
            }
        }
        out
    };

    let mut cases = Vec::new();
    for valid in corpus(TEST) {
        let mut fields = elf_fields(&valid.data);
        fields.extend(pe_fields(&valid.data));
        for (label, data) in poisoned(&valid.data, fields) {
            cases.push(valid.mutated(format!("{}: {label}", valid.label), data));
        }
    }

    cases.extend(pdb_cases(TEST, |pdb| {
        let fields = pdb_fields(pdb);
        assert!(fields.len() > 8, "the PDB's fields were found");
        poisoned(pdb, fields)
    }));

    let failures = failures(cases);
    assert!(failures.is_empty(), "panicked on: {failures:?}");
}

/// The sweep that knows nothing about the formats: runs of pseudo-random bytes over a
/// valid file at pseudo-random places, for what a field-targeted sweep never names.
#[test]
fn random_splats_do_not_panic() {
    const TEST: &str = "splats";
    let splatted = |valid: &[u8]| -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        for seed in 1..200u64 {
            // Everything about a case comes out of its seed, so its label reproduces it.
            let mut state = seed | 1;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };

            let mut data = valid.to_vec();
            for _ in 0..=(next() % 4) {
                let at = next() as usize % data.len();
                let end = (at + 1 + next() as usize % 16).min(data.len());
                data[at..end].copy_from_slice(&garbage(next(), end - at));
            }
            out.push((format!("splat seed {seed}"), data));
        }
        out
    };

    let mut cases = Vec::new();
    for valid in corpus(TEST) {
        for (label, data) in splatted(&valid.data) {
            cases.push(valid.mutated(format!("{} {label}", valid.label), data));
        }
    }
    cases.extend(pdb_cases(TEST, splatted));

    let failures = failures(cases);
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

/// Every `(offset, width)` in an MSF 7.0 file — a `.pdb` — that its reader takes as a count,
/// an offset or a size: the superblock (page size, free page map, pages used, directory
/// size, the page holding the directory's page list), that page list, and the stream
/// directory's own numbers — how many streams, each one's size, and each one's first page.
fn pdb_fields(data: &[u8]) -> Vec<(usize, usize)> {
    const MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";
    let mut fields = Vec::new();
    if data.len() < 56 || !data.starts_with(MAGIC) {
        return fields;
    }
    let u32_at = |offset: usize| -> Option<usize> {
        data.get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
    };
    for offset in [32usize, 36, 40, 44, 52] {
        fields.push((offset, 4));
    }

    let (Some(page_size), Some(directory_size), Some(map_page)) =
        (u32_at(32), u32_at(44), u32_at(52))
    else {
        return fields;
    };
    if page_size == 0 {
        return fields;
    }

    // The directory's page list, one page number per page of the directory.
    let map = map_page * page_size;
    let directory_pages = directory_size.div_ceil(page_size);
    for i in 0..directory_pages {
        if map + 4 * i + 4 <= data.len() {
            fields.push((map + 4 * i, 4));
        }
    }

    // The directory itself: the stream count, the sizes, then every stream's page numbers,
    // of which the first is taken.
    let Some(directory) = u32_at(map).map(|page| page * page_size) else {
        return fields;
    };
    let Some(streams) = u32_at(directory) else {
        return fields;
    };
    fields.push((directory, 4));
    let mut pages_at = directory + 4 + 4 * streams;
    for i in 0..streams {
        let Some(size) = u32_at(directory + 4 + 4 * i) else {
            break;
        };
        fields.push((directory + 4 + 4 * i, 4));
        // A size of `u32::MAX` is a stream that does not exist and has no pages.
        let pages = if size == u32::MAX as usize {
            0
        } else {
            size.div_ceil(page_size)
        };
        if pages > 0 && pages_at + 4 <= data.len() {
            fields.push((pages_at, 4));
        }
        pages_at += 4 * pages;
    }

    fields
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

/// The same for a PE image: the COFF header, the PE32+ optional header, the export,
/// exception and debug data directories, the section table, the export directory it points
/// at — what `declared_code` walks in an image with no symbol table — every
/// `RUNTIME_FUNCTION`'s three words in the exception directory, and the debug directory's
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
    // SizeOfImage, SizeOfHeaders, NumberOfRvaAndSizes, the export data directory, the
    // exception data directory and the debug data directory.
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
        (136, 4),
        (140, 4),
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
    let exception_rva = u32::from_le_bytes(data[opt + 136..opt + 140].try_into().unwrap());
    let exception_size = u32::from_le_bytes(data[opt + 140..opt + 144].try_into().unwrap());
    let debug_rva = u32::from_le_bytes(data[opt + 160..opt + 164].try_into().unwrap());
    let mut export = None;
    let mut exception = None;
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

        // Where in the file the export, exception and debug directories are, if they are in
        // this section.
        let size = u32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap());
        let rva = u32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap());
        let pointer = u32::from_le_bytes(data[base + 20..base + 24].try_into().unwrap()) as usize;
        if export_rva >= rva && export_rva < rva.saturating_add(size) {
            export = Some(pointer + (export_rva - rva) as usize);
        }
        if exception_rva != 0 && exception_rva >= rva && exception_rva < rva.saturating_add(size) {
            exception = Some(pointer + (exception_rva - rva) as usize);
        }
        if debug_rva != 0 && debug_rva >= rva && debug_rva < rva.saturating_add(size) {
            debug = Some(pointer + (debug_rva - rva) as usize);
        }
    }

    // Every `RUNTIME_FUNCTION` in the exception directory: BeginAddress, EndAddress and
    // UnwindInfoAddress, 12 bytes each, as many as the directory's size says fit.
    if let Some(base) = exception {
        let entries = (exception_size / 12) as usize;
        for i in 0..entries {
            let entry = base + 12 * i;
            if entry + 12 > data.len() {
                break;
            }
            for offset in [0usize, 4, 8] {
                fields.push((entry + offset, 4));
            }
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

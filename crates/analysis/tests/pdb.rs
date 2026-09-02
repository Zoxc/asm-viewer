//! The crate pinned against a PE image and the `.pdb` a real linker wrote for it, where
//! every other linked image in the suite is hand-assembled in memory and no writer in the
//! tree can produce a PDB at all.
//!
//! Both files are committed, built from `tests/fixtures/line_fixture.c` — the same source the
//! two gcc objects were built from, so the three functions and their line numbers are the
//! ones `real_object.rs` already asserts. From `tests/fixtures/`, with exactly:
//!
//! ```text
//! clang-cl --target=x86_64-pc-windows-msvc /c /Z7 /Od /GS- -ffile-compilation-dir=/fixture \
//!     /clang:-gcolumn-info /Fo line_fixture.obj line_fixture.c
//! "$(rustc +stable --print sysroot)"/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld \
//!     -flavor link /DEBUG /Brepro /PDBALTPATH:line_fixture.pdb /PDBSOURCEPATH:/fixture \
//!     /NODEFAULTLIB /NOENTRY /DLL /EXPORT:add /EXPORT:twice /EXPORT:sum_to \
//!     /OUT:line_fixture.dll /PDB:line_fixture.pdb line_fixture.obj
//! rm line_fixture.obj line_fixture.lib
//! ```
//!
//! built with clang version 22.1.8 (Fedora 22.1.8-4.fc44) and the `rust-lld` of rustc 1.98.0
//! (88d9e12ae 2026-08-18), `lld-link` in all but name.
//!
//! What each flag is for: `/Z7` puts CodeView in the object for the linker to gather;
//! `-ffile-compilation-dir=/fixture` records the source as `/fixture/line_fixture.c` rather
//! than as whoever's checkout it was built in (`-fdebug-prefix-map`'s job for the gcc
//! objects); `-gcolumn-info` because `clang-cl` records no columns by default and the rows
//! are asserted with theirs; `/Od` keeps each statement on a row of its own; `/Brepro` makes
//! the image's timestamp and GUID a hash of its contents; `/PDBALTPATH` records a bare
//! `line_fixture.pdb` in the debug directory, so the recorded path is found *beside* the
//! DLL and not at a build machine's absolute path; `/PDBSOURCEPATH` does the same for the
//! working directory the PDB records; `/NODEFAULTLIB /NOENTRY /DLL` link nothing but the one
//! object, so no CRT is needed and the `.text` is exactly the three functions; and the
//! three `/EXPORT`s are what names them, since `/DEBUG` writes no COFF symbol table into the
//! image. The PDB still records the linker's own path and the object's absolute path in its
//! build records; nothing asserts on either.
//!
//! The DLL is 2.5 KB and the PDB 72 KB — an MSF file's smallest shape, 18 pages of 4 KB.
//!
//! A **second pair**, `line_fixture_noexport.dll` + `.pdb`, is the same object linked with
//! no `/EXPORT`s at all, so the image declares nothing — no symbol table, no exports, no
//! entry point — and every name it shows is the PDB's. From `tests/fixtures/`, with exactly:
//!
//! ```text
//! clang-cl --target=x86_64-pc-windows-msvc /c /Z7 /Od /GS- -ffile-compilation-dir=/fixture \
//!     /clang:-gcolumn-info /Foline_fixture_noexport.obj line_fixture.c
//! "$(rustc +stable --print sysroot)"/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld \
//!     -flavor link /DEBUG /Brepro /PDBALTPATH:line_fixture_noexport.pdb \
//!     /PDBSOURCEPATH:/fixture /NODEFAULTLIB /NOENTRY /DLL \
//!     /OUT:line_fixture_noexport.dll /PDB:line_fixture_noexport.pdb line_fixture_noexport.obj
//! rm line_fixture_noexport.obj
//! ```
//!
//! built with the same clang 22.1.8 (Fedora 22.1.8-4.fc44) and the `rust-lld` of rustc 1.98.0
//! (88d9e12ae 2026-08-18). `/Fo` takes its name attached — with a space `clang-cl` reads a
//! bare `/Fo` and names the object after the source, which is how the first recipe's
//! `/Fo line_fixture.obj` happened to work. No `.lib` is written for an image exporting
//! nothing. `/Brepro` hashes the contents, so the two pairs have different GUIDs and neither
//! PDB matches the other's DLL. Same 2.5 KB and 72 KB.

mod common;

use analysis::{parse_object, LineInfo, Object, SourceDigests, SourceHash};
use common::{
    committed_fixture, committed_fixture_path, names, pe_image, symbol, CodeViewRecord,
    ExportedSymbol, PeDll,
};
use object::{Object as _, ObjectKind, ObjectSection};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DLL: &str = "line_fixture.dll";
const PDB: &str = "line_fixture.pdb";

/// The source as the PDB spells it: `-ffile-compilation-dir` joined to the name given.
const SOURCE: &str = "/fixture/line_fixture.c";

/// The image base `rust-lld` gives a DLL, plus `.text`'s RVA: where the three functions
/// are, in the address space `SymbolData::address` speaks.
const TEXT: u64 = 0x1_8000_1000;

/// The fixture parsed **under its real path**, which is where its `.pdb` is looked for.
fn parse() -> Arc<Object> {
    let bytes = committed_fixture(DLL);
    parse_object(
        bytes.as_slice().into(),
        DLL.to_string(),
        committed_fixture_path(DLL),
    )
    .expect("the DLL parses")
}

/// The fixture is a linked x86-64 image with no symbol table, whose debug directory names
/// the `.pdb` committed beside it — the shape a stripped `.exe`/`.dll` built with `/DEBUG`
/// has, and the one nothing else in the suite reaches.
#[test]
fn the_fixture_is_a_linked_image_naming_its_pdb() {
    let bytes = committed_fixture(DLL);
    let file = object::File::parse(bytes.as_slice()).expect("a PE image");
    assert_eq!(file.kind(), ObjectKind::Dynamic);
    assert_eq!(file.architecture(), object::Architecture::X86_64);
    assert_eq!(
        file.symbols().count(),
        0,
        "/DEBUG writes no COFF symbol table"
    );

    let codeview = file
        .pdb_info()
        .expect("the debug directory parses")
        .expect("there is a CodeView record");
    assert_eq!(
        codeview.path(),
        PDB.as_bytes(),
        "/PDBALTPATH records a bare name"
    );
    assert_eq!(codeview.age(), 1);
    assert_ne!(codeview.guid(), [0; 16]);

    assert!(
        committed_fixture_path(PDB).is_file(),
        "the .pdb is committed beside the .dll"
    );
}

/// The in-memory PE writer can name a `.pdb` too, in the same debug-directory shape the
/// linker uses — which is how a test points an image at any path, GUID and age it likes.
#[test]
fn an_image_built_in_memory_can_name_a_pdb() {
    let image = pe_image(PeDll {
        text: &[0x90, 0xC3],
        symbols: &[ExportedSymbol {
            name: "first",
            offset: 0,
            size: 2,
            code: true,
        }],
        entry: None,
        codeview: Some(CodeViewRecord {
            guid: *b"0123456789abcdef",
            age: 7,
            path: "C:\\build\\fixture.pdb",
        }),
    });
    let file = object::File::parse(image.as_slice()).expect("a PE image");
    let codeview = file
        .pdb_info()
        .expect("the debug directory parses")
        .expect("there is a CodeView record");
    assert_eq!(codeview.guid(), *b"0123456789abcdef");
    assert_eq!(codeview.age(), 7);
    assert_eq!(codeview.path(), b"C:\\build\\fixture.pdb");

    // And the export directory beside it still reads.
    let object = common::parse(&image);
    assert_eq!(names(&object), ["first"]);
}

/// The three exports are the three functions, at the addresses the linker laid them out at:
/// `add` first, `twice` and `sum_to` each on the next 32-byte boundary.
#[test]
fn the_exports_are_the_three_functions() {
    let object = parse();
    assert_eq!(names(&object), ["add", "sum_to", "twice"]);
    assert_eq!(symbol(&object, "add").address, TEXT);
    assert_eq!(symbol(&object, "twice").address, TEXT + 0x20);
    assert_eq!(symbol(&object, "sum_to").address, TEXT + 0x40);
    for name in ["add", "twice", "sum_to"] {
        let symbol = symbol(&object, name);
        assert!(
            symbol.assembly(&object).is_some(),
            "{name} decodes from its export"
        );
    }
}

/// Every row as `(start, end, line, column)` relative to `.text`, the form the expectations
/// are written in.
fn rows(info: &LineInfo) -> Vec<(u64, u64, Option<u32>, Option<u32>)> {
    info.rows()
        .iter()
        .map(|row| {
            (
                row.range.start - TEXT,
                row.range.end - TEXT,
                row.line,
                row.column,
            )
        })
        .collect()
}

fn line_info(object: &Object, name: &str) -> Arc<LineInfo> {
    symbol(object, name)
        .line_info(object)
        .unwrap_or_else(|| panic!("{name} has line info"))
}

/// A directory of this test's own under the target directory, empty, for the copies a
/// finder case needs on disk.
fn scratch(case: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("pdb")
        .join(case);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// The DLL's bytes parsed as if they sat at `path`, which is where its `.pdb` is looked for.
fn parse_at(bytes: &[u8], path: PathBuf) -> Arc<Object> {
    parse_object(bytes.into(), DLL.to_string(), path).expect("the DLL parses")
}

/// Every row of the three functions, verbatim: `clang-cl` at `/Od` puts each expression on a
/// row of its own with its column, and a row on the line of the opening brace with none.
/// The loop in `sum_to` walks *back* through the source, as the gcc build's does.
#[test]
fn every_row_of_the_three_functions_verbatim() {
    let object = parse();

    assert_eq!(
        rows(&line_info(&object, "add")),
        [
            (0x00, 0x08, Some(22), None),
            (0x08, 0x0b, Some(23), Some(9)),
            (0x0b, 0x0f, Some(23), Some(11)),
            (0x0f, 0x11, Some(23), Some(2)),
        ]
    );
    assert_eq!(
        rows(&line_info(&object, "twice")),
        [
            (0x20, 0x28, Some(27), None),
            (0x28, 0x2c, Some(28), Some(16)),
            (0x2c, 0x30, Some(28), Some(13)),
            (0x30, 0x36, Some(28), Some(9)),
            (0x36, 0x3b, Some(28), Some(2)),
        ]
    );
    assert_eq!(
        rows(&line_info(&object, "sum_to")),
        [
            (0x40, 0x48, Some(32), None),
            (0x48, 0x50, Some(33), Some(6)),
            (0x50, 0x58, Some(35), Some(11)),
            (0x58, 0x5c, Some(35), Some(18)),
            (0x5c, 0x60, Some(35), Some(20)),
            (0x60, 0x62, Some(35), Some(2)),
            (0x62, 0x66, Some(36), Some(22)),
            (0x66, 0x6a, Some(36), Some(15)),
            (0x6a, 0x6f, Some(36), Some(11)),
            (0x6f, 0x73, Some(36), Some(9)),
            (0x73, 0x7e, Some(35), Some(27)),
            (0x7e, 0x80, Some(35), Some(2)),
            (0x80, 0x84, Some(38), Some(9)),
            (0x84, 0x89, Some(38), Some(2)),
        ]
    );
}

/// The one file is named as the compiler recorded it — verbatim, forward slashes and all —
/// and carries the MD5 the compiler took of it, which is the MD5 of the committed source.
#[test]
fn the_file_is_named_verbatim_and_carries_the_compilers_md5() {
    let object = parse();
    let info = line_info(&object, "add");
    let files: Vec<&str> = info.files().iter().map(|file| &**file).collect();
    assert_eq!(files, [SOURCE]);
    assert_eq!(info.file_of(&info.rows()[0]), Some(SOURCE));

    let recorded = info.hash_of(0).expect("the PDB records a checksum");
    assert!(matches!(recorded, SourceHash::Md5(_)));
    assert_eq!(info.hash_of(1), None, "there is no second file");

    let source = SourceDigests::of(&committed_fixture("line_fixture.c"));
    assert!(
        recorded.matches(&source),
        "the committed source is the one compiled"
    );
    let edited = SourceDigests::of(b"int add(int a, int b) { return a - b; }\n");
    assert!(!recorded.matches(&edited));
}

/// A procedure's declared length is the symbol's extent: the export table has no size, so
/// the estimate reaches to the next export (and past `sum_to` to the end of `.text`), and
/// the PDB is what trims each back to its function.
#[test]
fn a_procedures_length_is_the_declared_extent() {
    let object = parse();
    for (name, len) in [("add", 0x11), ("twice", 0x1b), ("sum_to", 0x49)] {
        let symbol = symbol(&object, name);
        assert_eq!(symbol.debug_extent(&object), Some(len), "{name}");
        assert_eq!(symbol.extent(&object), Some(len), "{name}");
        // The estimate reaches to the next export, or for `sum_to` to the end of `.text`,
        // which is exactly where its last instruction is: the PDB trims the first two and
        // agrees about the third.
        let estimate = symbol.estimate_size().unwrap();
        assert!(estimate >= len, "{name}: the estimate under-reaches");
        assert_eq!(estimate > len, name != "sum_to", "{name}");
    }
    // An address that begins no procedure declares no extent.
    let text = object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect(".text");
    assert_eq!(object.function_extent(text, TEXT + 0x08), None);
    assert_eq!(object.function_extent(text, TEXT + 0x200), None);
}

/// The rows hold `LineInfo`'s invariants — ascending, non-overlapping, inside the range asked
/// about — and every instruction is answered from them.
#[test]
fn the_rows_hold_the_invariants() {
    let object = parse();
    for name in ["add", "twice", "sum_to"] {
        let symbol = symbol(&object, name);
        let info = line_info(&object, name);
        let end = symbol.address + symbol.extent(&object).unwrap();
        let mut previous = symbol.address;
        for row in info.rows() {
            assert!(
                row.range.start >= previous,
                "{name}: rows overlap or descend"
            );
            assert!(row.range.end <= end, "{name}: a row past the extent");
            previous = row.range.end;
        }
        for address in symbol.address..end {
            let row = info
                .row_at(address)
                .unwrap_or_else(|| panic!("{name}: no row at +{:#x}", address - symbol.address));
            assert!(row.range.contains(&address));
        }
        assert!(info.row_at(end).is_none());
    }
}

/// The reverse direction runs through the same rows: a line answers with the symbols it was
/// compiled into, and every line a symbol's rows name answers with that symbol.
#[test]
fn a_line_maps_back_to_the_symbol_compiled_from_it() {
    let object = parse();
    let names_at = |line: u32| -> Vec<String> {
        object
            .symbols_at_line(SOURCE, line)
            .iter()
            .map(|symbol| symbol.name.clone())
            .collect()
    };
    assert_eq!(names_at(23), ["add"]);
    assert_eq!(names_at(28), ["twice"]);
    assert_eq!(names_at(35), ["sum_to"]);
    assert_eq!(
        names_at(25),
        Vec::<String>::new(),
        "a blank line compiled into nothing"
    );
    assert_eq!(names_at(23).len(), 1);

    let all: Vec<String> = object
        .symbols_from_source(SOURCE, 1..100)
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(all, ["add", "twice", "sum_to"], "address order");

    assert!(
        object.symbols_at_line("line_fixture.c", 23).is_empty(),
        "matched exactly"
    );

    for name in ["add", "twice", "sum_to"] {
        let info = line_info(&object, name);
        for row in info.rows() {
            let line = row.line.expect("every row here names a line");
            assert!(
                names_at(line).contains(&name.to_string()),
                "{name}'s line {line} does not answer with {name}"
            );
        }
    }
}

/// Where the `RSDS` record sits in the DLL, so a test can spoil its GUID or age in memory.
fn codeview_record(dll: &[u8]) -> usize {
    dll.windows(4)
        .position(|window| window == b"RSDS")
        .expect("an RSDS record")
}

/// A `.pdb` is taken only when both its GUID and its age are the image's: the GUID says
/// which build and the age which relink, and an incremental relink keeps the GUID.
#[test]
fn a_pdb_with_another_guid_or_age_is_not_this_images() {
    let path = committed_fixture_path(DLL);
    let record = codeview_record(&committed_fixture(DLL));

    let mut other_guid = committed_fixture(DLL);
    other_guid[record + 4] ^= 0x01;
    let object = parse_at(&other_guid, path.clone());
    assert!(symbol(&object, "add").line_info(&object).is_none());
    assert!(object.symbols_at_line(SOURCE, 23).is_empty());

    let mut other_age = committed_fixture(DLL);
    other_age[record + 20] = 2;
    let object = parse_at(&other_age, path);
    assert!(symbol(&object, "add").line_info(&object).is_none());
    assert!(symbol(&object, "add").debug_extent(&object).is_none());
}

/// The `.pdb` is looked for in three places, in order: at the recorded path where it is
/// absolute, under the recorded name beside the binary, and under the binary's own name
/// beside it. Nowhere is "no line info", not an error.
#[test]
fn the_pdb_is_found_beside_the_binary_by_either_name_or_not_at_all() {
    let dll = committed_fixture(DLL);
    let pdb = committed_fixture(PDB);

    // Under the recorded name, beside a binary called something else.
    let dir = scratch("recorded_name");
    std::fs::write(dir.join(PDB), &pdb).unwrap();
    let object = parse_at(&dll, dir.join("renamed.dll"));
    assert_eq!(rows(&line_info(&object, "add")).len(), 4);

    // Under the binary's own name, the recorded name being nowhere.
    let dir = scratch("binary_name");
    std::fs::write(dir.join("renamed.pdb"), &pdb).unwrap();
    let object = parse_at(&dll, dir.join("renamed.dll"));
    assert_eq!(rows(&line_info(&object, "add")).len(), 4);

    // Beside the binary under another name entirely: not found.
    let dir = scratch("other_name");
    std::fs::write(dir.join("elsewhere.pdb"), &pdb).unwrap();
    let object = parse_at(&dll, dir.join("renamed.dll"));
    assert!(symbol(&object, "add").line_info(&object).is_none());

    // Nothing beside it at all.
    let dir = scratch("alone");
    let object = parse_at(&dll, dir.join("alone.dll"));
    assert!(symbol(&object, "add").line_info(&object).is_none());
    assert!(object.symbols_at_line(SOURCE, 23).is_empty());
}

/// The recorded path itself is tried first where it is absolute — the build machine's path,
/// when this *is* the build machine. An image assembled in memory with the fixture's own
/// `.text`, GUID and age, naming a copy of the PDB by absolute path from a directory holding
/// no `.pdb`, is answered from that copy; the same image naming a relative path is not.
#[test]
fn an_absolute_recorded_path_is_tried_as_recorded() {
    let dll = committed_fixture(DLL);
    let file = object::File::parse(dll.as_slice()).unwrap();
    let codeview = file.pdb_info().unwrap().unwrap();
    let text = file
        .section_by_name(".text")
        .unwrap()
        .data()
        .unwrap()
        .to_vec();
    const EXPORTS: &[ExportedSymbol] = &[
        ExportedSymbol {
            name: "add",
            offset: 0,
            size: 0,
            code: true,
        },
        ExportedSymbol {
            name: "twice",
            offset: 0x20,
            size: 0,
            code: true,
        },
        ExportedSymbol {
            name: "sum_to",
            offset: 0x40,
            size: 0,
            code: true,
        },
    ];

    let elsewhere = scratch("absolute_elsewhere");
    let copy = elsewhere.join("build.pdb");
    std::fs::write(&copy, committed_fixture(PDB)).unwrap();
    let empty = scratch("absolute_empty");

    let image = |recorded: &str| {
        pe_image(PeDll {
            text: &text,
            symbols: EXPORTS,
            entry: None,
            codeview: Some(CodeViewRecord {
                guid: codeview.guid(),
                age: codeview.age(),
                path: recorded,
            }),
        })
    };

    let object = parse_at(&image(copy.to_str().unwrap()), empty.join("image.dll"));
    let add = symbol(&object, "add");
    assert_eq!(add.debug_extent(&object), Some(0x11));
    let info = add.line_info(&object).expect("found at the recorded path");
    assert_eq!(info.rows().len(), 4);
    assert_eq!(
        info.rows()[0].range.start,
        add.address,
        "in the image's own address space"
    );
    assert_eq!(symbol(&object, "sum_to").debug_extent(&object), Some(0x49));

    let object = parse_at(&image("build\\build.pdb"), empty.join("image.dll"));
    assert!(symbol(&object, "add").line_info(&object).is_none());
}

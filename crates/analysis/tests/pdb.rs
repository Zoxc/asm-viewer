//! The crate pinned against a PE image and the `.pdb` a real linker wrote for it, where
//! every other linked image in the suite is built in memory and no writer in the
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
//! no `/EXPORT`s at all, so the image names nothing — no symbol table, no exports, no entry
//! point; its `.pdata` still states where its three functions begin and end — and every
//! name it shows is the PDB's. From `tests/fixtures/`, with exactly:
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
//!
//! A **third pair**, `line_fixture_public.dll` + `.pdb`, is that same object linked together
//! with a second one, `public_fixture.cpp`, compiled **without** `/Z7`: its one function,
//! `helper`, has no module symbols in the PDB at all, so the only name the PDB holds for it
//! is the linker's public (`S_PUB32`) — and, `public_fixture.cpp` being C++, that name is the
//! decorated `?helper@@YAHXZ`, where the C functions' publics are the plain `add`, `twice`
//! and `sum_to`. Again no `/EXPORT`s, so the image names nothing — and `helper`, a leaf, has
//! no unwind entry either. From `tests/fixtures/`, with exactly:
//!
//! ```text
//! clang-cl --target=x86_64-pc-windows-msvc /c /Z7 /Od /GS- -ffile-compilation-dir=/fixture \
//!     /clang:-gcolumn-info /Foline_fixture_public.obj line_fixture.c
//! clang-cl --target=x86_64-pc-windows-msvc /c /Od /GS- /Fopublic_fixture.obj public_fixture.cpp
//! "$(rustc +stable --print sysroot)"/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld \
//!     -flavor link /DEBUG /Brepro /PDBALTPATH:line_fixture_public.pdb /PDBSOURCEPATH:/fixture \
//!     /NODEFAULTLIB /NOENTRY /DLL /OUT:line_fixture_public.dll /PDB:line_fixture_public.pdb \
//!     line_fixture_public.obj public_fixture.obj
//! rm line_fixture_public.obj public_fixture.obj line_fixture_public.lib
//! ```
//!
//! built with the same clang 22.1.8 (Fedora 22.1.8-4.fc44) and the `rust-lld` of rustc 1.98.0
//! (88d9e12ae 2026-08-18). This pair stands in for a **stripped** PDB — `/PDBSTRIPPED`, which
//! keeps the publics and drops every module stream, so publics are all a PDB has — because
//! that `rust-lld` accepts `/PDBSTRIPPED` and then warns `ignoring /pdbstripped flag, it is
//! not yet supported`, writing no stripped file; an object without `/Z7` is the same shape
//! for its one function. `helper` is at `.text` + 0x90, six bytes after `sum_to`'s end
//! rounded up to 16, and the DLL is again 2.5 KB with the PDB 76 KB, one page more for the
//! second module.

mod common;

use analysis::{parse_object, LineInfo, Object, SourceDigests, SourceHash};
use common::{
    at, committed_fixture, committed_fixture_path, names, pe_image, symbol, CodeViewRecord,
    ExportedSymbol, PeDll,
};
use object::{Object as _, ObjectKind, ObjectSection};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DLL: &str = "line_fixture.dll";
const PDB: &str = "line_fixture.pdb";
const NOEXPORT_DLL: &str = "line_fixture_noexport.dll";
const PUBLIC_DLL: &str = "line_fixture_public.dll";

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
        unwind: &[],
        fragments: &[],
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
    assert_eq!(symbol(&object, "add").address, at(TEXT));
    assert_eq!(symbol(&object, "twice").address, at(TEXT + 0x20));
    assert_eq!(symbol(&object, "sum_to").address, at(TEXT + 0x40));
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
                row.range.start.get() - TEXT,
                row.range.end.get() - TEXT,
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
    let files: Vec<&str> = info.files().map(|file| &**file).collect();
    assert_eq!(files, [SOURCE]);
    assert_eq!(common::file_of(&info, &info.rows()[0]), Some(SOURCE));

    let recorded = info.hash_for(SOURCE).expect("the PDB records a checksum");
    assert!(matches!(recorded, SourceHash::Md5(_)));
    assert_eq!(info.hash_for("other.c"), None, "there is no second file");

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
        assert_eq!(
            symbol.extent(&object).map(|extent| extent.bytes),
            Some(len),
            "{name}"
        );
        // The estimate reaches to the next export, or for `sum_to` to the end of `.text`,
        // which is exactly where its last instruction is: the PDB trims the first two and
        // agrees about the third.
        let estimate = symbol.estimate_size(&object).unwrap().bytes;
        assert!(estimate >= len, "{name}: the estimate under-reaches");
        assert_eq!(estimate > len, name != "sum_to", "{name}");
    }
    // An address that begins no procedure declares no extent.
    let text = object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect(".text");
    assert_eq!(object.function_extent(text, at(TEXT + 0x08)), None);
    assert_eq!(object.function_extent(text, at(TEXT + 0x200)), None);
}

/// The rows hold `LineInfo`'s invariants — ascending, non-overlapping, inside the range asked
/// about — and every instruction is answered from them.
#[test]
fn the_rows_hold_the_invariants() {
    let object = parse();
    for name in ["add", "twice", "sum_to"] {
        let symbol = symbol(&object, name);
        let info = line_info(&object, name);
        let extent = symbol.extent(&object).unwrap().bytes;
        let end = symbol
            .address
            .checked_add(extent)
            .expect("the fixture fits in the address space");
        let mut previous = symbol.address;
        for row in info.rows() {
            assert!(
                row.range.start >= previous,
                "{name}: rows overlap or descend"
            );
            assert!(row.range.end <= end, "{name}: a row past the extent");
            previous = row.range.end;
        }
        for offset in 0..extent {
            let address = at(symbol.address.get() + offset);
            let row = info
                .row_at(address)
                .unwrap_or_else(|| panic!("{name}: no row at +{offset:#x}"));
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
            .symbols_from_lines(SOURCE, line..=line)
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
        .symbols_from_lines(SOURCE, 1..=99)
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(all, ["add", "twice", "sum_to"], "address order");

    assert!(
        object
            .symbols_from_lines("line_fixture.c", 23..=23)
            .is_empty(),
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
    assert!(object.symbols_from_lines(SOURCE, 23..=23).is_empty());

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
    assert!(object.symbols_from_lines(SOURCE, 23..=23).is_empty());
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
            unwind: &[],
            fragments: &[],
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

/// Where the image's unwind table and its PDB both say how long a function is, the image's
/// word is the extent and the PDB's is only the declared size: the same `.text`, GUID and
/// age as the committed DLL, but an entry for `add` reaching seven bytes into the `int3`
/// padding after it, gives `add` those seven bytes too. The two agree in every real image;
/// this pins which one is asked.
#[test]
fn a_stated_end_beats_the_procedures_length() {
    let dll = committed_fixture(DLL);
    let file = object::File::parse(dll.as_slice()).unwrap();
    let codeview = file.pdb_info().unwrap().unwrap();
    let text = file
        .section_by_name(".text")
        .unwrap()
        .data()
        .unwrap()
        .to_vec();
    let dir = scratch("stated_end");
    std::fs::write(dir.join("build.pdb"), committed_fixture(PDB)).unwrap();
    let image = pe_image(PeDll {
        text: &text,
        symbols: &[ExportedSymbol {
            name: "add",
            offset: 0,
            size: 0,
            code: true,
        }],
        entry: None,
        codeview: Some(CodeViewRecord {
            guid: codeview.guid(),
            age: codeview.age(),
            path: "build.pdb",
        }),
        unwind: &[(0, 0x18)],
        fragments: &[],
    });

    let object = parse_at(&image, dir.join("image.dll"));
    let add = symbol(&object, "add");
    assert_eq!(
        add.debug_extent(&object),
        Some(0x11),
        "the procedure's length"
    );
    assert_eq!(add.size, None, "an export declares no size");
    assert_eq!(
        add.extent(&object).map(|extent| extent.bytes),
        Some(0x18),
        "the unwind entry's end"
    );
    // Not `rows`, which is written for the linker's image base; this image is at the
    // writer's.
    assert_eq!(
        line_info(&object, "add").rows().len(),
        4,
        "the PDB was read"
    );
}

/// The three `<function 0x…>` names the no-export images show on their own: what their
/// `.pdata` states and nothing names, in the Symbols list's byte order.
fn unwind_names() -> Vec<String> {
    [0x00, 0x20, 0x40]
        .map(|offset| format!("<function {:#x}>", TEXT + offset))
        .to_vec()
}

/// The image that names nothing shows, on its own — parsed at a path with no `.pdb` beside
/// it — only what its `.pdata` states: three functions, each `<function 0x…>` by its address
/// with the entry's stated length and no lines; and, parsed where its `.pdb` is, shows the
/// PDB's three procedures as symbols: named as the records name them, at the addresses the
/// linker laid them out at, each with its line info and its declared length, and answering
/// the reverse index as an export would.
#[test]
fn procedures_are_symbols_where_the_image_names_none() {
    let bytes = committed_fixture(NOEXPORT_DLL);
    let file = object::File::parse(bytes.as_slice()).expect("a PE image");
    assert_eq!(file.symbols().count(), 0);
    assert_eq!(file.exports().unwrap().count(), 0, "no /EXPORT");
    assert_eq!(
        file.entry(),
        file.relative_address_base(),
        "/NOENTRY: an entry RVA of 0, which `object` adds the base to"
    );

    let alone = parse_at(&bytes, scratch("noexport_alone").join("alone.dll"));
    assert_eq!(names(&alone), unwind_names());
    for (offset, len) in [(0x00, 0x11), (0x20, 0x1b), (0x40, 0x49)] {
        let function = symbol(&alone, &format!("<function {:#x}>", TEXT + offset));
        assert_eq!(function.address, at(TEXT + offset));
        assert_eq!(function.size, Some(len), "the entry's stated length");
        assert_eq!(function.debug_extent(&alone), None, "no PDB");
        assert_eq!(
            function.extent(&alone).map(|extent| extent.bytes),
            Some(len),
            "the stated end"
        );
        assert!(function.line_info(&alone).is_none(), "no PDB, no lines");
    }

    let object = parse_at(&bytes, committed_fixture_path(NOEXPORT_DLL));
    assert_eq!(names(&object), ["add", "sum_to", "twice"]);
    for (name, offset, len) in [
        ("add", 0x00, 0x11),
        ("twice", 0x20, 0x1b),
        ("sum_to", 0x40, 0x49),
    ] {
        let symbol = symbol(&object, name);
        assert_eq!(symbol.address, at(TEXT + offset), "{name}");
        assert_eq!(
            symbol.size,
            Some(len),
            "{name}: the procedure's length is the declared size"
        );
        assert_eq!(
            symbol.demangled, None,
            "{name}: a display name demangles to nothing"
        );
        assert_eq!(symbol.debug_extent(&object), Some(len), "{name}");
        assert_eq!(
            symbol.extent(&object).map(|extent| extent.bytes),
            Some(len),
            "{name}"
        );
        assert!(symbol.assembly(&object).is_some(), "{name} decodes");
    }
    assert_eq!(rows(&line_info(&object, "add")).len(), 4);
    assert_eq!(rows(&line_info(&object, "twice")).len(), 5);
    assert_eq!(rows(&line_info(&object, "sum_to")).len(), 14);

    let at_23: Vec<String> = object
        .symbols_from_lines(SOURCE, 23..=23)
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(at_23, ["add"]);
}

/// Where an export already names an address, the PDB's procedure at that address adds no
/// second symbol: the exported pair still lists exactly its three exports, under their
/// exported names.
#[test]
fn a_procedure_never_displaces_an_export() {
    let object = parse();
    assert_eq!(names(&object), ["add", "sum_to", "twice"]);
    assert_eq!(object.symbols.len(), 3);
    for name in ["add", "twice", "sum_to"] {
        assert_eq!(
            symbol(&object, name).size,
            None,
            "{name}: an export declares no size"
        );
    }
}

/// A `.pdb` that is not this image's names nothing either, whatever it knows: the symbols
/// are the unwind table's own, as with no `.pdb` at all.
#[test]
fn a_pdb_with_another_guid_adds_no_names() {
    let mut other_guid = committed_fixture(NOEXPORT_DLL);
    let record = codeview_record(&other_guid);
    other_guid[record + 4] ^= 0x01;
    let object = parse_at(&other_guid, committed_fixture_path(NOEXPORT_DLL));
    assert_eq!(names(&object), unwind_names());
}

/// A function no module's symbols describe is named by the PDB's **publics**: the third pair
/// parsed beside its PDB lists `helper` — as the linker spelled it, decorated, and demangled
/// from that — at the address its public states, with no size, no extent and no line info,
/// since a public is a name and an address and nothing else; and the three functions the
/// modules do describe are still the procedures, with their lengths. Parsed at a path with
/// no `.pdb` beside it, the image, which exports nothing, shows only what its `.pdata`
/// states — and `helper`, a leaf with no unwind entry, is not among them.
#[test]
fn a_public_names_the_function_no_module_describes() {
    let bytes = committed_fixture(PUBLIC_DLL);
    let file = object::File::parse(bytes.as_slice()).expect("a PE image");
    assert_eq!(file.symbols().count(), 0);
    assert_eq!(file.exports().unwrap().count(), 0, "no /EXPORT");

    let alone = parse_at(&bytes, scratch("public_alone").join("alone.dll"));
    assert_eq!(names(&alone), unwind_names());

    let object = parse_at(&bytes, committed_fixture_path(PUBLIC_DLL));
    assert_eq!(names(&object), ["?helper@@YAHXZ", "add", "sum_to", "twice"]);

    let helper = symbol(&object, "?helper@@YAHXZ");
    assert_eq!(helper.address, at(TEXT + 0x90));
    assert_eq!(helper.size, None, "a public declares no size");
    assert_eq!(
        helper.demangled.as_deref(),
        Some("int helper(void)"),
        "a public's name is the decorated one, and is demangled"
    );
    assert_eq!(helper.debug_extent(&object), None, "no module knows it");
    assert_eq!(
        helper.extent(&object).map(|extent| extent.bytes),
        Some(6),
        "a leaf with no unwind entry: the estimate, to the section's end"
    );
    assert!(
        helper.line_info(&object).is_none(),
        "no module has its lines"
    );
    let assembly = helper.assembly(&object).expect("helper decodes");
    assert_eq!(assembly.instructions.len(), 2, "mov eax, 7; ret");

    for (name, offset, len) in [
        ("add", 0x00, 0x11),
        ("twice", 0x20, 0x1b),
        ("sum_to", 0x40, 0x49),
    ] {
        let symbol = symbol(&object, name);
        assert_eq!(symbol.address, at(TEXT + offset), "{name}");
        assert_eq!(
            symbol.size,
            Some(len),
            "{name}: still the procedure's length"
        );
        assert_eq!(
            symbol.extent(&object).map(|extent| extent.bytes),
            Some(len),
            "{name}"
        );
        assert_eq!(symbol.demangled, None, "{name}");
    }
    assert_eq!(rows(&line_info(&object, "add")).len(), 4);
}

/// Where a procedure already names an address, the public at that address adds no second
/// symbol: the no-export pair's PDB holds a public for each of its three functions too, and
/// the image still lists exactly three, under the procedures' names and with their lengths.
#[test]
fn a_public_never_displaces_a_procedure() {
    let bytes = committed_fixture(NOEXPORT_DLL);
    let object = parse_at(&bytes, committed_fixture_path(NOEXPORT_DLL));
    assert_eq!(names(&object), ["add", "sum_to", "twice"]);
    assert_eq!(object.symbols.len(), 3);
    for (name, len) in [("add", 0x11), ("twice", 0x1b), ("sum_to", 0x49)] {
        assert_eq!(
            symbol(&object, name).size,
            Some(len),
            "{name}: the procedure's length"
        );
    }
}

/// A PDB's multi-stream file as far as a test patches it: the page size, and where each
/// stream's pages are. The fixtures' stream directory fits on one page.
struct Msf {
    bytes: Vec<u8>,
    page: usize,
    pages: Vec<Vec<usize>>,
}

impl Msf {
    fn new(pdb: &[u8]) -> Msf {
        let u32_at = |at: usize| u32::from_le_bytes(pdb[at..at + 4].try_into().unwrap());
        let page = u32_at(32) as usize;
        let directory = u32_at(u32_at(52) as usize * page) as usize * page;
        let count = u32_at(directory) as usize;
        let sizes: Vec<u32> = (0..count)
            .map(|stream| u32_at(directory + 4 + 4 * stream))
            .collect();
        let mut at = directory + 4 + 4 * count;
        let pages = sizes
            .iter()
            .map(|&size| {
                let count = if size == u32::MAX {
                    0
                } else {
                    (size as usize).div_ceil(page)
                };
                let pages = (0..count)
                    .map(|index| u32_at(at + 4 * index) as usize)
                    .collect();
                at += 4 * count;
                pages
            })
            .collect();
        Msf {
            bytes: pdb.to_vec(),
            page,
            pages,
        }
    }

    /// Where byte `at` of `stream` is in the file.
    fn offset(&self, stream: usize, at: usize) -> usize {
        self.pages[stream][at / self.page] * self.page + at % self.page
    }

    fn u32_at(&self, stream: usize, at: usize) -> u32 {
        let at = self.offset(stream, at);
        u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap())
    }

    fn u16_at(&self, stream: usize, at: usize) -> u16 {
        let at = self.offset(stream, at);
        u16::from_le_bytes(self.bytes[at..at + 2].try_into().unwrap())
    }

    fn write(&mut self, stream: usize, at: usize, bytes: &[u8]) {
        for (index, &byte) in bytes.iter().enumerate() {
            let at = self.offset(stream, at + index);
            self.bytes[at] = byte;
        }
    }
}

/// The DBI is stream 3.
const DBI: usize = 3;

/// The stream module `index`'s symbols are in: its DBI record states it at 34. The records
/// follow the DBI's 64-byte header, each 64 bytes of fields and then two NUL-terminated
/// names, padded to 4.
fn module_stream(msf: &Msf, index: usize) -> usize {
    usize::from(msf.u16_at(DBI, module_record(msf, index) + 34))
}

/// Where module `index`'s record is in the DBI. The records follow the DBI's 64-byte header,
/// each 64 bytes of fields and then two NUL-terminated names, padded to 4.
fn module_record(msf: &Msf, index: usize) -> usize {
    let mut record = 64;
    for _ in 0..index {
        let mut at = record + 64;
        for _ in 0..2 {
            while msf.bytes[msf.offset(DBI, at)] != 0 {
                at += 1;
            }
            at += 1;
        }
        record = at.next_multiple_of(4);
    }
    record
}

/// The symbol record at `at` in `stream` rewritten as one of `kind` whose data begins with
/// `count`. The record keeps its length, so the walk steps past it as before.
fn rewritten(msf: &mut Msf, stream: usize, at: usize, kind: u16, count: u32) {
    msf.write(stream, at + 2, &kind.to_le_bytes());
    msf.write(stream, at + 4, &count.to_le_bytes());
}

/// An `S_INLINEES` claiming more inlinees than it holds, which a debug build of `pdb2`
/// asserts against as it parses the record (`notes/upstream/pdb2.md`).
const INLINEES: (u16, u32) = (0x1168, 0xFFFF);

/// An `S_CALLEES` stating a count of `u32::MAX`, which `pdb2` allocates 16 GiB for before
/// it checks that the record holds that many (`notes/upstream/pdb2.md`). The allocator of
/// this binary refuses the request ([`Capped`]).
const CALLEES: (u16, u32) = (0x115a, u32::MAX);

/// A symbol record of a kind the walks do not use costs nothing, whatever it states: it is
/// never parsed. Before, every record was parsed. The miscounted `S_INLINEES` panicked in a
/// debug build, costing its module's names or its public. The `S_CALLEES` asked for 16 GiB,
/// an abort no guard catches, which [`Capped`] makes certain. The third pair's PDB has three
/// modules: the object's with the three procedures, `public_fixture.obj`'s with none, and the
/// linker's with none; and a public for each of the four functions.
#[test]
fn a_symbol_record_the_walks_do_not_use_is_not_parsed() {
    let dll = committed_fixture(PUBLIC_DLL);
    let pdb = committed_fixture("line_fixture_public.pdb");
    let all = ["?helper@@YAHXZ", "add", "sum_to", "twice"];

    for (case, (kind, count)) in [("inlinees", INLINEES), ("callees", CALLEES)] {
        // The linker's module: its first record, the object name.
        let mut msf = Msf::new(&pdb);
        let linker = module_stream(&msf, 2);
        rewritten(&mut msf, linker, 4, kind, count);
        let dir = scratch(&format!("linker_{case}"));
        std::fs::write(dir.join("line_fixture_public.pdb"), &msf.bytes).unwrap();
        let object = parse_at(&dll, dir.join(PUBLIC_DLL));
        assert_eq!(names(&object), all, "{case}");
        assert_eq!(symbol(&object, "add").size, Some(0x11), "{case}");

        // The object's module: its procedures are still read, by the parse and by the
        // decode.
        let mut msf = Msf::new(&pdb);
        let module = module_stream(&msf, 0);
        rewritten(&mut msf, module, 4, kind, count);
        let dir = scratch(&format!("object_{case}"));
        std::fs::write(dir.join("line_fixture_public.pdb"), &msf.bytes).unwrap();
        let object = parse_at(&dll, dir.join(PUBLIC_DLL));
        assert_eq!(names(&object), all, "{case}");
        assert_eq!(
            symbol(&object, "add").size,
            Some(0x11),
            "{case}: a procedure's"
        );
        assert_eq!(symbol(&object, "add").debug_extent(&object), Some(0x11));
        assert_eq!(rows(&line_info(&object, "add")).len(), 4, "{case}");

        // The public for `add`, in the symbol records stream the DBI names at 20.
        let mut msf = Msf::new(&pdb);
        let records = usize::from(msf.u16_at(DBI, 20));
        rewritten(&mut msf, records, 32, kind, count);
        let dir = scratch(&format!("public_{case}"));
        std::fs::write(dir.join("line_fixture_public.pdb"), &msf.bytes).unwrap();
        let object = parse_at(&dll, dir.join(PUBLIC_DLL));
        assert_eq!(names(&object), all, "{case}");
    }
}

/// The system allocator, refusing any one request past 1 GiB. Nothing a test here reads
/// comes near that, so a refusal is a count `pdb2` believed ([`CALLEES`]); refused, it is an
/// abort that fails the run at once, where granted it would have been the machine's memory.
struct Capped;

const CAP: usize = 1 << 30;

// SAFETY: every call goes to `System` unchanged, or is refused with a null pointer, which
// `GlobalAlloc` allows for any request.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > CAP {
            return std::ptr::null_mut();
        }
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > CAP {
            return std::ptr::null_mut();
        }
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > CAP {
            return std::ptr::null_mut();
        }
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Capped = Capped;

/// The object's module with the first block of its first line subsection stating a size far
/// past the subsection. A module's C13 line data follows its symbols and its C11 lines, whose
/// sizes the module's DBI record states at 36 and 40, and is a run of subsections, each a kind
/// and a size; `pdb2` walks the lines subsections (kind `0xF2`) in order of the `offset`,
/// `section` pair each begins with, so the first is the one with the lowest.
fn first_line_block_overstated(pdb: &[u8]) -> Vec<u8> {
    let mut msf = Msf::new(pdb);
    let record = module_record(&msf, 0);
    let stream = module_stream(&msf, 0);
    let mut at = (msf.u32_at(DBI, record + 36) + msf.u32_at(DBI, record + 40)) as usize;
    let end = at + msf.u32_at(DBI, record + 44) as usize;
    let mut first: Option<((u16, u32), usize)> = None;
    while at < end {
        let (kind, size) = (msf.u32_at(stream, at), msf.u32_at(stream, at + 4));
        let data = at + 8;
        if kind == 0xF2 {
            let key = (msf.u16_at(stream, data + 4), msf.u32_at(stream, data));
            if first.is_none_or(|(first, _)| key < first) {
                first = Some((key, data));
            }
        }
        at = data + size as usize;
    }
    let (_, lines) = first.expect("a lines subsection");
    // After the subsection's 12-byte header, the block's file and line count.
    msf.write(stream, lines + 12 + 8, &0xFFFFu32.to_le_bytes());
    msf.bytes
}

/// A line block that will not read ends the module's rows there, and the module is counted;
/// before, it was without a word. `add`'s one block is the one broken, and `pdb2` walks the
/// module's subsections as one, so every row after it goes too. The procedures still give the
/// extents.
#[test]
fn a_line_block_that_does_not_read_is_counted() {
    let dll = committed_fixture(NOEXPORT_DLL);
    let pdb = committed_fixture("line_fixture_noexport.pdb");
    let dir = scratch("line_block_overstated");
    std::fs::write(
        dir.join("line_fixture_noexport.pdb"),
        first_line_block_overstated(&pdb),
    )
    .unwrap();
    let object = parse_at(&dll, dir.join(NOEXPORT_DLL));

    assert!(symbol(&object, "add").line_info(&object).is_none());
    assert_eq!(symbol(&object, "add").debug_extent(&object), Some(0x11));
    assert_eq!(object.debug_info_skipped(), 1);
}

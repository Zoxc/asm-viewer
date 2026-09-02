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

mod common;

use analysis::{parse_object, Object};
use common::{
    committed_fixture, committed_fixture_path, names, pe_image, symbol, CodeViewRecord,
    ExportedSymbol, PeDll,
};
use object::{Object as _, ObjectKind};
use std::sync::Arc;

const DLL: &str = "line_fixture.dll";
const PDB: &str = "line_fixture.pdb";

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

/// Nothing reads a `.pdb` yet: the DLL has no DWARF, so every question about its lines is
/// answered with nothing. The PDB backend flips this test.
#[test]
fn line_info_is_none_without_a_pdb_backend() {
    let object = parse();
    for name in ["add", "twice", "sum_to"] {
        let symbol = symbol(&object, name);
        assert!(symbol.line_info(&object).is_none(), "{name} has line info");
        assert!(
            symbol.debug_extent(&object).is_none(),
            "{name} has an extent"
        );
    }
    assert!(object
        .symbols_at_line("/fixture/line_fixture.c", 23)
        .is_empty());
}

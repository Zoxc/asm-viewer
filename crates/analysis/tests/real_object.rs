//! Line info read out of an object a **real compiler** produced, rather than one written
//! in memory by `gimli::write` the way every other fixture in this suite is.
//!
//! `tests/line_info.rs` proves the round trip through the formats; this proves the crate
//! against what a toolchain actually emits — DWARF 5 rather than 4, strings in
//! `.debug_line_str` behind `DW_FORM_line_strp` rather than inline, a `.debug_rnglists`
//! range list rather than a `.debug_ranges` one, `DW_AT_decl_line` disagreeing with where
//! the line program puts a function's first row, and a line program whose rows walk *back*
//! through the source as a loop is laid out. None of that is something a fixture builder
//! would think to synthesize, which is the whole reason the object is committed. The one
//! test here that is not about line info — the branch edges of `sum_to`'s loop, at the
//! bottom — is here for that same reason and no other.
//!
//! # The fixtures
//!
//! Both are built from `tests/fixtures/line_fixture.c` — three functions, `add`, `twice`
//! and `sum_to`, on the line numbers this file asserts. From `tests/fixtures`:
//!
//! ```sh
//! gcc -gdwarf-5 -O0 -fdebug-prefix-map="$PWD"=/fixture -c line_fixture.c -o line_fixture.o
//! gcc -gdwarf-5 -O0 -ffunction-sections -fdebug-prefix-map="$PWD"=/fixture \
//!     -c line_fixture.c -o line_fixture_split.o
//! ```
//!
//! with gcc (GCC) 16.1.1 20260515 (Red Hat 16.1.1-2), target x86_64-redhat-linux-gnu.
//! `-fdebug-prefix-map` is what makes the committed objects machine-independent: it is why
//! `DW_AT_comp_dir` is `/fixture` and not whoever's checkout they were built in.
//!
//! The second one exists because the first cannot show everything: gcc's default puts all
//! three functions in one `.text`, where a section bias is zero and the identity question
//! never arises. `-ffunction-sections` gives each function a section of its own, **all
//! three at address 0**, which is the shape rustc emits for every function it compiles and
//! the shape that made line info conflate whole objects (`notes/Bugs.md`). `line_info.rs`
//! covers that case with a synthesized fixture; this covers it with a real one, where the
//! same debug sections also carry relocations against `.debug_str` and `.debug_line_str`
//! that are *offsets*, not addresses, and so must survive the biasing untouched.
//!
//! Hardcoded line numbers are correct here in a way they would not be anywhere else: the
//! objects are committed and never rebuilt, so nothing in a test run can move them. To
//! keep them honest anyway, `the_source_still_says_what_the_object_says_it_does` reads the
//! `.c` beside them and checks the lines it asserts still hold the statements they name.

use analysis::{parse_object, LineInfo, Object, SymbolData};
use object::{Object as _, ObjectKind, ObjectSection, SectionKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The one `.text` build: `add` at 0, `twice` at 0x14, `sum_to` at 0x30.
const FLAT: &str = "line_fixture.o";
/// The `-ffunction-sections` build: three `.text.<name>` sections, every one at 0.
const SPLIT: &str = "line_fixture_split.o";

/// `DW_AT_comp_dir` + `DW_AT_name`, joined the way `addr2line` joins them.
const SOURCE: &str = "/fixture/line_fixture.c";

/// Read a committed fixture. Unlike the machine-local sample files in the repo root, these
/// are *in the repository*, so a missing one is a broken checkout and not a reason to skip:
/// the test fails, loudly, and says how to put it back.
fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}\n\
             This fixture is committed to the repository, not generated. Restore it from \
             git, or rebuild it with the command in tests/fixtures/line_fixture.c.",
            path.display()
        )
    })
}

fn parse(name: &str) -> Arc<Object> {
    let bytes = fixture(name);
    parse_object(
        bytes.as_slice().into(),
        name.to_string(),
        PathBuf::from(name),
    )
    .unwrap_or_else(|| panic!("{name} parses"))
}

fn symbol(object: &Object, name: &str) -> Arc<SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("{name} is a text symbol of the fixture"))
        .clone()
}

/// Every row as `(start, end, line, column)`, which is the form the expectations below are
/// written in.
fn rows(info: &LineInfo) -> Vec<(u64, u64, Option<u32>, Option<u32>)> {
    info.rows()
        .iter()
        .map(|row| (row.range.start, row.range.end, row.line, row.column))
        .collect()
}

fn line_info(object: &Object, name: &str) -> Arc<LineInfo> {
    symbol(object, name)
        .line_info(object)
        .unwrap_or_else(|| panic!("{name} has line info"))
}

// ---------------------------------------------------------------------------------------
// What the toolchain produced
// ---------------------------------------------------------------------------------------

/// The committed objects are relocatable `.o`s — not linked images — which is what makes
/// them worth having: a linked binary would have real section addresses and would never
/// exercise the relocation or biasing paths at all.
#[test]
fn the_fixtures_are_relocatable_objects() {
    for name in [FLAT, SPLIT] {
        let bytes = fixture(name);
        let file = object::File::parse(&*bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.kind(), ObjectKind::Relocatable, "{name}");
        assert!(file.is_little_endian(), "{name}");

        // DWARF 5: `.debug_line_str` is the section version 5 introduced for the strings
        // `DW_FORM_line_strp` points at, and none of the synthesized fixtures has one —
        // they write DWARF 4 with the strings inline in the line program header.
        for section in [".debug_info", ".debug_line", ".debug_line_str"] {
            assert!(
                file.section_by_name(section).is_some(),
                "{name} has no {section}"
            );
        }
    }
}

/// gcc's default is one `.text` holding every function, laid out back to back; passing
/// `-ffunction-sections` gives each its own section, and then every one of them starts at
/// address 0. Both shapes are committed because only the second can show that an address
/// alone does not identify code in a relocatable object.
#[test]
fn one_text_or_several_is_the_difference_between_the_two_fixtures() {
    let text = |name: &str| -> Vec<(String, u64, u64)> {
        let bytes = fixture(name);
        let file = object::File::parse(&*bytes).expect("parses");
        file.sections()
            .filter(|section| section.kind() == SectionKind::Text)
            // gcc leaves an empty `.text` behind in the `-ffunction-sections` build.
            .filter(|section| section.size() > 0)
            .map(|section| {
                (
                    section.name().unwrap_or_default().to_string(),
                    section.address(),
                    section.size(),
                )
            })
            .collect()
    };

    assert_eq!(text(FLAT), [(".text".to_string(), 0, 0x6e)]);

    assert_eq!(
        text(SPLIT),
        [
            (".text.add".to_string(), 0, 20),
            (".text.twice".to_string(), 0, 28),
            (".text.sum_to".to_string(), 0, 62),
        ]
    );
}

/// A real compiler declares `st_size` for its functions, where the writer-built fixtures
/// all declare 0 — so this is the one place the estimate can be checked against a truth
/// the file itself states.
#[test]
fn declared_sizes_agree_with_the_estimate() {
    let object = parse(FLAT);
    for (name, address, size) in [("add", 0x00, 20), ("twice", 0x14, 28), ("sum_to", 0x30, 62)] {
        let symbol = symbol(&object, name);
        assert_eq!(symbol.address, address, "{name} address");
        assert_eq!(symbol.size, size, "{name} declared size");
        assert_eq!(symbol.estimate_size(), Some(size), "{name} estimated size");
    }
}

// ---------------------------------------------------------------------------------------
// The mapping
// ---------------------------------------------------------------------------------------

/// The address→line mapping, written out in full. Committed objects, so these numbers are
/// facts about a file in the repository rather than about whatever compiler is installed.
///
/// Two things a real toolchain does that the synthesized fixtures do not:
///
/// * A function's first row names its **opening brace**, not its declaration — `add` is
///   declared on line 21 and its prologue is line 22. `DW_AT_decl_line` says 21; the line
///   program says 22, and the line program is what this crate reads.
/// * `sum_to`'s rows walk *backwards* through the source and then forwards again — 35, 35,
///   36, 35, 35, 38 — because gcc lays the loop's increment and test out after its body.
///   Rows ascend by address, never by line, and only the first is an invariant.
#[test]
fn every_row_of_every_function_is_where_the_compiler_put_it() {
    let object = parse(FLAT);

    assert_eq!(
        rows(&line_info(&object, "add")),
        [
            // `{` — the prologue, at the brace and not at the declaration on line 21.
            (0x00, 0x0a, Some(22), Some(1)),
            (0x0a, 0x12, Some(23), Some(11)), // return a + b;
            (0x12, 0x14, Some(24), Some(1)),  // `}` — the epilogue.
        ]
    );

    assert_eq!(
        rows(&line_info(&object, "twice")),
        [
            (0x14, 0x1f, Some(27), Some(1)),
            (0x1f, 0x2e, Some(28), Some(9)), // return add(n, n);
            (0x2e, 0x30, Some(29), Some(1)),
        ]
    );

    assert_eq!(
        rows(&line_info(&object, "sum_to")),
        [
            (0x30, 0x3b, Some(32), Some(1)),  // `{`
            (0x3b, 0x42, Some(33), Some(6)),  // int total = 0;
            (0x42, 0x49, Some(35), Some(11)), // for (int i = 1; ...
            (0x49, 0x4b, Some(35), Some(2)),  // ... the jump into the test.
            (0x4b, 0x5d, Some(36), Some(11)), // total = add(total, i);
            (0x5d, 0x61, Some(35), Some(27)), // i++
            (0x61, 0x69, Some(35), Some(20)), // i <= n
            (0x69, 0x6c, Some(38), Some(9)),  // return total;
            (0x6c, 0x6e, Some(39), Some(1)),  // `}`
        ]
    );
}

/// One source file, named the same way for every symbol: `DW_AT_comp_dir` joined with the
/// line program's file entry. A DWARF 5 file table has the primary source at index 0 *and*
/// again at index 1, which is where an off-by-one in file indices would show up as a
/// second, differently spelled entry.
#[test]
fn the_file_set_is_the_one_source_file() {
    for name in [FLAT, SPLIT] {
        let object = parse(name);
        for function in ["add", "twice", "sum_to"] {
            let info = line_info(&object, function);
            assert_eq!(
                info.files(),
                [Arc::from(SOURCE)],
                "{name}: {function}'s files"
            );
            for row in info.rows() {
                assert_eq!(info.file_of(row), Some(SOURCE), "{name}: {function}");
            }
        }
    }
}

/// The query API in the shape the assembly view uses it: one call per symbol, then every
/// instruction answered locally. Every instruction of a `-O0` function belongs to some
/// source line, so none of them may come back without one.
#[test]
fn every_instruction_of_a_function_has_a_source_line() {
    let object = parse(FLAT);
    let add = symbol(&object, "add");
    let info = add.line_info(&object).expect("add has line info");

    let assembly = add.assembly(&object).expect("add disassembles");
    assert!(assembly.instructions.len() > 3);

    let lines: Vec<_> = assembly
        .instructions
        .iter()
        .map(|instruction| {
            info.location(instruction.address)
                .unwrap_or_else(|| panic!("no position for {:#x}", instruction.address))
                .line
        })
        .collect();

    // Ascending, since `add` has no loop: the prologue's line, then the body's, then the
    // epilogue's, and nothing else.
    assert_eq!(lines.first(), Some(&Some(22)));
    assert_eq!(lines.last(), Some(&Some(24)));
    assert!(lines.iter().all(|line| matches!(line, Some(22..=24))));

    // Past the end of the symbol there is nothing, even though `twice` starts there.
    assert!(info.row_at(0x14).is_none());
}

// ---------------------------------------------------------------------------------------
// The invariants, against real DWARF
// ---------------------------------------------------------------------------------------

/// `LineInfo`'s invariant — rows ascending, non-overlapping, each answering for its own
/// addresses — asserted against a compiler's line program rather than a written one. It is
/// what `row_at`'s binary search needs, and it holds here despite the rows moving back and
/// forth through the source.
#[test]
fn rows_are_ascending_and_do_not_overlap() {
    for name in [FLAT, SPLIT] {
        let object = parse(name);
        for function in ["add", "twice", "sum_to"] {
            let symbol = symbol(&object, function);
            let end = symbol.address + symbol.estimate_size().expect("a size");
            let info = line_info(&object, function);

            let mut previous = symbol.address;
            for row in info.rows() {
                assert!(row.range.start < row.range.end, "{name}/{function}: empty row");
                assert!(
                    row.range.start >= previous,
                    "{name}/{function}: {:?} overlaps the row ending at {previous}",
                    row.range
                );
                assert!(row.range.end <= end, "{name}/{function}: {:?}", row.range);
                previous = row.range.end;

                for address in [row.range.start, row.range.end - 1] {
                    let found = info
                        .row_at(address)
                        .expect("every address inside a row is answered by it");
                    assert_eq!(found.range, row.range, "{name}/{function}");
                }
            }
            assert!(info.row_at(end).is_none(), "{name}/{function}: past the end");
        }
    }
}

/// The real-toolchain form of the section-conflation bug: with `-ffunction-sections` all
/// three functions live at address 0, and each must still report only its own rows.
///
/// This is also the case where the debug sections carry both kinds of absolute relocation
/// at once — `.text.*` ones, which are addresses and get biased, and `.debug_str` /
/// `.debug_line_str` ones, which are offsets into another section and must not be. If the
/// biasing touched the second kind the file names below would come back as garbage rather
/// than as a path.
#[test]
fn functions_sharing_address_zero_keep_their_own_rows() {
    let object = parse(SPLIT);

    for function in ["add", "twice", "sum_to"] {
        assert_eq!(symbol(&object, function).address, 0, "{function}");
    }

    // Rebased to each section's own zero, but otherwise exactly the flat build's rows.
    assert_eq!(
        rows(&line_info(&object, "add")),
        [
            (0x00, 0x0a, Some(22), Some(1)),
            (0x0a, 0x12, Some(23), Some(11)),
            (0x12, 0x14, Some(24), Some(1)),
        ]
    );
    assert_eq!(
        rows(&line_info(&object, "twice")),
        [
            (0x00, 0x0b, Some(27), Some(1)),
            (0x0b, 0x1a, Some(28), Some(9)),
            (0x1a, 0x1c, Some(29), Some(1)),
        ]
    );
    assert_eq!(
        rows(&line_info(&object, "sum_to")),
        [
            (0x00, 0x0b, Some(32), Some(1)),
            (0x0b, 0x12, Some(33), Some(6)),
            (0x12, 0x19, Some(35), Some(11)),
            (0x19, 0x1b, Some(35), Some(2)),
            (0x1b, 0x2d, Some(36), Some(11)),
            (0x2d, 0x31, Some(35), Some(27)),
            (0x31, 0x39, Some(35), Some(20)),
            (0x39, 0x3c, Some(38), Some(9)),
            (0x3c, 0x3e, Some(39), Some(1)),
        ]
    );
}

/// `sum_to`'s `while` loop, as gcc lays it out: the loop is entered by jumping *over* the
/// body to the condition at the bottom, which then branches back up into it. Both fixtures
/// must agree — the edges are instruction indices, so the same function at address 0x30 in
/// one build and at 0 in the other has exactly the same ones — and the `call add` between
/// them must contribute nothing, its displacement being a relocation placeholder.
///
/// Branch edges are not line info, which is what the rest of this file is about, but the
/// reason for reading a real compiler's output is the same in both cases: a fixture written
/// by hand has whatever shape it was written to have, and a `while` loop compiled by gcc
/// has the shape a reader will actually meet in the gutter.
#[test]
fn a_real_loop_is_two_edges_and_the_call_between_them_is_none() {
    for name in [FLAT, SPLIT] {
        let object = parse(name);
        let symbol = symbol(&object, "sum_to");
        let assembly = symbol.assembly(&object).expect("sum_to disassembles");

        let edges: Vec<_> = assembly
            .edges
            .iter()
            .map(|edge| (edge.from, edge.to))
            .collect();
        assert_eq!(edges, [(6, 14), (16, 7)], "{name}");

        let mnemonic = |index: usize| assembly.instructions[index].format[0].0.trim().to_owned();
        assert_eq!(mnemonic(6), "jmp", "{name}");
        assert_eq!(mnemonic(16), "jle", "{name}");
        assert!(!assembly.edges[0].is_backward(), "{name}");
        assert!(assembly.edges[1].is_backward(), "{name}");

        // The one branch out of the function, which is also the one relocated instruction.
        assert_eq!(mnemonic(11), "call", "{name}");
        assert!(assembly.instructions[11].relocation.is_some(), "{name}");
    }
}

/// The committed object is never rebuilt, so the line numbers above can only go wrong one
/// way: somebody edits the `.c` and does not rebuild the `.o`. Reading the source back and
/// checking that each asserted line still holds the statement it is asserted for turns that
/// into a failure here rather than a puzzle later.
#[test]
fn the_source_still_says_what_the_object_says_it_does() {
    let source = String::from_utf8(fixture("line_fixture.c")).expect("the fixture source is UTF-8");
    let lines: Vec<&str> = source.lines().collect();
    let at = |line: usize| lines[line - 1].trim();

    assert_eq!(at(21), "int add(int a, int b)");
    assert_eq!(at(22), "{");
    assert_eq!(at(23), "return a + b;");
    assert_eq!(at(24), "}");

    assert_eq!(at(27), "{");
    assert_eq!(at(28), "return add(n, n);");

    assert_eq!(at(32), "{");
    assert_eq!(at(33), "int total = 0;");
    assert_eq!(at(35), "for (int i = 1; i <= n; i++)");
    assert_eq!(at(36), "total = add(total, i);");
    assert_eq!(at(38), "return total;");
}

//! The crate pinned against DWARF a real toolchain emits, where every other fixture in the
//! suite is written in memory by `gimli::write`: DWARF 5 rather than 4, strings in
//! `.debug_line_str` rather than inline, a `.debug_rnglists` range list, `DW_AT_decl_line`
//! disagreeing with the line program, and rows that walk *back* through the source as a
//! loop is laid out. None of that is something a fixture builder would think to synthesize.
//!
//! Both objects are committed, built from `tests/fixtures/line_fixture.c` by the command in
//! that file's own header. The `-ffunction-sections` one exists because gcc's default puts
//! all three functions in one `.text`, where a section bias is zero and the identity
//! question never arises; splitting them puts **all three at address 0**, which is the
//! shape rustc emits and the shape that made line info conflate whole objects
//! (`notes/Bugs.md`). Its debug sections also carry relocations against `.debug_str` and
//! `.debug_line_str` that are *offsets*, not addresses, and so must survive the biasing.
//!
//! Hardcoded line numbers are safe because the objects are never rebuilt;
//! `the_source_still_says_what_the_object_says_it_does` keeps them honest anyway.

mod common;

use analysis::{parse_object, LineInfo, Object};
use common::{committed_fixture, symbol};
use object::{Object as _, ObjectKind, ObjectSection, SectionKind};
use std::path::PathBuf;
use std::sync::Arc;

/// The one `.text` build: `add` at 0, `twice` at 0x14, `sum_to` at 0x30.
const FLAT: &str = "line_fixture.o";
/// The `-ffunction-sections` build: three `.text.<name>` sections, every one at 0.
const SPLIT: &str = "line_fixture_split.o";

/// `DW_AT_comp_dir` + `DW_AT_name`, joined the way `addr2line` joins them.
const SOURCE: &str = "/fixture/line_fixture.c";

fn parse(name: &str) -> Arc<Object> {
    let bytes = committed_fixture(name);
    parse_object(
        bytes.as_slice().into(),
        name.to_string(),
        PathBuf::from(name),
    )
    .unwrap_or_else(|| panic!("{name} parses"))
}

/// Every row as `(start, end, line, column)`, the form the expectations are written in.
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

/// The fixtures are relocatable `.o`s, not linked images: a linked binary would have real
/// section addresses and would never exercise the relocation or biasing paths.
#[test]
fn the_fixtures_are_relocatable_objects() {
    for name in [FLAT, SPLIT] {
        let bytes = committed_fixture(name);
        let file = object::File::parse(&*bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(file.kind(), ObjectKind::Relocatable, "{name}");
        assert!(file.is_little_endian(), "{name}");

        // `.debug_line_str` is the section DWARF 5 introduced for the strings
        // `DW_FORM_line_strp` points at; no synthesized fixture has one.
        for section in [".debug_info", ".debug_line", ".debug_line_str"] {
            assert!(
                file.section_by_name(section).is_some(),
                "{name} has no {section}"
            );
        }
    }
}

/// Both shapes are committed because only the split one can show that an address alone
/// does not identify code in a relocatable object.
#[test]
fn one_text_or_several_is_the_difference_between_the_two_fixtures() {
    let text = |name: &str| -> Vec<(String, u64, u64)> {
        let bytes = committed_fixture(name);
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

/// A real compiler declares `st_size`, where the writer-built fixtures all declare 0 — so
/// this is the one place the estimate can be checked against what the file itself states.
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

/// The address→line mapping in full. Two things a real toolchain does that the synthesized
/// fixtures do not: a function's first row names its **opening brace** rather than its
/// declaration (`add` is declared on 21, its prologue is 22, and `DW_AT_decl_line` says 21
/// where the line program — which is what this crate reads — says 22); and `sum_to`'s rows
/// walk backwards through the source and forwards again, gcc laying the loop's increment
/// and test out after its body. Rows ascend by address, never by line.
#[test]
fn every_row_of_every_function_is_where_the_compiler_put_it() {
    let object = parse(FLAT);

    assert_eq!(
        rows(&line_info(&object, "add")),
        [
            (0x00, 0x0a, Some(22), Some(1)), // `{` — the prologue, not the declaration.
            (0x0a, 0x12, Some(23), Some(11)), // return a + b;
            (0x12, 0x14, Some(24), Some(1)), // `}` — the epilogue.
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

/// A DWARF 5 file table has the primary source at index 0 *and* again at index 1, which is
/// where an off-by-one in file indices shows up as a second, differently spelled entry.
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
/// instruction answered locally. Every instruction of a `-O0` function has a source line.
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

    assert_eq!(lines.first(), Some(&Some(22)));
    assert_eq!(lines.last(), Some(&Some(24)));
    assert!(lines.iter().all(|line| matches!(line, Some(22..=24))));

    // Past the end of the symbol there is nothing, even though `twice` starts there.
    assert!(info.row_at(0x14).is_none());
}

/// `LineInfo`'s invariant — rows ascending, non-overlapping, each answering for its own
/// addresses — asserted against a compiler's line program, whose rows move back and forth
/// through the source. It is what `row_at`'s binary search needs.
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
                assert!(
                    row.range.start < row.range.end,
                    "{name}/{function}: empty row"
                );
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
            assert!(
                info.row_at(end).is_none(),
                "{name}/{function}: past the end"
            );
        }
    }
}

/// The real-toolchain form of the section-conflation bug: with `-ffunction-sections` all
/// three functions live at address 0 and each must still report only its own rows. It is
/// also where the debug sections carry both kinds of absolute relocation at once — `.text.*`
/// addresses, which get biased, and `.debug_str` offsets, which must not be, or the file
/// names below would come back as garbage rather than as a path.
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

/// A loop as gcc actually lays one out — entered by jumping *over* the body to the
/// condition at the bottom, which branches back up into it — rather than as a fixture
/// written by hand has it. Both builds must agree, the edges being instruction indices, and
/// the `call add` between them must contribute nothing, its displacement being a
/// relocation placeholder.
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

/// The reverse mapping over DWARF a compiler actually emitted. `SPLIT` is the case that
/// matters: all three functions begin at address 0, so which symbol a line belongs to is
/// answered by the section bias and by nothing else — and the reverse direction is where an
/// address is the answer rather than the question.
#[test]
fn a_source_line_names_the_function_it_was_compiled_into() {
    for name in [FLAT, SPLIT] {
        let object = parse(name);
        let found = |line: u32| {
            object
                .symbols_at_line(SOURCE, line)
                .iter()
                .map(|symbol| symbol.name.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(found(23), ["add"], "{name}"); // return a + b;
        assert_eq!(found(28), ["twice"], "{name}"); // return add(n, n);
                                                    // Three rows of `sum_to`'s loop are line 35, and three rows are one answer.
        assert_eq!(found(35), ["sum_to"], "{name}");

        // A line of the file that produced no code: the licence header, a blank line, and
        // one past the end of the file.
        for line in [1, 20, 25, 1000] {
            assert!(found(line).is_empty(), "{name}: line {line}");
        }
        // The file under any other spelling is a file this object does not name.
        assert!(
            object.symbols_at_line("line_fixture.c", 23).is_empty(),
            "{name}"
        );
    }
}

/// The invariant Step 4 walks — index → symbols → `line_info` → ranges — over the only DWARF
/// in the suite nobody here wrote: every line the forward direction gives a function answers
/// with that function again.
#[test]
fn every_line_a_function_names_finds_that_function_again() {
    for name in [FLAT, SPLIT] {
        let object = parse(name);
        for function in ["add", "twice", "sum_to"] {
            let symbol = symbol(&object, function);
            let info = line_info(&object, function);
            for row in info.rows() {
                let (Some(file), Some(line)) = (info.file_of(row), row.line) else {
                    continue;
                };
                let back = object.symbols_at_line(file, line);
                assert!(
                    back.iter().any(|found| Arc::ptr_eq(found, &symbol)),
                    "{name}: {file}:{line} is in {function} but answers with {:?}",
                    back.iter().map(|s| &s.name).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// The line numbers above can only go wrong one way: somebody edits the `.c` and does not
/// rebuild the `.o`. Reading the source back turns that into a failure here rather than a
/// puzzle later.
#[test]
fn the_source_still_says_what_the_object_says_it_does() {
    let source = String::from_utf8(committed_fixture("line_fixture.c"))
        .expect("the fixture source is UTF-8");
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

/// The `-ffunction-sections` object's three code sections all start at 0, and the parse
/// lays them out the way a linker would — each after the last, rounded up — so a listing of
/// the object's code has an address for every byte and the line info reads the same layout.
#[test]
fn split_sections_are_each_given_a_place_of_their_own() {
    let object = parse(SPLIT);
    let mut code: Vec<&Arc<analysis::Section>> = object
        .sections
        .iter()
        .filter(|section| section.code)
        .collect();
    code.sort_by_key(|section| section.bias);

    // gcc leaves the ordinary `.text` in too, empty; a zero-length section still takes an
    // address of its own, so that two of them are two places.
    assert_eq!(
        code.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        [".text", ".text.add", ".text.twice", ".text.sum_to"]
    );
    assert!(code[0].data.is_empty());
    let mut placed_end = 0;
    for section in &code {
        assert_eq!(section.address, 0);
        assert!(
            section.bias >= placed_end,
            "{} overlaps the section before it",
            section.name
        );
        assert_eq!(section.bias % 16, 0);
        placed_end = section.bias + section.data.len() as u64;
    }
    assert_eq!(code[0].bias, 0);
    assert_eq!(code[1].bias, 0x10, "an empty section still takes one grain");
    assert_eq!(code[2].bias, 0x30, "add is 0x14 bytes, rounded up to 16");

    // And a section that is not code is not moved, whatever its address.
    for section in &object.sections {
        if !section.code {
            assert_eq!(section.bias, 0, "{}", section.name);
        }
    }

    // The one `.text` build has nothing to move.
    let flat = parse(FLAT);
    assert!(flat.sections.iter().all(|section| section.bias == 0));
}

/// A relocatable object's `.eh_frame` is not read: gcc writes the FDEs before the addresses
/// are known, so their address fields are zero with a relocation each, and read as they lie
/// they decode to ranges that fall inside `.text` — `0x20..0x34` for `add` at 0 — which
/// would give `sum_to` at 0x30 an extent of 4 instead of its 62. The unwind reader takes a
/// linked image only; this pins the gate.
#[test]
fn a_relocatable_objects_eh_frame_is_not_read() {
    let bytes = committed_fixture(FLAT);
    let file = object::File::parse(bytes.as_slice()).unwrap();
    let eh_frame = file.section_by_name(".eh_frame").expect("gcc writes one");
    assert_eq!(eh_frame.relocations().count(), 3, "one per FDE");

    let object = parse(FLAT);
    let sum_to = symbol(&object, "sum_to");
    assert!(sum_to.section.as_ref().unwrap().unwind.is_empty());
    assert_eq!(sum_to.extent(&object), Some(62));
}

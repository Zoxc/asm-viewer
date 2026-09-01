//! Line info read back out of DWARF written by `gimli::write`, so the round trip is
//! exercised without a compiler in the loop. A fixture built from a real compiler's
//! output is Step 4c.

mod common;

use analysis::{parse_object, Object};
use common::{elf_x86_64_with_dwarf, DwarfFixture, DwarfRow, DwarfSection, TextSymbol};
use std::path::PathBuf;
use std::sync::Arc;

const COMP_DIR: &str = "/src";

/// Two symbols, `first` at 0 and `second` at 6, with a line program that gives `first`
/// two source lines from `main.c` and `second` one from `other.c`.
fn two_files(base_symbol: Option<usize>) -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: COMP_DIR,
        files: &["main.c", "other.c"],
        sections: &[DwarfSection {
            name: None,
            symbols: &[
                TextSymbol {
                    name: "first",
                    // nop; nop; nop; nop; nop; ret
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
                    // The "left edge": DWARF's way of saying it has no column.
                    column: 0,
                },
                DwarfRow {
                    address: 6,
                    file: 1,
                    line: 42,
                    column: 7,
                },
                DwarfRow {
                    // Line 0: instructions belonging to no source line at all.
                    address: 7,
                    file: 1,
                    line: 0,
                    column: 0,
                },
            ],
            length: 8,
            subprograms: &[],
            base_symbol,
        }],
    })
}

fn parse(data: &[u8]) -> Arc<Object> {
    parse_object(data.into(), "line.o".into(), PathBuf::from("/line.o"))
        .expect("the fixture parses")
}

/// The one code section of a single-`.text` fixture. A range is only a question when it
/// is asked of a section — see `Object::line_info`.
fn text_section(object: &Object) -> &analysis::Section {
    object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect("the fixture has a .text")
}

fn symbol(object: &Object, name: &str) -> Arc<analysis::SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("{name} parses"))
        .clone()
}

#[test]
fn a_symbols_instructions_map_to_source_positions() {
    let data = two_files(None);
    let object = parse(&data);
    let first = symbol(&object, "first");

    let info = first.line_info(&object).expect("first has line info");

    // Two rows, coalesced from the line program's runs and clipped to the symbol.
    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0..3);
    assert_eq!(info.rows()[1].range, 3..6);

    // The whole symbol is answered by one call; every instruction then resolves locally.
    let assembly = first.assembly(&object).expect("first disassembles");
    let lines: Vec<_> = assembly
        .instructions
        .iter()
        .map(|instruction| {
            let location = info
                .location(instruction.address)
                .expect("every instruction of first has a position");
            (location.file, location.line, location.column)
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            (Some("/src/main.c"), Some(10), Some(3)),
            (Some("/src/main.c"), Some(10), Some(3)),
            (Some("/src/main.c"), Some(10), Some(3)),
            (Some("/src/main.c"), Some(11), None),
            (Some("/src/main.c"), Some(11), None),
            (Some("/src/main.c"), Some(11), None),
        ]
    );
}

#[test]
fn a_symbol_touches_only_its_own_files() {
    let data = two_files(None);
    let object = parse(&data);

    let first = symbol(&object, "first").line_info(&object).expect("first");
    assert_eq!(first.files(), [Arc::from("/src/main.c")]);

    let second = symbol(&object, "second")
        .line_info(&object)
        .expect("second");
    assert_eq!(second.files(), [Arc::from("/src/other.c")]);
}

#[test]
fn line_and_column_are_absent_rather_than_zero() {
    let data = two_files(None);
    let object = parse(&data);
    let info = symbol(&object, "second")
        .line_info(&object)
        .expect("second has line info");

    // Address 6 has a line and a column; address 7 is DWARF line 0, which is not line 0
    // and not line 1 but "no line", and carries no column either.
    let with = info.row_at(6).expect("a row at 6");
    assert_eq!((with.line, with.column), (Some(42), Some(7)));

    let without = info.row_at(7).expect("a row at 7");
    assert_eq!((without.line, without.column), (None, None));
    assert_eq!(info.file_of(without), Some("/src/other.c"));
}

/// Rows are clipped to the range asked about, so nothing a symbol's line info reports
/// falls outside that symbol.
#[test]
fn rows_do_not_leak_past_the_symbol() {
    let data = two_files(None);
    let object = parse(&data);

    for name in ["first", "second"] {
        let symbol = symbol(&object, name);
        let end = symbol.address + symbol.estimate_size().expect("a size");
        let info = symbol.line_info(&object).expect("line info");
        for row in info.rows() {
            assert!(
                row.range.start >= symbol.address && row.range.end <= end,
                "{name}: row {:?} outside {:?}",
                row.range,
                symbol.address..end
            );
        }
        assert!(info.row_at(end).is_none());
    }
}

/// In a relocatable object the addresses in `.debug_line` are not in the file: the
/// `DW_LNE_set_address` operand is zero with a relocation against a symbol. Read without
/// applying it, every function in the object maps to address 0.
///
/// The fixture bases its one sequence on `second`, which sits at 6, so a correctly
/// relocated read finds the sequence at 6..14 and `first` — still at 0 — has no line
/// info at all. Reading the operand literally would put the sequence at 0 instead, which
/// is exactly the failure this asserts against.
#[test]
fn debug_line_relocations_are_applied() {
    let data = two_files(Some(1));
    let object = parse(&data);

    let second = symbol(&object, "second");
    assert_eq!(second.address, 6);
    let info = second.line_info(&object).expect("second has line info");
    // The sequence starts where `second` does, not at the literal 0 in the file.
    assert_eq!(info.rows()[0].range.start, second.address);
    assert_eq!(info.rows()[0].line, Some(10));

    // 0..6 is now before the sequence, so `first` genuinely has nothing.
    assert!(symbol(&object, "first").line_info(&object).is_none());
}

/// The shape rustc emits: one `.text.<name>` section per function, **both at address 0**,
/// each with its own sequence in the unit's line program relocated against its own
/// symbol. Two functions therefore share an address, and an address alone cannot say
/// which of them a row belongs to — the section is part of the identity.
///
/// Read without scoping the query to a section, both sequences resolve to 0 and pile up
/// on top of each other: `first` reports `second`'s rows and files as well as its own.
/// That is `notes/Bugs.md`'s "Line info conflates every function in a relocatable
/// object", reduced to two functions.
fn two_sections() -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: COMP_DIR,
        files: &["main.c", "other.c"],
        sections: &[
            DwarfSection {
                name: Some(".text.first"),
                symbols: &[TextSymbol {
                    name: "first",
                    // nop; nop; nop; nop; nop; ret
                    bytes: &[0x90, 0x90, 0x90, 0x90, 0x90, 0xC3],
                }],
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
                ],
                length: 6,
                subprograms: &[],
                base_symbol: Some(0),
            },
            DwarfSection {
                name: Some(".text.second"),
                symbols: &[TextSymbol {
                    name: "second",
                    bytes: &[0x90, 0xC3],
                }],
                rows: &[DwarfRow {
                    address: 0,
                    file: 1,
                    line: 42,
                    column: 7,
                }],
                length: 2,
                subprograms: &[],
                base_symbol: Some(0),
            },
        ],
    })
}

/// Two functions in two sections, both at address 0: each one's line info must be its
/// own. Before line-info queries were scoped to a section this reported both sequences
/// for both symbols.
#[test]
fn a_symbol_does_not_pick_up_another_sections_rows() {
    let data = two_sections();
    let object = parse(&data);

    let first = symbol(&object, "first");
    let second = symbol(&object, "second");
    // The premise: in a relocatable object the address is not a key, and these two
    // genuinely share one.
    assert_eq!((first.address, second.address), (0, 0));
    assert_eq!(first.estimate_size(), Some(6));
    assert_eq!(second.estimate_size(), Some(2));

    let info = first.line_info(&object).expect("first has line info");
    assert_eq!(info.files(), [Arc::from("/src/main.c")]);
    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0..3);
    assert_eq!(info.rows()[1].range, 3..6);
    assert_eq!(
        info.location(5),
        Some(analysis::Location {
            file: Some("/src/main.c"),
            line: Some(11),
            column: None,
        })
    );

    let info = second.line_info(&object).expect("second has line info");
    assert_eq!(info.files(), [Arc::from("/src/other.c")]);
    assert_eq!(info.rows().len(), 1);
    assert_eq!(info.rows()[0].range, 0..2);
    assert_eq!(
        info.location(1),
        Some(analysis::Location {
            file: Some("/src/other.c"),
            line: Some(42),
            column: Some(7),
        })
    );
}

/// The rows of any one answer are ascending and do not overlap, whatever the input:
/// `row_at`'s binary search for the last row starting at or before an address is only
/// well defined when no row is nested inside another.
#[test]
fn rows_are_ascending_and_do_not_overlap() {
    let data = two_sections();
    let object = parse(&data);

    for name in ["first", "second"] {
        let info = symbol(&object, name).line_info(&object).expect("line info");
        let mut previous = 0;
        for row in info.rows() {
            assert!(row.range.start < row.range.end, "{name}: empty row");
            assert!(
                row.range.start >= previous,
                "{name}: {:?} overlaps the row ending at {previous}",
                row.range
            );
            previous = row.range.end;
            // Every address inside a row is answered by that row, which is what
            // overlapping rows would break.
            for address in [row.range.start, row.range.end - 1] {
                let found = info
                    .row_at(address)
                    .expect("a row covering its own address");
                assert_eq!(found.range, row.range);
            }
        }
    }
}

/// Both "no debug info" and "debug info that says nothing about this symbol" have to
/// come back as `None`, so a caller has one thing to check.
#[test]
fn an_object_without_dwarf_has_no_line_info() {
    let data = common::caller_and_target();
    let object = parse(&data);

    assert!(!object.symbols_sorted.is_empty());
    for symbol in &object.symbols_sorted {
        assert!(symbol.line_info(&object).is_none());
    }
    for section in &object.sections {
        assert!(object.line_info(section, 0..u64::MAX).is_none());
    }
}

#[test]
fn a_range_no_unit_covers_has_no_line_info() {
    let data = two_files(None);
    let object = parse(&data);
    let text = text_section(&object);

    assert!(object.line_info(text, 0x1000..0x2000).is_none());
    // An empty range asks about nothing and gets nothing, rather than everything.
    assert!(object.line_info(text, 0..0).is_none());
}

/// `Object` is shared as an `Arc` across threads and the DWARF context is built behind a
/// `OnceLock`, so several threads racing to be the first to ask must all get the same
/// answer and none of them may deadlock.
#[test]
fn line_info_is_usable_from_several_threads_at_once() {
    let data = two_files(None);
    let object = parse(&data);

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let object = object.clone();
            std::thread::spawn(move || {
                let info = symbol(&object, "first")
                    .line_info(&object)
                    .expect("line info");
                (info.rows().len(), info.files().to_vec())
            })
        })
        .collect();

    for thread in threads {
        let (rows, files) = thread.join().expect("no panic on a worker thread");
        assert_eq!(rows, 2);
        assert_eq!(files, [Arc::from("/src/main.c")]);
    }
}

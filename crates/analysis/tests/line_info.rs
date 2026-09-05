//! Line info read back out of DWARF written by `gimli::write`.

mod common;

use common::{
    elf_x86_64_with_dwarf, parse, symbol, DwarfFixture, DwarfRow, DwarfSection, TextSymbol,
    UnitRanges,
};
use std::sync::Arc;

const COMP_DIR: &str = "/src";

/// `first` at 0 with two lines from `main.c`, `second` at 6 with one from `other.c`.
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
                    // DWARF column 0 is the "left edge": no column.
                    column: 0,
                },
                DwarfRow {
                    address: 6,
                    file: 1,
                    line: 42,
                    column: 7,
                },
                DwarfRow {
                    // DWARF line 0: instructions belonging to no source line at all.
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
        unit_ranges: UnitRanges::Relocated,
    })
}

#[test]
fn a_symbols_instructions_map_to_source_positions() {
    let data = two_files(None);
    let object = parse(&data);
    let first = symbol(&object, "first");

    let info = first.line_info(&object).expect("first has line info");

    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0..3);
    assert_eq!(info.rows()[1].range, 3..6);

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

    let with = info.row_at(6).expect("a row at 6");
    assert_eq!((with.line, with.column), (Some(42), Some(7)));

    // Address 7 is DWARF line 0, which is "no line" rather than line 0 or line 1.
    let without = info.row_at(7).expect("a row at 7");
    assert_eq!((without.line, without.column), (None, None));
    assert_eq!(info.file_of(without), Some("/src/other.c"));
}

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

/// In a relocatable object the `DW_LNE_set_address` operand is zero with a relocation
/// against a symbol, so read literally every function maps to address 0. The fixture
/// bases its one sequence on `second` at 6: relocated it covers 6..14 and `first` has no
/// line info at all, where reading the operand literally would put it at 0.
#[test]
fn debug_line_relocations_are_applied() {
    let data = two_files(Some(1));
    let object = parse(&data);

    let second = symbol(&object, "second");
    assert_eq!(second.address, 6);
    let info = second.line_info(&object).expect("second has line info");
    assert_eq!(info.rows()[0].range.start, second.address);
    assert_eq!(info.rows()[0].line, Some(10));

    assert!(symbol(&object, "first").line_info(&object).is_none());
}

/// The shape rustc emits: one `.text.<name>` per function, both at address 0, each with
/// its own sequence relocated against its own symbol — so an address alone cannot say
/// which function a row belongs to.
fn two_sections(unit_ranges: UnitRanges) -> Vec<u8> {
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
        unit_ranges,
    })
}

/// Before line-info queries were scoped to a section this reported both sequences for
/// both symbols.
#[test]
fn a_symbol_does_not_pick_up_another_sections_rows() {
    let data = two_sections(UnitRanges::Relocated);
    let object = parse(&data);

    let first = symbol(&object, "first");
    let second = symbol(&object, "second");
    // The premise: the two genuinely share an address.
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

/// `row_at`'s binary search for the last row starting at or before an address is only
/// well defined when no row is nested inside another.
#[test]
fn rows_are_ascending_and_do_not_overlap() {
    let data = two_sections(UnitRanges::Relocated);
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
            for address in [row.range.start, row.range.end - 1] {
                let found = info
                    .row_at(address)
                    .expect("a row covering its own address");
                assert_eq!(found.range, row.range);
            }
        }
    }
}

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
    let text = object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect("the fixture has a .text");

    assert!(object.line_info(text, 0x1000..0x2000).is_none());
    // An empty range asks about nothing and gets nothing, rather than everything.
    assert!(object.line_info(text, 0..0).is_none());
}

/// The DWARF context is built behind a `OnceLock`, so threads racing to be the first to
/// ask must all get the same answer and none may deadlock.
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

/// The same two sections, with the unit's ranges written as offsets from a `DW_AT_low_pc`
/// that carries no relocation: the line program's addresses move to their section's place
/// and the declared range does not, so the unit stops covering its own code. Before the
/// stale ranges were dropped this was a silent "no line info" for every section but the one
/// left at 0.
#[test]
fn a_unit_whose_ranges_did_not_move_with_its_code_still_answers() {
    let data = two_sections(UnitRanges::OffsetPairs);
    let object = parse(&data);

    let first = symbol(&object, "first");
    let second = symbol(&object, "second");

    let info = first.line_info(&object).expect("first has line info");
    assert_eq!(info.files(), [Arc::from("/src/main.c")]);
    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0..3);
    assert_eq!(info.rows()[1].range, 3..6);

    // The one the unit's stale range does not reach.
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

/// A linked image holds the addresses its linker resolved, and one linked with
/// `--emit-relocs` keeps the relocations that resolved them; `object` hands those over
/// whatever the file kind. Applying one again added the symbol's address a second time
/// wherever the addend sits in the bytes rather than in the relocation (ELF `REL`, so i386
/// and ARM32), and the row landed past the function it describes: a silent "no line info"
/// for every one of its instructions.
#[test]
fn a_linked_images_retained_relocations_are_not_applied_again() {
    let data = common::elf_i386_linked_with_relocations();
    let object = parse(&data);

    let only = symbol(&object, "only");
    let info = only.line_info(&object).expect("only has line info");

    assert_eq!(info.files(), [Arc::from("/src/main.c")]);
    assert_eq!(info.rows().len(), 1);
    assert_eq!(info.rows()[0].range, 0x100..0x110);
    assert_eq!(
        info.location(0x100),
        Some(analysis::Location {
            file: Some("/src/main.c"),
            line: Some(7),
            column: None,
        })
    );
}

/// A relocatable object may state where a section goes — a Mach-O `.o` always does — and
/// state it above where the parse placed it. The bias was then the wrapped difference: the
/// rows landed where they belonged and every query saturated past them, so the section's
/// symbols had no line info and nothing about it looked wrong.
#[test]
fn a_section_stating_an_address_of_its_own_still_answers() {
    let mut data = two_sections(UnitRanges::Relocated);
    common::elf_place_section(&mut data, ".text.first", 0x1000);
    let object = parse(&data);

    // The premise: one section states an address above where the other's bytes put it.
    let first = symbol(&object, "first");
    let second = symbol(&object, "second");
    assert_eq!((first.address, second.address), (0x1000, 0));

    let info = first.line_info(&object).expect("first has line info");
    assert_eq!(info.files(), [Arc::from("/src/main.c")]);
    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0x1000..0x1003);
    assert_eq!(info.rows()[1].range, 0x1003..0x1006);

    let info = second.line_info(&object).expect("second has line info");
    assert_eq!(info.files(), [Arc::from("/src/other.c")]);
    assert_eq!(info.rows()[0].range, 0..2);
}

/// A text section whose bytes will not read is dropped by the parse, and the layout still
/// has to place it: read back off the sections that were kept it had no place, its rows
/// were relocated to 0, and the section sitting there answered its symbols with them.
#[test]
fn a_section_that_would_not_read_keeps_its_rows_off_another() {
    let mut data = two_sections(UnitRanges::Relocated);
    common::elf_unreadable_section(&mut data, ".text.second");
    let object = parse(&data);

    // The premise: the parse kept the one section and dropped the other.
    let sections: Vec<&str> = object
        .sections
        .iter()
        .filter(|section| section.code)
        .map(|section| section.name.as_str())
        .collect();
    assert_eq!(sections, [".text.first"]);

    let info = symbol(&object, "first")
        .line_info(&object)
        .expect("first has line info");
    assert_eq!(info.files(), [Arc::from("/src/main.c")]);
    assert_eq!(info.rows().len(), 2);
    assert_eq!(info.rows()[0].range, 0..3);
    assert_eq!(info.rows()[1].range, 3..6);
}

/// An `addr2line` 0.21 defect, pinned rather than worked around
/// (`notes/upstream/addr2line.md`): `LocationRangeUnitIter::new` maps every miss but a probe
/// below the unit's first sequence to "past the last one", so a query starting in the gap
/// between two sequences of a unit is answered with nothing — while the reverse index, which
/// walks the unit from 0, sees the very rows the query cannot. Delete this test with the
/// note, when the crate moves.
#[test]
fn a_query_starting_between_two_sequences_of_a_unit_is_answered_with_nothing() {
    let data = common::elf_x86_64_two_sequences();
    let object = parse(&data);

    // The premise: `middle` begins in the gap after the first sequence and runs to the end
    // of the second, whose one row is inside it and names it.
    let middle = symbol(&object, "middle");
    assert_eq!((middle.address, middle.extent(&object)), (6, Some(0x10)));
    let named: Vec<String> = object
        .symbols_at_line("/src/other.c", 42)
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(named, ["middle"]);

    // A query starting inside a sequence is answered, so the unit is reached and read.
    let info = symbol(&object, "before")
        .line_info(&object)
        .expect("before has line info");
    assert_eq!(info.files(), [Arc::from("/src/main.c")]);

    // And a query from the start of the section is answered with both sequences, so the
    // unit's range covers the gap and the second sequence is there to be found.
    let section = middle.section.clone().expect("middle is in a section");
    let whole = object
        .line_info(&section, 0..0x16)
        .expect("the section's own range has line info");
    assert_eq!(
        whole.files(),
        [Arc::from("/src/main.c"), Arc::from("/src/other.c")]
    );

    assert!(
        middle.line_info(&object).is_none(),
        "0.21 answers this; the note is out of date"
    );
}

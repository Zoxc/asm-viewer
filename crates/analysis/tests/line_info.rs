//! Line info read back out of DWARF written by `gimli::write`, so the round trip is
//! exercised without a compiler in the loop. A fixture built from a real compiler's
//! output is Step 4c.

mod common;

use analysis::{parse_object, Object};
use common::{elf_x86_64_with_dwarf, DwarfFixture, DwarfRow, TextSymbol};
use std::path::PathBuf;
use std::sync::Arc;

const COMP_DIR: &str = "/src";

/// Two symbols, `first` at 0 and `second` at 6, with a line program that gives `first`
/// two source lines from `main.c` and `second` one from `other.c`.
fn two_files(base_symbol: Option<usize>) -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
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
        comp_dir: COMP_DIR,
        files: &["main.c", "other.c"],
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
        base_symbol,
    })
}

fn parse(data: &[u8]) -> Arc<Object> {
    parse_object(data.into(), "line.o".into(), PathBuf::from("/line.o")).expect("the fixture parses")
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

    let second = symbol(&object, "second").line_info(&object).expect("second");
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
    assert!(object.line_info(0..u64::MAX).is_none());
}

#[test]
fn a_range_no_unit_covers_has_no_line_info() {
    let data = two_files(None);
    let object = parse(&data);

    assert!(object.line_info(0x1000..0x2000).is_none());
    // An empty range asks about nothing and gets nothing, rather than everything.
    assert!(object.line_info(0..0).is_none());
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

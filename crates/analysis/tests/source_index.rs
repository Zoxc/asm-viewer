//! The reverse mapping: a source file and a line, out to the symbols compiled from it.
//!
//! The forward direction's own tests are `line_info.rs`; these are about the index built on
//! top of it, so what they pin is what only the reverse direction can get wrong — one line
//! landing in *several* symbols, a line landing in none, and the section bias telling two
//! functions at address 0 apart in the direction where the address is the answer rather than
//! the question.

mod common;

use analysis::{Object, SymbolData};
use common::{elf_x86_64_with_dwarf, parse, DwarfFixture, DwarfRow, DwarfSection, TextSymbol};
use std::sync::Arc;

const COMP_DIR: &str = "/src";
const MAIN: &str = "/src/main.c";
const OTHER: &str = "/src/other.c";

/// The names a query answers with, which is what the expectations are written in.
fn at(object: &Object, file: &str, line: u32) -> Vec<String> {
    named(object.symbols_at_line(file, line))
}

fn named(symbols: Vec<Arc<SymbolData>>) -> Vec<String> {
    symbols.iter().map(|symbol| symbol.name.clone()).collect()
}

/// One `.text`: `first` at 0 (6 bytes) and `second` at 6 (2 bytes). **Both have a row naming
/// `main.c` line 10** — a header included twice, a generic instantiated twice, a line inlined
/// into its caller — which is the case the index exists for. Line 11 is `first`'s alone and
/// `other.c` line 42 is `second`'s.
fn shared_line() -> Vec<u8> {
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
                    column: 0,
                },
                DwarfRow {
                    address: 6,
                    file: 0,
                    line: 10,
                    column: 3,
                },
                DwarfRow {
                    address: 7,
                    file: 1,
                    line: 42,
                    column: 7,
                },
            ],
            length: 8,
            subprograms: &[],
            base_symbol: None,
        }],
    })
}

#[test]
fn one_line_compiles_into_every_symbol_that_holds_it() {
    let object = parse(&shared_line());

    // In address order, which is the order the listings they name are in.
    assert_eq!(at(&object, MAIN, 10), ["first", "second"]);
    assert_eq!(at(&object, MAIN, 11), ["first"]);
    assert_eq!(at(&object, OTHER, 42), ["second"]);
}

#[test]
fn a_range_answers_for_every_line_in_it_and_says_each_symbol_once() {
    let object = parse(&shared_line());

    // 10 gives both and 11 gives `first` again: `first` is one hit, not two.
    assert_eq!(
        named(object.symbols_from_source(MAIN, 10..12)),
        ["first", "second"]
    );
    assert_eq!(named(object.symbols_from_source(MAIN, 11..12)), ["first"]);
    // A range and the single line inside it are the same question.
    assert_eq!(
        named(object.symbols_from_source(MAIN, 10..11)),
        at(&object, MAIN, 10)
    );
    // An empty range asks about nothing rather than about everything.
    assert!(object.symbols_from_source(MAIN, 10..10).is_empty());
}

#[test]
fn nothing_is_invented_for_a_line_a_file_or_an_object_that_says_nothing() {
    let object = parse(&shared_line());

    // A line of the file no code came from.
    assert!(object.symbols_at_line(MAIN, 12).is_empty());
    // A file this object does not name — including the one it names, spelt differently.
    assert!(object.symbols_at_line("/src/absent.c", 10).is_empty());
    assert!(object.symbols_at_line("main.c", 10).is_empty());
    assert!(object.symbols_at_line("/SRC/MAIN.C", 10).is_empty());

    // An object with no DWARF at all: an empty answer rather than a panic or a guess.
    let without = parse(&common::caller_and_target());
    assert!(!without.symbols_sorted.is_empty());
    assert!(without.symbols_at_line(MAIN, 10).is_empty());
}

/// The shape rustc emits: one `.text.<name>` per function, both at address 0. Read without
/// [`section_biases`] the two functions occupy the same addresses, so *every* line would
/// answer with both.
fn two_sections() -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: COMP_DIR,
        files: &["main.c", "other.c"],
        sections: &[
            DwarfSection {
                name: Some(".text.first"),
                symbols: &[TextSymbol {
                    name: "first",
                    bytes: &[0x90, 0x90, 0x90, 0x90, 0x90, 0xC3],
                }],
                rows: &[DwarfRow {
                    address: 0,
                    file: 0,
                    line: 10,
                    column: 3,
                }],
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

#[test]
fn two_functions_at_address_zero_answer_for_their_own_lines_only() {
    let object = parse(&two_sections());

    // The premise: the two genuinely share an address, so nothing but the section tells
    // them apart.
    let addresses: Vec<u64> = object
        .symbols_sorted
        .iter()
        .map(|symbol| symbol.address)
        .collect();
    assert_eq!(addresses, [0, 0]);

    assert_eq!(at(&object, MAIN, 10), ["first"]);
    assert_eq!(at(&object, OTHER, 42), ["second"]);
}

/// The invariant everything built on this depends on: whatever the forward direction says a
/// symbol's lines are, the reverse direction hands that symbol back for each of them. Step 4
/// walks index → symbols → `line_info` → ranges, and a symbol with no ranges would be a row
/// in a results panel pointing nowhere.
fn round_trips(object: &Object) {
    let mut checked = 0;
    for symbol in &object.symbols_sorted {
        let Some(info) = symbol.line_info(object) else {
            continue;
        };
        for row in info.rows() {
            let (Some(file), Some(line)) = (info.file_of(row), row.line) else {
                continue;
            };
            let back = object.symbols_at_line(file, line);
            assert!(
                back.iter().any(|found| Arc::ptr_eq(found, symbol)),
                "{}:{line} is in {} but answers with {:?}",
                file,
                symbol.name,
                named(back)
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the fixture has no rows to round-trip");
}

#[test]
fn every_line_a_symbol_names_finds_that_symbol_again() {
    round_trips(&parse(&shared_line()));
    round_trips(&parse(&two_sections()));
    round_trips(&parse(&common::dwarf_fixture(&[(0, 6), (1, 2)])));
}

/// The index is built behind a `OnceLock` and takes the context's lock while it builds, and
/// the extents it needs take that same lock — so threads racing to be the first to ask must
/// all get the same answer and none may deadlock.
#[test]
fn the_index_is_usable_from_several_threads_at_once() {
    let object = parse(&shared_line());

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let object = object.clone();
            std::thread::spawn(move || at(&object, MAIN, 10))
        })
        .collect();

    for thread in threads {
        let found = thread.join().expect("no panic on a worker thread");
        assert_eq!(found, ["first", "second"]);
    }
}

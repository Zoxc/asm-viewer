//! A function's extent taken from the debug info that states it, rather than derived
//! from where the next symbol starts.
//!
//! The two answers differ by the alignment padding a linker leaves between functions,
//! which is what these fixtures put there on purpose: `first` is six bytes of code with
//! four bytes of `int3` after it, so the next-symbol estimate says ten and DWARF says
//! six.

mod common;

use analysis::{parse_object, Object, Section, SymbolData};
use common::{elf_x86_64_with_dwarf, DwarfFixture, DwarfRow, DwarfSection, TextSymbol};
use std::path::PathBuf;
use std::sync::Arc;

/// Five instructions and a `ret`, then four bytes of the padding a linker inserts to
/// align what follows. `first` is the whole ten bytes as far as the symbol table can
/// tell; DWARF knows it is the first six.
const FIRST: &[u8] = &[
    0x90, 0x90, 0x90, 0x90, 0x90, 0xC3, // the function
    0xCC, 0xCC, 0xCC, 0xCC, // padding
];
const SECOND: &[u8] = &[0x90, 0xC3];

fn fixture(subprograms: &[(usize, u64)], base_symbol: Option<usize>) -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: "/src",
        files: &["main.c"],
        sections: &[DwarfSection {
            name: None,
            symbols: &[
                TextSymbol {
                    name: "first",
                    bytes: FIRST,
                },
                TextSymbol {
                    name: "second",
                    bytes: SECOND,
                },
            ],
            rows: &[
                DwarfRow {
                    address: 0,
                    file: 0,
                    line: 10,
                    column: 0,
                },
                DwarfRow {
                    address: 10,
                    file: 0,
                    line: 20,
                    column: 0,
                },
            ],
            length: 12,
            subprograms,
            base_symbol,
        }],
    })
}

fn parse(data: &[u8]) -> Arc<Object> {
    parse_object(data.into(), "e.o".into(), PathBuf::from("/e.o")).expect("the fixture parses")
}

fn named<'a>(object: &'a Object, name: &str) -> &'a Arc<SymbolData> {
    object
        .symbols_sorted
        .iter()
        .find(|symbol| symbol.name == name)
        .expect("the fixture has this symbol")
}

fn text(object: &Object) -> &Section {
    object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect("the fixture has a .text")
}

#[test]
fn a_subprogram_extent_is_preferred_to_the_next_symbols_address() {
    let object = parse(&fixture(&[(0, 6), (1, 2)], Some(0)));
    let first = named(&object, "first");

    // What the symbol table alone can say: everything up to `second`, padding included.
    assert_eq!(first.estimate_size(), Some(10));
    // What DWARF says.
    assert_eq!(first.dwarf_extent(&object), Some(6));
    assert_eq!(first.extent(&object), Some(6));

    // And the disassembly stops at the `ret` rather than running into four `int3`s.
    let assembly = first.assembly(&object).expect("a listing");
    assert_eq!(assembly.instructions.len(), 6);
    assert_eq!(first.data_in(&object), Some(&FIRST[..6]));
    // `data()` is the answer without an object in hand, and is unchanged.
    assert_eq!(first.data(), Some(FIRST));
}

#[test]
fn a_symbol_no_subprogram_describes_keeps_the_estimate() {
    // Only `second` gets a subprogram DIE.
    let object = parse(&fixture(&[(1, 2)], Some(0)));
    let first = named(&object, "first");

    assert_eq!(first.dwarf_extent(&object), None);
    assert_eq!(first.extent(&object), first.estimate_size());
    assert_eq!(first.extent(&object), Some(10));
}

#[test]
fn an_object_with_no_debug_info_at_all_keeps_the_estimate() {
    let object = parse(&common::caller_and_target());
    let caller = named(&object, "caller");

    assert_eq!(caller.dwarf_extent(&object), None);
    assert_eq!(caller.extent(&object), Some(6));
}

#[test]
fn a_subprogram_reaching_past_the_next_symbol_is_clipped_to_it() {
    // `DW_AT_high_pc` describes the function, not the symbol asked about, so a symbol
    // sitting inside a subprogram — an alias, a label the assembler emitted — would
    // otherwise swallow everything after it. The smaller of the two answers wins.
    let object = parse(&fixture(&[(0, 12)], Some(0)));
    let first = named(&object, "first");

    assert_eq!(first.dwarf_extent(&object), Some(12));
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.extent(&object), Some(10));
}

#[test]
fn the_extent_is_the_range_line_info_is_asked_about() {
    let object = parse(&fixture(&[(0, 6), (1, 2)], Some(0)));
    let first = named(&object, "first");

    // Row two starts at 10, which is inside the estimate and outside the extent, so
    // asking over the extent is what keeps the padding's line out of the answer.
    let info = first.line_info(&object).expect("line info for `first`");
    assert_eq!(info.rows().len(), 1);
    assert_eq!(info.rows()[0].line, Some(10));
    assert_eq!(info.rows()[0].range, 0..6);
}

#[test]
fn a_linked_image_is_asked_in_its_own_addresses() {
    // `base_symbol: None` writes the line program and the subprogram DIEs at literal
    // addresses, the way a linked image holds them — so no section bias is in play and
    // the query must not add one.
    let object = parse(&fixture(&[(0, 6)], None));
    let first = named(&object, "first");

    assert_eq!(first.address, 0);
    assert_eq!(object.subprogram_extent(text(&object), 0), Some(6));
    assert_eq!(first.extent(&object), Some(6));
}

/// The rustc shape: one `.text.<name>` per function, **both at address 0**, each
/// subprogram relocated against its own section's symbol. An address alone cannot say
/// which function it means, so the query has to carry the section's bias in and the
/// answer has to be the one from that section.
fn two_sections() -> Vec<u8> {
    elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: "/src",
        files: &["main.c"],
        sections: &[
            DwarfSection {
                name: Some(".text.first"),
                symbols: &[TextSymbol {
                    name: "first",
                    bytes: FIRST,
                }],
                rows: &[DwarfRow {
                    address: 0,
                    file: 0,
                    line: 10,
                    column: 0,
                }],
                length: 10,
                subprograms: &[(0, 6)],
                base_symbol: Some(0),
            },
            DwarfSection {
                name: Some(".text.second"),
                symbols: &[TextSymbol {
                    name: "second",
                    bytes: SECOND,
                }],
                rows: &[DwarfRow {
                    address: 0,
                    file: 0,
                    line: 20,
                    column: 0,
                }],
                length: 2,
                subprograms: &[(0, 2)],
                base_symbol: Some(0),
            },
        ],
    })
}

#[test]
fn two_functions_at_address_zero_get_their_own_extents() {
    let object = parse(&two_sections());
    let first = named(&object, "first");
    let second = named(&object, "second");

    // The premise: the address is not the key here.
    assert_eq!(first.address, 0);
    assert_eq!(second.address, 0);

    assert_eq!(first.dwarf_extent(&object), Some(6));
    assert_eq!(second.dwarf_extent(&object), Some(2));

    // `first` is alone in its section, so the estimate runs to the section's end —
    // padding included — and DWARF is what trims it back to the function.
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.extent(&object), Some(6));
}

/// A derived extent past [`MAX_DERIVED_SIZE`] is the derivation saying nothing rather
/// than a function that long: an export table declares a handful of the functions in an
/// image, so the gap to the next declaration spans everything unexported in between.
#[test]
fn a_derivation_reaching_a_megabyte_is_cut_off() {
    // One symbol at the front of a section far larger than the cap.
    let mut text = vec![0x90u8; (2 << 20) + 16];
    *text.last_mut().unwrap() = 0xC3;
    let object = parse(&elf_x86_64_with_dwarf(DwarfFixture {
        comp_dir: "/src",
        files: &["main.c"],
        sections: &[DwarfSection {
            name: None,
            symbols: &[TextSymbol {
                name: "huge",
                bytes: &text,
            }],
            rows: &[DwarfRow {
                address: 0,
                file: 0,
                line: 1,
                column: 0,
            }],
            length: text.len() as u64,
            subprograms: &[],
            base_symbol: Some(0),
        }],
    }));

    let huge = named(&object, "huge");
    assert_eq!(huge.estimate_size(), Some(1 << 20));
    assert_eq!(huge.extent(&object), Some(1 << 20));
    assert_eq!(huge.data().map(<[u8]>::len), Some(1 << 20));
}

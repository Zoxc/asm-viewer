//! A function's extent taken from what the file states — its symbol table's `st_size`, or
//! the debug info — rather than derived from where the next symbol starts. A stated extent
//! and a derived one differ by the alignment padding a linker leaves between functions,
//! which is what these fixtures put there on purpose.

mod common;

use common::{
    coff_x86_64, elf_shared_object, elf_x86_64_with_dwarf, elf_x86_64_with_dwarf_declaring, named,
    parse, DwarfFixture, DwarfRow, DwarfSection, ExportedSymbol, SharedObject, TextSymbol,
    UnitRanges, TEXT_ADDRESS,
};

/// Six bytes of code then four bytes of padding: ten to the symbol table, six to DWARF.
const FIRST: &[u8] = &[
    0x90, 0x90, 0x90, 0x90, 0x90, 0xC3, // the function
    0xCC, 0xCC, 0xCC, 0xCC, // padding
];
const SECOND: &[u8] = &[0x90, 0xC3];

fn fixture(subprograms: &[(usize, u64)], base_symbol: Option<usize>) -> Vec<u8> {
    declaring(subprograms, base_symbol, &[])
}

/// [`fixture`] with an `st_size` per symbol, `first` first.
fn declaring(subprograms: &[(usize, u64)], base_symbol: Option<usize>, sizes: &[u64]) -> Vec<u8> {
    elf_x86_64_with_dwarf_declaring(
        DwarfFixture {
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
            unit_ranges: UnitRanges::Relocated,
        },
        sizes,
    )
}

#[test]
fn a_subprogram_extent_is_preferred_to_the_next_symbols_address() {
    let object = parse(&fixture(&[(0, 6), (1, 2)], Some(0)));
    let first = named(&object, "first");

    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.debug_extent(&object), Some(6));
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

    assert_eq!(first.debug_extent(&object), None);
    assert_eq!(first.extent(&object), first.estimate_size());
    assert_eq!(first.extent(&object), Some(10));
}

#[test]
fn an_object_with_no_debug_info_at_all_keeps_the_estimate() {
    let object = parse(&common::caller_and_target());
    let caller = named(&object, "caller");

    assert_eq!(caller.debug_extent(&object), None);
    assert_eq!(caller.extent(&object), Some(6));
}

#[test]
fn a_subprogram_reaching_past_the_next_symbol_is_clipped_to_it() {
    // `DW_AT_high_pc` describes the function, not the symbol asked about, so a symbol
    // sitting inside a subprogram — an alias, a label the assembler emitted — would
    // otherwise swallow everything after it. The smaller of the two answers wins.
    let object = parse(&fixture(&[(0, 12)], Some(0)));
    let first = named(&object, "first");

    assert_eq!(first.debug_extent(&object), Some(12));
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
    // `base_symbol: None` writes literal addresses, the way a linked image holds them, so
    // no section bias is in play and the query must not add one.
    let object = parse(&fixture(&[(0, 6)], None));
    let first = named(&object, "first");
    let text = object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect("the fixture has a .text");

    assert_eq!(first.address, 0);
    assert_eq!(object.function_extent(text, 0), Some(6));
    assert_eq!(first.extent(&object), Some(6));
}

/// The rustc shape: one `.text.<name>` per function, both at address 0, each subprogram
/// relocated against its own section's symbol — so the query has to carry the section's
/// bias in and answer from that section.
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
        unit_ranges: UnitRanges::Relocated,
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

    assert_eq!(first.debug_extent(&object), Some(6));
    assert_eq!(second.debug_extent(&object), Some(2));

    // `first` is alone in its section, so the estimate runs to the section's end —
    // padding included — and DWARF is what trims it back to the function.
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.extent(&object), Some(6));
}

/// A derived extent past `MAX_DERIVED_SIZE` is the derivation saying nothing rather than
/// a function that long: an export table declares a handful of an image's functions, so
/// the gap to the next declaration spans everything unexported in between.
#[test]
fn a_derivation_reaching_a_megabyte_is_cut_off() {
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
        unit_ranges: UnitRanges::Relocated,
    }));

    let huge = named(&object, "huge");
    assert_eq!(huge.estimate_size(), Some(1 << 20));
    assert_eq!(huge.extent(&object), Some(1 << 20));
    assert_eq!(huge.data().map(<[u8]>::len), Some(1 << 20));
}

/// The symbol table's own answer, taken before the debug info is opened. `first` declares
/// six bytes where DWARF says twelve and the next symbol is ten away: the declaration wins,
/// and the walk that would have said twelve is what it spares.
#[test]
fn a_declared_size_is_taken_before_the_debug_info() {
    let object = parse(&declaring(&[(0, 12)], Some(0), &[6, 2]));
    let first = named(&object, "first");

    assert_eq!(first.size, 6);
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.debug_extent(&object), Some(12));
    assert_eq!(first.extent(&object), Some(6));

    // And the disassembly stops at the `ret` rather than running into four `int3`s.
    let assembly = first.assembly(&object).expect("a listing");
    assert_eq!(assembly.instructions.len(), 6);
}

/// 0 is the size an object file most often declares, and it has to mean "nothing declared"
/// rather than "no bytes": `first` falls through to DWARF, `second` beside it does not.
#[test]
fn a_declared_zero_still_falls_through_to_the_debug_info() {
    let object = parse(&declaring(&[(0, 6), (1, 2)], Some(0), &[0, 2]));
    let first = named(&object, "first");

    assert_eq!(first.size, 0);
    assert_eq!(first.extent(&object), Some(6));
    assert_eq!(named(&object, "second").extent(&object), Some(2));
}

/// A declaration reaching past the next symbol is clipped to it, as an unwind entry's
/// stated end is: a listing decodes one stretch per symbol, and rows drawn twice are worse
/// than a stretch that stops early.
#[test]
fn a_declared_size_past_the_next_symbol_is_clipped_to_it() {
    let object = parse(&declaring(&[], Some(0), &[100]));
    let first = named(&object, "first");

    assert_eq!(first.size, 100);
    assert_eq!(first.extent(&object), Some(10));
}

/// The unwind table is still asked first. `first` declares eight bytes and its FDE covers
/// four; the table is the image's statement to its loader, so it is the one that stands.
#[test]
fn an_unwind_entry_outranks_a_declared_size() {
    let object = parse(&elf_shared_object(SharedObject {
        text: &[0x90, 0x90, 0x90, 0xC3, 0xCC, 0xCC, 0xCC, 0xC3],
        dynamic: &[],
        static_symbols: &[ExportedSymbol {
            name: "first",
            offset: 0,
            size: 8,
            code: true,
        }],
        entry: None,
        eh_frame: &[(0, 4)],
    }));
    let first = named(&object, "first");

    assert_eq!(first.address, TEXT_ADDRESS);
    assert_eq!(first.size, 8);
    assert_eq!(first.extent(&object), Some(4));
}

/// A COFF function symbol's size is the `TotalSize` of its auxiliary function-definition
/// record, which is written for COFF's line-number data and is not a measurement of the
/// code. Only ELF's `st_size` is trusted, so this one is read, displayed and ignored: the
/// extent is the next symbol's address, six bytes on.
#[test]
fn a_coff_total_size_is_not_a_functions_length() {
    let object = parse(&coff_x86_64(&[
        (
            TextSymbol {
                name: "first",
                bytes: &[0x90, 0x90, 0x90, 0x90, 0x90, 0xC3],
            },
            2,
        ),
        (
            TextSymbol {
                name: "second",
                bytes: SECOND,
            },
            0,
        ),
    ]));
    let first = named(&object, "first");

    assert_eq!(object.format, analysis::BinaryFormat::Coff);
    assert_eq!(first.size, 2);
    assert_eq!(first.extent(&object), Some(6));
    assert_eq!(
        first
            .assembly(&object)
            .expect("a listing")
            .instructions
            .len(),
        6
    );
}

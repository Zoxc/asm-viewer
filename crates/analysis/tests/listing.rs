//! A whole section as one address-keyed listing: the label skeleton built from the symbol
//! addresses alone, each stretch decoded on demand, and the bytes between one symbol's
//! extent and the next label said as a gap rather than decoded.

mod common;

use analysis::{parse_object, Architecture, GapKind, Listing, Object, Section};
use common::{
    caller_and_target, committed_fixture, declared_code_images, elf_text_padded, elf_x86_64,
    elf_x86_64_with_dwarf, named, parse, pe_dll, text, DwarfFixture, DwarfRow, DwarfSection,
    ExportedSymbol, TextSymbol, UnitRanges, TEXT_ADDRESS,
};
use std::{path::PathBuf, sync::Arc};

/// Six bytes of code then four of padding, with DWARF saying the function is six: the
/// shape a linker leaves between two functions.
const FIRST: &[u8] = &[
    0x90, 0x90, 0x90, 0x90, 0x90, 0xC3, // the function
    0xCC, 0xCC, 0xCC, 0xCC, // padding
];
const SECOND: &[u8] = &[0x90, 0xC3];

/// `first` at 0 with a stated extent of 6, `second` at 10.
fn padded() -> Vec<u8> {
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
            subprograms: &[(0, 6), (1, 2)],
            base_symbol: Some(0),
        }],
        unit_ranges: UnitRanges::Relocated,
    })
}

/// One symbol whose derivation runs past `MAX_DERIVED_SIZE`, from `tests/extent.rs`.
fn huge() -> Vec<u8> {
    let mut text = vec![0x90u8; (2 << 20) + 16];
    *text.last_mut().unwrap() = 0xC3;
    elf_x86_64_with_dwarf(DwarfFixture {
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
    })
}

/// `jumper` ends in a `jmp` to `target`, the next symbol: a tail call.
fn tail_jump() -> Vec<u8> {
    elf_x86_64(
        &[
            TextSymbol {
                name: "jumper",
                bytes: &[0xEB, 0x01, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[],
    )
}

/// Two names at one address, then a third symbol.
fn aliased() -> Vec<u8> {
    elf_x86_64(
        &[
            TextSymbol {
                name: "alias",
                bytes: &[],
            },
            TextSymbol {
                name: "function",
                bytes: &[0x90, 0x90, 0xC3],
            },
            TextSymbol {
                name: "next",
                bytes: &[0xC3],
            },
        ],
        &[],
    )
}

/// Three bytes nothing names, then two symbols.
fn leading_gap(architecture: Architecture) -> Vec<u8> {
    elf_text_padded(
        architecture,
        &[0xCC, 0xCC, 0xCC],
        &[
            TextSymbol {
                name: "first",
                bytes: &[0x90, 0xC3],
            },
            TextSymbol {
                name: "second",
                bytes: &[0xC3],
            },
        ],
        &[],
    )
}

fn committed(name: &str) -> Arc<Object> {
    parse_object(
        committed_fixture(name).as_slice().into(),
        name.to_string(),
        PathBuf::from(name),
    )
    .expect("the committed fixture parses")
}

/// Every shape the suite can build, plus the two objects a real toolchain built.
fn corpus() -> Vec<(String, Arc<Object>)> {
    let mut corpus = vec![
        ("caller and target".to_owned(), parse(&caller_and_target())),
        ("padded".to_owned(), parse(&padded())),
        ("huge".to_owned(), parse(&huge())),
        ("tail jump".to_owned(), parse(&tail_jump())),
        ("aliased".to_owned(), parse(&aliased())),
        (
            "leading gap".to_owned(),
            parse(&leading_gap(Architecture::X86_64)),
        ),
        (
            "aarch64".to_owned(),
            parse(&leading_gap(Architecture::Aarch64)),
        ),
        ("line_fixture.o".to_owned(), committed("line_fixture.o")),
        (
            "line_fixture_split.o".to_owned(),
            committed("line_fixture_split.o"),
        ),
    ];
    for (name, image) in declared_code_images() {
        corpus.push((name.to_owned(), parse(&image)));
    }
    corpus
}

fn section_end(section: &Section) -> u64 {
    section.address + section.data.len() as u64
}

fn listing_of(object: &Arc<Object>, name: &str) -> Listing {
    let section = object
        .sections
        .iter()
        .find(|section| section.name == name)
        .unwrap_or_else(|| panic!("a section named {name}"));
    Listing::new(object, section.clone())
}

/// The names at each stretch's label, in stretch order.
fn labels(listing: &Listing) -> Vec<Vec<&str>> {
    listing
        .stretches()
        .iter()
        .map(|stretch| {
            stretch
                .symbols
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect()
        })
        .collect()
}

fn ranges(listing: &Listing) -> Vec<(u64, u64)> {
    listing
        .stretches()
        .iter()
        .map(|stretch| (stretch.range.start, stretch.range.end))
        .collect()
}

/// The invariants every listing holds, whatever the object: the stretches partition the
/// section's bytes exactly, in order, only the first without a label; every symbol inside
/// the section is at exactly one label; and a stretch's code is the symbol's own listing,
/// with the gap picking up exactly where the symbol's extent stops.
#[test]
fn every_listing_partitions_its_section_and_agrees_with_the_symbols() {
    for (name, object) in corpus() {
        for section in &object.sections {
            let listing = Listing::new(&object, section.clone());
            let stretches = listing.stretches();
            let context = format!("{name}, section {}", section.name);

            if section.data.is_empty() {
                assert!(stretches.is_empty(), "{context}: no bytes, no stretches");
                continue;
            }

            let end = section_end(section);
            assert_eq!(
                stretches.first().map(|s| s.range.start),
                Some(section.address),
                "{context}: starts at the section's start"
            );
            assert_eq!(
                stretches.last().map(|s| s.range.end),
                Some(end),
                "{context}: ends at the section's end"
            );
            for (index, stretch) in stretches.iter().enumerate() {
                assert!(
                    stretch.range.start < stretch.range.end,
                    "{context}: stretch {index} is empty"
                );
                if let Some(next) = stretches.get(index + 1) {
                    assert_eq!(
                        stretch.range.end,
                        next.range.start,
                        "{context}: stretches {index} and {} are not contiguous",
                        index + 1
                    );
                }
                assert!(
                    index == 0 || !stretch.symbols.is_empty(),
                    "{context}: only the first stretch may have no label"
                );
                for symbol in &stretch.symbols {
                    assert_eq!(symbol.address, stretch.range.start, "{context}");
                }
                assert_eq!(listing.stretch_at(stretch.range.start), Some(index));
                assert_eq!(listing.stretch_at(stretch.range.end - 1), Some(index));
            }
            assert_eq!(listing.stretch_at(end), None, "{context}");

            let inside = object
                .symbols
                .values()
                .filter(|symbol| {
                    symbol
                        .section
                        .as_ref()
                        .is_some_and(|own| Arc::ptr_eq(own, section))
                })
                .filter(|symbol| (section.address..end).contains(&symbol.address))
                .count();
            let labelled: usize = stretches.iter().map(|s| s.symbols.len()).sum();
            assert_eq!(
                inside, labelled,
                "{context}: every symbol inside is at one label"
            );

            for (index, stretch) in stretches.iter().enumerate() {
                let decoded = listing.decode(&object, index).expect("a stretch decodes");
                let Some(symbol) = stretch.symbol() else {
                    assert!(decoded.code.is_none(), "{context}: a gap has no code");
                    assert_eq!(
                        decoded.gap.as_ref().map(|gap| gap.range.clone()),
                        Some(stretch.range.clone()),
                        "{context}: the leading stretch is all gap"
                    );
                    continue;
                };

                let own = symbol.assembly(&object);
                assert_eq!(
                    own.is_some(),
                    decoded.code.is_some(),
                    "{context}: {} decodes the same way both ways",
                    symbol.name
                );
                if let (Some(own), Some(code)) = (own, decoded.code) {
                    assert_eq!(own.undecodable, code.undecodable);
                    let rows = |assembly: &analysis::Assembly| {
                        assembly
                            .instructions
                            .iter()
                            .map(|instruction| (instruction.address, text(instruction)))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(rows(&own), rows(&code), "{context}: {}", symbol.name);
                }

                let claimed = stretch.range.start + symbol.extent(&object).unwrap_or(0);
                match decoded.gap {
                    Some(gap) => {
                        assert_eq!(gap.range.start, claimed, "{context}: {}", symbol.name);
                        assert_eq!(gap.range.end, stretch.range.end, "{context}");
                    }
                    None => assert!(
                        claimed >= stretch.range.end,
                        "{context}: {} reaches the next label",
                        symbol.name
                    ),
                }
            }
        }
    }
}

#[test]
fn the_skeleton_is_the_symbol_addresses() {
    let object = parse(&caller_and_target());
    let listing = listing_of(&object, ".text");

    assert_eq!(labels(&listing), [vec!["caller"], vec!["target"]]);
    assert_eq!(ranges(&listing), [(0, 6), (6, 7)]);
}

#[test]
fn padding_past_a_stated_extent_is_a_gap_of_bytes() {
    let object = parse(&padded());
    let listing = listing_of(&object, ".text");
    assert_eq!(ranges(&listing), [(0, 10), (10, 12)]);

    // `first` is six bytes of code by DWARF; the four `int3`s up to `second` are said, and
    // not decoded.
    let first = listing.decode(&object, 0).expect("first decodes");
    let code = first.code.expect("first has code");
    assert_eq!(code.instructions.len(), 6);
    let gap = first.gap.expect("the padding is a gap");
    assert_eq!(gap.range, 6..10);
    assert_eq!(gap.kind, GapKind::Bytes);
    assert_eq!(&listing.section().data[6..10], &FIRST[6..]);

    // `second` reaches the section's end exactly.
    let second = listing.decode(&object, 1).expect("second decodes");
    assert_eq!(second.code.map(|code| code.instructions.len()), Some(2));
    assert_eq!(second.gap, None);
}

#[test]
fn the_rest_of_a_stretch_cut_at_a_megabyte_is_said_to_be_cut() {
    let object = parse(&huge());
    let listing = listing_of(&object, ".text");
    let length = (2 << 20) + 16;
    assert_eq!(ranges(&listing), [(0, length)]);

    let decoded = listing.decode(&object, 0).expect("huge decodes");
    assert_eq!(
        decoded.code.map(|code| code.instructions.len()),
        Some(1 << 20)
    );
    let gap = decoded.gap.expect("the rest is a gap");
    assert_eq!(gap.range, (1 << 20)..length);
    assert_eq!(gap.kind, GapKind::Cut);
}

#[test]
fn bytes_before_the_first_symbol_are_a_stretch_with_no_label() {
    let object = parse(&leading_gap(Architecture::X86_64));
    let listing = listing_of(&object, ".text");

    assert_eq!(labels(&listing), [vec![], vec!["first"], vec!["second"]]);
    assert_eq!(ranges(&listing), [(0, 3), (3, 5), (5, 6)]);

    let leading = listing
        .decode(&object, 0)
        .expect("the leading stretch decodes");
    assert!(leading.code.is_none());
    assert_eq!(
        leading.gap,
        Some(analysis::Gap {
            range: 0..3,
            kind: GapKind::Bytes
        })
    );
    // And the first symbol's own stretch is unaffected by what sits before it.
    let first = listing.decode(&object, 1).expect("first decodes");
    assert_eq!(first.code.map(|code| code.instructions.len()), Some(2));
    assert_eq!(first.gap, None);
}

#[test]
fn a_section_with_no_symbols_is_one_stretch_of_bytes() {
    let object = parse(&elf_text_padded(
        Architecture::X86_64,
        &[0x90, 0x90, 0xC3],
        &[],
        &[],
    ));
    let listing = listing_of(&object, ".text");

    assert_eq!(labels(&listing), [Vec::<&str>::new()]);
    assert_eq!(ranges(&listing), [(0, 3)]);
    let decoded = listing.decode(&object, 0).expect("the stretch decodes");
    assert!(decoded.code.is_none());
    assert_eq!(decoded.gap.map(|gap| gap.kind), Some(GapKind::Bytes));
}

#[test]
fn two_names_at_one_address_share_a_stretch_in_the_files_order() {
    let object = parse(&aliased());
    let listing = listing_of(&object, ".text");

    assert_eq!(labels(&listing), [vec!["alias", "function"], vec!["next"]]);
    assert_eq!(ranges(&listing), [(0, 3), (3, 4)]);
    let decoded = listing.decode(&object, 0).expect("the run decodes");
    assert_eq!(decoded.code.map(|code| code.instructions.len()), Some(3));
    assert_eq!(decoded.gap, None);
}

#[test]
fn stretch_at_finds_the_stretch_an_address_is_in() {
    let object = parse(&padded());
    let listing = listing_of(&object, ".text");

    assert_eq!(listing.stretch_at(0), Some(0));
    assert_eq!(listing.stretch_at(7), Some(0), "inside first's padding");
    assert_eq!(listing.stretch_at(10), Some(1));
    assert_eq!(listing.stretch_at(11), Some(1));
    assert_eq!(listing.stretch_at(12), None, "the section's end");
    assert_eq!(listing.stretch_at(u64::MAX), None);
}

#[test]
fn a_tail_jump_names_the_next_symbols_stretch() {
    let object = parse(&tail_jump());
    let listing = listing_of(&object, ".text");
    assert_eq!(labels(&listing), [vec!["jumper"], vec!["target"]]);

    let jumper = listing.decode(&object, 0).expect("jumper decodes");
    let code = jumper.code.expect("jumper has code");
    // Not an edge of the symbol's own — it leaves it — but the row says where it goes,
    // and the listing has a stretch there.
    assert!(code.edges.is_empty());
    let target = code.instructions[0]
        .branch
        .expect("the jump names an address");
    assert_eq!(target, 3);
    assert_eq!(listing.stretch_at(target), Some(1));
    assert_eq!(listing.stretches()[1].range.start, target);
}

#[test]
fn an_architecture_nothing_decodes_still_has_its_skeleton() {
    let object = parse(&leading_gap(Architecture::Aarch64));
    let listing = listing_of(&object, ".text");
    assert_eq!(labels(&listing), [vec![], vec!["first"], vec!["second"]]);

    let first = listing.decode(&object, 1).expect("first decodes");
    let code = first.code.expect("a third answer, not none");
    assert_eq!(code.undecodable, Some("aarch64"));
    assert!(code.instructions.is_empty());
    assert_eq!(first.gap, None);

    let leading = listing.decode(&object, 0).expect("the gap decodes");
    assert_eq!(leading.gap.map(|gap| gap.kind), Some(GapKind::Bytes));
}

#[test]
fn a_symbol_pointed_outside_its_section_is_left_out() {
    let mut data = elf_x86_64(
        &[
            TextSymbol {
                name: "good",
                bytes: &[0x90, 0xC3],
            },
            TextSymbol {
                name: "wild",
                bytes: &[0xC3],
            },
        ],
        &[],
    );

    // Point `wild` clean out of `.text`, which the writer will not do for us.
    {
        use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
        let file = object::File::parse(&data[..]).expect("the fixture parses");
        let symtab = file
            .section_by_name(".symtab")
            .expect(".symtab is there")
            .file_range()
            .expect(".symtab is in the file")
            .0 as usize;
        let index = file
            .symbols()
            .find(|symbol| symbol.name() == Ok("wild"))
            .expect("the symbol is there")
            .index()
            .0;
        data[symtab + index * 24 + 8..symtab + index * 24 + 16]
            .copy_from_slice(&0xDEAD_BEEFu64.to_le_bytes());
    }

    let object = parse(&data);
    assert_eq!(named(&object, "wild").address, 0xDEAD_BEEF);
    let listing = listing_of(&object, ".text");

    // `good` runs to the section's end: the wild address is past it and bounds nothing.
    assert_eq!(labels(&listing), [vec!["good"]]);
    assert_eq!(ranges(&listing), [(0, 3)]);
    let good = listing.decode(&object, 0).expect("good decodes");
    assert_eq!(good.code.map(|code| code.instructions.len()), Some(3));
    assert_eq!(good.gap, None);
}

#[test]
fn a_linked_image_lists_its_declared_code_at_its_addresses() {
    // `first` is exported at 0 and the entry point declared at 4, in a `.text` the writer
    // pads out to its file alignment with bytes nothing names.
    const TEXT: &[u8] = &[0x90, 0x90, 0x90, 0xC3, 0x90, 0xC3];
    let object = parse(&pe_dll(
        TEXT,
        &[ExportedSymbol {
            name: "first",
            offset: 0,
            size: 0,
            code: true,
        }],
        Some(4),
    ));
    let listing = listing_of(&object, ".text");
    let end = section_end(listing.section());
    assert!(end >= TEXT_ADDRESS + 6);

    assert_eq!(labels(&listing), [vec!["first"], vec!["<entry point>"]]);
    assert_eq!(
        ranges(&listing),
        [(TEXT_ADDRESS, TEXT_ADDRESS + 4), (TEXT_ADDRESS + 4, end)]
    );

    let first = listing.decode(&object, 0).expect("first decodes");
    assert_eq!(first.code.map(|code| code.instructions.len()), Some(4));
    assert_eq!(first.gap, None);
    // Without DWARF the entry point's extent is everything up to the section's end, the
    // alignment padding included: no gap, because nothing says where the function stops.
    let entry = listing.decode(&object, 1).expect("the entry point decodes");
    assert!(entry.code.is_some());
    assert_eq!(entry.gap, None);
}

#[test]
fn the_committed_objects_list_every_function_at_its_own_address() {
    let flat = committed("line_fixture.o");
    let listing = listing_of(&flat, ".text");
    assert_eq!(
        labels(&listing),
        [vec!["add"], vec!["twice"], vec!["sum_to"]]
    );
    assert_eq!(
        listing
            .stretches()
            .iter()
            .map(|stretch| stretch.range.start)
            .collect::<Vec<_>>(),
        [0, 0x14, 0x30]
    );

    // Three sections, every one at address 0 with one function at its start: pointer
    // identity is what keeps each listing to its own section's symbol.
    let split = committed("line_fixture_split.o");
    for name in ["add", "twice", "sum_to"] {
        let listing = listing_of(&split, &format!(".text.{name}"));
        assert_eq!(labels(&listing), [vec![name]]);
        assert_eq!(listing.stretches()[0].range.start, 0);
    }
}

/// A section whose `sh_addr` is the last address there is has bytes with no addresses to be
/// at: nothing to list, and nothing to panic over. Found by the mutation sweep, where the
/// section's end wrapping to its own start is the cheapest thing a poisoned header buys.
#[test]
fn a_section_at_the_end_of_the_address_space_lists_nothing() {
    let mut data = caller_and_target();

    // Point `.text` at `u64::MAX`, which the writer will not do for us.
    {
        use object::{Object as _, ObjectSection as _};
        let file = object::File::parse(&data[..]).expect("the fixture parses");
        let index = file
            .section_by_name(".text")
            .expect(".text is there")
            .index()
            .0;
        let shoff = u64::from_le_bytes(data[0x28..0x30].try_into().unwrap()) as usize;
        let shentsize = u16::from_le_bytes(data[0x3A..0x3C].try_into().unwrap()) as usize;
        let header = shoff + index * shentsize;
        // `sh_addr` is the third field of an ELF64 section header, after two 32-bit ones.
        data[header + 16..header + 24].copy_from_slice(&u64::MAX.to_le_bytes());
    }

    let object = parse(&data);
    let listing = listing_of(&object, ".text");
    assert_eq!(listing.section().address, u64::MAX);
    assert_eq!(listing.section().data.len(), 7);
    assert!(listing.stretches().is_empty());
    assert_eq!(listing.stretch_at(u64::MAX), None);
    assert!(listing.decode(&object, 0).is_none());
}

use super::*;
use analysis::{CodeListing, Object};
use std::collections::HashMap;
use std::path::Path;

/// One of the two committed gcc objects the analysis crate is pinned against, parsed the
/// way the app parses it.
fn fixture(name: &str) -> Arc<Object> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/analysis/tests/fixtures")
        .join(name);
    analysis::open_files(vec![path])
        .into_iter()
        .next()
        .expect("the fixture parses")
}

/// The `-ffunction-sections` build: `add`, `twice` and `sum_to` in three sections, every
/// one at 0 in the file and placed at 0x10, 0x30 and 0x50.
fn split() -> (Arc<Object>, Arc<CodeListing>) {
    let object = fixture("line_fixture_split.o");
    let code = Arc::new(CodeListing::new(&object));
    assert_eq!(code.sections().len(), 3, "the fixture's layout moved");
    (object, code)
}

/// What decoding stretch `flat` of `code` draws, exactly as the worker will build it.
fn decode(object: &Object, code: &CodeListing, rows: &Rows, flat: usize) -> Body {
    let place = rows.place(flat).expect("the stretch exists");
    let decoded = code.decode(object, place).expect("the stretch decodes");
    let lanes = Arc::new(match &decoded.code {
        Some(assembly) => Lanes::new(&assembly.edges, assembly.instructions.len()),
        None => Lanes::new(&[], 0),
    });
    Body {
        assembly: decoded.code,
        lanes,
        gap: decoded.gap.map(|gap| gap.range),
    }
}

/// The rows stretch `flat` takes, header and labels included: a question only these
/// tests ask, off the prefix sums.
fn rows_of(rows: &Rows, flat: usize) -> Range<usize> {
    rows.starts[flat]..rows.starts[flat + 1]
}

fn nothing_decoded(code: Arc<CodeListing>) -> Rows {
    Rows::new(code, |_| None)
}

fn kinds(rows: &Rows) -> Vec<Row> {
    (0..rows.len()).map(|i| rows.row(i).unwrap()).collect()
}

/// Before a byte is decoded, a stretch is its header where a section starts, a label per
/// symbol, and as many empty rows as its bytes suggest -- and never none, so that every
/// label has a row under it.
#[test]
fn a_stretch_nobody_decoded_is_a_run_of_empty_rows_sized_by_its_bytes() {
    let (_, code) = split();
    let rows = nothing_decoded(code.clone());

    assert_eq!(rows.stretches.len(), 3);
    let mut expected = 0;
    for (flat, placed) in code.sections().iter().enumerate() {
        let stretch = &placed.listing.stretches()[0];
        let bytes = stretch.range.end - stretch.range.start;
        let estimate = bytes.div_ceil(ESTIMATED_BYTES_PER_ROW).max(1) as usize;
        let first = rows_of(&rows, flat);
        assert_eq!(
            first.start, expected,
            "stretch {flat} starts where the last ended"
        );
        // The rule over the stretch and the blank under it, which every stretch but the
        // listing's first has, then the header, the blank under it and the label.
        let mut at = first.start;
        if flat > 0 {
            assert_eq!(rows.row(at), Some(Row::Rule { stretch: flat }));
            assert_eq!(
                rows.row(at + 1),
                Some(Row::Space {
                    stretch: flat,
                    under: false
                })
            );
            at += 2;
        }
        assert_eq!(rows.row(at), Some(Row::Header { section: flat }));
        assert_eq!(
            rows.row(at + 1),
            Some(Row::Space {
                stretch: flat,
                under: true
            })
        );
        assert_eq!(
            rows.row(at + 2),
            Some(Row::Label {
                stretch: flat,
                index: 0
            })
        );
        let above = at + 3 - first.start;
        assert_eq!(rows.body_start(flat), Some(first.start + above));
        for k in 0..estimate {
            assert_eq!(
                rows.row(first.start + above + k),
                Some(Row::Empty {
                    stretch: flat,
                    index: k
                })
            );
        }
        expected += above + estimate;
        assert_eq!(first.end, expected);
    }
    assert_eq!(rows.len(), expected);
    assert_eq!(rows.row(expected), None, "no row past the end");
}

/// A rule stands over every stretch but the listing's first, with a blank under it, and
/// a blank under a section's header: so one function is told from the next by a line, and
/// neither the line nor a header is ever drawn against a name.
#[test]
fn a_rule_and_a_blank_stand_over_every_stretch_and_a_blank_under_every_header() {
    let object = fixture("line_fixture.o");
    let code = Arc::new(CodeListing::new(&object));
    let rows = nothing_decoded(code);
    assert!(rows.stretches.len() > 2, "the fixture's layout moved");
    let kinds = kinds(&rows);

    for flat in 0..rows.stretches.len() {
        let first = rows_of(&rows, flat).start;
        let over = 2 * usize::from(flat > 0);
        if flat == 0 {
            assert!(
                !matches!(kinds[first], Row::Rule { .. } | Row::Space { .. }),
                "the listing opens on a rule"
            );
        } else {
            assert_eq!(kinds[first], Row::Rule { stretch: flat });
            assert_eq!(
                kinds[first + 1],
                Row::Space {
                    stretch: flat,
                    under: false
                }
            );
        }
        if matches!(kinds[first + over], Row::Header { .. }) {
            assert_eq!(
                kinds[first + over + 1],
                Row::Space {
                    stretch: flat,
                    under: true
                }
            );
        }
    }

    // Which is the whole of the point: nothing but a blank, or another name at the same
    // address, is ever drawn against the row above a label -- the rule included.
    for (row, kind) in kinds.iter().enumerate().skip(1) {
        if matches!(kind, Row::Label { .. }) {
            assert!(
                matches!(kinds[row - 1], Row::Space { .. } | Row::Label { .. }),
                "row {row}'s label sits on {:?}",
                kinds[row - 1]
            );
        }
    }
}

/// Every row names an address and that address finds the row again, decoded or not:
/// what keeps the reader's row still while the rows around it change.
#[test]
fn an_address_finds_the_row_that_draws_it_and_the_row_names_it_back() {
    let (object, code) = split();
    let empty = nothing_decoded(code.clone());
    let body = decode(&object, &code, &empty, 1);
    let half = Rows::new(code.clone(), |flat| (flat == 1).then(|| body.clone()));

    for (name, rows) in [("estimated", &empty), ("half decoded", &half)] {
        for row in 0..rows.len() {
            let address = rows
                .address_of(row)
                .unwrap_or_else(|| panic!("{name}: row {row} has an address"));
            let found = rows.row_for(address);
            let kind = rows.row(row).unwrap();
            let stretch = match kind {
                Row::Header { section } => rows
                    .flat(Place {
                        section,
                        stretch: 0,
                    })
                    .unwrap(),
                Row::Rule { stretch }
                | Row::Space { stretch, .. }
                | Row::Label { stretch, .. }
                | Row::Empty { stretch, .. }
                | Row::Instruction { stretch, .. }
                | Row::Separator { stretch, .. }
                | Row::Gap { stretch, .. } => stretch,
            };
            let expected = if rows.start_of(stretch) == Some(address) {
                // The header, the labels and the first instruction all sit at the
                // stretch's start, which finds the stretch's first row.
                rows_of(&rows, stretch).start
            } else if matches!(kind, Row::Separator { .. }) {
                // A separator shares its address with the instruction below it, which is
                // the row an address finds.
                row + 1
            } else {
                row
            };
            assert_eq!(
                found,
                Some(expected),
                "{name}: row {row} ({kind:?}) at {address:#x}"
            );
        }
    }

    // Between two sections is nowhere, and so is past the end.
    let air = code.sections()[0].range().end;
    assert!(air < code.sections()[1].range().start);
    assert_eq!(empty.row_for(air), None);
    assert_eq!(empty.row_for(u64::MAX), None);
}

/// An address that is no row's own -- inside an instruction, inside a row of bytes,
/// between two guessed rows -- finds the row **at or below** it: the last row of its
/// stretch whose address is not past it, which is where a call into the middle of a
/// function lands a reader. Decoded or not, so a target in a stretch the worker has not
/// reached lands on its guess and, once it has, on its instruction.
#[test]
fn an_address_inside_a_row_finds_the_row_at_or_below_it() {
    let (object, code) = split();
    let empty = nothing_decoded(code.clone());
    let body = decode(&object, &code, &empty, 2);
    let half = Rows::new(code.clone(), |flat| (flat == 2).then(|| body.clone()));
    // A gap too: `add` decoded as if its extent stopped at its third instruction, the
    // rest of its stretch left over as rows of bytes. The fixture's functions fill their
    // stretches, so the gap is made by hand out of the same decode.
    let mut cut = decode(&object, &code, &empty, 0);
    let assembly = cut.assembly.clone().expect("add decodes");
    assert!(assembly.instructions.len() > 3, "add is short");
    let cut_at = assembly.instructions[2].address;
    let stretch = &code.sections()[0].listing.stretches()[0];
    cut.gap = Some(cut_at..stretch.range.end);
    cut.assembly = Some(Arc::new(Assembly {
        instructions: assembly.instructions[..2].to_vec(),
        edges: Vec::new(),
        undecodable: None,
    }));
    cut.lanes = Arc::new(Lanes::new(&[], 2));
    let with_gap = Rows::new(code.clone(), |flat| (flat == 0).then(|| cut.clone()));

    let mut inside = 0;
    for (name, rows) in [
        ("estimated", &empty),
        ("half decoded", &half),
        ("with a gap", &with_gap),
    ] {
        for flat in 0..rows.stretches.len() {
            let range = rows_of(rows, flat);
            let start = rows.start_of(flat).unwrap();
            let end = start + rows.stretches[flat].bytes;
            for address in start..end {
                let expected = if address == start {
                    range.start
                } else {
                    // The last row of the stretch at or before the address; a separator
                    // shares its address with the row below it, which is the one found.
                    range
                        .clone()
                        .filter(|&row| rows.address_of(row).is_some_and(|own| own <= address))
                        .last()
                        .unwrap()
                };
                let found = rows.row_for(address);
                assert_eq!(
                    found,
                    Some(expected),
                    "{name}: {address:#x} is drawn in row {expected}"
                );
                if rows.address_of(expected) != Some(address) {
                    inside += 1;
                }
            }
        }
    }
    assert!(inside > 0, "no address inside a row was tried");

    // Spelt out: the instruction holding the byte, and the row of bytes covering it.
    let bias = code.sections()[0].bias();
    let body = with_gap.body_start(0).unwrap();
    let second = &assembly.instructions[1];
    assert!(second.bytes.len() > 1, "a one-byte instruction");
    assert_eq!(with_gap.row_for(second.address + bias + 1), Some(body + 1));
    assert_eq!(
        with_gap.row(body + 2),
        Some(Row::Gap {
            stretch: 0,
            index: 0
        })
    );
    assert_eq!(with_gap.row_for(cut_at + bias + 3), Some(body + 2));
}

/// The row a caret goes on for an address is the row **holding** the byte: `row_for`'s
/// answer, except that a stretch's start -- the header's and the labels' address as much
/// as the first instruction's -- answers the first row of the body, decoded or guessed,
/// the name over it being no row of code. Everything inside the stretch is `row_for`.
#[test]
fn a_caret_goes_on_the_row_holding_the_byte_and_never_on_a_label() {
    let (object, code) = split();
    let empty = nothing_decoded(code.clone());
    let body = decode(&object, &code, &empty, 1);
    let half = Rows::new(code.clone(), |flat| (flat == 1).then(|| body.clone()));

    for (name, rows) in [("estimated", &empty), ("half decoded", &half)] {
        for flat in 0..rows.stretches.len() {
            let start = rows.start_of(flat).unwrap();
            let end = start + rows.stretches[flat].bytes;
            let body = rows.body_start(flat).unwrap();
            assert_ne!(
                rows.row_for(start),
                Some(body),
                "{name}: the view lands on the label"
            );
            assert_eq!(
                rows.body_row_for(start),
                Some(body),
                "{name}: stretch {flat}"
            );
            assert!(matches!(
                rows.row(body),
                Some(Row::Instruction { index: 0, .. } | Row::Empty { index: 0, .. })
            ));
            for address in start + 1..end {
                assert_eq!(
                    rows.body_row_for(address),
                    rows.row_for(address),
                    "{name}: {address:#x}"
                );
            }
        }
    }

    let air = code.sections()[0].range().end;
    assert_eq!(empty.body_row_for(air), None);
}

/// Decoding a stretch replaces its guess with its rows; every row above it stays where it
/// was and every row below moves by the difference, which is what an address-keyed anchor
/// absorbs.
#[test]
fn decoding_a_stretch_settles_its_rows_and_moves_none_above_it() {
    let (object, code) = split();
    let before = nothing_decoded(code.clone());
    let body = decode(&object, &code, &before, 1);
    let after = Rows::new(code.clone(), |flat| (flat == 1).then(|| body.clone()));

    let middle = rows_of(&before, 1);
    let settled = rows_of(&after, 1);
    assert_eq!(
        settled.start, middle.start,
        "the stretch starts where it did"
    );
    let listing = body
        .lanes
        .listing_rows(body.assembly.as_ref().unwrap().instructions.len());
    let gap = body.gap.as_ref().map_or(0, |gap| gap_rows(gap));
    // The rule over the stretch and its blank, its header, the blank under that, and
    // its label.
    assert_eq!(settled.end - settled.start, 5 + listing + gap);
    assert_ne!(middle.len(), settled.len(), "the guess was not the truth");

    for row in 0..middle.start {
        assert_eq!(
            before.address_of(row),
            after.address_of(row),
            "row {row} moved"
        );
    }
    let shift = settled.len() as isize - middle.len() as isize;
    for row in middle.end..before.len() {
        let moved = (row as isize + shift) as usize;
        assert_eq!(
            before.address_of(row),
            after.address_of(moved),
            "row {row} → {moved}"
        );
    }
    assert_eq!(after.len() as isize, before.len() as isize + shift);

    // And the decoded rows are the symbol's own, in the symbol's own order.
    let kinds: Vec<Row> = kinds(&after)[settled.start..settled.end].to_vec();
    assert_eq!(kinds[0], Row::Rule { stretch: 1 });
    assert_eq!(
        kinds[1],
        Row::Space {
            stretch: 1,
            under: false
        }
    );
    assert_eq!(kinds[2], Row::Header { section: 1 });
    assert_eq!(
        kinds[3],
        Row::Space {
            stretch: 1,
            under: true
        }
    );
    assert_eq!(
        kinds[4],
        Row::Label {
            stretch: 1,
            index: 0
        }
    );
    assert_eq!(
        kinds[5],
        Row::Instruction {
            stretch: 1,
            index: 0
        }
    );
    assert!(kinds.iter().all(|kind| !matches!(kind, Row::Empty { .. })));
}

/// The addresses the listing draws are the layout's, so three functions that are all at
/// 0 in the file are at three addresses here, and an instruction's is its own plus the
/// section's place.
#[test]
fn a_relocatable_objects_sections_draw_at_their_placed_addresses() {
    let (object, code) = split();
    let empty = nothing_decoded(code.clone());
    let bodies: HashMap<usize, Body> = (0..3)
        .map(|flat| (flat, decode(&object, &code, &empty, flat)))
        .collect();
    let rows = Rows::new(code.clone(), |flat| bodies.get(&flat).cloned());

    let labels: Vec<u64> = (0..rows.len())
        .filter(|&row| matches!(rows.row(row), Some(Row::Label { .. })))
        .map(|row| rows.address_of(row).unwrap())
        .collect();
    assert_eq!(labels, [0x10, 0x30, 0x50]);

    for flat in 0..3 {
        let body = rows.body_start(flat).unwrap();
        assert_eq!(
            rows.row(body),
            Some(Row::Instruction {
                stretch: flat,
                index: 0
            })
        );
        assert_eq!(
            rows.address_of(body),
            rows.start_of(flat),
            "a symbol's first instruction is at its label"
        );
        assert_eq!(rows.bias(flat), Some(code.sections()[flat].bias()));
        let second = rows.address_of(body + 1).unwrap();
        let own = bodies[&flat].assembly.as_ref().unwrap().instructions[1].address;
        assert_eq!(second, own + code.sections()[flat].bias());
    }

    // The one-`.text` build is the same three functions at their own addresses, unmoved.
    let flat = fixture("line_fixture.o");
    let code = Arc::new(CodeListing::new(&flat));
    let rows = nothing_decoded(code);
    let labels: Vec<u64> = (0..rows.len())
        .filter(|&row| matches!(rows.row(row), Some(Row::Label { .. })))
        .map(|row| rows.address_of(row).unwrap())
        .collect();
    assert_eq!(labels, [0, 0x14, 0x30]);
    assert_eq!(
        (0..rows.len())
            .filter(|&row| matches!(rows.row(row), Some(Row::Header { .. })))
            .count(),
        1
    );
}

/// The window is the stretches within the buffer around the view that are not held yet,
/// nearest the middle of the view first, and no more than the cap.
#[test]
fn a_window_is_the_stretches_around_the_view_nearest_first_less_those_held() {
    let (_, code) = split();
    let rows = nothing_decoded(code);
    let middle = rows_of(&rows, 1);
    // A view of two rows in the middle stretch, with no buffer: that stretch alone.
    let view = middle.start + 1..middle.start + 3;
    assert_eq!(rows.window(view.clone(), 0, |_| false, 8), [1]);
    // A buffer reaching into both neighbours: the middle first, then the nearer one.
    let reach = rows.len();
    assert_eq!(rows.window(view.clone(), reach, |_| false, 8), [1, 0, 2]);
    // Held stretches are not asked for again, and the cap cuts the far ones.
    assert_eq!(
        rows.window(view.clone(), reach, |flat| flat == 1, 8),
        [0, 2]
    );
    assert_eq!(rows.window(view.clone(), reach, |_| false, 2), [1, 0]);
    // A view past the end wants nothing.
    assert!(rows
        .window(rows.len()..rows.len() + 5, 0, |_| false, 8)
        .is_empty());
}

/// A separator is the instruction below it for every purpose but drawing: its address is
/// that row's, and the stretch's body starts after its labels.
#[test]
fn a_separator_row_belongs_to_the_instruction_below_it() {
    let (object, code) = split();
    let empty = nothing_decoded(code.clone());
    // `sum_to` is the one with a loop, and so a block boundary.
    let body = decode(&object, &code, &empty, 2);
    let rows = Rows::new(code, |flat| (flat == 2).then(|| body.clone()));

    let separators: Vec<usize> = (0..rows.len())
        .filter(|&row| matches!(rows.row(row), Some(Row::Separator { .. })))
        .collect();
    assert!(!separators.is_empty(), "sum_to has a block boundary");
    for row in separators {
        let Some(Row::Separator { below, stretch }) = rows.row(row) else {
            unreachable!()
        };
        assert_eq!(
            rows.row(row + 1),
            Some(Row::Instruction {
                stretch,
                index: below
            })
        );
        assert_eq!(rows.address_of(row), rows.address_of(row + 1));
    }
}

/// A stretch whose decode found no instructions -- an architecture no backend reads --
/// draws its bytes as a gap. Without that its body is no rows at all: the function
/// collapses to its label as the worker answers, and no address inside it has a row.
#[test]
fn a_stretch_that_decoded_to_no_instructions_draws_its_bytes() {
    let (_, code) = split();
    let stretch = &code.sections()[1].listing.stretches()[0];
    let bytes = stretch.range.end - stretch.range.start;
    assert!(bytes > GAP_BYTES_PER_ROW, "one row of bytes proves little");
    // What the worker answers for a symbol on an architecture no backend decodes: an
    // assembly saying so, with no instructions, and no gap, the extent being the whole
    // stretch.
    let undecodable = Body {
        assembly: Some(Arc::new(Assembly {
            instructions: Vec::new(),
            edges: Vec::new(),
            undecodable: Some("aarch64"),
        })),
        lanes: Arc::new(Lanes::new(&[], 0)),
        gap: None,
    };
    let rows = Rows::new(code.clone(), |flat| {
        (flat == 1).then(|| undecodable.clone())
    });

    let range = rows_of(&rows, 1);
    let body = rows.body_start(1).unwrap();
    assert_eq!(range.end - body, bytes.div_ceil(GAP_BYTES_PER_ROW) as usize);
    let start = rows.start_of(1).unwrap();
    assert_eq!(rows.address_of(body), Some(start));
    for byte in 0..bytes {
        let index = (byte / GAP_BYTES_PER_ROW) as usize;
        let address = start + byte;
        assert_eq!(rows.row(body + index), Some(Row::Gap { stretch: 1, index }));
        assert_eq!(
            rows.body_row_for(address),
            Some(body + index),
            "{address:#x}"
        );
        let expected = if byte == 0 { range.start } else { body + index };
        assert_eq!(rows.row_for(address), Some(expected), "{address:#x}");
    }
}

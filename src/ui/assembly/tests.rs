//! What an instruction row draws, held against what the same row copies.

use super::*;
use analysis::BranchEdge;
use freya_testing::TestingRunner;
use std::path::Path;

/// An app that draws nothing. [`in_runtime`] wants a runtime, not a tree.
fn nothing() -> impl IntoElement {
    rect()
}

/// Run `body` inside a freya runtime, which is what asking for a colour or a font needs:
/// both are global states, and reading one outside a runtime panics.
fn in_runtime(body: impl FnOnce()) {
    TestingRunner::new(nothing, (10., 10.).into(), |_| body(), 1.);
}

fn span(text: &str, kind: SpanKind) -> (String, SpanKind) {
    (text.to_owned(), kind)
}

/// One instruction with nothing named on it, for a case below to name what it is about.
fn instruction(address: u64, format: Vec<(String, SpanKind)>) -> Instruction {
    Instruction {
        address,
        bytes: Vec::new(),
        format,
        relocation: None,
        relocation_span: None,
        branch_span: None,
        branch: None,
        target: None,
        target_span: None,
    }
}

/// The row's drawn text as columns: the head spans, the link as the one unit the text
/// engine counts it as, then the tail spans. The link's own text is taken from the copy,
/// which is what makes the columns around it the thing being compared -- and the two are
/// first held to agreeing about whether there is a link at all.
fn drawn(text: &Text<Option<InlineLink>>) -> Line {
    let inline = text.line.pieces.iter().find_map(|piece| match piece {
        crate::chars::Piece::Inline(name) => Some(name.clone()),
        crate::chars::Piece::Text(_) => None,
    });
    assert_eq!(
        inline.is_some(),
        text.links.is_some(),
        "one half has a link and the other has none"
    );

    let mut line = Line::default();
    for span in &text.head {
        line.push_text(span.text.to_string());
    }
    if let Some(name) = inline {
        line.push_inline(name);
    }
    for span in &text.tail {
        line.push_text(span.text.to_string());
    }
    line
}

/// A listing holding one instruction of every kind a row draws differently: no link at
/// all, a relocation's name in the operand it applies to, one appended because the
/// formatter offered no operand to put it in, a branch this listing has the row for, a
/// bare target, and a name inside a memory operand, which is the case with a tail. Every
/// one of them is padded to the operand column, which is what the drawing rewrites.
fn listing(target: Arc<SymbolData>) -> Assembly {
    let mut instructions = vec![
        instruction(
            0x00,
            vec![
                span("mov", SpanKind::Mnemonic),
                span("     ", SpanKind::Other),
                span("rax", SpanKind::Register),
                span(", ", SpanKind::Other),
                span("rbx", SpanKind::Register),
            ],
        ),
        instruction(
            0x08,
            vec![
                span("call", SpanKind::Mnemonic),
                span("    ", SpanKind::Other),
                span("0x0", SpanKind::Address),
            ],
        ),
        instruction(
            0x10,
            vec![
                span("nop", SpanKind::Mnemonic),
                span("   ", SpanKind::Other),
            ],
        ),
        instruction(
            0x18,
            vec![
                span("jmp", SpanKind::Mnemonic),
                span("     ", SpanKind::Other),
                span("0x0", SpanKind::Address),
            ],
        ),
        instruction(
            0x20,
            vec![
                span("call", SpanKind::Mnemonic),
                span("    ", SpanKind::Other),
                span("0x2000", SpanKind::Address),
            ],
        ),
        instruction(
            0x28,
            vec![
                span("mov", SpanKind::Mnemonic),
                span("     ", SpanKind::Other),
                span("rax", SpanKind::Register),
                span(", [", SpanKind::Other),
                span("0x0", SpanKind::Address),
                span("]", SpanKind::Other),
            ],
        ),
    ];

    // The relocation's name, in the operand it applies to.
    instructions[1].relocation = Some(target.clone());
    instructions[1].relocation_span = Some(2);
    // The same name with no operand to go in: appended, as `asm_line` appends it.
    instructions[2].relocation = Some(target.clone());
    // A branch back to the first row, which this listing has.
    instructions[3].branch_span = Some(2);
    instructions[3].branch = Some(0x00);
    // A target with no name and no row here.
    instructions[4].target = Some(0x2000);
    instructions[4].target_span = Some(2);
    // The name inside a memory operand, so the row has a tail.
    instructions[5].relocation = Some(target);
    instructions[5].relocation_span = Some(4);

    Assembly {
        instructions,
        edges: vec![BranchEdge { from: 3, to: 0 }],
        undecodable: None,
    }
}

/// **What a row copies is what it draws.** The spans a row is drawn from and the line it
/// is copied as come out of one [`split`], and the only place the two can drift apart is
/// the padding to the operand column, which the drawing rewrites in non-breaking spaces so
/// that skia does not trim it away. One unit each, so a column into the drawn text lands
/// on the character that same column of the copy lands on -- and a sweep selects what the
/// reader swept over.
#[test]
fn a_column_into_what_a_row_draws_is_a_column_into_what_it_copies() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();
    let symbol = Symbol {
        object: object.clone(),
        data: object
            .symbols_sorted
            .first()
            .expect("the fixture holds a symbol")
            .clone(),
    };
    let assembly = Arc::new(listing(symbol.data.clone()));
    let studied = Studied::with_assembly(symbol, Some(assembly.clone()));
    let width = studied.lanes.width;
    let data = AsmData::of(studied, None, 0, 0, width, false).expect("the fixture decodes");

    in_runtime(|| {
        for index in 0..data.assembly().instructions.len() {
            let text = instruction_text(&data, index, RowChars::default(), None, None, None);
            let (drawn, copied) = (drawn(&text), &text.line);

            for col in 0..copied.units() {
                assert_eq!(
                    drawn.slice(col, col + 1).replace('\u{a0}', " "),
                    copied.slice(col, col + 1),
                    "row {index}, column {col}: drew {drawn} and copied {copied}"
                );
            }
            // Past the end of the copy the drawn text holds only the formatter's padding,
            // which the copy trims and skia does not measure either.
            assert!(
                drawn
                    .slice(copied.units(), drawn.units())
                    .chars()
                    .all(|c| c == ' ' || c == '\u{a0}'),
                "row {index} draws {drawn} past the end of {copied}"
            );
        }
    });
}

/// **Every link a row can have is one inline piece, the fourth included.** [`split`] says
/// which of the four it lifted out and what that one says, so a line is head, link, tail
/// whichever it was: a relocation's name in the operand it applies to, a branch this
/// listing has the row for, a bare target, and -- the case with no span of its own -- a
/// name the formatter offered no operand for, appended behind one space. That last one is
/// the whole of what `asm_line` puts after the address, so the two are held together here
/// as well.
#[test]
fn every_kind_of_link_is_one_inline_piece() {
    let target = Arc::new(SymbolData {
        name: "_ZN3add3addE".to_owned(),
        demangled: Some("add".to_owned()),
        address: 0x100,
        section: None,
        size: 0,
    });
    let assembly = listing(target);
    let kinds = (0..assembly.instructions.len())
        .map(|index| {
            split(&assembly.instructions[index], linked(&assembly, index))
                .1
                .map(|link| link.kind)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            None,
            Some(Link::Relocation),
            Some(Link::Appended),
            Some(Link::Branch),
            Some(Link::Target),
            Some(Link::Relocation),
        ]
    );

    let lines = (0..assembly.instructions.len())
        .map(|index| instruction_line(&assembly, index))
        .collect::<Vec<_>>();
    let inlines = |line: &Line| {
        line.pieces
            .iter()
            .filter_map(|piece| match piece {
                crate::chars::Piece::Inline(name) => Some(name.clone()),
                crate::chars::Piece::Text(_) => None,
            })
            .collect::<Vec<_>>()
    };

    // The padding after the last span is not text, and a row with no link has no piece
    // the text engine counts as one unit.
    assert_eq!(lines[0].to_string(), "mov     rax, rbx");
    assert!(inlines(&lines[0]).is_empty());

    assert_eq!(lines[1].to_string(), "call    add");
    assert_eq!(inlines(&lines[1]), ["add"]);
    // Appended: every span, then the space, then the name -- one unit, as the others are.
    assert_eq!(lines[2].to_string(), "nop    add");
    assert_eq!(inlines(&lines[2]), ["add"]);
    assert_eq!(lines[2].units(), "nop    ".len() + 1);
    assert_eq!(lines[3].to_string(), "jmp     0x0");
    assert_eq!(inlines(&lines[3]), ["0x0"]);
    assert_eq!(lines[4].to_string(), "call    0x2000");
    assert_eq!(inlines(&lines[4]), ["0x2000"]);
    // The name inside a memory operand: the tail is drawn after it.
    assert_eq!(lines[5].to_string(), "mov     rax, [add]");
    assert_eq!(inlines(&lines[5]), ["add"]);

    for (index, line) in lines.iter().enumerate() {
        let instruction = &assembly.instructions[index];
        assert_eq!(
            asm_line(instruction, 0),
            format!("{:016X} {line}", instruction.address),
            "instruction {index}"
        );
    }
}

/// **A reveal the pane owes goes to a listing row, and the pair's is an instruction.**
/// The pane's own run is already counted in listing rows; the other pane's is the first
/// instruction its lines produced, which the separators above it make a row of.
#[test]
fn the_row_a_reveal_goes_to_is_the_runs_own_or_the_paired_instructions() {
    // A listing of six instructions with a branch landing on the fourth, which is the
    // one separator: instruction 3 is drawn at row 4.
    let lanes = Lanes::new(&[BranchEdge { from: 0, to: 3 }], 6);
    let pick = line_pick(Arc::from("now.c"), 7, None, Owed::default());

    assert_eq!(
        owed_row(&Owing::Own(4..=6), &lanes, |_| panic!(
            "its own run asks the listing nothing"
        )),
        Some(4),
        "a run of this pane's own is already in listing rows"
    );
    assert_eq!(
        owed_row(&Owing::Pair(pick.clone()), &lanes, |_| Some(3)),
        Some(4),
        "the paired instruction, as a listing row"
    );
    assert_eq!(
        owed_row(&Owing::Pair(pick), &lanes, |_| None),
        None,
        "lines that produced no instruction in this listing"
    );
}

/// **A planted address lands on the instruction holding it**, which is the last one at or
/// below it; an address before the first is dropped.
#[test]
fn a_planted_address_lands_on_the_instruction_holding_it() {
    let instructions = [instruction(0x10, Vec::new()), instruction(0x18, Vec::new())];
    assert_eq!(planted_index(&instructions, 0x10), Some(0));
    assert_eq!(
        planted_index(&instructions, 0x14),
        Some(0),
        "inside the first"
    );
    assert_eq!(planted_index(&instructions, 0x18), Some(1));
    assert_eq!(planted_index(&instructions, 0x20), Some(1), "past the last");
    assert_eq!(
        planted_index(&instructions, 0x0),
        None,
        "before the listing's first instruction"
    );
    assert_eq!(planted_index(&[], 0x10), None, "a listing with no rows");
}

/// **One answer for what a press on a link does**, over the two modifiers and the listing
/// the link is drawn in. A press that is no door is left to the row -- and a label with
/// Ctrl held is a door that opens nothing, so the row does not get that one either.
#[test]
fn what_a_press_on_a_link_opens_turns_on_alt_ctrl_and_the_listing() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();
    let target = Arc::new(SymbolData {
        name: "_ZN3add3addE".to_owned(),
        demangled: Some("add".to_owned()),
        address: 0x100,
        section: None,
        size: 0,
    });
    let in_code = Door::Symbol {
        object: object.clone(),
        target: target.clone(),
        code_tab: true,
    };
    let alone = Door::Symbol {
        object: object.clone(),
        target: target.clone(),
        code_tab: false,
    };
    let address = Door::Address {
        object: object.clone(),
        address: 0x2000,
    };
    let label = Door::Label {
        symbol: Symbol {
            object,
            data: target.clone(),
        },
    };
    let row = Door::Row {
        to: 12,
        at: Some(LinePos {
            file: Arc::from("now.c"),
            line: 3,
        }),
    };

    // Alt shuts every door in every pane: the press is a selection this time.
    for door in [&in_code, &alone, &address, &label, &row] {
        assert!(door.opens(true, false).is_none());
        assert!(door.opens(true, true).is_none());
    }
    assert!(
        label.opens(false, false).is_none(),
        "a label without Ctrl has nowhere to go, and the press is the row's"
    );
    assert!(
        matches!(label.opens(false, true), Some(Opens::Nothing)),
        "a label with Ctrl is a door the row must not also get"
    );

    // In the unified view a plain press moves down the listing already on screen, at the
    // address that listing draws the target at; Ctrl opens the symbol on its own.
    assert!(matches!(
        in_code.opens(false, false),
        Some(Opens::InCode { placed, .. }) if placed == target.placed(target.address)
    ));
    assert!(matches!(
        in_code.opens(false, true),
        Some(Opens::Symbol { .. })
    ));
    // In a symbol's own listing there is nowhere to move to, with Ctrl or without.
    assert!(matches!(
        alone.opens(false, false),
        Some(Opens::Symbol { .. })
    ));
    assert!(matches!(
        alone.opens(false, true),
        Some(Opens::Symbol { .. })
    ));

    assert!(matches!(
        address.opens(false, false),
        Some(Opens::Code {
            address: 0x2000,
            ..
        })
    ));
    assert!(matches!(
        row.opens(false, false),
        Some(Opens::Row { to: 12, at: Some(at) }) if at.line == 3
    ));
}

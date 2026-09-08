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
    let data = AsmData::of(studied, assembly, None, 0, 0, width, false);

    in_runtime(|| {
        for index in 0..data.assembly.instructions.len() {
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

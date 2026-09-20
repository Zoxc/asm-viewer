//! What an instruction row draws, held against what the same row copies.

use super::*;
use analysis::{BranchEdge, Extent};
use freya_testing::TestingRunner;
use std::path::Path;

/// An app that draws nothing. [`in_runtime`] wants a runtime, not a tree.
fn nothing() -> impl IntoElement {
    rect()
}

/// Run `body` inside a freya runtime, in the root scope: asking for a colour or a font
/// needs the runtime, both being global states, and making a `State` needs the scope.
fn in_runtime(body: impl FnOnce()) {
    TestingRunner::new(
        nothing,
        (10., 10.).into(),
        |runner| runner.provide_root_context(body),
        1.,
    );
}

/// What a link reaches for, built the way the root and a list build it: every root
/// context made here, since `roots` is the one list of them, and a [`Listing`] with no
/// box behind it, nothing here being pressed.
fn link_states() -> LinkStates {
    let roots = roots(None, &Settings::default());
    LinkStates {
        ctrl: roots.keys.ctrl,
        doors: roots.doors,
        listing: Listing::detached(),
    }
}

fn span(text: &str, kind: SpanKind) -> (String, SpanKind) {
    (text.to_owned(), kind)
}

/// One instruction with nothing named on it, for a case below to name what it is about.
fn instruction(address: u64, format: Vec<(String, SpanKind)>) -> Instruction {
    Instruction {
        address: SectionAddress::new(address),
        bytes: Vec::new(),
        format,
        operand: None,
    }
}

/// The row's drawn text: its spans, joined.
fn drawn(text: &Text) -> Line {
    Line::text(
        text.spans
            .iter()
            .map(|span| span.text.to_string())
            .collect::<String>(),
    )
}

/// A listing holding one instruction of every kind a row draws differently: no link at
/// all, a relocation's name in the operand it applies to, one appended because the
/// formatter offered no operand to put it in, a branch this listing has the row for, a
/// bare target, and a name inside a memory operand, which is the case with a tail. Every
/// one of them is padded to the operand column, which the copy trims where it ends a row.
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
    instructions[1].operand = Some(Operand::SymbolName {
        symbol: target.clone(),
        span: Some(2),
    });
    // The same name with no operand to go in: appended, as `asm_line` appends it.
    instructions[2].operand = Some(Operand::SymbolName {
        symbol: target.clone(),
        span: None,
    });
    // A branch back to the first row, which this listing has.
    instructions[3].operand = Some(Operand::Branch {
        address: SectionAddress::new(0x00),
        span: 2,
    });
    // A target with no name and no row here.
    instructions[4].operand = Some(Operand::Call {
        address: SectionAddress::new(0x2000),
        span: 2,
    });
    // The name inside a memory operand, so the row has a tail.
    instructions[5].operand = Some(Operand::SymbolName {
        symbol: target,
        span: Some(4),
    });

    // The bytes the six rows above cover, eight apiece.
    Assembly {
        instructions,
        edges: vec![BranchEdge { from: 3, to: 0 }],
        undecodable: None,
        range: SectionAddress::new(0)..SectionAddress::new(0x30),
        extent: Extent {
            bytes: 0x30,
            capped: false,
        },
    }
}

/// **What a row copies is what it draws.** The spans a row is drawn from and the line it
/// is copied as come out of one walk ([`pieces`]), so a column into the drawn text lands
/// on the character that same column of the copy lands on -- and a sweep selects what the
/// reader swept over. The copy trims the formatter's padding after the last span, and
/// that is all it leaves out. Each row's link goes where its operand says.
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
    let data = AsmData::of(studied, In::Alone { subject: None }).expect("the fixture decodes");

    in_runtime(|| {
        // What a link is handed, as the list hands it: the root's own contexts, and a
        // listing with no box behind it -- nothing here presses one.
        let states = link_states();
        for index in 0..data.assembly().instructions.len() {
            let text = instruction_text(&data, index, RowChars::default(), None, &states);
            let (drawn, copied) = (drawn(&text).to_string(), text.line.to_string());
            assert_eq!(
                drawn.trim_end(),
                copied,
                "row {index} draws what it does not copy"
            );
        }
    });

    // The door each row's link is: none for a row that names nothing, the symbol for a
    // relocation wherever its name goes, the row a branch lands on where this listing has
    // it, and the object's code at the address a call goes to.
    let doors = (0..assembly.instructions.len())
        .map(|index| match door_of(&data, index) {
            None => "none",
            Some(Door::Symbol { .. }) => "symbol",
            Some(Door::Row { .. }) => "row",
            Some(Door::Address { .. }) => "address",
            Some(Door::Label { .. }) => "label",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        doors,
        ["none", "symbol", "symbol", "row", "address", "symbol"]
    );
}

/// **Every link a row can have is one run of its text**: a relocation's name in the
/// operand it applies to, a branch's displacement, a call's target, and -- the case with
/// no span of its own -- a name the formatter offered no operand for, appended behind one
/// space.
#[test]
fn every_kind_of_link_is_one_run_of_the_text() {
    let target = Arc::new(SymbolData::new(
        "_ZN3add3addE".to_owned(),
        Some("add".to_owned()),
        SectionAddress::new(0x100),
        None,
        0,
    ));
    let assembly = listing(target);
    let lines = assembly
        .instructions
        .iter()
        .map(text_of)
        .collect::<Vec<_>>();
    // The text the link's columns cover, which is what a press on it follows.
    let linked_text = |(line, columns): &(Line, Option<Range<usize>>)| {
        columns
            .clone()
            .map(|columns| line.slice(columns.start, columns.end).to_owned())
    };

    // The padding after the last span is not text, and a row with no link has no run.
    assert_eq!(lines[0].0.to_string(), "mov     rax, rbx");
    assert_eq!(linked_text(&lines[0]), None);

    assert_eq!(lines[1].0.to_string(), "call    add");
    assert_eq!(linked_text(&lines[1]).as_deref(), Some("add"));
    // Appended: every span, then the space, then the name.
    assert_eq!(lines[2].0.to_string(), "nop    add");
    assert_eq!(linked_text(&lines[2]).as_deref(), Some("add"));
    assert_eq!(lines[3].0.to_string(), "jmp     0x0");
    assert_eq!(linked_text(&lines[3]).as_deref(), Some("0x0"));
    assert_eq!(lines[4].0.to_string(), "call    0x2000");
    assert_eq!(linked_text(&lines[4]).as_deref(), Some("0x2000"));
    // The name inside a memory operand: the tail is drawn after it.
    assert_eq!(lines[5].0.to_string(), "mov     rax, [add]");
    assert_eq!(linked_text(&lines[5]).as_deref(), Some("add"));
}

/// A symbol's name is the file's to say, and it can end in whitespace or be nothing but
/// it. The copy trims the padding after the last span, which takes that whitespace with
/// it, and the link's columns never run past what is left: a name of spaces alone is no
/// link at all, and one ending in them is the name before them.
#[test]
fn a_link_named_in_whitespace_stays_inside_the_line() {
    let named = |name: &str| {
        let mut nop = instruction(
            0,
            vec![
                span("nop", SpanKind::Mnemonic),
                span("   ", SpanKind::Other),
            ],
        );
        nop.operand = Some(Operand::SymbolName {
            symbol: Arc::new(SymbolData::new(
                name.to_owned(),
                None,
                SectionAddress::new(0),
                None,
                0,
            )),
            span: None,
        });
        text_of(&nop)
    };

    let (line, link) = named("   ");
    assert_eq!(line.to_string(), "nop");
    assert_eq!(link, None, "a name of spaces alone is a link past the line");

    let (line, link) = named("f  ");
    assert_eq!(line.to_string(), "nop    f");
    assert_eq!(link, Some(7..8), "the link runs past the trimmed line");
}

/// **A reveal the pane owes goes to a listing row, and the pair's is an instruction.**
/// The pane's own run is already counted in listing rows; the other pane's is the first
/// instruction its lines produced, which the separators above it make a row of.
#[test]
fn the_row_a_reveal_goes_to_is_the_runs_own_or_the_paired_instructions() {
    // A listing of six instructions with a branch landing on the fourth, which is the
    // one separator: instruction 3 is drawn at row 4.
    let lanes = Lanes::new(&[BranchEdge { from: 0, to: 3 }], 6);
    let pick = line_pick(Arc::from("now.c"), 7, None, Owed::default()).expect("line 7 is a row");

    assert_eq!(
        owed_listing_row(&Owing::Own(4..=6), &lanes, |_| panic!(
            "its own run asks the listing nothing"
        )),
        Some(4),
        "a run of this pane's own is already in listing rows"
    );
    assert_eq!(
        owed_listing_row(&Owing::Pair(pick.clone()), &lanes, |_| Some(3)),
        Some(4),
        "the paired instruction, as a listing row"
    );
    assert_eq!(
        owed_listing_row(&Owing::Pair(pick), &lanes, |_| None),
        None,
        "lines that produced no instruction in this listing"
    );
}

/// **A planted address lands on the instruction holding it**, which is the last one at or
/// below it; an address before the first is dropped.
#[test]
fn a_planted_address_lands_on_the_instruction_holding_it() {
    let instructions = [instruction(0x10, Vec::new()), instruction(0x18, Vec::new())];
    assert_eq!(
        planted_index(&instructions, SectionAddress::new(0x10)),
        Some(0)
    );
    assert_eq!(
        planted_index(&instructions, SectionAddress::new(0x14)),
        Some(0),
        "inside the first"
    );
    assert_eq!(
        planted_index(&instructions, SectionAddress::new(0x18)),
        Some(1)
    );
    assert_eq!(
        planted_index(&instructions, SectionAddress::new(0x20)),
        Some(1),
        "past the last"
    );
    assert_eq!(
        planted_index(&instructions, SectionAddress::new(0x0)),
        None,
        "before the listing's first instruction"
    );
    assert_eq!(
        planted_index(&[], SectionAddress::new(0x10)),
        None,
        "a listing with no rows"
    );
}

/// **One answer for what a press on a link does**, over Ctrl and the listing the link is
/// drawn in, and the whole of it: where the target opens as well as which target it is.
/// A press that is no door is left to the row, which a label is without Ctrl. Alt is the
/// row's, which asks it before any door.
///
/// The reach is the half `Opens::go` used to decide, from a second read of the same key,
/// which left "Ctrl opens a tab of its own" untestable here.
#[test]
fn what_a_press_on_a_link_opens_turns_on_ctrl_and_the_listing() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();
    let target = Arc::new(SymbolData::new(
        "_ZN3add3addE".to_owned(),
        Some("add".to_owned()),
        SectionAddress::new(0x100),
        None,
        0,
    ));
    let symbol = Symbol {
        object: object.clone(),
        data: target.clone(),
    };
    let in_code = Door::Symbol {
        symbol: symbol.clone(),
        code_tab: true,
    };
    let alone = Door::Symbol {
        symbol: symbol.clone(),
        code_tab: false,
    };
    let address = Door::Address {
        object: object.clone(),
        address: PlacedAddress::new(0x2000),
    };
    let label = Door::Label {
        symbol: symbol.clone(),
    };
    let row = Door::Row {
        to: 12,
        at: Some(LinePos {
            file: Arc::from("now.c"),
            line: 3,
        }),
    };

    assert!(
        label.opens(false).is_none(),
        "a label without Ctrl has nowhere to go, and the press is the row's"
    );
    assert!(
        label.opens(true) == Some(Opens::Symbol(symbol.clone(), Reach::NewTab)),
        "a label with Ctrl opens its symbol in a tab of its own"
    );

    // In the unified view a plain press moves down the listing already on screen, at the
    // address that listing draws the target at; Ctrl opens the symbol on its own.
    assert!(matches!(
        in_code.opens(false),
        Some(Opens::InCode { placed, .. }) if placed == target.placed(target.address)
    ));
    // With Ctrl either door is the symbol on its own, in a tab that stays: the two
    // listings differ in where a plain press goes and not in what Ctrl means.
    for door in [&in_code, &alone] {
        assert!(
            door.opens(true) == Some(Opens::Symbol(symbol.clone(), Reach::NewTab)),
            "Ctrl on a name opens the symbol in a tab of its own"
        );
    }
    // In a symbol's own listing there is nowhere to move to, so a plain press follows the
    // name in place, the way a browser follows a link.
    assert!(alone.opens(false) == Some(Opens::Symbol(symbol.clone(), Reach::InPlace)));

    // A bare address is the object's code, in place or in a tab of its own by the same
    // rule.
    assert!(
        address.opens(false)
            == Some(Opens::Code {
                object: object.clone(),
                address: PlacedAddress::new(0x2000),
                reach: Reach::InPlace,
            })
    );
    assert!(
        address.opens(true)
            == Some(Opens::Code {
                object,
                address: PlacedAddress::new(0x2000),
                reach: Reach::NewTab,
            })
    );

    // A row of the listing on screen is no document at all, so no modifier moves it.
    let to_the_row = Some(Opens::Row {
        to: 12,
        at: Some(LinePos {
            file: Arc::from("now.c"),
            line: 3,
        }),
    });
    assert!(row.opens(false) == to_the_row);
    assert!(row.opens(true) == to_the_row);
}

/// **Every field of the three listing props takes part in its comparison.** A field left
/// out is a listing that stops re-rendering when only that field changed. The three
/// derive `PartialEq`, which is what the derive buys, and `AsmData` is where it counts:
/// it holds the worker's `Studied` whole so that a field added there reaches the rows
/// without a builder to thread it through.
#[test]
fn every_field_of_a_listing_prop_is_compared() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();
    let mut symbols = object.symbols_sorted.iter().map(|data| Symbol {
        object: object.clone(),
        data: data.clone(),
    });
    let symbol = symbols.next().expect("the fixture holds a symbol");
    let other = symbols.next().expect("the fixture holds a second symbol");

    let assembly = Arc::new(listing(symbol.data.clone()));
    let studied = |symbol: &Symbol| Studied::with_assembly(symbol.clone(), Some(assembly.clone()));
    let data =
        AsmData::of(studied(&symbol), In::Alone { subject: None }).expect("the fixture decodes");

    assert!(data == data.clone(), "a listing differs from itself");
    for (field, changed) in [
        // Two analyses of one symbol are two lane layouts, which `Studied` compares by
        // pointer.
        (
            "studied",
            AsmData {
                studied: studied(&symbol),
                ..data.clone()
            },
        ),
        (
            "subject",
            AsmData {
                listing: In::Alone {
                    subject: Some(Subject {
                        tab: DocId::unfiled(),
                        file: Arc::from("main.rs"),
                    }),
                },
                ..data.clone()
            },
        ),
        // Which listing it is, with nothing else to tell the two apart: the gutter width
        // and the doors a row gets both follow it.
        (
            "listing",
            AsmData {
                listing: In::Code {
                    base: 0,
                    bias: Bias::NONE,
                },
                ..data.clone()
            },
        ),
        (
            "base",
            AsmData {
                listing: In::Code {
                    base: 1,
                    bias: Bias::NONE,
                },
                ..data.clone()
            },
        ),
        (
            "bias",
            AsmData {
                listing: In::Code {
                    base: 0,
                    bias: Bias::new(1),
                },
                ..data.clone()
            },
        ),
    ] {
        assert!(data != changed, "AsmData ignores {field}");
    }

    let mut docs = Docs::default();
    let tab = docs.open(Document::Symbol(symbol.clone()));
    let elsewhere = docs.open(Document::Symbol(other.clone()));

    let rows = InstructionList {
        tab,
        data: data.clone(),
        asked: Ask::Symbol(symbol.clone()),
        answered: None,
    };
    assert!(rows == rows.clone(), "a list of rows differs from itself");
    for (field, changed) in [
        (
            "tab",
            InstructionList {
                tab: elsewhere,
                ..rows.clone()
            },
        ),
        (
            "data",
            InstructionList {
                data: AsmData {
                    listing: In::Code {
                        base: 1,
                        bias: Bias::NONE,
                    },
                    ..data.clone()
                },
                ..rows.clone()
            },
        ),
        (
            "asked",
            InstructionList {
                asked: Ask::Symbol(other.clone()),
                ..rows.clone()
            },
        ),
        (
            "answered",
            InstructionList {
                answered: Some(Ask::Symbol(other.clone())),
                ..rows.clone()
            },
        ),
    ] {
        assert!(rows != changed, "InstructionList ignores {field}");
    }

    let pane = AssemblyPane {
        tab,
        document: Document::Symbol(symbol),
    };
    assert!(pane == pane.clone(), "a pane differs from itself");
    for (field, changed) in [
        (
            "tab",
            AssemblyPane {
                tab: elsewhere,
                ..pane.clone()
            },
        ),
        (
            "document",
            AssemblyPane {
                document: Document::Symbol(other),
                ..pane.clone()
            },
        ),
    ] {
        assert!(pane != changed, "AssemblyPane ignores {field}");
    }
}

/// **Which of the two spaces a row's address is in is the listing's**, and the two are
/// different numbers wherever a section was placed anywhere but 0: a symbol read alone
/// draws the addresses the file states, and the same symbol among its neighbours draws
/// them with its section's bias added.
///
/// Over `line_fixture_split.o`, whose three `.text.<name>` sections the parse lays out at
/// 0x10, 0x30 and 0x50, so the answers cannot agree by accident. Every other test of this
/// runs over the flat fixture, where the one `.text` has no bias and the two spaces are
/// the same numbers — which is exactly the case a wrong answer survives.
#[test]
fn a_rows_address_is_its_listings_space_and_the_two_differ_where_a_section_was_placed() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/analysis/tests/fixtures/line_fixture_split.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();
    let symbol = object
        .symbols_sorted
        .iter()
        .find(|data| data.name == "twice")
        .map(|data| Symbol {
            object: object.clone(),
            data: data.clone(),
        })
        .expect("the fixture holds twice");

    let section = symbol.data.section.clone().expect("twice is in a section");
    let bias = section.bias();
    assert_ne!(bias, Bias::NONE, "the fixture's sections are unplaced");

    let studied = Studied::new(symbol.clone());
    let own = studied
        .assembly
        .as_ref()
        .expect("twice decodes")
        .instructions[0]
        .address;

    let alone = AsmData::of(studied.clone(), In::Alone { subject: None }).expect("it decodes");
    let among = AsmData::of(studied, In::Code { base: 0, bias }).expect("it decodes");

    assert_eq!(alone.drawn_address(0), Address::Local(own));
    assert_eq!(among.drawn_address(0), Address::Placed(own.placed(bias)));
    assert_ne!(
        alone.drawn_address(0).get(),
        among.drawn_address(0).get(),
        "the two spaces answered one number, so this fixture pins nothing"
    );
}

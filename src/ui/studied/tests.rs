//! What [`Analyzed`] does with an answer, and what it asks for.

use super::*;

/// A real object, so the symbols compare by the pointers the rules are written in terms
/// of. Parsed per call, so two calls are two distinct objects exactly as two members of
/// an archive are.
fn fixture() -> Arc<Object> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    analysis::open_files(vec![path])
        .first()
        .expect("the fixture parses")
        .clone()
}

fn symbol_of(object: &Arc<Object>) -> Symbol {
    Symbol {
        object: object.clone(),
        data: object
            .symbols_sorted
            .first()
            .expect("the fixture has a symbol")
            .clone(),
    }
}

fn source(file: &str, line: u32) -> Ask {
    Ask::Source {
        at: LinePos {
            file: Arc::from(file),
            line,
        },
        chosen: None,
    }
}

/// A listing of `symbol`, worked out for `ask`.
fn shown(ask: &Ask, symbol: &Symbol) -> Shown {
    Shown {
        ask: ask.clone(),
        studied: Studied::new(symbol.clone()),
    }
}

#[test]
fn an_answer_to_a_question_the_reader_has_clicked_past_is_not_taken() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let asked = Ask::Symbol(symbol.clone());
    let since = source("other.c", 3);

    let mut state = Analyzed {
        pending: Some(Pending::asked(since.clone())),
        ..Analyzed::default()
    };
    let studied = Some(Studied::new(symbol));
    assert!(!state.take(asked, studied, Some(&since), &[object]));
    assert!(state.shown.is_none(), "the listing is not put up");
    assert!(state.answered.is_none(), "and nothing is recorded of it");
}

#[test]
fn an_answer_out_of_a_binary_closed_since_it_was_asked_for_is_not_drawn() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let ask = source("line_fixture.c", 3);

    let mut state = Analyzed {
        pending: Some(Pending::asked(ask.clone())),
        ..Analyzed::default()
    };
    let studied = Some(Studied::new(symbol));
    // Closed while the worker ran: the answer holds the whole file's bytes.
    assert!(state.take(ask.clone(), studied, Some(&ask), &[]));
    assert!(state.shown.is_none());
    assert!(
        state.answered == Some(ask),
        "the question was still answered"
    );
    assert!(state.pending.is_none(), "and nothing is still waiting");
}

#[test]
fn a_line_that_named_no_symbol_leaves_this_tabs_listing_up_and_takes_another_tabs_down() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let up = source("line_fixture.c", 3);
    let nothing = source("line_fixture.c", 4);

    let mut state = Analyzed {
        shown: Some(shown(&up, &symbol)),
        ..Analyzed::default()
    };
    assert!(state.take(nothing.clone(), None, Some(&nothing), &[object.clone()]));
    assert!(
        state.shown.is_some(),
        "a line of the file the tab is showing leaves its listing up"
    );

    let elsewhere = source("other.c", 1);
    assert!(state.take(elsewhere.clone(), None, Some(&elsewhere), &[object]));
    assert!(
        state.shown.is_none(),
        "a line of another file does not leave that listing under its tab"
    );
}

#[test]
fn an_answer_the_ask_had_already_settled_writes_nothing() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let ask = Ask::Symbol(symbol.clone());
    // The listing that is up, and the answer -- one value, as they are when the effect
    // retagged what was already in hand while this answer was on its way.
    let studied = Studied::new(symbol);

    let mut state = Analyzed {
        shown: Some(Shown {
            ask: ask.clone(),
            studied: studied.clone(),
        }),
        answered: Some(ask.clone()),
        ..Analyzed::default()
    };
    assert!(
        !state.take(ask.clone(), Some(studied), Some(&ask), &[object]),
        "an answer that leaves everything as it was costs no render"
    );
}

#[test]
fn a_listing_that_answers_the_new_question_is_retagged_rather_than_asked_for_again() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let line = source("line_fixture.c", 3);
    let outright = Ask::Symbol(symbol.clone());

    let mut state = Analyzed {
        shown: Some(shown(&line, &symbol)),
        answered: Some(line),
        ..Analyzed::default()
    };
    let visits = Visits::default();
    let (question, _) = state.asked(Some(&outright), &[object], &visits);
    assert!(question.is_none(), "nothing is asked for a listing in hand");
    assert!(
        state.shown.expect("the listing is kept").ask == outright,
        "and it is retagged with the question it now answers"
    );
}

#[test]
fn a_listing_whose_binary_has_closed_is_asked_for_again_out_of_what_is_left() {
    let object = fixture();
    let symbol = symbol_of(&object);
    let line = source("line_fixture.c", 3);

    let mut state = Analyzed {
        shown: Some(shown(&line, &symbol)),
        answered: Some(line.clone()),
        ..Analyzed::default()
    };
    let visits = Visits::default();
    let (question, _) = state.asked(Some(&line), &[], &visits);
    assert!(
        matches!(question, Some(Question::Resolve { .. })),
        "the question is asked again"
    );
    assert!(
        state.shown.is_none(),
        "and the closed file's listing is gone"
    );
    assert!(state.waiting() == Some(&line));
}

#[test]
fn a_question_already_on_its_way_is_not_asked_twice() {
    let object = fixture();
    let ask = Ask::Symbol(symbol_of(&object));
    let mut state = Analyzed {
        pending: Some(Pending::asked(ask.clone())),
        ..Analyzed::default()
    };
    let visits = Visits::default();
    assert!(
        matches!(state.asked(Some(&ask), &[object], &visits), (None, false)),
        "and asking it again changes nothing"
    );
}

#[test]
fn a_place_with_no_listing_leaves_nothing_waiting() {
    let object = fixture();
    let ask = Ask::Symbol(symbol_of(&object));
    let mut state = Analyzed {
        pending: Some(Pending { ask, slow: true }),
        ..Analyzed::default()
    };
    let visits = Visits::default();
    assert!(state.asked(None, &[object], &visits).0.is_none());
    assert!(state.pending.is_none(), "the wait outlived the question");
}

/// The base is where the two index spaces meet, and one function crosses it: `touching`
/// and `places` share it, so the gutter cannot light an edge the pane will not scroll
/// to. A run opening above this listing starts at its first row, one ending above it
/// holds nothing of it, and a separator opening the run is inside it while one closing
/// the run is not -- all of it with a base under the rows.
#[test]
fn a_run_of_listing_rows_crosses_into_this_listings_instructions_by_its_base() {
    // Branches 0 -> 2 and 3 -> 5 over nine instructions. A separator is drawn above
    // each landing row, so instruction 5 is drawn at row 7 of this listing and the
    // separator opening its block at row 6.
    let edges = [
        analysis::BranchEdge { from: 0, to: 2 },
        analysis::BranchEdge { from: 3, to: 5 },
    ];
    let studied = Studied {
        symbol: symbol_of(&fixture()),
        assembly: None,
        lanes: Arc::new(Lanes::new(&edges, 9)),
        lines: SymbolLines {
            info: None,
            file: None,
            line: None,
        },
    };
    assert_eq!(studied.lanes.row_of(5), 7);

    // The listing row the section view draws this stretch's first instruction at.
    let base = 100;
    let lit = |rows: RangeInclusive<usize>| studied.touching(rows, base);

    // The run ends above this listing, so it holds none of it. The end is checked and
    // not clamped: clamped, it would light the branch off the first instruction.
    assert!(lit(90..=99).is_empty());
    // The run opens above and reaches in. The start saturates to the first row, and
    // that same branch is lit.
    assert_eq!(lit(90..=101).len(), 1);
    // A separator closing the run belongs to the instruction below it, which is then
    // outside: instruction 5 is not in rows 5..=6 and its branch stays dark.
    assert!(lit(105..=106).is_empty());
    // Opening the run, that instruction is inside it.
    assert_eq!(lit(106..=107).len(), 1);
    // A run that is the separator alone holds no instruction at all.
    assert!(lit(106..=106).is_empty());
    // And the symbol read alone, with no base, answers the same run shifted down: the
    // two panes ask one function.
    assert_eq!(lit(106..=107), studied.touching(6..=7, 0));
}

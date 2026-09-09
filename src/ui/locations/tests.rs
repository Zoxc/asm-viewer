//! What [`Located`] does with an answer.

use super::*;

impl Found {
    /// The symbols it answered with, and `None` where it was a question for the server.
    /// The tests' way of asking; the panel matches on `what` instead.
    pub(crate) fn symbols(&self) -> Option<&Shared<Symbol>> {
        match &self.what {
            What::Symbols(symbols) => Some(symbols),
            What::Places(_) => None,
        }
    }

    /// The places it answered with, and `None` where it was a question about symbols.
    /// The tests' way of asking, as `symbols` is.
    pub(crate) fn places(&self) -> Option<&references::References> {
        match &self.what {
            What::Places(places) => Some(places),
            What::Symbols(_) => None,
        }
    }
}

fn fixture() -> Arc<Object> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    analysis::open_files(vec![path])
        .first()
        .expect("the fixture parses")
        .clone()
}

fn symbols_of(object: &Arc<Object>) -> Vec<Symbol> {
    object
        .symbols_sorted
        .iter()
        .map(|data| Symbol {
            object: object.clone(),
            data: data.clone(),
        })
        .collect()
}

fn query(file: &str, line: u32) -> Query {
    Query::line(LinePos {
        file: Arc::from(file),
        line,
    })
}

#[test]
fn an_answer_to_the_question_before_this_one_is_not_taken() {
    let object = fixture();
    let asked = query("line_fixture.c", 9);
    let mut state = Located {
        asked: Some(asked),
        ..Located::default()
    };

    let stale = query("line_fixture.c", 3);
    assert!(!state.take(stale, symbols_of(&object), &[object]));
    assert!(state.found.is_none(), "the panel is not given what it left");
}

#[test]
fn a_binary_closed_while_the_worker_ran_is_not_put_back_by_its_answer() {
    let object = fixture();
    let asked = query("line_fixture.c", 3);
    let mut state = Located {
        asked: Some(asked.clone()),
        ..Located::default()
    };

    let symbols = symbols_of(&object);
    assert!(!symbols.is_empty(), "the fixture answers with something");
    // Answered over an object the reader has closed since.
    assert!(state.take(asked, symbols, &[]));
    let found = state.found.expect("an empty answer is an answer");
    assert!(found.symbols().expect("symbols were asked for").is_empty());
}

#[test]
fn a_close_drops_the_symbols_it_takes_with_it_and_a_load_writes_nothing() {
    let object = fixture();
    let asked = query("line_fixture.c", 3);
    let mut state = Located {
        asked: Some(asked.clone()),
        ..Located::default()
    };
    assert!(state.take(asked, symbols_of(&object), &[object.clone()]));

    assert!(
        !state.retain_open(&[object.clone()]),
        "nothing went, so nothing is written"
    );
    assert!(state.retain_open(&[]), "the closed file's symbols went");
    let found = state.found.expect("the question is still answered");
    assert!(found.symbols().expect("symbols").is_empty());
}

/// **A location row drives the tab the question was asked from, and only while that tab
/// is still open on that file.** The drive is written under the place the tab is at and
/// not under the file, a stop the trail does not hold being a drive nothing reads.
#[test]
fn which_door_a_location_row_presses_through() {
    let file: Arc<str> = Arc::from("now.c");
    let at = LinePos {
        file: file.clone(),
        line: 9,
    };
    let mut docs = Docs::default();
    let id = docs.open(Document::Source(file.clone()));
    // The tab has since been stepped to a line of its own, which is the place it is at.
    docs.push(id, Stop::on(file.clone(), 4));
    let subject = Some((id, file.clone()));

    assert!(
        matches!(chosen(&docs, None, subject.clone()), Chosen::Alone),
        "an answer naming no line opens the symbol alone"
    );
    match chosen(&docs, Some(at.clone()), subject.clone()) {
        Chosen::Driving { entry, at: line } => {
            assert!(entry.0 == id);
            assert!(
                entry.1 == Stop::on(file.clone(), 4),
                "the place the tab is at, not the file"
            );
            assert_eq!(
                line.line, 9,
                "driven from the line the question was asked on"
            );
        }
        _ => panic!("the tab the question was asked from is still open on the file"),
    }

    docs.close(id);
    assert!(
        matches!(chosen(&docs, Some(at.clone()), subject), Chosen::Landing(_)),
        "a tab that has been closed drives nothing"
    );

    let mut docs = Docs::default();
    let elsewhere = docs.open(Document::Source(Arc::from("before.h")));
    assert!(
        matches!(
            chosen(&docs, Some(at), Some((elsewhere, file))),
            Chosen::Landing(_)
        ),
        "a tab that has moved off the file drives nothing"
    );
}

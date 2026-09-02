use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use analysis::{Architecture, BinaryFormat, Object, ObjectData, SymbolData};

use super::*;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

/// `landing` is asked *before* anything is removed, so each of these passes the whole
/// list and the predicate that is about to thin it.
fn shut(items: &[&str], showing: &str, closing: &[&str]) -> Option<String> {
    let open = strings(items);
    let closing = strings(closing);
    landing(&open, Some(&showing.to_string()), |open| {
        closing.contains(open)
    })
}

#[test]
fn landing_moves_to_the_tab_on_its_right() {
    assert_eq!(shut(&["a", "b", "c"], "b", &["b"]), Some("c".to_owned()));
}

#[test]
fn landing_on_the_last_tab_moves_to_the_one_on_its_left() {
    assert_eq!(shut(&["a", "b", "c"], "c", &["c"]), Some("b".to_owned()));
}

#[test]
fn landing_with_nothing_left_is_nothing() {
    assert_eq!(shut(&["a"], "a", &["a"]), None);
}

/// The bulk case: the reader ends up where closing the one tab by hand would have
/// put them, whether the tabs around it went with it or not.
#[test]
fn landing_after_several_is_the_first_survivor_after_the_shown_one() {
    assert_eq!(
        shut(&["a", "b", "c", "d"], "b", &["a", "b", "c"]),
        Some("d".to_owned())
    );
}

/// A tab that survives is its own answer, which is what lets a caller ask without
/// first working out whether what is on screen is going anywhere.
#[test]
fn a_surviving_shown_tab_is_its_own_landing() {
    assert_eq!(shut(&["a", "b", "c"], "b", &["c"]), Some("b".to_owned()));
}

/// Nothing on screen is a state the app is really in — an empty strip — and a close
/// asked for from it still has to say which tab is left. It lands on the last
/// survivor, exactly where a tab that is not open at all lands.
#[test]
fn landing_from_nothing_shown_is_the_last_survivor() {
    let open = strings(&["a", "b", "c"]);
    assert_eq!(
        landing(&open, None, |open| open == "b"),
        Some("c".to_owned())
    );
    let missing = "z".to_owned();
    assert_eq!(
        landing(&open, Some(&missing), |open| open == "b"),
        Some("c".to_owned())
    );
}

fn positions(at: &[(&str, usize)]) -> Positions<String> {
    let mut positions = Positions::default();
    for (tab, row) in at {
        positions.remember((*tab).to_owned(), *row);
    }
    positions
}

#[test]
fn a_tab_never_seen_is_at_no_row_and_opens_at_the_top() {
    let positions = positions(&[]);
    assert_eq!(positions.at(&"a".to_owned()), None);
    assert_eq!(positions.row(&"a".to_owned(), 100), 0);
}

#[test]
fn a_remembered_row_comes_back() {
    let positions = positions(&[("a", 12), ("b", 40)]);
    assert_eq!(positions.at(&"a".to_owned()), Some(12));
    assert_eq!(positions.row(&"b".to_owned(), 100), 40);
}

#[test]
fn remembering_a_tab_twice_replaces_its_row() {
    let mut positions = positions(&[("a", 12)]);
    positions.remember("a".to_owned(), 13);
    assert_eq!(positions.at(&"a".to_owned()), Some(13));
    assert_eq!(positions.at.len(), 1);
}

/// The listing has shrunk under the position — a rebuilt binary, a source file edited
/// since it was read — so the row is the last one there now rather than one past the
/// end, and an empty listing is row 0 rather than an underflow.
#[test]
fn a_row_past_the_end_clamps_to_the_last_one() {
    let positions = positions(&[("a", 900)]);
    assert_eq!(positions.row(&"a".to_owned(), 100), 99);
    assert_eq!(positions.row(&"a".to_owned(), 0), 0);
    // And `at` still says what was remembered: only the answer given to a pane is
    // clamped, because only a pane knows what it is holding.
    assert_eq!(positions.at(&"a".to_owned()), Some(900));
}

#[test]
fn forgetting_a_tab_leaves_the_others() {
    let mut positions = positions(&[("a", 1), ("b", 2)]);
    positions.forget(&"a".to_owned());
    assert_eq!(positions.at(&"a".to_owned()), None);
    assert_eq!(positions.at(&"b".to_owned()), Some(2));
    // And forgetting one that was never there is not an error.
    positions.forget(&"c".to_owned());
    assert_eq!(positions.at(&"b".to_owned()), Some(2));
}

#[test]
fn a_closing_binary_forgets_every_position_into_it() {
    let mut positions = positions(&[("lib.a:one", 1), ("some.dll:two", 2), ("lib.a:three", 3)]);
    positions.forgetting(|tab| !tab.starts_with("lib.a:"));
    assert_eq!(positions.at(&"some.dll:two".to_owned()), Some(2));
    assert_eq!(positions.at(&"lib.a:one".to_owned()), None);
    assert_eq!(positions.at(&"lib.a:three".to_owned()), None);
}

/// A source-driven tab, which is the only kind [`Driven`] ever holds. Compared by its
/// text, so two of these naming one file are one tab.
fn source(file: &str) -> Document {
    Document::Source(file.into())
}

#[test]
fn a_tab_nothing_was_clicked_in_is_driven_from_nothing() {
    let driven = Driven::default();
    assert_eq!(driven.line(&source("main.rs")), None);
}

#[test]
fn a_driven_line_comes_back_and_a_second_click_replaces_it() {
    let mut driven = Driven::default();
    driven.remember(source("main.rs"), 42);
    driven.remember(source("lib.rs"), 7);
    assert_eq!(driven.line(&source("main.rs")), Some(42));

    driven.remember(source("main.rs"), 43);
    assert_eq!(driven.line(&source("main.rs")), Some(43));
    assert_eq!(driven.line(&source("lib.rs")), Some(7));
}

#[test]
fn closing_a_tab_forgets_what_it_was_driven_from() {
    let mut driven = Driven::default();
    driven.remember(source("main.rs"), 42);
    driven.remember(source("lib.rs"), 7);

    driven.forget(&source("main.rs"));
    assert_eq!(driven.line(&source("main.rs")), None);
    assert_eq!(driven.line(&source("lib.rs")), Some(7));
    // And forgetting a tab that was never driven is not an error.
    driven.forget(&source("other.rs"));
    assert_eq!(driven.line(&source("lib.rs")), Some(7));
}

/// A bare symbol in a bare object at `path`: only what [`Driven::release`] looks at.
fn symbol(path: &str, name: &str) -> Symbol {
    let data = Arc::new(SymbolData {
        name: name.to_owned(),
        demangled: None,
        address: 0,
        section: None,
        size: 0,
    });
    let object = Arc::new(Object {
        path: PathBuf::from(path),
        name: path.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted: vec![data.clone()],
        sections: Vec::new(),
        data: ObjectData::from(b"bytes".as_slice()),
        dwarf: Default::default(),
    });
    Symbol { object, data }
}

#[test]
fn a_choice_comes_back_and_outlives_the_next_line() {
    let mut driven = Driven::default();
    let (first, second) = (symbol("lib.a", "f<u8>"), symbol("lib.a", "f<u16>"));
    assert!(driven.choice(&source("main.rs")).is_none());

    driven.remember(source("main.rs"), 42);
    driven.choose(source("main.rs"), first.clone());
    assert!(driven.choice(&source("main.rs")) == Some(first.clone()));
    // Reading down the function inside the instantiation picked is the point of
    // picking one, so the next line keeps the choice.
    driven.remember(source("main.rs"), 43);
    assert!(driven.choice(&source("main.rs")) == Some(first.clone()));
    // A second choice replaces the first, and belongs to its own tab.
    driven.choose(source("main.rs"), second.clone());
    assert!(driven.choice(&source("main.rs")) == Some(second));
    assert!(driven.choice(&source("lib.rs")).is_none());
}

#[test]
fn closing_a_tab_forgets_its_choice_and_a_closing_binary_releases_the_choices_into_it() {
    let mut driven = Driven::default();
    driven.remember(source("main.rs"), 42);
    driven.choose(source("main.rs"), symbol("lib.a", "f"));
    driven.remember(source("lib.rs"), 7);
    driven.choose(source("lib.rs"), symbol("other.o", "g"));

    driven.forget(&source("main.rs"));
    assert!(driven.choice(&source("main.rs")).is_none());
    assert!(driven.choice(&source("lib.rs")).is_some());

    // The file closing takes the choice into it and leaves the line: the tab stands, and
    // its next ask answers out of whatever is still open.
    driven.release(&PathBuf::from("other.o"));
    assert!(driven.choice(&source("lib.rs")).is_none());
    assert_eq!(driven.line(&source("lib.rs")), Some(7));
}

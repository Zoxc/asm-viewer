use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use analysis::{Architecture, BinaryFormat, Object, ObjectData, SymbolData};

use super::*;
use crate::docs::Docs;
use crate::history::Stop;
use crate::project::Document;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

/// A strip of document tabs, ids taken in the order given so a test can name them, with
/// the table they came out of for a test that opens one more.
fn strip(count: u32) -> (Strip, Vec<Tab>, Docs) {
    let mut docs = Docs::default();
    let tabs: Vec<Tab> = (0..count)
        .map(|nth| {
            let file: Arc<str> = Arc::from(format!("{nth}.rs").as_str());
            Tab::Document(docs.open(Document::Source(file)))
        })
        .collect();
    let mut strip = Strip::default();
    for tab in &tabs {
        strip.push(*tab);
    }
    (strip, tabs, docs)
}

#[test]
fn a_new_tab_opens_beside_the_tab_on_screen() {
    let (mut strip, tabs, _docs) = strip(3);
    let page = Tab::Page(Page::Settings);
    strip.raise(tabs[0]);
    strip.show(page);
    assert_eq!(strip.tabs(), [tabs[0], page, tabs[1], tabs[2]]);
    assert_eq!(strip.active(), Some(page));
}

/// A page is a tab like any other, so a document opened over one lands beside it and not
/// at the end of the bar.
#[test]
fn a_tab_opened_over_a_page_lands_beside_it() {
    let (mut strip, tabs, mut docs) = strip(2);
    let page = Tab::Page(Page::Project);
    strip.show(page);
    let opened = Tab::Document(docs.open(Document::Source(Arc::from("opened.rs"))));
    strip.show(opened);
    assert_eq!(strip.tabs(), [tabs[0], tabs[1], page, opened]);
}

/// Showing a tab that is already open is a raise and never a second copy of it.
#[test]
fn showing_an_open_tab_only_raises_it() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.show(tabs[0]);
    assert_eq!(strip.tabs(), tabs);
    assert_eq!(strip.active(), Some(tabs[0]));
    assert!(!strip.raise(Tab::Page(Page::Settings)), "a tab not open");
    assert_eq!(strip.active(), Some(tabs[0]));
}

#[test]
fn closing_the_tab_on_screen_lands_on_its_neighbour() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.raise(tabs[1]);
    assert_eq!(strip.close(|tab| *tab == tabs[1]), [tabs[1]]);
    assert_eq!(strip.tabs(), [tabs[0], tabs[2]]);
    assert_eq!(strip.active(), Some(tabs[2]));
}

/// The tab on screen is left where it is when it is not one of the ones closing: the
/// write would notify whether or not it changed anything.
#[test]
fn closing_around_the_tab_on_screen_leaves_it_showing() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.raise(tabs[1]);
    assert_eq!(strip.close(|tab| *tab != tabs[1]), [tabs[0], tabs[2]]);
    assert_eq!(strip.tabs(), [tabs[1]]);
    assert_eq!(strip.active(), Some(tabs[1]));
}

/// Nothing matched is answered as nothing removed, which is what lets a caller tell it
/// from "nothing is left" and leave a live tab's positions alone.
#[test]
fn closing_nothing_removes_nothing() {
    let (mut strip, tabs, _docs) = strip(2);
    assert!(strip
        .close(|tab| *tab == Tab::Page(Page::Project))
        .is_empty());
    assert_eq!(strip.tabs(), tabs);
    assert_eq!(strip.active(), Some(tabs[1]));
}

#[test]
fn closing_the_last_tab_shows_nothing() {
    let (mut strip, tabs, _docs) = strip(1);
    assert_eq!(strip.close(|_| true), tabs);
    assert!(strip.tabs().is_empty());
    assert_eq!(strip.active(), None);
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
    positions.forgetting(|tab| tab != "a");
    assert_eq!(positions.at(&"a".to_owned()), None);
    assert_eq!(positions.at(&"b".to_owned()), Some(2));
    // And forgetting one that was never there is not an error.
    positions.forgetting(|tab| tab != "c");
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

/// A value that is not `Copy` -- the runs a place keeps are not -- comes back whole,
/// is replaced whole, and is forgotten with its tab like a row is.
#[test]
fn a_value_that_is_not_copy_comes_back_whole() {
    let mut kept: Positions<String, Vec<usize>> = Positions::default();
    assert_eq!(kept.at(&"a".to_owned()), None);
    kept.remember("a".to_owned(), vec![3, 4]);
    kept.remember("b".to_owned(), Vec::new());
    assert_eq!(kept.at(&"a".to_owned()), Some(vec![3, 4]));
    // Seen with nothing in it is not the same as never seen.
    assert_eq!(kept.at(&"b".to_owned()), Some(Vec::new()));

    kept.remember("a".to_owned(), vec![5]);
    assert_eq!(kept.at(&"a".to_owned()), Some(vec![5]));
    assert_eq!(kept.at.len(), 2);

    kept.forgetting(|tab| tab != "a");
    assert_eq!(kept.at(&"a".to_owned()), None);
    assert_eq!(kept.at(&"b".to_owned()), Some(Vec::new()));
}

/// An entry of a source-driven tab's trail, which is the only kind [`Driven`] ever
/// holds, on the tab `nth` ids from the first. The document is compared by its text, so
/// two of these naming one file on one tab are one entry.
fn source_on(nth: u32, file: &str) -> Entry {
    let mut docs = Docs::default();
    let document = Document::Source(file.into());
    let mut id = docs.open(document.clone());
    for _ in 0..nth {
        id = docs.open(document.clone());
    }
    (id, Stop::whole(document))
}

/// The same, on the first tab.
fn source(file: &str) -> Entry {
    source_on(0, file)
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

/// A line is kept per tab *and* entry: two tabs on one file are driven from two lines,
/// and closing one tab forgets every entry of it and nothing of the other.
#[test]
fn closing_a_tab_forgets_what_its_entries_were_driven_from() {
    let mut driven = Driven::default();
    driven.remember(source("main.rs"), 42);
    driven.remember(source_on(1, "main.rs"), 43);
    driven.remember(source("lib.rs"), 7);
    assert_eq!(driven.line(&source("main.rs")), Some(42));
    assert_eq!(driven.line(&source_on(1, "main.rs")), Some(43));

    driven.forget_tab(source("main.rs").0);
    assert_eq!(driven.line(&source("main.rs")), None);
    assert_eq!(driven.line(&source("lib.rs")), None);
    assert_eq!(driven.line(&source_on(1, "main.rs")), Some(43));
    // And forgetting a tab that was never driven is not an error.
    driven.forget_tab(source_on(2, "other.rs").0);
    assert_eq!(driven.line(&source_on(1, "main.rs")), Some(43));
}

/// A closing binary takes entries off surviving trails, and the lines kept by them go
/// too, whichever tab they were on.
#[test]
fn a_closing_binary_forgets_the_lines_of_the_entries_it_takes() {
    let mut driven = Driven::default();
    driven.remember(source("main.rs"), 42);
    driven.remember(source_on(1, "lib.rs"), 7);
    driven.forgetting(|(_, stop)| stop.document != Document::Source("main.rs".into()));
    assert_eq!(driven.line(&source("main.rs")), None);
    assert_eq!(driven.line(&source_on(1, "lib.rs")), Some(7));
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
        debug_info: Default::default(),
        by_address: Default::default(),
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
    let other = source_on(1, "lib.rs");
    driven.remember(source("main.rs"), 42);
    driven.choose(source("main.rs"), symbol("lib.a", "f"));
    driven.remember(other.clone(), 7);
    driven.choose(other.clone(), symbol("other.o", "g"));

    driven.forget_tab(source("main.rs").0);
    assert!(driven.choice(&source("main.rs")).is_none());
    assert!(driven.choice(&other).is_some());

    // The file closing takes the choice into it and leaves the line: the tab stands, and
    // its next ask answers out of whatever is still open.
    driven.release(&PathBuf::from("other.o"));
    assert!(driven.choice(&other).is_none());
    assert_eq!(driven.line(&other), Some(7));
}

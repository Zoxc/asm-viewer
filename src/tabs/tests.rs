use std::sync::Arc;

use super::*;
use crate::docs::Docs;
use crate::document::Document;

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
        strip.show(*tab);
    }
    (strip, tabs, docs)
}

/// A session is written with the stored name and not the title, so the two are pinned
/// apart: every page round-trips, and a name this build does not have is dropped rather
/// than guessed at.
///
/// The Debug page round-trips like the rest. It is kept out of the menu unless it is asked
/// for, and a reader with one open has asked: a restart puts it back.
#[test]
fn a_page_round_trips_through_the_name_a_session_holds() {
    for page in Page::ALL {
        assert_eq!(Page::from_stored(page.stored()), Some(page));
    }
    assert_eq!(Page::from_stored("Settings"), None);
    assert_eq!(Page::from_stored("terminal"), None);
    // Every page has a name of its own, so no two tabs answer to one word.
    let names: std::collections::HashSet<&str> = Page::ALL.iter().map(|p| p.stored()).collect();
    assert_eq!(names.len(), Page::ALL.len());
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
}

/// `would_raise` is what `raise_tab` asks before it writes (`src/ui/documents.rs`), and
/// it says no twice over: for a tab that is not open, and for the one already on screen.
/// A tab that is not open is not shown either.
#[test]
fn raising_says_when_it_would_change_what_is_on_screen() {
    let (mut strip, tabs, _docs) = strip(3);
    let page = Tab::Page(Page::Settings);
    strip.raise(tabs[0]);
    assert!(!strip.would_raise(tabs[0]), "already on screen");
    assert!(!strip.would_raise(page), "not open");
    assert!(strip.would_raise(tabs[1]));
    strip.raise(page);
    assert_eq!(strip.active(), Some(tabs[0]));
}

/// `has_others` is what a chip's menu asks before it draws the close-others row
/// (`src/ui/strip.rs`). It asks about the tabs and not about the one handed in: any open
/// tab that is not that one answers it true. So the only tab open has no company, a tab
/// that is not open has company in every tab there is, and an empty strip answers false
/// whatever it is asked about.
#[test]
fn a_tab_has_company_when_another_tab_is_open() {
    let page = Tab::Page(Page::Settings);
    let (three, tabs, _docs) = strip(3);
    assert!(three.has_others(tabs[0]));

    let (one, only, _docs) = strip(1);
    assert!(!one.has_others(only[0]), "the only tab open");
    assert!(one.has_others(page), "not open, and one other tab is");

    assert!(!Strip::default().has_others(page), "nothing is open");
}

#[test]
fn closing_the_tab_on_screen_lands_on_its_neighbour() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.raise(tabs[1]);
    assert!(strip.close(|tab| *tab == tabs[1]));
    assert_eq!(strip.tabs(), [tabs[0], tabs[2]]);
    assert_eq!(strip.active(), Some(tabs[2]));
}

/// The tab on screen is left where it is when it is not one of the ones closing: the
/// write would notify whether or not it changed anything.
#[test]
fn closing_around_the_tab_on_screen_leaves_it_showing() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.raise(tabs[1]);
    assert!(strip.close(|tab| *tab != tabs[1]));
    assert_eq!(strip.tabs(), [tabs[1]]);
    assert_eq!(strip.active(), Some(tabs[1]));
}

/// Nothing matched is answered as nothing removed, which is what lets a caller tell it
/// from "nothing is left" and leave a live tab's positions alone.
#[test]
fn closing_nothing_removes_nothing() {
    let (mut strip, tabs, _docs) = strip(2);
    assert!(!strip.close(|tab| *tab == Tab::Page(Page::Project)));
    assert_eq!(strip.tabs(), tabs);
    assert_eq!(strip.active(), Some(tabs[1]));
}

#[test]
fn closing_the_last_tab_shows_nothing() {
    let (mut strip, _tabs, _docs) = strip(1);
    assert!(strip.close(|_| true));
    assert!(strip.tabs().is_empty());
    assert_eq!(strip.active(), None);
}

#[test]
fn a_tab_moves_to_where_the_tab_it_was_dropped_on_is() {
    let (mut strip, tabs, _docs) = strip(4);
    strip.move_to(tabs[3], 1);
    assert_eq!(strip.tabs(), [tabs[0], tabs[3], tabs[1], tabs[2]]);
    strip.move_to(tabs[3], 4);
    assert_eq!(strip.tabs(), [tabs[0], tabs[1], tabs[2], tabs[3]]);
    assert_eq!(strip.active(), Some(tabs[3]), "a move is not an opening");
}

/// A chip dragged while its document is closed under it carries an id that stands for
/// nothing, and a drop of one must not put it back.
#[test]
fn moving_a_tab_the_strip_does_not_hold_puts_nothing_there() {
    let (mut strip, tabs, _docs) = strip(2);
    strip.move_to(Tab::Page(Page::Scratchpad), 0);
    assert_eq!(strip.tabs(), tabs);
}

/// `landing` is asked *before* anything is removed, so each of these passes the whole
/// list and the predicate that is about to thin it. The tabs are named by where they sit
/// in `open`.
fn shut(open: &[Tab], showing: usize, closing: &[usize]) -> Option<Tab> {
    let closing: Vec<Tab> = closing.iter().map(|&nth| open[nth]).collect();
    landing(open, Some(open[showing]), |tab| closing.contains(tab))
}

#[test]
fn landing_moves_to_the_tab_on_its_right() {
    let (_, tabs, _docs) = strip(3);
    assert_eq!(shut(&tabs, 1, &[1]), Some(tabs[2]));
}

#[test]
fn landing_on_the_last_tab_moves_to_the_one_on_its_left() {
    let (_, tabs, _docs) = strip(3);
    assert_eq!(shut(&tabs, 2, &[2]), Some(tabs[1]));
}

#[test]
fn landing_with_nothing_left_is_nothing() {
    let (_, tabs, _docs) = strip(1);
    assert_eq!(shut(&tabs, 0, &[0]), None);
}

/// The bulk case: the reader ends up where closing the one tab by hand would have
/// put them, whether the tabs around it went with it or not.
#[test]
fn landing_after_several_is_the_first_survivor_after_the_shown_one() {
    let (_, tabs, _docs) = strip(4);
    assert_eq!(shut(&tabs, 1, &[0, 1, 2]), Some(tabs[3]));
}

/// A tab that survives is its own answer, which is what lets a caller ask without
/// first working out whether what is on screen is going anywhere.
#[test]
fn a_surviving_shown_tab_is_its_own_landing() {
    let (_, tabs, _docs) = strip(3);
    assert_eq!(shut(&tabs, 1, &[2]), Some(tabs[1]));
}

/// Nothing on screen is a state the app is really in — an empty strip — and a close
/// asked for from it still has to say which tab is left. It lands on the last
/// survivor, exactly where a tab that is not open at all lands.
#[test]
fn landing_from_nothing_shown_is_the_last_survivor() {
    let (_, tabs, mut docs) = strip(3);
    assert_eq!(landing(&tabs, None, |tab| *tab == tabs[1]), Some(tabs[2]));
    let missing = Tab::Document(docs.open(Document::Source(Arc::from("z.rs"))));
    assert_eq!(
        landing(&tabs, Some(missing), |tab| *tab == tabs[1]),
        Some(tabs[2])
    );
}

/// A step along the bar **wraps at both ends**, so Ctrl+Tab held down walks the whole bar
/// round rather than stopping at the last chip.
#[test]
fn a_step_along_the_bar_wraps_at_both_ends() {
    let (mut strip, tabs, _docs) = strip(3);
    strip.raise(tabs[0]);
    assert_eq!(strip.stepped(Along::Next), Some(tabs[1]));
    assert_eq!(strip.stepped(Along::Previous), Some(tabs[2]));

    strip.raise(tabs[2]);
    assert_eq!(strip.stepped(Along::Next), Some(tabs[0]));
    assert_eq!(strip.stepped(Along::Previous), Some(tabs[1]));
}

/// One tab steps to itself, either way, which is a raise of the tab already on screen and
/// so nothing at all. An empty bar has nothing to step from.
#[test]
fn a_bar_of_one_steps_to_itself_and_an_empty_one_nowhere() {
    let (mut strip, tabs, _docs) = strip(1);
    assert_eq!(strip.stepped(Along::Next), Some(tabs[0]));
    assert_eq!(strip.stepped(Along::Previous), Some(tabs[0]));

    strip.close(|_| true);
    assert_eq!(strip.stepped(Along::Next), None);
    assert_eq!(strip.stepped(Along::Previous), None);
}

/// The numbers count from 1, and **9 is the last however many there are** -- three tabs
/// and Ctrl+9 is the third. A number the bar is too short for names nothing.
#[test]
fn the_ninth_tab_is_the_last_however_many_there_are() {
    let (three, tabs, _docs) = strip(3);
    assert_eq!(three.nth(1), Some(tabs[0]));
    assert_eq!(three.nth(3), Some(tabs[2]));
    assert_eq!(three.nth(9), Some(tabs[2]));
    assert_eq!(three.nth(4), None);
    assert_eq!(three.nth(0), None);

    // Long enough for the digit to be a number in its own right, and 9 is still the last.
    let (many, tabs, _docs) = strip(12);
    assert_eq!(many.nth(8), Some(tabs[7]));
    assert_eq!(many.nth(9), Some(tabs[11]));
}

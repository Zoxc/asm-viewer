use super::*;

// --- the landing rule, on its own -------------------------------------
//
// The same rules as the `Tabs` tests above, asked of the free function directly, so
// that "a close lands on the right-hand neighbour" keeps its coverage wherever the
// tabs are being kept. `landing` is asked *before* anything is removed, so each of
// these passes the whole list and the predicate that is about to thin it.

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

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

#[test]
fn landing_after_closing_the_newest_several_moves_left() {
    assert_eq!(
        shut(&["a", "b", "c", "d"], "c", &["c", "d"]),
        Some("b".to_owned())
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

/// `landing` removes nothing, so it cannot tell "nothing was closed" from "nothing
/// is left" — it answers the tab that is still there, and the distinction is
/// [`Tabs::close_all`]'s to draw because it is the one doing the removing.
#[test]
fn landing_that_closes_nothing_answers_the_shown_tab() {
    let open = strings(&["a", "b"]);
    assert_eq!(
        landing(&open, Some(&"a".to_owned()), |_| false),
        Some("a".to_owned())
    );
    assert_eq!(
        landing::<String>(&[], Some(&"a".to_owned()), |_| false),
        None
    );
}

// --- where each tab was left ------------------------------------------

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
    // Replaced, not appended: the second answer is the only answer.
    assert_eq!(positions.at.len(), 1);
}

/// The listing has shrunk under the position — a rebuilt binary, a source file edited
/// since it was read — so the row is the last one there now rather than one past the
/// end. The end is where the reader was; the top is not.
#[test]
fn a_row_past_the_end_clamps_to_the_last_one() {
    let positions = positions(&[("a", 900)]);
    assert_eq!(positions.row(&"a".to_owned(), 100), 99);
    // And `at` still says what was remembered: only the answer given to a pane is
    // clamped, because only a pane knows what it is holding.
    assert_eq!(positions.at(&"a".to_owned()), Some(900));
}

#[test]
fn an_empty_listing_has_no_row_but_the_first() {
    let positions = positions(&[("a", 900)]);
    assert_eq!(positions.row(&"a".to_owned(), 0), 0);
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

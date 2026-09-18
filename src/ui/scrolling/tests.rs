//! Where a run of [`use_kept_position`] moves the view.

use super::*;

/// Two tabs, each showing a file of its own. A source stop needs no object, so this stays
/// object-free.
fn tabs() -> (Entry, Entry) {
    let mut docs = Docs::default();
    let (first, second) = (
        Stop::whole(Document::Source(Arc::from("a.rs"))),
        Stop::whole(Document::Source(Arc::from("b.rs"))),
    );
    let a = (docs.open(first.clone()), first);
    let b = (docs.open(second.clone()), second);
    (a, b)
}

/// The decision, with the owner answered as whether it is `owner` -- an [`Entry`] holds a
/// [`Document`], which has nothing to print itself with.
fn moved(
    holding: Option<&Entry>,
    tab: &Entry,
    known: bool,
    back_to: TopRow,
    opening: Option<usize>,
    owner: Option<&Entry>,
) -> (bool, Option<Move>) {
    let (whose, moving) = kept_move(holding, tab, known, back_to, opening);
    (whose.as_ref() == owner, moving)
}

#[test]
fn a_pane_still_on_the_tab_it_is_scrolled_for_moves_nothing() {
    let (a, _) = tabs();
    assert_eq!(
        moved(Some(&a), &a, true, TopRow::at(12), Some(3), Some(&a)),
        (true, None)
    );
    assert_eq!(
        moved(Some(&a), &a, false, TopRow::at(0), Some(3), Some(&a)),
        (true, None),
        "and nothing remembered is no reason to open it again"
    );
}

#[test]
fn a_switch_writes_the_offset_down_under_the_tab_being_left() {
    let (a, b) = tabs();
    assert_eq!(
        moved(Some(&a), &b, true, TopRow::at(12), Some(3), Some(&a)),
        (true, Some(Move::Place(TopRow::at(12)))),
        "the arriving tab goes back to its own row"
    );
    assert_eq!(
        moved(Some(&a), &b, false, TopRow::at(0), Some(3), Some(&a)),
        (true, Some(Move::Open(3))),
        "a tab seen for the first time opens where the pane says"
    );
    assert_eq!(
        moved(Some(&a), &b, false, TopRow::at(0), None, Some(&a)),
        (true, Some(Move::Place(TopRow::default()))),
        "and at the top with nothing to say, which still moves"
    );
}

#[test]
fn the_first_run_writes_nothing_down_for_a_tab_it_is_putting_back() {
    let (a, _) = tabs();
    assert_eq!(
        moved(None, &a, true, TopRow::at(12), Some(3), None),
        (true, Some(Move::Place(TopRow::at(12)))),
        "a remount or a restored session: nothing to write down"
    );
    assert_eq!(
        moved(None, &a, false, TopRow::at(0), Some(3), Some(&a)),
        (true, Some(Move::Open(3))),
        "the ordinary first open of a tab"
    );
    assert_eq!(
        moved(None, &a, false, TopRow::at(0), None, Some(&a)),
        (true, None),
        "a `0` is left alone rather than scrolled to"
    );
}

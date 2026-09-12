//! The two rules the effects here spend: where a run of [`use_kept_position`] moves the
//! view, and when a source run is dropped.

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
    back_to: usize,
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
        moved(Some(&a), &a, true, 12, Some(3), Some(&a)),
        (true, None)
    );
    assert_eq!(
        moved(Some(&a), &a, false, 0, Some(3), Some(&a)),
        (true, None),
        "and nothing remembered is no reason to open it again"
    );
}

#[test]
fn a_switch_writes_the_offset_down_under_the_tab_being_left() {
    let (a, b) = tabs();
    assert_eq!(
        moved(Some(&a), &b, true, 12, Some(3), Some(&a)),
        (true, Some(Move::Place(12))),
        "the arriving tab goes back to its own row"
    );
    assert_eq!(
        moved(Some(&a), &b, false, 0, Some(3), Some(&a)),
        (true, Some(Move::Open(3))),
        "a tab seen for the first time opens where the pane says"
    );
    assert_eq!(
        moved(Some(&a), &b, false, 0, None, Some(&a)),
        (true, Some(Move::Place(0))),
        "and at the top with nothing to say, which still moves"
    );
}

#[test]
fn the_first_run_writes_nothing_down_for_a_tab_it_is_putting_back() {
    let (a, _) = tabs();
    assert_eq!(
        moved(None, &a, true, 12, Some(3), None),
        (true, Some(Move::Place(12))),
        "a remount or a restored session: nothing to write down"
    );
    assert_eq!(
        moved(None, &a, false, 0, Some(3), Some(&a)),
        (true, Some(Move::Open(3))),
        "the ordinary first open of a tab"
    );
    assert_eq!(
        moved(None, &a, false, 0, None, Some(&a)),
        (true, None),
        "a `0` is left alone rather than scrolled to"
    );
}

#[test]
fn a_source_run_is_dropped_only_where_the_pane_moved_off_the_file_it_is_in() {
    let (a, b) = tabs();
    let (now, before) = (Arc::<str>::from("now.c"), Arc::<str>::from("before.h"));

    assert!(
        moved_off(Some(&a), Some(&a), Some(&now), Some(&before), Some(&now)),
        "one place, another file, and the run is in the file left"
    );
    assert!(
        !moved_off(Some(&a), Some(&b), Some(&now), Some(&before), Some(&now)),
        "a switch of place, which `use_land` owns whole"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&now), Some(&now)),
        "the same file still on screen"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&before), Some(&before)),
        "a landing's run, planted in the file arriving"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&before), None),
        "no run to drop"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), None, Some(&now), Some(&now)),
        "a pane that was drawing no file was on no run's file"
    );
}

/// **A row and a line convert one way each**, and line 0 is no row: debug info writes it
/// for instructions belonging to no source line, and a stored place or a compiler's own
/// message can state it too, so it reaches the panes from a file. Read as row 0 it would
/// pick out the first line of the file, which is somewhere the reader was never sent.
#[test]
fn line_0_is_no_row_of_any_file() {
    let file: Arc<str> = Arc::from("now.c");

    assert_eq!(LinePos::of_row(file.clone(), 0).line, 1);
    assert_eq!(LinePos::of_row(file.clone(), 39).row(), Some(39));
    assert_eq!(LinePos::line_of(usize::MAX), u32::MAX, "no file has it");

    assert_eq!(LinePos::row_of(0), None);
    assert_eq!(LinePos::row_of(1), Some(0));
    let none = LinePos {
        file: file.clone(),
        line: 0,
    };
    assert_eq!(none.row(), None);

    // And the run a door onto a line makes, which is where the two used to disagree: the
    // panes read the line as a row, so line 0 is a door onto nowhere.
    assert!(line_pick(file.clone(), 0, None, Owed::BOTH).is_none());
    let first = line_pick(file.clone(), 1, None, Owed::BOTH).expect("line 1 is row 0");
    assert!(
        !first.is_line(&file, 0),
        "the run on the first row is not a run of line 0"
    );
    assert!(first.is_line(&file, 1));
}

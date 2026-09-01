use super::*;

#[test]
fn a_press_selects_the_row_it_is_on_and_starts_a_drag() {
    let selection = RowSelection::at(7);
    assert_eq!(selection.rows(), 7..=7);
    assert!(selection.contains(7));
    assert!(!selection.contains(6));
    assert_eq!(selection.dragged_to(9).rows(), 7..=9);
}

#[test]
fn a_run_reads_the_same_way_round_whichever_way_it_was_picked() {
    let down = RowSelection::at(4).extended(9);
    let up = RowSelection::at(9).extended(4);
    assert_eq!(down.rows(), 4..=9);
    assert_eq!(up.rows(), 4..=9);
}

#[test]
fn extending_moves_the_lead_and_leaves_the_anchor() {
    // Two shift-clicks either side of the anchor, which is what makes the second one
    // a correction of the first rather than a run from wherever the first one ended.
    let selection = RowSelection::at(5).extended(9).extended(2);
    assert_eq!(selection.rows(), 2..=5);
}

#[test]
fn a_row_entered_with_the_button_up_is_not_a_drag() {
    let selection = RowSelection::at(3).released();
    assert_eq!(selection.dragged_to(20).rows(), 3..=3);
    // Shift-clicking still extends it: letting go ended the gesture, not the run.
    // And it arms the next drag, so sweeping on from a shift-click carries on.
    let extended = selection.extended(20);
    assert_eq!(extended.rows(), 3..=20);
    assert_eq!(extended.dragged_to(25).rows(), 3..=25);
}

#[test]
fn releasing_keeps_the_rows() {
    let selection = RowSelection::at(3).dragged_to(6);
    assert_eq!(selection.released().rows(), 3..=6);
}

#[test]
fn select_all_covers_the_listing_and_an_empty_listing_selects_nothing() {
    let selection = RowSelection::all(3).unwrap();
    assert_eq!(selection.rows(), 0..=2);
    // Not a drag: nothing is under the pointer, so a row entered afterwards must not
    // be swept into it.
    assert_eq!(selection.dragged_to(9).rows(), 0..=2);
    assert_eq!(RowSelection::all(0), None);
}

#[test]
fn text_is_the_rows_in_listing_order() {
    let line = |row: usize| format!("row {row}");
    assert_eq!(
        RowSelection::at(2).extended(4).text(line),
        "row 2\nrow 3\nrow 4"
    );
    // The backwards drag copies forwards.
    assert_eq!(
        RowSelection::at(4).extended(2).text(line),
        "row 2\nrow 3\nrow 4"
    );
    assert_eq!(RowSelection::at(1).text(line), "row 1");
}

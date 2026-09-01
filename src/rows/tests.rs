use super::*;

/// The one-row run a press starts. It is a drag until the button comes up, which is what
/// lets a sweep carry it out.
#[test]
fn a_press_selects_the_row_it_is_on_and_starts_a_drag() {
    let selection = RowSelection {
        anchor: 7,
        lead: 7,
        dragging: true,
    };
    assert_eq!(selection.rows(), 7..=7);
    assert!(selection.contains(7));
    assert!(!selection.contains(6));
    assert_eq!(selection.extended(9).rows(), 7..=9);
}

#[test]
fn extending_moves_the_lead_and_leaves_the_anchor() {
    // Two shift-clicks either side of the anchor, which is what makes the second one a
    // correction of the first rather than a run from wherever the first one ended.
    let selection = RowSelection {
        anchor: 5,
        lead: 5,
        dragging: true,
    }
    .extended(9)
    .extended(2);
    assert_eq!(selection.rows(), 2..=5);
}

/// A shift-click extends a run whose drag is over, and arms the next drag with it — which
/// is what lets holding the button after one and sweeping on carry on from there.
#[test]
fn extending_a_finished_run_arms_the_next_drag() {
    let selection = RowSelection {
        anchor: 3,
        lead: 3,
        dragging: false,
    };

    let extended = selection.extended(20);

    assert_eq!(extended.rows(), 3..=20);
    assert!(extended.dragging);
}

/// The rows come out in listing order whichever way round they were picked, so a drag
/// that went upwards reads the same as one that went down.
#[test]
fn the_rows_are_in_listing_order_whichever_way_they_were_picked() {
    let forwards = RowSelection {
        anchor: 2,
        lead: 4,
        dragging: false,
    };
    let backwards = RowSelection {
        anchor: 4,
        lead: 2,
        dragging: false,
    };

    assert_eq!(forwards.rows(), 2..=4);
    assert_eq!(backwards.rows(), 2..=4);
    assert!(backwards.contains(3));
    assert!(!backwards.contains(5));
}

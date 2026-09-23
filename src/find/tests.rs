use super::*;

/// A hit on one row, for the steps below.
fn hit(row: usize, columns: Range<usize>) -> Hit {
    Hit { row, columns }
}

/// The caret a press leaves: an empty run.
fn caret(row: usize, col: usize) -> CharSelection {
    CharSelection::at(Caret { row, col })
}

/// The run a step leaves on `hit`: its columns picked out, the lead at the end.
fn on(hit: &Hit) -> CharSelection {
    CharSelection::between(
        Caret {
            row: hit.row,
            col: hit.columns.start,
        },
        Caret {
            row: hit.row,
            col: hit.columns.end,
        },
    )
}

/// A step goes to the next hit and wraps at the end; back goes the other way.
#[test]
fn a_step_walks_the_hits_and_wraps() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    assert_eq!(
        step(&hits, Some(0), on(&hits[0]), Direction::Forward),
        Some(1)
    );
    assert_eq!(
        step(&hits, Some(2), on(&hits[2]), Direction::Forward),
        Some(0)
    );
    assert_eq!(step(&hits, Some(1), on(&hits[1]), Direction::Back), Some(0));
    assert_eq!(step(&hits, Some(0), on(&hits[0]), Direction::Back), Some(2));
}

/// With nothing to step to there is nowhere to go, and the bar says so instead.
#[test]
fn a_step_with_no_hits_goes_nowhere() {
    assert_eq!(step(&[], None, caret(0, 0), Direction::Forward), None);
    assert_eq!(step(&[], Some(0), caret(0, 0), Direction::Back), None);
}

/// The **first** step reads the caret, so a find starts from where the reader is looking
/// rather than from the top of the pane.
#[test]
fn the_first_step_starts_from_the_caret() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    let between = caret(2, 0);
    assert_eq!(step(&hits, None, between, Direction::Forward), Some(1));
    assert_eq!(step(&hits, None, between, Direction::Back), Some(0));

    // Past the last hit forward, and before the first backward, each wrap once.
    let past = caret(20, 0);
    assert_eq!(step(&hits, None, past, Direction::Forward), Some(0));
    let before = caret(0, 0);
    assert_eq!(step(&hits, None, before, Direction::Back), Some(2));
}

/// A caret sitting inside a hit -- a click in the middle of one -- steps out of it, not
/// back onto it.
#[test]
fn a_caret_inside_a_hit_steps_out_of_it() {
    let hits = [hit(0, 0..3), hit(4, 1..4)];

    let inside = caret(4, 2);
    assert_eq!(step(&hits, None, inside, Direction::Back), Some(0));
    // Forward, the hit the caret is in is behind it too -- its start is -- so there is
    // nothing ahead and the step wraps.
    assert_eq!(step(&hits, None, inside, Direction::Forward), Some(0));
}

/// A stale index -- the pattern changed and there are fewer hits than there were -- falls
/// back to the caret rather than pointing at nothing.
#[test]
fn a_step_from_an_index_that_is_gone_reads_the_caret() {
    let hits = [hit(0, 0..3), hit(4, 1..4)];

    assert_eq!(
        step(&hits, Some(9), caret(4, 0), Direction::Forward),
        Some(1)
    );
}

/// **A click since the last step is where the next one starts**, not the hit the pane was
/// on: the reader has moved, and the find goes on from where they are. A click at a hit's
/// end is not that hit either -- stepping back from it goes to the hit itself, where
/// stepping back from the hit goes to the one before.
#[test]
fn a_step_after_a_click_starts_from_the_click() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    let below = caret(6, 0);
    assert_eq!(step(&hits, Some(0), below, Direction::Forward), Some(2));
    assert_eq!(step(&hits, Some(0), below, Direction::Back), Some(1));

    let at_end = caret(4, 4);
    assert_eq!(step(&hits, Some(1), at_end, Direction::Back), Some(1));
    assert_eq!(step(&hits, Some(1), on(&hits[1]), Direction::Back), Some(0));
}

/// **A run that is itself a hit is stepped off on the first press**, whichever end its lead
/// is at: a word picked out and then found has the bar's index cleared, and the step
/// must not land back on the word it was opened over.
#[test]
fn a_first_step_from_a_run_on_a_hit_leaves_it() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    // A double-click leaves the lead at the end.
    let rightward = on(&hits[1]);
    assert_eq!(step(&hits, None, rightward, Direction::Back), Some(0));
    assert_eq!(step(&hits, None, rightward, Direction::Forward), Some(2));

    // A sweep leftward leaves it at the start.
    let leftward = CharSelection::between(Caret { row: 4, col: 4 }, Caret { row: 4, col: 1 });
    assert_eq!(step(&hits, None, leftward, Direction::Back), Some(0));
    assert_eq!(step(&hits, None, leftward, Direction::Forward), Some(2));
}

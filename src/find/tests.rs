use super::*;

/// A hit on one row, for the steps below.
fn hit(row: usize, columns: Range<usize>) -> Hit {
    Hit { row, columns }
}

/// A step goes to the next hit and wraps at the end; back goes the other way.
#[test]
fn a_step_walks_the_hits_and_wraps() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];
    let caret = Caret { row: 0, col: 0 };

    assert_eq!(step(&hits, Some(0), caret, Direction::Forward), Some(1));
    assert_eq!(step(&hits, Some(2), caret, Direction::Forward), Some(0));
    assert_eq!(step(&hits, Some(1), caret, Direction::Back), Some(0));
    assert_eq!(step(&hits, Some(0), caret, Direction::Back), Some(2));
}

/// With nothing to step to there is nowhere to go, and the bar says so instead.
#[test]
fn a_step_with_no_hits_goes_nowhere() {
    assert_eq!(
        step(&[], None, Caret { row: 0, col: 0 }, Direction::Forward),
        None
    );
    assert_eq!(
        step(&[], Some(0), Caret { row: 0, col: 0 }, Direction::Back),
        None
    );
}

/// The **first** step reads the caret, so a find starts from where the reader is looking
/// rather than from the top of the pane.
#[test]
fn the_first_step_starts_from_the_caret() {
    let hits = [hit(0, 0..3), hit(4, 1..4), hit(9, 0..2)];

    let between = Caret { row: 2, col: 0 };
    assert_eq!(step(&hits, None, between, Direction::Forward), Some(1));
    assert_eq!(step(&hits, None, between, Direction::Back), Some(0));

    // Past the last hit forward, and before the first backward, each wrap once.
    let past = Caret { row: 20, col: 0 };
    assert_eq!(step(&hits, None, past, Direction::Forward), Some(0));
    let before = Caret { row: 0, col: 0 };
    assert_eq!(step(&hits, None, before, Direction::Back), Some(2));
}

/// A caret sitting inside a hit -- a click in the middle of one -- steps out of it, not
/// back onto it.
#[test]
fn a_caret_inside_a_hit_steps_out_of_it() {
    let hits = [hit(0, 0..3), hit(4, 1..4)];

    let inside = Caret { row: 4, col: 2 };
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
    let caret = Caret { row: 4, col: 0 };

    assert_eq!(step(&hits, Some(9), caret, Direction::Forward), Some(1));
}

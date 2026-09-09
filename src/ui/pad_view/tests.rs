//! What a run of [`use_follow_tail`] makes of where the pane is.

use super::*;

/// Twenty rows ten pixels tall in a view a hundred tall: the bottom is a hundred up, and
/// the newest row begins ninety past the top of it.
fn tail(offset: i32, arrived: bool, following: bool) -> Tail {
    tail_move(offset, 10.0, 100.0, 20, arrived, following)
}

#[test]
fn lines_arriving_move_a_following_pane_and_judge_nothing() {
    assert_eq!(tail(0, true, true), Tail::To(-100));
    assert_eq!(tail(-100, true, true), Tail::Stay, "already at the bottom");
    assert_eq!(tail(0, true, false), Tail::Stay, "scrolled away, and left");
}

#[test]
fn a_scroll_arms_the_follow_where_the_newest_row_is_drawn_at_all() {
    assert_eq!(
        tail(-100, false, false),
        Tail::Follow(true),
        "at the bottom"
    );
    assert_eq!(
        tail(-91, false, false),
        Tail::Follow(true),
        "the newest row's first pixel is drawn"
    );
    assert_eq!(
        tail(-90, false, true),
        Tail::Follow(false),
        "and one pixel short of it is not"
    );
    assert_eq!(tail(0, false, true), Tail::Follow(false), "at the top");
    assert_eq!(
        tail_move(0, 10.0, 100.0, 0, false, false),
        Tail::Follow(true),
        "an empty list is at its own bottom"
    );
}

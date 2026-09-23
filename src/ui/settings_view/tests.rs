use super::*;

/// A press moves to the neighbouring half point in its direction: from a size on the grid by
/// one step, and from a desktop's size between two points to the nearer one that way.
#[test]
fn a_step_lands_on_the_next_half_point_its_way() {
    assert_eq!(stepped(10.5, SIZE_STEP), 11.0);
    assert_eq!(stepped(10.5, -SIZE_STEP), 10.0);
    assert_eq!(stepped(13.75, SIZE_STEP), 14.0);
    assert_eq!(stepped(13.75, -SIZE_STEP), 13.5);
    assert_eq!(stepped(13.25, SIZE_STEP), 13.5);
    assert_eq!(stepped(13.25, -SIZE_STEP), 13.0);
    assert_eq!(stepped(32.0, SIZE_STEP), 32.0);
    assert_eq!(stepped(5.0, -SIZE_STEP), 5.0);
}

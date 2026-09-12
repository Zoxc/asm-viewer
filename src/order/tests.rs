use super::*;

fn order(entries: [u32; 3]) -> Order<u32> {
    let mut order = Order::default();
    for entry in entries {
        order.touch(entry);
    }
    order
}

/// The list *is* the answer to "where was the reader last", so the newest entry is the
/// first and touching the one already there changes nothing -- which is what keeps a
/// startup that reopens the front entry from writing a file.
#[test]
fn touching_puts_an_entry_first_and_says_whether_that_moved_it() {
    let mut list: Order<u32> = Order::default();
    assert!(list.touch(1u32));
    assert!(list.touch(2u32));
    assert_eq!(list.entries(), [2, 1]);
    assert_eq!(list.first(), Some(&2));

    assert!(!list.touch(2u32));
    assert_eq!(list.entries(), [2, 1]);
}

/// One entry per place, never one per visit: an entry already in the list moves out of
/// wherever it was rather than being added again.
#[test]
fn touching_an_entry_again_moves_it_rather_than_repeating_it() {
    let mut list = order([1, 2, 3]);

    assert!(list.touch(1u32));
    assert_eq!(list.entries(), [1, 3, 2]);
    assert_eq!(list.position(&2), Some(2));
}

/// Entries from outside are not trusted: every restore collects one of these, and the
/// occurrence kept is the newest, which is the first.
#[test]
fn collecting_collapses_duplicates_onto_the_newest_occurrence() {
    let list: Order<u32> = [3, 2, 3, 1, 2].into_iter().collect();
    assert_eq!(list.entries(), [3, 2, 1]);
}

#[test]
fn truncating_keeps_the_newest_and_forgetting_takes_one_out() {
    let mut list = order([1, 2, 3]);

    list.truncate(2);
    assert_eq!(list.entries(), [3, 2]);

    assert!(list.forget(&2));
    assert!(!list.forget(&2));
    assert_eq!(list.entries(), [3]);
}

/// The two steps a list with a cap takes on the way in, as one. The cap is the owner's
/// number and is handed in, but the pair is one rule: a touch that moved something cuts
/// what it pushed past the end, and a touch that moved nothing leaves the list alone --
/// which is what lets the caller skip the write as well.
#[test]
fn touching_within_a_cap_drops_the_oldest_past_it() {
    let mut list = order([1, 2, 3]);

    assert!(list.touch_within(4u32, 3));
    assert_eq!(list.entries(), [4, 3, 2]);

    assert!(!list.touch_within(4u32, 1));
    assert_eq!(list.entries(), [4, 3, 2]);
}

/// What a restore from a file goes through: the collapse runs **before** the cut, so a file
/// with every place in it twice comes back as a full list and not as half of one.
#[test]
fn restoring_within_a_cap_collapses_duplicates_before_cutting() {
    let saved = [3u32, 3, 2, 2, 1, 1];

    assert_eq!(Order::restored_within(saved, 3).entries(), [3, 2, 1]);
    assert_eq!(Order::restored_within(saved, 2).entries(), [3, 2]);
    assert!(Order::restored_within(saved, 0).entries().is_empty());
}

/// What a trail does with the entries in front of its cursor: they are abandoned, and the
/// one the cursor was on is left the newest.
#[test]
fn dropping_what_is_newer_leaves_that_entry_first() {
    let mut list = order([1, 2, 3]);

    list.drop_newer_than(1);
    assert_eq!(list.entries(), [2, 1]);

    // Past the end drops the lot rather than panicking.
    list.drop_newer_than(9);
    assert!(list.entries().is_empty());
}

#[test]
fn retaining_keeps_the_order_of_what_it_keeps() {
    let list = order([1, 2, 3]);

    let kept = list.retaining(|entry| *entry != 2);
    assert_eq!(kept.entries(), [3, 1]);
    assert_eq!(list.len(), 3, "retaining changed the original");
}

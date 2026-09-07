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

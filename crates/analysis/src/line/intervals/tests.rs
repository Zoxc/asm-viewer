use super::*;

fn over(intervals: &Intervals<u64, char>, query: Range<u64>) -> Vec<char> {
    intervals.over(query).copied().collect()
}

#[test]
fn overlapping_ranges_are_all_found() {
    let intervals = Intervals::new([(10..20, 'a'), (15..30, 'b'), (40..50, 'c')]);
    assert_eq!(over(&intervals, 18..19), ['b', 'a']);
    assert_eq!(over(&intervals, 20..40), ['b']);
    assert_eq!(over(&intervals, 30..40), [] as [char; 0]);
    assert_eq!(over(&intervals, 0..100), ['c', 'b', 'a']);
}

/// A long range early on keeps the walk going past the short ones after it.
#[test]
fn a_nested_range_does_not_hide_the_one_around_it() {
    let intervals = Intervals::new([(30..32, 'c'), (0..100, 'a'), (10..12, 'b')]);
    assert_eq!(over(&intervals, 50..51), ['a']);
    assert_eq!(over(&intervals, 31..40), ['c', 'a']);
}

#[test]
fn an_empty_query_takes_the_ranges_on_both_sides_of_it() {
    let intervals = Intervals::new([(10..20, 'a'), (20..30, 'b')]);
    assert_eq!(over(&intervals, 15..15), ['a']);
    assert_eq!(over(&intervals, 20..20), [] as [char; 0]);
}

#[test]
fn an_empty_range_is_dropped() {
    // Empty and backwards on purpose, so the lint against one written by mistake is off.
    #[allow(clippy::reversed_empty_ranges)]
    let intervals = Intervals::new([(10..10, 'a'), (12..11, 'b'), (5..6, 'c')]);
    assert_eq!(over(&intervals, 0..100), ['c']);
}

#[test]
fn ranges_with_one_start_keep_their_order() {
    let intervals = Intervals::new([(10..20, 'a'), (10..12, 'b'), (10..30, 'c')]);
    assert_eq!(over(&intervals, 10..11), ['c', 'b', 'a']);
}

#[test]
fn a_query_at_the_top_of_the_address_space() {
    let intervals = Intervals::new([(u64::MAX - 1..u64::MAX, 'a'), (0..1, 'b')]);
    assert_eq!(over(&intervals, u64::MAX - 1..u64::MAX), ['a']);
    assert_eq!(over(&intervals, u64::MAX..u64::MAX), [] as [char; 0]);
    assert_eq!(over(&intervals, 0..u64::MAX), ['a', 'b']);
}

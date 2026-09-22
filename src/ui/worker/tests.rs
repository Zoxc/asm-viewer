use super::*;

/// The last job of each key is kept, every job with no key is kept, and what is kept
/// stays in the order it arrived in.
#[test]
fn newest_by_keeps_the_last_of_each_key_in_arrival_order() {
    let key = |job: &(Option<char>, u32)| job.0;
    let kept = newest_by(
        (Some('a'), 1),
        vec![
            (Some('b'), 2),
            (None, 3),
            (Some('a'), 4),
            (None, 5),
            (Some('b'), 6),
        ]
        .into_iter(),
        key,
    );
    assert_eq!(kept, [(None, 3), (Some('a'), 4), (None, 5), (Some('b'), 6)]);

    // One job is simply itself.
    assert_eq!(
        newest_by((Some('a'), 1), std::iter::empty(), key),
        [(Some('a'), 1)]
    );
}

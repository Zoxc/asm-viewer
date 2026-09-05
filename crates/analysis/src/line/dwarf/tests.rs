//! [`clipped`] on its own, which is the only way to ask it about the row that made it
//! necessary: reaching one through `addr2line` needs a line program that steps backwards,
//! and the step before it is a subtract-with-overflow panic wherever overflow checks are on
//! — which is every build the tests run in.

use super::clipped;

/// A row lying below the query is dropped rather than moved out of the biased space. Taking
/// the bias off first made its end a value near `u64::MAX`, so the row was kept as one
/// covering the rest of the address space.
#[test]
fn a_row_below_the_query_is_dropped_rather_than_wrapped() {
    // A section placed at 0x1000, asked about six bytes of it.
    let bias = 0x1000;
    let query = 0x1000..0x1006;

    assert_eq!(clipped(0x8..0xa, &query, bias), None);
    assert_eq!(clipped(0x8..0x1000, &query, bias), None);

    // One that does reach into the query is clipped to it at both ends.
    assert_eq!(clipped(0x8..0x1004, &query, bias), Some(0..4));
    assert_eq!(clipped(0x1002..0x2000, &query, bias), Some(2..6));
    assert_eq!(clipped(0x1000..0x1006, &query, bias), Some(0..6));
}

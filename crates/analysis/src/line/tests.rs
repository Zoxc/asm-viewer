//! [`RowCollector::push`] on its own, which is the only way to ask it about the row that
//! made its clip-then-unbias order necessary: reaching one through `addr2line` needs a line
//! program that steps backwards, and the step before it is a subtract-with-overflow panic
//! wherever overflow checks are on — which is every build the tests run in.

use super::RowCollector;
use crate::{Bias, PlacedAddress, SectionAddress};
use std::ops::Range;

/// A row as a backend hands it over, in the placed space it reads in.
fn placed(range: Range<u64>) -> Range<PlacedAddress> {
    PlacedAddress::new(range.start)..PlacedAddress::new(range.end)
}

/// The same range once the bias has come off: the section's own.
fn local(range: Range<u64>) -> Range<SectionAddress> {
    SectionAddress::new(range.start)..SectionAddress::new(range.end)
}

/// One row pushed into a collector over `query`, and what it was kept as.
fn pushed(
    row: Range<PlacedAddress>,
    query: &Range<PlacedAddress>,
    bias: Bias,
) -> Option<Range<SectionAddress>> {
    let mut rows = RowCollector::over(query.clone(), bias);
    rows.push(row, None, None, None);
    rows.rows.first().map(|row| row.range.clone())
}

/// A row lying below the query is dropped rather than moved out of the placed space. Taking
/// the bias off first made its end a value near `u64::MAX`, so the row was kept as one
/// covering the rest of the address space.
#[test]
fn a_row_below_the_query_is_dropped_rather_than_wrapped() {
    // A section placed at 0x1000, asked about six bytes of it.
    let bias = Bias::new(0x1000);
    let query = placed(0x1000..0x1006);

    assert_eq!(pushed(placed(0x8..0xa), &query, bias), None);
    assert_eq!(pushed(placed(0x8..0x1000), &query, bias), None);

    // One that does reach into the query is clipped to it at both ends.
    assert_eq!(pushed(placed(0x8..0x1004), &query, bias), Some(local(0..4)));
    assert_eq!(
        pushed(placed(0x1002..0x2000), &query, bias),
        Some(local(2..6))
    );
    assert_eq!(
        pushed(placed(0x1000..0x1006), &query, bias),
        Some(local(0..6))
    );
}

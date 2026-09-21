//! [`RowCollector::push`] on its own, which is the only way to ask it about the row that
//! made its clip-then-unbias order necessary: reaching one through `addr2line` needs a line
//! program that steps backwards, and the step before it is a subtract-with-overflow panic
//! wherever overflow checks are on — which is every build the tests run in. And
//! [`LineInfo::opening`], whose fallbacks are reached through the collector for the same
//! reason: a file no surviving row names is one a query clipped away.

use super::RowCollector;
use crate::{Bias, LineInfo, PlacedAddress, SectionAddress};
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

/// What [`LineInfo::opening`] says, as a pair that is easy to write down.
fn opening(info: &LineInfo, address: u64) -> Option<(&str, Option<u32>)> {
    info.opening(SectionAddress::new(address))
        .map(|(file, line)| (&**file, line))
}

/// The opening file and line come from one row: the one covering the address where it names
/// a file, else the first row that does. A prologue on no line does not answer.
#[test]
fn the_opening_file_and_line_are_one_rows() {
    let mut rows = RowCollector::whole();
    let file = rows.file("a.c", None);
    rows.push(placed(0..4), None, None, None);
    rows.push(placed(4..8), Some(file), Some(7), None);
    let info = rows.finish().unwrap();

    // The prologue names no file, so the first row that does answers both.
    assert_eq!(opening(&info, 0), Some(("a.c", Some(7))));
    assert_eq!(opening(&info, 4), Some(("a.c", Some(7))));
    // Past every row, the same fallback.
    assert_eq!(opening(&info, 0x100), Some(("a.c", Some(7))));
}

/// A file the query clipped every row of is still the opening file, and no line comes with
/// it: the line is a row's and there is no row to take one from.
#[test]
fn a_file_no_surviving_row_names_opens_with_no_line() {
    let mut rows = RowCollector::over(placed(0x10..0x20), Bias::NONE);
    let file = rows.file("a.c", None);
    rows.push(placed(0..0x10), Some(file), Some(3), None);
    rows.push(placed(0x10..0x18), None, None, None);
    let info = rows.finish().unwrap();

    assert_eq!(opening(&info, 0x10), Some(("a.c", None)));
}

/// Rows that name no file at all open nowhere.
#[test]
fn rows_naming_no_file_have_no_opening() {
    let mut rows = RowCollector::whole();
    rows.push(placed(0..4), None, Some(7), None);
    let info = rows.finish().unwrap();

    assert_eq!(opening(&info, 0), None);
}

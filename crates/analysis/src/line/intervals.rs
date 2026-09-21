//! Half-open ranges, each carrying a value, searched for the ones a range overlaps.

use std::ops::Range;

/// Half-open ranges, each carrying a `T`, searched for the ones overlapping a range: a PDB's
/// section contributions, and the symbols the source index attributes rows to.
///
/// The ranges are sorted by start and may overlap (an alias, a split cold part, two
/// contributions of one module). Each entry also holds `max_end`, the furthest end of it and
/// of every entry before it. A search takes the entries that start before the query ends
/// and walks them back from the last. `max_end` never grows on the way back, so the first
/// entry whose `max_end` is at or before the query's start ends the walk: that entry and
/// every one before it end there or earlier, and none of them can overlap. `addr2line`'s
/// unit index is built the same way.
pub(super) struct Intervals<A, T> {
    entries: Vec<Entry<A, T>>,
}

struct Entry<A, T> {
    start: A,
    end: A,
    max_end: A,
    value: T,
}

impl<A: Ord + Copy, T> Intervals<A, T> {
    /// From ranges in any order. An empty or backwards range is dropped, since it overlaps
    /// nothing. The sort is stable, so ranges with one start keep the order they came in.
    pub(super) fn new(ranges: impl IntoIterator<Item = (Range<A>, T)>) -> Self {
        let mut entries: Vec<Entry<A, T>> = ranges
            .into_iter()
            .filter(|(range, _)| range.start < range.end)
            .map(|(range, value)| Entry {
                start: range.start,
                end: range.end,
                max_end: range.end,
                value,
            })
            .collect();
        entries.sort_by_key(|entry| entry.start);

        let mut max_end = None;
        for entry in &mut entries {
            let furthest = max_end.map_or(entry.end, |max: A| max.max(entry.end));
            entry.max_end = furthest;
            max_end = Some(furthest);
        }

        Intervals { entries }
    }

    /// Every value whose range overlaps `query`, the latest start first. An empty query
    /// `a..a` takes the ranges holding both sides of `a`: those starting before it and
    /// ending after it.
    pub(super) fn over(&self, query: Range<A>) -> impl Iterator<Item = &T> {
        let pos = self
            .entries
            .partition_point(|entry| entry.start < query.end);
        self.entries[..pos]
            .iter()
            .rev()
            .take_while(move |entry| entry.max_end > query.start)
            .filter(move |entry| entry.end > query.start)
            .map(|entry| &entry.value)
    }
}

#[cfg(test)]
mod tests;

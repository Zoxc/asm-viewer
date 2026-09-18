//! Everywhere the reader has been, across every tab: the record the History panel lists.
//!
//! Not a trail. A tab's trail ([`crate::history::History`]) has a cursor and is what Back
//! and Forward walk; this has none and is walked by nothing -- it is how a reader finds
//! somewhere they were, whichever tab they were in at the time. It is an [`Order`] and
//! nothing else: one entry per place, newest first, so a place visited again moves to the
//! top of the panel rather than appearing twice. Entries compare by `Arc` pointer, as a
//! trail's do, so entries made before a re-parse never compare equal to ones made after
//! it. Persisted as [`crate::project::SavedHistory`].

use crate::document::Document;
use crate::order::Order;

/// The most places ever recorded; the oldest are dropped past it.
pub const MAX_VISITS: usize = 200;

/// The places visited, newest first, no two equal.
pub type Visits = Order<Document>;

impl Visits {
    /// A record rebuilt from a saved session, `entries` newest first. They come from
    /// outside, so duplicates are collapsed onto their newest occurrence and the list is
    /// then trimmed to the newest [`MAX_VISITS`] -- which is [`Order::restored_within`],
    /// with the cap this record's own.
    pub fn restored(entries: Vec<Document>) -> Visits {
        Order::restored_within(entries, MAX_VISITS)
    }

    /// Put `document` at the top, moving it there if it is already recorded, and enforce
    /// the cap. Whether anything changed, so a caller can skip a write that would wake
    /// the panel for nothing.
    pub fn record(&mut self, document: Document) -> bool {
        self.touch_within(document, MAX_VISITS)
    }
}

#[cfg(test)]
mod tests;

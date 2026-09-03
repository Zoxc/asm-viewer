//! Everywhere the reader has been, across every tab: the record the History panel lists.
//!
//! Not a trail. A tab's trail ([`crate::history::History`]) has a cursor and is what Back
//! and Forward walk; this has none and is walked by nothing -- it is how a reader finds
//! somewhere they were, whichever tab they were in at the time. One entry per place, so a
//! place visited again moves to the top of the panel rather than appearing twice. Entries
//! compare by `Arc` pointer, as a trail's do. Persisted as
//! [`crate::project::SavedHistory`].

use crate::project::Document;

/// The most places ever recorded; the oldest are dropped past it.
pub const MAX_VISITS: usize = 200;

/// The places visited, oldest first, no two equal.
#[derive(Clone, Default)]
pub struct Visits {
    entries: Vec<Document>,
}

impl Visits {
    /// A record rebuilt from a saved session, `entries` oldest first. They come from
    /// outside, so duplicates are collapsed onto their newest occurrence and the list is
    /// then trimmed to the newest [`MAX_VISITS`].
    pub fn restored(entries: Vec<Document>) -> Visits {
        let mut visits = Visits::default();
        for entry in entries {
            visits.record(entry);
        }
        visits
    }

    /// Every place, oldest first -- what persistence saves.
    pub fn entries(&self) -> &[Document] {
        &self.entries
    }

    /// Every place, newest first -- the order the History panel shows them in.
    pub fn recent(&self) -> impl ExactSizeIterator<Item = &Document> + '_ {
        self.entries.iter().rev()
    }

    /// Whether [`Visits::record`] would change anything: false for the place already at
    /// the top, so a caller can ask before making a write that would wake the panel for
    /// nothing.
    pub fn would_record(&self, document: &Document) -> bool {
        self.entries.last() != Some(document)
    }

    /// Put `document` at the top, moving it there if it is already recorded, and enforce
    /// the cap.
    pub fn record(&mut self, document: Document) {
        if !self.would_record(&document) {
            return;
        }
        self.entries.retain(|entry| *entry != document);
        self.entries.push(document);

        let excess = self.entries.len().saturating_sub(MAX_VISITS);
        self.entries.drain(..excess);
    }

    /// The same record with only the places `keep` accepts: what a closing binary leaves.
    pub fn retaining(&self, keep: impl Fn(&Document) -> bool) -> Visits {
        Visits {
            entries: self
                .entries
                .iter()
                .filter(|entry| keep(entry))
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;

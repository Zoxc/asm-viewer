//! One list of places, newest first and no two the same: what a tab's trail, the record
//! of visits and the two recent orders on disk are all made of.
//!
//! The four differ only in what they hold, where they stop and whether anything points
//! into them, so the cap and the cursor stay with whoever owns them
//! ([`crate::history::History`], [`crate::visits::Visits`], [`crate::store::MAX_ORDER`]).
//!
//! Framework-free: no freya types appear here.

use serde::{Deserialize, Serialize};

/// A list of entries, newest first, no two equal.
///
/// [`Order::touch`] is the way in: an entry already in the list **moves** to the front
/// rather than being added again, so the list says where the reader has been and never
/// how often. Collecting one enforces the same rule on entries from outside, which is
/// what every restore from a file goes through.
///
/// There is no cap here. The lists built on this stop at three different lengths, and the
/// two on disk cap what is *written* rather than what is held, so [`Order::truncate`] is
/// called by whoever owns that rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order<T> {
    /// `Vec::new` and not a plain `default`, which serde's derive would spell as a
    /// `Default` bound on `T` — a file's default is the empty order whatever is in it.
    ///
    /// The name is the key the orders on disk are written under, and the only reason this
    /// is a named field rather than a newtype: a TOML file wants a table with an array in
    /// it.
    #[serde(default = "Vec::new")]
    order: Vec<T>,
}

impl<T> Default for Order<T> {
    fn default() -> Order<T> {
        Order { order: Vec::new() }
    }
}

/// Entries from outside, in the order given, with every entry equal to an earlier one
/// dropped: the earlier one is the newer, so this collapses duplicates onto their newest
/// occurrence.
impl<T: PartialEq> FromIterator<T> for Order<T> {
    fn from_iter<I: IntoIterator<Item = T>>(entries: I) -> Order<T> {
        let mut order = Order::default();
        for entry in entries {
            if !order.order.contains(&entry) {
                order.order.push(entry);
            }
        }
        order
    }
}

impl<T> Order<T> {
    /// Every entry, newest first.
    pub fn entries(&self) -> &[T] {
        &self.order
    }

    pub fn into_entries(self) -> Vec<T> {
        self.order
    }

    /// The newest entry: which project to reopen, which pad to show.
    pub fn first(&self) -> Option<&T> {
        self.order.first()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.order.get(index)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Keep at most the newest `len` entries, dropping the oldest.
    pub fn truncate(&mut self, len: usize) {
        self.order.truncate(len);
    }

    /// Drop everything newer than the entry at `index`, leaving it the newest. What a
    /// trail does with the entries in front of its cursor. An index past the end drops
    /// the lot.
    pub fn drop_newer_than(&mut self, index: usize) {
        self.order.drain(..index.min(self.order.len()));
    }
}

impl<T: PartialEq> Order<T> {
    /// Whether [`Order::touch`] would change anything: false for the entry already at the
    /// front, so a caller can ask before making a write nothing would come of.
    pub fn would_touch(&self, entry: &T) -> bool {
        self.first() != Some(entry)
    }

    /// Put `entry` at the front, moving it there if it is already in the list, and say
    /// whether that changed anything.
    pub fn touch(&mut self, entry: impl Into<T>) -> bool {
        let entry = entry.into();
        if !self.would_touch(&entry) {
            return false;
        }
        self.order.retain(|other| *other != entry);
        self.order.insert(0, entry);
        true
    }

    /// Drop `entry`, and say whether it was there. Nothing prunes one of these on load,
    /// so something that has gone for good is taken out here.
    pub fn forget(&mut self, entry: &T) -> bool {
        let before = self.order.len();
        self.order.retain(|other| other != entry);
        self.order.len() != before
    }

    /// Where `entry` is, for a cursor that has to follow the entry it was on rather than
    /// the index it was at.
    pub fn position(&self, entry: &T) -> Option<usize> {
        self.order.iter().position(|other| other == entry)
    }
}

impl<T: Clone> Order<T> {
    /// The same list with only the entries `keep` accepts, in the order they were in. No
    /// two of those were equal either, so nothing else has to be checked.
    pub fn retaining(&self, keep: impl Fn(&T) -> bool) -> Order<T> {
        Order {
            order: self
                .order
                .iter()
                .filter(|entry| keep(entry))
                .cloned()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests;

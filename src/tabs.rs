//! The two rules that go with a strip of open tabs, and no strip.
//!
//! [`landing`] is the rule a close obeys; [`Positions`] is where each tab was left. Both
//! are framework-free, so they are unit-tested without mounting a UI.

/// The tab to show in place of `showing` once every tab `closing` answers true for is
/// gone: the one that moves into its place, else the last survivor, else `None`.
///
/// Asked of the list as it stands *before* anything is removed. `showing` need not be in
/// `open`, and a `showing` that is not closed is its own answer. `None` means only
/// "nothing is left", never "nothing was closed": this cannot tell, having removed
/// nothing.
pub fn landing<T: Clone + PartialEq>(
    open: &[T],
    showing: Option<&T>,
    closing: impl Fn(&T) -> bool,
) -> Option<T> {
    // Where the tab that moves into `showing`'s place will be once the closed ones
    // are gone: how many of the tabs before it survive. A tab that is not open at
    // all — or no tab at all — counts as being past the end, which lands on the last
    // survivor.
    let position = showing
        .and_then(|showing| open.iter().position(|open| open == showing))
        .unwrap_or(open.len());
    let landing = open[..position]
        .iter()
        .filter(|open| !closing(open))
        .count();

    let surviving = || open.iter().filter(|open| !closing(open));
    surviving()
        .nth(landing)
        .or_else(|| surviving().last())
        .cloned()
}

/// Where each tab was left: the row that was at the top of its pane.
///
/// **A row and not a pixel offset**, so it survives a row height that follows the fonts,
/// a listing that has grown or shrunk under a rebuilt binary, and a file edited since it
/// was last read. A tab that was never scrolled has no entry here and reads as the top.
///
/// A `Vec` of pairs and not a `HashMap`, because the key is whatever the tab list holds:
/// a [`crate::project::Document`] is compared by `Arc` pointer identity where it is a
/// place in a binary and hashes by nothing at all.
pub struct Positions<T> {
    at: Vec<(T, usize)>,
}

impl<T> Default for Positions<T> {
    fn default() -> Self {
        Positions { at: Vec::new() }
    }
}

impl<T: Clone + PartialEq> Positions<T> {
    /// The row `tab` was left at, or `None` when it has never been anywhere — which a
    /// pane needs in order to tell "never seen" from "seen at the top".
    pub fn at(&self, tab: &T) -> Option<usize> {
        self.at
            .iter()
            .find(|(open, _)| open == tab)
            .map(|(_, row)| *row)
    }

    /// The row to put `tab` back on, in a pane now holding `length` rows. A saved
    /// position is a hint and not a fact, so this clamps twice: a tab never seen is the
    /// top, and a row past the end of what the tab holds now is its last row.
    pub fn row(&self, tab: &T, length: usize) -> usize {
        self.at(tab).unwrap_or(0).min(length.saturating_sub(1))
    }

    /// Remember that `tab` is at `row`, replacing whatever it was at before.
    pub fn remember(&mut self, tab: T, row: usize) {
        match self.at.iter_mut().find(|(open, _)| *open == tab) {
            Some((_, at)) => *at = row,
            None => self.at.push((tab, row)),
        }
    }

    /// Forget where `tab` was, because it is no longer open.
    ///
    /// Not an optimisation: a [`crate::project::Document`] that is a place in a binary
    /// holds the `Arc<Object>` it points into, so a position kept for a closed tab would
    /// hold that binary's bytes for as long as the app ran.
    pub fn forget(&mut self, tab: &T) {
        self.at.retain(|(open, _)| open != tab);
    }

    /// Forget every position `keep` answers false for, for the tabs a closing binary
    /// takes with it.
    pub fn forgetting(&mut self, keep: impl Fn(&T) -> bool) {
        self.at.retain(|(open, _)| keep(open));
    }
}

#[cfg(test)]
mod tests;

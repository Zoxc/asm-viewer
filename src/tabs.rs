//! The two rules that go with a strip of open tabs, and no strip.
//!
//! Framework-free, like [`crate::history`] and [`crate::filter`] — rules with no pixels
//! on them, so they are unit-tested without mounting a UI.
//!
//! There used to be a `Tabs<T>` here, the list of open documents. There is no such list
//! any more: an open document is a tab in the dock, so the *order* the reader put them in
//! is the document panel's own `tabs` vec and `crate::docs::Docs` holds only the handle
//! each tab is known by. What did not belong to that list is what is left here.
//!
//! [`landing`] is the rule a close obeys — the tab that moves into the closed one's place,
//! else the one before it. It is a rule of the app rather than a property of any one
//! container, which is exactly why it outlived the container. [`Positions`] is where each
//! tab was left, which was never part of the list either: it is written by the pane a tab
//! is shown *in*, so keeping the two apart is what stops a scroll of the reader's from
//! re-rendering the tab bar.

/// The tab to show in place of `showing` once every tab `closing` answers true for is
/// gone: the one that moves into its place, else the last survivor, else `None`.
///
/// Asked of the list as it stands *before* anything is removed, which is what makes it a
/// function rather than a method — the answer is about the shape of the list, and the
/// removal itself belongs to whoever owns it. `showing` need not be in `open`, and a
/// `showing` that is not closed is its own answer.
///
/// It is a free function because the rule outlives the list it is currently written
/// against. The order the reader's tabs are in is not always a `Vec` this module owns,
/// and "closing a tab lands you on its right-hand neighbour" is a rule of the app rather
/// than a property of any one container — so it is stated once, tested once, and called
/// from wherever the tabs happen to live.
///
/// `None` means only "nothing is left", never "nothing was closed": this cannot tell,
/// having removed nothing. [`Tabs::close_all`] adds that distinction because it does the
/// removing and can compare the lengths.
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
/// The other half of a tab, and deliberately not a field of one. A [`Tabs`] entry is
/// what the strip draws and what the session writes out; this is a view of it, written
/// by the pane a tab is *shown in* and read by nothing else — so keeping the two apart
/// is what stops a scroll of the reader's from re-rendering the strip, and what lets a
/// tab's two sides each carry one of these without the list type learning about pixels.
///
/// **A row and not a pixel offset.** The two are convertible (`code_row_height()` is the
/// two code panes' `item_size`, and these positions are only ever theirs), so this is a
/// choice about which one to keep, and the row is the
/// one that survives everything that can happen between leaving a tab and coming back to
/// it: a row height that follows the fonts, which it now does, a listing that has grown or
/// shrunk under a rebuilt binary, and a file edited since it was last read. It is also
/// the unit the answer is wanted in — "the fourteenth instruction", not "364 pixels" —
/// which is why the clamping in [`Positions::row`] can be written at all.
///
/// A tab that was never scrolled has no entry here and reads as the top, which is the
/// same answer a tab seen for the first time gets. That is the point of `Option` in
/// [`Positions::at`] and of there being no `insert`-on-read: nothing has to seed this
/// when a tab opens, and nothing leaks when one closes without ever being scrolled.
///
/// A `Vec` of pairs and not a `HashMap`, because the key is whatever the tab list holds:
/// a [`crate::project::Document`] is compared by `Arc` pointer identity where it is a
/// place in a binary and hashes by nothing at all, and a strip is a handful of tabs, not
/// a table.
pub struct Positions<T> {
    at: Vec<(T, usize)>,
}

impl<T> Default for Positions<T> {
    fn default() -> Self {
        Positions { at: Vec::new() }
    }
}

impl<T: Clone + PartialEq> Positions<T> {
    /// The row `tab` was left at, or `None` when it has never been anywhere.
    ///
    /// The raw answer, for a caller that has to tell "never seen" from "seen at the top"
    /// — which the pane does, since writing back what it already holds would wake the
    /// save observer on every pointer move over a scrollbar.
    pub fn at(&self, tab: &T) -> Option<usize> {
        self.at
            .iter()
            .find(|(open, _)| open == tab)
            .map(|(_, row)| *row)
    }

    /// The row to put `tab` back on, in a pane now holding `length` rows.
    ///
    /// The answer a pane actually wants, and it clamps twice over: a tab never seen is
    /// the top, and a remembered row past the end of what the tab holds now is its last
    /// row. A saved position is a hint and not a fact — the binary may have been rebuilt
    /// and the source file edited since — so the only wrong answer here is one that is
    /// not a row of this listing.
    ///
    /// The end and not the top, when a listing has shrunk: the reader was near the end of
    /// it, and the end is what is still nearest to where they were.
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
    /// holds the `Arc<Object>` it points into, so a position kept for a tab that was
    /// closed with its binary would hold that binary's bytes — 331 MB of them, for the
    /// sample the app is developed against — for as long as the app ran.
    pub fn forget(&mut self, tab: &T) {
        self.at.retain(|(open, _)| open != tab);
    }

    /// Forget every position `keep` answers false for. The bulk form of
    /// [`Positions::forget`], for the tabs a closing binary takes with it.
    pub fn forgetting(&mut self, keep: impl Fn(&T) -> bool) {
        self.at.retain(|(open, _)| keep(open));
    }
}

#[cfg(test)]
mod tests;

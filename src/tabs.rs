//! The three rules that go with a strip of open tabs, and no strip.
//!
//! [`landing`] is the rule a close obeys, [`Positions`] is where each tab was left, and
//! [`Driven`] is which source line a source-driven tab's assembly side follows. All three
//! are framework-free, so they are unit-tested without mounting a UI.

use std::path::Path;

use analysis::Symbol;

use crate::docs::{DocId, Entry};

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
///
/// The value is a row for the two panes, an **address** for the listing of an object's
/// whole code, whose rows are counted afresh as it is decoded and where a row means
/// nothing for long, and the **runs** each pane had picked out at the place -- which are
/// not `Copy`, so a value need only be `Clone`; the map is the same map in all three,
/// and only [`row`](Positions::row), the clamp against a listing's length, is a
/// rows-only answer.
pub struct Positions<T, V = usize> {
    at: Vec<(T, V)>,
}

impl<T, V> Default for Positions<T, V> {
    fn default() -> Self {
        Positions { at: Vec::new() }
    }
}

impl<T: Clone + PartialEq, V: Clone + PartialEq> Positions<T, V> {
    /// Where `tab` was left, or `None` when it has never been anywhere — which a pane
    /// needs in order to tell "never seen" from "seen at the top".
    pub fn at(&self, tab: &T) -> Option<V> {
        self.at
            .iter()
            .find(|(open, _)| open == tab)
            .map(|(_, at)| at.clone())
    }

    /// Remember that `tab` is at `at`, replacing whatever it was at before.
    pub fn remember(&mut self, tab: T, at: V) {
        match self.at.iter_mut().find(|(open, _)| *open == tab) {
            Some((_, was)) => *was = at,
            None => self.at.push((tab, at)),
        }
    }

    /// Forget every position `keep` answers false for: a closing tab's, or a closing
    /// binary's.
    ///
    /// Not an optimisation: a [`crate::project::Document`] that is a place in a binary
    /// holds the `Arc<Object>` it points into, so a position kept for a closed tab would
    /// hold that binary's bytes for as long as the app ran.
    pub fn forgetting(&mut self, keep: impl Fn(&T) -> bool) {
        self.at.retain(|(open, _)| keep(open));
    }
}

impl<T: Clone + PartialEq> Positions<T> {
    /// The row to put `tab` back on, in a pane now holding `length` rows. A saved
    /// position is a hint and not a fact, so this clamps twice: a tab never seen is the
    /// top, and a row past the end of what the tab holds now is its last row.
    pub fn row(&self, tab: &T, length: usize) -> usize {
        self.at(tab).unwrap_or(0).min(length.saturating_sub(1))
    }
}

/// Where a listing of an object's whole code was left: the placed address at the top of
/// the pane, and how many rows past that address's row -- a stretch's header, its labels
/// and its first instruction all sit at one address, and the rows are what tell them
/// apart. The address is what a session saves; the rows are a nicety that does not
/// survive a restart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spot {
    pub address: u64,
    pub rows: usize,
}

/// Which source line each source-driven tab's assembly side is driven from.
///
/// [`Positions`]-shaped and deliberately not [`Positions`] itself: that type's
/// [`row`](Positions::row) clamps a saved row against the listing that is there *now*,
/// which is a rows-only answer and would have to grow a meaningless one for lines.
///
/// Keyed by an [`Entry`] -- a tab and one of the documents on its trail -- as the
/// positions are, so a file reached twice along one trail is driven from one line and two
/// tabs on one file are driven from two. The value is a line and not the symbol it
/// resolves to, so the line holds no `Arc<Object>`: a driven line survives its binary being
/// closed, and the next ask simply answers out of whatever is still open. Lines are
/// 1-based, as DWARF's are.
///
/// Beside the line, **which of the many symbols the line compiles into the tab follows**,
/// where the reader has said -- a row chosen in the Locations panel. That one is a
/// [`Symbol`] and does hold the file's bytes, so it is [`released`](Driven::release) with
/// its file where the line beside it stays. It outlives the line: reading down a generic
/// function inside the instantiation picked is the point of picking one, so a click on
/// the next line keeps the choice and the pick falls back only where the choice was not
/// compiled from that line.
#[derive(Default)]
pub struct Driven {
    from: Vec<(Entry, u32)>,
    chosen: Vec<(Entry, Symbol)>,
}

impl Driven {
    /// The line `tab` is driven from, or `None` for a tab nothing has been clicked in —
    /// which is a source-driven tab whose assembly side is still empty.
    pub fn line(&self, tab: &Entry) -> Option<u32> {
        self.from
            .iter()
            .find(|(open, _)| open == tab)
            .map(|(_, line)| *line)
    }

    /// Drive `tab` from `line`, in place of whatever it was driven from before.
    pub fn remember(&mut self, tab: Entry, line: u32) {
        match self.from.iter_mut().find(|(open, _)| *open == tab) {
            Some((_, from)) => *from = line,
            None => self.from.push((tab, line)),
        }
    }

    /// The symbol `tab`'s assembly side follows among the many its line compiles into,
    /// or `None` where the reader has not chosen one.
    pub fn choice(&self, tab: &Entry) -> Option<Symbol> {
        self.chosen
            .iter()
            .find(|(open, _)| open == tab)
            .map(|(_, symbol)| symbol.clone())
    }

    /// Have `tab`'s assembly side follow `symbol`, in place of whatever was chosen before.
    pub fn choose(&mut self, tab: Entry, symbol: Symbol) {
        match self.chosen.iter_mut().find(|(open, _)| *open == tab) {
            Some((_, chosen)) => *chosen = symbol,
            None => self.chosen.push((tab, symbol)),
        }
    }

    /// Forget what every entry of the tab `id` was driven from and what it chose, because
    /// the tab is no longer open. For the line, consistency and not [`Positions::forget`]'s
    /// reason -- a [`crate::project::Document::Source`] key holds no `Arc<Object>`; for the choice, that
    /// reason.
    pub fn forget_tab(&mut self, id: DocId) {
        self.from.retain(|((open, _), _)| *open != id);
        self.chosen.retain(|((open, _), _)| *open != id);
    }

    /// Forget every line and choice `keep` answers false for: what a closing binary does
    /// with the entries it takes off the surviving tabs' trails.
    pub fn forgetting(&mut self, keep: impl Fn(&Entry) -> bool) {
        self.from.retain(|(open, _)| keep(open));
        self.chosen.retain(|(open, _)| keep(open));
    }

    /// Let go of every choice into the file at `path`, because it is closing: a
    /// [`Symbol`] holds the file's bytes. The lines stay, and the tabs they drive answer
    /// out of whatever is still open.
    pub fn release(&mut self, path: &Path) {
        self.chosen.retain(|(_, symbol)| symbol.object.path != path);
    }
}

#[cfg(test)]
mod tests;

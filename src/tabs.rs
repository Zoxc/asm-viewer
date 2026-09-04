//! The strip of open tabs and the three rules that go with it.
//!
//! [`Strip`] is what is open and which of it is on screen, [`landing`] the rule a close
//! obeys, [`Positions`] where each tab was left, and [`Driven`] which source line a
//! source-driven tab's assembly side follows. All of it is framework-free, so it is
//! unit-tested without mounting a UI.

use std::path::Path;

use analysis::Symbol;

use crate::docs::{DocId, Entry};

/// One of the app's own pages: a tab that is not a document.
///
/// Three of them, one of a kind, each drawn from state that lives at the root of the app
/// rather than in the tab -- so closing one loses nothing, and it comes back as it was.
///
/// [`Page::stored`] and not [`Page::title`] is what a session is written with: the title is
/// what the reader sees, and a title reworded as prose would empty every saved bar.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Project,
    Settings,
    Scratchpad,
}

impl Page {
    /// Every page, in the order a menu lists them.
    pub const ALL: [Page; 3] = [Page::Project, Page::Settings, Page::Scratchpad];

    /// What the tab is called.
    pub fn title(self) -> &'static str {
        match self {
            Page::Project => "Project",
            Page::Settings => "Settings",
            Page::Scratchpad => "Scratchpad",
        }
    }

    /// What a session names it. See the type.
    pub fn stored(self) -> &'static str {
        match self {
            Page::Project => "project",
            Page::Settings => "settings",
            Page::Scratchpad => "scratchpad",
        }
    }

    /// The page a session named, or `None` for a name this build does not have -- a file
    /// a build with one more page wrote, which drops that tab and keeps the rest.
    pub fn from_stored(stored: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|page| page.stored() == stored)
    }
}

/// One tab in the strip: an open document, or one of the app's pages.
///
/// A document is carried as the [`DocId`] [`crate::docs::Docs`] knows it by, because a tab
/// is `Copy` -- it is a drag's payload, a list's key and a menu row's capture -- and a
/// document is not.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tab {
    Document(DocId),
    Page(Page),
}

/// The bar of open tabs: their order, which is the reader's own, and which one is on
/// screen.
///
/// There is no second list: what is open *is* this vec, and [`crate::docs::Docs`] holds
/// the trail behind each document tab and no order at all. Every rule about the bar is
/// here, so the UI above it has none of its own: a tab opens beside the tab on screen,
/// a close lands on the neighbour ([`landing`]), and a move is a move.
#[derive(Default)]
pub struct Strip {
    tabs: Vec<Tab>,
    active: Option<Tab>,
}

impl Strip {
    /// The tabs, in the order they are drawn in.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The tab on screen, or `None` when nothing is open.
    pub fn active(&self) -> Option<Tab> {
        self.active
    }

    /// Whether `tab` is one of the open ones.
    pub fn contains(&self, tab: Tab) -> bool {
        self.tabs.contains(&tab)
    }

    /// Every open document tab's id, in the order the tabs are in.
    pub fn documents(&self) -> impl Iterator<Item = DocId> + '_ {
        self.tabs.iter().filter_map(|tab| match tab {
            Tab::Document(id) => Some(*id),
            Tab::Page(_) => None,
        })
    }

    /// Put `tab` at `position`, or at the end when that is past it, and show it: what a
    /// restore does, stating the saved order outright rather than reproducing it a tab at
    /// a time. A tab already open only comes to the front.
    pub fn insert(&mut self, tab: Tab, position: usize) {
        if !self.contains(tab) {
            self.tabs.insert(position.min(self.tabs.len()), tab);
        }
        self.active = Some(tab);
    }

    /// Show `tab`, opening it **beside the tab on screen** when it is not open yet -- the
    /// way a browser opens a link, so a place opened out of a function sits next to the
    /// function.
    pub fn show(&mut self, tab: Tab) {
        if !self.contains(tab) {
            let after = self
                .active
                .and_then(|active| self.tabs.iter().position(|open| *open == active));
            match after {
                Some(index) => self.tabs.insert(index + 1, tab),
                None => self.tabs.push(tab),
            }
        }
        self.active = Some(tab);
    }

    /// Make an open tab the one on screen, answering whether it was open at all. Nothing
    /// is written for a tab that is already showing: a write notifies whether or not it
    /// changed anything, and re-raising the tab on top must wake nothing.
    pub fn raise(&mut self, tab: Tab) -> bool {
        if !self.contains(tab) {
            return false;
        }
        if self.active != Some(tab) {
            self.active = Some(tab);
        }
        true
    }

    /// Move `tab` so that it sits where the tab now at `position` does, which is what a
    /// drop on that tab's chip means. Past the end is the end, and the tab on screen does
    /// not change: a tab dragged is not a tab opened.
    ///
    /// **A tab the strip does not hold is not put there.** A chip dragged while its
    /// document is closed under it carries an id that stands for nothing, and inserting it
    /// would raise a closed document from the dead.
    pub fn move_to(&mut self, tab: Tab, position: usize) {
        let Some(from) = self.tabs.iter().position(|open| *open == tab) else {
            return;
        };
        self.tabs.remove(from);
        let to = match position > from {
            true => position - 1,
            false => position,
        };
        self.tabs.insert(to.min(self.tabs.len()), tab);
    }

    /// Close every tab `closing` answers true for, landing on the neighbour when the tab
    /// on screen was one of them. Answers what it removed, in the order the tabs were in.
    ///
    /// The landing is worked out before anything is removed, which is what [`landing`]
    /// asks of its caller, and the tab on screen is left alone when it survives.
    pub fn close(&mut self, closing: impl Fn(&Tab) -> bool) -> Vec<Tab> {
        let closed: Vec<Tab> = self
            .tabs
            .iter()
            .copied()
            .filter(|tab| closing(tab))
            .collect();
        if closed.is_empty() {
            return closed;
        }
        let showing = self.active.is_some_and(|active| closing(&active));
        if showing {
            self.active = landing(&self.tabs, self.active.as_ref(), &closing);
        }
        self.tabs.retain(|tab| !closing(tab));
        closed
    }
}

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
/// Keyed by an [`Entry`] -- a tab and one of the places on its trail -- as the positions
/// are, so a file reached twice along one trail is driven from one line, a file being one
/// place, and two tabs on one file are driven from two. The value is a line and not the symbol it
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

//! The strip of open tabs and the rule a close obeys.
//!
//! [`Strip`] is what is open and which of it is on screen; [`landing`] is the tab a
//! close lands on. Both are framework-free, so they are unit-tested without mounting a
//! UI. Where each tab was left is [`crate::positions`].

use crate::docs::DocId;

/// One of the app's own pages: a tab that is not a document.
///
/// Each is one of a kind, and each is drawn from state that lives at the root of the app
/// rather than in the tab -- so closing one loses nothing, and it comes back as it was.
///
/// [`Page::stored`] and not [`Page::title`] is what a session is written with: the title is
/// what the reader sees, and a title reworded as prose would empty every saved bar.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Project,
    Settings,
    /// Every key and every mouse gesture the app answers to (`src/shortcuts.rs`). Beside
    /// Settings, both being a page a reader opens to find out how the app is worked
    /// rather than to read a binary with.
    Shortcuts,
    Scratchpad,
    /// The ways to make the app misbehave on purpose, so that what it does about it can
    /// be looked at (`src/ui/debug_view.rs`). Last in every list here, being the one page
    /// a reader has no ordinary reason to open.
    Debug,
}

impl Page {
    /// Every page, in the order a menu lists them. Whether a menu lists the last of them
    /// is the menu's own question, and is asked of the keyboard rather than of this
    /// (`src/ui/strip.rs`).
    pub const ALL: [Page; 5] = [
        Page::Project,
        Page::Settings,
        Page::Shortcuts,
        Page::Scratchpad,
        Page::Debug,
    ];

    /// What the tab is called.
    pub fn title(self) -> &'static str {
        match self {
            Page::Project => "Project",
            Page::Settings => "Settings",
            Page::Shortcuts => "Shortcuts",
            Page::Scratchpad => "Scratchpad",
            Page::Debug => "Debug",
        }
    }

    /// What a session names it. See the type.
    pub fn stored(self) -> &'static str {
        match self {
            Page::Project => "project",
            Page::Settings => "settings",
            Page::Shortcuts => "shortcuts",
            Page::Scratchpad => "scratchpad",
            Page::Debug => "debug",
        }
    }

    /// The page a session named, or `None` for a name this build does not have -- a file
    /// a build with one more page wrote, which drops that tab and keeps the rest.
    ///
    /// [`Page::Debug`] is *not* filtered here. It is kept out of the menu unless it is
    /// asked for, and a reader who has one open has asked for it: a session that names it
    /// puts it back, the way a session naming any other page does.
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

#[cfg(test)]
mod tests;

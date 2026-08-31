//! The open tabs: the places the reader has open at once.
//!
//! Framework-free, like [`crate::history`] and [`crate::filter`] — this is a list with
//! three rules on it and no pixels, so the rules are unit-tested without mounting a UI.
//!
//! Two lists in the app are this shape: the functions and objects open in the content
//! area, which hold [`crate::project::Selection`]s, and the source files open in the
//! Source pane, which hold paths. Neither of them records *which* tab is on screen.
//! That is deliberate and is the whole reason this type is as small as it is: the active
//! function already has a home in `Sel`, which is what the history records and the session
//! saves, and giving the list a second answer to the same question would be two states to
//! keep in step. The active tab is simply the one equal to what is on screen, which is
//! well defined because no two tabs are ever equal.
//!
//! The other thing worth saying is what this is *not*. [`crate::history::History`] grows
//! on its own — every place the reader goes is recorded, so it dedups by bumping an entry
//! to the newest position and is capped at a maximum. A tab appears only when it is asked
//! for and goes away only when it is closed, so opening one that is already open changes
//! nothing at all (a tab strip that reordered itself under the pointer would be unusable)
//! and there is no cap: the list is exactly as long as the reader made it.

/// The tabs that are open, in the order they were opened.
pub struct Tabs<T> {
    open: Vec<T>,
}

impl<T> Default for Tabs<T> {
    fn default() -> Self {
        Tabs { open: Vec::new() }
    }
}

impl<T: Clone + PartialEq> Tabs<T> {
    /// Every open tab, oldest first — what a strip of them draws.
    pub fn tabs(&self) -> &[T] {
        &self.open
    }

    /// The open tab equal to `item`, or `None` when it is not open.
    ///
    /// Hands back the copy that is *in the list* rather than a bool, because for a source
    /// file the two are equal without being the same allocation: two `LineInfo`s naming
    /// one path hold two `Arc<str>`s of it, and keeping the one already open is what keeps
    /// the pointer identity the rows are keyed by stable across a selection change.
    pub fn find(&self, item: &T) -> Option<&T> {
        self.open.iter().find(|open| *open == item)
    }

    /// Open a tab for `item`, or leave the list exactly as it is when one is open already.
    pub fn open(&mut self, item: T) {
        if self.find(&item).is_none() {
            self.open.push(item);
        }
    }

    /// Close the tab showing `item`, and hand back the tab to show instead: the one that
    /// moves into its place, else the one before it, else `None` when that was the last
    /// tab open.
    ///
    /// The answer is only wanted when the tab being closed is the one on screen, which is
    /// the caller's business rather than this list's — it does not know which that is.
    /// Closing something that is not open is a no-op and answers `None`.
    pub fn close(&mut self, item: &T) -> Option<T> {
        let position = self.open.iter().position(|open| open == item)?;
        self.open.remove(position);
        self.open
            .get(position)
            .or_else(|| self.open.last())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(items: &[&str]) -> Tabs<String> {
        let mut tabs = Tabs::default();
        for item in items {
            tabs.open((*item).to_owned());
        }
        tabs
    }

    #[test]
    fn opening_appends_in_order() {
        let tabs = tabs(&["a", "b", "c"]);
        assert_eq!(tabs.tabs(), ["a", "b", "c"]);
    }

    #[test]
    fn opening_something_already_open_changes_nothing() {
        // Not a bump to the newest position, which is what `History::push` does: a strip
        // of tabs that reordered itself when one of them was clicked would move every
        // other tab out from under the pointer.
        let mut tabs = tabs(&["a", "b", "c"]);
        tabs.open("a".to_owned());
        assert_eq!(tabs.tabs(), ["a", "b", "c"]);
    }

    #[test]
    fn find_hands_back_the_copy_in_the_list() {
        let tabs = tabs(&["a"]);
        let asked = "a".to_owned();
        let found = tabs.find(&asked).expect("the open tab");
        assert_eq!(found, "a");
        // The one in the list, not the one asked about.
        assert!(!std::ptr::eq(found, &asked));
        assert_eq!(tabs.find(&"b".to_owned()), None);
    }

    #[test]
    fn closing_moves_to_the_tab_on_its_right() {
        let mut tabs = tabs(&["a", "b", "c"]);
        assert_eq!(tabs.close(&"b".to_owned()), Some("c".to_owned()));
        assert_eq!(tabs.tabs(), ["a", "c"]);
    }

    #[test]
    fn closing_the_last_tab_moves_to_the_one_on_its_left() {
        let mut tabs = tabs(&["a", "b"]);
        assert_eq!(tabs.close(&"b".to_owned()), Some("a".to_owned()));
        assert_eq!(tabs.tabs(), ["a"]);
    }

    #[test]
    fn closing_the_only_tab_leaves_nothing_to_show() {
        let mut tabs = tabs(&["a"]);
        assert_eq!(tabs.close(&"a".to_owned()), None);
        assert!(tabs.tabs().is_empty());
    }

    #[test]
    fn closing_something_that_is_not_open_does_nothing() {
        let mut tabs = tabs(&["a", "b"]);
        assert_eq!(tabs.close(&"c".to_owned()), None);
        assert_eq!(tabs.tabs(), ["a", "b"]);
    }
}

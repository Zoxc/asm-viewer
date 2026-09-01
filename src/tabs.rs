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
mod tests {
    use super::*;

    // --- the landing rule, on its own -------------------------------------
    //
    // The same rules as the `Tabs` tests above, asked of the free function directly, so
    // that "a close lands on the right-hand neighbour" keeps its coverage wherever the
    // tabs are being kept. `landing` is asked *before* anything is removed, so each of
    // these passes the whole list and the predicate that is about to thin it.

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn shut(items: &[&str], showing: &str, closing: &[&str]) -> Option<String> {
        let open = strings(items);
        let closing = strings(closing);
        landing(&open, Some(&showing.to_string()), |open| {
            closing.contains(open)
        })
    }

    #[test]
    fn landing_moves_to_the_tab_on_its_right() {
        assert_eq!(shut(&["a", "b", "c"], "b", &["b"]), Some("c".to_owned()));
    }

    #[test]
    fn landing_on_the_last_tab_moves_to_the_one_on_its_left() {
        assert_eq!(shut(&["a", "b", "c"], "c", &["c"]), Some("b".to_owned()));
    }

    #[test]
    fn landing_with_nothing_left_is_nothing() {
        assert_eq!(shut(&["a"], "a", &["a"]), None);
    }

    /// The bulk case: the reader ends up where closing the one tab by hand would have
    /// put them, whether the tabs around it went with it or not.
    #[test]
    fn landing_after_several_is_the_first_survivor_after_the_shown_one() {
        assert_eq!(
            shut(&["a", "b", "c", "d"], "b", &["a", "b", "c"]),
            Some("d".to_owned())
        );
    }

    #[test]
    fn landing_after_closing_the_newest_several_moves_left() {
        assert_eq!(
            shut(&["a", "b", "c", "d"], "c", &["c", "d"]),
            Some("b".to_owned())
        );
    }

    /// A tab that survives is its own answer, which is what lets a caller ask without
    /// first working out whether what is on screen is going anywhere.
    #[test]
    fn a_surviving_shown_tab_is_its_own_landing() {
        assert_eq!(shut(&["a", "b", "c"], "b", &["c"]), Some("b".to_owned()));
    }

    /// Nothing on screen is a state the app is really in — an empty strip — and a close
    /// asked for from it still has to say which tab is left. It lands on the last
    /// survivor, exactly where a tab that is not open at all lands.
    #[test]
    fn landing_from_nothing_shown_is_the_last_survivor() {
        let open = strings(&["a", "b", "c"]);
        assert_eq!(
            landing(&open, None, |open| open == "b"),
            Some("c".to_owned())
        );
        let missing = "z".to_owned();
        assert_eq!(
            landing(&open, Some(&missing), |open| open == "b"),
            Some("c".to_owned())
        );
    }

    /// `landing` removes nothing, so it cannot tell "nothing was closed" from "nothing
    /// is left" — it answers the tab that is still there, and the distinction is
    /// [`Tabs::close_all`]'s to draw because it is the one doing the removing.
    #[test]
    fn landing_that_closes_nothing_answers_the_shown_tab() {
        let open = strings(&["a", "b"]);
        assert_eq!(
            landing(&open, Some(&"a".to_owned()), |_| false),
            Some("a".to_owned())
        );
        assert_eq!(
            landing::<String>(&[], Some(&"a".to_owned()), |_| false),
            None
        );
    }

    // --- where each tab was left ------------------------------------------

    fn positions(at: &[(&str, usize)]) -> Positions<String> {
        let mut positions = Positions::default();
        for (tab, row) in at {
            positions.remember((*tab).to_owned(), *row);
        }
        positions
    }

    #[test]
    fn a_tab_never_seen_is_at_no_row_and_opens_at_the_top() {
        let positions = positions(&[]);
        assert_eq!(positions.at(&"a".to_owned()), None);
        assert_eq!(positions.row(&"a".to_owned(), 100), 0);
    }

    #[test]
    fn a_remembered_row_comes_back() {
        let positions = positions(&[("a", 12), ("b", 40)]);
        assert_eq!(positions.at(&"a".to_owned()), Some(12));
        assert_eq!(positions.row(&"b".to_owned(), 100), 40);
    }

    #[test]
    fn remembering_a_tab_twice_replaces_its_row() {
        let mut positions = positions(&[("a", 12)]);
        positions.remember("a".to_owned(), 13);
        assert_eq!(positions.at(&"a".to_owned()), Some(13));
        // Replaced, not appended: the second answer is the only answer.
        assert_eq!(positions.at.len(), 1);
    }

    /// The listing has shrunk under the position — a rebuilt binary, a source file edited
    /// since it was read — so the row is the last one there now rather than one past the
    /// end. The end is where the reader was; the top is not.
    #[test]
    fn a_row_past_the_end_clamps_to_the_last_one() {
        let positions = positions(&[("a", 900)]);
        assert_eq!(positions.row(&"a".to_owned(), 100), 99);
        // And `at` still says what was remembered: only the answer given to a pane is
        // clamped, because only a pane knows what it is holding.
        assert_eq!(positions.at(&"a".to_owned()), Some(900));
    }

    #[test]
    fn an_empty_listing_has_no_row_but_the_first() {
        let positions = positions(&[("a", 900)]);
        assert_eq!(positions.row(&"a".to_owned(), 0), 0);
    }

    #[test]
    fn forgetting_a_tab_leaves_the_others() {
        let mut positions = positions(&[("a", 1), ("b", 2)]);
        positions.forget(&"a".to_owned());
        assert_eq!(positions.at(&"a".to_owned()), None);
        assert_eq!(positions.at(&"b".to_owned()), Some(2));
        // And forgetting one that was never there is not an error.
        positions.forget(&"c".to_owned());
        assert_eq!(positions.at(&"b".to_owned()), Some(2));
    }

    #[test]
    fn a_closing_binary_forgets_every_position_into_it() {
        let mut positions = positions(&[("lib.a:one", 1), ("some.dll:two", 2), ("lib.a:three", 3)]);
        positions.forgetting(|tab| !tab.starts_with("lib.a:"));
        assert_eq!(positions.at(&"some.dll:two".to_owned()), Some(2));
        assert_eq!(positions.at(&"lib.a:one".to_owned()), None);
        assert_eq!(positions.at(&"lib.a:three".to_owned()), None);
    }
}

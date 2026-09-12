//! The Bookmarks view: the project's own list of places, one row each, live against what is
//! loaded and kept when it is not.

use super::*;

/// One bookmark. Live when its place resolves against the objects loaded now, in which case
/// pressing it is a navigation like a press in the Symbols list; dead when it does not, in
/// which case it is drawn dimmed and does nothing, and is still there -- a reader's own list
/// does not shrink behind their back.
#[derive(Clone, PartialEq)]
struct BookmarkRow {
    /// Which bookmark this is, in the reader's own list -- what its menu removes by.
    index: usize,
    bookmark: Bookmark,
    live: Option<Document>,
    /// Where this row is in the list as it is drawn, which under a filter is not `index`.
    at: usize,
    /// Where the filter matched in the label, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

keyed!(BookmarkRow);

impl Component for BookmarkRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        // Consumed and not read: a row hands the list an index back and draws nothing of
        // it that the tab has not already handed it.
        let doors = use_doors();
        let ctrl = use_consume::<Ctrl>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let picking = use_picking(Panel::Bookmarks);
        let index = self.index;
        let at = self.at;
        let pick = Pick::Bookmark(self.bookmark.clone());
        let dead = self.live.is_none();

        // The same three rules a live document's row draws by, over the saved place: the
        // row keeps its spelling when its binary goes.
        let Names { text, tooltip, .. } = Names::of_saved(&self.bookmark);

        // A dead row has no handlers at all, like a dimmed history button: nothing to go
        // to, so nothing to light up for, and nothing to pick out either. Nothing about
        // the tab on screen picks a live row out -- a bookmark is a place and the tab may
        // be anywhere -- so a row here lights when the reader pressed it and not
        // otherwise.
        let row = match &self.live {
            Some(live) => {
                let live = live.clone();
                list_row(hovering, picking.drawn(&pick, false)).on_press(move |_| {
                    picking.press(pick.clone(), at, || opened(doors, ctrl, live.clone()));
                })
            }
            None => dead_list_row(),
        };

        name_tooltip(
            fitted.cut(),
            &text.clone(),
            tooltip,
            row.on_secondary_down(move |e: Event<PressEventData>| {
                ContextMenu::open_from_event(&e, remove_menu(bookmarked, index));
            })
            .child(saved_icon(&self.bookmark.document))
            .child(tree_name_fitted(fitted, text, dead, &self.marks)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// Which kind of place a saved document is, as the glyph its live tab would wear.
fn saved_icon(saved: &SavedDocument) -> Element {
    kind_icon(saved.kind())
}

/// The menu a bookmark row opens on a right-click: one item, removing that row. By index
/// and not by place, because a dead row is exactly the one that resolves to no place and
/// the one this is most wanted on. Built per press, as every menu is (`menus.rs`).
fn remove_menu(bookmarked: State<Bookmarks>, index: usize) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                let mut bookmarked = bookmarked;
                bookmarked.write().remove(index);
            })
            .child("Remove bookmark"),
    )
}

/// What the panel builds a row from: where the bookmark is in the reader's own list,
/// which its menu removes by, the bookmark, and the place it resolves to now.
type Listed = (usize, Bookmark, Option<Document>);

/// The Bookmarks list: every bookmark of the project, in the order the reader added them,
/// filtered on the whole name the way the History list is.
#[derive(PartialEq)]
pub(crate) struct BookmarksPanel;

impl Component for BookmarksPanel {
    fn render(&self) -> impl IntoElement {
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::Bookmarks);
        // What Enter on a row reaches through, consumed here because the handler that
        // uses them runs no hook.
        let doors = use_doors();
        let ctrl = use_consume::<Ctrl>().0;
        // A handful of rows, so the filter is applied where they are built -- but the one
        // compiled filter all the same, so the bar's error is what these rows were kept by.
        let marking = use_list_marking(filter);
        let marking = marking.read().clone();
        let matcher = marking.matcher();

        // Resolved where the rows are built, against the objects as they are now: reading
        // both is what re-resolves every row when a binary is opened or closed, which is
        // the whole of how a bookmark comes back to life. A handful of rows, so no memo.
        let (rows, listed, any): (Vec<Element>, Vec<Listed>, bool) = {
            let bookmarked = bookmarked.read();
            let objects = objects.read();
            let entries = bookmarked.entries();
            // What the rows are of, kept beside them: each row's bookmark, which is what
            // the arrows pick out, and the place it resolved to, which is what Enter
            // opens. A dead one keeps its row and opens nothing, as pressing it does.
            let listed: Vec<Listed> = entries
                .iter()
                .enumerate()
                .filter(|(_, bookmark)| matcher.matches(&bookmark.label()))
                .map(|(index, bookmark)| {
                    (
                        index,
                        bookmark.clone(),
                        bookmark.document.resolve_by_name(&objects),
                    )
                })
                .collect();
            let rows = listed
                .iter()
                .enumerate()
                .map(|(at, (index, bookmark, live))| {
                    BookmarkRow {
                        index: *index,
                        bookmark: bookmark.clone(),
                        live: live.clone(),
                        at,
                        marks: matcher.marks(&bookmark.label()),
                        key: DiffKey::None,
                    }
                    .key((*index, bookmark))
                    .into()
                })
                .collect();
            (rows, listed, !entries.is_empty())
        };
        let keys = ListKeys::over(
            listed,
            |(_, bookmark, _): &Listed| Pick::Bookmark(bookmark.clone()),
            move |(_, _, live): &Listed| match live {
                Some(live) => opened(doors, ctrl, live.clone()),
                None => Pressed::Folded,
            },
        );

        pane.filtered(
            filter,
            &marking,
            keys,
            pane.short_list(rows, any, "No bookmarks"),
        )
    }
}

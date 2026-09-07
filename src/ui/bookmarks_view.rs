//! The Bookmarks view: the project's own list of places, one row each, live against what is
//! loaded and kept when it is not.

use super::*;

/// One bookmark. Live when its place resolves against the objects loaded now, in which case
/// pressing it is a navigation like a press in the Symbols list; dead when it does not, in
/// which case it is drawn dimmed and does nothing, and is still there -- a reader's own list
/// does not shrink behind their back.
#[derive(Clone)]
struct BookmarkRow {
    index: usize,
    bookmark: Bookmark,
    live: Option<Document>,
    key: DiffKey,
}

impl PartialEq for BookmarkRow {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.bookmark == other.bookmark && self.live == other.live
    }
}

impl KeyExt for BookmarkRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for BookmarkRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let doors = use_doors();
        // Consumed and not read: a row hands the list an index back and draws nothing of
        // it that the tab has not already handed it.
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let index = self.index;
        let dead = self.live.is_none();

        // Drawn from the bookmark whether or not the place is live, so a row does not
        // change its spelling when its binary is closed.
        let label = self.bookmark.label();
        let text = match &self.bookmark.document {
            SavedDocument::Symbol { .. } => short_name(&label),
            _ => label.to_string(),
        };
        let tooltip = match &self.bookmark.document {
            SavedDocument::Source { path } => path.clone(),
            SavedDocument::Object {
                path,
                shown: SavedShown::Code,
                ..
            } => path.display().to_string(),
            _ => label.to_string(),
        };

        // A dead row has no handlers at all, like a dimmed history button: nothing to go
        // to, so nothing to light up for. Nothing is picked out in this list either -- a
        // bookmark is a place, not a selection.
        let row = match &self.live {
            Some(live) => {
                let live = live.clone();
                list_row(hovering, false).on_press(move |_| {
                    open_document(open, visits, live.clone(), reach(ctrl));
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
            .child(tree_name_fitted(fitted, text, dead)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// Which kind of place a saved document is, as the glyph its live tab would wear.
fn saved_icon(saved: &SavedDocument) -> Element {
    let (name, svg) = match saved {
        SavedDocument::Object {
            shown: SavedShown::Code,
            ..
        } => ("scroll-text", lucide::scroll_text()),
        SavedDocument::Object { .. } | SavedDocument::Symbol { .. } => ("binary", lucide::binary()),
        SavedDocument::Source { .. } => ("file-code", lucide::file_code()),
    };
    document_glyph((name, svg))
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

/// The menu a Symbols or History row opens on a right-click: [`bookmark_item`] and
/// nothing else.
pub(crate) fn bookmark_menu(
    bookmarked: State<Bookmarks>,
    objects: State<Vec<Arc<Object>>>,
    document: Document,
) -> Menu {
    Menu::new().child(bookmark_item(bookmarked, objects, document, "Add bookmark"))
}

/// The Bookmarks list: every bookmark of the project, in the order the reader added them,
/// filtered on the whole name the way the History list is.
#[derive(PartialEq)]
pub(crate) struct BookmarksPanel;

impl Component for BookmarksPanel {
    fn render(&self) -> impl IntoElement {
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let filter = use_state(Filter::default);
        let matcher = filter.read().matcher();

        // Resolved where the rows are built, against the objects as they are now: reading
        // both is what re-resolves every row when a binary is opened or closed, which is
        // the whole of how a bookmark comes back to life. A handful of rows, so no memo.
        let (rows, any): (Vec<Element>, bool) = {
            let bookmarked = bookmarked.read();
            let objects = objects.read();
            let entries = bookmarked.entries();
            let rows = entries
                .iter()
                .enumerate()
                .filter(|(_, bookmark)| matcher.matches(&bookmark.label()))
                .map(|(index, bookmark)| {
                    BookmarkRow {
                        index,
                        bookmark: bookmark.clone(),
                        live: bookmark.document.resolve_by_name(&objects),
                        key: DiffKey::None,
                    }
                    .key((index, bookmark))
                    .into()
                })
                .collect();
            (rows, !entries.is_empty())
        };

        use_filter_pane(
            filter,
            palette().symbol_pane_bg,
            match (any, rows.is_empty()) {
                (false, _) => placeholder("No bookmarks"),
                (true, true) => placeholder("No matches"),
                (true, false) => ScrollView::new()
                    .child(rect().width(Size::fill()).children(rows).into_element())
                    .into_element(),
            },
        )
    }
}

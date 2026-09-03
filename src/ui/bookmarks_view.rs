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
        let mut hovering = use_state(|| false);
        let open = use_open();
        // Consumed and not read: a row hands the list an index back and draws nothing of
        // it that the tab has not already handed it.
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let index = self.index;
        let live = self.live.clone();
        let dead = live.is_none();

        // Drawn from the stored name whether or not the place is live, so a row does not
        // change its spelling when its binary is closed.
        let text = match &self.bookmark.document {
            SavedDocument::Symbol { .. } => short_name(&self.bookmark.name),
            _ => self.bookmark.name.clone(),
        };
        let tooltip = match &self.bookmark.document {
            SavedDocument::Source { path } => path.clone(),
            SavedDocument::Code { path, .. } => path.display().to_string(),
            _ => self.bookmark.name.clone(),
        };

        let background = match hovering() && !dead {
            true => palette().symbol_hover_bg,
            false => Color::TRANSPARENT,
        };

        row_tooltip(
            tooltip,
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                // A dead row has no handlers at all, like a dimmed history button: nothing
                // to go to, so nothing to light up for.
                .maybe(!dead, move |row| {
                    let live = live.clone();
                    row.on_pointer_over(move |_| hovering.set_if_modified(true))
                        .on_pointer_out(move |_| hovering.set_if_modified(false))
                        .on_press(move |_| {
                            if let Some(live) = live.clone() {
                                open_document(open, visits, live, reach(ctrl));
                            }
                        })
                })
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, remove_menu(bookmarked, index));
                })
                .child(saved_icon(&self.bookmark.document))
                .child(tree_name(text, dead)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// Which kind of place a saved document is, as the glyph its live tab would wear.
fn saved_icon(saved: &SavedDocument) -> Element {
    let (name, svg) = match saved {
        SavedDocument::Object { .. } | SavedDocument::Symbol { .. } => ("binary", lucide::binary()),
        SavedDocument::Source { .. } => ("file-code", lucide::file_code()),
        SavedDocument::Code { .. } => ("scroll-text", lucide::scroll_text()),
    };
    document_glyph((name, svg))
}

/// The menu a bookmark row opens on a right-click: one item, removing that row. By index
/// and not by place, because a dead row is exactly the one that resolves to no place and
/// the one this is most wanted on. Built per press, as every menu here is.
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

/// The one menu item every bookmark gesture is: adding a bookmark of `document`, or
/// removing the one that points at it, whichever is true at the press. Which it is comes
/// from `Bookmarks::matching` -- by resolution, so a symbol that moved under a rebuild still
/// reads as bookmarked -- and the name a new one gets is the whole `entry_name`, what the
/// row's tooltip says. `add` is what the item says when there is none yet: a sidebar row
/// and a tab say "Add bookmark", an instruction row "Bookmark symbol", since the row is not
/// the symbol and has to say what it would bookmark. Built per press; the states come in
/// as arguments because no hook may run in an event handler.
pub(crate) fn bookmark_item(
    bookmarked: State<Bookmarks>,
    objects: State<Vec<Arc<Object>>>,
    document: Document,
    add: &'static str,
) -> MenuButton {
    let bookmarked_already = bookmarked
        .peek()
        .matching(&document, &objects.peek())
        .is_some();
    let text = match bookmarked_already {
        true => "Remove bookmark",
        false => add,
    };
    MenuButton::new()
        .on_press(move |_| {
            let mut bookmarked = bookmarked;
            // The objects are peeked before the list is written: two different states,
            // and the write wakes the panel.
            let loaded = objects.peek().clone();
            bookmarked
                .write()
                .toggle(&document, entry_name(&document), &loaded);
        })
        .child(text)
}

/// The menu a Symbols or History row opens on a right-click: the item above and nothing
/// else.
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
pub(crate) struct BookmarksTab;

impl Component for BookmarksTab {
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
                .filter(|(_, bookmark)| matcher.matches(&bookmark.name))
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

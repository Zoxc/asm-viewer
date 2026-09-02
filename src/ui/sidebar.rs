//! The three lists a reader browses a binary with -- Objects, Symbols and History -- and
//! the rows each is built out of.
//!
//! The Objects list is a **tree that is a shape in the data and never in the element
//! tree** -- a `VirtualScrollView` is told a length and asked for row *n*, so `tree.rs`
//! flattens the fold state into rows and this file only draws them. Each row is a
//! `Component` with its own hover state, there being no `.hover()` pseudo-state, and a row
//! and the view over it must agree about [`list_row_height`], or scrolling misaligns.

use super::*;

/// One opened file that contributed several objects -- an archive -- and the row its
/// members fold under. It has no `Object` behind it, so it selects nothing: pressing it
/// folds it open or shut.
#[derive(Clone)]
struct ArchiveRow {
    name: String,
    path: PathBuf,
    members: usize,
    expansion: Expansion,
    /// Whether objects may still be arriving out of this file. The tag column says so and
    /// the name is dimmed with it, rather than a spinner: a sidebar row is one of hundreds
    /// and none of the others move.
    loading: bool,
    /// The group this row is, in the tab's set of the groups the reader has opened.
    /// [`None`] for a file that has contributed nothing yet, there being nothing to fold.
    group: Option<usize>,
    expanded: State<HashSet<usize>>,
    key: DiffKey,
}

impl PartialEq for ArchiveRow {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.path == other.path
            && self.members == other.members
            && self.expansion == other.expansion
            && self.loading == other.loading
            && self.group == other.group
    }
}

impl KeyExt for ArchiveRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ArchiveRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut expanded = self.expanded;
        let group = self.group;
        let expansion = self.expansion;
        // Consumed here, in the render, because the handler that uses them may not run a
        // hook.
        let states = use_project_states();
        let path = self.path.clone();

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        // `Forced` draws no triangle, only the space one would have taken: the filter is
        // holding the file open and folding it would hide the rows the filter put on
        // screen. A row with no group has nothing behind it to fold.
        let chevron = match expansion {
            _ if self.group.is_none() => "",
            Expansion::Collapsed => "\u{25b8}",
            Expansion::Expanded => "\u{25be}",
            Expansion::Forced => "",
        };
        // Which format a file is is not known until it has been parsed.
        let tag = if self.loading {
            "\u{2026}"
        } else {
            ARCHIVE_TAG
        };

        row_tooltip(
            self.path.display().to_string(),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                // The name is the `flex` child taking what the fixed columns leave, which
                // torin only works out under `Content::Flex`.
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    // A file that has contributed no object yet has nothing to fold.
                    let Some(group) = group else {
                        return;
                    };
                    if expansion == Expansion::Forced {
                        return;
                    }
                    let mut expanded = expanded.write();
                    if !expanded.remove(&group) {
                        expanded.insert(group);
                    }
                })
                // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
                // without it panics.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                })
                .child(
                    label()
                        .text(chevron)
                        .width(Size::px(CHEVRON_WIDTH))
                        .color(palette().address_fg)
                        .max_lines(1),
                )
                .child(tag_label(tag))
                .child(tree_name(self.name.clone(), self.loading))
                // How many objects came out of this file, which under a filter is how many
                // of them matched -- the one thing about an archive that is not visible
                // while it is folded shut. A file that has produced nothing yet shows no
                // count rather than a zero.
                //
                // A column of its own, `COUNT_GUTTER` and all, rather than a label at the
                // end of the row: the count is measured whole before the name is handed
                // what the columns leave, so a sidebar dragged narrow ellipsises the name
                // and never eats the digits, and the ellipsis never runs into them.
                .child(
                    rect()
                        .padding(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))
                        .child(
                            label()
                                .text(if self.members == 0 {
                                    String::new()
                                } else {
                                    self.members.to_string()
                                })
                                .font_size(TAG_FONT_SIZE)
                                .color(palette().address_fg)
                                .max_lines(1),
                        ),
                ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One object: an archive member indented under its file, or a file that contributed
/// exactly one object and so is a row of its own.
#[derive(Clone)]
struct ObjectRow {
    object: Arc<Object>,
    selected: bool,
    /// Whether this object is one of several a file contributed. It decides the indent and
    /// what the tooltip says: a member's own name gets cut off, while a lone object is
    /// named after its file and the useful extra is where that file is.
    member: bool,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && self.selected == other.selected
            && self.member == other.member
    }
}

impl KeyExt for ObjectRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ObjectRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let (open, history) = (states.open, states.history);
        let object = self.object.clone();
        let path = self.object.path.clone();

        let background = if self.selected {
            palette().selected_bg
        } else if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        let tooltip = if self.member {
            self.object.name.clone()
        } else {
            self.object.path.display().to_string()
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
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                // What pressing an object opens is all of its code as one listing --
                // the one thing an object has to show that a symbol does not.
                .on_press(move |_| {
                    activate(
                        open,
                        history,
                        Some(Document::Code(object.clone())),
                        Visit::Went,
                    );
                })
                // A lone object *is* the file it came out of, so it closes like one. A
                // member was never opened on its own, and closing one would take the 195
                // rows beside it, so right-clicking one does nothing.
                .maybe(!self.member, move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                    })
                })
                // The column a file row's triangle sits in, kept empty so the tags of a
                // file and of a lone object line up; a member is indented past it.
                .child(rect().width(Size::px(if self.member {
                    CHEVRON_WIDTH + TREE_INDENT
                } else {
                    CHEVRON_WIDTH
                })))
                .child(tag_label(format_tag(self.object.format)))
                .child(tree_name(self.object.name.clone(), false)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
struct SymbolRow {
    symbols: SymbolList,
    index: usize,
    selected: bool,
    key: DiffKey,
}

impl PartialEq for SymbolRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
    }
}

impl KeyExt for SymbolRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SymbolRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;
        // Consumed, never read: 115k rows subscribed to the bookmarks would re-render the
        // whole list on every bookmark made.
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let symbol = self.symbols.0[self.index].clone();
        let text = symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone();
        let document = Document::Assembly(Selection::Symbol(symbol.clone()));

        let background = if self.selected {
            palette().selected_bg
        } else if hovering() {
            palette().symbol_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            text.clone(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    activate(
                        open,
                        history,
                        Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
                        Visit::Went,
                    );
                })
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(
                        &e,
                        bookmark_menu(bookmarked, objects, document.clone()),
                    );
                })
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One visited document in the history list. Clicking it moves the history cursor to this
/// entry rather than recording a new one, which is what `Nav::To` is for.
#[derive(Clone)]
struct HistoryRow {
    entry: Document,
    index: usize,
    /// Whether the cursor is on this entry, i.e. this is what is on screen.
    current: bool,
    key: DiffKey,
}

impl PartialEq for HistoryRow {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry && self.index == other.index && self.current == other.current
    }
}

impl KeyExt for HistoryRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for HistoryRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        // Consuming does not subscribe -- only reading would, and this row only hands an
        // index back to `navigate`.
        let history = use_consume::<Hist>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let index = self.index;
        let text = entry_text(&self.entry);
        let entry = self.entry.clone();

        let background = if self.current {
            palette().selected_bg
        } else if hovering() {
            palette().symbol_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            entry_tooltip(&self.entry),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| navigate(open, history, Nav::To(index)))
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(
                        &e,
                        bookmark_menu(bookmarked, objects, entry.clone()),
                    );
                })
                .child(entry_icon(&self.entry))
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(PartialEq)]
pub(crate) struct ObjectsTab;

impl Component for ObjectsTab {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;
        let filter = use_state(Filter::default);
        // Which files the reader has folded open: a view of a list and not part of the
        // session, so a `use_state` here. The set holds group keys, which are `Arc`
        // pointers, so an entry left behind by a closed file is harmless.
        let expanded = use_state(HashSet::<usize>::new);
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many rows
        // there are before it builds any of them. Reading `loading` here is what puts a
        // file on screen the moment it is asked for and takes the indicator off it when
        // the last of its objects has landed.
        let tree = use_memo(move || {
            ObjectTree::new(
                &objects.read(),
                &loading.read(),
                &filter.read().matcher(),
                &expanded.read(),
            )
        });
        let tree = tree.read().clone();
        // The selected object as the address its rows are keyed by: everything handed to a
        // `VirtualScrollView` has to be `PartialEq` and an `Object` is not, while pointer
        // identity compares as a number.
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Object(object)) | Document::Code(object)) => {
                Some(Arc::as_ptr(object).addr())
            }
            _ => None,
        };
        let length = tree.len();

        filter_pane(
            filter,
            palette().pane_bg,
            // `new_with_data`, never a capture: the builder closure is not compared across
            // renders.
            VirtualScrollView::new_with_data(
                (tree, selected, expanded),
                |row,
                 (tree, selected, expanded): &(
                    ObjectTree,
                    Option<usize>,
                    State<HashSet<usize>>,
                )| {
                    match tree.row(row) {
                        TreeRow::File {
                            name,
                            path,
                            group,
                            members,
                            expansion,
                            loading,
                        } => ArchiveRow {
                            name: name.clone(),
                            path: path.clone(),
                            members: *members,
                            expansion: *expansion,
                            loading: *loading,
                            group: *group,
                            expanded: *expanded,
                            key: DiffKey::None,
                        }
                        // The path as well as the group, since a file with nothing behind
                        // it yet has no group and the path is the only identity it has.
                        .key((*group, path))
                        .into(),
                        TreeRow::Object { object, member } => ObjectRow {
                            object: object.clone(),
                            selected: *selected == Some(Arc::as_ptr(object).addr()),
                            member: *member,
                            key: DiffKey::None,
                        }
                        .key(Arc::as_ptr(object).addr())
                        .into(),
                    }
                },
            )
            .length(length)
            .item_size(list_row_height()),
        )
    }
}

#[derive(PartialEq)]
pub(crate) struct SymbolsTab;

impl Component for SymbolsTab {
    fn render(&self) -> impl IntoElement {
        let symbols = use_consume::<Symbols>().0;
        let filter = use_state(Filter::default);
        // The one list where the filtering has to be a memo: 115k names on
        // `viewer-sample`, and the `VirtualScrollView` has to be told its length before it
        // builds any row.
        let filtered =
            use_memo(move || Filtered::new(symbols.read().clone(), &filter.read().matcher()));
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Symbol(symbol))) => Some(symbol.clone()),
            _ => None,
        };
        let length = filtered.len();

        filter_pane(
            filter,
            palette().symbol_pane_bg,
            VirtualScrollView::new_with_data(
                (filtered, selected),
                |row, (filtered, selected): &(Filtered, Option<Symbol>)| {
                    // The row's place in the filtered list is not the symbol's place in the
                    // list it was filtered out of, and everything below is about the
                    // symbol.
                    let index = filtered.index(row);
                    let symbol = &filtered.symbols.0[index];
                    SymbolRow {
                        symbols: filtered.symbols.clone(),
                        index,
                        selected: selected.as_ref() == Some(symbol),
                        key: DiffKey::None,
                    }
                    .key(Arc::as_ptr(&symbol.data).addr())
                    .into()
                },
            )
            .length(length)
            .item_size(list_row_height()),
        )
    }
}

#[derive(PartialEq)]
pub(crate) struct HistoryTab;

impl Component for HistoryTab {
    fn render(&self) -> impl IntoElement {
        let history = use_consume::<Hist>().0;
        let filter = use_state(Filter::default);
        // A session's history is a handful of entries, so it is filtered where the rows
        // are built rather than through a memo.
        let matcher = filter.read().matcher();

        // `visited` is asked of the whole history rather than of the rows, because an empty
        // list means two different things -- nowhere has been visited yet, or nothing
        // visited matches -- and the two are worth different words.
        let (rows, visited): (Vec<Element>, bool) = {
            let history = history.read();
            let cursor = history.cursor();
            let visited = history.recent().len() > 0;
            let rows = history
                .recent()
                // The whole name and not the shortened one the row draws: the generic
                // arguments a tab has no room for are still worth searching for.
                .filter(|(_, entry)| matcher.matches(&entry_name(entry)))
                .map(|(index, entry)| {
                    HistoryRow {
                        entry: entry.clone(),
                        index,
                        current: cursor == Some(index),
                        key: DiffKey::None,
                    }
                    .key((index, entry_key(entry)))
                    .into()
                })
                .collect();

            (rows, visited)
        };

        // A plain `ScrollView` rather than a `VirtualScrollView`: a handful of one-label
        // rows, built straight from the state instead of routed through `new_with_data`.
        filter_pane(
            filter,
            palette().symbol_pane_bg,
            match (visited, rows.is_empty()) {
                (false, _) => placeholder("Nothing visited yet"),
                (true, true) => placeholder("No matches"),
                (true, false) => ScrollView::new()
                    .child(rect().width(Size::fill()).children(rows).into_element())
                    .into_element(),
            },
        )
    }
}

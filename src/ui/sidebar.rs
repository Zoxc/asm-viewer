//! The three lists a reader browses a binary with -- Objects, Symbols and History -- the
//! Info pane beside them, and the rows each is built out of.
//!
//! Info is here rather than with the panes because it is a sidebar view over the same
//! selection the other three drive: it reads the state and draws no listing.
//!
//! The Objects list is a **tree that is a shape in the data and never in the element
//! tree** -- a `VirtualScrollView` is told a length and asked for row *n*, so `tree.rs`
//! flattens the fold state into rows and this file only draws them. Each list filters
//! itself, and each row is a `Component` with its own hover state, there being no
//! `.hover()` pseudo-state. A row and the view over it must agree about
//! [`list_row_height`], or scrolling misaligns.

use super::*;

/// One opened file that contributed several objects — an archive — and the row its
/// members fold under. It has no `Object` behind it, an `.a`/`.lib` not being one, so it
/// selects nothing: pressing it folds it open or shut, which is all a file row is for
/// until Step 6c decides what an object *is* to the selection.
#[derive(Clone)]
struct ArchiveRow {
    name: String,
    path: PathBuf,
    members: usize,
    expansion: Expansion,
    /// Whether objects may still be arriving out of this file, which is the whole of the
    /// indicator: the tag column says so and the name is dimmed with it, rather than a
    /// spinner, because a sidebar row is one of hundreds and none of the others move.
    loading: bool,
    /// The group this row is, in the tab's set of the groups the reader has opened.
    /// [`None`] for a file that has contributed nothing yet: there is nothing behind it to
    /// fold, so there is nothing for the set to hold either.
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
        // The states closing a file has to answer for. Consumed here, in the render,
        // because the handler that uses them may not run a hook.
        let states = use_project_states();
        let path = self.path.clone();

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        // `Forced` draws no triangle, only the space one would have taken: while the
        // filter is holding the file open, folding it would hide the very rows the filter
        // put on screen, so there is nothing here to press. See `Expansion::Forced`. A row
        // with no group is the same answer for the other reason -- there is nothing behind
        // it yet -- and the space keeps its tag lined up with the rest.
        let chevron = match expansion {
            _ if self.group.is_none() => "",
            Expansion::Collapsed => "\u{25b8}",
            Expansion::Expanded => "\u{25be}",
            Expansion::Forced => "",
        };
        // Which format a file is is not known until it has been parsed, so one still being
        // read wears the one tag that is true of it: it is being read.
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
                // The name is the `flex` child that takes what the three fixed columns
                // beside it leave, which torin only works out under `Content::Flex`.
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    // Nothing behind the row and nothing to fold: a file that has
                    // contributed no object yet.
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
                // The archive is a file the reader opened, so it is one they can close,
                // even though it selects nothing and has no `Object` behind it.
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
                // Dimmed while it is being read, which is the second half of the
                // indicator: the tag says what is happening and the colour says that the
                // row is not yet the whole answer.
                .child(tree_name(self.name.clone(), self.loading))
                // How many objects came out of this file, which under a filter is how
                // many of them matched. It is the one thing about an archive that is not
                // visible while it is folded shut. A file that has produced nothing yet
                // shows no count rather than a zero: the count says what is behind the
                // row, and "nothing, so far" is what the rest of the row already says.
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
    /// Whether this object is one of several a file contributed. It decides the indent,
    /// and it decides what the tooltip says: a member's own name is the thing that gets
    /// cut off, while a lone object is named after its file and the useful extra is
    /// where that file is.
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
                .on_press(move |_| {
                    activate(
                        open,
                        history,
                        Some(Document::Assembly(Selection::Object(object.clone()))),
                        Visit::Went,
                    );
                })
                // A lone object *is* the file it came out of, so it closes like one. A
                // member is not: it was never opened on its own, and the row that can
                // close the file it belongs to is the one above it. Right-clicking a
                // member therefore does nothing rather than quietly taking 195 rows the
                // reader was not pointing at with it.
                .maybe(!self.member, move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                    })
                })
                // The column a file row's triangle sits in, kept empty here so that the
                // tags of a file and of a lone object line up; a member is indented past
                // it instead.
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
        let symbol = self.symbols.0[self.index].clone();
        let text = symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone();

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
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One visited document in the history list. Clicking it moves the history cursor to
/// this entry rather than recording a new one, which is what `Nav::To` is for.
///
/// A visited *source file* is an entry like any function, which is the whole of what
/// Step 1e asked of this list: the history records documents, so it can list one, and the
/// row wears the same kind icon its tab does.
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
        // `Document`'s own `PartialEq` is written in terms of `Arc::ptr_eq` for a place
        // in a binary and of text for a file.
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
        // Consuming does not subscribe -- only reading would, and this row never reads
        // the history; it only hands an index back to `navigate`.
        let history = use_consume::<Hist>().0;
        let index = self.index;
        let text = entry_text(&self.entry);

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
                .child(entry_icon(&self.entry))
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

fn symbol_info(symbol: &Symbol) -> impl IntoElement {
    let data = &symbol.data;

    rect()
        .width(Size::fill())
        .child(info_line(format!("Symbol: `{}`", data.name)))
        .maybe_child(
            data.demangled
                .as_ref()
                .map(|demangled| info_line(format!("Demangled: `{}`", demangled))),
        )
        .maybe_child(
            data.section
                .as_ref()
                .map(|section| info_line(format!("Section: `{}`", section.name))),
        )
        .child(info_line(format!("Declared size: {} bytes", data.size)))
        // The declared size above is frequently 0 and is only ever displayed; what the
        // app actually reads is `extent`, so that is the number worth showing beside it.
        // `data_in` rather than `data`: the latter is the next-symbol estimate on its own,
        // which is not the range `assembly` decodes or `line_info` is asked about.
        .child(info_line(format!(
            "Extent: {} bytes",
            data.data_in(&symbol.object)
                .map(|bytes| bytes.len())
                .unwrap_or_default()
        )))
}

#[derive(PartialEq)]
pub(crate) struct ObjectsTab;

impl Component for ObjectsTab {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;
        let filter = use_state(Filter::default);
        // Which files the reader has folded open. It belongs to the tab exactly the way
        // the filter does — a fold is a view of a list, not part of the session — so it
        // is a `use_state` here and nothing about it reaches `project.rs`. The set holds
        // group keys, which are `Arc` pointers (see `TreeRow::File`), so an entry left
        // behind by a file that has since been closed is harmless: nothing looks it up
        // again.
        let expanded = use_state(HashSet::<usize>::new);
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many
        // rows there are before it builds any of them, and the answer depends on the
        // filter *and* on which files are open. It is tens of names rather than the
        // symbol list's hundred thousand, but the length has to come from somewhere and
        // that somewhere is the flattened tree.
        // Reading `loading` here is what puts a file on screen the moment it is asked for
        // and takes the indicator off it when the last of its objects has landed: the memo
        // follows the list of files being read exactly as it follows the objects.
        let tree = use_memo(move || {
            ObjectTree::new(
                &objects.read(),
                &loading.read(),
                &filter.read().matcher(),
                &expanded.read(),
            )
        });
        let tree = tree.read().clone();
        // The selected object as the address its rows are keyed by, rather than as the
        // `Arc` itself: everything handed to a `VirtualScrollView` has to be `PartialEq`
        // and an `Object` is not, while pointer identity — which is the only identity the
        // UI uses anyway — compares as a number.
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Object(object))) => Some(Arc::as_ptr(object).addr()),
            _ => None,
        };
        let length = tree.len();

        filter_pane(
            filter,
            palette().pane_bg,
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
                        // The two agree for every row that has both: one file is one row.
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
        // The one list where the filtering has to be a memo. It is 115k names on
        // `viewer-sample`, so the pass belongs to a change of the list or of the filter
        // rather than to a render — and the rows cannot each test themselves either, since
        // the `VirtualScrollView` has to be told its length before it builds any of them.
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
                    // The row's place in the filtered list is not the symbol's place in
                    // the list it was filtered out of, and everything below — the key, the
                    // selection, `SymbolRow` itself — is about the symbol.
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
pub(crate) struct InfoTab;

impl Component for InfoTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Active>().0.read().clone();

        match &current {
            None => placeholder("Nothing selected"),
            Some(Document::Source(_)) => placeholder("No symbol selected"),
            Some(Document::Assembly(Selection::Object(object))) => rect()
                .expanded()
                .background(palette().pane_bg)
                .child(info_line(format!("Object: `{}`", object.name)))
                .child(info_line(format!("Format: {:?}", object.format)))
                .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
                .into(),
            Some(Document::Assembly(Selection::Symbol(symbol))) => rect()
                .expanded()
                .background(palette().pane_bg)
                .child(ScrollView::new().child(symbol_info(symbol).into_element()))
                .into(),
        }
    }
}

#[derive(PartialEq)]
pub(crate) struct HistoryTab;

impl Component for HistoryTab {
    fn render(&self) -> impl IntoElement {
        let history = use_consume::<Hist>().0;
        let filter = use_state(Filter::default);
        // A session's history is a handful of entries, so this is the objects list's
        // arrangement and not the symbol list's: filtered where the rows are built.
        let matcher = filter.read().matcher();

        // Reading subscribes this tab to the history, so a recorded entry or a moved
        // cursor re-renders the list and nothing else. `visited` is asked of the whole
        // history rather than of the rows, because an empty list means two different
        // things — nowhere has been yet, or nowhere that has been matches — and the two
        // are worth different words.
        let (rows, visited): (Vec<Element>, bool) = {
            let history = history.read();
            let cursor = history.cursor();
            let visited = history.recent().len() > 0;
            let rows = history
                .recent()
                .filter(|(_, entry)| matcher.matches(&entry_text(entry)))
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

        // A plain `ScrollView` rather than a `VirtualScrollView`: a session's history is
        // a handful of entries, the rows are one label each, and this way the list is
        // built straight from the state it read instead of having to route the entries
        // through `new_with_data`. The same shape the objects list uses.
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

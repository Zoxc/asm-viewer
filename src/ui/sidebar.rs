//! The three lists a reader browses a binary with -- Objects, Symbols and History -- and
//! the rows each is built out of.
//!
//! The Objects list is a **tree that is a shape in the data and never in the element
//! tree** -- a `VirtualScrollView` is told a length and asked for row *n*, so `tree.rs`
//! flattens the fold state into rows and this file only draws them. Each row is a
//! `Component` with its own hover state, there being no `.hover()` pseudo-state, and each
//! is framed by [`list_row`] -- which is where a row and the view over it agree about
//! [`list_row_height`], as they must or scrolling misaligns.

use super::*;

/// Fold the file row's group away, or open it: what pressing an archive row does, and
/// what Enter on one does. A row the filter is holding open (`Forced`) is left alone,
/// since folding it would hide the rows the filter put on screen.
fn fold_archive(
    mut expanded: State<HashSet<usize>>,
    group: usize,
    expansion: Expansion,
) -> Pressed {
    if expansion == Expansion::Forced {
        return Pressed::Folded;
    }
    let mut expanded = expanded.write();
    if !expanded.remove(&group) {
        expanded.insert(group);
    }
    Pressed::Folded
}

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
    group: usize,
    expanded: State<HashSet<usize>>,
    /// Where this row is in the list as it is drawn, which is what the arrows step and
    /// what a press writes down with the pick (`ui/picks.rs`).
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
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
            && self.at == other.at
            && self.marks == other.marks
    }
}

impl KeyExt for ArchiveRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ArchiveRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let at = self.at;
        let expanded = self.expanded;
        let group = self.group;
        let expansion = self.expansion;
        // Consumed here, in the render, because the handler that uses them may not run a
        // hook.
        let states = use_project_states();
        let picking = use_picking(Panel::Objects);
        let path = self.path.clone();
        let pick = Pick::Path(self.path.clone());

        // `Forced` draws no triangle, only the space one would have taken: the filter is
        // holding the file open and folding it would hide the rows the filter put on
        // screen.
        let open = match expansion {
            Expansion::Collapsed => Some(false),
            Expansion::Expanded => Some(true),
            Expansion::Forced => None,
        };
        // Which format a file is is not known until it has been parsed.
        let tag = if self.loading {
            "\u{2026}"
        } else {
            ARCHIVE_TAG
        };

        extra_tooltip(
            self.path.display().to_string(),
            // An archive row has no object behind it, so nothing about the tab on screen
            // ever picks one out: it lights when the reader pressed it and not otherwise.
            list_row(hovering, picking.drawn(&pick, false))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || {
                        fold_archive(expanded, group, expansion)
                    });
                })
                // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
                // without it panics.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                })
                .child(disclosure(open))
                .child(tag_label(tag))
                .child(tree_name(self.name.clone(), self.loading, &self.marks))
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

/// A file that has been asked for and has produced nothing yet. It is a row so that the
/// reader can see the file was opened and close it again, and there is nothing under it to
/// fold: no triangle, no count, and `\u{2026}` where the format tag goes, since what a file
/// is is not known until it has been parsed.
#[derive(Clone, PartialEq)]
struct PendingRow {
    name: String,
    path: PathBuf,
    /// Where this row is in the list as it is drawn, which is what the arrows step and
    /// what a press writes down with the pick (`ui/picks.rs`).
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

impl KeyExt for PendingRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for PendingRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let at = self.at;
        // Consumed here, in the render, because the handler that uses them may not run a
        // hook.
        let states = use_project_states();
        let picking = use_picking(Panel::Objects);
        let path = self.path.clone();
        let pick = Pick::Path(self.path.clone());

        extra_tooltip(
            self.path.display().to_string(),
            // Nothing behind the row to open, so a press only picks it out.
            list_row(hovering, picking.drawn(&pick, false))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || Pressed::Folded);
                })
                // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
                // without it panics.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                })
                .child(disclosure(None))
                .child(tag_label("\u{2026}"))
                // Dimmed, and the tag beside it, rather than a spinner: a sidebar row is one
                // of hundreds and none of the others move.
                .child(tree_name(self.name.clone(), true, &self.marks))
                // The count column every tree row keeps, empty: a file that has produced
                // nothing shows no count rather than a zero.
                .child(rect().padding(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))),
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
    /// Where this row is in the list as it is drawn.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && self.selected == other.selected
            && self.member == other.member
            && self.at == other.at
            && self.marks == other.marks
    }
}

impl KeyExt for ObjectRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ObjectRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let states = use_project_states();
        let (open, visits) = (states.open, states.visits);
        let ctrl = use_consume::<Ctrl>().0;
        let picking = use_picking(Panel::Objects);
        let pick = Pick::Object(self.object.clone());
        let at = self.at;
        let object = self.object.clone();
        let path = self.object.path.clone();

        let tooltip = if self.member {
            self.object.name.clone()
        } else {
            self.object.path.display().to_string()
        };

        name_tooltip(
            fitted.cut(),
            &self.object.name,
            tooltip,
            list_row(hovering, picking.drawn(&pick, self.selected))
                // What pressing an object opens is all of its code as one listing --
                // the one thing an object has to show that a symbol does not. A row is
                // a click from outside the panes: a preview, or a tab of its own with
                // Ctrl. With Alt it opens nothing and the row is only picked out.
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || {
                        open_document(
                            open,
                            visits,
                            Document::Code(object.clone()),
                            Reach::outside(ctrl),
                        );
                        Pressed::Opened
                    });
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
                    chevron_width() + TREE_INDENT
                } else {
                    chevron_width()
                })))
                .child(tag_label(format_tag(self.object.format)))
                .child(tree_name_fitted(
                    fitted,
                    self.object.name.clone(),
                    false,
                    &self.marks,
                )),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
struct SymbolRow {
    symbols: Shared<Symbol>,
    /// Which symbol this is, in the list the filter narrowed.
    index: usize,
    selected: bool,
    /// Where this row is in the list as it is drawn, which under a filter is not `index`.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

impl PartialEq for SymbolRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
            && self.at == other.at
            && self.marks == other.marks
    }
}

impl KeyExt for SymbolRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SymbolRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let doors = use_doors();
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        // Consumed, never read: 115k rows subscribed to the bookmarks would re-render the
        // whole list on every bookmark made.
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let picking = use_picking(Panel::Symbols);
        let at = self.at;
        let symbol = self.symbols[self.index].clone();
        let pick = Pick::Symbol(symbol.clone());
        let text = symbol.data.display().to_owned();
        let document = Document::Assembly(Selection::Symbol(symbol));

        cut_tooltip(
            fitted.cut(),
            text.clone(),
            list_row(hovering, picking.drawn(&pick, self.selected))
                .on_press({
                    let document = document.clone();
                    move |_| {
                        picking.press(pick.clone(), at, || {
                            open_document(open, visits, document.clone(), Reach::outside(ctrl));
                            Pressed::Opened
                        });
                    }
                })
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(
                        &e,
                        bookmark_menu(bookmarked, objects, document.clone()),
                    );
                })
                .child(tree_name_fitted(fitted, text, false, &self.marks)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One visited place in the History list. Clicking it is a click from outside the panes
/// like any other row's: the place opens in the temporal tab, or the tab already showing
/// it is raised, and the visit goes to the top of the list.
#[derive(Clone)]
struct HistoryRow {
    entry: Document,
    /// Whether this is what the tab on screen shows.
    current: bool,
    /// Where this row is in the list as it is drawn.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    key: DiffKey,
}

impl PartialEq for HistoryRow {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry
            && self.current == other.current
            && self.at == other.at
            && self.marks == other.marks
    }
}

impl KeyExt for HistoryRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for HistoryRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let doors = use_doors();
        // Consuming does not subscribe -- only reading would, and this row only records
        // into it.
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        let picking = use_picking(Panel::History);
        let at = self.at;
        // One build of the name for both spellings: the row draws the short one and its
        // tooltip says the whole one (`entry_labels`).
        let (text, tooltip) = entry_labels(&self.entry);
        let entry = self.entry.clone();
        let target = self.entry.clone();
        let pick = Pick::Visit(self.entry.clone());

        let drawn = text.clone();

        name_tooltip(
            fitted.cut(),
            &drawn,
            tooltip,
            list_row(hovering, picking.drawn(&pick, self.current))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || {
                        open_document(open, visits, target.clone(), Reach::outside(ctrl));
                        Pressed::Opened
                    });
                })
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(
                        &e,
                        bookmark_menu(bookmarked, objects, entry.clone()),
                    );
                })
                .child(entry_icon(&self.entry))
                .child(tree_name_fitted(fitted, text, false, &self.marks)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(PartialEq)]
pub(crate) struct ObjectsPanel;

/// The control at the top of the Objects panel: what the top bar's Open button was.
///
/// It moved here because this is the list it adds to. The bar is about the *project* --
/// which one is open, and what becomes of it -- and a binary is one of the things a project
/// holds, so the place to add one is over the panel that lists them. It is a button and not
/// a menu item for the same reason the Files view's Open is one: adding a binary is a thing
/// a reader does over and over while they work.
#[derive(PartialEq)]
struct AddBinaries;

impl Component for AddBinaries {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;

        rect()
            .width(Size::fill())
            .horizontal()
            .padding(Gaps::new(4.0, 4.0, 0.0, 4.0))
            .child(
                Button::new()
                    .on_press(move |_| {
                        // `spawn_forever`, not `spawn`: the dialog is not modal to the
                        // window, so the reader can drag this panel elsewhere in the dock
                        // while it is up -- and that unmounts the scope a `spawn` would
                        // belong to, losing the files they then chose.
                        spawn_forever(async move {
                            let Some(handles) = AsyncFileDialog::new()
                                .set_title("Add binaries to the project...")
                                .pick_files()
                                .await
                            else {
                                return;
                            };
                            let paths: Vec<PathBuf> =
                                handles.iter().map(|h| h.path().to_path_buf()).collect();
                            open_binaries(objects, loading, paths).await;
                        });
                    })
                    .child("Add binaries..."),
            )
    }
}

impl Component for ObjectsPanel {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::Objects);
        // What Enter on a row reaches through, consumed here because the handler that
        // uses them runs no hook.
        let doors = use_doors();
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
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
            Some((
                _,
                Stop {
                    document: Document::Assembly(Selection::Object(object)) | Document::Code(object),
                    ..
                },
            )) => Some(Arc::as_ptr(object).addr()),
            _ => None,
        };
        let length = tree.len();
        // What the rows mark in the names they draw, memoized on the filter beside the
        // tree above, which compiles one of its own to narrow the list with.
        let marking = use_list_marking(filter);
        // One more clone of the rows, shared by the two closures the keys are: the arrows
        // ask what a row is and Enter asks what pressing one does, and both are the tree
        // the panel is drawing and not one worked out again.
        let rows = Rc::new(tree.clone());
        let keys = ListKeys {
            length,
            at: {
                let rows = rows.clone();
                Box::new(move |at| {
                    (at < rows.len()).then(|| match &rows[at] {
                        TreeRow::File { path, .. } | TreeRow::Pending { path, .. } => {
                            Pick::Path(path.clone())
                        }
                        TreeRow::Object { object, .. } => Pick::Object(object.clone()),
                    })
                })
            },
            open: Box::new(move |at| {
                if at >= rows.len() {
                    return Pressed::Folded;
                }
                match &rows[at] {
                    TreeRow::File {
                        group, expansion, ..
                    } => fold_archive(expanded, *group, *expansion),
                    // Nothing under it to fold and nothing behind it to open.
                    TreeRow::Pending { .. } => Pressed::Folded,
                    TreeRow::Object { object, .. } => {
                        open_document(
                            open,
                            visits,
                            Document::Code(object.clone()),
                            Reach::outside(ctrl),
                        );
                        Pressed::Opened
                    }
                }
            }),
        };

        let pane = pane.filtered(
            filter,
            keys,
            // `new_with_data`, never a capture: the builder closure is not compared across
            // renders.
            VirtualScrollView::new_with_data(
                (tree, selected, expanded, marking),
                |row,
                 (tree, selected, expanded, marking): &(
                    ObjectTree,
                    Option<usize>,
                    State<HashSet<usize>>,
                    Marking,
                )| {
                    match &tree[row] {
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
                            at: row,
                            marks: marking.marks(name),
                            key: DiffKey::None,
                        }
                        .key(*group)
                        .into(),
                        // Keyed by the path, the only identity a file with nothing behind
                        // it yet has.
                        TreeRow::Pending { name, path } => PendingRow {
                            name: name.clone(),
                            path: path.clone(),
                            at: row,
                            marks: marking.marks(name),
                            key: DiffKey::None,
                        }
                        .key(path)
                        .into(),
                        TreeRow::Object { object, member } => ObjectRow {
                            object: object.clone(),
                            selected: *selected == Some(Arc::as_ptr(object).addr()),
                            member: *member,
                            at: row,
                            marks: marking.marks(&object.name),
                            key: DiffKey::None,
                        }
                        .key(Arc::as_ptr(object).addr())
                        .into(),
                    }
                },
            )
            .length(length)
            .item_size(list_row_height())
            .scroll_controller(pane.controller),
        );

        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(AddBinaries)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(pane),
            )
    }
}

#[derive(PartialEq)]
pub(crate) struct SymbolsPanel;

impl Component for SymbolsPanel {
    fn render(&self) -> impl IntoElement {
        let symbols = use_consume::<Symbols>().0;
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::Symbols);
        // What Enter on a row reaches through, consumed here because the handler that
        // uses them runs no hook.
        let doors = use_doors();
        let (open, visits) = (doors.open, doors.visits);
        let ctrl = use_consume::<Ctrl>().0;
        // The one list where the filtering has to be a memo: 115k names on
        // `viewer-sample`, and the `VirtualScrollView` has to be told its length before it
        // builds any row.
        let filtered = use_memo(move || {
            let symbols = symbols.read().clone();
            Filtered::new(symbols, &filter.read().matcher(), |symbol| {
                symbol.data.display()
            })
        });
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Active>().0.read() {
            Some((
                _,
                Stop {
                    document: Document::Assembly(Selection::Symbol(symbol)),
                    ..
                },
            )) => Some(symbol.clone()),
            _ => None,
        };
        let length = filtered.len();
        // What the rows mark in the names they draw, memoized on the filter beside the
        // list above.
        let marking = use_list_marking(filter);
        // Cheap to hand to both closures: a `Filtered` is the list behind an `Arc` and
        // the indices the filter kept.
        let rows = Rc::new(filtered.clone());
        let symbol_at = move |rows: &Filtered<Symbol>, at: usize| rows.at(at).cloned();
        let stepped = rows.clone();
        let keys = ListKeys {
            length,
            at: Box::new(move |at| symbol_at(&stepped, at).map(Pick::Symbol)),
            open: Box::new(move |at| match symbol_at(&rows, at) {
                Some(symbol) => {
                    open_document(
                        open,
                        visits,
                        Document::Assembly(Selection::Symbol(symbol)),
                        Reach::outside(ctrl),
                    );
                    Pressed::Opened
                }
                None => Pressed::Folded,
            }),
        };

        // An empty list means the same two things here as in `short_list`, and the whole
        // list says which: no symbols at all draws the empty view, a filter that left
        // nothing of them says so.
        if length == 0 && !filtered.list().is_empty() {
            return pane.filtered(filter, keys, placeholder("No matches"));
        }

        pane.filtered(
            filter,
            keys,
            VirtualScrollView::new_with_data(
                (filtered, selected, marking),
                |row, (filtered, selected, marking): &(Filtered<Symbol>, Option<Symbol>, Marking)| {
                    // The row's place in the filtered list is not the symbol's place in the
                    // list it was filtered out of, and everything below is about the
                    // symbol.
                    let index = filtered.index(row);
                    let symbol = &filtered.list()[index];
                    SymbolRow {
                        symbols: filtered.list().clone(),
                        index,
                        selected: selected.as_ref() == Some(symbol),
                        at: row,
                        marks: marking.marks(symbol.data.display()),
                        key: DiffKey::None,
                    }
                    .key(Arc::as_ptr(&symbol.data).addr())
                    .into()
                },
            )
            .length(length)
            .item_size(list_row_height())
            .scroll_controller(pane.controller),
        )
    }
}

#[derive(PartialEq)]
pub(crate) struct HistoryPanel;

impl Component for HistoryPanel {
    fn render(&self) -> impl IntoElement {
        let visits = use_project_states().visits;
        // The place the tab on screen shows is the row marked, the way the Symbols list
        // marks its symbol: the record itself has no cursor, the tabs having theirs.
        let current = use_consume::<Active>()
            .0
            .read()
            .clone()
            .map(|(_, stop)| stop.document);
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::History);
        // What Enter on a row reaches through, consumed here because the handler that
        // uses them runs no hook.
        let open = use_open();
        let ctrl = use_consume::<Ctrl>().0;
        // A session's record is a couple of hundred places at most, so it is filtered
        // where the rows are built rather than through a memo.
        let matcher = filter.read().matcher();

        // `visited` is asked of the whole record and not of the rows: no rows means
        // either of the two things `short_list` has a word for.
        let (rows, listed, visited): (Vec<Element>, Vec<Document>, bool) = {
            let visits = visits.read();
            let visited = !visits.entries().is_empty();
            // The places the rows are of, with the name each row draws beside it: a
            // couple of hundred at most, and what the arrows step and Enter opens. Both
            // spellings are built once here, the filter reading the whole one and the row
            // marking the short one it draws.
            let kept: Vec<(Document, String)> = visits
                .entries()
                .iter()
                .filter_map(|entry| {
                    let (text, whole) = entry_spellings(entry);
                    // The whole name and not the shortened one the row draws: the generic
                    // arguments a tab has no room for are still worth searching for.
                    matcher.matches(&whole).then(|| (entry.clone(), text))
                })
                .collect();
            let rows = kept
                .iter()
                .enumerate()
                .map(|(at, (entry, text))| {
                    HistoryRow {
                        entry: entry.clone(),
                        current: current.as_ref() == Some(entry),
                        at,
                        marks: matcher.marks(text),
                        key: DiffKey::None,
                    }
                    .key(entry_key(entry))
                    .into()
                })
                .collect();

            let listed = kept.into_iter().map(|(entry, _)| entry).collect();

            (rows, listed, visited)
        };
        let keys = {
            let stepped = listed.clone();
            ListKeys {
                length: listed.len(),
                at: Box::new(move |at| stepped.get(at).cloned().map(Pick::Visit)),
                open: Box::new(move |at| match listed.get(at) {
                    Some(entry) => {
                        open_document(open, visits, entry.clone(), Reach::outside(ctrl));
                        Pressed::Opened
                    }
                    None => Pressed::Folded,
                }),
            }
        };

        pane.filtered(
            filter,
            keys,
            short_list(pane.controller, rows, visited, "Nothing visited yet"),
        )
    }
}

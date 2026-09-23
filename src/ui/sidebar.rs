//! The three lists a reader browses a binary with -- Objects, Symbols and History -- and
//! the rows each is built out of.
//!
//! The Objects list is a **tree that is a shape in the data and never in the element
//! tree** -- a `VirtualScrollView` is told a length and asked for row *n*, so `tree.rs`
//! flattens the fold state into rows and this file only draws them. Each row is a
//! `Component` with its own hover state, there being no `.hover()` pseudo-state, and each
//! is framed by [`list_row`] -- which is where a row and the view over it agree about
//! [`list_row_height`], as they must or scrolling misaligns.
//!
//! The Symbols list is drawn in **two** panels: here, over every object's symbols, and in
//! the Locations panel, over the ones a line was compiled into (`ui/locations.rs`). One
//! [`SymbolRow`], one filtered list and one set of keys for both, with [`SymbolPress`] the
//! whole of what a panel says about its own.

use super::*;

/// Fold the file row at `path` away, or open it: what pressing an archive row does, and
/// what Enter on one does. A row the filter is holding open (`Forced`) is left alone,
/// since folding it would hide the rows the filter put on screen.
fn fold_archive(
    mut expanded: State<HashSet<PathBuf>>,
    path: &Path,
    expansion: Expansion,
) -> Pressed {
    if expansion == Expansion::Forced {
        return Pressed::Folded;
    }
    let mut expanded = expanded.write();
    if !expanded.remove(path) {
        expanded.insert(path.to_path_buf());
    }
    Pressed::Folded
}

/// The part of a file's row that a file still being read has not got: how many objects
/// came out of it, which way it is folded now, and the paths of the files the reader has
/// folded open.
#[derive(Clone, Copy, PartialEq)]
struct Folds {
    members: usize,
    expansion: Expansion,
    expanded: State<HashSet<PathBuf>>,
}

/// One opened file, and the row its objects fold under. It has no `Object` behind it, so
/// it selects nothing: pressing it folds it open or shut.
///
/// **A file still working on its first object is the same row with `folds: None`**: no
/// triangle, no count, and the loading tag. It was a component of its own once -- the same
/// fifty-five lines with three values fixed -- which is two right-click handlers to keep in
/// step and two spellings of the tag. `TreeRow::Pending` stays a variant of its own all the
/// same: that argument is about the model, a file with nothing behind it having no group
/// key and nothing to fold (`agents/Sidebar.md`).
#[derive(Clone, PartialEq)]
struct ArchiveRow {
    name: String,
    path: PathBuf,
    folds: Option<Folds>,
    /// Whether objects may still be arriving out of this file. The tag column says so and
    /// the name is dimmed with it, rather than a spinner: a sidebar row is one of hundreds
    /// and none of the others move.
    loading: bool,
    /// Where this row is in the list as it is drawn, which is what the arrows step and
    /// what a press writes down with the pick (`ui/picks.rs`).
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    /// What this row's press and its menu reach for, told to it by the list: see
    /// [`ListStates`]. Compares equal always, so it costs the row no render.
    states: ListStates,
    key: DiffKey,
}

keyed!(ArchiveRow);

impl Component for ArchiveRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let at = self.at;
        let folds = self.folds;
        let picking = self.states.picking;
        let states = self.states.project;
        let path = self.path.clone();
        let fold_path = self.path.clone();
        let pick = Pick::Path(self.path.clone());

        // `Forced` draws no triangle, only the space one would have taken: the filter is
        // holding the file open and folding it would hide the rows the filter put on
        // screen. Nor does a file with nothing under it yet, which has nothing to fold.
        let open = match folds.map(|folds| folds.expansion) {
            Some(Expansion::Collapsed) => Some(false),
            Some(Expansion::Expanded) => Some(true),
            Some(Expansion::Forced) | None => None,
        };
        // Which format a file is is not known until it has been parsed.
        let tag = if self.loading {
            "\u{2026}"
        } else {
            ARCHIVE_TAG
        };
        // How many objects came out of this file, which under a filter is how many of them
        // matched. A file that has produced nothing yet shows no count rather than a zero.
        let count = match folds.map_or(0, |folds| folds.members) {
            0 => None,
            members => Some(members),
        };

        extra_tooltip(
            self.path.display().to_string(),
            // An archive row has no object behind it, so nothing about the tab on screen
            // ever picks one out: it lights when the reader pressed it and not otherwise.
            list_row(hovering, picking.drawn(&pick, false))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || match folds {
                        Some(folds) => fold_archive(folds.expanded, &fold_path, folds.expansion),
                        // Nothing under it to fold and nothing behind it to open.
                        None => Pressed::Folded,
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
                // The count -- the one thing about an archive that is not visible while it
                // is folded shut -- in the column `count_column` draws, which every row of
                // a list that counts anything keeps, empty or not.
                .child(count_column(count)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
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
    /// What this row's press and its menu reach for, told to it by the list: see
    /// [`ListStates`].
    states: ListStates,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && self.selected == other.selected
            && self.member == other.member
            && self.at == other.at
            && self.marks == other.marks
        // `states` compares equal always -- handles the root never replaces -- so it is
        // left out.
    }
}

keyed!(ObjectRow);

impl Component for ObjectRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let ListStates {
            picking,
            doors,
            ctrl,
            project: states,
            ..
        } = self.states;
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
                        opened(doors, ctrl, Document::Code(object.clone()))
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
        self.keyed()
    }
}

/// One symbol in a filtered list: the name marked, and the object it is in after it
/// where the list names one.
///
/// **Both lists of symbols are this row** -- the Symbols panel's of every object's, and
/// the Locations panel's of the ones a line was compiled into -- with [`SymbolPress`] the
/// whole of what differs between them.
#[derive(Clone, PartialEq)]
struct SymbolRow {
    symbols: Shared<Symbol>,
    /// Which symbol this is, in the list the filter narrowed.
    index: usize,
    selected: bool,
    /// Where this row is in the list as it is drawn, which under a filter is not `index`.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    /// Which of the two lists this is a row of.
    press: SymbolPress,
    /// What this row's press and its menu reach for, told to it by the list: see
    /// [`ListStates`]. Each of the Symbols list's 115k rows reached for eight contexts a
    /// render before it was handed one.
    states: ListStates,
    key: DiffKey,
}

/// Which filtered symbol list a row is in: what pressing it does, and what it draws after
/// the name. Which panel's pick it answers to is the pane's and not said here.
///
/// A prop and not a reading of anything, so a row and the panel's Enter are handed the
/// same one and cannot open different places.
#[derive(Clone, PartialEq)]
pub(crate) enum SymbolPress {
    /// The Symbols panel: the symbol opens as a tab of its own, and a right-click offers
    /// to bookmark it.
    Open,
    /// The Locations panel: the symbol opens on the line the answer was about
    /// ([`press_location`]), and the row draws the object it is in, since the same name in
    /// two objects is two rows and the object is what tells them apart.
    Located {
        /// The answer's own line, and [`None`] where it named none: what a press opens the
        /// symbol on.
        asked_at: Option<LinePos>,
        /// The source-driven tab the question was asked from, whose entry a press writes
        /// the choice under.
        subject: Option<Subject>,
    },
}

impl SymbolPress {
    /// Whether the row names the object the symbol is in after the name.
    fn about(&self) -> bool {
        matches!(self, SymbolPress::Located { .. })
    }

    /// Whether a right-click on the row offers to bookmark the symbol. A bookmark is a
    /// place the reader means to come back to, which is the symbol itself; a location row
    /// is one answer to a question about a line, and offers none.
    fn bookmarks(&self) -> bool {
        matches!(self, SymbolPress::Open)
    }

    /// What pressing the row for `symbol` does, which is what Enter on the row the arrows
    /// left the pick on does.
    fn goes(&self, to: ListStates, symbol: Symbol) -> Pressed {
        match self {
            SymbolPress::Open => opened(to.doors, to.ctrl, Document::Symbol(symbol)),
            SymbolPress::Located { asked_at, subject } => {
                press_location(to, asked_at.clone(), subject.clone(), symbol)
            }
        }
    }
}

keyed!(SymbolRow);

impl Component for SymbolRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        // The two texts a row can draw, each measured: the name, and the object it is in.
        // The second is taken whatever the row draws, a hook having to be called every
        // render, and nothing attaches to it where the row draws no object, so it never
        // says it was cut.
        let (named, about) = (use_fitted(), use_fitted());
        // Every door a press of either list goes through, and the two states the menu
        // needs. Never read: 115k rows subscribed to the bookmarks would re-render the
        // whole list on every bookmark made.
        let to = self.states;
        let bookmarked = self.states.project.bookmarks;
        let objects = self.states.project.objects;
        let press = self.press.clone();
        let picking = self.states.picking;
        let at = self.at;
        let symbol = self.symbols[self.index].clone();
        let pick = Pick::Symbol(symbol.clone());
        let name = symbol.data.display().to_owned();
        let object = symbol.object.name.clone();
        let document = Document::Symbol(symbol.clone());
        // One tooltip over both texts, so it is shown where either of them was cut.
        let whole = match press.about() {
            true => format!("{name} \u{2014} {object}"),
            false => name.clone(),
        };

        cut_tooltip(
            named.cut() || about.cut(),
            whole,
            list_row(hovering, picking.drawn(&pick, self.selected))
                .on_press({
                    let press = press.clone();
                    move |_| {
                        let symbol = symbol.clone();
                        picking.press(pick.clone(), at, || press.goes(to, symbol));
                    }
                })
                .maybe(press.bookmarks(), move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(
                            &e,
                            bookmark_menu(bookmarked, objects, document.clone()),
                        );
                    })
                })
                .child(tree_name_fitted(named, name, false, &self.marks))
                // Capped rather than measured, or a long member name would take the row
                // and leave the symbol it is about with nothing.
                .maybe_child(press.about().then(|| {
                    rect()
                        .max_width(Size::percent(45.0))
                        .overflow(Overflow::Clip)
                        .child(one_line_fitted(about, object).color(palette().address_fg))
                        .into_element()
                })),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// The one filtered symbol list both panels narrow: `symbols` read under the filter, in a
/// memo because the Symbols list is 115k names on `viewer-sample` and a
/// `VirtualScrollView` has to be told its length before it builds a row.
pub(crate) fn use_filtered_symbols(
    marking: Memo<Marking>,
    symbols: impl Fn() -> Shared<Symbol> + 'static,
) -> Memo<Filtered<Symbol>> {
    use_memo(move || {
        let symbols = symbols();
        let marking = marking.read();
        Filtered::new(symbols, marking.matcher(), |symbol| symbol.data.display())
    })
}

/// What the arrows and Enter do over a filtered symbol list: the rows the panel is
/// drawing, and the door a press on one goes through, which is the row's own
/// ([`SymbolPress::goes`]).
pub(crate) fn symbol_keys(
    filtered: Filtered<Symbol>,
    to: ListStates,
    press: SymbolPress,
) -> ListKeys {
    ListKeys::over(
        filtered,
        |symbol: &Symbol| Pick::Symbol(symbol.clone()),
        move |symbol: &Symbol| press.goes(to, symbol.clone()),
    )
}

/// The rows themselves, in `pane`'s `VirtualScrollView`: `selected` is the symbol the row
/// is lit for, and `marking` what it marks the name with.
///
/// Keyed by the symbol's data **and** its object, which is a [`Symbol`]'s own identity. A
/// `SymbolData` is allocated by the parse it came out of and so belongs to one object,
/// which is what makes the data pointer alone enough to key a list drawn from any number of
/// them; the pair is what a `Symbol` is, and needs no such argument.
pub(crate) fn symbol_rows(
    pane: &ListPane,
    filtered: Filtered<Symbol>,
    selected: Option<Symbol>,
    marking: Marking,
    press: SymbolPress,
) -> Element {
    pane.virtual_rows(
        filtered.len(),
        (filtered, selected, marking, press),
        |row,
         (filtered, selected, marking, press): &(
            Filtered<Symbol>,
            Option<Symbol>,
            Marking,
            SymbolPress,
        ),
         states| {
            // The row's place in the filtered list is not the symbol's place in the list
            // it was filtered out of, and everything below is about the symbol.
            let index = filtered.index(row);
            let symbol = &filtered.list()[index];
            SymbolRow {
                symbols: filtered.list().clone(),
                index,
                selected: selected.as_ref() == Some(symbol),
                at: row,
                marks: marking.marks(symbol.data.display()),
                press: press.clone(),
                states,
                key: DiffKey::None,
            }
            .key((
                Arc::as_ptr(&symbol.object).addr(),
                Arc::as_ptr(&symbol.data).addr(),
            ))
            .into()
        },
    )
}

/// One visited place in the History list. Clicking it is a click from outside the panes
/// like any other row's: the place opens in the temporal tab, or the tab already showing
/// it is raised, and the visit goes to the top of the list.
#[derive(Clone, PartialEq)]
struct HistoryRow {
    entry: Document,
    /// Whether this is what the tab on screen shows.
    current: bool,
    /// Where this row is in the list as it is drawn.
    at: usize,
    /// Where the filter matched in the name, for the row to mark.
    marks: Vec<Range<usize>>,
    /// What this row's press and its menu reach for, told to it by the list: see
    /// [`ListStates`].
    states: ListStates,
    key: DiffKey,
}

keyed!(HistoryRow);

impl Component for HistoryRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        // None of these is read -- only reading would subscribe the row, and it records
        // into them and no more.
        let ListStates {
            picking,
            doors,
            ctrl,
            project,
            ..
        } = self.states;
        let (bookmarked, objects) = (project.bookmarks, project.objects);
        let at = self.at;
        // One build of the name for every spelling: the row draws the short one and its
        // tooltip says the whole one.
        let Names { text, tooltip, .. } = Names::of(&self.entry);
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
                    picking.press(pick.clone(), at, || opened(doors, ctrl, target.clone()));
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
        self.keyed()
    }
}

#[derive(PartialEq)]
pub(crate) struct ObjectsPanel;

/// Load `paths` into the project open at `asked`, which is the one "Add binaries..." was
/// pressed in. Nothing if the reader has left that project while the dialog was up.
pub(crate) async fn added_binaries(states: ProjectStates, asked: Stay, paths: Vec<PathBuf>) {
    if states.left(asked) {
        return;
    }
    open_binaries(states.objects, states.loading, paths).await;
}

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
        let states = use_project_states();

        rect()
            .width(Size::fill())
            .horizontal()
            .padding(Gaps::new(4.0, 4.0, 0.0, 4.0))
            .child(
                Button::new()
                    .on_press(move |_| {
                        // The same dialog the menu's "Open a file as a project..." puts
                        // up, and on a task that outlives this panel: the reader can drag
                        // it elsewhere in the dock while the dialog is up (`ask_files`).
                        let asked = states.stay();
                        ask_files(
                            binaries_dialog("Add binaries to the project..."),
                            move |paths| added_binaries(states, asked, paths),
                        );
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
        // What Enter on a row reaches through: the pane's, which is where the rows' own
        // states are consumed too.
        let (doors, ctrl) = (pane.states.doors, pane.states.ctrl);
        // Which files the reader has folded open, by path: a view of a list and not part
        // of the session, so a `use_state` here. A file closed and opened again, or
        // reloaded after a build, comes back folded the way it was left.
        let expanded = use_state(HashSet::<PathBuf>::new);
        // The one compiled filter: what narrows the tree below, what the rows mark with,
        // and what the bar prints for a pattern that will not compile.
        let marking = use_list_marking(filter);
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many rows
        // there are before it builds any of them. Reading `loading` here is what puts a
        // file on screen the moment it is asked for and takes the indicator off it when
        // the last of its objects has landed.
        let tree = use_memo(move || {
            let marking = marking.read();
            ObjectTree::new(
                &objects.read(),
                &loading.read(),
                marking.matcher(),
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
                    document: Document::Object(object) | Document::Code(object),
                    ..
                },
            )) => Some(Arc::as_ptr(object).addr()),
            _ => None,
        };
        let length = tree.len();
        let marking = marking.read().clone();
        let keys = ListKeys::folding(
            tree.clone(),
            |row| match row {
                TreeRow::File { path, .. } | TreeRow::Pending { path, .. } => {
                    Pick::Path(path.clone())
                }
                TreeRow::Object { object, .. } => Pick::Object(object.clone()),
            },
            move |row| match row {
                TreeRow::File {
                    path, expansion, ..
                } => fold_archive(expanded, path, *expansion),
                // Nothing under it to fold and nothing behind it to open.
                TreeRow::Pending { .. } => Pressed::Folded,
                TreeRow::Object { object, .. } => {
                    opened(doors, ctrl, Document::Code(object.clone()))
                }
            },
            // Only a file row has anything under it: an object's row is a leaf, and so is
            // a file still being read, which has no members yet. A row already folded the
            // way the key asks is left alone, and so is one the filter is holding open --
            // `fold_archive`'s own rule, folding it away would hide the matches it points
            // at.
            move |row, unfold| {
                let TreeRow::File {
                    path, expansion, ..
                } = row
                else {
                    return;
                };
                if unfold == (*expansion == Expansion::Expanded) {
                    return;
                }
                fold_archive(expanded, path, *expansion);
            },
        );

        let pane = pane.filtered(
            filter,
            &marking,
            keys,
            pane.virtual_rows(
                length,
                (tree, selected, expanded, marking.clone()),
                |row,
                 (tree, selected, expanded, marking): &(
                    ObjectTree,
                    Option<usize>,
                    State<HashSet<PathBuf>>,
                    Marking,
                ),
                 states| {
                    match &tree[row] {
                        TreeRow::File {
                            name,
                            path,
                            members,
                            expansion,
                            loading,
                        } => ArchiveRow {
                            name: name.clone(),
                            path: path.clone(),
                            folds: Some(Folds {
                                members: *members,
                                expansion: *expansion,
                                expanded: *expanded,
                            }),
                            loading: *loading,
                            at: row,
                            marks: marking.marks(name),
                            states,
                            key: DiffKey::None,
                        }
                        .key(path)
                        .into(),
                        // The same row with nothing to fold, keyed by the same path.
                        TreeRow::Pending { name, path } => ArchiveRow {
                            name: name.clone(),
                            path: path.clone(),
                            folds: None,
                            loading: true,
                            at: row,
                            marks: marking.marks(name),
                            states,
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
                            states,
                            key: DiffKey::None,
                        }
                        .key(Arc::as_ptr(object).addr())
                        .into(),
                    }
                },
            ),
        );

        // The Search and Locations panels' frame with a button where their heading is.
        // The ground is named here, the button having none of its own.
        headed(AddBinaries.into_element(), pane).background(palette().pane_bg)
    }
}

#[derive(PartialEq)]
pub(crate) struct SymbolsPanel;

impl Component for SymbolsPanel {
    fn render(&self) -> impl IntoElement {
        let symbols = use_consume::<Symbols>().0;
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::Symbols);
        // What a press and Enter on a row both reach through: the pane's, so the rows and
        // the keys cannot be handed two sets.
        let to = pane.states;
        // The one compiled filter: what narrows the list below, what the rows mark with,
        // and what the bar prints for a pattern that will not compile.
        let marking = use_list_marking(filter);
        let filtered = use_filtered_symbols(marking, move || symbols.read().clone());
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Active>().0.read() {
            Some((
                _,
                Stop {
                    document: Document::Symbol(symbol),
                    ..
                },
            )) => Some(symbol.clone()),
            _ => None,
        };
        let marking = marking.read().clone();
        let keys = symbol_keys(filtered.clone(), to, SymbolPress::Open);

        // An empty list means the same two things here as in `short_list`, and the whole
        // list says which: no symbols at all draws the empty view, a filter that left
        // nothing of them says so.
        if filtered.len() == 0 && !filtered.list().is_empty() {
            return pane.filtered(filter, &marking, keys, placeholder("No matches"));
        }

        pane.filtered(
            filter,
            &marking,
            keys,
            symbol_rows(
                &pane,
                filtered,
                selected,
                marking.clone(),
                SymbolPress::Open,
            ),
        )
    }
}

#[derive(PartialEq)]
pub(crate) struct HistoryPanel;

impl Component for HistoryPanel {
    fn render(&self) -> impl IntoElement {
        let filter = use_state(Filter::default);
        let pane = use_list_pane(Panel::History);
        // What Enter on a row reaches through: the pane's, which is where the rows' own
        // states are consumed too. The record the rows are built from is the one the
        // doors carry.
        let (doors, ctrl) = (pane.states.doors, pane.states.ctrl);
        let visits = doors.visits;
        // The place the tab on screen shows is the row marked, the way the Symbols list
        // marks its symbol: the record itself has no cursor, the tabs having theirs.
        let current = use_consume::<Active>()
            .0
            .read()
            .clone()
            .map(|(_, stop)| stop.document);
        // A session's record is a couple of hundred places at most, so it is filtered
        // where the rows are built rather than through a memo. The one compiled filter all
        // the same, so the bar's error is what these rows were kept by.
        let marking = use_list_marking(filter);
        let marking = marking.read().clone();
        let matcher = marking.matcher();

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
                    let names = Names::of(entry);
                    // The whole name and not the shortened one the row draws: the generic
                    // arguments a tab has no room for are still worth searching for.
                    matcher
                        .matches(&names.whole)
                        .then(|| (entry.clone(), names.text))
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
                        states: pane.states,
                        key: DiffKey::None,
                    }
                    .key(entry_key(entry))
                    .into()
                })
                .collect();

            let listed = kept.into_iter().map(|(entry, _)| entry).collect();

            (rows, listed, visited)
        };
        let keys = ListKeys::over(
            listed,
            |entry: &Document| Pick::Visit(entry.clone()),
            move |entry: &Document| opened(doors, ctrl, entry.clone()),
        );

        pane.filtered(
            filter,
            &marking,
            keys,
            pane.short_list(rows, visited, "Nothing visited yet"),
        )
    }
}

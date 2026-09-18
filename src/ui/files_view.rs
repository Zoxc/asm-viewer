//! The Files view: the project's directory as a tree, read one level per unfold. A file's
//! row opens it as a source-driven tab, and its context menu offers it to `open_binaries`,
//! which is where whether it is an object is decided, and to the desktop's file manager;
//! a directory's row folds, and its menu is the file manager alone.

use super::*;

/// One row of the tree: a directory that folds, or a file that opens. The tree is the fold
/// state, so a directory row writes the tree itself and holds no expansion set.
#[derive(Clone, PartialEq)]
struct EntryRow {
    row: FileRow,
    tree: State<Option<FileTree>>,
    /// Where this row is in the tree as it is drawn, which is what the arrows step and
    /// what a press writes down with the pick (`ui/picks.rs`).
    at: usize,
    /// What this row's press and its menu reach for, told to it by the list: see
    /// [`ListStates`]. Compares equal always, so it costs the row no render.
    states: ListStates,
    key: DiffKey,
}

/// Fold the directory at `path`, or read it: the one writer of the tree, for the press
/// and for Left and Right alike.
fn toggle_directory(tree: State<Option<FileTree>>, path: &Path) {
    // A path that is not in the tree, or a file, changes nothing and wakes nothing.
    write_if(tree, |tree| {
        tree.as_mut().is_some_and(|tree| tree.toggle(path))
    });
}

/// What pressing a row does: a folder folds, and a file opens as source. Shared by the
/// press and by Enter on the row the arrows left the pick on.
///
/// Anything the pane could show opens; what the file *is* is not judged. A file past the
/// source cache's bound is left alone rather than opened into a tab that would only say
/// so, which is `open_source_file`'s own guard.
fn press_entry(
    states: ProjectStates,
    tree: State<Option<FileTree>>,
    ctrl: State<bool>,
    fold: Option<Fold>,
    path: &Path,
) -> Pressed {
    match fold {
        Some(_) => {
            toggle_directory(tree, path);
            Pressed::Folded
        }
        None => {
            open_source_file(states, path, Reach::outside(ctrl));
            Pressed::Opened
        }
    }
}

keyed!(EntryRow);

impl Component for EntryRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let tree = self.tree;
        let ListStates {
            picking,
            ctrl,
            project: states,
            ..
        } = self.states;
        let at = self.at;
        let fold = self.row.fold;
        let path = self.row.path.clone();
        let pressed = path.clone();
        // A copy per row *drawn* and never per row built, which is what the row's `Arc`
        // is for: `place_pick`'s rule (`src/ui/place_row.rs`) under both trees.
        let pick = Pick::Path(path.to_path_buf());

        // A failed directory keeps its triangle: pressing it tries the read again.
        let open = match fold {
            None => None,
            Some(Fold::Unfolded) => Some(true),
            Some(Fold::Folded | Fold::Failed) => Some(false),
        };
        let icon = match fold {
            None => ("file", lucide::file()),
            Some(Fold::Unfolded) => ("folder-open", lucide::folder_open()),
            Some(Fold::Folded | Fold::Failed) => ("folder", lucide::folder()),
        };
        let failed = fold == Some(Fold::Failed);

        extra_tooltip(
            self.row.path.display().to_string(),
            // A file the reader has open is not picked out here: this list is the
            // directory, not what is on screen. What lights a row is the reader having
            // pressed it.
            list_row(hovering, picking.drawn(&pick, false))
                .on_press(move |_| {
                    picking.press(pick.clone(), at, || {
                        press_entry(states, tree, ctrl, fold, &pressed)
                    });
                })
                // Every row's menu. A file's opens with the binary item, which is
                // `file_menu`'s choice between Open and Close: opening a binary is a
                // deliberate act, so it is not the press, and whether the file *is* one is
                // the parser's question, asked when the reader chooses to open it. A
                // directory has no such item, having no object in it to open. Under
                // whatever there is, for either kind of row, sits the item that shows the
                // path in the desktop's file manager. Needs the `ContextMenuViewer`
                // mounted at the root; opening one without it panics.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    let menu = match fold {
                        Some(_) => Menu::new(),
                        None => file_menu(states, path.to_path_buf()),
                    };
                    // Appended after the match, beside the reveal, so the Objects rows -- which
                    // share `close_menu` -- keep the one item they had. A project file is the one
                    // kind of file this view knows something more about than "it is a file".
                    let menu = menu
                        .maybe(project::is_project_file(&path), |menu| {
                            let path = path.clone();
                            menu.child(
                                MenuButton::new()
                                    .on_press(move |_| switch_project(states, path.to_path_buf()))
                                    .child("Open as project"),
                            )
                        })
                        .child(reveal_item(path.to_path_buf()));
                    ContextMenu::open_from_event(&e, menu);
                })
                .child(rect().width(Size::px(self.row.depth as f32 * TREE_INDENT)))
                .child(disclosure(open))
                .child(glyph(icon))
                .child(tree_name(self.row.name.to_string(), failed, &[])),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// The Files list: the project's directory, or a placeholder saying why there is none.
///
/// The tree is a `use_state` here and not a root context: which directories a reader has
/// unfolded is a view of a list, never part of the session, and a project switch resets it
/// by changing the directory it is over.
#[derive(PartialEq)]
pub(crate) struct FilesPanel;

impl Component for FilesPanel {
    fn render(&self) -> impl IntoElement {
        let workspace = use_consume::<Workspace>().0;
        let pane = use_list_pane(Panel::Files);
        // What Enter on a row reaches through: the pane's, which is where the rows' own
        // states are consumed too.
        let (states, ctrl) = (pane.states.project, pane.states.ctrl);
        // Read, not peeked: a keystroke in the Project view's directory box is a change
        // of what this is a tree of, and costs one `read_dir` of a half-typed path.
        let directory = workspace.read().clone();
        let first = directory.clone();
        let started = directory.clone();
        // Built here at the first render rather than by the effect below, which runs a
        // beat later and would draw the "not a directory" placeholder for one frame.
        let mut tree = use_state(move || first.as_deref().and_then(FileTree::new));
        // Read again whenever the project is over another directory. Which one the tree
        // is over is what the last run saw ([`use_on_change`]), seeded with the one the
        // render above built it for: the first run is no switch, and without the seed it
        // could not be told from one -- it comes a beat after that render, by which time
        // a session being restored has already named a directory. A plain captured value
        // and not a state: nothing draws it, and a state was a render per switch.
        use_on_change(
            move || workspace.read().clone(),
            move |before, directory: &Option<PathBuf>| {
                if before.unwrap_or(&started) == directory {
                    return;
                }
                tree.set(directory.as_deref().and_then(FileTree::new));
            },
        );
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many rows
        // there are before it builds any of them.
        let rows = use_memo(move || tree.read().as_ref().map(FileTree::rows));
        let rows = rows.read().clone();

        let mut keys = ListKeys::none();
        let body = match (directory, rows) {
            (None, _) => placeholder("No project directory. Set one in the Project view."),
            (Some(directory), None) => {
                placeholder(format!("Not a directory: {}", directory.display()))
            }
            (Some(_), Some(rows)) => {
                let length = rows.len();
                keys = ListKeys::folding(
                    rows.clone(),
                    |row: &FileRow| Pick::Path(row.path.to_path_buf()),
                    move |row: &FileRow| press_entry(states, tree, ctrl, row.fold, &row.path),
                    // A file row has nothing under it, and a directory already the way
                    // the key asks is left alone. A read that failed counts as folded, so
                    // Right tries it again -- which is what a press on it does.
                    move |row: &FileRow, unfold| {
                        let Some(fold) = row.fold else {
                            return;
                        };
                        if unfold == (fold == Fold::Unfolded) {
                            return;
                        }
                        toggle_directory(tree, &row.path);
                    },
                );
                pane.virtual_rows(
                    length,
                    (rows, tree),
                    |index, (rows, tree): &(FileRows, State<Option<FileTree>>), states| {
                        let row = &rows[index];
                        EntryRow {
                            row: row.clone(),
                            tree: *tree,
                            at: index,
                            states,
                            key: DiffKey::None,
                        }
                        .key(&row.path)
                        .into()
                    },
                )
            }
        };

        // The same box every other panel gets from its filter pane, minus the bar: this
        // one has nothing to filter by.
        pane.plain(keys, body)
    }
}

//! The Files view: the project's directory as a tree, read one level per unfold. A file's
//! row opens it as a source-driven tab, and its context menu offers it to `open_binaries`,
//! which is where whether it is an object is decided; a directory's row folds.

use super::*;

/// One row of the tree: a directory that folds, or a file that opens. The tree is the fold
/// state, so a directory row writes the tree itself and holds no expansion set.
#[derive(Clone)]
struct EntryRow {
    row: FileRow,
    tree: State<Option<FileTree>>,
    key: DiffKey,
}

impl PartialEq for EntryRow {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl KeyExt for EntryRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for EntryRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut tree = self.tree;
        // Consumed here, in the render, because the handlers that use them may not run a
        // hook.
        let states = use_project_states();
        let ctrl = use_consume::<Ctrl>().0;
        let fold = self.row.fold;
        let path = self.row.path.clone();
        let pressed = path.clone();

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        // A failed directory keeps its triangle: pressing it tries the read again.
        let chevron = match fold {
            None => "",
            Some(Fold::Unfolded) => "\u{25be}",
            Some(Fold::Folded | Fold::Failed) => "\u{25b8}",
        };
        let glyph = match fold {
            None => ("file", lucide::file()),
            Some(Fold::Unfolded) => ("folder-open", lucide::folder_open()),
            Some(Fold::Folded | Fold::Failed) => ("folder", lucide::folder()),
        };
        let failed = fold == Some(Fold::Failed);

        row_tooltip(
            self.row.path.display().to_string(),
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
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| match fold {
                    Some(_) => {
                        if let Some(tree) = tree.write().as_mut() {
                            tree.toggle(&pressed);
                        }
                    }
                    // Anything the pane could show opens; what the file *is* is not
                    // judged. A file past the source cache's bound is left alone rather
                    // than opened into a tab that would only say so.
                    None => {
                        if shows_as_source(&pressed) {
                            let file = Document::Source(Arc::from(&*pressed.to_string_lossy()));
                            open_document(states.open, states.visits, file, reach(ctrl));
                        }
                    }
                })
                // Every file's menu: opening a binary is a deliberate act, so it is not
                // the press, and whether the file *is* one is the parser's question, asked
                // when the reader chooses to open it. The item is Close when the path is
                // already loaded or loading, since opening a path twice puts a second copy
                // of each of its objects in the list. Needs the `ContextMenuViewer`
                // mounted at the root; opening one without it panics.
                .maybe(fold.is_none(), move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        let loaded = states.objects.peek().iter().any(|o| o.path == path)
                            || states.loading.peek().is_loading(&path);
                        let menu = if loaded {
                            close_menu(states, path.clone())
                        } else {
                            open_menu(states.objects, states.loading, path.clone())
                        };
                        ContextMenu::open_from_event(&e, menu);
                    })
                })
                .child(rect().width(Size::px(self.row.depth as f32 * TREE_INDENT)))
                .child(
                    label()
                        .text(chevron)
                        .width(Size::px(CHEVRON_WIDTH))
                        .color(palette().address_fg)
                        .max_lines(1),
                )
                .child(document_glyph(glyph))
                .child(tree_name(self.row.name.clone(), failed)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The Files list: the project's directory, or a placeholder saying why there is none.
///
/// The tree is a `use_state` here and not a root context: which directories a reader has
/// unfolded is a view of a list, never part of the session, and a project switch resets it
/// by changing the directory it is over.
#[derive(PartialEq)]
pub(crate) struct FilesTab;

impl Component for FilesTab {
    fn render(&self) -> impl IntoElement {
        let proj = use_consume::<Proj>().0;
        // Read, not peeked: a keystroke in the Project view's directory box is a change
        // of what this is a tree of, and costs one `read_dir` of a half-typed path.
        let directory = given(&proj.read().directory).map(str::to_owned);
        let first = directory.clone();
        // Built at the first render rather than by the effect below, which runs a beat
        // later and would draw the "not a directory" placeholder for one frame.
        let mut tree = use_state(move || {
            first
                .as_deref()
                .and_then(|directory| FileTree::new(Path::new(directory)))
        });
        use_side_effect_with_deps(&directory, move |directory: &Option<String>| {
            let next = directory
                .as_deref()
                .and_then(|directory| FileTree::new(Path::new(directory)));
            tree.set(next);
        });
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many rows
        // there are before it builds any of them.
        let rows = use_memo(move || tree.read().as_ref().map(FileTree::rows));
        let rows = rows.read().clone();

        let body = match (directory, rows) {
            (None, _) => placeholder("No project directory. Set one in the Project view."),
            (Some(directory), None) => placeholder(format!("Not a directory: {directory}")),
            (Some(_), Some(rows)) => {
                let length = rows.len();
                // `new_with_data`, never a capture: the builder closure is not compared
                // across renders.
                VirtualScrollView::new_with_data(
                    (rows, tree),
                    |index, (rows, tree): &(FileRows, State<Option<FileTree>>)| {
                        let row = rows.row(index);
                        EntryRow {
                            row: row.clone(),
                            tree: *tree,
                            key: DiffKey::None,
                        }
                        .key(&row.path)
                        .into()
                    },
                )
                .length(length)
                .item_size(list_row_height())
                .into_element()
            }
        };

        rect().expanded().background(palette().pane_bg).child(body)
    }
}

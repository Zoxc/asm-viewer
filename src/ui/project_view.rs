//! Which project is open: the pane that says so, when the two files are written, and
//! what a switch empties and a restore fills back in.

use super::*;

fn binary_row(path: &Path, objects: usize) -> Element {
    let text = path.to_string_lossy().into_owned();
    row_tooltip(
        text.clone(),
        rect()
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(8.0)
            .content(Content::Flex)
            .child(tree_name(text, false))
            .child(
                label()
                    .text(match objects {
                        1 => "1 object".to_owned(),
                        many => format!("{many} objects"),
                    })
                    .color(palette().address_fg)
                    .max_lines(1),
            ),
    )
    .into_element()
}

/// One project in the recent list. Pressing it opens this one in place of the one on
/// screen.
#[derive(Clone, PartialEq)]
struct RecentRow {
    recent: Recent,
    key: DiffKey,
}

impl KeyExt for RecentRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for RecentRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let id = self.recent.id.clone();
        let recent = &self.recent;

        // The id where there is no name, drawn as a tag rather than as a name.
        let (text, color) = match &recent.name {
            Some(name) => (name.clone(), palette().text_fg),
            None => (recent.id.as_str().to_owned(), palette().address_fg),
        };
        // What is known about it without opening it, out of its own file.
        let about = match &recent.directory {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => match recent.binaries {
                0 => "empty".to_owned(),
                1 => "1 binary".to_owned(),
                many => format!("{many} binaries"),
            },
        };

        row_tooltip(
            recent.id.as_str().to_owned(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .padding(Gaps::new_symmetric(0.0, 4.0))
                .spacing(8.0)
                .content(Content::Flex)
                .background(match hovering() {
                    true => palette().object_hover_bg,
                    false => Color::TRANSPARENT,
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| switch_project(states, id.clone()))
                .child(
                    label()
                        .text(text)
                        .width(Size::flex(1.0))
                        .color(color)
                        .max_lines(1),
                )
                .child(label().text(about).color(palette().address_fg).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The Project pane: the project the app is in, the two things about it the reader can
/// say, and the other projects they can go to. The recent list leaves out the open
/// project, which the pane above it already describes.
#[derive(PartialEq)]
pub(crate) struct ProjectTab;

impl Component for ProjectTab {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut proj = states.proj;
        let objects = states.objects;

        // Read on mount and again when the open project changes, never per render:
        // each row is a small read of another project's own file.
        let mut recents = use_state(project::recent_projects);
        let open = proj.read().clone();
        use_side_effect_with_deps(&open.id, move |_: &Option<ProjectId>| {
            recents.set(project::recent_projects());
        });

        let binaries: Vec<Element> = {
            let objects = objects.read();
            project::binaries(&objects)
                .into_iter()
                .map(|path| {
                    let count = objects.iter().filter(|object| object.path == path).count();
                    binary_row(&path, count)
                })
                .collect()
        };

        let others: Vec<Element> = recents
            .read()
            .iter()
            .filter(|recent| Some(&recent.id) != open.id.as_ref())
            .map(|recent| {
                RecentRow {
                    recent: recent.clone(),
                    key: DiffKey::None,
                }
                .key(recent.id.as_str().to_owned())
                .into()
            })
            .collect();

        let on_choose = move |_| {
            spawn(async move {
                let Some(handle) = AsyncFileDialog::new()
                    .set_title("Choose the project's directory...")
                    .pick_folder()
                    .await
                else {
                    return;
                };
                proj.write().directory = handle.path().to_string_lossy().into_owned();
            });
        };

        rect()
            .expanded()
            .background(palette().pane_bg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Project", None))
                        // Each box writes straight into `Proj`, so a keystroke is a
                        // state change the save observer sees and `project.toml` is
                        // written at once.
                        .child(field_row(
                            "Name",
                            Input::new(
                                proj.into_writable()
                                    .map(|open| &open.name, |open| &mut open.name),
                            )
                            .placeholder("Unnamed")
                            .compact()
                            .width(Size::flex(1.0)),
                        ))
                        .child(field_row(
                            "Directory",
                            rect()
                                .width(Size::flex(1.0))
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .content(Content::Flex)
                                .spacing(6.0)
                                .child(
                                    Input::new(
                                        proj.into_writable().map(
                                            |open| &open.directory,
                                            |open| &mut open.directory,
                                        ),
                                    )
                                    .placeholder("None")
                                    .compact()
                                    .width(Size::flex(1.0)),
                                )
                                .child(Button::new().on_press(on_choose).child("Choose...")),
                        ))
                        // The directory the project is stored in, which is its identity
                        // and is written inside neither of the files in it.
                        .child(field_row(
                            "Stored as",
                            label()
                                .text(match &open.id {
                                    Some(id) => id.as_str().to_owned(),
                                    // A project directory is made by the first write
                                    // that has something to put in it.
                                    None => "not saved yet".to_owned(),
                                })
                                .color(palette().address_fg)
                                .max_lines(1),
                        ))
                        .child(section_heading("Binaries", None))
                        .child(match binaries.is_empty() {
                            true => info_line("Nothing open".to_owned()).into_element(),
                            false => rect().width(Size::fill()).children(binaries).into_element(),
                        })
                        .child(section_heading(
                            "Recent projects",
                            Some(
                                Button::new()
                                    .on_press(move |_| new_project(states))
                                    .child("New project")
                                    .into_element(),
                            ),
                        ))
                        .child(match others.is_empty() {
                            true => info_line("No other projects".to_owned()).into_element(),
                            false => rect().width(Size::fill()).children(others).into_element(),
                        }),
                ),
            )
            .into_element()
    }
}

/// Tell the save policy what the session looks like, whenever it changes.
///
/// `use_side_effect` re-runs whenever a `State` `read()` inside it changes, so the
/// `read()` calls below *are* the subscriptions: this one observer is the choke point
/// every mutation flows through. Whether a change reaches the disk now or at the next
/// `use_periodic_save` tick is `project::record`'s decision, not this one's.
pub(crate) fn use_save_on_change(states: ProjectStates) {
    let ProjectStates {
        proj,
        objects,
        // What is still being read is deliberately not saved and deliberately does not
        // What is still being read is deliberately not saved: `binaries` is derived
        // from the objects.
        loading: _,
        open,
        asm_at,
        src_at,
        code_at,
        driven,
        history,
    } = states;

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it. Each
        let objects = objects.read();
        project::record(proj.read().details(), project::binaries(&objects), {
            // The dock and the table rather than `Active`, which is a memo and so a
            // beat behind.
            let (dock, docs) = (open.dock.read(), open.docs.read());
            Session::from_state(
                &objects,
                &open_documents(&dock, &docs),
                &asm_at.read(),
                &src_at.read(),
                &code_at.read(),
                &driven.read(),
                active_document(&dock, &docs).as_ref(),
                &history.read(),
            )
        });
    });
}

/// Write out a pending change every `AUTOSAVE_INTERVAL`. A tick that finds nothing
/// pending does no IO at all.
pub(crate) fn use_periodic_save() {
    use_hook(|| {
        spawn(async move {
            loop {
                Timer::after(project::AUTOSAVE_INTERVAL).await;
                project::flush();
            }
        });
    });
}

/// Reopen the last project -- its name, binaries, tabs and selection -- once, at startup.
/// Which project that is, is `project::reopen`'s answer.
pub(crate) fn use_restore_on_startup(states: ProjectStates) {
    use_hook(move || {
        let Some((id, project, session)) = project::reopen() else {
            return;
        };

        // Synchronously, and before anything else here: `project::reopen` has just
        // seeded the save policy's baseline from this same project, and the two have to
        // agree by the time the first effect runs or the save observer would see the
        // name as a change and write it straight back out.
        let mut proj = states.proj;
        proj.set(OpenProject::opened(id, &project));

        restore_project(states, project, session);
    });
}

/// Put a project's binaries, tabs, active document and history on screen. Shared by the
/// two things that do a restore -- the app starting and a switch -- so the second cannot
/// drift from the first. Every step degrades silently.
///
/// Two orderings are load-bearing. **Tabs before the active document**: `activate` opens
/// what it cannot find, so restoring the active one first would leave its tab at the end
/// of the strip. **The rows go into the two `Positions` maps before the tabs are
/// opened**: a pane puts its view back when it notices the tab it is showing has
/// changed, so a row arriving after the `activate` arrives after the only moment
/// anything looks at it.
fn restore_project(states: ProjectStates, project: Project, session: Session) {
    let ProjectStates {
        objects,
        loading,
        open,
        mut asm_at,
        mut src_at,
        mut code_at,
        mut driven,
        history,
        ..
    } = states;

    if project.binaries.is_empty() {
        return;
    }

    spawn(async move {
        // The objects arrive as they are parsed, but the *session* waits for the whole
        // load: a tab is resolved against the objects by name, and resolving one against
        // a half-filled list would drop the tabs whose object had not landed yet.
        open_binaries(objects, loading, project.binaries.clone()).await;

        let (objects, mut history) = (objects, history);
        // Nothing opened: leave the app empty *and* leave the file alone.
        if objects.peek().is_empty() {
            return;
        }

        // Resolved against everything now loaded rather than just what this load
        // All three computed before any is set, so no read guard is live when anything
        // is notified.
        let (restored_history, restored_tabs, restored_active) = {
            let loaded = objects.read();
            (
                session.resolve_history(&loaded),
                session.resolve_tabs(&loaded),
                session.resolve(&loaded),
            )
        };

        // The history first, so the `Visit::Went` below has a cursor to dedup against.
        history.set(restored_history);
        // Where each side of each tab was left, and what drove it, go in before the tab
        // is opened. The line for the same reason the rows are: a pane looks at what it
        // has been told exactly once, when it notices the tab it is showing has changed.
        {
            let (mut asm, mut src, mut from) = (asm_at.write(), src_at.write(), driven.write());
            let mut places = code_at.write();
            for tab in &restored_tabs {
                asm.remember(tab.document.clone(), tab.asm_row);
                src.remember(tab.document.clone(), tab.src_row);
                if let Some(line) = tab.line {
                    from.remember(tab.document.clone(), line);
                }
                if let Some(address) = tab.address {
                    places.remember(tab.document.clone(), Spot { address, rows: 0 });
                }
            }
        }
        for tab in restored_tabs {
            // Reopening a tab is not visiting it.
            activate(open, history, Some(tab.document), Visit::Moved);
        }
        // The document the app lands on is a place it went.
        activate(open, history, restored_active, Visit::Went);
    });
}

/// Empty the app of everything that belonged to the project being left, through the
/// functions that hold the invariants and never by writing the lists.
///
/// A closing binary deliberately leaves source-driven tabs standing, so they are closed
/// here; the history is emptied outright, which is the one thing no walk reaches.
pub(crate) fn clear_project(states: ProjectStates) {
    let ProjectStates {
        objects,
        mut loading,
        open,
        asm_at,
        src_at,
        code_at,
        driven,
        history,
        ..
    } = states;

    // Every load at once, and before the closes: a file that has produced nothing yet is
    // not in the objects list for the walk below to reach.
    loading.write().clear();

    // Both reads are bound before anything writes -- the read-guard rule, and also that
    // `close_binary` writes the very list being walked.
    let binaries = project::binaries(&objects.peek());
    for path in binaries {
        close_binary(
            objects, loading, open, asm_at, src_at, code_at, driven, history, &path,
        );
    }

    let remaining = open.documents();
    for tab in &remaining {
        close_tab(open, history, asm_at, src_at, code_at, driven, tab);
    }

    // And the history outright, which neither walk above does.
    let mut history = history;
    history.set(History::default());
}

/// Leave the project on screen and open the one `id` names in its place.
///
/// The order is what makes a switch safe: `project::switch` flushes the old project and
/// re-points every baseline while the policy still points at it, and only then is the app
/// emptied -- so the save observer, woken by a notify after this handler, sees one
/// settled state that matches the baseline and writes nothing.
fn switch_project(states: ProjectStates, id: ProjectId) {
    let Some((project, session)) = project::switch(&id) else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &project));
    restore_project(states, project, session);
}

/// Start a project nobody has named yet and go to it.
fn new_project(states: ProjectStates) {
    let Some(id) = project::start_new() else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &Project::default()));
}

//! The session as the UI keeps it in step with `project.rs`: what is saved when, what a
//! restore fills in, and what a switch empties. And the settings the same way: the wiring
//! between `settings.toml`, the appearance and the fonts. Nothing here draws; `app()`
//! calls all of it.

use super::*;

/// The whole of the wiring between the settings and what they are settings of: the
/// appearance, the fonts, and `settings.toml`. The write is handed in because
/// [`Settings::save`] writes the machine's real settings file, so a test that mounted
/// this would be editing the settings of whoever ran it.
pub(crate) fn use_settings_with(
    prefs: State<EditedSettings>,
    mut save: impl FnMut(&Settings) + 'static,
) {
    // What the file currently says -- not what was loaded. It has to *move*, or a reader
    // who changes a setting and changes it back would leave the file holding the middle
    // answer. An `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let written = use_hook(|| Rc::new(RefCell::new(prefs.peek().settings())));
    let settings = prefs.read().settings();

    apply_theme(settings.theme);

    use_side_effect_with_deps(&settings, move |settings: &Settings| {
        set_fonts(fonts::resolve(settings));

        let mut written = written.borrow_mut();
        if *settings != *written {
            *written = settings.clone();
            save(settings);
        }
    });
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
        // Where the files go is `project::record`'s own, out of what the policy was
        // pointed at when the project was opened.
        store: _,
        objects,
        // What is still being read is not itself saved -- `binaries` is derived from
        // the objects -- but a list still filling in is not the app's list, so the
        // record below is told. Reading it also re-runs this when the load ends, which
        // is the record that writes.
        loading,
        open,
        places,
        visits,
        bookmarks,
        // A search is a view of the project's files, not part of the session.
        searched: _,
        build,
        arranged,
    } = states;

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it.
        let objects = objects.read();
        let loading = !loading.read().is_empty();
        // One read, for the two halves it feeds: what the user said goes in the project
        // file and the agreement goes in the session.
        let about = proj.read().clone();
        project::record(
            &about.details(),
            &project::binaries(&objects),
            loading,
            bookmarks.read().entries(),
            {
                // The dock and the table rather than `Active`, which is a memo and so a
                // beat behind.
                let (strip, docs) = (open.strip.read(), open.docs.read());
                // The bar in its own order, pages and documents alike. A document whose
                // trail has gone is not a tab the session can name, and is left out.
                let tabs: Vec<SavingTab<'_>> = strip
                    .tabs()
                    .iter()
                    .filter_map(|tab| match tab {
                        Tab::Page(page) => Some(SavingTab::Page(*page)),
                        Tab::Document(id) => docs.trail(*id).map(|trail| SavingTab::Document {
                            id: *id,
                            trail,
                            temporal: docs.temporal() == Some(*id),
                        }),
                    })
                    .collect();
                // What was on screen: a page, a document, or neither. Bound before the
                // borrow below it, the document being read out of the two states.
                let shown_document = active_tab(&strip, &docs).map(|(_, at)| at.document);
                let shown = match (strip.active(), &shown_document) {
                    (Some(Tab::Page(page)), _) => OnScreen::Page(page),
                    (_, Some(document)) => OnScreen::Document(document),
                    _ => OnScreen::Nothing,
                };
                Session::from_state(
                    &objects,
                    &tabs,
                    // By name, the three maps being of near-identical type. What each
                    // place had picked out (`places.marks_at`) is a view of its tab, and
                    // not saved.
                    &LeftAt {
                        asm_rows: &places.asm_at.read(),
                        src_rows: &places.src_at.read(),
                        places: &places.code_at.read(),
                        driven: &places.driven.read(),
                    },
                    shown,
                    &visits.read(),
                    Noticed {
                        trusted: about.trusted,
                        artifacts: &build.read().previous,
                        // Reading these three is what subscribes the observer to a panel
                        // being dragged and to either handle being moved.
                        ui: SavedUi {
                            sidebar: Some(*arranged.sidebar.read()),
                            split: Some(*arranged.split.read()),
                            dock: Some(arranged.dock.read().saved()),
                        },
                    },
                )
            },
        );
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

/// The store this run keeps its files in, or the failure that says there is none: what a
/// way into a project asks before it opens anything.
///
/// Nowhere to keep anything is not a fact about the file, so the load never answers it
/// ([`project::Reason::NoStore`]); the caller does, and this is the one place it is built.
fn store_for(states: ProjectStates, path: &Path) -> Result<Store, project::Failure> {
    states.store.peek().clone().ok_or_else(|| project::Failure {
        path: path.to_path_buf(),
        reason: project::Reason::NoStore,
    })
}

/// Reopen the last project -- its name, binaries, tabs and selection -- once, at startup.
/// Which project that is, is `project::reopen`'s answer.
pub(crate) fn use_restore_on_startup(states: ProjectStates, opening: Option<PathBuf>) {
    // Outside the hook, a hook running inside another being what it is.
    let mut unopened = use_consume::<Unopened>().0;
    use_hook(move || {
        // What the app was given beats what it was last in. A file that will not parse
        // opens nothing and is said so, the same as one picked from a menu would be: it is
        // the reader's own file and is left exactly as it is.
        let opened = match &opening {
            Some(path) => Some(
                // The path the app was given, put beside the two halves so every branch
                // here answers the same shape; `open_at` does not touch it.
                store_for(states, path)
                    .and_then(|store| project::open_at(&store, path))
                    .map(|(project, session)| (path.clone(), project, session)),
            ),
            // With nowhere to keep anything there is nothing to reopen and nothing to say:
            // a startup that would have reopened a project has the empty screen to show
            // for it either way. A reader who asked for one is told, in the arm above.
            None => {
                let store = states.store.peek().clone();
                store.as_ref().and_then(project::reopen)
            }
        };
        // Nothing to reopen is the empty screen and not something to say; `project::reopen`
        // counts a last project whose file has gone as one of those.
        let (file, project, session) = match opened {
            None => return,
            Some(Ok(opened)) => opened,
            Some(Err(failure)) => {
                unopened.set(Some(failure));
                return;
            }
        };

        enter_project(states, file, project, session);
    });
}

/// Put a project on screen: which project is open, its bookmarks, and everything
/// [`restore_project`] restores. The one way in, so the three ways a project is entered
/// -- the app starting, a switch, and a new project -- cannot drift apart.
///
/// Both writes are **synchronous, and before anything else**: the save policy's baselines
/// have just been seeded from this same project, and the two have to agree by the time the
/// first effect runs or the save observer would see the name, or the bookmarks, as a
/// change and write them straight back out.
fn enter_project(states: ProjectStates, file: PathBuf, project: Project, session: Session) {
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::opened(file, &project, session.trusted));
    bookmarks.set(Bookmarks::from_entries(project.bookmarks.clone()));

    restore_project(states, project, session);
}

/// Put a project's binaries, tabs, active document and visits on screen. Every way into a
/// project comes here through [`enter_project`], so none can drift from another. Every
/// step degrades silently, and a project with nothing saved restores nothing.
///
/// The **pages go back first and synchronously**: one resolves against no object, so a
/// session whose only tab was Settings has nothing to wait for.
///
/// The documents follow, through [`restore_documents`]: after the load where there are
/// binaries, and **at once where there are none**. What waits for a load is resolving a
/// tab against the objects by name, and only object and symbol tabs do that -- so a
/// project with no binaries still comes back with the source files the reader had open.
///
/// Two orderings are load-bearing among the documents. **Tabs before the active
/// document**: `open_document` opens what it cannot find, so restoring the active one
/// first would leave its tab out of place in the bar. **The rows go into the `Positions`
/// maps before each tab is shown**: a pane puts its view back when it notices the tab it
/// is showing has changed, so a row arriving after the tab is on screen arrives after the
/// only moment anything looks at it. A tab's trail is opened whole and its rows go in per
/// entry, so Back after a restart comes back to the rows that were left.
pub(crate) fn restore_project(states: ProjectStates, project: Project, session: Session) {
    // How the window was arranged. Before everything else: it is about the window and not
    // about what is open in it, so a project with no binaries left still comes back
    // arranged the way it was left.
    restore_ui(states.arranged, session.ui.as_ref());

    // What the last build produced, which the next build replaces. A project whose
    // binaries are all gone still knows what it built.
    let mut build = states.build;
    let mut next = build.peek().clone();
    next.previous = session
        .cargo
        .as_ref()
        .map(|cargo| cargo.artifacts.clone())
        .unwrap_or_default();
    build.set(next);

    let ProjectStates {
        objects,
        loading,
        open,
        ..
    } = states;

    // The pages, at the places they had in the bar, and the one that was on screen.
    // Before the documents, whose own count of places steps over theirs.
    {
        let mut strip = open.strip;
        let mut strip = strip.write();
        for (position, page) in session.pages() {
            strip.insert(Tab::Page(page), position);
        }
        if let Some(page) = session.shown_page() {
            strip.raise(Tab::Page(page));
        }
    }

    // Nothing to load, so nothing to wait for.
    if project.binaries.is_empty() {
        restore_documents(states, &session);
        return;
    }

    // `spawn_forever`, not `spawn`: a task belongs to the scope that spawned it, and on a
    // switch that scope is the recent project's row, which the press unmounts -- the row
    // is left out of the list the moment its project is the open one, so the restore
    // would be dropped before its first poll.
    spawn_forever(async move {
        // The objects arrive as they are parsed, but the *session* waits for the whole
        // load: an object or a symbol tab is resolved against the objects by name, and
        // resolving one against a half-filled list would drop the tabs whose object had
        // not landed yet.
        open_binaries(objects, loading, project.binaries.clone()).await;
        restore_documents(states, &session);
    });
}

/// The visits, the tabs and the active document, against every object now loaded.
///
/// **Called whatever that list holds.** A load that produced nothing and a project with
/// no binaries at all are the same case: `Session::restore` drops the tabs naming an
/// object that is not there, and a source place resolves against nothing and so cannot
/// fail, so what comes back is what the reader can still be shown.
///
/// Peeked and not read: with no binaries this runs during a render, where a read would
/// subscribe the rendering scope to every object that lands later. The peek is released
/// before anything is set either way, so no guard is live when a write notifies.
fn restore_documents(states: ProjectStates, session: &Session) {
    let ProjectStates {
        objects,
        open,
        places,
        mut visits,
        ..
    } = states;

    // The visits, the tabs and the active document in one call: they are one question,
    // and resolving them apart would let a tab and the active document be read against
    // two different answers about which binaries have changed.
    let restored = {
        let loaded = objects.peek();
        session.restore(&loaded)
    };

    // The record first, so the opening below finds the active place already at its top
    // and records nothing over it.
    visits.set(restored.visits);
    // Where in the bar the next tab goes. Counted over what survived rather than read off
    // the saved list, so the tabs that resolved keep their order around the pages already
    // put back.
    let mut position = 0;
    for tab in restored.tabs {
        let RestoredTab::Document {
            temporal,
            trail,
            entries,
        } = tab
        else {
            // A page is in the bar already, put there first; what it owes the count is
            // its place.
            position += 1;
            continue;
        };
        // The trail whole, with the maps filled before the chip goes in the bar.
        // Reopening a tab is not visiting it. Put at the place it had rather than beside
        // the tab on screen: the saved order is stated outright.
        let opened = open.insert_tab(trail, temporal, position, |id| {
            place_entries(places, id, entries)
        });
        if opened.is_none() {
            continue;
        }
        position += 1;
    }
    // The document the app lands on is a place it went: the tab showing it is raised, or
    // -- degraded to its object, say -- it opens in a tab of its own.
    if let Some(active) = restored.active {
        open_document(open, visits, active, Reach::NewTab);
    }
}

/// Where each side of every place on one restored tab was left, and what drove it, into
/// the maps a pane reads them back out of.
///
/// **Those maps are the one thing a restore writes directly**, everything else it does
/// going through `Open` and `open_document`, so the writes have a name rather than
/// sitting three levels deep in the loop above. This is what `Open::insert_tab` is handed:
/// it runs before the tab is put in the bar, since a pane puts its view back when it
/// notices the place it is showing has changed, so a row arriving after the tab is on
/// screen arrives after the only moment anything looks at it.
fn place_entries(places: Places, id: DocId, entries: Vec<RestoredEntry>) {
    let Places {
        mut asm_at,
        mut src_at,
        mut code_at,
        mut driven,
        ..
    } = places;
    let (mut asm, mut src, mut from) = (asm_at.write(), src_at.write(), driven.write());
    let mut code = code_at.write();
    for entry in entries {
        // The place itself, address and line and all: two stops in one object's code, or
        // in one file, are two keys, as they were when they were saved.
        let key = (id, entry.stop());
        asm.remember(key.clone(), entry.asm_row);
        src.remember(key.clone(), entry.src_row);
        if let Some(line) = entry.line {
            from.remember(key.clone(), line);
        }
        if let Some(address) = entry.address {
            code.remember(key, Spot { address, rows: 0 });
        }
    }
}

/// Empty the app of everything that belonged to the project being left, through the
/// functions that hold the invariants and never by writing the lists.
///
/// A closing binary deliberately leaves source-driven tabs standing, so they are closed
/// here; the record of visits is emptied outright, which is the one thing no walk
/// reaches. The bookmarks are not touched: they are the file's content, and the project
/// coming in sets them the way it sets the name.
pub(crate) fn clear_project(states: ProjectStates) {
    let ProjectStates {
        objects,
        mut loading,
        open,
        places,
        visits,
        mut searched,
        ..
    } = states;

    // Every load at once, and before the closes: a file that has produced nothing yet is
    // not in the objects list for the walk below to reach.
    loading.write().clear();

    // Both reads are bound before anything writes -- the read-guard rule, and also that
    // `close_binary` writes the very list being walked.
    let binaries = project::binaries(&objects.peek());
    for path in binaries {
        close_binary(states, &path);
    }

    let remaining = open.ids();
    for id in remaining {
        close_tab(open, places, id);
    }

    // The pages go with the documents: which of them is open is this project's session,
    // and the project being opened puts its own back.
    {
        let mut strip = open.strip;
        strip.write().close(|tab| matches!(tab, Tab::Page(_)));
    }

    // And the record outright, which neither walk above does.
    let mut visits = visits;
    visits.set(Visits::default());

    // The search likewise: its hits are places in the directory being left, and dropping
    // the question is also what stops a walk still running -- the task takes the next
    // batch, sees a search it is not, and lets its end of the channel go.
    //
    // The id is bumped and not set back to nothing. A walk from this project is parked in
    // its receiver and learns nothing until its next batch, so a counter that restarted
    // at zero would hand the next project the very numbers that walk still answers to.
    let id = searched.peek().id.wrapping_add(1);
    searched.set(Searched {
        id,
        ..Searched::default()
    });

    // What one project built says nothing about the next, and a list left standing would
    // have the first build over there replace binaries opened over here.
    let mut build = states.build;
    build.set(Builds::default());
}

/// Add what the load that just ran moved aside to what the window is already naming.
///
/// The one place any load says what it moved: the startup's in `app()`, a project
/// switch's below, and whatever comes later. Added to rather than set, so a window still
/// naming an earlier load's files does not lose them; and nothing at all when the load
/// moved nothing, so a quiet load leaves a closed window closed.
pub(crate) fn note_moved(mut rescued: State<Vec<PathBuf>>) {
    let moved = store::moved();
    if moved.is_empty() {
        return;
    }
    let mut naming = rescued.peek().clone();
    naming.extend(moved);
    rescued.set(naming);
}

/// Leave the project on screen and open the one the file at `path` holds in its place.
///
/// The order is what makes a switch safe: `project::switch` flushes the old project and
/// re-points every baseline while the policy still points at it, and only then is the app
/// emptied -- so the save observer, woken by a notify after this handler, sees one
/// settled state that matches the baseline and writes nothing.
pub(crate) fn switch_project(
    states: ProjectStates,
    rescued: State<Vec<PathBuf>>,
    mut unopened: State<Option<project::Failure>>,
    path: PathBuf,
) {
    let switched = store_for(states, &path).and_then(|store| project::switch(&store, &path));
    let (project, session) = match switched {
        Ok(both) => both,
        Err(failure) => {
            // A project file is never moved aside and nothing is written over it, so
            // telling the reader why is the whole of what is left to do.
            unopened.set(Some(failure));
            return;
        }
    };

    note_moved(rescued);

    clear_project(states);
    enter_project(states, path, project, session);
}

/// Which file a dialog asks the reader for: one that is there, a directory, or a name to
/// write under. The last is the app's one **save** dialog: the only place the reader names
/// a file that is not there yet.
#[derive(Clone, Copy)]
pub(crate) enum AskFor {
    File,
    Folder,
    Save,
}

/// Put `dialog` up and hand the path it answered with to `then`. Nothing at all where it
/// was dismissed.
///
/// **On a task that outlives the scope this was called from.** The dialog is asynchronous
/// and, through the xdg portal, not modal to the window, so the reader can raise another
/// tab or drag a panel out from under the button while it is up -- and that unmounts the
/// scope a `spawn` would belong to, losing the file they then chose. Everything `then`
/// writes is a root state, so the write is good whatever is on screen.
///
/// The one place that reason is written, so the next dialog cannot be the one that gets
/// it wrong.
pub(crate) fn ask_file(
    dialog: AsyncFileDialog,
    asking: AskFor,
    then: impl FnOnce(PathBuf) + 'static,
) {
    spawn_forever(async move {
        let picked = match asking {
            AskFor::File => dialog.pick_file().await,
            AskFor::Folder => dialog.pick_folder().await,
            AskFor::Save => dialog.save_file().await,
        };
        let Some(handle) = picked else {
            return;
        };
        then(handle.path().to_path_buf());
    });
}

/// The same for the several files one dialog can answer with. `then` is awaited, both
/// callers having a load to run; the task and the reason are [`ask_file`]'s.
pub(crate) fn ask_files<F: std::future::Future<Output = ()> + 'static>(
    dialog: AsyncFileDialog,
    then: impl FnOnce(Vec<PathBuf>) -> F + 'static,
) {
    spawn_forever(async move {
        let Some(handles) = dialog.pick_files().await else {
            return;
        };
        then(handles.iter().map(|h| h.path().to_path_buf()).collect()).await;
    });
}

/// The dialog that asks for binaries, under `title`.
///
/// The Objects panel's "Add binaries..." and the menu's "Open a file as a project..." are
/// the same gesture from two places, so the title is all they differ in. An object-file
/// filter, which there is none of today, would be one edit here.
pub(crate) fn binaries_dialog(title: &str) -> AsyncFileDialog {
    AsyncFileDialog::new().set_title(title)
}

/// Ask for a project file and open it in place of the one on screen.
pub(crate) fn ask_for_a_project(
    states: ProjectStates,
    rescued: State<Vec<PathBuf>>,
    unopened: State<Option<project::Failure>>,
) {
    ask_file(
        AsyncFileDialog::new()
            .set_title("Open a project...")
            .add_filter("Project", &[project::PROJECT_EXTENSION]),
        AskFor::File,
        move |path| switch_project(states, rescued, unopened, path),
    );
}

/// Ask for a directory and start a project about it.
pub(crate) fn ask_for_a_directory(states: ProjectStates) {
    let mut proj = states.proj;
    ask_file(
        AsyncFileDialog::new().set_title("Open a directory as a project..."),
        AskFor::Folder,
        move |path| {
            new_project(states);
            proj.write().workspace_text = path.to_string_lossy().into_owned();
        },
    );
}

/// Ask for binaries and start a project holding them.
pub(crate) fn ask_for_a_binary(states: ProjectStates) {
    ask_files(
        binaries_dialog("Open a file as a project..."),
        move |paths| async move {
            new_project(states);
            open_binaries(states.objects, states.loading, paths).await;
        },
    );
}

/// Put the window back the way the session left it: the sidebar's arrangement, and the two
/// widths that were dragged.
///
/// Each is taken on its own, so a file that says nothing about one of them leaves that one
/// as it comes. A `None` from `DockArea::restored` is a saved arrangement this build can
/// make nothing of, which is the default sidebar and not an empty one.
fn restore_ui(arranged: Arrangement, ui: Option<&SavedUi>) {
    let Some(ui) = ui else {
        return;
    };
    let Arrangement {
        mut dock,
        mut sidebar,
        mut split,
    } = arranged;
    if let Some(saved) = ui.dock.as_ref().and_then(DockArea::restored) {
        dock.set(saved);
    }
    if let Some(width) = ui.sidebar {
        sidebar.set(width);
    }
    if let Some(ratio) = ui.split {
        split.set(ratio);
    }
}

/// Ask where to put the open project, and put it there.
///
/// A **save** dialog, which is the one place in the app that has one: the reader is naming
/// a file that is not there yet, and the extension is what makes it a project.
pub(crate) fn ask_where_to_save(states: ProjectStates, put: project::Put) {
    let mut proj = states.proj;
    let store = states.store;
    let suggested = proj
        .peek()
        .file
        .as_ref()
        .and_then(|file| {
            file.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|_| put == project::Put::Copy)
        .unwrap_or_else(|| format!("project.{}", project::PROJECT_EXTENSION));

    ask_file(
        AsyncFileDialog::new()
            .set_title("Save the project as...")
            .add_filter("Project", &[project::PROJECT_EXTENSION])
            .set_file_name(suggested),
        AskFor::Save,
        move |path| {
            let store = store.peek().clone();
            if store.is_some_and(|store| project::put_in(&store, &path, put)) {
                // The only thing that changed is where the project is kept, so this is
                // the only state that moves; the save observer sees no change and writes
                // nothing.
                proj.write().file = Some(path);
            }
        },
    );
}

/// Leave the project the app is in with none in its place.
pub(crate) fn close_project(states: ProjectStates) {
    project::close();
    empty_the_app(states);
}

/// The same, and take the project away with it.
pub(crate) fn delete_project(states: ProjectStates) {
    if project::delete() {
        empty_the_app(states);
    }
}

/// What both of those leave behind. `project::close` and `project::delete` re-point the
/// save policy **before** this runs, for `switch_project`'s reason: a baseline still
/// describing the project just left would read the emptying as a change and write it back
/// into it.
fn empty_the_app(states: ProjectStates) {
    clear_project(states);
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::default());
    bookmarks.set(Bookmarks::default());
}

/// Start a project the reader has not given a place and go to it.
pub(crate) fn new_project(states: ProjectStates) {
    let store = states.store.peek().clone();
    let Some(path) = store.and_then(|store| project::start_new(&store)) else {
        return;
    };

    clear_project(states);
    // The same way in as the other two. A default project and an empty session have
    // nothing to put back, so the restore does nothing.
    enter_project(states, path, Project::default(), Session::default());
}

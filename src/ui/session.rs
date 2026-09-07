//! The session as the UI keeps it in step with `project.rs`: what is saved when, what a
//! restore fills in, and what a switch empties. And the settings the same way: the wiring
//! between `settings.toml`, the appearance and the fonts. Nothing here draws; `app()`
//! calls all of it.

use super::*;

/// Resolve the appearance from the stored choice and the platform's own, and write it
/// through [`set_appearance`] -- the one function that may change it, and so the one that
/// empties `HIGHLIGHTED`.
///
/// **Not a `use_hook`**: reading `Platform::preferred_theme` subscribes this scope, so a
/// desktop that goes dark while the app is running repaints. It resolves in the render
/// body rather than in an effect, an effect being a frame late and a frame late on a dark
/// desktop a white flash; the write is idempotent, so that costs nothing.
pub(crate) fn use_theme(choice: ThemeChoice) {
    let preferred = *Platform::get().preferred_theme.read();

    set_appearance(resolve_appearance(choice, preferred));
}

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

    use_theme(settings.theme);

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
        asm_at,
        src_at,
        code_at,
        driven,
        // What each place had picked out is a view of its tab, and not saved.
        marks_at: _,
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
            about.details(),
            project::binaries(&objects),
            loading,
            bookmarks.read().entries().to_vec(),
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
                let shown_document = active_document(&strip, &docs);
                let shown = match (strip.active(), &shown_document) {
                    (Some(Tab::Page(page)), _) => OnScreen::Page(page),
                    (_, Some(document)) => OnScreen::Document(document),
                    _ => OnScreen::Nothing,
                };
                Session::from_state(
                    &objects,
                    &tabs,
                    &asm_at.read(),
                    &src_at.read(),
                    &code_at.read(),
                    &driven.read(),
                    shown,
                    &visits.read(),
                    &build.read().previous,
                    about.trusted,
                    // Reading these three is what subscribes the observer to a panel being
                    // dragged and to either handle being moved.
                    SavedUi {
                        sidebar: Some(*arranged.sidebar.read()),
                        split: Some(*arranged.split.read()),
                        dock: Some(arranged.dock.read().saved()),
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

/// Reopen the last project -- its name, binaries, tabs and selection -- once, at startup.
/// Which project that is, is `project::reopen`'s answer.
pub(crate) fn use_restore_on_startup(states: ProjectStates, opening: Option<PathBuf>) {
    // Outside the hook, a hook running inside another being what it is.
    let mut unopened = use_consume::<Unopened>().0;
    use_hook(move || {
        // What the app was given beats what it was last in. A file that will not parse
        // opens nothing and is said so, the same as one picked from a menu would be: it is
        // the reader's own file and is left exactly as it is.
        let store = states.store.peek().clone();
        let opened = match (store.as_ref(), &opening) {
            (Some(store), Some(path)) => Some(project::open_at(store, path)),
            (Some(store), None) => project::reopen(store),
            // Nowhere to keep anything, so nothing opens. Only worth saying to a reader
            // who asked for a project; a startup that would have reopened one has the
            // empty screen to show for it either way.
            (None, opening) => opening.clone().map(|path| {
                Err(project::Failure {
                    path,
                    reason: project::Reason::NoStore,
                })
            }),
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

        // Synchronously, and before anything else here: `project::reopen` has just
        // seeded the save policy's baselines from this same project, and the two have to
        // agree by the time the first effect runs or the save observer would see the
        // name, or the bookmarks, as a change and write them straight back out.
        let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
        proj.set(OpenProject::opened(file, &project, session.trusted));
        bookmarks.set(Bookmarks::from_entries(project.bookmarks.clone()));

        restore_project(states, project, session);
    });
}

/// Put a project's binaries, tabs, active document and visits on screen. Shared by the
/// two things that do a restore -- the app starting and a switch -- so the second cannot
/// drift from the first. Every step degrades silently.
///
/// The **pages go back first and synchronously**: one resolves against no object, so a
/// session whose only tab was Settings has nothing to wait for, and a project with no
/// binaries at all still comes back as the reader left it.
///
/// Two orderings are load-bearing among the documents. **Tabs before the active
/// document**: `open_document` opens what it cannot find, so restoring the active one
/// first would leave its tab out of place in the bar. **The rows go into the `Positions`
/// maps before each tab is shown**: a pane puts its view back when it notices the tab it
/// is showing has changed, so a row arriving after the tab is on screen arrives after the
/// only moment anything looks at it. A tab's trail is opened whole and its rows go in per
/// entry, so Back after a restart comes back to the rows that were left.
pub(crate) fn restore_project(states: ProjectStates, project: Project, session: Session) {
    // How the window was arranged. Before everything else and outside the early return
    // below: it is about the window and not about what is open in it, so a project with no
    // binaries left still comes back arranged the way it was left.
    restore_ui(states.arranged, session.ui.as_ref());

    // What the last build produced, which the next build replaces. Set before the early
    // return below: a project whose binaries are all gone still knows what it built.
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
        mut asm_at,
        mut src_at,
        mut code_at,
        mut driven,
        visits,
        ..
    } = states;

    // The pages, at the places they had in the bar, and the one that was on screen.
    // Before the two returns below, both of which are about binaries.
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

    if project.binaries.is_empty() {
        return;
    }

    // `spawn_forever`, not `spawn`: a task belongs to the scope that spawned it, and on a
    // switch that scope is the recent project's row, which the press unmounts -- the row
    // is left out of the list the moment its project is the open one, so the restore
    // would be dropped before its first poll.
    spawn_forever(async move {
        // The objects arrive as they are parsed, but the *session* waits for the whole
        // load: a tab is resolved against the objects by name, and resolving one against
        // a half-filled list would drop the tabs whose object had not landed yet.
        open_binaries(objects, loading, project.binaries.clone()).await;

        let (objects, mut visits) = (objects, visits);
        // Nothing opened: leave the app empty *and* leave the file alone.
        if objects.peek().is_empty() {
            return;
        }

        // Resolved against everything now loaded rather than just what this load
        // produced. All three computed before any is set, so no read guard is live when
        // anything is notified.
        let (restored_visits, restored_tabs, restored_active) = {
            let loaded = objects.read();
            (
                session.resolve_history(&loaded),
                session.resolve_tabs(&loaded),
                session.resolve(&loaded),
            )
        };

        // The record first, so the opening below finds the active place already at its
        // top and records nothing over it.
        visits.set(restored_visits);
        let (mut strip, mut docs) = (open.strip, open.docs);
        // Where in the bar the next tab goes. Counted over what survived rather than read
        // off the saved list, so the tabs that resolved keep their order around the pages
        // already put back.
        let mut position = 0;
        for tab in restored_tabs {
            let RestoredTab::Document {
                temporal,
                trail,
                entries,
            } = tab
            else {
                // A page is in the bar already, put there before the load; what it owes
                // the count is its place.
                position += 1;
                continue;
            };
            // The trail whole, in a statement of its own so the guard is gone before
            // the maps are written.
            let id = docs.write().open_trail(trail, temporal);
            let Some(id) = id else {
                continue;
            };
            // Where each side of each place was left, and what drove it, go in before
            // the tab is shown. The line for the same reason the rows are: a pane looks
            // at what it has been told exactly once, when it notices the place it is
            // showing has changed.
            {
                let (mut asm, mut src, mut from) = (asm_at.write(), src_at.write(), driven.write());
                let mut places = code_at.write();
                for entry in entries {
                    // The place itself, address and line and all: two stops in one
                    // object's code, or in one file, are two keys, as they were when
                    // they were saved.
                    let key = (
                        id,
                        Stop {
                            document: entry.document,
                            address: entry.address,
                            line: entry.src_line,
                        },
                    );
                    asm.remember(key.clone(), entry.asm_row);
                    src.remember(key.clone(), entry.src_row);
                    if let Some(line) = entry.line {
                        from.remember(key.clone(), line);
                    }
                    if let Some(address) = entry.address {
                        places.remember(key, Spot { address, rows: 0 });
                    }
                }
            }
            // Reopening a tab is not visiting it. Put at the place it had rather than
            // beside the tab on screen: the saved order is stated outright.
            strip.write().insert(Tab::Document(id), position);
            position += 1;
        }
        // The document the app lands on is a place it went: the tab showing it is
        // raised, or -- degraded to its object, say -- it opens in a tab of its own.
        if let Some(active) = restored_active {
            open_document(open, visits, active, Reach::NewTab);
        }
    });
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
        asm_at,
        src_at,
        code_at,
        driven,
        marks_at,
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
        close_binary(
            objects, loading, open, asm_at, src_at, code_at, driven, marks_at, visits, &path,
        );
    }

    let remaining = open.ids();
    for id in remaining {
        close_tab(open, asm_at, src_at, code_at, driven, marks_at, id);
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

/// Leave the project on screen and open the one the file at `path` holds in its place.
///
/// The order is what makes a switch safe: `project::switch` flushes the old project and
/// re-points every baseline while the policy still points at it, and only then is the app
/// emptied -- so the save observer, woken by a notify after this handler, sees one
/// settled state that matches the baseline and writes nothing.
pub(crate) fn switch_project(
    states: ProjectStates,
    mut rescued: State<Vec<PathBuf>>,
    mut unopened: State<Option<project::Failure>>,
    path: PathBuf,
) {
    let store = states.store.peek().clone();
    let switched = match store.as_ref() {
        Some(store) => project::switch(store, &path),
        None => Err(project::Failure {
            path: path.clone(),
            reason: project::Reason::NoStore,
        }),
    };
    let (project, session) = match switched {
        Ok(both) => both,
        Err(failure) => {
            // A project file is never moved aside and nothing is written over it, so
            // telling the reader why is the whole of what is left to do.
            unopened.set(Some(failure));
            return;
        }
    };

    // The other of the two loads a run makes, the startup's being `app()`'s. Added to
    // rather than set: a window still naming what the startup moved must not lose it.
    let moved = store::moved();
    if !moved.is_empty() {
        let mut naming = rescued.peek().clone();
        naming.extend(moved);
        rescued.set(naming);
    }

    clear_project(states);
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::opened(path, &project, session.trusted));
    bookmarks.set(Bookmarks::from_entries(project.bookmarks.clone()));
    restore_project(states, project, session);
}

/// Ask for a project file and open it in place of the one on screen.
///
/// `spawn_forever` in all three of these, not `spawn`: the dialog is asynchronous and,
/// through the xdg portal, not modal to the window, so the reader can raise another tab
/// while it is up -- and that unmounts the scope a `spawn` would belong to, losing the file
/// they then chose. Every state written here is a root state, so the write is good whatever
/// is on screen.
pub(crate) fn ask_for_a_project(
    states: ProjectStates,
    rescued: State<Vec<PathBuf>>,
    unopened: State<Option<project::Failure>>,
) {
    spawn_forever(async move {
        let Some(handle) = AsyncFileDialog::new()
            .set_title("Open a project...")
            .add_filter("Project", &[project::PROJECT_EXTENSION])
            .pick_file()
            .await
        else {
            return;
        };
        switch_project(states, rescued, unopened, handle.path().to_path_buf());
    });
}

/// Ask for a directory and start a project about it.
pub(crate) fn ask_for_a_directory(states: ProjectStates) {
    let mut proj = states.proj;
    spawn_forever(async move {
        let Some(handle) = AsyncFileDialog::new()
            .set_title("Open a directory as a project...")
            .pick_folder()
            .await
        else {
            return;
        };
        new_project(states);
        proj.write().directory = handle.path().to_string_lossy().into_owned();
    });
}

/// Ask for binaries and start a project holding them.
pub(crate) fn ask_for_a_binary(states: ProjectStates) {
    spawn_forever(async move {
        let Some(handles) = AsyncFileDialog::new()
            .set_title("Open a file as a project...")
            .pick_files()
            .await
        else {
            return;
        };
        new_project(states);
        let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();
        open_binaries(states.objects, states.loading, paths).await;
    });
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

    spawn_forever(async move {
        let Some(handle) = AsyncFileDialog::new()
            .set_title("Save the project as...")
            .add_filter("Project", &[project::PROJECT_EXTENSION])
            .set_file_name(suggested)
            .save_file()
            .await
        else {
            return;
        };
        let path = handle.path().to_path_buf();
        let store = store.peek().clone();
        if store.is_some_and(|store| project::put_in(&store, &path, put)) {
            // The only thing that changed is where the project is kept, so this is the
            // only state that moves; the save observer sees no change and writes nothing.
            proj.write().file = Some(path);
        }
    });
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
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::opened(path, &Project::default(), false));
    bookmarks.set(Bookmarks::default());
}

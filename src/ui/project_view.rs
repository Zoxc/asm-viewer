//! Which project is open: the pane that says so, and the machinery behind it -- when the two
//! files are written, what a switch empties and what a restore fills back in.
//!
//! **One view and not two**, where `notes/Goals.md` asks for a project view and a
//! recent-projects view separately: they are one question, and the recent list is how a
//! reader *leaves* the project the pane above it describes. The pane and the lifecycle are
//! one file for that same reason -- [`switch_project`] is what those rows call.
//!
//! A switch is a close and a restore through the functions a reader's own clicks go through,
//! never a write to the list. The ordering is what makes it safe: the baselines are emptied
//! before the app is, so the save observer -- woken by a notify and run after the whole
//! handler -- sees one settled state and writes nothing.

use super::*;

/// One binary the project has open, and how many objects came out of it.
///
/// Read off the loaded objects rather than off the saved `binaries`, because that is what
/// `project::binaries` derives the saved list *from*: what this row draws is therefore
/// what the next write will say, and a file closed from the Objects panel leaves this
/// list in the same instant it leaves that one.
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

/// One project in the recent list. Pressing it leaves the project on screen and opens
/// this one in its place.
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

        // The id where there is no name, in the colour a tag is drawn in: a project is
        // its directory, so the one thing it always has to be called is that directory's
        // name -- and drawing it as a name would claim the reader chose it.
        let (text, color) = match &recent.name {
            Some(name) => (name.clone(), palette().text_fg),
            None => (recent.id.as_str().to_owned(), palette().address_fg),
        };
        // What is known about it without opening it: where it points, and how much is in
        // it. Both come out of that project's own file.
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

/// The Project pane: everything the app knows about the project it is in, the two things
/// about it the reader can say, and the other projects they can go to.
///
/// **One view and not two**, where `notes/Goals.md` asks for a project view and a
/// recent-projects view separately. They are one question -- which project am I in, and
/// what else is there -- and the recent list is how the reader *leaves* the project the
/// rest of the pane describes, so a tab of its own would be a tab that is empty in every
/// session where a project was reopened, which is all of them after the first. The goal's
/// "if none was open" case is answered by the pane itself: with no project the top half
/// says so and the list is the whole of what there is to do.
///
/// The recent list deliberately leaves out the project that is open. The pane above it is
/// already describing that one, in more detail and from live state rather than from a
/// file, so a row for it would be a second and staler copy of the name being typed three
/// lines higher up.
#[derive(PartialEq)]
pub(crate) struct ProjectTab;

impl Component for ProjectTab {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut proj = states.proj;
        let objects = states.objects;

        // Every row of the recent list is a small read of another project's own file, so
        // it is read when this view is mounted and again when the open project changes --
        // never per render, which a hover is. The effect also runs once on mount, which
        // costs one extra reading of a handful of short files and buys the alternative
        // not being a frame of "no recent projects" before the first one arrives.
        let mut recents = use_state(project::recent_projects);
        let open = proj.read().clone();
        use_side_effect_with_deps(&open.id, move |_: &Option<ProjectId>| {
            recents.set(project::recent_projects());
        });

        // What is open, grouped the way the saved list is: by path, in the order the
        // files were opened.
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
                        // The two editable fields. Each writes straight into `Proj`, so a
                        // keystroke is a state change the save observer sees like any
                        // other -- and `name` and `directory` live in `project.toml`,
                        // which is the file written at once, so a rename is on disk before
                        // the next click. That is `Goals.md`'s "user project changes save
                        // immediately" taken literally, and it costs a few hundred bytes
                        // written atomically per keystroke of something typed once.
                        .child(field_row(
                            "Name",
                            Input::new(
                                proj.into_writable()
                                    .map(|open| &open.name, |open| &mut open.name),
                            )
                            // An empty box is a project that has not been named, which is
                            // what makes it anonymous -- so the placeholder says that
                            // rather than inviting a name.
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
                        // The directory the project is *stored* in, which is its identity
                        // and is never written inside either of the files in it. Shown
                        // because it is what the recent list names a project by and what
                        // a reader looking for these files on disk needs.
                        .child(field_row(
                            "Stored as",
                            label()
                                .text(match &open.id {
                                    Some(id) => id.as_str().to_owned(),
                                    // Not an error and not a missing project: a project
                                    // directory is made by the first write that has
                                    // something to put in it, so a run in which nothing
                                    // has been opened or named has none yet.
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
/// `use_side_effect` re-runs its callback whenever a `State` that was `read()` inside
/// it changes (`freya-core/src/lifecycle/effect.rs`), so reading the state contexts
/// here makes this one observer the single choke point every mutation flows through:
/// `activate`, the toolbar's `objects.write()`, the history push inside `activate` and
/// the tab list know nothing about persistence, and neither will any future one. The subscriptions *are* the `read()` calls, which
/// is the whole of what makes adding a persisted field to `Session::from_state` also
/// add the state behind it to what wakes this.
///
/// Whether a change reaches the disk now or at the next `use_periodic_save` tick is
/// `project::record`'s decision, not this one's: opening a binary is written at once,
/// a document, a tab or a history entry is left pending. That policy is framework-free
/// and unit-tested in `project.rs`; all this hook owns is *when to look*.
///
/// One visit wakes this up to three times -- for `Active`, for the tab `activate` opened
/// and for the history entry it pushed -- which costs three derivations and three
/// comparisons and, since none of them is a binaries change, no write at all.
///
/// Scrolling a pane wakes it too, which is the one input here that a reader can produce
/// continuously. It costs no more than the three above, and it is bounded by the unit the
/// position is kept in: a viewing position is a *row*, so a scroll writes nothing until
/// the pane has moved a whole row, and `use_kept_position` compares before it
/// writes.
pub(crate) fn use_save_on_change(states: ProjectStates) {
    let ProjectStates {
        proj,
        objects,
        // What is still being read is deliberately not saved and deliberately does not
        // wake this: `binaries` is derived from the objects, so a file joins the saved
        // list when its first object lands and a file that never parses is never named,
        // which is exactly what it did before anything streamed.
        loading: _,
        open,
        asm_at,
        src_at,
        history,
    } = states;

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it. Each
        // guard lives to the end of the statement it is created in, which is the one
        // `record` call, and nothing here writes anything, so holding several at once is
        // the safe half of the `peek`/`write` gotcha rather than the dangerous one.
        let objects = objects.read();
        project::record(
            // The user-given half, which since 8e is a state like the rest rather than
            // something the save policy had to carry: the project view holds it, so it
            // arrives here the same way the binaries do and a rename is recorded by the
            // same observer that records everything else.
            proj.read().details(),
            project::binaries(&objects),
            {
                // The dock and the table rather than `Active`: this has to write down
                // what is open *now*, and `Active` is a memo that catches up a beat
                // later. Reading the dock here is also what wakes this on a layout drag
                // -- `record` compares against its baselines and writes nothing, so that
                // is a wasted wake rather than a wasted write.
                let (dock, docs) = (open.dock.read(), open.docs.read());
                Session::from_state(
                    &objects,
                    &open_documents(&dock, &docs),
                    &asm_at.read(),
                    &src_at.read(),
                    active_document(&dock, &docs).as_ref(),
                    &history.read(),
                )
            },
        );
    });
}

/// Write out a pending change every `AUTOSAVE_INTERVAL`.
///
/// `use_hook` runs its initializer on mount and never again, so exactly one of these
/// loops exists; `spawn` is freya's own task spawner, and `async_io::Timer` is what
/// freya itself waits on inside spawned tasks (`freya-animation`'s hook and
/// `freya-sdk`'s timeout both do), so this adds no runtime -- async-io's reactor is
/// already in the process.
///
/// A tick that finds nothing pending does no IO at all, which is what makes the empty
/// baseline in `Saves` matter here: a tick during the startup parse, before anything
/// has been restored, has nothing to write and so cannot put an empty project over a
/// good file.
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
///
/// *Which* project that is, and what a project even is, is `project::reopen`'s: the app
/// asks for the last one and is handed its id and its two halves, or nothing. Nothing
/// here chooses, which is what keeps the recent-projects view and this hook from being
/// two answers to the same question: that view goes through [`switch_project`], which
/// ends in the same [`restore_project`] this does.
///
/// `use_hook` runs its initializer on mount and never again, which is what makes this
/// happen exactly once.
pub(crate) fn use_restore_on_startup(states: ProjectStates) {
    use_hook(move || {
        let Some((id, project, session)) = project::reopen() else {
            return;
        };

        // Synchronously, and before anything else here: `project::reopen` has just seeded
        // the save policy's baseline from this same project, and the two have to agree by
        // the time the first effect runs or the save observer would see the name as a
        // change and write it straight back out -- with the binaries still empty, since
        // those are restored a worker thread later. Hooks run during the parent's render
        // and effects after it, which is what makes "before" a fact rather than a hope.
        let mut proj = states.proj;
        proj.set(OpenProject::opened(id, &project));

        restore_project(states, project, session);
    });
}

/// Put a project's binaries, tabs, active document and history on screen.
///
/// The whole of what a restore *is*, and shared by the two things that do one -- the app
/// starting and a switch to another project -- so that the second cannot drift from the
/// first. It is the toolbar's `on_open` pattern verbatim for the parsing itself:
/// CPU-bound `open_files` on a `std::thread`, the result back over an `async_channel`,
/// `spawn` being freya's own task spawner and callable both during render and from an
/// event handler. So a large binary parses with the window already up and interactive.
///
/// Every step degrades silently: no project or an unreadable one is `None`, a path that
/// no longer exists or no longer parses just contributes no `Object` (`open_files`
/// swallows its own failures), `Session::resolve` falls back from a vanished symbol to
/// its object and from a vanished object to nothing, and `Session::resolve_history` and
/// `Session::resolve_tabs` drop what no longer points anywhere -- the history keeping
/// its cursor on the right one. A source-driven tab resolves against nothing and so
/// always comes back, a deleted file included: it returns as a tab over the pane's own
/// "Source file not found", which is the true answer and a visible one.
///
/// **The strip is rebuilt through the functions that hold the app's invariants**, never
/// by writing the list directly, so a restored session is in a state the app could have
/// got into by hand: every tab through [`activate`], of either kind. Two orderings follow
/// from that and are the only genuinely new rules here:
///
/// - The **tabs before the active document**. `activate` opens what it cannot find, so
///   restoring the active one first would leave its tab at the end of the strip instead
///   of in the place the reader left it. The other direction is safe: it can have
///   degraded to its object while the strip still holds the symbol, and `activate` simply
///   opens a tab for it, which is also what the reader would see had they closed that tab
///   themselves.
/// - The **rows go into the two `Positions` maps before the tabs are opened**. Those maps
///   are the one thing the restore writes directly, and a pane puts its view back when it
///   notices the tab it is showing has changed, so a row arriving after the `activate`
///   arrives after the only moment anything looks at it.
///
/// Every write below happens in one go, before the frame can end: freya's effects are
/// woken by an async notify (`Effect::create`) rather than run at the write, so
/// `use_save_on_change` sees the settled result once and not each intermediate `Active`
/// the tab loop passes through.
fn restore_project(states: ProjectStates, project: Project, session: Session) {
    let ProjectStates {
        objects,
        loading,
        open,
        mut asm_at,
        mut src_at,
        history,
        ..
    } = states;

    if project.binaries.is_empty() {
        return;
    }

    spawn(async move {
        // The objects arrive as they are parsed and the sidebar fills in behind them, so
        // the reader can be clicking through the first archive member before the last one
        // exists. What waits for the whole load is the *session*: a tab, the active
        // document or a history entry is resolved against the objects by name, and
        // resolving one against a half-filled list would drop the tabs whose object had
        // not landed yet.
        open_binaries(objects, loading, project.binaries.clone()).await;

        let (objects, mut history) = (objects, history);
        // Nothing opened: leave the app empty *and* leave the file alone, so a
        // binary that is only temporarily missing is not forgotten.
        if objects.peek().is_empty() {
            return;
        }

        // Resolved against everything now loaded rather than just what this load
        // contributed, so this stays correct if the user managed to open something
        // first. All three are computed before any of them is set so the read guard is
        // long gone by the time anything is notified.
        let (restored_history, restored_tabs, restored_active) = {
            let loaded = objects.read();
            (
                session.resolve_history(&loaded),
                session.resolve_tabs(&loaded),
                session.resolve(&loaded),
            )
        };

        // The history first, so that the `Visit::Went` at the end of this has a cursor to
        // dedup against.
        history.set(restored_history);

        // Where each side of each tab was left goes in *before* the tab is opened; see
        // above. Then the strip, oldest tab first, and then the one that was active. Each
        // of these is an `Active` write that will be overwritten by the next, which is the
        // price of there being exactly one way to open a tab; the last one is the only one
        // anything observes.
        {
            let (mut asm, mut src) = (asm_at.write(), src_at.write());
            for (tab, asm_row, src_row) in &restored_tabs {
                asm.remember(tab.clone(), *asm_row);
                src.remember(tab.clone(), *src_row);
            }
        }
        for (tab, _, _) in restored_tabs {
            // Reopening a tab is not visiting it: the reader had it open, and the history
            // this restore has just set is the record of where they went.
            activate(open, history, Some(tab), Visit::Moved);
        }
        // The one exception, and it is what keeps the cursor and the app in step: the
        // document the app *lands on* is a place it went. `would_push` makes it free in
        // the ordinary case — the saved cursor entry is the saved active document, and
        // the two resolve through the same lookup to the same `Arc`s — and records it
        // exactly when they differ, which is when the cursor entry was dropped or the
        // active document degraded and the app really is somewhere new.
        activate(open, history, restored_active, Visit::Went);
    });
}

/// Empty the app of everything that belonged to the project being left.
///
/// **Through the functions that hold the invariants and nothing else**, which is the
/// same rule a restore goes through in the other direction: closing every binary takes
/// its objects, its assembly-driven tabs, their viewing positions, the history entries
/// into it and the active document with them ([`close_binary`]), and the source-driven
/// tabs it deliberately leaves standing are then closed one by one ([`close_tab`]).
/// Writing the list directly would be shorter and would be the one place in the app where
/// "the active document is the active tab" was held by hand.
///
/// The **history** is then emptied outright, which is the one thing here that no walk
/// reaches: `close_binary` drops only the entries into the file it closes and `close_tab`
/// drops none at all, so a visited source file — which belongs to no binary — would
/// otherwise survive into the project that comes next.
///
/// The source tabs go here where a closing *binary* deliberately leaves them alone: a
/// file tab outlives the binary that led the reader to it because the text stands on its
/// own, but it does not outlive the project, whose session is what recorded that it was
/// open.
pub(crate) fn clear_project(states: ProjectStates) {
    let ProjectStates {
        objects,
        mut loading,
        open,
        asm_at,
        src_at,
        history,
        ..
    } = states;

    // Every load at once, and before the closes rather than through them: a file that has
    // been asked for and has produced nothing yet is not in the objects list, so nothing
    // below would reach it, and its objects would arrive into the project that comes next.
    loading.write().clear();

    // Both reads are bound before anything writes, which is the `peek` guard rule and
    // also the plain iteration rule: `close_binary` writes the very list being walked.
    let binaries = project::binaries(&objects.peek());
    for path in binaries {
        close_binary(objects, loading, open, asm_at, src_at, history, &path);
    }

    let remaining = open.documents();
    for tab in &remaining {
        close_tab(open, history, asm_at, src_at, tab);
    }

    // And the history outright, which the two walks above deliberately do not do for it.
    // `close_binary` drops only the entries into the file it is closing, and `close_tab`
    // drops none at all -- a history entry outlives its tab, which is the whole point of
    // there being two lists. Neither reaches a visited *source file*, which belongs to no
    // binary; and the history belongs to the project, whose session is what recorded it.
    let mut history = history;
    history.set(History::default());
}

/// Leave the project on screen and open the one `id` names in its place.
///
/// Three steps, in an order that is the whole of why a switch is safe. `project::switch`
/// goes first: it flushes what the old project had pending while the save policy still
/// points at it, and re-points every baseline at the new one — empty, because the app is
/// about to be empty. Only then is the app emptied, so the save observer, which is woken
/// by a notify and runs after this handler rather than during it, sees one settled state
/// that matches the baseline exactly and writes nothing at all. The restore then arrives
/// as an ordinary change and is written into the new project the way any other is.
///
/// A project whose directory has gone since the list named it does nothing but leave the
/// reader where they are; the row goes on the next reading of the list.
fn switch_project(states: ProjectStates, id: ProjectId) {
    let Some((project, session)) = project::switch(&id) else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &project));
    restore_project(states, project, session);
}

/// Start a project nobody has named yet and go to it. [`switch_project`] with nothing to
/// restore, an empty project being empty.
fn new_project(states: ProjectStates) {
    let Some(id) = project::start_new() else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &Project::default()));
}

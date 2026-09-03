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

/// One thing the last build produced. Pressing it opens the file as a binary, unless it is
/// open already: opening a path twice would put a second copy of each of its objects in
/// the list.
#[derive(Clone, PartialEq)]
struct ArtifactRow {
    artifact: cargo::Artifact,
    key: DiffKey,
}

impl KeyExt for ArtifactRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ArtifactRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let path = self.artifact.path.clone();
        let text = path.to_string_lossy().into_owned();
        // What cargo calls the target, and what kind it is: the two things that tell one
        // row from another when the file names are hashes.
        let about = format!("{} {}", self.artifact.target, self.artifact.kind);

        row_tooltip(
            text.clone(),
            CursorArea::new().child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height()))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(8.0)
                    .content(Content::Flex)
                    .background(match hovering() {
                        true => palette().object_hover_bg,
                        false => Color::TRANSPARENT,
                    })
                    .on_pointer_over(move |_| hovering.set_if_modified(true))
                    .on_pointer_out(move |_| hovering.set_if_modified(false))
                    .on_press(move |_| {
                        let open = states
                            .objects
                            .peek()
                            .iter()
                            .any(|object| object.path == path)
                            || states.loading.peek().is_loading(&path);
                        if open {
                            return;
                        }

                        let (objects, loading, path) =
                            (states.objects, states.loading, path.clone());
                        spawn(async move {
                            open_binaries(objects, loading, vec![path]).await;
                        });
                    })
                    .child(tree_name(text, false))
                    .child(label().text(about).color(palette().address_fg).max_lines(1)),
            ),
        )
    }
}

/// The place a diagnostic points at, drawn as a **target** where this pane can reach it:
/// pressing it opens that file as source, on the line and column the compiler named.
///
/// cargo spells the file relative to where it ran, so the place is the project's directory
/// joined with it. A file **outside** that directory -- a dependency's, out of the registry
/// -- keeps the plain label it would have had: the app opens a source file it can read, and
/// a target that did nothing when pressed would be worse than never offering one.
fn source_place(directory: Option<&Path>, diagnostic: &Diagnostic) -> Option<Element> {
    let span = diagnostic.span.as_ref()?;
    let file = directory.map(|directory| directory.join(&span.file));
    let own = file.as_deref().is_some_and(|file| {
        file.starts_with(directory.unwrap_or(Path::new(""))) && shows_as_source(file)
    });
    let text = diagnostic_place(span, own);

    Some(match (own, file) {
        (true, Some(file)) => SourceTarget {
            file: Arc::from(&*file.to_string_lossy()),
            line: span.line as u32,
            text,
        }
        .into_element(),
        _ => label()
            .text(text)
            .color(palette().address_fg)
            .max_lines(1)
            .into_element(),
    })
}

/// A diagnostic's place, as a row of the project's own source. The relocation link's own
/// hover, which is what says "this can be pressed" everywhere else in this app.
#[derive(Clone, PartialEq)]
struct SourceTarget {
    file: Arc<str>,
    line: u32,
    text: String,
}

impl Component for SourceTarget {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let (file, line) = (self.file.clone(), self.line);

        CursorArea::new().child(
            rect()
                .maybe(hovering(), |rect| {
                    rect.background(palette().link_hover_bg).corner_radius(6.0)
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // The blocks are in a `ScrollView` that drags to scroll, and a press
                    // that reached it would be the start of one.
                    e.stop_propagation();

                    land(
                        states.open,
                        states.visits,
                        marked,
                        landing,
                        plant,
                        Landing {
                            tab: Document::Source(file.clone()),
                            at: Some(LinePos {
                                file: file.clone(),
                                line,
                            }),
                            // A source file and no instruction: the compiler named a line.
                            address: None,
                            columns: None,
                        },
                        reach(ctrl),
                    );
                })
                .child(
                    label()
                        .text(self.text.clone())
                        .max_lines(1)
                        .color(match hovering() {
                            true => palette().name_hover_fg,
                            false => palette().address_fg,
                        }),
                ),
        )
    }
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

        // What there is to build, and what to build it with. The read subscribes this
        // component to the build, so a finished one redraws the rows below.
        let build = states.build;
        let held = build.read().clone();
        let jobs = use_consume::<BuildJobs>();
        let directory = workspace(&open);
        let profile = open.profile;

        // The manifest is read on mount and whenever the directory or the profile
        // changes -- the two things that decide what the answer is. A keystroke in the
        // directory box costs one `read_to_string` of a half-typed path, which fails
        // cheaply, `files_view`'s own bargain.
        use_side_effect_with_deps(&(directory.clone(), profile), {
            let jobs = jobs.clone();
            move |(directory, profile): &(Option<PathBuf>, Profile)| {
                let Some(directory) = directory.clone() else {
                    return;
                };
                jobs.send(BuildJob::Read {
                    directory,
                    profile: *profile,
                });
            }
        });

        let artifacts: Vec<Element> = held
            .artifacts()
            .iter()
            .map(|artifact| {
                ArtifactRow {
                    artifact: artifact.clone(),
                    key: DiffKey::None,
                }
                .key(artifact.path.to_string_lossy().into_owned())
                .into()
            })
            .collect();

        let diagnostics: Vec<Element> = held
            .diagnostics()
            .iter()
            .map(|diagnostic| {
                diagnostic_block(diagnostic, source_place(directory.as_deref(), diagnostic))
            })
            .collect();

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
                            "Cargo build",
                            directory.clone().map(|directory| {
                                let jobs = jobs.clone();
                                Button::new()
                                    // Two builds cannot go at once: the second would
                                    // compile what the first is writing.
                                    .enabled(held.manifest.is_some() && !held.building)
                                    .on_press(move |_| {
                                        start_build(build, &jobs, directory.clone(), profile)
                                    })
                                    .child(match held.building {
                                        true => "Building...",
                                        false => "Build",
                                    })
                                    .into_element()
                            }),
                        ))
                        .child(match &held.manifest {
                            None => info_line(match directory.is_some() {
                                true => "No Cargo.toml in the directory".to_owned(),
                                false => "No directory".to_owned(),
                            })
                            .into_element(),
                            Some(manifest) => rect()
                                .width(Size::fill())
                                .spacing(6.0)
                                // The file cargo is run over, named rather than implied:
                                // what is built is a question the directory alone answers
                                // only for a reader who knows the rule.
                                .child(field_row(
                                    "Manifest",
                                    label()
                                        .text(manifest.to_string_lossy().into_owned())
                                        .color(palette().address_fg)
                                        .max_lines(1),
                                ))
                                .child(field_row(
                                    "Profile",
                                    SegmentedButton::new().children(
                                        [(Profile::Debug, "Debug"), (Profile::Release, "Release")]
                                            .map(|(choice, text)| {
                                                ButtonSegment::new()
                                                    .key(text)
                                                    .selected(profile == choice)
                                                    // Straight into `Proj`, so the save
                                                    // observer sees it like a rename and
                                                    // `project.toml` is written at once.
                                                    .on_press(move |_| {
                                                        proj.write().profile = choice;
                                                    })
                                                    .child(text)
                                                    .into()
                                            }),
                                    ),
                                ))
                                // What a binary with no line information costs is the
                                // whole source side, so the offer is made where the
                                // profile is chosen and goes as soon as it is taken.
                                .maybe_child((!held.debug_lines).then(|| {
                                    let jobs = jobs.clone();
                                    let directory = directory.clone();
                                    field_row(
                                        "Debug lines",
                                        rect()
                                            .width(Size::flex(1.0))
                                            .horizontal()
                                            .cross_align(Alignment::Center)
                                            .content(Content::Flex)
                                            .spacing(6.0)
                                            .child(
                                                label()
                                                    .text("Off, so there is no source side")
                                                    .width(Size::flex(1.0))
                                                    .color(palette().address_fg),
                                            )
                                            .child(
                                                Button::new()
                                                    .on_press(move |_| {
                                                        let Some(directory) = directory.clone()
                                                        else {
                                                            return;
                                                        };
                                                        jobs.send(BuildJob::AddDebugLines {
                                                            directory,
                                                            profile,
                                                        });
                                                    })
                                                    .child("Turn on"),
                                            ),
                                    )
                                    .into_element()
                                }))
                                .maybe_child(held.status().map(|(text, bad)| {
                                    info_line_in(
                                        text,
                                        match bad {
                                            true => palette().invalid_fg,
                                            false => palette().address_fg,
                                        },
                                    )
                                    .into_element()
                                }))
                                .children(artifacts)
                                // cargo's own words, for what it says nowhere else.
                                .maybe_child(
                                    held.refusal()
                                        .map(|message| text_block(message, palette().text_fg)),
                                )
                                // Drawn straight into the pane's own scroll: a wrapping
                                // block has no height a virtual list could use, and this
                                // whole view scrolls already.
                                .children(diagnostics)
                                .into_element(),
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
        // What each place had picked out is a view of its tab, and not saved.
        marks_at: _,
        visits,
        bookmarks,
        // A search is a view of the project's files, not part of the session.
        searched: _,
        build,
    } = states;

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it.
        let objects = objects.read();
        project::record(
            proj.read().details(),
            project::binaries(&objects),
            bookmarks.read().entries().to_vec(),
            {
                // The dock and the table rather than `Active`, which is a memo and so a
                // beat behind.
                let (dock, docs) = (open.dock.read(), open.docs.read());
                let tabs: Vec<(DocId, &History, bool)> = open_ids(&dock)
                    .into_iter()
                    .filter_map(|id| {
                        docs.trail(id)
                            .map(|trail| (id, trail, docs.temporal() == Some(id)))
                    })
                    .collect();
                Session::from_state(
                    &objects,
                    &tabs,
                    &asm_at.read(),
                    &src_at.read(),
                    &code_at.read(),
                    &driven.read(),
                    active_document(&dock, &docs).as_ref(),
                    &visits.read(),
                    &build.read().previous,
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
pub(crate) fn use_restore_on_startup(states: ProjectStates) {
    use_hook(move || {
        let Some((id, project, session)) = project::reopen() else {
            return;
        };

        // Synchronously, and before anything else here: `project::reopen` has just
        // seeded the save policy's baselines from this same project, and the two have to
        // agree by the time the first effect runs or the save observer would see the
        // name, or the bookmarks, as a change and write them straight back out.
        let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
        proj.set(OpenProject::opened(id, &project));
        bookmarks.set(Bookmarks::from_entries(project.bookmarks.clone()));

        restore_project(states, project, session);
    });
}

/// Put a project's binaries, tabs, active document and visits on screen. Shared by the
/// two things that do a restore -- the app starting and a switch -- so the second cannot
/// drift from the first. Every step degrades silently.
///
/// Two orderings are load-bearing. **Tabs before the active document**: `open_document`
/// opens what it cannot find, so restoring the active one first would leave its tab out
/// of place in the strip. **The rows go into the `Positions` maps before each tab is
/// shown**: a pane puts its view back when it notices the tab it is showing has changed,
/// so a row arriving after the tab is on screen arrives after the only moment anything
/// looks at it. A tab's trail is opened whole and its rows go in per entry, so Back
/// after a restart comes back to the rows that were left.
fn restore_project(states: ProjectStates, project: Project, session: Session) {
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

    if project.binaries.is_empty() {
        return;
    }

    spawn(async move {
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
        let (mut dock, mut docs) = (open.dock, open.docs);
        for tab in restored_tabs {
            // The trail whole, in a statement of its own so the guard is gone before
            // the maps are written.
            let id = docs.write().open_trail(tab.trail, tab.temporal);
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
                for entry in tab.entries {
                    let key = (id, entry.document);
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
            // Reopening a tab is not visiting it.
            dock.write().show_document(Tab::Document(id));
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

    // And the record outright, which neither walk above does.
    let mut visits = visits;
    visits.set(Visits::default());

    // The search likewise: its hits are places in the directory being left, and dropping
    // the question is also what stops a walk still running -- the task takes the next
    // batch, sees a search it is not, and lets its end of the channel go.
    searched.set(Searched::default());

    // What one project built says nothing about the next, and a list left standing would
    // have the first build over there replace binaries opened over here.
    let mut build = states.build;
    build.set(Builds::default());
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
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::opened(id, &project));
    bookmarks.set(Bookmarks::from_entries(project.bookmarks.clone()));
    restore_project(states, project, session);
}

/// Start a project nobody has named yet and go to it.
fn new_project(states: ProjectStates) {
    let Some(id) = project::start_new() else {
        return;
    };

    clear_project(states);
    let (mut proj, mut bookmarks) = (states.proj, states.bookmarks);
    proj.set(OpenProject::opened(id, &Project::default()));
    bookmarks.set(Bookmarks::default());
}

//! Which project is open: the pane that says so, what the reader can say about it, and
//! the other projects they can go to.

use super::*;

/// One binary the project holds, by the path it was opened from, with how many objects
/// came out of it.
///
/// A component and not a function of the pane's render: the row's tooltip is the path it
/// already draws, so it is shown only where the path was cut, and asking that needs a
/// hook -- which a function called once per binary cannot hold.
#[derive(Clone, PartialEq)]
struct BinaryRow {
    path: PathBuf,
    objects: usize,
    key: DiffKey,
}

impl KeyExt for BinaryRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for BinaryRow {
    fn render(&self) -> impl IntoElement {
        let fitted = use_fitted();
        let text = self.path.to_string_lossy().into_owned();

        cut_tooltip(
            fitted.cut(),
            text.clone(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(8.0)
                .content(Content::Flex)
                .child(tree_name_fitted(fitted, text, false))
                .child(
                    label()
                        .text(match self.objects {
                            1 => "1 object".to_owned(),
                            many => format!("{many} objects"),
                        })
                        .color(palette().address_fg)
                        .max_lines(1),
                ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One setting the project's own `.vscode/settings.json` gave the language server: the
/// name with `rust-analyzer.` off it, and the value as it will be sent.
///
/// A row and not a `field_row`: these names are long enough to be the whole of the left
/// column and there is no reason for the values to line up with the fields above.
fn override_row(name: &str, value: &str) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .padding(Gaps::new_symmetric(0.0, 5.0))
        .child(
            label()
                .text(name.to_owned())
                .color(palette().text_fg)
                .max_lines(1),
        )
        .child(
            label()
                .text(value.to_owned())
                .width(Size::flex(1.0))
                .color(palette().address_fg)
                .max_lines(1),
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
        let fitted = use_fitted();
        let states = use_project_states();
        let path = self.artifact.path.clone();
        let text = path.to_string_lossy().into_owned();
        // What cargo calls the target, and what kind it is: the two things that tell one
        // row from another when the file names are hashes.
        let about = format!("{} {}", self.artifact.target, self.artifact.kind);

        cut_tooltip(
            fitted.cut(),
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
                        // `spawn_forever`, not `spawn`: a task belongs to the scope
                        // that spawned it, and this row is drawn only while the
                        // Project page is the tab on screen. Raising another tab
                        // would drop the load half-read, leaving a row in the
                        // Objects list that never stops loading.
                        spawn_forever(async move {
                            open_binaries(objects, loading, vec![path]).await;
                        });
                    })
                    .child(tree_name_fitted(fitted, text, false))
                    .child(label().text(about).color(palette().address_fg).max_lines(1)),
            ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
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
                .maybe(hovering(), |rect| link_chrome(rect, None))
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
/// screen. Drawn by the Project view and by the screen a window with no project is
/// (`src/ui/no_project.rs`).
#[derive(Clone, PartialEq)]
pub(crate) struct RecentRow {
    pub(crate) recent: Recent,
    pub(crate) key: DiffKey,
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
        let rescued = use_consume::<Rescued>().0;
        let unopened = use_consume::<Unopened>().0;
        let path = self.recent.path.clone();
        let recent = &self.recent;

        // What the project is called, which is what its file is called.
        let text = project::label(&recent.path);
        // What is known about it without opening it, out of its own file.
        let about = match &recent.directory {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => match recent.binaries {
                0 => "empty".to_owned(),
                1 => "1 binary".to_owned(),
                many => format!("{many} binaries"),
            },
        };

        extra_tooltip(
            recent.path.to_string_lossy().into_owned(),
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
                .on_press(move |_| switch_project(states, rescued, unopened, path.clone()))
                .child(one_line(text).width(Size::flex(1.0)))
                .child(label().text(about).color(palette().address_fg).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The Project pane: the project the app is in, what the reader can say about it, and the
/// other projects they can go to. The recent list leaves out the open project, which the
/// pane above it already describes.
#[derive(PartialEq)]
pub(crate) struct ProjectTab;

impl Component for ProjectTab {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut proj = states.proj;
        let objects = states.objects;

        // Read on mount and again when the open project changes, never per render:
        // each row is a small read of another project's own file.
        let store = states.store;
        let mut recents = use_state(move || recents_of(store));
        let open = proj.read().clone();
        use_side_effect_with_deps(&open.file, move |_: &Option<PathBuf>| {
            recents.set(recents_of(store));
        });

        let binaries: Vec<Element> = {
            let objects = objects.read();
            project::binaries(&objects)
                .into_iter()
                .map(|path| {
                    let count = objects.iter().filter(|object| object.path == path).count();
                    BinaryRow {
                        key: DiffKey::None,
                        objects: count,
                        path: path.clone(),
                    }
                    .key(path.to_string_lossy().into_owned())
                    .into()
                })
                .collect()
        };

        // What there is to build, and what to build it with. The read subscribes this
        // component to the build, so a finished one redraws the rows below.
        let build = states.build;
        let held = build.read().clone();
        // The language server's, which this view only says how it went; the control that
        // starts and stops it is in the top bar (`src/ui/language.rs`).
        let language = use_consume::<Talking>().0;
        // Read into a value of its own: the presses below write the state this looked at.
        let spoken = language.read().clone();
        let lsp = use_consume::<LspJobs>();
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
            .filter(|recent| Some(&recent.path) != open.file.as_ref())
            .map(|recent| {
                RecentRow {
                    recent: recent.clone(),
                    key: DiffKey::None,
                }
                .key(recent.path.to_string_lossy().into_owned())
                .into()
            })
            .collect();

        let on_choose = move |_| {
            // `spawn_forever`, not `spawn`: the dialog is asynchronous and, through the
            // xdg portal, not modal to the window, so the reader can raise another tab
            // while it is up -- and that unmounts the scope a `spawn` would belong to,
            // losing the folder they then chose. `proj` is a root state, so the write
            // is good whatever is on screen.
            spawn_forever(async move {
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
                        // The box writes straight into `Proj`, so a keystroke is a state
                        // change the save observer sees and the project file is written
                        // at once.
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
                        // The file the project is kept in, which is its identity and is
                        // written inside neither it nor the session beside it.
                        .child(field_row(
                            "Kept in",
                            label()
                                .text(match &open.file {
                                    Some(file) => file.to_string_lossy().into_owned(),
                                    // A project file is made by the first write that has
                                    // something to put in it.
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
                                // Where its `[profile.*]` is read from, when that is not
                                // the file above: cargo takes profiles from the workspace
                                // root alone, so a member project is offered an edit to a
                                // manifest it does not hold, and it is named rather than
                                // written to behind the reader's back.
                                .maybe_child(held.profiles.as_ref().map(|profiles| {
                                    field_row(
                                        "Profiles",
                                        label()
                                            .text(profiles.to_string_lossy().into_owned())
                                            .color(palette().address_fg)
                                            .max_lines(1),
                                    )
                                    .into_element()
                                }))
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
                            "Language server",
                            Some({
                                let lsp = lsp.clone();
                                let started = spoken.started();
                                Button::new()
                                    // Nothing to run one over is the one state neither
                                    // press has an answer to.
                                    .enabled(directory.is_some())
                                    .on_press(move |_| {
                                        // Asked again at the press, and bound before the
                                        // write: the state may have moved since the
                                        // render, and a guard held over a write panics.
                                        let started = language.peek().started();
                                        match started {
                                            true => stop_server(language, &lsp),
                                            // Which asks first where the reader has not
                                            // agreed to the directory yet.
                                            false => start_server(language, proj, &lsp),
                                        }
                                    })
                                    .child(match started {
                                        true => "Stop",
                                        false => "Start",
                                    })
                                    .into_element()
                            }),
                        ))
                        // Which program, named rather than assumed: a project on a
                        // toolchain of its own is read by a server this app cannot guess,
                        // and a reader who has one needs somewhere to say so. Straight
                        // into `Proj`, so a keystroke is saved like a rename.
                        .child(field_row(
                            "Program",
                            Input::new(proj.into_writable().map(
                                |open| &open.language_server,
                                |open| &mut open.language_server,
                            ))
                            .placeholder(source::Language::Rust.server().unwrap_or_default())
                            .width(Size::fill()),
                        ))
                        // Which of the project's files that server is for. A server
                        // answers about a file whatever language it is -- rust-analyzer
                        // reads a C file as Rust and names things in it the app would
                        // draw as links -- and the app knows the program and not what it
                        // serves, so this is where a project says.
                        .child(field_row(
                            "Files",
                            Input::new(
                                proj.into_writable().map(
                                    |open| &open.language_files,
                                    |open| &mut open.language_files,
                                ),
                            )
                            .placeholder(match given(&open.language_server) {
                                Some(_) => "every file opened",
                                None => source::Language::Rust.spoken(),
                            })
                            .width(Size::fill()),
                        ))
                        // Whether the reader has agreed to a server reading this
                        // directory, and the way back. Agreeing happens where the question
                        // is asked, at the start it holds up; taking it back has nowhere
                        // else to live, and a reader who cannot see the answer they gave
                        // cannot change their mind about it.
                        .maybe_child(directory.is_some().then(|| {
                            field_row(
                                "Directory",
                                rect()
                                    .width(Size::fill())
                                    .horizontal()
                                    .cross_align(Alignment::Center)
                                    .content(Content::Flex)
                                    .spacing(8.0)
                                    .child(
                                        label()
                                            .text(match open.trusted {
                                                true => "Agreed to".to_owned(),
                                                false => "Not agreed to".to_owned(),
                                            })
                                            .width(Size::flex(1.0))
                                            .color(palette().address_fg)
                                            .max_lines(1),
                                    )
                                    .maybe(open.trusted, |row| {
                                        row.child(
                                            Button::new()
                                                .on_press(move |_| {
                                                    revoke_trust(language, proj, &lsp)
                                                })
                                                .child("Take it back"),
                                        )
                                    }),
                            )
                            .into_element()
                        }))
                        .child({
                            let (text, bad) = spoken.status(directory.is_some());
                            info_line_in(
                                text,
                                match bad {
                                    true => palette().invalid_fg,
                                    false => palette().address_fg,
                                },
                            )
                            .into_element()
                        })
                        // What the project's own settings file gave the server, so a
                        // reader can see what theirs is being told; and why it could not
                        // be used, in the colour the failure above is in, since that file
                        // is the one thing that stops a start before it is one. Nothing
                        // at all where a project said nothing, which is most of them.
                        .maybe_child(
                            spoken
                                .unreadable()
                                .map(|why| info_line_in(why, palette().invalid_fg).into_element()),
                        )
                        .maybe_child((!spoken.overrides().is_empty()).then(|| {
                            info_line_in(format!("From {}", lsp::SETTINGS), palette().address_fg)
                                .into_element()
                        }))
                        .children(
                            spoken
                                .overrides()
                                .iter()
                                .map(|(name, value)| override_row(name, value))
                                .collect::<Vec<Element>>(),
                        )
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

//! Which project is open: the pane that says so, the chip in the top bar that opens it,
//! what the reader can say about the project, the window that asks before a delete and the
//! one that says a project would not open, and the other projects they can go to.

use super::*;

/// The project the app is in, as the project view holds it.
///
/// Two of its fields are `String`s where [`Details`] has `Option`s, because this is what is
/// in two text boxes and a text box has no third state: an empty box *is* how a reader says
/// "I have not said". [`OpenProject::details`] is the one place the two spellings meet, and
/// [`OpenProject::workspace`] is the directory box as the path everything else wants.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct OpenProject {
    /// The file the project is kept in, which is its identity. `None` until a project
    /// exists on disk at all.
    pub(crate) file: Option<PathBuf>,
    /// The directory the project is over, as the reader typed it: what is built, walked,
    /// searched, and read with a language server. [`OpenProject::workspace`] is it as a path.
    pub(crate) workspace_text: String,
    /// The language server to read this project with, empty for the usual one. A box like
    /// the one above: a project on a toolchain of its own is the only one that fills it.
    pub(crate) language_server: String,
    /// Which of the project's files that server is for, as extensions with anything
    /// between them: `c h cpp`. Empty for the program's own answer, which is what nearly
    /// every project leaves it at.
    pub(crate) language_files: String,
    /// Whether the reader has agreed to a language server being run over the directory
    /// above. A plain value like the profile below: the prompt has no third answer, and
    /// a project that was never asked is one that has not agreed.
    pub(crate) trusted: bool,
    /// What to build the directory with. A plain value and not an `Option`, since the two
    /// buttons that set it have no third state either; the file is what leaves it out.
    pub(crate) profile: Profile,
}

impl OpenProject {
    /// The project as it was found on disk.
    pub(crate) fn opened(file: PathBuf, project: &Project, trusted: bool) -> OpenProject {
        OpenProject {
            file: Some(file),
            workspace_text: project
                .details
                .directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_default(),
            language_server: project.details.language_server.clone().unwrap_or_default(),
            language_files: project.details.language_files.clone().unwrap_or_default(),
            trusted,
            profile: project.details.cargo.clone().unwrap_or_default().profile,
        }
    }

    /// The project's directory as a path, or `None` when the reader has not named one.
    pub(crate) fn workspace(&self) -> Option<PathBuf> {
        given(&self.workspace_text).map(PathBuf::from)
    }

    /// The program to read this project with: what the reader named, or
    /// [`OpenProject::default_server`].
    pub(crate) fn server(&self) -> String {
        given(&self.language_server)
            .unwrap_or_else(|| OpenProject::default_server())
            .to_owned()
    }

    /// The program a project that names none is read with: the one the language this app
    /// is written for is read with (`languages::Language::server`). Its own function because
    /// the Program box draws it as the placeholder, and two spellings of the default could
    /// come to disagree.
    pub(crate) fn default_server() -> &'static str {
        languages::Language::Rust.server().unwrap_or_default()
    }

    /// Whether the reader named a server of their own rather than leaving the app's. What
    /// the Files box asks: a project that named its own server is asked about whatever it
    /// opens, that being the reader's business.
    pub(crate) fn names_server(&self) -> bool {
        given(&self.language_server).is_some()
    }

    /// What a server started now would be started as: the program, and the extensions
    /// the reader named for it, as they wrote them and in that order, with the dots off.
    /// Anything is a separator: what is wanted is the extensions, and `c, h` and `c h` and
    /// `.c .h` are all somebody saying the same thing.
    pub(crate) fn serving(&self) -> Serving {
        let files = self
            .language_files
            .split(|letter: char| !letter.is_alphanumeric() && letter != '+' && letter != '#')
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect();
        Serving {
            program: self.server(),
            files,
        }
    }

    /// What of this reaches the project file. Trimmed, so a box holding nothing but spaces
    /// is a box holding nothing. `trusted` is not here: the agreement is the session's, so
    /// it reaches the disk through [`Session::from_state`] instead.
    pub(crate) fn details(&self) -> Details {
        Details {
            directory: self.workspace(),
            language_server: given(&self.language_server).map(str::to_owned),
            language_files: given(&self.language_files).map(str::to_owned),
            // Absent while it says nothing the defaults do not: a reader who has never
            // touched the profile leaves no `[cargo]` behind, and choosing the default
            // back takes the section out again.
            cargo: (self.profile != Profile::default()).then(|| Cargo {
                profile: self.profile,
            }),
        }
    }
}

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

keyed!(BinaryRow);

impl Component for BinaryRow {
    fn render(&self) -> impl IntoElement {
        let fitted = use_fitted();
        let text = self.path.to_string_lossy().into_owned();

        cut_tooltip(
            fitted.cut(),
            text.clone(),
            dead_list_row()
                .child(tree_name_fitted(fitted, text, false, &[]))
                .child(dim_line(counted(self.objects, "object", "objects"))),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// One setting the project's own `.vscode/settings.json` gave the language server: the
/// name with `rust-analyzer.` off it, and the value as it will be sent.
///
/// A row and not a `field_row`: these names are long enough to be the whole of the left
/// column and there is no reason for the values to line up with the fields above.
fn override_row(name: &str, value: &str) -> Element {
    dead_list_row()
        .child(
            label()
                .text(name.to_owned())
                .color(palette().text_fg)
                .max_lines(1),
        )
        .child(dim_line(value.to_owned()).width(Size::flex(1.0)))
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

keyed!(ArtifactRow);

impl Component for ArtifactRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
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
                list_row(hovering, Chosen::No)
                    .on_press(move |_| {
                        // The same question a Files row's menu turns on: a path the app
                        // holds already is not opened a second time.
                        if states.holds_path(&path) {
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
                    .child(tree_name_fitted(fitted, text, false, &[]))
                    .child(dim_line(about)),
            ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// The place a diagnostic points at, drawn as a [`PlaceTarget`] where this pane can reach
/// it: pressing it opens that file as source on the line the compiler named, through
/// [`open_source_place`], the arrival every door into a place in a file makes. So the file
/// already open under another spelling is the tab this opens in, and the assembly side is
/// driven from that line.
///
/// The **column** is not carried. cargo counts one in characters and a landing's are bytes
/// along the line, which only the text of the line converts between; this pane has no text,
/// so the caret lands at the start of the line.
///
/// Which of these files may be opened, and the path each opens, is [`Builds::sources`],
/// picked out on the worker beside the build. A file it does not name -- a dependency's,
/// out of the registry, or one the source cache would refuse -- gets no press, which
/// [`PlaceTarget`] draws as the plain line it would have been: a target that did nothing
/// when pressed would be worse than never offering one.
///
/// The set is asked and never the filesystem. Deciding it here cost a `stat` per row per
/// frame, and a build says two hundred things as readily as two.
///
/// The states a press needs are the section's, consumed while it renders and handed down:
/// a hook may only be called while a component renders, and this is called once per
/// diagnostic.
fn source_place(
    doors: Doors,
    ctrl: State<bool>,
    build: &Builds,
    diagnostic: &Diagnostic,
) -> Option<Element> {
    let span = diagnostic.span.as_ref()?;
    // How the place is spelled is the *other* question: cargo spells a file of the
    // workspace relative to its root, a short path drawn whole whether or not the source
    // cache would read it, and anything else absolute, a path cut to its name.
    let text = match Path::new(&span.file).is_relative() {
        true => diagnostic_place(span),
        false => diagnostic_place_by_name(span),
    };
    let target = build.target(span).cloned();
    let line = span.line as u32;

    Some(
        PlaceTarget {
            text,
            press: target.map(|file| {
                EventHandler::new(move |_| {
                    open_source_place(doors, &file, line, None, Reach::outside(ctrl));
                })
            }),
        }
        .into_element(),
    )
}

/// One project in the recent list. Pressing it opens this one in place of the one on
/// screen. Drawn by the Project view and by the screen a window with no project is
/// (`src/ui/no_project.rs`).
#[derive(Clone, PartialEq)]
pub(crate) struct RecentRow {
    pub(crate) recent: Recent,
    pub(crate) key: DiffKey,
}

keyed!(RecentRow);

impl Component for RecentRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let states = use_project_states();
        let path = self.recent.path.clone();
        let recent = &self.recent;

        // What the project is called, which is what its file is called.
        let text = recent.label.clone();
        // What is known about it without opening it, out of its own file.
        let about = match &recent.directory {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => match recent.binaries {
                0 => "empty".to_owned(),
                many => counted(many, "binary", "binaries"),
            },
        };

        extra_tooltip(
            recent.path.to_string_lossy().into_owned(),
            list_row(hovering, Chosen::No)
                .on_press(move |_| switch_project(states, path.clone()))
                .child(one_line(text).width(Size::flex(1.0)))
                .child(dim_line(about)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// The project the app is in: the directory it is over, and the file it is kept in.
#[derive(PartialEq)]
struct IdentitySection;

impl Component for IdentitySection {
    fn render(&self) -> impl IntoElement {
        let mut proj = use_consume::<Proj>().0;
        let file = use_consume::<ProjFile>().0.read().clone();

        let on_choose = move |_| {
            // On a task that outlives this view, which is drawn only while its tab is on
            // screen and the dialog is not modal to the window (`ask_file`).
            ask_file(
                AsyncFileDialog::new().set_title("Choose the project's directory..."),
                AskFor::Folder,
                move |path| proj.write().workspace_text = path.to_string_lossy().into_owned(),
            );
        };

        section("Project", None)
            // The box writes straight into `Proj`, so a keystroke is a state change the
            // save observer sees, and the project file is written once the typing stops.
            .child(field_row(
                "Directory",
                value_row()
                    .child(
                        Input::new(
                            proj.into_writable()
                                .map(|open| &open.workspace_text, |open| &mut open.workspace_text),
                        )
                        .placeholder("None")
                        .compact()
                        .width(Size::flex(1.0)),
                    )
                    .child(Button::new().on_press(on_choose).child("Choose...")),
            ))
            // The file the project is kept in, which is its identity and is written
            // inside neither it nor the session beside it.
            .child(field_row(
                "Kept in",
                dim_line(match &file {
                    Some(file) => file.to_string_lossy().into_owned(),
                    // A project file is made by the first write that has something
                    // to put in it.
                    None => "not saved yet".to_owned(),
                }),
            ))
    }
}

/// The binaries the project holds, out of `Objects`, which the saved list is derived
/// from: what is drawn is what the next write will say.
#[derive(PartialEq)]
struct BinariesSection;

impl Component for BinariesSection {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;

        let binaries: Vec<Element> = {
            let objects = objects.read();
            project::binary_counts(&objects)
                .into_iter()
                .map(|(path, count)| {
                    let key = path.to_string_lossy().into_owned();
                    BinaryRow {
                        key: DiffKey::None,
                        objects: count,
                        path,
                    }
                    .key(key)
                    .into()
                })
                .collect()
        };

        section("Binaries", None).child(rows_or(binaries, "Nothing open"))
    }
}

/// What a diagnostic's place is pressed to reach.
///
/// **Consumed where the page renders and carried to the section as data.** Reaching for
/// a context is a hook, and the section renders again on every word the build worker
/// says, for a press that comes when the reader goes to the file.
///
/// The handles are the root's and are never replaced, so this **compares equal always**:
/// the section holding one is not re-rendered for it.
#[derive(Clone, Copy)]
struct PlaceStates {
    doors: Doors,
    /// Whether Ctrl is held, which is whether the file opens in a tab of its own.
    ctrl: State<bool>,
}

impl PartialEq for PlaceStates {
    fn eq(&self, _: &PlaceStates) -> bool {
        true
    }
}

/// Building the project's own workspace: what cargo is run over, with what, and what it
/// said. The manifest is read here: it is this section's own question.
#[derive(PartialEq)]
struct CargoSection {
    /// What a diagnostic's press reaches for, consumed once by the page: a handler may
    /// not run a hook ([`PlaceStates`]).
    places: PlaceStates,
}

impl Component for CargoSection {
    fn render(&self) -> impl IntoElement {
        let mut proj = use_consume::<Proj>().0;
        let build = use_consume::<Building>().0;
        let jobs = use_consume::<BuildJobs>();
        let PlaceStates { doors, ctrl } = self.places;
        // The two fields this draws, each a memo: `Proj` is written by every keystroke in
        // the boxes above and below ([`ProjFile`]).
        let directory = use_consume::<Workspace>().0.read().clone();
        let profile = use_memo(move || proj.read().profile)();

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
                jobs.send(BuildJob {
                    directory,
                    profile: *profile,
                    what: BuildWhat::Read,
                });
            }
        });

        // What there is to build, and what to build it with. After the hooks, so no guard
        // is held while one runs; the read subscribes this component to the build, so a
        // finished one redraws the rows below.
        let held = build.read();
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
                let place = source_place(doors, ctrl, &held, diagnostic);
                diagnostic_block(diagnostic, place)
            })
            .collect();

        section(
            "Cargo build",
            directory.clone().map(|directory| {
                let jobs = jobs.clone();
                HeadingButton {
                    icon: ("hammer", lucide::hammer()),
                    text: match held.building {
                        true => cargo::BUILDING,
                        false => "Build",
                    },
                    // Two builds cannot go at once: the second would compile what the
                    // first is writing.
                    live: held.manifest.path.is_some() && !held.building,
                    press: EventHandler::new(move |_| {
                        start_build(build, &jobs, directory.clone(), profile)
                    }),
                }
                .into_element()
            }),
        )
        .child(match &held.manifest.path {
            None => info_line(match directory.is_some() {
                true => "No Cargo.toml in the directory".to_owned(),
                false => "No directory".to_owned(),
            })
            .into_element(),
            Some(manifest) => {
                rect()
                    .width(Size::fill())
                    .spacing(SECTION_GAP)
                    // The file cargo is run over, named rather than implied: what is
                    // built is a question the directory alone answers only for a reader
                    // who knows the rule.
                    .child(field_row(
                        "Manifest",
                        dim_line(manifest.to_string_lossy().into_owned()),
                    ))
                    // Where its `[profile.*]` is read from, when that is not the file
                    // above: cargo takes profiles from the workspace root alone, so a
                    // member project is offered an edit to a manifest it does not hold,
                    // and it is named rather than written to behind the reader's back.
                    .maybe_child(held.manifest.profiles.as_ref().map(|profiles| {
                        field_row(
                            "Profiles",
                            dim_line(profiles.to_string_lossy().into_owned()),
                        )
                        .into_element()
                    }))
                    .child(field_row(
                        "Profile",
                        choice(
                            &[(Profile::Debug, "Debug"), (Profile::Release, "Release")],
                            profile,
                            // Straight into `Proj`, so the save observer sees it like
                            // a keystroke in a box and `project.toml` follows.
                            move |chosen| proj.write().profile = chosen,
                        ),
                    ))
                    // What a binary with no line information costs is the whole source
                    // side, so the offer is made where the profile is chosen and goes as
                    // soon as it is taken.
                    .maybe_child((!held.manifest.debug_lines).then(|| {
                        let jobs = jobs.clone();
                        let directory = directory.clone();
                        let row = field_row(
                            "Debug lines",
                            value_row()
                                .child(
                                    label()
                                        .text("Off, so there is no source side")
                                        .width(Size::flex(1.0))
                                        .color(palette().address_fg),
                                )
                                .child(
                                    Button::new()
                                        .on_press(move |_| {
                                            let Some(directory) = directory.clone() else {
                                                return;
                                            };
                                            jobs.send(BuildJob {
                                                directory,
                                                profile,
                                                what: BuildWhat::AddDebugLines,
                                            });
                                        })
                                        .child("Turn on"),
                                ),
                        );
                        rect()
                            .width(Size::fill())
                            .child(row)
                            // Why the last press did nothing. The row above is unchanged
                            // whether the write worked or not, so without this the reader
                            // is refused in silence.
                            .maybe_child(held.manifest.edit_refused.as_ref().map(|why| {
                                verdict_line(Verdict::bad_news(why.clone())).into_element()
                            }))
                            .into_element()
                    }))
                    .maybe_child(
                        held.verdict()
                            .map(|verdict| verdict_line(verdict).into_element()),
                    )
                    .children(artifacts)
                    // cargo's own words, for what it says nowhere else.
                    .maybe_child(held.refusal().map(text_block))
                    // Drawn straight into the pane's own scroll: a wrapping block has no
                    // height a virtual list could use, and this whole view scrolls
                    // already.
                    .children(diagnostics)
                    .into_element()
            }
        })
    }
}

/// The language server: which program reads the project, which of its files, whether the
/// reader has agreed to it, and how the last start went. This section only says how it
/// went; the control that starts and stops one is in the top bar
/// (`src/ui/language_view.rs`).
#[derive(PartialEq)]
struct LanguageSection;

impl Component for LanguageSection {
    fn render(&self) -> impl IntoElement {
        let proj = use_consume::<Proj>().0;
        let language = use_consume::<Talking>().0;
        let lsp = use_consume::<LspJobs>();
        let spoken = language.read();
        let open = proj.read();
        let directory = open.workspace();

        section(
            "Language server",
            Some({
                let lsp = lsp.clone();
                let started = spoken.started();
                HeadingButton {
                    icon: match started {
                        true => ("square", lucide::square()),
                        false => ("play", lucide::play()),
                    },
                    text: match started {
                        true => "Stop",
                        false => "Start",
                    },
                    // Nothing to run one over is the one state neither press has an
                    // answer to.
                    live: directory.is_some(),
                    // The toggle the top bar's control and the window's chord press,
                    // which is what asks the state again at the press and puts the
                    // question first where the reader has not agreed to the directory
                    // yet. The `started` above is the caption and nothing else.
                    press: EventHandler::new(move |_| toggle_server(language, proj, &lsp)),
                }
                .into_element()
            }),
        )
        // Which program, named rather than assumed: a project on a toolchain of its
        // own is read by a server this app cannot guess, and a reader who has one
        // needs somewhere to say so. Straight into `Proj`, so a keystroke is saved
        // like a rename.
        .child(field_row(
            "Program",
            Input::new(proj.into_writable().map(
                |open| &open.language_server,
                |open| &mut open.language_server,
            ))
            .placeholder(OpenProject::default_server())
            .width(Size::fill()),
        ))
        // Which of the project's files that server is for. A server answers about a
        // file whatever language it is -- rust-analyzer reads a C file as Rust and
        // names things in it the app would draw as links -- and the app knows the
        // program and not what it serves, so this is where a project says.
        .child(field_row(
            "Files",
            Input::new(
                proj.into_writable()
                    .map(|open| &open.language_files, |open| &mut open.language_files),
            )
            .placeholder(match open.names_server() {
                true => "every file opened",
                false => languages::Language::Rust.spoken(),
            })
            .width(Size::fill()),
        ))
        // Whether the reader has agreed to a server reading this directory, and the
        // way back. Agreeing happens where the question is asked, at the start it
        // holds up; taking it back has nowhere else to live, and a reader who cannot
        // see the answer they gave cannot change their mind about it.
        .maybe_child(directory.is_some().then(|| {
            field_row(
                "Directory",
                wide_row()
                    .child(
                        dim_line(match open.trusted {
                            true => "Agreed to".to_owned(),
                            false => "Not agreed to".to_owned(),
                        })
                        .width(Size::flex(1.0)),
                    )
                    .maybe(open.trusted, |row| {
                        row.child(
                            Button::new()
                                .on_press(move |_| revoke_trust(language, proj, &lsp))
                                .child("Take it back"),
                        )
                    }),
            )
            .into_element()
        }))
        .child(verdict_line(spoken.verdict(directory.as_deref())).into_element())
        // What the project's own settings file gave the server, so a reader can see
        // what theirs is being told; and why it could not be used, in the colour the
        // failure above is in, since that file is the one thing that stops a start
        // before it is one. Nothing at all where a project said nothing, which is
        // most of them.
        .maybe_child(
            spoken
                .unreadable()
                .map(|why| verdict_line(Verdict::bad_news(why)).into_element()),
        )
        .maybe_child((!spoken.overrides().is_empty()).then(|| {
            verdict_line(Verdict::plain(format!("From {}", lsp::SETTINGS))).into_element()
        }))
        .children(
            spoken
                .overrides()
                .iter()
                .map(|(name, value)| override_row(name, value))
                .collect::<Vec<Element>>(),
        )
    }
}

/// The other projects the reader can go to, and the way to a new one. The list leaves out
/// the open project, which the sections above already describe.
#[derive(PartialEq)]
struct RecentsSection;

impl Component for RecentsSection {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let file = use_consume::<ProjFile>().0.read().clone();
        let recents = use_consume::<Recents>().0;
        let others: Vec<Element> = recents
            .read()
            .iter()
            .filter(|recent| Some(&recent.path) != file.as_ref())
            .map(|recent| {
                RecentRow {
                    recent: recent.clone(),
                    key: DiffKey::None,
                }
                .key(recent.path.to_string_lossy().into_owned())
                .into()
            })
            .collect();

        section(
            "Recent projects",
            Some(
                HeadingButton {
                    icon: ("plus", lucide::plus()),
                    text: "New project",
                    live: true,
                    press: EventHandler::new(move |_| new_project(states)),
                }
                .into_element(),
            ),
        )
        .child(rows_or(others, "No other projects"))
    }
}

/// The Project pane: the project the app is in, what the reader can say about it, and the
/// other projects they can go to.
///
/// Five sections in a column, each a component reading the contexts it draws from, so a
/// section that did not read what changed is not redrawn: an answer from the language
/// server redraws that one alone, and a binary opening redraws the list of them alone.
/// The reader types into this pane, so the difference is one they see.
#[derive(PartialEq)]
pub(crate) struct ProjectTab;

impl Component for ProjectTab {
    fn render(&self) -> impl IntoElement {
        // What a diagnostic's place is pressed to reach, consumed here and handed down:
        // the section below it renders again on every word the build worker says, and a
        // hook may only be called while a component renders ([`PlaceStates`]).
        let places = PlaceStates {
            doors: use_doors(),
            ctrl: use_consume::<Ctrl>().0,
        };
        page(
            None,
            page_column()
                .child(IdentitySection)
                .child(BinariesSection)
                .child(CargoSection { places })
                .child(LanguageSection)
                .child(RecentsSection),
        )
        .into_element()
    }
}

/// What one of the project's buttons in the bar does.
///
/// An enum and not a handler, for [`TabClose`]'s reason: a `Component` is `PartialEq` and a
/// closure is not, so a button holding one would re-render on every render of the bar.
#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// Let the project go, leaving the app with none. It is left where it is.
    Close,
    /// Put it in a file, which is what an unsaved project has instead of a close.
    Save,
    /// Take it away. Asks first.
    Delete,
}

/// One of them, drawn the way the bar's other controls are.
#[derive(Clone, Copy, PartialEq)]
struct ChipButton {
    doing: Doing,
}

impl Component for ChipButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let states = use_project_states();
        let mut deleting = use_consume::<Deleting>().0;
        let doing = self.doing;

        let (tooltip, icon) = match doing {
            Doing::Close => ("Close the project", ("x", lucide::x())),
            Doing::Save => ("Save the project to a file", ("save", lucide::save())),
            Doing::Delete => ("Delete the project", ("trash-2", lucide::trash_2())),
        };

        extra_tooltip(
            tooltip.to_owned(),
            bar_button(hovering, true, Glow::No)
                .on_press(move |_| match doing {
                    Doing::Close => close_project(states),
                    Doing::Save => ask_where_to_save(states, project::Put::Move),
                    Doing::Delete => {
                        let store = states.store.peek().clone();
                        let file = states.proj.peek().file.clone();
                        let name = file
                            .zip(store)
                            .map(|(file, store)| project::label(&store, &file));
                        deleting.set(name);
                    }
                })
                .child(glyph(icon)),
        )
    }
}

/// The open project, in the top bar beside the menu: what it is called, and what can be
/// done with it.
///
/// **A close button, or Save and Delete.** A project the reader gave a place needs only to
/// be let go of; one the app is keeping has nowhere to be let go *to*, so the two things it
/// can have done to it are named outright rather than hidden behind a x that would mean one
/// of them. Delete asks first ([`DeleteProjectPopup`]); Save does not, having nothing to
/// undo, and it is a move rather than a copy -- there is no second project afterwards.
///
/// Pressing the name shows the Project view, which is where the rest of it is. Hovering it
/// says where the project is kept, which is the one thing the name leaves out.
#[derive(PartialEq)]
pub(crate) struct ProjectChip;

impl Component for ProjectChip {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let open = use_open();
        // Read and not peeked: the bar follows the project being saved, closed or opened.
        let file = use_consume::<ProjFile>().0.read().clone();
        let store = use_consume::<Storage>().0.peek().clone();
        // No store is a run that opens no project.
        let (Some(file), Some(store)) = (file, store) else {
            return rect().into_element();
        };
        let unsaved = project::unsaved(&store, &file);

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(2.0)
            .child(extra_tooltip(
                file.to_string_lossy().into_owned(),
                bar_pill(hovering, true, Glow::No)
                    .on_press(move |_| show_page(open, Page::Project))
                    .child(
                        label()
                            .text(chars::elide(
                                &project::label(&store, &file),
                                CHIP_NAME_CHARS,
                            ))
                            .max_lines(1),
                    ),
            ))
            .maybe(!unsaved, |chip| {
                chip.child(ChipButton {
                    doing: Doing::Close,
                })
            })
            .maybe(unsaved, |chip| {
                chip.child(ChipButton { doing: Doing::Save })
                    .child(ChipButton {
                        doing: Doing::Delete,
                    })
            })
            .into_element()
    }
}

/// The window that says a project would not open. Drawn as nothing at all until there is
/// something to say, the way `RescuedPopup` is.
#[derive(PartialEq)]
pub(crate) struct UnopenedPopup;

impl Component for UnopenedPopup {
    fn render(&self) -> impl IntoElement {
        let mut unopened = use_consume::<Unopened>().0;
        let naming = unopened.read().clone();

        notice(move |_| unopened.set(None)).map(naming, |popup, failure| {
            popup
                .child(
                    notice_body()
                        .child(label().text("That project would not open".to_owned()))
                        // What went wrong, in the reason's own words. A paragraph and not
                        // a label: a parser's message is as long as it is.
                        .child(
                            paragraph()
                                .color(palette().address_fg)
                                .span(failure.reason.to_string()),
                        )
                        // Said only where there is a file to have left alone. The app
                        // never moves a project of the reader's aside, and a window about
                        // a file that will not parse is the one place that is worth
                        // saying.
                        .maybe_child(
                            (failure.reason != project::Reason::Missing).then(|| {
                                notice_line("It has been left exactly as it is.".to_owned())
                            }),
                        )
                        .child(notice_path(failure.path.to_string_lossy().into_owned())),
                )
                .child(
                    PopupButtons::new().child(
                        Button::new()
                            .filled()
                            .on_press(move |_| unopened.set(None))
                            .child("Close"),
                    ),
                )
        })
    }
}

/// The window the app asks before deleting a project.
///
/// Nothing is deleted until it is answered: the control in the bar sets [`Deleting`] and
/// this is what acts. `on_close_request` is the "no" for free -- Escape and a press outside
/// -- and an empty `Popup` draws nothing at all, so with nothing to ask this lays out as
/// nothing.
#[derive(PartialEq)]
pub(crate) struct DeleteProjectPopup;

impl Component for DeleteProjectPopup {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut deleting = use_consume::<Deleting>().0;
        let asking = deleting.read().clone();

        notice(move |_| deleting.set(None)).map(asking, |popup, name| {
            popup
                .child(
                    notice_body()
                        .child(label().text(format!("Delete {name}?")))
                        .child(notice_line(
                            "It is saved nowhere else: its binaries and its bookmarks go \
                             with it."
                                .to_owned(),
                        )),
                )
                .child(
                    PopupButtons::new()
                        .child(
                            Button::new()
                                .on_press(move |_| deleting.set(None))
                                .child("Cancel"),
                        )
                        .child(
                            Button::new()
                                .filled()
                                .on_press(move |_| {
                                    deleting.set(None);
                                    delete_project(states);
                                })
                                .child("Delete"),
                        ),
                )
        })
    }
}

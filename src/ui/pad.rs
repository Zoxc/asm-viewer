//! The scratchpad itself: the model the app holds, the jobs its worker does, and what a
//! build, a run and a stop each mean. The drawing of it is the file beside this one.
//!
//! **One worker thread and not three.** Reading a scratchpad back, writing the package and
//! `cargo build` are all blocking, but the point is not only that the UI thread stays free:
//! the directory has a single writer, so a save cannot land inside the build that is reading
//! what it writes. **Saves supersede and builds never do** -- a keystroke is a save, so the
//! loop drains its queue while what it holds is one; a build is what the reader asked for and
//! its answer is the point.
//!
//! **Running does not sit on that worker and stopping does not go near it.** A run has no
//! bound on how long it takes, so queued like a build it would freeze every save behind it
//! and the reader could not edit their way out; a stop queued behind a build would arrive
//! after the thing it was meant to interrupt. And [`RunState`] has four states because
//! `Starting` is the one a `bool` loses: a fork is fast but not instant, and a Stop pressed
//! inside that window has to be remembered.

use super::*;

/// The scratchpad the app has open, and what its worker is doing about it.
///
/// A root context and not state inside the view, for the reason [`Prefs`] and [`Proj`]
/// are: the Scratchpad pane is a dockable tab, and a dock tab that is not the active one
/// in its panel is *unmounted*. A buffer the reader is typing into cannot live somewhere
/// that a click on the tab beside it throws away.
#[derive(Clone, Copy)]
pub(crate) struct Pad(pub(crate) State<PadState>);

/// The scratchpad's source, as `freya-code-editor` holds it: a rope, a cursor, an undo
/// history and the tree-sitter blocks the rows are drawn from.
///
/// Beside [`Pad`] rather than inside it, and it is the editor's copy that is the live
/// one: `Scratchpad::source` is a `String` the model writes out, while this is what the
/// keyboard edits, so one of the two has to follow the other and it is the model that
/// follows. `use_scratchpad_with`'s first effect is the whole of that mirroring.
///
/// Also a root context, for [`Pad`]'s reason and one more: the theme effect below has to
/// reach it whether or not the pane is on screen, since a `SyntaxBlocks` holds resolved
/// colours and nothing a re-render does would repaint them (see [`HIGHLIGHTED`]).
#[derive(Clone, Copy)]
pub(crate) struct PadText(pub(crate) State<CodeEditorData>);

/// The way to ask the scratchpad's worker for something, shared through context so that a
/// button in the pane can ask without the pane owning the thread.
///
/// Two senders and not one, because they carry traffic of two different shapes. `jobs` is
/// what the reader asked for, one message per press. `events` is what a *running program*
/// is saying, which is as many messages a second as it cares to write -- so it is
/// [`RUN_EVENTS`]-bounded where the other is unbounded, and that bound is the app's half
/// of the backpressure `scratchpad.rs` documents: a full channel blocks the thread reading
/// the pipe, which fills the pipe, which blocks the program.
#[derive(Clone)]
pub(crate) struct PadJobs {
    jobs: async_channel::Sender<PadJob>,
    events: async_channel::Sender<(u64, RunEvent)>,
}

/// How many of a running program's lines may sit between the pipe and the pane.
///
/// Big enough that an ordinary burst is never throttled and small enough that the queue is
/// not somewhere output can pile up unnoticed. It is a *bound* and not a buffer size: the
/// point is that there is a number here at all.
const RUN_EVENTS: usize = 512;

/// Everything the Scratchpad pane draws.
#[derive(Clone, Default)]
pub(crate) struct PadState {
    pub(crate) scratchpad: Scratchpad,
    /// Whether the worker has yet said what is on disk.
    ///
    /// `Saves::written`'s rule, in a second place and for the same reason: the app boots
    /// holding [`Scratchpad::default`] and the reader's own source arrives a thread
    /// later, so a save that ran before that answer landed would write the default source
    /// over a good scratchpad. Nothing is saved until this is true.
    pub(crate) opened: bool,
    /// Whether a build is running. It is what disables the Build button, which is the
    /// whole of "two builds cannot be started at once": one worker thread runs the jobs
    /// in order anyway, but a second job queued behind the first would build bytes the
    /// reader has since changed and answer for them afterwards.
    pub(crate) building: bool,
    /// What the last build of this run came back with, or `None` before there has been
    /// one. A build is not remembered across runs: it describes bytes on disk that the
    /// next `cargo build` will replace.
    pub(crate) built: Option<Build>,
    /// Why the package on disk is not what is on screen, or `None` when it is.
    ///
    /// [`Scratchpad::write`] refuses outright rather than generating a manifest that
    /// differs from the rows -- which is the model's rule and a good one -- so a bad row
    /// stops the *source* being written too, and the pane has to say so where the reader
    /// is looking. It is one sentence over the rows, which each say their own half.
    pub(crate) unsaved: Option<Failure>,
    /// Which run the arriving output belongs to, counted up by [`request_run`].
    ///
    /// **A number, where `use_analysis` was at pains not to have one** -- and the
    /// difference is worth stating, since the rule there is that superseding is a
    /// comparison and never a counter. It could compare because an answer carries the
    /// `Symbol` it is about and that symbol existed *before* the request. Here the thing an
    /// event is about is the process, and the process does not exist until the worker has
    /// forked -- by which time the first lines can already be on their way. There is
    /// nothing yet to compare against, so the run is numbered instead. It matters for a
    /// gesture that is one keypress long: stopping a program and starting another leaves
    /// the first one's last lines and its `Ended` still in flight, and untagged they would
    /// land in the new run's output and mark it finished.
    run: u64,
    pub(crate) run_state: RunState,
    /// What the running program has written. Behind an `Arc` because this struct is cloned
    /// on every render and on every answer the worker sends, and the deque under it holds
    /// thousands of lines: the clone is a refcount bump, and appending is one
    /// `Arc::make_mut` per *batch* of arrivals rather than one per line.
    pub(crate) output: Arc<RunOutput>,
}

/// Where the program the reader started has got to.
///
/// Four states and not a `bool`, because three of them draw differently and the fourth --
/// [`RunState::Starting`] -- is the one a `bool` would get wrong: a fork is fast but it is
/// not instant, and a Stop pressed in that window has to be remembered rather than
/// dropped. `Idle` is not "not running", it is *nothing has been run*, which is why the
/// output pane is absent rather than empty before the first press.
#[derive(Clone, Default)]
pub(crate) enum RunState {
    #[default]
    Idle,
    /// Asked for; the worker has not come back with a handle yet.
    Starting,
    Going(Running),
    Over(Ended),
}

impl PadState {
    /// What the compiler said about the last build. Warnings on a build that succeeded
    /// and errors on one that did not are the same list to a reader.
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        match &self.built {
            Some(Build::Built { diagnostics, .. }) => diagnostics,
            Some(Build::Rejected { diagnostics, .. }) => diagnostics,
            Some(Build::Unavailable(_)) | None => &[],
        }
    }

    /// cargo's own words, when they are about the dependency rows.
    ///
    /// **This is the whole of how a failed build points back at a row**, and it is a
    /// structural test rather than a search for a crate name in a sentence. A rejected
    /// build with no compiler diagnostics at all is cargo refusing *before* it compiled
    /// anything, and the only part of the generated package a reader can get wrong from
    /// this pane is `[dependencies]` -- so `no matching package named ... found`, which
    /// `analysis`' own note says is stated on stderr and nowhere else, is drawn under the
    /// rows it is about instead of in the diagnostics list. Once the compiler has spoken
    /// the same stderr is only `could not compile ... due to 1 previous error`, which
    /// says nothing the list below does not, so it is dropped.
    pub(crate) fn refusal(&self) -> Option<&str> {
        match &self.built {
            Some(Build::Rejected {
                diagnostics,
                message,
            }) if diagnostics.is_empty() && !message.is_empty() => Some(message),
            _ => None,
        }
    }

    /// The one line over the pane saying where the last build got to, and whether that
    /// line is bad news.
    pub(crate) fn status(&self) -> Option<(String, bool)> {
        if self.building {
            return Some(("Building...".to_owned(), false));
        }

        let count = |level: Level, one: &str, many: &str| {
            let count = self
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.level == level)
                .count();
            match count {
                0 => String::new(),
                1 => format!(": 1 {one}"),
                count => format!(": {count} {many}"),
            }
        };

        match self.built.as_ref()? {
            Build::Built { .. } => Some((
                format!("Built{}", count(Level::Warning, "warning", "warnings")),
                false,
            )),
            Build::Rejected { .. } => Some((
                format!("Not built{}", count(Level::Error, "error", "errors")),
                true,
            )),
            // Nothing was compiled, and the reason is a sentence written to be shown as
            // it stands -- a bad row, no cargo on the `PATH`, nowhere to keep a
            // scratchpad.
            Build::Unavailable(failure) => Some((failure.to_string(), true)),
        }
    }

    /// What the last build made, and so what there is to run.
    ///
    /// The path cargo *named*, carried through from the build rather than derived here --
    /// which is the same argument `scratchpad.rs` makes for asking cargo in the first
    /// place, and the reason the Run button is unavailable until something has been built:
    /// what runs is then, by construction, what the diagnostics on screen are about.
    pub(crate) fn executable(&self) -> Option<&Path> {
        match &self.built {
            Some(Build::Built { executable, .. }) => Some(executable),
            _ => None,
        }
    }

    /// Whether a program is on its way up or already going.
    pub(crate) fn is_running(&self) -> bool {
        matches!(self.run_state, RunState::Starting | RunState::Going(_))
    }

    /// The line over the output, saying where the run got to, and whether that is bad
    /// news. `None` before anything has been run, which is what leaves the pane out.
    pub(crate) fn run_status(&self) -> Option<(String, bool)> {
        let dropped = match self.output.dropped() {
            0 => String::new(),
            1 => " (1 earlier line dropped)".to_owned(),
            count => format!(" ({count} earlier lines dropped)"),
        };

        let (text, bad) = match &self.run_state {
            RunState::Idle => return None,
            RunState::Starting => ("Starting...".to_owned(), false),
            RunState::Going(_) => ("Running".to_owned(), false),
            RunState::Over(Ended::Exited(Some(0))) => ("Exited".to_owned(), false),
            RunState::Over(Ended::Exited(Some(code))) => (format!("Exited with {code}"), true),
            // A signal on Unix. Spelt as what is *known* rather than as a guess at which,
            // since the number is not portable and the app has no use for it.
            RunState::Over(Ended::Exited(None)) => ("Ended with no exit code".to_owned(), true),
            RunState::Over(Ended::Stopped) => ("Stopped".to_owned(), false),
            RunState::Over(Ended::Failed(error)) => (format!("Could not run it: {error}"), true),
        };

        Some((format!("{text}{dropped}"), bad))
    }
}

/// What the scratchpad's worker thread is asked for. Each carries the whole scratchpad
/// rather than a handle to one, so nothing the worker touches can change under it while
/// it is writing or building.
pub(crate) enum PadJob {
    Open(Scratchpad),
    Save(Scratchpad),
    Build(Scratchpad),
    /// Start what the last build made. The odd one out: it is not blocking work, and what
    /// it hands back is a handle rather than an answer. It goes to the worker all the same
    /// because it *forks*, and the thread that draws has no business doing that -- and
    /// because the scratchpad's directory, which becomes the program's working directory,
    /// is this thread's to hand out.
    Run {
        /// Which run this is, carried so that a handle arriving after the reader has moved
        /// on can be recognised and stopped rather than stored. See [`PadState::run`].
        run: u64,
        scratchpad: Scratchpad,
        executable: PathBuf,
        /// Where each line goes as it is written. A boxed callback rather than a channel,
        /// so `scratchpad.rs` never learns what the app carries its values in.
        emit: Box<dyn FnMut(RunEvent) + Send>,
    },
}

/// What it answers with.
pub(crate) enum PadAnswer {
    Opened(Scratchpad),
    /// Why the package could not be written, or `None` when it was.
    Saved(Option<Failure>),
    Built(Build),
    /// The handle to a started program, or why there is none. Everything the program then
    /// *says* arrives on the other channel, not here: this is the answer to "did it
    /// start", and the run itself has no answer, only an end.
    Started(u64, Result<Running, Failure>),
}

/// The work itself: the three blocking calls `scratchpad.rs` documents as never belonging
/// on a UI thread, and nothing else. Split out so [`use_scratchpad_with`] can be handed
/// something that answers without a disk or a compiler -- `use_analysis_with`'s shape and
/// for its reason.
fn pad_work(job: PadJob) -> PadAnswer {
    match job {
        PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad.opened()),
        PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.write().err()),
        PadJob::Build(scratchpad) => PadAnswer::Built(scratchpad.build()),
        PadJob::Run {
            run,
            scratchpad,
            executable,
            emit,
        } => PadAnswer::Started(run, scratchpad.run(&executable, emit)),
    }
}

/// The scratchpad's whole wiring: one worker thread, the editor's text mirrored into the
/// model, the model written out as it changes, and the theme carried into the editor's
/// own syntax blocks.
///
/// **One worker thread, and it is the only thing that ever touches the scratchpad's
/// directory.** Reading it back, writing the package and running `cargo build` are all
/// documented in `scratchpad.rs` as blocking, and a `cargo build` is seconds; putting
/// them on one thread rather than one each is not only about the UI thread staying free
/// but about the directory having a single writer, so a save cannot land in the middle of
/// the build that is reading what it writes.
///
/// **Saves supersede, builds never do.** A keystroke is a save and a reader types
/// faster than a package is written, so the loop drains its queue while what it is
/// holding is a save: only the newest says anything, and a build that has arrived behind
/// one writes the package itself on its way past. A build is what the reader *asked* for
/// and its answer is the point, so it is never dropped.
///
/// **A run does not sit on that thread**, which is the one thing 10d added to the shape.
/// `PadJob::Run` only starts the program and comes straight back; the program itself lives
/// on two threads of `scratchpad.rs`'s and reports on a second channel. It has to be that
/// way round for a reason the other three jobs do not have: a run has no bound on how long
/// it takes -- an accidental `loop {}` is the ordinary case in a buffer somebody is
/// experimenting in -- and a run queued like a build would freeze every save behind it, so
/// the reader could not even edit their way out of it. **Stopping does not go through the
/// worker either**, for the same reason turned around: a stop queued behind a build would
/// arrive after the thing it was meant to interrupt.
pub(crate) fn use_scratchpad(
    pad: State<PadState>,
    text: State<CodeEditorData>,
    states: ProjectStates,
) {
    use_scratchpad_with(pad, text, states, pad_work);
}

/// [`use_scratchpad`] with the work handed in, so a test can drive the wiring without
/// writing to the machine's own state directory or waiting on a compiler.
pub(crate) fn use_scratchpad_with(
    mut pad: State<PadState>,
    mut text: State<CodeEditorData>,
    states: ProjectStates,
    work: impl Fn(PadJob) -> PadAnswer + Send + 'static,
) -> PadJobs {
    // What the worker was last handed, which is what the disk therefore says. The
    // baseline `Saves::written` is, and it starts empty for the reason that one does: the
    // app boots holding [`Scratchpad::default`], and a baseline seeded from it would make
    // the reader's own scratchpad -- which arrives a thread later -- look like a change to
    // be written back. It is *seeded by the answer* instead, so a run in which nothing is
    // typed writes nothing at all and a scratchpad nobody has opened leaves no directory
    // behind, which is `project.rs`'s rule about a file made by the first write that has
    // something to say.
    //
    // An `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let sent = use_hook(|| Rc::new(RefCell::new(None::<Scratchpad>)));

    let requests = use_hook({
        let sent = sent.clone();
        move || {
            let (requests, jobs) = async_channel::unbounded::<PadJob>();
            let (answered, answers) = async_channel::unbounded::<PadAnswer>();
            // One channel for the app's lifetime rather than one per run, which is what
            // makes the run number on each event necessary and is also what makes it
            // enough: a stopped run's last lines have somewhere to go, and are recognised
            // and dropped when they get there.
            let (emitted, events) = async_channel::bounded::<(u64, RunEvent)>(RUN_EVENTS);

            std::thread::spawn(move || {
                while let Ok(job) = jobs.recv_blocking() {
                    let mut job = job;
                    // Superseded saves, dropped before they are started. Whatever is behind
                    // one is either a newer save or a build, and a build writes the package
                    // itself -- so nothing is lost by not writing this one.
                    while matches!(job, PadJob::Save(_)) {
                        match jobs.try_recv() {
                            Ok(newer) => job = newer,
                            Err(_) => break,
                        }
                    }

                    // A send that fails is the app shutting down and taking the receiver
                    // with it.
                    if answered.send_blocking(work(job)).is_err() {
                        return;
                    }
                }
            });

            spawn(async move {
                while let Ok(answer) = answers.recv().await {
                    match answer {
                        PadAnswer::Opened(scratchpad) => {
                            // The buffer is replaced rather than edited into place: this is
                            // the first thing that happens to it, so there is no cursor and
                            // no undo history to preserve, and `CodeEditorData` has no way to
                            // set its text that would keep either honest anyway.
                            //
                            // `palette()` is asked here on the UI thread -- freya's `spawn`
                            // runs its tasks there -- so this is the same thread-local every
                            // component reads, and reading it outside a reactive scope simply
                            // subscribes nothing.
                            let mut editor = CodeEditorData::new(
                                Rope::from_str(&scratchpad.source),
                                language(Path::new(SOURCE_FILE)),
                            );
                            editor.set_theme(palette().syntax());
                            // Without this the editor has no blocks at all and draws no
                            // lines: `CodeEditorData::new` configures the highlighter and
                            // never runs it.
                            editor.parse();
                            text.set(editor);

                            // The baseline, seeded by the answer rather than at mount: what
                            // is on disk is by definition what was last written, so a run in
                            // which nothing is typed asks for no save at all.
                            *sent.borrow_mut() = Some(scratchpad.clone());

                            let mut next = pad.peek().clone();
                            next.scratchpad = scratchpad;
                            next.opened = true;
                            pad.set(next);
                        }
                        PadAnswer::Saved(failure) => {
                            let mut next = pad.peek().clone();
                            next.unsaved = failure;
                            pad.set(next);
                        }
                        PadAnswer::Built(build) => {
                            let executable = match &build {
                                Build::Built { executable, .. } => Some(executable.clone()),
                                _ => None,
                            };

                            let mut next = pad.peek().clone();
                            next.building = false;
                            next.built = Some(build);
                            // A build writes the package on its way, so the reason the last
                            // save could not is answered by it too.
                            if !matches!(
                                next.built,
                                Some(Build::Unavailable(Failure::Dependencies(_)))
                            ) {
                                next.unsaved = None;
                            }
                            pad.set(next);

                            if let Some(executable) = executable {
                                reopen_binary(states, executable);
                            }
                        }
                        PadAnswer::Started(run, started) => {
                            let mut next = pad.peek().clone();
                            // A handle for a run the reader has already left -- they
                            // pressed Stop or Run again inside the fork. It is stopped
                            // here and nowhere else, because this is the first moment
                            // anything in the app is holding it: dropping it instead would
                            // leave a process running that nothing could ever name again.
                            let mine =
                                next.run == run && matches!(next.run_state, RunState::Starting);
                            match started {
                                Ok(running) if mine => next.run_state = RunState::Going(running),
                                Ok(running) => running.stop(),
                                Err(failure) if mine => {
                                    next.run_state =
                                        RunState::Over(Ended::Failed(failure.to_string()))
                                }
                                Err(_) => {}
                            }
                            pad.set(next);
                        }
                    }
                }
            });

            // What a running program is saying. A task of its own beside the answers,
            // since the two channels are answering different questions and a program that
            // never ends would otherwise be sharing a loop with every save.
            spawn(async move {
                while let Ok(first) = events.recv().await {
                    // Everything else already queued, taken in one go. A program printing
                    // in a tight loop would otherwise wake this task per line, and each
                    // wake is a state write and so a render: coalescing makes the cost one
                    // render per batch, which is the same "drain the queue" the analysis
                    // worker does for the same reason.
                    let mut batch = vec![first];
                    while let Ok(more) = events.try_recv() {
                        batch.push(more);
                    }

                    let mut next = pad.peek().clone();
                    let mut changed = false;
                    for (run, event) in batch {
                        // A run the reader has left. Its lines are not this run's output
                        // and its ending is not this run's ending.
                        if run != next.run {
                            continue;
                        }
                        changed = true;
                        match event {
                            RunEvent::Wrote(line) => Arc::make_mut(&mut next.output).push(line),
                            RunEvent::Ended(ended) => next.run_state = RunState::Over(ended),
                        }
                    }

                    if changed {
                        pad.set(next);
                    }
                }
            });

            PadJobs {
                jobs: requests,
                events: emitted,
            }
        }
    });

    // How the pane asks for a build. A context rather than an argument, because the
    // button that asks is inside a dockable view that is handed nothing, and returned as
    // well so that a test can ask without going through a button.
    let jobs = use_provide_context(|| requests.clone());

    // What is on disk, asked for once. `use_hook` runs on mount and never again, which is
    // what makes this the app's one reading of the scratchpad.
    use_hook({
        let requests = requests.clone();
        move || {
            let _ = requests
                .jobs
                .try_send(PadJob::Open(pad.peek().scratchpad.clone()));
        }
    });

    // The editor's text into the model. Reading the editor subscribes this to every edit;
    // a cursor move wakes it too, and the comparison is what makes that free.
    use_side_effect(move || {
        let typed = text.read().rope.to_string();

        let changed = pad.peek().scratchpad.source != typed;
        if changed {
            pad.write().scratchpad.source = typed;
        }
    });

    // The model onto the disk. Nothing is written while the two are the same, and the
    // baseline moves to what was last *sent*: a reader who changes a row and changes it
    // back has to write again, or the file would be left holding the middle answer.
    use_side_effect(move || {
        let state = pad.read().clone();
        if !state.opened {
            return;
        }

        let mut sent = sent.borrow_mut();
        if sent.as_ref() != Some(&state.scratchpad) {
            *sent = Some(state.scratchpad.clone());
            let _ = requests.jobs.try_send(PadJob::Save(state.scratchpad));
        }
    });

    // The theme, carried into the editor's own blocks. This is `HIGHLIGHTED`'s hazard in
    // a second place: a `SyntaxBlocks` holds colours already resolved out of the palette,
    // so the entries are not stale after a switch, they are the wrong theme -- and
    // `set_appearance`'s clear cannot reach inside a `CodeEditorData`. Re-setting the
    // theme rebuilds the highlighter's capture colours and `parse` re-colours every line.
    //
    // Reading the appearance here subscribes the root, which already reads it twice.
    use_side_effect_with_deps(&appearance(), move |_: &Appearance| {
        let mut editor = text.write();
        editor.set_theme(palette().syntax());
        editor.parse();
    });

    jobs
}

/// Ask for a build of what is on screen, unless one is already running.
///
/// The guard is here as well as on the button, so that "two builds cannot be started at
/// once" is a property of the request rather than of one control's disabled state.
pub(crate) fn request_build(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    if state.building {
        return;
    }

    // **A rebuild stops what the last one started.** Three reasons and each is sufficient:
    // cargo is about to write over the very file this process is running, which on some
    // systems is refused outright and on the rest silently makes the running program a
    // different program from the one on screen; `reopen_binary` is about to close the
    // objects that describe those bytes, so the listing the reader would go back to is
    // gone; and there is one Run button for one scratchpad, so a build that left a program
    // going would leave the reader with an output pane belonging to a build they can no
    // longer see. Editing stops nothing, deliberately -- a run is of an executable and not
    // of the buffer, and a keystroke that killed the reader's program would make it
    // impossible to take a note about what it printed.
    stop_run(pad);

    pad.write().building = true;
    let _ = jobs.jobs.try_send(PadJob::Build(state.scratchpad));
}

/// Run what the last build made.
///
/// Nothing happens without an executable, which is why the button is unavailable until a
/// build has succeeded: the alternative -- Run building first -- makes one press mean two
/// things, and puts a page of diagnostics on screen in answer to a request to run.
///
/// Whatever was running is stopped first. One scratchpad, one program: two generations of
/// output arriving into one list is a pane with no answer to "what is this", and the
/// second run's own first line would sit under the first run's last.
pub(crate) fn request_run(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    let Some(executable) = state.executable().map(Path::to_path_buf) else {
        return;
    };

    stop_run(pad);

    // The output starts empty and the run is numbered: everything still on its way from
    // the run before this one is now for a number nobody is listening to.
    let run = state.run + 1;
    let mut next = pad.peek().clone();
    next.run = run;
    next.run_state = RunState::Starting;
    next.output = Arc::new(RunOutput::default());
    pad.set(next);

    let events = jobs.events.clone();
    let _ = jobs.jobs.try_send(PadJob::Run {
        run,
        scratchpad: state.scratchpad,
        executable,
        // `send_blocking` and not `try_send`: a full channel has to *stop* the thread
        // reading the pipe, which is what puts the brakes on the program itself. Dropping
        // the line instead would be an output with silent holes in it.
        emit: Box::new(move |event| {
            let _ = events.send_blocking((run, event));
        }),
    });
}

/// Stop the program, for real.
///
/// The `Going` case is the whole of it: `Running::stop` kills the process, and the state
/// is *not* set to `Over` here -- the run's own `Ended` event is what says it, and it is
/// emitted only once the process has been reaped. So the pane says "Stopped" when the
/// program is actually gone rather than when the button was pressed.
///
/// `Starting` is the case a `bool` would have lost: the fork has been asked for and has
/// not come back, so there is nothing to kill yet. Leaving `Starting` behind is what makes
/// the handle unwanted when it arrives, and the answer handler stops it there.
pub(crate) fn stop_run(mut pad: State<PadState>) {
    let state = pad.peek().clone();
    match &state.run_state {
        RunState::Going(running) => running.stop(),
        RunState::Starting => {
            let mut next = state;
            next.run_state = RunState::Over(Ended::Stopped);
            pad.set(next);
        }
        RunState::Idle | RunState::Over(_) => {}
    }
}

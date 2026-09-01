//! The scratchpad itself: the model the app holds, the jobs its worker does, and what a
//! build, a run and a stop each mean. The drawing of it is the file beside this one.
//!
//! **One worker thread**, so the scratchpad's directory has a single writer and a save
//! cannot land inside the build that is reading what it writes. **Saves supersede and
//! builds never do.** **A run does not sit on that worker and a stop does not go near
//! it**: a run has no bound on how long it takes, and a stop queued behind a build would
//! arrive after the thing it was meant to interrupt.

use super::*;

/// The scratchpad the app has open, and what its worker is doing about it. A root
/// context, since a dock tab that is not the active one in its panel is unmounted and a
/// buffer the reader is typing into cannot live there.
#[derive(Clone, Copy)]
pub(crate) struct Pad(pub(crate) State<PadState>);

/// The scratchpad's source, as `freya-code-editor` holds it. The editor's copy is the
/// live one and `Scratchpad::source` follows it; [`use_scratchpad_with`]'s first effect is
/// that mirroring. A root context for [`Pad`]'s reason, and because the theme effect has
/// to reach it whether or not the pane is on screen.
#[derive(Clone, Copy)]
pub(crate) struct PadText(pub(crate) State<CodeEditorData>);

/// The way to ask the scratchpad's worker for something, shared through context.
///
/// Two senders: `jobs` is one message per press and unbounded; `events` is what a running
/// program is saying and is bounded, which is the app's half of the backpressure -- a full
/// channel blocks the pipe thread, which blocks the program.
#[derive(Clone)]
pub(crate) struct PadJobs {
    jobs: async_channel::Sender<PadJob>,
    events: async_channel::Sender<(u64, RunEvent)>,
}

/// Everything the Scratchpad pane draws.
#[derive(Clone, Default)]
pub(crate) struct PadState {
    pub(crate) scratchpad: Scratchpad,
    /// Whether the worker has yet said what is on disk. **Nothing is saved until this is
    /// true**: the app boots holding [`Scratchpad::default`] and the reader's own source
    /// arrives a thread later, so a save before then writes the default over a kept
    /// scratchpad.
    pub(crate) opened: bool,
    /// Whether a build is running, which is the whole of "two builds cannot be started at
    /// once": a second job queued behind the first would build bytes the reader has since
    /// changed.
    pub(crate) building: bool,
    /// What the last build came back with. Not remembered across runs: it describes bytes
    /// the next `cargo build` will replace.
    pub(crate) built: Option<Build>,
    /// Why the package on disk is not what is on screen, or `None` when it is.
    /// [`Scratchpad::write_to`] refuses outright for a bad row, so a bad row stops the
    /// *source* being written too and the pane has to say so.
    pub(crate) unsaved: Option<Failure>,
    /// Which run the arriving output belongs to, counted up by [`request_run`].
    ///
    /// **Events carry a run number** where `use_analysis` compares identities instead: the
    /// thing an event is about is the process, which does not exist until the worker has
    /// forked, so there is nothing to compare against. Stopping one program and starting
    /// another is one keypress, and untagged the first one's last lines and its `Ended`
    /// would land in the second's output.
    run: u64,
    pub(crate) run_state: RunState,
    /// What the running program has written. Behind an `Arc` because this struct is cloned
    /// on every render and the deque under it holds thousands of lines.
    pub(crate) output: Arc<RunOutput>,
}

/// Where the program the reader started has got to.
///
/// Four states and not a `bool`, because [`RunState::Starting`] is the one a `bool` would
/// get wrong: a fork is not instant, and a Stop pressed in that window has to be
/// remembered rather than dropped.
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

    /// cargo's own words, when they are about the dependency rows: a rejected build with
    /// no compiler diagnostics at all is cargo refusing before it compiled anything, and
    /// `[dependencies]` is the only part of the generated package this pane can get wrong.
    /// Once the compiler has spoken the same stderr says nothing the list below does not.
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
            Build::Unavailable(failure) => Some((failure.to_string(), true)),
        }
    }

    /// What the last build made, and so what there is to run: the path cargo *named*,
    /// carried through from the build rather than derived here.
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
            // A signal on Unix; the number is not portable and the app has no use for it.
            RunState::Over(Ended::Exited(None)) => ("Ended with no exit code".to_owned(), true),
            RunState::Over(Ended::Stopped) => ("Stopped".to_owned(), false),
            RunState::Over(Ended::Failed(error)) => (format!("Could not run it: {error}"), true),
        };

        Some((format!("{text}{dropped}"), bad))
    }
}

/// What the scratchpad's worker thread is asked for. Each carries the whole scratchpad, so
/// nothing the worker touches can change under it while it is writing or building.
pub(crate) enum PadJob {
    Open(Scratchpad),
    Save(Scratchpad),
    Build(Scratchpad),
    /// Start what the last build made. It goes to the worker because it *forks* and
    /// because the directory it hands the program is that thread's, not because it blocks.
    Run {
        /// Which run this is, so a handle arriving after the reader has moved on can be
        /// recognised and stopped rather than stored. See [`PadState::run`].
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
    /// The handle to a started program, or why there is none. What the program then *says*
    /// arrives on the other channel.
    Started(u64, Result<Running, Failure>),
}

/// The blocking work itself. Split out so [`use_scratchpad_with`] can be handed something
/// that answers without a disk or a compiler. Each arm resolves the scratchpad's own
/// directory first: without one there is nowhere to read, write, build or run in.
pub(crate) fn pad_work(job: PadJob) -> PadAnswer {
    match job {
        PadJob::Open(scratchpad) => PadAnswer::Opened(match scratchpad.directory() {
            Some(directory) => scratchpad.opened_in(&directory),
            // Nowhere to have been read from, so what was handed in is what there is.
            None => scratchpad,
        }),
        PadJob::Save(scratchpad) => PadAnswer::Saved(match scratchpad.directory() {
            Some(directory) => scratchpad.write_to(&directory).err(),
            None => Some(Failure::NoDirectory),
        }),
        PadJob::Build(scratchpad) => PadAnswer::Built(match scratchpad.directory() {
            Some(directory) => scratchpad.build_in(&directory),
            None => Build::Unavailable(Failure::NoDirectory),
        }),
        PadJob::Run {
            run,
            scratchpad,
            executable,
            emit,
        } => PadAnswer::Started(
            run,
            match scratchpad.directory() {
                Some(directory) => run_in(&executable, &directory, emit),
                None => Err(Failure::NoDirectory),
            },
        ),
    }
}

/// The scratchpad's whole wiring: one worker thread, the editor's text mirrored into the
/// model, the model written out as it changes, and the theme carried into the editor's own
/// syntax blocks. See this file's header for why the worker is one thread, why saves
/// supersede, and why a run and a stop do not go through it.
///
/// The work is handed in, so a test can drive the wiring without writing to the machine's
/// own state directory or waiting on a compiler.
pub(crate) fn use_scratchpad_with(
    mut pad: State<PadState>,
    mut text: State<CodeEditorData>,
    states: ProjectStates,
    work: impl Fn(PadJob) -> PadAnswer + Send + 'static,
) -> PadJobs {
    // What the worker was last handed, which is what the disk therefore says. It starts
    // empty and is *seeded by the answer*, never at mount: the app boots holding
    // [`Scratchpad::default`], and a baseline seeded from that would make the reader's own
    // scratchpad look like a change to be written back over it.
    //
    // An `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let sent = use_hook(|| Rc::new(RefCell::new(None::<Scratchpad>)));

    let requests = use_hook({
        let sent = sent.clone();
        move || {
            let (requests, jobs) = async_channel::unbounded::<PadJob>();
            let (answered, answers) = async_channel::unbounded::<PadAnswer>();
            // One channel for the app's lifetime rather than one per run, which is what
            // makes the run number on each event both necessary and enough.
            // 512: how many of a running program's lines may sit between the pipe and
            // the pane.
            let (emitted, events) = async_channel::bounded::<(u64, RunEvent)>(512);

            std::thread::spawn(move || {
                while let Ok(job) = jobs.recv_blocking() {
                    let mut job = job;
                    // Superseded saves, dropped before they are started. Whatever is behind
                    // one is a newer save or a build, and a build writes the package itself.
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
                            // no undo history to preserve.
                            let mut editor = CodeEditorData::new(
                                Rope::from_str(&scratchpad.source),
                                language(Path::new(SOURCE_FILE)),
                            );
                            editor.set_theme(palette().syntax());
                            // Without this the editor has no blocks at all and draws no
                            // lines: `CodeEditorData::new` never runs the highlighter.
                            editor.parse();
                            text.set(editor);

                            // The baseline, seeded by the answer rather than at mount.
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
                            // A build writes the package on its way.
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
                            // A handle for a run the reader has already left. Stopped here
                            // and nowhere else, this being the first moment anything in the
                            // app is holding it: dropping it would leave a process running
                            // that nothing could ever name again.
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

            // What a running program is saying. A task of its own, since a program that
            // never ends would otherwise share a loop with every save.
            spawn(async move {
                while let Ok(first) = events.recv().await {
                    // Everything else already queued, taken in one go: each wake is a state
                    // write and so a render, so coalescing makes it one render per batch.
                    let mut batch = vec![first];
                    while let Ok(more) = events.try_recv() {
                        batch.push(more);
                    }

                    let mut next = pad.peek().clone();
                    let mut changed = false;
                    for (run, event) in batch {
                        // A run the reader has left: not this run's output, not its ending.
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

    // How the pane asks for a build. A context, because the button that asks is inside a
    // dockable view that is handed nothing; returned as well, so a test can ask directly.
    let jobs = use_provide_context(|| requests.clone());

    // What is on disk, asked for once: `use_hook` runs on mount and never again.
    use_hook({
        let requests = requests.clone();
        move || {
            let _ = requests
                .jobs
                .try_send(PadJob::Open(pad.peek().scratchpad.clone()));
        }
    });

    // The editor's text into the model. Reading the editor subscribes this to every edit;
    // the comparison is what makes a bare cursor move free.
    use_side_effect(move || {
        let typed = text.read().rope.to_string();

        let changed = pad.peek().scratchpad.source != typed;
        if changed {
            pad.write().scratchpad.source = typed;
        }
    });

    // The model onto the disk, with the baseline moving to what was last *sent*: a reader
    // who changes a row and changes it back has to write again.
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

    // The theme, carried into the editor's own blocks: a `SyntaxBlocks` holds colours
    // already resolved out of the palette, so after a switch the entries are the wrong
    // theme rather than stale, and `set_appearance`'s clear cannot reach inside a
    // `CodeEditorData`. Re-setting the theme and parsing again is what repaints them.
    use_side_effect_with_deps(&appearance(), move |_: &Appearance| {
        let mut editor = text.write();
        editor.set_theme(palette().syntax());
        editor.parse();
    });

    jobs
}

/// Ask for a build of what is on screen, unless one is already running. The guard is here
/// as well as on the button, so "two builds cannot be started at once" is a property of
/// the request rather than of one control's disabled state.
pub(crate) fn request_build(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    if state.building {
        return;
    }

    // A rebuild stops what the last one started: cargo is about to write over the very file
    // that process is running, and `reopen_binary` is about to close the objects that
    // describe those bytes. Editing stops nothing, deliberately.
    stop_run(pad);

    pad.write().building = true;
    let _ = jobs.jobs.try_send(PadJob::Build(state.scratchpad));
}

/// Run what the last build made. Nothing happens without an executable, which is why the
/// button is unavailable until a build has succeeded. Whatever was running is stopped
/// first: two generations of output arriving into one list is a pane with no answer to
/// "what is this".
pub(crate) fn request_run(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    let Some(executable) = state.executable().map(Path::to_path_buf) else {
        return;
    };

    stop_run(pad);

    // The output starts empty and the run is numbered, so everything still on its way from
    // the run before this one is for a number nobody is listening to.
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
        // reading the pipe, which is what puts the brakes on the program itself.
        emit: Box::new(move |event| {
            let _ = events.send_blocking((run, event));
        }),
    });
}

/// Stop the program, for real.
///
/// The state is *not* set to `Over` in the `Going` case: the run's own `Ended` event says
/// it, and that is emitted only once the process has been reaped, so the pane says
/// "Stopped" when the program is gone rather than when the button was pressed.
/// `Starting` is the case a `bool` would have lost -- the fork has not come back, so
/// leaving `Starting` behind is what makes the handle unwanted when it arrives.
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

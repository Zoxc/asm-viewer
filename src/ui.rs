//! The freya UI.
//!
//! The module is a directory of its own, and every one of its files is a **cut out of one
//! 8 700-line file** rather than a boundary designed from scratch: what each holds is
//! exactly what the section banners and the cross-references in `AGENTS.md` already said
//! belonged together, and nothing changed on the way across.
//!
//! Two mechanical decisions come with that and are worth stating once, here, rather than
//! being rediscovered in each file.
//!
//! **The imports below are the module's own prelude.** They are `pub(crate) use` and every
//! file under this one begins `use super::*;`, so each keeps the exact set of names it had
//! while it was a section of one file. The alternative -- a tailored import block per file
//! -- is tidier and buys nothing here: these files are the halves of one UI and they use
//! one another's types freely, so the tailored blocks would be eighteen copies of most of
//! this one.
//!
//! **Each file is re-exported straight back out.** A `mod x;` is followed by a
//! `pub(crate) use x::*;`, so a name means what it meant before the split wherever it is
//! written, `src/ui/tests.rs`'s `use super::*` included. The globs cannot collide, by
//! construction: every one of these names already shared a single namespace.
//!
//! **What is `pub(crate)` is what the compiler asked for and no more.** The blanket
//! alternative was a `pub(crate)` on all four hundred-odd items, fields and methods, which
//! is one line of script and a worse answer: with the visibility minimal, the annotations
//! *are* the list of what crosses a file boundary, and a struct whose fields are still
//! private is a struct nothing outside its file takes apart. It is `pub(crate)` and never
//! `pub` because dead-code analysis still runs on a `pub(crate)` item, so nothing here can
//! quietly stop being used and stay compiled.

pub(crate) use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};

pub(crate) use async_io::Timer;
pub(crate) use freya::code_editor::{
    CodeEditor, CodeEditorData, EditorLanguage, EditorSyntaxTheme, EditorThemePartialExt, Rope,
    SyntaxBlocks, SyntaxHighlighter, TextNode,
};
pub(crate) use freya::icons::lucide;
pub(crate) use freya::prelude::*;
pub(crate) use rfd::AsyncFileDialog;

pub(crate) use analysis::{
    open_files_streaming, Assembly, Instruction, LineInfo, Object, Progress, SpanKind, Symbol,
    SymbolData,
};

pub(crate) use crate::docs::{DocId, Docs};
pub(crate) use crate::filter::{Filter, Matcher};
pub(crate) use crate::fonts::{self, Font, Fonts};
pub(crate) use crate::history::History;
pub(crate) use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
pub(crate) use crate::project::{
    self, Details, Document, Project, ProjectId, Recent, Selection, Session,
};
pub(crate) use crate::rows::RowSelection;
pub(crate) use crate::scratchpad::{
    Build, Dependency, Diagnostic, Ended, Failure, Half, Level, Problem, RunEvent, RunOutput,
    Running, Scratchpad, Stream,
};
pub(crate) use crate::settings::{Appearance, FontSetting, Settings, Theme as ThemeChoice};
pub(crate) use crate::source::{self, SourceFile};
pub(crate) use crate::tabs::{self, Positions};
pub(crate) use crate::tree::{
    format_tag, Expansion, LoadId, Loads, ObjectTree, TreeRow, ARCHIVE_TAG,
};

mod analyzed;
pub(crate) use analyzed::*;
mod assembly;
pub(crate) use assembly::*;
mod dock;
pub(crate) use dock::*;
mod documents;
pub(crate) use documents::*;
mod filter_bar;
pub(crate) use filter_bar::*;
mod focus;
pub(crate) use focus::*;
mod highlight;
pub(crate) use highlight::*;
mod marks;
pub(crate) use marks::*;
mod metrics;
pub(crate) use metrics::*;
mod palette;
pub(crate) use palette::*;
mod parts;
pub(crate) use parts::*;
mod project_view;
pub(crate) use project_view::*;
mod settings_view;
/// Named again explicitly because `freya::prelude` exports a `use_theme` too, and two globs
/// offering one name is an ambiguity at every call site rather than a shadowing. An explicit
/// import wins over a glob, so this line is what `use_theme` means under `ui` -- ours. Do not
/// tidy it away as a duplicate of the glob below it; it is the disambiguation, and the `allow`
/// is because it disambiguates for `tests.rs` alone, nothing else naming it through `ui`.
#[allow(unused_imports)]
pub(crate) use settings_view::use_theme;
pub(crate) use settings_view::*;
mod sidebar;
pub(crate) use sidebar::*;
mod source_view;
pub(crate) use source_view::*;
mod state;
pub(crate) use state::*;

// ---------------------------------------------------------------------------
// Scratchpad
// ---------------------------------------------------------------------------

/// The scratchpad the app has open, and what its worker is doing about it.
///
/// A root context and not state inside the view, for the reason [`Prefs`] and [`Proj`]
/// are: the Scratchpad pane is a dockable tab, and a dock tab that is not the active one
/// in its panel is *unmounted*. A buffer the reader is typing into cannot live somewhere
/// that a click on the tab beside it throws away.
#[derive(Clone, Copy)]
struct Pad(State<PadState>);

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
struct PadText(State<CodeEditorData>);

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
struct PadJobs {
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
struct PadState {
    scratchpad: Scratchpad,
    /// Whether the worker has yet said what is on disk.
    ///
    /// `Saves::written`'s rule, in a second place and for the same reason: the app boots
    /// holding [`Scratchpad::default`] and the reader's own source arrives a thread
    /// later, so a save that ran before that answer landed would write the default source
    /// over a good scratchpad. Nothing is saved until this is true.
    opened: bool,
    /// Whether a build is running. It is what disables the Build button, which is the
    /// whole of "two builds cannot be started at once": one worker thread runs the jobs
    /// in order anyway, but a second job queued behind the first would build bytes the
    /// reader has since changed and answer for them afterwards.
    building: bool,
    /// What the last build of this run came back with, or `None` before there has been
    /// one. A build is not remembered across runs: it describes bytes on disk that the
    /// next `cargo build` will replace.
    built: Option<Build>,
    /// Why the package on disk is not what is on screen, or `None` when it is.
    ///
    /// [`Scratchpad::write`] refuses outright rather than generating a manifest that
    /// differs from the rows -- which is the model's rule and a good one -- so a bad row
    /// stops the *source* being written too, and the pane has to say so where the reader
    /// is looking. It is one sentence over the rows, which each say their own half.
    unsaved: Option<Failure>,
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
    run_state: RunState,
    /// What the running program has written. Behind an `Arc` because this struct is cloned
    /// on every render and on every answer the worker sends, and the deque under it holds
    /// thousands of lines: the clone is a refcount bump, and appending is one
    /// `Arc::make_mut` per *batch* of arrivals rather than one per line.
    output: Arc<RunOutput>,
}

/// Where the program the reader started has got to.
///
/// Four states and not a `bool`, because three of them draw differently and the fourth --
/// [`RunState::Starting`] -- is the one a `bool` would get wrong: a fork is fast but it is
/// not instant, and a Stop pressed in that window has to be remembered rather than
/// dropped. `Idle` is not "not running", it is *nothing has been run*, which is why the
/// output pane is absent rather than empty before the first press.
#[derive(Clone, Default)]
enum RunState {
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
    fn diagnostics(&self) -> &[Diagnostic] {
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
    fn refusal(&self) -> Option<&str> {
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
    fn status(&self) -> Option<(String, bool)> {
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
    fn executable(&self) -> Option<&Path> {
        match &self.built {
            Some(Build::Built { executable, .. }) => Some(executable),
            _ => None,
        }
    }

    /// Whether a program is on its way up or already going.
    fn is_running(&self) -> bool {
        matches!(self.run_state, RunState::Starting | RunState::Going(_))
    }

    /// The line over the output, saying where the run got to, and whether that is bad
    /// news. `None` before anything has been run, which is what leaves the pane out.
    fn run_status(&self) -> Option<(String, bool)> {
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
enum PadJob {
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
enum PadAnswer {
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
fn use_scratchpad(pad: State<PadState>, text: State<CodeEditorData>, states: ProjectStates) {
    use_scratchpad_with(pad, text, states, pad_work);
}

/// [`use_scratchpad`] with the work handed in, so a test can drive the wiring without
/// writing to the machine's own state directory or waiting on a compiler.
fn use_scratchpad_with(
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
fn request_build(mut pad: State<PadState>, jobs: &PadJobs) {
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
fn request_run(mut pad: State<PadState>, jobs: &PadJobs) {
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
fn stop_run(mut pad: State<PadState>) {
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

/// The file a scratchpad's source is, as cargo and rustc spell it: what `language` is
/// asked about, and what a diagnostic's span names when it is about the reader's own
/// source rather than a crate they depend on.
const SOURCE_FILE: &str = "src/main.rs";

/// How much of a dependency row the crate name takes against the version beside it. A
/// name is a word and a requirement is a handful of characters, and both boxes have to
/// shrink together in a 300px sidebar.
const NAME_FLEX: f32 = 2.0;
const VERSION_FLEX: f32 = 1.0;

/// What the compiler's own word for a level is drawn in.
///
/// The palette has one red and one warm hue, and this is what they are for here: an error
/// is the red every invalid thing in the app is written in, a warning is the terracotta a
/// string literal is (the one warm colour in the set, and the only other thing that is
/// meant to catch the eye), and a note recedes into the colour everything secondary is
/// written in.
fn level_color(level: Level) -> Color {
    match level {
        Level::Error => palette().invalid_fg,
        Level::Warning => palette().string_fg,
        Level::Note => palette().address_fg,
    }
}

fn level_text(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Note => "note",
    }
}

/// A block of a tool's own output, laid out the way it wrote it: one label per line, in
/// the fixed-width font, so rustc's carets sit under what they point at.
///
/// One label per line rather than one holding the newlines, for the reason every list in
/// this app builds rows: a paragraph that wraps would put a caret under the wrong
/// character, and the whole point of a rendered diagnostic is the column it points at.
fn text_block(text: &str, color: Color) -> Element {
    rect()
        .width(Size::fill())
        .overflow(Overflow::Clip)
        .children(
            text.lines()
                .map(|line| {
                    label()
                        .text(line.to_owned())
                        .assembly_font()
                        .color(color)
                        .max_lines(1)
                        .into()
                })
                .collect::<Vec<Element>>(),
        )
        .into_element()
}

/// One thing the compiler said: a line that can be scanned, and cargo's own rendering of
/// it under that.
///
/// The header repeats the message the block below it opens with, which is deliberate and
/// is what every problems list does: the header is what a reader runs their eye down and
/// the block is what they stop to read. What the header adds is the **place**, taken from
/// the span rather than from the text -- `src/main.rs:3:5` for the reader's own source and
/// the file's name alone for a diagnostic out of a crate they depend on, which is a
/// distinction only the span can make.
fn diagnostic_block(diagnostic: &Diagnostic) -> Element {
    let place = diagnostic.span.as_ref().map(|span| {
        let file = match span.file == SOURCE_FILE {
            true => span.file.clone(),
            // A registry path is most of a line on its own, and which crate it is in is
            // the useful half of it.
            false => file_name(&span.file),
        };

        format!("{file}:{}:{}", span.line, span.column)
    });

    rect()
        .width(Size::fill())
        .padding(Gaps::new(2.0, 0.0, 6.0, 0.0))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .content(Content::Flex)
                .child(
                    label()
                        .text(level_text(diagnostic.level))
                        .color(level_color(diagnostic.level))
                        .max_lines(1),
                )
                .maybe_child(place.map(|place| {
                    label()
                        .text(place)
                        .color(palette().address_fg)
                        .max_lines(1)
                        .into_element()
                }))
                .child(
                    label()
                        .text(diagnostic.message.clone())
                        .width(Size::flex(1.0))
                        .max_lines(1),
                ),
        )
        .child(text_block(&diagnostic.rendered, palette().text_fg))
        .into_element()
}

/// One `[dependencies]` row: the crate, the version required of it, and the × that drops
/// it.
///
/// The problem is a prop rather than something worked out here, because it is a property
/// of the *list* -- `Problem::Repeated` is about two rows -- and `Scratchpad::problems`
/// answers for all of them at once so that every bad row can be marked rather than the
/// first one.
#[derive(Clone, PartialEq)]
struct DependencyRow {
    index: usize,
    dependency: Dependency,
    problem: Option<Problem>,
    key: DiffKey,
}

impl KeyExt for DependencyRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for DependencyRow {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let index = self.index;
        let problem = self.problem.clone();
        // Which box is wrong is the model's answer and not this pane's guess at one:
        // `Repeated` is about the name, and nothing in its wording says so.
        let half = problem.as_ref().map(Problem::half);

        // The two boxes write straight into the row they are drawn from, so a keystroke
        // is a state change the save effect sees like any other -- the project view's
        // name box, one level deeper. Indexing is safe because a row is mounted only for
        // an index the list has: the × below shortens the list, and the rows are rebuilt
        // from the shorter one before either box is read again.
        let name = pad.into_writable().map(
            move |pad: &PadState| &pad.scratchpad.dependencies[index].name,
            move |pad: &mut PadState| &mut pad.scratchpad.dependencies[index].name,
        );
        let version = pad.into_writable().map(
            move |pad: &PadState| &pad.scratchpad.dependencies[index].version,
            move |pad: &mut PadState| &mut pad.scratchpad.dependencies[index].version,
        );

        let marked = |input: Input, box_half: Half| {
            input.maybe(half == Some(box_half), |input: Input| {
                input
                    .color(palette().invalid_fg)
                    .focus_border_fill(palette().invalid_fg)
            })
        };

        rect()
            .width(Size::fill())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height() + 8.0))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .content(Content::Flex)
                    .spacing(6.0)
                    .child(marked(
                        Input::new(name)
                            .placeholder("crate")
                            .compact()
                            .width(Size::flex(NAME_FLEX)),
                        Half::Name,
                    ))
                    .child(marked(
                        Input::new(version)
                            .placeholder("version")
                            .compact()
                            .width(Size::flex(VERSION_FLEX)),
                        Half::Version,
                    ))
                    .child(
                        Button::new()
                            .compact()
                            .on_press(move |_| {
                                pad.write().scratchpad.dependencies.remove(index);
                            })
                            .child("\u{00d7}"),
                    ),
            )
            // Against the row it belongs to and never as one message at the top, which is
            // what `Scratchpad::problems` answering with every row's index is for.
            .maybe_child(problem.map(|problem| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 0.0, 4.0, 2.0))
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .text(problem.to_string())
                            .color(palette().invalid_fg)
                            .max_lines(1),
                    )
            }))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The scratchpad's source, in freya's own `CodeEditor`.
///
/// **5a rejected this component for the read-only source pane and 10c takes it, which is
/// not a reversal**: both of 5a's reasons were about painting and scrolling a listing
/// from outside, and neither survives the pane being one the reader is typing in.
/// `editor_line.rs` paints a line background for exactly one line, the cursor's own --
/// which is wrong for the source pane, where a set of lines maps to an instruction, and
/// exactly right here, where the only current line *is* the caret's. Its
/// `ScrollController` is built in a `use_hook` of its own and `CodeEditorData::scrolls` is
/// `pub(crate)` -- which stopped 5c from scrolling the pane to a line, and there is
/// nothing here that wants to. What is left is a real editor: a cursor, a selection, an
/// undo history, the clipboard, IME preedit, and an *incremental* tree-sitter re-parse per
/// keystroke through the same pipeline the source pane already borrows. Hand-rolling that
/// would be several hundred lines of text editing to end up with less.
///
/// Two things are still ours, and both are the reason the app looks like one app: the
/// colours come from the palette rather than from `EditorTheme::light()`, and the font is
/// the desktop's fixed-width one.
#[derive(PartialEq)]
struct SourceEditor;

impl Component for SourceEditor {
    fn render(&self) -> impl IntoElement {
        let text = use_consume::<PadText>().0;
        let a11y_id = use_hook(AccessibilityId::new_unique);

        let font = fonts();
        let size = font.mono.size();
        // The editor takes **one** family where everything else in the app takes a chain,
        // and freya appends the parent element's families behind an element's own -- so
        // the rest of the chain arrives by inheritance from the box around it, which is
        // what keeps a desktop naming a font that is not installed from silently landing
        // the listing in a proportional face.
        let family = font
            .mono
            .families
            .first()
            .map(|family| family.to_string())
            .unwrap_or_default();
        // The editor multiplies its font size by this and floors the answer, and what is
        // wanted is `code_row_height()` exactly -- so half a pixel of slack is what makes the
        // product land on it rather than one below it.
        let line_height = (code_row_height() + 0.5) / size;

        rect()
            .expanded()
            .background(palette().pane_bg)
            .assembly_font()
            .child(
                CodeEditor::new(text, a11y_id)
                    .font_size(size)
                    .font_family(family)
                    .line_height(line_height)
                    // The source pane draws indentation as plain spaces, and two panes of
                    // code in one window disagreeing about that would read as two editors.
                    .show_whitespace(false)
                    .background(palette().pane_bg)
                    .text(palette().name_fg)
                    .cursor(palette().text_fg)
                    // What would land on the clipboard, which is what `row_select_bg`
                    // already says in both code panes -- a character selection here where
                    // it is a run of rows there, and the same question either way.
                    .highlight(palette().row_select_bg)
                    // "You are here", which is `code_row_hover_bg`'s job in the other two
                    // panes. Reusing it rather than adding a ninth wash to the palette is
                    // safe because the editor paints no pointer hover at all, so the two
                    // meanings can never be on screen together in this pane.
                    .line_selected_background(palette().code_row_hover_bg)
                    .gutter_selected(palette().text_fg)
                    .gutter_unselected(palette().address_fg)
                    .whitespace(palette().punctuation_fg),
            )
    }
}

/// The lines a running program has written, as the row builder is handed them.
///
/// A wrapper for the identity: `PartialEq` here is `Arc::ptr_eq`, the app's rule
/// everywhere, and it is load-bearing rather than an optimisation -- deriving it would
/// compare thousands of strings on every render of a pane that is being appended to
/// several times a second.
#[derive(Clone)]
struct OutputRows(Arc<RunOutput>);

impl PartialEq for OutputRows {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One line, in the colour of the stream it came from.
///
/// **stdout and stderr are told apart by colour and by nothing else**, and the colour is
/// deliberately not the red every invalid thing in the app wears: a program writing to
/// stderr is not a program in error -- logs, progress and prompts all go there -- so it
/// takes `string_fg`, the palette's one warm hue and the colour a warning already is. Both
/// are palette fields, so both are answered in the dark theme by the same contrast floor
/// every other foreground is held to.
fn output_row(line: &crate::scratchpad::OutputLine) -> Element {
    let color = match line.stream {
        Stream::Out => palette().text_fg,
        Stream::Err => palette().string_fg,
    };

    rect()
        .width(Size::fill())
        .height(Size::px(code_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 12.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(line.text.to_string())
                .assembly_font()
                .color(color)
                .max_lines(1),
        )
        .into_element()
}

/// The Scratchpad pane: a source file the reader edits, the crates it asks for, a build,
/// and what the compiler said about it.
///
/// **A view and not a document**, which is the rule 8e settled and this inherits whole: a
/// chip in the content strip is a `Selection` -- a place in a binary -- and a scratchpad
/// is not one. What it *builds* is, and needs no rule at all: the executable goes through
/// `open_files` like any other binary and its functions are ordinary chips.
#[derive(PartialEq)]
struct ScratchpadTab;

impl Component for ScratchpadTab {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let state = pad.read().clone();

        // Every bad row at once, keyed by the row it belongs to. `Scratchpad::problems`
        // answers with all of them precisely so that a reader who has two rows wrong is
        // not shown them one at a time.
        let problems: HashMap<usize, Problem> = state.scratchpad.problems().into_iter().collect();
        let rows: Vec<Element> = state
            .scratchpad
            .dependencies
            .iter()
            .enumerate()
            .map(|(index, dependency)| {
                DependencyRow {
                    index,
                    dependency: dependency.clone(),
                    problem: problems.get(&index).cloned(),
                    key: DiffKey::None,
                }
                .key(index)
                .into()
            })
            .collect();

        let diagnostics: Vec<Element> = state.diagnostics().iter().map(diagnostic_block).collect();
        let refusal = state
            .refusal()
            .map(|message| text_block(message, palette().text_fg));
        let package = match state.scratchpad.directory() {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => "nowhere to keep a scratchpad".to_owned(),
        };

        // **One button, because there is one program.** While something is running the
        // only thing to want from it is to stop it, and a Run beside a Stop would be two
        // controls whose combined meaning has to be worked out. It is never both disabled
        // and hiding something: with nothing built there is nothing to run, and the status
        // line above says whether a build has happened.
        let running = state.is_running();
        let run_jobs = jobs.clone();
        let run = Button::new()
            .enabled(running || (state.executable().is_some() && !state.building))
            .on_press(move |_| match running {
                true => stop_run(pad),
                false => request_run(pad, &run_jobs),
            })
            .child(match running {
                true => "Stop",
                false => "Run",
            })
            .into_element();

        let output = state.run_status().map(|(text, bad)| {
            let lines = state.output.clone();
            let length = lines.len();

            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .background(palette().asm_pane_bg)
                .border(bottom_hairline())
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::px(list_row_height()))
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .padding(Gaps::new_symmetric(0.0, 12.0))
                        .spacing(8.0)
                        .content(Content::Flex)
                        .overflow(Overflow::Clip)
                        .child(label().text("Output").font_weight(FontWeight::BOLD))
                        .child(
                            label()
                                .text(text)
                                .width(Size::flex(1.0))
                                .color(match bad {
                                    true => palette().invalid_fg,
                                    false => palette().address_fg,
                                })
                                .max_lines(1),
                        ),
                )
                .child(
                    // The lines go through `new_with_data` and are not captured, which is
                    // the gotcha this list would otherwise walk straight into: the builder
                    // closure is never compared across renders, so a captured `Arc` would
                    // draw the first batch of output for ever.
                    VirtualScrollView::new_with_data(
                        OutputRows(lines),
                        |index, rows: &OutputRows| match rows.0.line(index) {
                            Some(line) => output_row(line),
                            // Only reachable if the list shortened between the length
                            // being read and the row being asked for, which the cap cannot
                            // do -- it drops from the front and keeps the count. An empty
                            // row rather than an index that panics all the same.
                            None => rect().height(Size::px(code_row_height())).into_element(),
                        },
                    )
                    .length(length)
                    .item_size(code_row_height()),
                )
                .into_element()
        });

        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new_symmetric(8.0, 12.0))
                    .spacing(6.0)
                    .child(section_heading(
                        "Scratchpad",
                        Some(
                            rect()
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .spacing(6.0)
                                .child(
                                    Button::new()
                                        // The whole of "two builds cannot be started at
                                        // once", on the control as well as in
                                        // `request_build`: a build takes seconds, and a
                                        // button that goes on looking pressable through
                                        // them is a button that gets pressed again.
                                        .enabled(!state.building)
                                        .on_press(move |_| request_build(pad, &jobs))
                                        .child(match state.building {
                                            true => "Building...",
                                            false => "Build",
                                        }),
                                )
                                .child(run)
                                .into_element(),
                        ),
                    ))
                    // The crate it generates, which is also what the executable it
                    // builds is called -- so the row that appears in the Objects list
                    // after a build is recognisable as this.
                    .child(field_row(
                        "Crate",
                        label()
                            .text(state.scratchpad.name().to_owned())
                            .width(Size::flex(1.0))
                            .max_lines(1),
                    ))
                    // Where it is on disk, which is the whole of what there is to know
                    // about a scratchpad the app did not have to invent a format for: the
                    // package cargo is handed *is* the storage. In a tooltip as well,
                    // because a state directory is longer than any pane this can be
                    // docked in -- which is what a tooltip is for everywhere else here.
                    .child(row_tooltip(
                        package.clone(),
                        field_row(
                            "Package",
                            label()
                                .text(package)
                                .width(Size::flex(1.0))
                                .color(palette().address_fg)
                                .max_lines(1),
                        ),
                    ))
                    .maybe_child(state.status().map(|(text, bad)| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(text)
                                    .color(match bad {
                                        true => palette().invalid_fg,
                                        false => palette().address_fg,
                                    })
                                    .max_lines(1),
                            )
                    }))
                    .child(section_heading(
                        "Dependencies",
                        Some(
                            Button::new()
                                .compact()
                                .on_press(move |_| {
                                    pad.write()
                                        .scratchpad
                                        .dependencies
                                        .push(Dependency::default());
                                })
                                .child("Add")
                                .into_element(),
                        ),
                    ))
                    .child(match rows.is_empty() {
                        true => info_line("No crates asked for".to_owned()).into_element(),
                        false => rect().width(Size::fill()).children(rows).into_element(),
                    })
                    // The package is what the reader is looking at, so the sentence saying
                    // it is not the package on disk goes with the rows that say why.
                    .maybe_child(state.unsaved.map(|failure| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(format!("Not saved: {failure}"))
                                    .color(palette().invalid_fg)
                                    .max_lines(1),
                            )
                    }))
                    // cargo's own words, when they are about these rows and are said
                    // nowhere else. See `PadState::refusal`.
                    .maybe_child(refusal),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(2.0))
                    .border(bottom_hairline())
                    .child(SourceEditor),
            )
            .maybe_child((!diagnostics.is_empty()).then(|| {
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .background(palette().asm_pane_bg)
                    .child(
                        ScrollView::new().child(
                            rect()
                                .width(Size::fill())
                                .padding(Gaps::new_symmetric(4.0, 12.0))
                                .children(diagnostics)
                                .into_element(),
                        ),
                    )
                    .into_element()
            }))
            // Under the diagnostics rather than over them: what the compiler said is about
            // the source directly above it, and what the program said is the newest thing
            // in the pane. Both are `flex(1)` against the editor's `flex(2)`, so a run in a
            // pane that is already showing warnings costs the editor a third of its height
            // and not all of it.
            .maybe_child(output)
    }
}

fn toolbar(objects: State<Vec<Arc<Object>>>, loading: State<Loads>) -> impl IntoElement {
    let on_open = move |_| {
        spawn(async move {
            let Some(handles) = AsyncFileDialog::new()
                .set_title("Open a binary file...")
                .pick_files()
                .await
            else {
                return;
            };

            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();

            // Off the UI thread, and one object at a time: the sidebar has a row per file
            // from here on and fills it in as the objects arrive. See `open_binaries`.
            open_binaries(objects, loading, paths).await;
        });
    };

    rect()
        .horizontal()
        .width(Size::fill())
        .border(bottom_hairline())
        .child(
            rect()
                .margin(4.0)
                .child(Button::new().on_press(on_open).child("Open")),
        )
}

pub fn app() -> impl IntoElement {
    // What the user has said: the theme choice and the two font overrides, read off disk
    // once and then edited by the settings page. Before everything, because the theme and
    // the fonts are resolved from it and both have to be right on the first frame.
    let prefs =
        use_provide_context(|| Prefs(State::create(EditedSettings::of(&Settings::load())))).0;
    // The theme, the fonts and the file, from that one state. See `use_settings`.
    use_settings(prefs);
    // freya's own components -- the filter boxes, the scrollbars, the resizable handle,
    // the tooltips -- take their colours from its `Theme` and not from the palette, so
    // the sheet has to follow the appearance too; `interface_theme` is also where the
    // tooltip's font size is set, which is the one thing freya's theme is used for that
    // has nothing to do with colours -- and the one place a font change has to be carried
    // into freya's theming rather than being picked up by a re-render, which is why the
    // interface size is a dep here beside the appearance.
    //
    // Two calls and not one: `use_init_theme` builds its value in a `use_hook`, so it
    // answers for the first render only, and the effect is what carries a later switch
    // into it. The effect's deps change on a theme or a font change and never per render.
    let mut interface = use_init_theme(|| interface_theme(appearance()));
    use_side_effect_with_deps(
        &(appearance(), fonts().ui.size()),
        move |(appearance, _): &(Appearance, f32)| {
            interface.set(interface_theme(*appearance));
        },
    );

    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    // The files on their way into it, which is what the Objects tree draws its
    // still-being-read rows from. Beside `objects` because it is the same list seen a
    // moment earlier.
    let loading = use_provide_context(|| Loading(State::create(Loads::default()))).0;
    // The id side table the dock's document tabs are handles into. Not the list of open
    // documents -- that is the document panel's own `tabs` vec -- only the mapping from
    // the handle a tab can hold to the document it stands for.
    let docs = use_provide_context(|| OpenDocs(State::create(Docs::default()))).0;
    let sidebar_dock = use_state(|| {
        DockArea::column(vec![
            vec![Tab::View(View::Objects)],
            vec![Tab::View(View::Symbols), Tab::View(View::Info)],
            vec![Tab::View(View::History)],
        ])
    });
    // Panel 0 of the content area is where documents live. Project, Settings and the
    // Scratchpad are tabbed in it beside them, which is where they have always been --
    // behind the Assembly pane, which is now a document's left-hand side rather than a
    // view of its own.
    let content_dock = use_state(|| {
        DockArea::row(vec![vec![
            Tab::View(View::Project),
            Tab::View(View::Settings),
            Tab::View(View::Scratchpad),
        ]])
        .with_documents(DOCUMENT_PANEL)
    });
    use_hook(move || {
        let (mut sidebar_dock, mut content_dock) = (sidebar_dock, content_dock);
        sidebar_dock.write().other = Some(content_dock);
        content_dock.write().other = Some(sidebar_dock);
        sidebar_dock.write().docs = Some(docs);
        content_dock.write().docs = Some(docs);
    });
    let content_dock = use_provide_context(move || ContentDock(content_dock)).0;
    let open = Open {
        dock: content_dock,
        docs,
    };
    // The active document, *derived* from the two above rather than kept beside them:
    // it is the document panel's active tab. See `Active` for why it is a memo and why
    // being a beat behind is right for everything that renders and wrong for the three
    // functions that hold the invariants.
    let active = use_provide_context(move || {
        Active(Memo::create(move || {
            active_document(&content_dock.read(), &docs.read())
        }))
    })
    .0;
    // Where each side of each tab was left, which is a view of what is open rather than a
    // second copy of it: an entry appears when a pane is scrolled and goes when the tab
    // it belongs to is closed, so the same functions hold this true as hold the tabs
    // themselves.
    // The assembly/source split, kept out here because a document's panes are unmounted
    // on every tab switch and take their sizes with them. See `SplitRatio`.
    use_provide_context(|| SplitRatio(State::create(DEFAULT_SPLIT)));
    use_provide_context(|| {
        Splits(State::create(ResizableContext {
            direction: Direction::Horizontal,
            ..Default::default()
        }))
    });
    let asm_at = use_provide_context(|| AsmAt(State::create(Positions::default()))).0;
    let src_at = use_provide_context(|| SrcAt(State::create(Positions::default()))).0;
    let history = use_provide_context(|| Hist(State::create(History::default()))).0;
    // Where the pointer is pointing, which the assembly and source panes answer for each
    // other. A plain state like the ones above rather than something derived from them:
    // it is an input, written by whichever row the pointer is on.
    let focused = use_provide_context(|| Focused(State::create(None))).0;
    // Where a click fixed the two panes, which outlives the pointer moving on and is what
    // asks the other pane to scroll. Beside the focus rather than inside it because the
    // two answer different questions and a row can be either, neither or both.
    let pinned = use_provide_context(|| Pinned(State::create(None))).0;
    // The run of rows picked out to be copied, and whether the keyboard is holding Shift,
    // which is what turns the next click into "reach to here". Both are inputs like the
    // two above: one selection for the whole app, in whichever pane last took one.
    let marked = use_provide_context(|| Marked(State::create(None))).0;
    let mut shift = use_provide_context(|| Shift(State::create(false))).0;
    // Which project all of the above belongs to, and the two things the reader has said
    // about it. A state rather than something read out of `project.rs` when it is drawn,
    // because the project view both draws it and edits it -- which is also what let the
    // save policy stop carrying the name across its own calls.
    let proj = use_provide_context(|| Proj(State::create(OpenProject::default()))).0;
    // The eight of them together, since a project switch closes all of them and reopens
    // all of them.
    let states = ProjectStates {
        proj,
        objects,
        loading,
        open,
        asm_at,
        src_at,
        history,
    };
    use_save_on_change(states);
    use_clear_focus(active, focused, pinned);
    use_periodic_save();
    // After the save effect on purpose: the effect is in place, with the save policy's
    // empty baseline, before the restore can put anything into any of the states it
    // observes, so the restored session is seen by it as an ordinary change.
    use_restore_on_startup(states);

    // Rebuilt only when the object list changes, not on every selection change.
    let symbols = use_memo(move || {
        SymbolList(Arc::new(
            objects
                .read()
                .iter()
                .flat_map(|object| {
                    object.symbols_sorted.iter().cloned().map(|data| Symbol {
                        object: object.clone(),
                        data,
                    })
                })
                .collect::<Vec<_>>(),
        ))
    });
    use_provide_context(move || Symbols(symbols));

    // The selected symbol's disassembly and line info, worked out once on a worker thread
    // for every pane that wants them. Both used to run in `render` -- the disassembly in
    // the Assembly pane and the line info in a `use_memo` here -- which is worker-thread
    // work by the analysis crate's own note on it: the first line-info query against a big
    // binary builds the whole DWARF context (267 MB for `viewer-sample`) and stalled the
    // frame that asked for it.
    let analysis = use_provide_context(|| Analysis(State::create(Analyzed::default()))).0;
    use_analysis(active, analysis);
    // After the analysis, because the file the Source pane draws for an assembly-driven
    // tab is what the analysis says it is.
    use_clear_marks(active, analysis, marked);

    // The scratchpad: the source the reader edits, the crates it asks for, and the worker
    // that is the only thing which ever reads or writes its directory. Both states are
    // provided here rather than held by the view because a dock tab that is not the active
    // one in its panel is unmounted, and a buffer being typed into cannot live there.
    let pad = use_provide_context(|| Pad(State::create(PadState::default()))).0;
    let pad_text = use_provide_context(|| {
        PadText(State::create(CodeEditorData::new(
            Rope::from_str(&pad.peek().scratchpad.source),
            language(Path::new(SOURCE_FILE)),
        )))
    })
    .0;
    use_scratchpad(pad, pad_text, states);

    // One docking area per resizable pane: the left one a column of Objects, then
    // Symbols with Info tabbed beside it, then History at the bottom -- which is
    // where the goal asks for it, and where it is visible without a click. The
    // cost is that the three groups start at equal heights, so the symbol list is
    // shorter than it was; the handles between them, and dragging History onto the
    // middle panel, are both one gesture away. The right one is the split view the
    // goals ask to be the default: the source a symbol was compiled from beside its
    // assembly, at equal widths. All nine tabs share one `DockDrag<Tab>`, which
    // `use_drag` keeps at the root, so a tab can be dragged from either area into
    // the other; each area is told about the other so the one taking a tab can evict
    // it from the one losing it.

    // The split is freya's own `ResizableContainer`: the sidebar panel keeps the
    // original fixed 300px (`PanelSize::px`, so the initial width is unchanged) and
    // the content panel is the single proportional one, which makes it take whatever
    // is left over -- the same thing the old `Size::flex(1.0)` did. Between them
    // freya inserts a `ResizableHandle`, a 4px draggable divider that replaces the
    // hairline border the sidebar used to draw. Docking cannot express a pixel
    // width, which is why this outer split is not itself a `DockingArea`.
    let split = ResizableContainer::new()
        .direction(Direction::Horizontal)
        .panel(
            ResizablePanel::new(PanelSize::px(300.0))
                .min_size(120.0)
                .child(docking_area(sidebar_dock, docs)),
        )
        .panel(
            ResizablePanel::new(PanelSize::percent(100.0))
                .min_size(10.0)
                // No strip of its own any more: the open documents *are* tabs in the
                // dock's document panel, so the bar over them is that panel's own tab
                // bar. `DockingArea` renders itself `.expanded()`, so it needs a parent
                // that has been given the height.
                .child(docking_area(content_dock, docs)),
        );

    rect()
        .expanded()
        .content(Content::Flex)
        .interface_font()
        // The interface text, set once here and inherited: freya resolves an element's
        // unset `color` from its parent's, so every label in the chrome that does not ask
        // for a colour of its own follows this one. In the light palette it is the black
        // that was already the default, so this changes nothing until the theme does.
        .color(palette().text_fg)
        .background(palette().pane_bg)
        // The mouse's own back and forward buttons drive the history. freya does
        // deliver them: winit turns X11 buttons 8 and 9, and Wayland's BTN_BACK/
        // BTN_SIDE and BTN_FORWARD/BTN_EXTRA, into `MouseButton::Back`/`Forward`,
        // freya-winit maps those one for one and puts them in the `PlatformEvent`,
        // and nothing between there and the handler filters on which button it is.
        // `on_global_pointer_down` rather than `on_pointer_down`: a global event is
        // emitted to its listeners with no hit test at all, so this fires wherever
        // in the window the cursor happens to be and no child can swallow it by
        // stopping propagation. The rows are unaffected -- `on_press` is left-button
        // only -- and so is `on_secondary_down`, which asks for the right button.
        .on_global_pointer_down(move |e: Event<PointerEventData>| match e.button() {
            Some(MouseButton::Back) => navigate(open, history, Nav::Back),
            Some(MouseButton::Forward) => navigate(open, history, Nav::Forward),
            _ => {}
        })
        // A row selection is swept out with the button down and ends wherever the button
        // comes up, which is very often not over the pane it started in -- so the end of
        // the gesture is watched for here, at the root, rather than by either list.
        .on_global_pointer_press(move |_| mark_release(marked))
        // Shift, watched globally for the reason on `Shift` itself: a pointer event
        // carries no modifiers, so the state of the key has to be known before the click
        // that asks about it. Global rather than on the focused pane so that the first
        // shift-click into a pane extends, instead of only the ones after it has the
        // keyboard; and it is a bool being set, so it costs a listening pane nothing.
        //
        // The key itself is tested as well as the modifier mask, and freya-edit's
        // `TextDragging` does the same: the press that turns Shift *on* is the one
        // platforms disagree about, some reporting the mask before the key it names and
        // some after. The mask is what keeps the two in step when a key event is missed
        // -- the window losing focus mid-gesture, say.
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            shift.set_if_modified(
                e.key == Key::Named(NamedKey::Shift) || e.modifiers.contains(Modifiers::SHIFT),
            );
        })
        .on_global_key_up(move |e: Event<KeyboardEventData>| {
            shift.set_if_modified(
                e.key != Key::Named(NamedKey::Shift) && e.modifiers.contains(Modifiers::SHIFT),
            );
        })
        // The context menu the objects tree opens on a file row. It is the *viewer* that
        // has to be here: it provides the root state `ContextMenu::open_from_event` looks
        // up -- opening a menu without one in an ancestor scope panics -- and it draws
        // the menu itself, at the pointer, on the overlay layer. At the root so the menu
        // inherits the interface font, as freya's own documentation asks, and it lays out
        // as nothing at all until a menu is open.
        .child(ContextMenuViewer::new())
        .child(toolbar(objects, loading))
        // `ResizableContainer` renders itself `.expanded()`, so it needs a parent
        // that has already been given the leftover height under the toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(split),
        )
}

#[cfg(test)]
mod tests;

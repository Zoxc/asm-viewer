//! The language server as the app holds it: whether one is running, the worker that talks
//! to it, and the control in the top bar that starts and stops it.
//!
//! `use_building_with`'s shape (`src/ui/building.rs`), and for its reasons: talking to a
//! server blocks, so it goes to a thread of its own, and it is **one** thread because
//! there is one server and one conversation with it.
//!
//! Nothing starts it by itself. A language server reads a whole project and keeps it in
//! memory, and most of what this app is for -- reading a binary somebody else built -- has
//! no use for one; so it is a control the reader presses, and it is off when the app
//! opens however it was left.
//!
//! A press is not enough on its own the first time. A server runs the project's own
//! build scripts and proc macros, so a directory the reader has not agreed to is asked
//! about instead of started. The answer is kept in `project.toml`, and it is about a
//! directory: change the project's directory, or leave for another project, and it goes.
//!
//! Two things say which server an answer is about, and they are not the same thing. The
//! **run** counts starts and stops, so an answer for a server that has been stopped is
//! dropped rather than shown: `use_analysis` compares questions instead, but the thing an
//! answer here is about is a process, which does not exist until the worker has started
//! it. The **handle** is what ends that process, and the app holds it from the moment the
//! worker hands it over -- a handle dropped instead of stopped is a language server
//! nothing can ever find again.

use super::*;

/// Where the language server is.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Lsp {
    #[default]
    Off,
    /// Asked for, and not yet answering: starting one takes a moment and reading the
    /// project takes longer.
    Starting,
    Running,
    /// It could not be started, or it stopped answering. What it says is the reason,
    /// which the control shows and nothing else does.
    Failed(String),
}

/// A start the reader has not agreed to yet: what would be run, and where.
///
/// Held rather than worked out again when they answer: the question named a directory,
/// and the agreement is to that one and not to whatever the box says by then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asking {
    pub(crate) directory: PathBuf,
    pub(crate) program: String,
}

/// The language server as the app holds it.
#[derive(Clone, Default)]
pub(crate) struct Language {
    pub(crate) state: Lsp,
    /// Whether it is reading the project rather than answering about it. Not a state of
    /// its own: a server that is working is running, and this is what it is doing.
    pub(crate) working: bool,
    /// The start that has been asked about and not answered yet. `None` unless the
    /// prompt is up.
    pub(crate) asking: Option<Asking>,
    /// What the project's own `.vscode/settings.json` said, or why it could not be used.
    /// `None` until the read that follows the project has answered.
    ///
    /// Read through the worker and held here rather than in the Project view: the view is
    /// a dock tab and an inactive tab is unmounted, while the control in the top bar
    /// starts a server from wherever the reader is. One read, at the root, answers both.
    settings: Option<Result<lsp::Settings, lsp::Unreadable>>,
    /// Which server the answers arriving are about, counted up by every start and every
    /// stop.
    pub(crate) run: u64,
    /// What ends the running server. `None` unless one is running.
    server: Option<lsp::Handle>,
}

impl PartialEq for Language {
    /// The handle is not compared: there is one server per run, so a run that has not
    /// moved is the same server.
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.run == other.run
            && self.working == other.working
            && self.asking == other.asking
            && self.settings == other.settings
    }
}

impl Language {
    /// Whether pressing the control stops it rather than starting it.
    pub(crate) fn started(&self) -> bool {
        matches!(self.state, Lsp::Starting | Lsp::Running)
    }

    /// Whether it is there to be asked a question about a whole file: running, and done
    /// reading the project. A question put before that would hold the one conversation
    /// until it was answered, with every click queued behind it (`src/ui/linking.rs`).
    pub(crate) fn ready(&self) -> bool {
        matches!(self.state, Lsp::Running) && !self.working
    }

    /// Whether something is going on: starting one, or a server reading the project.
    /// What the control draws a turning loader for instead of its own icon.
    pub(crate) fn busy(&self) -> bool {
        matches!(self.state, Lsp::Starting) || self.working
    }

    /// What the Project view says about it: the line, and whether it is bad news.
    ///
    /// `directory` is whether the project has one to run a server over, which is the
    /// state's own answer to nothing and the reason there is no server all the same.
    pub(crate) fn status(&self, directory: bool) -> (String, bool) {
        if !directory {
            return ("No directory".to_owned(), false);
        }
        match &self.state {
            Lsp::Off => (
                "Not running. The control in the top bar starts it.".to_owned(),
                false,
            ),
            Lsp::Starting => ("Starting...".to_owned(), false),
            Lsp::Running if self.working => ("Reading the project...".to_owned(), false),
            Lsp::Running => ("Running".to_owned(), false),
            Lsp::Failed(why) => (why.clone(), true),
        }
    }

    /// What the project's own settings gave the server, as the Project view lists them:
    /// the name with `rust-analyzer.` off it, and the value as it will be sent.
    pub(crate) fn overrides(&self) -> &[(String, String)] {
        match &self.settings {
            Some(Ok(settings)) => &settings.overrides,
            _ => &[],
        }
    }

    /// Why the project's own settings could not be used, when they could not. A start is
    /// refused while this is here.
    pub(crate) fn unreadable(&self) -> Option<String> {
        match &self.settings {
            Some(Err(why)) => Some(why.to_string()),
            _ => None,
        }
    }

    /// What the control says on hover: the state, in words, and the reason when there is
    /// one.
    pub(crate) fn words(&self) -> String {
        match &self.state {
            Lsp::Off => "Start rust-analyzer".to_owned(),
            Lsp::Starting => "Starting rust-analyzer".to_owned(),
            Lsp::Running if self.working => "rust-analyzer is reading the project".to_owned(),
            Lsp::Running => "Stop rust-analyzer".to_owned(),
            Lsp::Failed(why) => why.clone(),
        }
    }
}

/// What the control is called. Not the program's name: the bar has room for three letters
/// beside two chevrons, and what the reader is being told is which of the app's parts this
/// is. The program's own name is in the tooltip and in the Project view.
const SERVER_NAME: &str = "LSP";

/// A place in a source file, as the protocol takes one: the line counts from zero and the
/// column is in UTF-16 units, which is what the rows are measured in (`src/chars.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lookup {
    pub(crate) file: PathBuf,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

/// Which question about a place is being asked. They all go out the same way and come
/// back the same way, so this is what tells one answer from the other -- and what keeps a
/// reader asking for references from cancelling a definition still in flight
/// (`worth_doing`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Wanted {
    /// Where the name is defined, which `ui::follow` opens.
    Definition,
    /// Where it is **declared**, which `ui::follow` opens as well. A different question of
    /// the server with the same answer and the same door, asked for an item in a trait
    /// `impl`, whose definition is itself and whose declaration is the trait's
    /// (`src/links.rs`).
    Declaration,
    /// What implements it, which the Locations panel lists.
    Implementations,
    /// Everywhere it is used, which the Locations panel lists.
    References,
}

impl Wanted {
    /// Whether the answer is a place to go to rather than a list to draw, which is what
    /// `worth_doing` buckets by: the panel's two questions supersede each other because
    /// the panel draws one of them at a time, and neither takes back a followed name.
    /// A definition and a declaration are one kind for the same reason -- only ever one
    /// of the two is asked of a name, and both open the same door.
    pub(crate) fn followed(self) -> bool {
        matches!(self, Wanted::Definition | Wanted::Declaration)
    }
}

/// What the worker is asked to do.
pub(crate) enum LspJob {
    /// Start a server over `directory` and shake hands with it. The channel is what the
    /// server's own remarks come back on, since they arrive long after this is answered.
    Start {
        run: u64,
        directory: PathBuf,
        /// The program to run, which is the project's when it named one.
        program: String,
        /// What to tell it about the project. Carried in the job because the worker
        /// thread may read no UI state, exactly as `program` and `directory` are.
        settings: lsp::Settings,
        notes: async_channel::Sender<(u64, lsp::Note)>,
    },
    /// Read the project's own `.vscode/settings.json`. A file read blocks, so it happens
    /// here rather than on the UI thread; it is this worker's and not the build worker's
    /// because what it answers is what a start has to carry.
    ReadSettings { directory: PathBuf },
    /// What is at a place: which of the four questions is in `want`.
    Ask { run: u64, at: Lookup, want: Wanted },
    /// What every name in one file is, which is a question about the file and not about
    /// a place in it. The file travels as the `Arc<str>` a document is named by, since
    /// that is what the answer has to be matched against.
    Tokens { run: u64, file: Arc<str> },
    /// Let go of the server: it has been stopped already, and this is what reaps it.
    Stop,
}

/// What came of it. Every answer names the run it is about, and one whose run has moved
/// on is not the answer to any question anybody still has.
pub(crate) enum LspAnswer {
    Started {
        run: u64,
        server: Result<lsp::Handle, lsp::Failure>,
    },
    /// What one question came back with, and which question it was.
    Answered {
        run: u64,
        want: Wanted,
        reply: Result<Reply, lsp::Failure>,
    },
    /// What every name in one file is, and which file. Its own answer and not a `Reply`,
    /// since it is the one question about a file rather than about a place in one.
    Linked {
        run: u64,
        file: Arc<str>,
        links: Result<links::Links, lsp::Failure>,
    },
    /// What the project's own settings file said. Named by the directory it was read in
    /// and not by a run: it is about a project and not about a process.
    Settings {
        directory: PathBuf,
        settings: Result<lsp::Settings, lsp::Unreadable>,
    },
}

/// What an answer holds, which is what was asked for.
///
/// A definition is places and nothing more -- what opens one is a file and a line.
/// References are grouped and carry the text of every line they are on, since the lines
/// are **read here**: the read blocks, and this is the thread that may block.
pub(crate) enum Reply {
    Defined(Vec<lsp::Place>),
    Referenced(references::References),
}

/// The blocking half, and the only part that talks to a server.
///
/// A closure holding the conversation rather than a plain function, since unlike the other
/// three workers this one has something to keep between jobs. The lock is never contended
/// -- one thread calls this -- and is what lets the seam stay the `Fn` the others are.
pub(crate) fn language_work() -> impl Fn(LspJob) -> Option<LspAnswer> + Send + 'static {
    let talking: Mutex<Option<lsp::Server>> = Mutex::new(None);

    move |job| {
        let mut talking = talking.lock().unwrap_or_else(|held| held.into_inner());
        match job {
            LspJob::Start {
                run,
                directory,
                program,
                settings,
                notes,
            } => {
                // Whatever was there is dropped first, which kills it: two servers over
                // one project would be twice the memory for one answer.
                *talking = None;
                // `send_blocking` and not `try_send`: the channel is bounded, and a note
                // dropped because the app was busy is a control that never stops saying
                // the server is working.
                let told = move |note| {
                    let _ = notes.send_blocking((run, note));
                };
                let started =
                    lsp::start_in(&program, &directory, told).and_then(|(mut server, handle)| {
                        server.initialize(&directory, settings.options())?;
                        Ok((server, handle))
                    });
                let server = match started {
                    Ok((server, handle)) => {
                        *talking = Some(server);
                        Ok(handle)
                    }
                    Err(failure) => Err(failure),
                };
                Some(LspAnswer::Started { run, server })
            }
            LspJob::Ask { run, at, want } => {
                let talk = talking.as_mut()?;
                let places = match want {
                    Wanted::Definition => talk.definition(&at.file, at.line, at.column),
                    Wanted::Declaration => talk.declaration(&at.file, at.line, at.column),
                    Wanted::Implementations => talk.implementations(&at.file, at.line, at.column),
                    Wanted::References => talk.references(&at.file, at.line, at.column),
                };
                // A conversation that ended is a server that is gone, so it is let go of
                // here and reported as the failure it is.
                if matches!(places, Err(lsp::Failure::Broken(_))) {
                    *talking = None;
                }
                // The references are grouped and their lines read here, with the ask: the
                // reading is what the panel draws and a file read is what this thread is
                // for.
                let reply = places.map(|places| match want {
                    Wanted::Definition | Wanted::Declaration => Reply::Defined(places),
                    Wanted::Implementations | Wanted::References => {
                        Reply::Referenced(references::References::of(&places, |path| {
                            std::fs::read_to_string(path).ok()
                        }))
                    }
                });
                Some(LspAnswer::Answered { run, want, reply })
            }
            LspJob::Tokens { run, file } => {
                let talk = talking.as_mut()?;
                // Classified here rather than on the UI thread: it is a walk of every
                // name in the file, and this is the thread that may take its time. Done
                // while the conversation is still in hand, the legend being its.
                let links = talk
                    .semantic_tokens(Path::new(&*file))
                    .map(|tokens| links::Links::of(talk.legend(), &tokens));
                // A conversation that ended is a server that is gone, as above.
                if matches!(links, Err(lsp::Failure::Broken(_))) {
                    *talking = None;
                }
                Some(LspAnswer::Linked { run, file, links })
            }
            LspJob::ReadSettings { directory } => Some(LspAnswer::Settings {
                settings: lsp::settings_in(&directory),
                directory,
            }),
            LspJob::Stop => {
                *talking = None;
                None
            }
        }
    }
}

/// The jobs worth doing, of the one taken off the channel and everything queued behind it.
///
/// Only the last question **of each kind** is asked: a reader clicking twice wants the
/// second answer, and the first is a conversation the second would only wait behind -- but
/// a reader who asks for a name's references has not taken back the definition they asked
/// for, and the two are answered by different parts of the app. A kind is a **consumer**
/// and not a question: `ui::follow` takes a definition or a declaration, never both at
/// once, and the Locations panel draws implementations or references in the one place. The
/// same for reading the project's settings, which a directory typed a letter at a time
/// asks for once a keystroke and only the last of which is about the project that is open.
/// Starting and stopping are never dropped -- they are what the reader pressed.
///
/// The match over `LspJob` is exhaustive on purpose: a job added with nothing said about
/// superseding would otherwise queue behind every one of its own kind in silence.
pub(crate) fn worth_doing(first: LspJob, queued: impl Iterator<Item = LspJob>) -> Vec<LspJob> {
    let jobs: Vec<LspJob> = std::iter::once(first).chain(queued).collect();
    let last = |wanted: &dyn Fn(&LspJob) -> bool| jobs.iter().rposition(|job| wanted(job));
    let asked = |followed: bool| {
        last(&|job| matches!(job, LspJob::Ask { want, .. } if want.followed() == followed))
    };
    let (following, listing) = (asked(true), asked(false));
    let read = last(&|job| matches!(job, LspJob::ReadSettings { .. }));
    // One pane shows one file, so a question about another is a question about what the
    // reader has already left.
    let linking = last(&|job| matches!(job, LspJob::Tokens { .. }));
    jobs.into_iter()
        .enumerate()
        .filter(|(at, job)| match job {
            LspJob::Ask { want, .. } => match want.followed() {
                true => Some(*at) == following,
                false => Some(*at) == listing,
            },
            LspJob::ReadSettings { .. } => Some(*at) == read,
            LspJob::Tokens { .. } => Some(*at) == linking,
            LspJob::Start { .. } | LspJob::Stop => true,
        })
        .map(|(_, job)| job)
        .collect()
}

/// How the control reaches the worker, and how the server reaches the control.
#[derive(Clone)]
pub(crate) struct LspJobs {
    jobs: async_channel::Sender<LspJob>,
    /// Handed to each server started, so what it says while nothing was asked arrives
    /// under the run it was started in.
    notes: async_channel::Sender<(u64, lsp::Note)>,
}

impl LspJobs {
    pub(crate) fn send(&self, job: LspJob) {
        // A full queue is impossible (it is unbounded) and a closed one is the app going
        // down, so there is nothing here to tell anybody about.
        let _ = self.jobs.try_send(job);
    }
}

/// The language server as a component sees it.
#[derive(Clone, Copy)]
pub(crate) struct Talking(pub(crate) State<Language>);

/// Start the worker and keep the state in step with it. Called once, at the root.
pub(crate) fn use_language_with(
    mut language: State<Language>,
    mut follow: State<Follow>,
    mut located: State<Located>,
    mut linked: State<Linked>,
    mut proj: State<OpenProject>,
    work: impl Fn(LspJob) -> Option<LspAnswer> + Send + 'static,
) -> LspJobs {
    let jobs = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<LspJob>();
        let (answered, answers) = async_channel::unbounded::<LspAnswer>();
        // Bounded: a server that reports progress in a tight loop is one the app can fall
        // behind, and the reader thread waiting is the only backpressure there is.
        let (told, notes) = async_channel::bounded::<(u64, lsp::Note)>(64);

        // A `std::thread` and not a spawned task: a server that is reading a project can
        // take a minute to answer, and freya's executor is the UI thread.
        std::thread::spawn(move || {
            while let Ok(job) = jobs.recv_blocking() {
                for job in worth_doing(job, std::iter::from_fn(|| jobs.try_recv().ok())) {
                    let Some(answer) = work(job) else {
                        continue;
                    };
                    // A send that fails is the app shutting down and taking the receiver
                    // with it.
                    if answered.send_blocking(answer).is_err() {
                        return;
                    }
                }
            }
        });

        spawn(async move {
            while let Ok(answer) = answers.recv().await {
                match answer {
                    LspAnswer::Started { run, server } => {
                        let held = language.peek().clone();
                        if held.run != run {
                            // Stopped, or restarted, while it was starting. This is the
                            // first moment anything in the app holds the handle, so
                            // dropping it would leave a server running that nothing could
                            // ever name again.
                            if let Ok(handle) = server {
                                handle.stop();
                            }
                            continue;
                        }
                        let (state, server) = match server {
                            Ok(handle) => (Lsp::Running, Some(handle)),
                            Err(failure) => (Lsp::Failed(failure.to_string()), None),
                        };
                        language.set(Language {
                            // Whatever the server has already said about itself: the
                            // handshake's answer and its first `$/progress` are two
                            // messages, and either can be taken first.
                            working: held.working,
                            asking: held.asking,
                            settings: held.settings,
                            state,
                            run,
                            server,
                        });
                    }
                    LspAnswer::Settings {
                        directory,
                        settings,
                    } => {
                        // A file read for a project that has since been left says nothing
                        // about the one that is open now.
                        let open = workspace(&proj.peek());
                        if open.as_deref() != Some(directory.as_path()) {
                            continue;
                        }
                        let held = language.peek().clone();
                        language.set(Language {
                            settings: Some(settings),
                            ..held
                        });
                    }
                    LspAnswer::Linked { run, file, links } => {
                        let held = language.peek().clone();
                        if held.run != run {
                            continue;
                        }
                        // Nothing found and a server that refused both leave the pane
                        // with no links, which is what it draws with no server either:
                        // there is nothing to say about a name nobody classified.
                        let why = match links {
                            Ok(links) => {
                                let mut waiting = linked.peek().clone();
                                if waiting.answer(run, file, links) {
                                    linked.set(waiting);
                                }
                                continue;
                            }
                            Err(failure @ lsp::Failure::Refused { .. }) => {
                                log::warn!("the language server refused a question: {failure}");
                                let mut waiting = linked.peek().clone();
                                if waiting.answer(run, file, links::Links::default()) {
                                    linked.set(waiting);
                                }
                                continue;
                            }
                            Err(failure) => failure,
                        };
                        let held = language.peek().clone();
                        language.set(Language {
                            state: Lsp::Failed(why.to_string()),
                            working: false,
                            asking: held.asking,
                            settings: held.settings,
                            run,
                            server: None,
                        });
                    }
                    LspAnswer::Answered { run, want, reply } => {
                        let held = language.peek().clone();
                        if held.run != run {
                            continue;
                        }
                        // Whichever question it was, whoever asked it takes the answer,
                        // and gives up on it where there is none. Bound before the write,
                        // as ever.
                        let mut take = |reply: Option<Reply>| match want {
                            Wanted::Definition | Wanted::Declaration => {
                                let places = match &reply {
                                    Some(Reply::Defined(places)) => places.as_slice(),
                                    _ => &[],
                                };
                                let mut waiting = follow.peek().clone();
                                let moved = match reply.is_some() {
                                    true => waiting.answer(run, places),
                                    false => waiting.give_up(run),
                                };
                                if moved {
                                    follow.set(waiting);
                                }
                            }
                            Wanted::Implementations | Wanted::References => {
                                let found = match reply {
                                    Some(Reply::Referenced(found)) => found,
                                    _ => references::References::default(),
                                };
                                let mut waiting = located.peek().clone();
                                // Nothing found and nothing to be found both leave the
                                // panel saying so: a question that stayed pending would
                                // say it was still looking for ever.
                                if waiting.answer_places(run, found) {
                                    located.set(waiting);
                                }
                            }
                        };
                        let why = match reply {
                            Ok(reply) => {
                                // An answer naming nowhere is an answer: the click was a
                                // question, not a promise.
                                take(Some(reply));
                                continue;
                            }
                            // The server refused the question -- it is still reading the
                            // project, or has no such file of its own. Nothing found, and
                            // not a server to say anything about: it is answering.
                            Err(failure @ lsp::Failure::Refused { .. }) => {
                                log::warn!("the language server refused a question: {failure}");
                                take(None);
                                continue;
                            }
                            Err(failure) => failure,
                        };
                        // What is left is a server that stopped answering, which is the
                        // one thing the control has to show.
                        take(None);
                        let held = language.peek().clone();
                        language.set(Language {
                            state: Lsp::Failed(why.to_string()),
                            working: false,
                            asking: held.asking,
                            settings: held.settings,
                            run,
                            server: None,
                        });
                    }
                }
            }
        });

        spawn(async move {
            while let Ok((run, note)) = notes.recv().await {
                let held = language.peek().clone();
                // A remark from a server that has been stopped is about nothing the
                // control still says.
                if held.run != run {
                    continue;
                }
                let lsp::Note::Busy(working) = note;
                if held.working == working {
                    continue;
                }
                language.set(Language { working, ..held });
            }
        });

        LspJobs {
            jobs: requests,
            notes: told,
        }
    });

    // Leaving a project ends its server and takes the reader's agreement with it: it is
    // the project's directory the server was started over and the directory they agreed
    // to, and a directory typed into the Project view is a different project's on both
    // counts. The two reads are bound first, since the stop below writes the state this
    // effect is about.
    let open = proj.read().clone();
    let deps = (open.id.clone(), workspace(&open));
    // What the effect last saw, so that it can tell the two changes apart. A directory
    // typed into the box is the reader pointing *this* project somewhere else, and the
    // agreement was to the old place; a project arriving is another project's answer
    // arriving with it, and that answer is its own to give. The mount is neither: what it
    // mounts with is the reopened project, the restore being an earlier hook of the same
    // render, so an agreement read out of `project.toml` survives the launch that read it.
    let seen: Rc<RefCell<Option<(Option<ProjectId>, Option<PathBuf>)>>> =
        use_hook(|| Rc::new(RefCell::new(None)));
    use_side_effect_with_deps(&deps, {
        let jobs = jobs.clone();
        move |(id, directory): &(Option<ProjectId>, Option<PathBuf>)| {
            stop_server(language, &jobs);
            // And the settings go with it: they were another project's. Read again here,
            // where a project arrives, so the answer is in hand before either press can
            // ask for a server and whether or not one is ever started -- the Project view
            // lists them either way. The read is bound before the write, as ever.
            let held = language.peek().clone();
            language.set(Language {
                settings: None,
                ..held
            });
            if let Some(directory) = directory.clone() {
                jobs.send(LspJob::ReadSettings { directory });
            }
            let before = seen.replace(Some((id.clone(), directory.clone())));
            let moved = before.is_some_and(|(was_id, was_directory)| {
                was_id == *id && was_directory != *directory
            });
            if moved {
                proj.write().trusted = false;
            }
        }
    });

    // A context, because the control that presses it is drawn from a component that is
    // handed nothing; returned as well, so a test can ask directly.
    use_provide_context(|| jobs.clone())
}

/// Start the project's server over the project's directory -- or, where the reader has
/// not agreed to that directory, put the question and start nothing.
///
/// The project rather than a directory and a program: both presses that reach here are
/// about the project that is open, and what is asked about has to be what would run.
pub(crate) fn start_server(
    mut language: State<Language>,
    proj: State<OpenProject>,
    jobs: &LspJobs,
) {
    let open = proj.peek().clone();
    // Nothing to run one over.
    let Some(directory) = workspace(&open) else {
        return;
    };
    let asking = Asking {
        directory,
        program: open.server(),
    };
    if open.trusted {
        run_server(language, jobs, asking);
        return;
    }
    let held = language.peek().clone();
    // A second press with the same question up asks it again, which is nothing.
    if held.asking.as_ref() == Some(&asking) {
        return;
    }
    language.set(Language {
        asking: Some(asking),
        ..held
    });
}

/// Start what was asked for, leaving whatever was running stopped.
///
/// **A settings file that could not be read starts nothing.** What it would otherwise
/// reach the server as is a name it ignores or a path that is not there, and a server
/// reading the wrong project is worse than one that says why it did not start. The check
/// is here because this is where a start happens, so neither press nor the agreement can
/// grow a path around it -- the same reason the trust gate is in `start_server`.
fn run_server(mut language: State<Language>, jobs: &LspJobs, asking: Asking) {
    let held = language.peek().clone();
    // Not read yet is nothing to lay over `wanted()`: the read follows the project, and
    // answers long before a press can reach here.
    let ready = match &held.settings {
        Some(Err(why)) => Err(why.to_string()),
        Some(Ok(settings)) => Ok(settings.clone()),
        None => Ok(lsp::Settings::none()),
    };
    if let Some(handle) = &held.server {
        handle.stop();
    }
    let run = held.run + 1;
    let (state, settings) = match ready {
        Ok(settings) => (Lsp::Starting, Some(settings)),
        Err(why) => (Lsp::Failed(why), None),
    };
    language.set(Language {
        state,
        working: false,
        asking: None,
        run,
        server: None,
        settings: held.settings,
    });
    let Some(settings) = settings else {
        return;
    };
    jobs.send(LspJob::Start {
        run,
        directory: asking.directory,
        program: asking.program,
        settings,
        notes: jobs.notes.clone(),
    });
}

/// The reader agrees: the project keeps the answer, and the start it was asked about
/// goes ahead.
pub(crate) fn agree_to_start(
    language: State<Language>,
    mut proj: State<OpenProject>,
    jobs: &LspJobs,
) {
    // Bound to a `let` of its own, since the two writes below are to the states this was
    // read from.
    let asked = language.peek().asking.clone();
    let Some(asking) = asked else {
        return;
    };
    proj.write().trusted = true;
    run_server(language, jobs, asking);
}

/// The reader declines: the question goes and **nothing is remembered**, so the next
/// press asks again.
pub(crate) fn decline_start(mut language: State<Language>) {
    let held = language.peek().clone();
    if held.asking.is_none() {
        return;
    }
    language.set(Language {
        asking: None,
        ..held
    });
}

/// The reader takes the agreement back: the project forgets it, and the server it was
/// given for stops.
///
/// Stopping is not tidiness. A reader who says they did not mean to let a program read
/// this directory has said something about the program that is reading it *now*; leaving
/// it running would answer them with a control that says "not agreed to" over a server
/// happily going through their project. An unanswered question goes with it, since it
/// asked about the very thing that has just been refused.
pub(crate) fn revoke_trust(
    language: State<Language>,
    mut proj: State<OpenProject>,
    jobs: &LspJobs,
) {
    // Bound before the writes, as ever.
    let agreed = proj.peek().trusted;
    if !agreed {
        return;
    }
    proj.write().trusted = false;
    stop_server(language, jobs);
    decline_start(language);
}

/// Stop the server, if there is one, and put the control back where it started -- which a
/// failure that is still on it needs as much as a running server does.
///
/// The kill happens here and the worker is only told afterwards: a worker waiting on a
/// server that will never answer is let go by the pipes closing, which is the kill and not
/// the job.
pub(crate) fn stop_server(mut language: State<Language>, jobs: &LspJobs) {
    let held = language.peek().clone();
    if matches!(held.state, Lsp::Off) && held.server.is_none() && held.asking.is_none() {
        return;
    }
    if let Some(handle) = &held.server {
        handle.stop();
    }
    // An unanswered question goes with it: it was about the project being left. The
    // project's own settings stay: they are the project's and not the server's.
    language.set(Language {
        state: Lsp::Off,
        working: false,
        asking: None,
        run: held.run + 1,
        server: None,
        settings: held.settings,
    });
    jobs.send(LspJob::Stop);
}

/// Ask `want` about the place `at`. The answer is the worker's, and arrives under the run
/// it was asked in, which is what this hands back so a caller can tell its own answer from
/// another's.
///
/// `None` with no server: there is nobody to ask, and a question is not what starts one --
/// that is the control, and only the reader presses it. One that is still starting is
/// asked all the same, the question queueing behind the start to be answered once there is
/// somebody to answer it, and finding nothing to talk to if the start failed.
pub(crate) fn ask_where(
    language: State<Language>,
    jobs: &LspJobs,
    at: Lookup,
    want: Wanted,
) -> Option<u64> {
    let held = language.peek().clone();
    if !held.started() {
        return None;
    }
    jobs.send(LspJob::Ask {
        run: held.run,
        at,
        want,
    });
    Some(held.run)
}

/// The control in the top bar: one press starts the language server, the next stops it.
///
/// Named, bordered and coloured rather than an icon alone. It is the only thing in the app
/// that starts a process the reader did not ask for by name, so what it is about is
/// written on it, and its state is the border and the colour rather than a shape a reader
/// has to have learned. **Nothing about it changes width** -- the two history buttons sit
/// beside it at the bar's right corner, and a label or an icon that grew would walk them
/// out from under the pointer -- so the state is said in the same three letters, the same
/// square of icon, and the tooltip.
///
/// The icon is a link and not a pair of braces: braces are code, which is what every other
/// icon in this app is already about -- a file of it, a function in it, a binary of it --
/// and they say nothing about what this one is for. What a language server is asked here is
/// where a name leads, and a link is that question rather than the machinery answering it.
/// It also holds its shape at the size the bar draws it, which a magnifier over a `</>`
/// does not. Beside three letters naming the kind of server, that is the whole caption.
///
/// Off it is text alone, with no border: a part of the app nobody has asked anything of
/// should not look like it is holding something. A border is what says a press would do
/// something, so it comes up under the pointer and stays while a server is there; running
/// puts `server_bg` under it, the one colour of its own in the app, since a process the
/// reader started is worth telling apart from a toggle that happens to be on; and
/// something going on -- starting, or a server reading the project -- turns the icon into
/// a loader, the only moving thing in the bar, which says an answer is not ready rather
/// than not there.
#[derive(Clone, PartialEq)]
pub(crate) struct ServerButton;

impl Component for ServerButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let language = use_consume::<Talking>().0;
        let proj = use_consume::<Proj>().0;
        let jobs = use_consume::<LspJobs>();

        // The reads, and with them the subscriptions. Bound to lets of their own and
        // dropped here: the press below writes the very state this looked at.
        let held = language.read().clone();
        let open = proj.read().clone();
        let directory = workspace(&open);

        // With no directory there is nothing to run a server over.
        let live = directory.is_some();
        let tooltip = match live {
            true => held.words(),
            false => "The project has no directory".to_owned(),
        };

        let (side, glyph) = (toggle_size(), icon_size());
        // Dim only where a press would do nothing. Off is a control the reader is meant
        // to find, not one that is unavailable, so it is written as plainly as the two
        // buttons beside it; what says it is off is the lack of a border and a colour.
        let colour = match (&held.state, live) {
            (_, false) => dimmed(palette().icon_fg, palette().pane_bg),
            (Lsp::Failed(_), _) => palette().invalid_fg,
            _ => palette().icon_fg,
        };
        // A border under the pointer, and while there is a server to press about; none at
        // all when it is off and nothing is over it. A server that is there wears the
        // icon's own colour faded into whatever the box encloses -- a line around
        // something working should be quieter than the thing inside it, and a failure is
        // the one state that gets the full colour, being the one worth looking at.
        let edge = match (&held.state, live && hovering()) {
            (Lsp::Failed(_), _) => palette().invalid_fg,
            (Lsp::Off, false) => Color::TRANSPARENT,
            (Lsp::Off, true) => palette().hairline,
            (Lsp::Running, _) => dimmed(colour, palette().server_bg),
            _ => dimmed(colour, palette().pane_bg),
        };
        let background = match (&held.state, hovering()) {
            (Lsp::Running, _) => palette().server_bg,
            (_, true) if live => palette().toggle_hover_bg,
            _ => Color::TRANSPARENT,
        };

        TooltipContainer::new(Tooltip::new(tooltip)).child(
            rect()
                .horizontal()
                .height(Size::px(side))
                .cross_align(Alignment::Center)
                .padding(Gaps::new_symmetric(0.0, 6.0))
                .spacing(4.0)
                .corner_radius(4.0)
                .background(background)
                .border(Border::new().fill(edge).width(1.0))
                .maybe(live, |button| {
                    button
                        .on_pointer_over(move |_| hovering.set_if_modified(true))
                        .on_pointer_out(move |_| hovering.set_if_modified(false))
                        .on_press({
                            let jobs = jobs.clone();
                            move |_| {
                                // Bound to a `let` of its own: a `match` holds its
                                // scrutinee's guard to the end of the statement, and both
                                // arms below write the state it was read from.
                                let started = language.peek().started();
                                match started {
                                    true => stop_server(language, &jobs),
                                    // Which asks first where the reader has not agreed to
                                    // the directory yet.
                                    false => start_server(language, proj, &jobs),
                                }
                            }
                        })
                })
                // The same square either way, so nothing beside it moves.
                .child(
                    rect()
                        .width(Size::px(glyph))
                        .height(Size::px(glyph))
                        .center()
                        .child(match held.busy() {
                            true => CircularLoader::new().size(glyph).into_element(),
                            false => SvgViewer::new(lucide::link())
                                .width(Size::px(glyph))
                                .height(Size::px(glyph))
                                .color(colour)
                                .into_element(),
                        }),
                )
                .child(label().text(SERVER_NAME.to_owned()).color(colour)),
        )
    }
}

/// The question a start puts when the reader has not agreed to the project's directory:
/// what would be run, what running it means, and where.
///
/// Under the top bar rather than in the Project view's own section, though that section
/// is where the other Start button is: the control above is pressed from wherever the
/// reader happens to be, and a question drawn in a tab they are not looking at is a press
/// that did nothing. It is a band and not a window over the app, and it lays out as
/// nothing while there is nothing to ask.
///
/// It wears the Symbols list's surface rather than the bar's, so that a question standing
/// in front of the app is a surface of its own and not more of the bar it hangs under.
/// Both colours it writes are already held legible on that one by the contrast tests.
///
/// The directory is written out, because it is what is being agreed to.
#[derive(Clone, PartialEq)]
pub(crate) struct TrustPrompt;

impl Component for TrustPrompt {
    fn render(&self) -> impl IntoElement {
        let language = use_consume::<Talking>().0;
        let proj = use_consume::<Proj>().0;
        let jobs = use_consume::<LspJobs>();

        let asked = language.read().asking.clone();
        let Some(asking) = asked else {
            return rect().into_element();
        };

        rect()
            .width(Size::fill())
            .horizontal()
            .cross_align(Alignment::Center)
            .content(Content::Flex)
            .spacing(8.0)
            .padding(Gaps::new_symmetric(6.0, 12.0))
            .background(palette().symbol_pane_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::flex(1.0))
                    .spacing(2.0)
                    .child(
                        label()
                            .text(format!("Let {} read this directory?", asking.program))
                            .color(palette().text_fg),
                    )
                    .child(
                        label()
                            .text("It runs the project's own build scripts and macros.".to_owned())
                            .color(palette().address_fg),
                    )
                    .child(
                        label()
                            .text(asking.directory.to_string_lossy().into_owned())
                            .color(palette().address_fg)
                            .max_lines(1),
                    ),
            )
            .child(
                Button::new()
                    .on_press({
                        let jobs = jobs.clone();
                        move |_| agree_to_start(language, proj, &jobs)
                    })
                    .child("Start it"),
            )
            .child(
                Button::new()
                    .on_press(move |_| decline_start(language))
                    .child("Not now"),
            )
            .into_element()
    }
}

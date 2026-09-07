//! The language server as the app holds it: whether one is running, the worker that talks
//! to it, and the control in the top bar that starts and stops it.
//!
//! [`use_worker`]'s shape (`src/ui/worker.rs`), and for the reasons every worker has one:
//! talking to a server blocks, so it goes to a thread of its own, and it is **one** thread
//! because there is one server and one conversation with it.
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
//!
//! Neither says which **question** an answer is to. A run lasts as long as the server, so
//! two questions inside one is the ordinary case; the id [`ask_where`] mints is what an
//! asker matches its own answer by.
//!
//! The handle arrives the moment the process does and not when the handshake is over: a
//! program that reads its input and answers nothing would otherwise hold the worker in
//! that read for the life of the app, with nothing for a stop to kill.

use std::sync::atomic::{AtomicU64, Ordering};

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
    /// What the server last said about having settled, and `None` for one that has never
    /// said -- which is every server but rust-analyzer, the notification being its own
    /// (`lsp::Note::Settled`).
    pub(crate) settled: Option<bool>,
    /// The start that has been asked about and not answered yet. `None` unless the
    /// prompt is up.
    pub(crate) asking: Option<Asking>,
    /// What the project's own `.vscode/settings.json` said, or why it could not be used.
    /// `None` until the read that follows the project has answered.
    ///
    /// Read through the worker and held here rather than in the Project view: the view is
    /// a tab, unmounted while it is not the one on screen, where the control in the top bar
    /// starts a server from wherever the reader is. One read, at the root, answers both.
    settings: Option<Result<lsp::Settings, lsp::Unreadable>>,
    /// Which server the answers arriving are about, counted up by every start and every
    /// stop.
    pub(crate) run: u64,
    /// What ends the server. Held from the moment the worker says the process is there,
    /// which is before the handshake: a stop while it is starting has to reach it too.
    server: Option<process::Handle>,
}

impl PartialEq for Language {
    /// The handle is not compared: there is one server per run, so a run that has not
    /// moved is the same server.
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.run == other.run
            && self.working == other.working
            && self.settled == other.settled
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
    /// until it was answered, with every click queued behind it (`src/ui/linking.rs`) --
    /// and would be answered with as much as the server had worked out so far.
    ///
    /// **What a server says about itself beats what its progress implies.** A server that
    /// reports having settled is taken at its word; one that never does is judged by its
    /// progress, which is the old rule and is only ever a guess: the gaps between progress
    /// tokens are not readiness (`lsp::Note::Settled`).
    pub(crate) fn ready(&self) -> bool {
        matches!(self.state, Lsp::Running)
            && match self.settled {
                Some(settled) => settled,
                None => !self.working,
            }
    }

    /// Whether the app is holding what would end a server, which is a process that
    /// exists. For the tests: nothing drawn asks it.
    #[cfg(test)]
    pub(crate) fn holding(&self) -> bool {
        self.server.is_some()
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

    /// A remark from run `run`'s server: whether it is reading the project rather than
    /// answering about it. Whether anything changed, so the caller writes only then
    /// ([`write_if`]) -- and so that a server reporting the same thing twice costs no
    /// render.
    ///
    /// A remark from a server that has been stopped is about nothing the control still
    /// says.
    fn noted(&mut self, run: u64, working: bool) -> bool {
        if self.run != run || self.working == working {
            return false;
        }
        self.working = working;
        true
    }

    /// Run `run`'s server says whether it has settled -- read the project and ready to
    /// answer about it. [`Language::noted`]'s rules: whether anything changed, and a
    /// remark from a server that has been stopped says nothing.
    fn noted_settled(&mut self, run: u64, settled: bool) -> bool {
        if self.run != run || self.settled == Some(settled) {
            return false;
        }
        self.settled = Some(settled);
        true
    }

    /// Run `run`'s process exists, and `handle` is what ends it. Held from this moment
    /// and not from the end of the handshake: a stop while it is starting has to reach it
    /// too.
    ///
    /// **A handle for a server stopped while it was starting is killed here rather than
    /// dropped.** The stop found nothing to kill, so the kill is this; and dropping it
    /// would leave a server running that nothing could ever name again. The worker is in
    /// the handshake, and the pipes closing is what lets it out.
    fn spawned(&mut self, run: u64, handle: process::Handle) -> bool {
        if self.run != run {
            handle.stop();
            return false;
        }
        self.server = Some(handle);
        true
    }

    /// The handshake with run `run`'s server is over: it is answering, or `server` says
    /// why there is none. [`Language::spawned`]'s rule for a run that has moved on, for
    /// its reason -- this is the first moment anything in the app holds the handle.
    ///
    /// What the server has already said about itself is kept: the handshake's answer and
    /// its first `$/progress` are two messages, and either can be taken first.
    fn running(&mut self, run: u64, server: Result<process::Handle, lsp::Failure>) -> bool {
        if self.run != run {
            if let Ok(handle) = server {
                handle.stop();
            }
            return false;
        }
        match server {
            Ok(handle) => {
                self.state = Lsp::Running;
                self.server = Some(handle);
            }
            Err(failure) => {
                self.state = Lsp::Failed(failure.to_string());
                self.server = None;
            }
        }
        true
    }

    /// Run `run`'s server stopped answering, `why` being what it said. The one thing the
    /// control has to show, and the end of that server as far as the app is concerned.
    fn failed(&mut self, run: u64, why: String) -> bool {
        if self.run != run {
            return false;
        }
        self.state = Lsp::Failed(why);
        self.working = false;
        self.settled = None;
        self.server = None;
        true
    }

    /// What the project's own settings file said, or why it could not be used.
    fn read_settings(&mut self, settings: Result<lsp::Settings, lsp::Unreadable>) -> bool {
        let read = Some(settings);
        if self.settings == read {
            return false;
        }
        self.settings = read;
        true
    }

    /// Leaving a project takes its settings with it: they were another project's.
    fn forget_settings(&mut self) -> bool {
        if self.settings.is_none() {
            return false;
        }
        self.settings = None;
        true
    }

    /// Put the start `asking` describes to the reader. A second press with the same
    /// question up asks it again, which is nothing.
    fn ask_to_start(&mut self, asking: Asking) -> bool {
        if self.asking.as_ref() == Some(&asking) {
            return false;
        }
        self.asking = Some(asking);
        true
    }

    /// The reader declines: the question goes and **nothing is remembered**, so the next
    /// press asks again.
    fn declined(&mut self) -> bool {
        if self.asking.is_none() {
            return false;
        }
        self.asking = None;
        true
    }

    /// Start over: whatever is running is stopped, the run is counted up, and the control
    /// says it is starting. Answers with the run to start under and the settings to start
    /// it with.
    ///
    /// **A settings file that could not be read starts nothing** ([`None`]): what it would
    /// otherwise reach the server as is a name it ignores or a path that is not there, and
    /// a server reading the wrong project is worse than one that says why it did not
    /// start. Not read yet is nothing to lay over the defaults: the read follows the
    /// project, and answers long before a press can reach here.
    fn starting(&mut self) -> Option<(u64, lsp::Settings)> {
        let ready = match &self.settings {
            Some(Err(why)) => Err(why.to_string()),
            Some(Ok(settings)) => Ok(settings.clone()),
            None => Ok(lsp::Settings::none()),
        };
        if let Some(handle) = &self.server {
            handle.stop();
        }
        self.working = false;
        // A new server has said nothing about itself yet, and what the last one said is
        // about a process that is gone.
        self.settled = None;
        self.asking = None;
        self.server = None;
        self.run += 1;
        match ready {
            Ok(settings) => {
                self.state = Lsp::Starting;
                Some((self.run, settings))
            }
            Err(why) => {
                self.state = Lsp::Failed(why);
                None
            }
        }
    }

    /// Stop the server, if there is one, and put the control back where it started --
    /// which a failure still on it needs as much as a running server does. Whether there
    /// was anything to stop, which is also whether the worker has to be told.
    ///
    /// An unanswered question goes with it: it was about the project being left. The
    /// project's own settings stay, being the project's and not the server's.
    fn stopped(&mut self) -> bool {
        if matches!(self.state, Lsp::Off) && self.server.is_none() && self.asking.is_none() {
            return false;
        }
        if let Some(handle) = &self.server {
            handle.stop();
        }
        self.state = Lsp::Off;
        self.working = false;
        self.settled = None;
        self.asking = None;
        self.server = None;
        self.run += 1;
        true
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

/// A place in a source file, as the language server is asked about one: the line counts
/// from zero, as the protocol counts, and the column is a byte offset into that line, as
/// every column outside the drawing is (`src/lsp.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Lookup {
    pub(crate) file: PathBuf,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

impl Lookup {
    /// The place `column` of row `at` is, as the server is asked about one.
    ///
    /// **The one place a line is counted down for the protocol.** Every line in the app
    /// is 1-based and the protocol's is not, and two spellings of that rule drift a line
    /// apart. `column` is already a byte offset into the row, which is what a column is
    /// everywhere but the drawing (`src/lsp.rs`).
    pub(crate) fn at(at: &LinePos, column: u32) -> Lookup {
        Lookup {
            file: PathBuf::from(&*at.file),
            line: at.line.saturating_sub(1),
            column,
        }
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
        /// Where [`LspAnswer::Spawned`] goes, which is the app's own answer channel. A
        /// start is the one job with something to say before it is done.
        spawned: async_channel::Sender<LspAnswer>,
    },
    /// Read the project's own `.vscode/settings.json`. A file read blocks, so it happens
    /// here rather than on the UI thread; it is this worker's and not the build worker's
    /// because what it answers is what a start has to carry.
    ReadSettings { directory: PathBuf },
    /// What is at a place: which of the four questions is in `want` (`lsp::Question`).
    /// `id` is the question's own, minted by [`ask_where`] and copied into the answer: a
    /// run says which server was asked and nothing about which question this is.
    Ask {
        run: u64,
        id: u64,
        at: Lookup,
        want: lsp::Question,
    },
    /// What every name in one file is, which is a question about the file and not about
    /// a place in it. The file travels as the `Arc<str>` a document is named by, since
    /// that is what the answer has to be matched against.
    Tokens { run: u64, file: Arc<str> },
    /// What the name under the pointer is. A question about a place like [`LspJob::Ask`]'s
    /// four, and **not** a fifth `lsp::Question`: those are bucketed by consumer, of which
    /// this is a third, and a pointer crossing a name must neither take back a definition
    /// the reader clicked for nor be taken back by one.
    Hover { run: u64, id: u64, at: Lookup },
    /// The app is showing this file, or has stopped showing it. Not a question: the
    /// server answers neither, and what they change is what every other question about
    /// the file is answered out of (`lsp::Talk::opened`).
    ///
    /// The text is not carried: the file is read on the worker, which is the thread that
    /// may block, and read at all only for a server that takes documents. The language is,
    /// since what a file is told to be is the project's to say (`src/ui/linking.rs`).
    Opened {
        run: u64,
        file: Arc<str>,
        language: String,
    },
    /// The other half, and the one job that names **no run**: a job's run is what stamps
    /// the answer it comes back as, and a close is answered with nothing.
    Closed { file: Arc<str> },
    /// Let go of the server: it has been stopped already, and this is what reaps it.
    Stop,
}

/// What came of it. Every answer names the run it is about, and one whose run has moved
/// on is not the answer to any question anybody still has.
pub(crate) enum LspAnswer {
    /// The process exists. Sent from inside the `Start` job and before the handshake,
    /// which is what puts the handle where a stop can reach it: until this the worker is
    /// in a read that only the pipes closing ends, and the pipes close with the process.
    Spawned { run: u64, handle: process::Handle },
    Started {
        run: u64,
        server: Result<process::Handle, lsp::Failure>,
    },
    /// What one question about a place came back with. Which question it was is the
    /// [`Reply`]'s to say; `id` is the [`LspJob::Ask`]'s, carried through untouched.
    Answered { run: u64, id: u64, reply: Reply },
    /// What every name in one file is, and which file. Its own answer and not a `Reply`,
    /// since it is the one question about a file rather than about a place in one.
    Linked {
        run: u64,
        file: Arc<str>,
        links: Result<links::Links, lsp::Failure>,
    },
    /// The server has been told about a file, so what it said about that file before is
    /// what it could work out without it. Not an answer to a question -- an opening is
    /// not one -- but the same shape, since what it does is put the question again.
    Reopened { run: u64, file: Arc<str> },
    /// What the server says the name at one place is. Its own answer and not a `Reply`,
    /// for the reason the links are: it is contents and a range where the four are places.
    Hovered {
        run: u64,
        id: u64,
        said: Result<Option<lsp::Hovered>, lsp::Failure>,
    },
    /// What the project's own settings file said. Named by the directory it was read in
    /// and not by a run: it is about a project and not about a process.
    Settings {
        directory: PathBuf,
        settings: Result<lsp::Settings, lsp::Unreadable>,
    },
}

/// What an answer holds, which is what was asked for: one variant per consumer, each
/// carrying the shape that consumer takes.
///
/// The kind is the variant and not a field beside it, so a question of one kind cannot
/// come back as the other's answer, and neither consumer needs an arm for one that did.
/// Both carry what the lines they name say, since the lines are **read on the worker**:
/// the read blocks, and that is the thread that may block. A followed answer's is the
/// caret its opening plants ([`Arrival`]); a listed one's is the text every row draws.
pub(crate) enum Reply {
    Followed(Result<Vec<Arrival>, lsp::Failure>),
    Listed(Result<references::References, lsp::Failure>),
}

/// The answer to `want`, out of what the server said. The one place the shape of an
/// answer is decided, and it is decided by the question.
///
/// `read` is how a named file's text is got, [`source::read_text`] on the worker: a path a
/// server answers with is file input, and two rules for what a source file is would be two
/// ideas of which files this app can show.
pub(crate) fn replied(
    want: lsp::Question,
    places: Result<Vec<lsp::Place>, lsp::Failure>,
    read: impl Fn(&Path) -> Option<String>,
) -> Reply {
    match want {
        // The caret each place opens on, counted into the pane's units off the line it is
        // on, which is read here.
        lsp::Question::Followed(_) => {
            Reply::Followed(places.map(|places| Arrival::of(places, read)))
        }
        // Grouped and their lines read with the ask, since that is what the panel draws.
        lsp::Question::Listed(_) => {
            Reply::Listed(places.map(|places| references::of(&places, read)))
        }
    }
}

/// Put one question to the server there is, and let go of a conversation that has ended.
///
/// [`None`] with no server: there is nobody to ask, so there is no answer to send. A
/// [`lsp::Failure::Broken`] is the conversation itself ending, so the server is dropped
/// here -- the one place that decision is made -- and reported as the failure it is. Every
/// job that says anything to a server goes through this, so a question added later cannot
/// leave a dead conversation in `talking` for the next one to fail against.
fn asked<T>(
    talking: &mut Option<lsp::Server>,
    ask: impl FnOnce(&mut lsp::Server) -> Result<T, lsp::Failure>,
) -> Option<Result<T, lsp::Failure>> {
    let answer = ask(talking.as_mut()?);
    if matches!(answer, Err(lsp::Failure::Broken(_))) {
        *talking = None;
    }
    Some(answer)
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
                spawned,
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
                        // Before the handshake, which a program that reads its input and
                        // answers nothing never returns from. What ends that read is the
                        // pipes closing, so the app has to be holding the handle by then
                        // or a stop has nothing to press against.
                        let _ = spawned.send_blocking(LspAnswer::Spawned {
                            run,
                            handle: handle.clone(),
                        });
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
            LspJob::Ask { run, id, at, want } => {
                let places = asked(&mut talking, |talk| {
                    talk.places(want, &at.file, at.line, at.column)
                })?;
                Some(LspAnswer::Answered {
                    run,
                    id,
                    reply: replied(want, places, source::read_text),
                })
            }
            LspJob::Tokens { run, file } => {
                // Classified here rather than on the UI thread: it is a walk of every
                // name in the file, and this is the thread that may take its time. Done
                // while the conversation is still in hand, the legend being its.
                let links = asked(&mut talking, |talk| {
                    talk.semantic_tokens(Path::new(&*file))
                        .map(|tokens| links::Links::of(talk.legend(), &tokens))
                })?;
                Some(LspAnswer::Linked { run, file, links })
            }
            LspJob::Hover { run, id, at } => {
                let said = asked(&mut talking, |talk| {
                    talk.hover(&at.file, at.line, at.column)
                })?;
                Some(LspAnswer::Hovered { run, id, said })
            }
            LspJob::Opened {
                run,
                file,
                language,
            } => {
                let path = PathBuf::from(&*file);
                let told = asked(&mut talking, |talk| {
                    // Asked before the file is read: a server that takes no documents is
                    // one this reads nothing for.
                    if !talk.opens() {
                        return Ok(false);
                    }
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        return Ok(false);
                    };
                    talk.opened(&path, &language, &text).map(|()| true)
                })?;
                // Everything the server said about this file before it had it is what it
                // could work out from the disk, which may have been nothing at all.
                matches!(told, Ok(true)).then_some(LspAnswer::Reopened { run, file })
            }
            LspJob::Closed { file } => {
                asked(&mut talking, |talk| talk.closed(Path::new(&*file)));
                None
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

/// Whose answer a job is, for [`superseded_as`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    Following,
    Listing,
    Linking,
    Hovering,
    Settings,
}

/// Which consumer a job's answer is for, and [`None`] for a job that is never dropped.
///
/// **A kind is a consumer and not a question**: `ui::follow` takes a definition or a
/// declaration, never both at once, and the Locations panel draws implementations or
/// references in the one place. A hover is a third consumer and not a fifth question --
/// the pointer crossing a line asks about every name on the way, and only the one it came
/// to rest on is worth a round trip, but none of them is a reader taking back the
/// definition they clicked for. The source pane holds one file's links; a directory typed
/// a letter at a time asks for the project's settings once a keystroke, and only the last
/// of those is about the project that is open.
///
/// The `match` is exhaustive on purpose, and this is the only place the rule is written:
/// a job added with nothing said about superseding would otherwise queue behind every one
/// of its own kind in silence.
fn superseded_as(job: &LspJob) -> Option<Kind> {
    match job {
        LspJob::Ask {
            want: lsp::Question::Followed(_),
            ..
        } => Some(Kind::Following),
        LspJob::Ask {
            want: lsp::Question::Listed(_),
            ..
        } => Some(Kind::Listing),
        LspJob::Tokens { .. } => Some(Kind::Linking),
        LspJob::Hover { .. } => Some(Kind::Hovering),
        LspJob::ReadSettings { .. } => Some(Kind::Settings),
        // Never dropped. The two documents are not questions: they are the difference
        // between what the server holds and what the reader has open, and a dropped one
        // leaves the two disagreeing for good. A start and a stop are what the reader
        // pressed.
        LspJob::Opened { .. } | LspJob::Closed { .. } | LspJob::Start { .. } | LspJob::Stop => None,
    }
}

/// The jobs worth doing, of the one taken off the channel and everything queued behind it.
///
/// Only the last question **of each kind** is kept: a reader clicking twice wants the
/// second answer, and the first is a conversation the second would only wait behind -- but
/// a reader who asks for a name's references has not taken back the definition they asked
/// for, and the two are answered by different parts of the app. What a kind is,
/// [`superseded_as`] says.
pub(crate) fn worth_doing(first: LspJob, queued: impl Iterator<Item = LspJob>) -> Vec<LspJob> {
    let jobs: Vec<LspJob> = std::iter::once(first).chain(queued).collect();
    let mut last: HashMap<Kind, usize> = HashMap::new();
    for (at, job) in jobs.iter().enumerate() {
        if let Some(kind) = superseded_as(job) {
            last.insert(kind, at);
        }
    }
    jobs.into_iter()
        .enumerate()
        .filter(|(at, job)| superseded_as(job).is_none_or(|kind| last.get(&kind) == Some(at)))
        .map(|(_, job)| job)
        .collect()
}

/// How the control reaches the worker, and how the server reaches the control.
#[derive(Clone)]
pub(crate) struct LspJobs {
    jobs: Requests<LspJob>,
    /// Handed to each server started, so what it says while nothing was asked arrives
    /// under the run it was started in.
    notes: async_channel::Sender<(u64, lsp::Note)>,
    /// The answers channel, handed to each start so the worker can say the process is
    /// there before it has finished shaking hands with it.
    spawned: async_channel::Sender<LspAnswer>,
    /// What the next question is numbered. One counter for every question put, so no two
    /// of them are ever asked under one id.
    asked: Arc<AtomicU64>,
}

impl LspJobs {
    /// The id the next question goes out under. Never handed out twice, which is what
    /// lets an answer name the question it is to and not merely the server it came from.
    fn next_question(&self) -> u64 {
        self.asked.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn send(&self, job: LspJob) {
        self.jobs.send(job);
    }
}

/// A result taken apart, so an answer can be handed on and the failure behind it kept:
/// what came back, and why nothing did.
fn split<T>(reply: Result<T, lsp::Failure>) -> (Option<T>, Option<lsp::Failure>) {
    match reply {
        Ok(answer) => (Some(answer), None),
        Err(why) => (None, Some(why)),
    }
}

/// The language server as a component sees it.
#[derive(Clone, Copy)]
pub(crate) struct Talking(pub(crate) State<Language>);

/// Start the worker and keep the state in step with it. Called once, at the root.
pub(crate) fn use_language_with(
    language: State<Language>,
    follow: State<Follow>,
    located: State<Located>,
    linked: State<Linked>,
    hover: State<Hover>,
    mut proj: State<OpenProject>,
    work: impl Fn(LspJob) -> Option<LspAnswer> + Send + 'static,
) -> LspJobs {
    // What a server says while nothing was asked, under the run it was started in. A
    // channel and a task of their own beside the worker's answers, and bounded: a server
    // that reports progress in a tight loop is one the app can fall behind, and the
    // reader thread waiting is the only backpressure there is.
    let told = use_hook(move || {
        let (told, notes) = async_channel::bounded::<(u64, lsp::Note)>(64);
        spawn(async move {
            while let Ok((run, note)) = notes.recv().await {
                match note {
                    lsp::Note::Busy(working) => {
                        let noted = write_if(language, |held| held.noted(run, working));
                        // A server that has gone quiet has read more of the project than
                        // it had when it refused a question about a file's names, so that
                        // question is put again. Here and not on every word it says: a
                        // server that goes on refusing would otherwise be asked in a tight
                        // loop. A server that says when it has settled says so below
                        // instead, and better.
                        if noted && !working {
                            write_if(linked, |waiting| waiting.forget_refusal());
                        }
                    }
                    lsp::Note::Settled(settled) => {
                        let noted = write_if(language, |held| held.noted_settled(run, settled));
                        // Everything asked before this was asked of a server still reading
                        // the project, and what it answered about a file's names was as
                        // far as it had got: fewer names, and some of them the wrong kind.
                        // So the answer is dropped and the question put again, which is
                        // the whole reason this notification is asked for.
                        if noted && settled {
                            write_if(linked, |waiting| waiting.forget_answer());
                        }
                    }
                }
            }
        });
        told
    });

    // A `std::thread` and not a spawned task: a server that is reading a project can take
    // a minute to answer, and freya's executor is the UI thread. The answer sender comes
    // back with the way to ask, for the one answer a job sends before it is finished.
    let (requests, spawning) = use_worker_answering(
        "the language server's worker",
        // Only the last question of each consumer, which is what `worth_doing` is.
        |job, queued, _| worth_doing(job, std::iter::from_fn(queued)),
        work,
        move |answer, _| match answer {
            LspAnswer::Spawned { run, handle } => {
                write_if(language, |held| held.spawned(run, handle));
            }
            LspAnswer::Started { run, server } => {
                write_if(language, |held| held.running(run, server));
            }
            LspAnswer::Settings {
                directory,
                settings,
            } => {
                // A file read for a project that has since been left says nothing about
                // the one that is open now. The project and not the server is what this
                // answer is about, which is why the run says nothing about it.
                let open = workspace(&proj.peek());
                if open.as_deref() != Some(directory.as_path()) {
                    return;
                }
                write_if(language, |held| held.read_settings(settings));
            }
            LspAnswer::Linked { run, file, links } => {
                // Bound to a `let` of its own, the writes below being of this state.
                let mine = language.peek().run == run;
                if !mine {
                    return;
                }
                // Nothing found and a server that refused both leave the pane with no
                // links, which is what it draws with no server either: there is nothing
                // to say about a name nobody classified. The two are still told apart: a
                // refusal is a question to put again, and an empty answer is the answer.
                let why = match links {
                    Ok(links) => {
                        write_if(linked, |waiting| waiting.answer(run, file, links));
                        return;
                    }
                    Err(failure @ lsp::Failure::Refused { .. }) => {
                        log::warn!("the language server refused a question: {failure}");
                        write_if(linked, |waiting| waiting.answer_refused(run, file));
                        return;
                    }
                    Err(failure) => failure,
                };
                write_if(language, |held| held.failed(run, why.to_string()));
            }
            LspAnswer::Reopened { run, file } => {
                // Bound to a `let` of its own, the write below being of another state.
                let mine = language.peek().run == run;
                if !mine {
                    return;
                }
                // What was said about the file before the server had it is what it could
                // work out from the disk, which is nothing until its own scan reaches the
                // file.
                write_if(linked, |waiting| waiting.forget_file(&file));
            }
            LspAnswer::Hovered { run, id, said } => {
                // Bound to a `let` of its own, the writes below being of this state.
                let mine = language.peek().run == run;
                if !mine {
                    return;
                }
                // A refusal is already no answer by the time it is here
                // (`lsp::Talk::hover`), so what is left is a name the server had nothing
                // to say about -- no box, and no question to put again -- or a
                // conversation that ended.
                let why = match said {
                    Ok(said) => {
                        write_if(hover, |waiting| {
                            waiting.answer(run, id, said.map(|said| said.text))
                        });
                        return;
                    }
                    Err(failure) => failure,
                };
                // The question is dropped either way: a box that stayed asked would keep
                // the name from ever being asked about again.
                write_if(hover, |waiting| waiting.answer(run, id, None));
                write_if(language, |held| held.failed(run, why.to_string()));
            }
            LspAnswer::Answered { run, id, reply } => {
                // An answer from a server that has been stopped is an answer to nobody.
                // Bound to a `let` of its own, the writes below being of this state.
                let mine = language.peek().run == run;
                if !mine {
                    return;
                }
                // Whoever asked takes the answer, and gives up on it where there is none.
                // The reply's own shape says which of them, so neither can be handed the
                // other's. An answer naming nowhere is an answer: the click was a
                // question, not a promise.
                let why = match reply {
                    Reply::Followed(reply) => {
                        let (places, why) = split(reply);
                        write_if(follow, |waiting| match &places {
                            Some(places) => waiting.answer(run, id, places),
                            None => waiting.give_up(run, id),
                        });
                        why
                    }
                    Reply::Listed(reply) => {
                        let (found, why) = split(reply);
                        // Nothing found and nothing to be found both leave the panel
                        // saying so: a question that stayed pending would say it was
                        // still looking for ever.
                        write_if(located, |waiting| {
                            waiting.answer_places(run, id, found.unwrap_or_default())
                        });
                        why
                    }
                };
                let Some(why) = why else {
                    return;
                };
                // The server refused the question -- it is still reading the project, or
                // has no such file of its own. Nothing found, and not a server to say
                // anything about: it is answering.
                if matches!(why, lsp::Failure::Refused { .. }) {
                    log::warn!("the language server refused a question: {why}");
                    return;
                }
                // What is left is a server that stopped answering, which is the one thing
                // the control has to show.
                write_if(language, |held| held.failed(run, why.to_string()));
            }
        },
    );

    // A context, because the control that presses it is drawn from a component that is
    // handed nothing; returned as well, so a test can ask directly. The counter is made
    // here, and once: no two questions may go out under one id.
    let jobs = use_provide_context(move || LspJobs {
        jobs: requests,
        notes: told,
        spawned: spawning,
        asked: Arc::new(AtomicU64::new(0)),
    });

    // Leaving a project ends its server and takes the reader's agreement with it: it is
    // the project's directory the server was started over and the directory they agreed
    // to, and a directory typed into the Project view is a different project's on both
    // counts. The two reads are bound first, since the stop below writes the state this
    // effect is about.
    let open = proj.read().clone();
    let deps = (open.file.clone(), workspace(&open));
    // What the effect last saw, so that it can tell the two changes apart. A directory
    // typed into the box is the reader pointing *this* project somewhere else, and the
    // agreement was to the old place; a project arriving is another project's answer
    // arriving with it, and that answer is its own to give. The mount is neither: what it
    // mounts with is the reopened project, the restore being an earlier hook of the same
    // render, so an agreement read out of `project.toml` survives the launch that read it.
    let seen: Rc<RefCell<Option<(Option<PathBuf>, Option<PathBuf>)>>> =
        use_hook(|| Rc::new(RefCell::new(None)));
    use_side_effect_with_deps(&deps, {
        let jobs = jobs.clone();
        move |(file, directory): &(Option<PathBuf>, Option<PathBuf>)| {
            stop_server(language, &jobs);
            // And the settings go with it: they were another project's. Read again here,
            // where a project arrives, so the answer is in hand before either press can
            // ask for a server and whether or not one is ever started -- the Project view
            // lists them either way.
            write_if(language, |held| held.forget_settings());
            if let Some(directory) = directory.clone() {
                jobs.send(LspJob::ReadSettings { directory });
            }
            let before = seen.replace(Some((file.clone(), directory.clone())));
            let moved = before.is_some_and(|(was_file, was_directory)| {
                was_file == *file && was_directory != *directory
            });
            if moved {
                proj.write().trusted = false;
            }
        }
    });

    jobs
}

/// Start the project's server over the project's directory -- or, where the reader has
/// not agreed to that directory, put the question and start nothing.
///
/// The project rather than a directory and a program: both presses that reach here are
/// about the project that is open, and what is asked about has to be what would run.
pub(crate) fn start_server(language: State<Language>, proj: State<OpenProject>, jobs: &LspJobs) {
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
    write_if(language, |held| held.ask_to_start(asking));
}

/// Start what was asked for, leaving whatever was running stopped.
///
/// **A settings file that could not be read starts nothing.** What it would otherwise
/// reach the server as is a name it ignores or a path that is not there, and a server
/// reading the wrong project is worse than one that says why it did not start. The check
/// is here because this is where a start happens, so neither press nor the agreement can
/// grow a path around it -- the same reason the trust gate is in `start_server`.
fn run_server(mut language: State<Language>, jobs: &LspJobs, asking: Asking) {
    // Bound before the write, as ever; the start it answers with is sent after it.
    let mut next = language.peek().clone();
    let starting = next.starting();
    language.set(next);
    let Some((run, settings)) = starting else {
        return;
    };
    jobs.send(LspJob::Start {
        run,
        directory: asking.directory,
        program: asking.program,
        settings,
        notes: jobs.notes.clone(),
        spawned: jobs.spawned.clone(),
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
pub(crate) fn decline_start(language: State<Language>) {
    write_if(language, |held| held.declined());
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
/// the job. A server that is still starting is killed the same way -- the handle is the
/// worker's the moment the process exists ([`LspAnswer::Spawned`]) and not the handshake's
/// -- and the `Stop` behind it reaches a worker that is out of the read rather than one
/// parked in it for good.
pub(crate) fn stop_server(language: State<Language>, jobs: &LspJobs) {
    // The worker is told only where there was something to stop.
    if write_if(language, |held| held.stopped()) {
        jobs.send(LspJob::Stop);
    }
}

/// Ask `want` about the place `at`. The answer is the worker's, and arrives under the run
/// it was asked in and the id minted here, which is what this hands back: a run tells one
/// server from another, and the id tells this question from the next one the same caller
/// puts.
///
/// `None` with no server: there is nobody to ask, and a question is not what starts one --
/// that is the control, and only the reader presses it. One that is still starting is
/// asked all the same, the question queueing behind the start to be answered once there is
/// somebody to answer it, and finding nothing to talk to if the start failed.
pub(crate) fn ask_where(
    language: State<Language>,
    jobs: &LspJobs,
    at: Lookup,
    want: lsp::Question,
) -> Option<(u64, u64)> {
    let held = language.peek().clone();
    if !held.started() {
        return None;
    }
    let id = jobs.next_question();
    jobs.send(LspJob::Ask {
        run: held.run,
        id,
        at,
        want,
    });
    Some((held.run, id))
}

/// Ask what the name at `at` is. [`ask_where`]'s rules, in every respect: the answer
/// arrives under the run it was asked in and the id minted here, there is nobody to ask
/// with no server, and a question put while one is starting waits for it.
pub(crate) fn ask_hover(
    language: State<Language>,
    jobs: &LspJobs,
    at: Lookup,
) -> Option<(u64, u64)> {
    let held = language.peek().clone();
    if !held.started() {
        return None;
    }
    let id = jobs.next_question();
    jobs.send(LspJob::Hover {
        run: held.run,
        id,
        at,
    });
    Some((held.run, id))
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

#[cfg(test)]
mod tests;

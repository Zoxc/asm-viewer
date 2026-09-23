//! The language server as the app holds it: whether one is running, and the hook and the
//! presses that start it, stop it and put questions to it.
//!
//! Two parts of it are files of their own: `language/worker.rs` is what runs on the worker
//! thread, the only part that names `lsp::Server`, and `language_view.rs` is what the
//! reader presses.
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
//! two questions inside one is the ordinary case; the run and the id [`ask_where`] mints
//! travel together as a [`Ticket`], which is what an asker matches its own answer by.
//!
//! The handle arrives the moment the process does and not when the handshake is over: a
//! program that reads its input and answers nothing would otherwise hold the worker in
//! that read for the life of the app, with nothing for a stop to kill.

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

mod worker;

/// The blocking half, which the hook below drives and everything else asks through.
pub(crate) use worker::*;

/// Where the language server is, and what having got there brings with it: the process
/// to end, and what the server has said about itself. Only two of the four states have a
/// server, so neither can be written down beside one that has none.
#[derive(Clone, Default)]
pub(crate) enum Lsp {
    #[default]
    Off,
    /// Asked for, and not yet answering: starting one takes a moment and reading the
    /// project takes longer. `server` is [`None`] until the worker says the process is
    /// there, which is before the handshake is over.
    Starting {
        server: Option<process::Handle>,
        said: Remarks,
        serving: Serving,
    },
    /// Answering, and `server` is what ends it.
    Running {
        server: process::Handle,
        said: Remarks,
        serving: Serving,
    },
    /// It could not be started, or it stopped answering. What it says is the reason,
    /// which the control shows and nothing else does.
    Failed(String),
}

/// Two states are the same state when they are the same kind of state and the server has
/// said the same things. The handle is not compared: there is one server per run, so a
/// run that has not moved is the same server.
impl PartialEq for Lsp {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Lsp::Off, Lsp::Off) => true,
            (Lsp::Starting { said: ours, .. }, Lsp::Starting { said: theirs, .. })
            | (Lsp::Running { said: ours, .. }, Lsp::Running { said: theirs, .. }) => {
                ours == theirs
            }
            (Lsp::Failed(ours), Lsp::Failed(theirs)) => ours == theirs,
            _ => false,
        }
    }
}

/// Written without the handle: a process is nothing to print, and has no `Debug` of its
/// own.
impl std::fmt::Debug for Lsp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lsp::Off => write!(f, "Off"),
            Lsp::Starting { said, .. } => write!(f, "Starting({said:?})"),
            Lsp::Running { said, .. } => write!(f, "Running({said:?})"),
            Lsp::Failed(why) => write!(f, "Failed({why:?})"),
        }
    }
}

/// What a server has said about itself while nothing was asked (`lsp::Note`). Kept across
/// the handshake: what it says and the handshake's own answer are two messages, and
/// either can arrive first.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Remarks {
    /// Whether it is reading the project rather than answering about it.
    pub(crate) working: bool,
    /// What the server last said about having settled, and `None` for one that has never
    /// said -- which is every server but rust-analyzer, the notification being its own
    /// (`lsp::Note::Settled`).
    pub(crate) settled: Option<bool>,
}

impl Remarks {
    /// Whether the server is done reading the project and ready to answer about it.
    ///
    /// **What a server says about itself beats what its progress implies.** A server that
    /// reports having settled is taken at its word; one that never does is judged by its
    /// progress, which is the old rule and is only ever a guess: the gaps between progress
    /// tokens are not readiness (`lsp::Note::Settled`).
    fn ready(&self) -> bool {
        match self.settled {
            Some(settled) => settled,
            None => !self.working,
        }
    }

    /// Take a remark: write the field it names, and answer whether that moved anything.
    ///
    /// The one place a note is turned into what is held of it, so a new `lsp::Note`
    /// variant is an arm here and not a second writer with the same two guards in front
    /// of it ([`Language::remarked`]).
    fn take(&mut self, note: &lsp::Note) -> bool {
        match *note {
            lsp::Note::Busy(working) => {
                let moved = self.working != working;
                self.working = working;
                moved
            }
            lsp::Note::Settled(settled) => {
                let moved = self.settled != Some(settled);
                self.settled = Some(settled);
                moved
            }
        }
    }
}

impl Lsp {
    /// What the server has said about itself, and nothing where there is no server.
    pub(crate) fn said(&self) -> Option<&Remarks> {
        match self {
            Lsp::Starting { said, .. } | Lsp::Running { said, .. } => Some(said),
            Lsp::Off | Lsp::Failed(_) => None,
        }
    }

    /// The same, to write on. A remark about a server the app no longer has is about
    /// nothing it still says.
    fn said_mut(&mut self) -> Option<&mut Remarks> {
        match self {
            Lsp::Starting { said, .. } | Lsp::Running { said, .. } => Some(said),
            Lsp::Off | Lsp::Failed(_) => None,
        }
    }

    /// What the server was started for, where there is one.
    pub(crate) fn serving(&self) -> Option<&Serving> {
        match self {
            Lsp::Starting { serving, .. } | Lsp::Running { serving, .. } => Some(serving),
            Lsp::Off | Lsp::Failed(_) => None,
        }
    }

    /// What ends the server, where there is one to end.
    fn handle(&self) -> Option<&process::Handle> {
        match self {
            Lsp::Starting { server, .. } => server.as_ref(),
            Lsp::Running { server, .. } => Some(server),
            Lsp::Off | Lsp::Failed(_) => None,
        }
    }
}

/// What a server is started as: the program, and the extensions the project named for
/// it.
///
/// Read off the Project view's boxes at the press and held while the server runs. What
/// the boxes say after that is for the next start, so typing in them opens and closes
/// nothing with the server that is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Serving {
    pub(crate) program: String,
    pub(crate) files: Vec<String>,
}

/// A start the reader has not agreed to yet: what would be run, and where.
///
/// Held rather than worked out again when they answer: the question named a directory,
/// and the agreement is to that one and not to whatever the box says by then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asking {
    pub(crate) directory: PathBuf,
    pub(crate) serving: Serving,
}

/// The language server as the app holds it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Language {
    pub(crate) state: Lsp,
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
}

impl Language {
    /// Whether pressing the control stops it rather than starting it.
    pub(crate) fn started(&self) -> bool {
        matches!(self.state, Lsp::Starting { .. } | Lsp::Running { .. })
    }

    /// Whether it is there to be asked a question about a whole file: running, and done
    /// reading the project. A question put before that would hold the one conversation
    /// until it was answered, with every click queued behind it (`src/ui/linking.rs`) --
    /// and would be answered with as much as the server had worked out so far. What being
    /// done reading amounts to is [`Remarks::ready`].
    pub(crate) fn ready(&self) -> bool {
        matches!(&self.state, Lsp::Running { said, .. } if said.ready())
    }

    /// The run to put a question under, where a server is started: [`started`]'s answer
    /// and the `u64` with it, and all most readers want of the state.
    ///
    /// [`started`]: Language::started
    pub(crate) fn current(&self) -> Option<u64> {
        self.started().then_some(self.run)
    }

    /// The run to ask about a whole file, where it is [`ready`].
    ///
    /// [`ready`]: Language::ready
    pub(crate) fn answering(&self) -> Option<u64> {
        self.ready().then_some(self.run)
    }

    /// Whether something is going on: starting one, or a server reading the project.
    /// What the control draws a turning loader for instead of its own icon.
    pub(crate) fn busy(&self) -> bool {
        match &self.state {
            Lsp::Starting { .. } => true,
            Lsp::Running { said, .. } => said.working,
            Lsp::Off | Lsp::Failed(_) => false,
        }
    }

    /// What the Project view says about it.
    ///
    /// `directory` is the one a server would be run over, which the state knows nothing
    /// of and which is the reason there is none all the same.
    pub(crate) fn verdict(&self, directory: Option<&Path>) -> Verdict {
        if directory.is_none() {
            return Verdict::plain("No directory");
        }
        match &self.state {
            Lsp::Off => Verdict::plain("Not running. The control in the top bar starts it."),
            Lsp::Starting { .. } => Verdict::plain("Starting..."),
            Lsp::Running { said, .. } if said.working => Verdict::plain("Reading the project..."),
            Lsp::Running { .. } => Verdict::plain("Running"),
            Lsp::Failed(why) => Verdict::bad_news(why.clone()),
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

    /// A remark from run `run`'s server, written on what the app holds of that server:
    /// whether it is reading the project, or whether it has settled. Which field a
    /// remark writes is [`Remarks::take`]'s; the two guards here are every remark's.
    ///
    /// Answers whether anything changed, so the caller writes only then ([`write_if`]) --
    /// and so that a server reporting the same thing twice costs no render.
    ///
    /// A remark from a server that has been stopped, or that stopped answering, is about
    /// nothing the control still says: the state it would be written on is gone.
    fn remarked(&mut self, run: u64, note: &lsp::Note) -> bool {
        if self.run != run {
            return false;
        }
        let Some(said) = self.state.said_mut() else {
            return false;
        };
        said.take(note)
    }

    /// Run `run`'s process exists, and `handle` is what ends it. Held from this moment
    /// and not from the end of the handshake: a stop while it is starting has to reach it
    /// too.
    ///
    /// **A handle for a server stopped while it was starting is killed here rather than
    /// dropped.** The stop found nothing to kill, so the kill is this; and dropping it
    /// would leave a server running that nothing could ever name again. The worker is in
    /// the handshake, and the pipes closing is what lets it out. A handle for anything but
    /// the start that is under way goes the same way, for the same reason.
    fn spawned(&mut self, run: u64, handle: process::Handle) -> bool {
        let starting = self.run == run;
        let Lsp::Starting { server, .. } = &mut self.state else {
            handle.stop();
            return false;
        };
        if !starting {
            handle.stop();
            return false;
        }
        *server = Some(handle);
        true
    }

    /// The handshake with run `run`'s server is over: it is answering, or `started` says
    /// why there is none. Nothing to stop for a run that has moved on --
    /// [`Language::spawned`] holds the only handle and its rule covers it.
    ///
    /// The handle is the one [`Language::spawned`] wrote, which is there by now: both
    /// answers come down the one channel and that one is sent first. Without it there is
    /// a process nothing in the app can ever end, so the reading that says so is the one
    /// taken.
    ///
    /// What the server has already said about itself is kept: the handshake's answer and
    /// its first `$/progress` are two messages, and either can be taken first.
    fn running(&mut self, run: u64, started: Result<(), lsp::Failure>) -> bool {
        if self.run != run {
            return false;
        }
        let said = self.state.said().cloned().unwrap_or_default();
        let handle = match &mut self.state {
            Lsp::Starting {
                server, serving, ..
            } => server.take().map(|server| (server, serving.clone())),
            _ => None,
        };
        self.state = match (started, handle) {
            (Ok(()), Some((server, serving))) => Lsp::Running {
                server,
                said,
                serving,
            },
            (Ok(()), None) => Lsp::Failed("it started with no handle to end it".to_owned()),
            (Err(failure), _) => Lsp::Failed(failure.to_string()),
        };
        true
    }

    /// Run `run`'s server stopped answering, `why` being what it said. The one thing the
    /// control has to show, and the end of that server as far as the app is concerned.
    fn failed(&mut self, run: u64, why: String) -> bool {
        if self.run != run {
            return false;
        }
        self.state = Lsp::Failed(why);
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
    /// says it is starting `serving`. Answers with the run to start under and the settings
    /// to start it with.
    ///
    /// **A settings file that could not be read starts nothing** ([`None`]): what it would
    /// otherwise reach the server as is a name it ignores or a path that is not there, and
    /// a server reading the wrong project is worse than one that says why it did not
    /// start. Not read yet is nothing to lay over the defaults: the read follows the
    /// project, and answers long before a press can reach here.
    fn starting(&mut self, serving: Serving) -> Option<(u64, lsp::Settings)> {
        let ready = match &self.settings {
            Some(Err(why)) => Err(why.to_string()),
            Some(Ok(settings)) => Ok(settings.clone()),
            None => Ok(lsp::Settings::none()),
        };
        if let Some(handle) = self.state.handle() {
            handle.stop();
        }
        self.asking = None;
        self.run += 1;
        match ready {
            Ok(settings) => {
                // A new server has said nothing about itself yet, and what the last one
                // said went with the state it was written on.
                self.state = Lsp::Starting {
                    server: None,
                    said: Remarks::default(),
                    serving,
                };
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
        if matches!(self.state, Lsp::Off) && self.asking.is_none() {
            return false;
        }
        if let Some(handle) = self.state.handle() {
            handle.stop();
        }
        self.state = Lsp::Off;
        self.asking = None;
        self.run += 1;
        true
    }

    /// What the control says on hover: the state, in words, and the reason when there is
    /// one.
    ///
    /// `program` is the server the project named ([`OpenProject::server`]): the control
    /// says only `LSP`, so the tooltip is where the program is spelled out, and a project
    /// on a toolchain of its own must not be told to start Rust's. A server that is started
    /// is named by what it was started as instead, since the box may have been typed into
    /// since. `directory` is the one it would be run over, as in [`Language::verdict`];
    /// with none there is nothing to press.
    pub(crate) fn words(&self, program: &str, directory: Option<&Path>) -> String {
        if directory.is_none() {
            return "The project has no directory".to_owned();
        }
        let program = self
            .state
            .serving()
            .map_or(program, |serving| &serving.program);
        match &self.state {
            Lsp::Off => format!("Start {program}"),
            Lsp::Starting { .. } => format!("Starting {program}"),
            Lsp::Running { said, .. } if said.working => {
                format!("{program} is reading the project")
            }
            Lsp::Running { .. } => format!("Stop {program}"),
            Lsp::Failed(why) => why.clone(),
        }
    }
}

impl Lookup {
    /// The place `column` of row `at` is, as the server is asked about one.
    ///
    /// **The one way the app builds a question about a place**: a `LinePos` is the UI's
    /// own, which is why this sits here and the type it makes sits with the rest of the
    /// protocol. Both units go through untouched -- the line is 1-based on either side
    /// (`lsp::Lookup`), and `column` is already a byte offset into the row, which is what
    /// a column is everywhere.
    pub(crate) fn at(at: &LinePos, column: usize) -> Lookup {
        Lookup {
            file: at.file.to_path_buf(),
            line: at.line,
            column,
        }
    }
}

/// One question put to one server: the run it was asked in, and its own number.
///
/// The two are always carried together and neither answers on its own. The run says which
/// server, and a run lasts as long as that server, so two questions inside one is the
/// ordinary case; the id, minted once per question ([`LspJobs::ticket`]), says which. A
/// job carries one, its answer carries it back, and whoever is waiting holds the one they
/// are waiting for -- so "is this mine" is `== ticket` and not a pair of fields compared
/// by hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Ticket {
    pub(crate) run: u64,
    pub(crate) id: u64,
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
    /// The [`Ticket`] the next question of run `run` goes out under. The id is never
    /// handed out twice, which is what lets an answer name the question it is to and not
    /// merely the server it came from.
    fn ticket(&self, run: u64) -> Ticket {
        Ticket {
            run,
            id: self.asked.fetch_add(1, Ordering::Relaxed),
        }
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

/// Whether an answer naming `run` is about the server the app still has. One whose run
/// has moved on answers a question nobody has any more, and is dropped.
///
/// Only for the answers that carry no [`Ticket`]: one that does is matched against the
/// ticket whoever is waiting holds, and that carries the run.
///
/// A function so the read ends with it: every caller writes the state this was read from,
/// and a guard held across that write panics.
fn is_run(language: State<Language>, run: u64) -> bool {
    language.peek().run == run
}

/// Start the worker and keep the state in step with it. Called once, at the root.
pub(crate) fn use_language(
    language: State<Language>,
    follow: State<Follow>,
    located: State<Located>,
    linked: State<Linked>,
    hover: State<Hover>,
    proj: State<OpenProject>,
) -> LspJobs {
    use_language_with(
        language,
        follow,
        located,
        linked,
        hover,
        proj,
        language_work(),
    )
}

/// The same, with the work an argument: the seam a test drives the whole mechanism
/// through, there being no server on the machine to talk to.
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
                // Every remark is written the same way. The match is only what each
                // one then means for a question already asked.
                let noted = write_if(language, |held| held.remarked(run, &note));
                match note {
                    // A server that has gone quiet has read more of the project than it
                    // had when it refused a question about a file's names, so that
                    // question is put again. Here and not on every word it says: a server
                    // that goes on refusing would otherwise be asked in a tight loop. A
                    // server that says when it has settled says so below instead, and
                    // better.
                    lsp::Note::Busy(working) if noted && !working => {
                        write_if(linked, |waiting| waiting.forget_refusal());
                    }
                    // Everything asked before this was asked of a server still reading
                    // the project, and what it answered about a file's names was as far
                    // as it had got: fewer names, and some of them the wrong kind. So the
                    // answer is dropped and the question put again, which is the whole
                    // reason this notification is asked for.
                    lsp::Note::Settled(settled) if noted && settled => {
                        write_if(linked, |waiting| waiting.forget_answer());
                    }
                    _ => {}
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
        |job, queued| worth_doing(job, queued),
        work,
        move |answer, _| match answer {
            LspAnswer::Spawned { run, handle } => {
                write_if(language, |held| held.spawned(run, handle));
            }
            LspAnswer::Started { run, started } => {
                write_if(language, |held| held.running(run, started));
            }
            LspAnswer::Settings {
                directory,
                settings,
            } => {
                // A file read for a project that has since been left says nothing about
                // the one that is open now. The project and not the server is what this
                // answer is about, which is why the run says nothing about it.
                let open = proj.peek().workspace();
                if open.as_deref() != Some(directory.as_path()) {
                    return;
                }
                write_if(language, |held| held.read_settings(settings));
            }
            LspAnswer::Linked { run, file, links } => {
                if !is_run(language, run) {
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
                if !is_run(language, run) {
                    return;
                }
                // What was said about the file before the server had it is what it could
                // work out from the disk, which is nothing until its own scan reaches the
                // file.
                write_if(linked, |waiting| waiting.forget_file(&file));
            }
            LspAnswer::Hovered { ticket, said } => {
                // No run check of its own: the ticket carries the run, so an answer to a
                // question nobody holds lands on nobody -- and so does a failure named
                // for a server that has moved on (`Language::failed`).
                //
                // A refusal is already no answer by the time it is here
                // (`lsp::Talk::hover`), so what is left is a name the server had nothing
                // to say about -- no box, and no question to put again -- or a
                // conversation that ended.
                let why = match said {
                    Ok(said) => {
                        write_if(hover, |waiting| {
                            waiting.answer(ticket, said.map(|said| said.text))
                        });
                        return;
                    }
                    Err(failure) => failure,
                };
                // The question is dropped either way: a box that stayed asked would keep
                // the name from ever being asked about again.
                write_if(hover, |waiting| waiting.answer(ticket, None));
                write_if(language, |held| held.failed(ticket.run, why.to_string()));
            }
            LspAnswer::Answered { ticket, reply } => {
                // An answer from a server that has been stopped is an answer to nobody,
                // and the ticket says so: it carries the run, and no question of a run
                // that has moved on is still held. Nor is a failure named for that server
                // (`Language::failed`), which is why there is no run check here.
                //
                // Whoever asked takes the answer, and gives up on it where there is none.
                // The reply's own shape says which of them, so neither can be handed the
                // other's. An answer naming nowhere is an answer: the click was a
                // question, not a promise.
                let why = match reply {
                    Reply::Followed(reply) => {
                        let (places, why) = split(reply);
                        write_if(follow, |waiting| match &places {
                            Some(places) => waiting.answer(ticket, places),
                            None => waiting.give_up(ticket),
                        });
                        why
                    }
                    Reply::Listed(reply) => {
                        let (found, why) = split(reply);
                        // Nothing found and nothing to be found both leave the panel
                        // saying so: a question that stayed pending would say it was
                        // still looking for ever.
                        write_if(located, |waiting| {
                            waiting.answer_places(ticket, found.unwrap_or_default())
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
                write_if(language, |held| held.failed(ticket.run, why.to_string()));
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

    // The two paths that say which project is open. **A memo and not a read**: this hook
    // is called at the root, and `Proj` is written by every keystroke in the Project
    // view's boxes, so reading it here would re-render the whole window for each -- the
    // cost `WindowBody` is a component of its own to avoid (`src/ui/no_project.rs`). The
    // memo is subscribed to `Proj` and the effect below to the memo, so neither the root
    // nor the effect wakes until one of the two paths changes.
    let places = use_memo(move || {
        let open = proj.read();
        (open.file.clone(), open.workspace())
    });
    // **The directory is what a change is judged by**: both paths are watched and only one
    // of them is what a server reads. A directory it is no longer over ends it, and the
    // settings go with it, having been that directory's. The file moving on its own is
    // Save and nothing else -- the same tree, the same settings -- so the server stays.
    // Stopping there threw away a server that had read a whole project for a gesture about
    // where a `project.toml` is kept.
    //
    // The file is watched for the two things only it can say. A directory typed into the
    // box with the file where it was is the reader pointing *this* project somewhere else,
    // and the agreement was to the old place, so it goes; a project arriving brings its own
    // answer with it, out of its own session, and that answer is its own to give. And a
    // project arriving over the directory the last one's server is still reading stops it
    // where it has not agreed: the agreement is one project's, and a server running for a
    // project that never gave one is what the prompt is there to prevent.
    //
    // The mount is neither: what it mounts with is the reopened project, the restore being
    // an earlier hook of the same render, so an agreement read out of `project.toml`
    // survives the launch that read it.
    //
    // The memo is read **in the deps and not in the render**, which is what subscribes the
    // effect to the two paths and leaves the root subscribed to neither: the box the
    // directory is typed into writes `Proj` on every keystroke.
    use_on_change(move || places.read().clone(), {
        let jobs = jobs.clone();
        move |before, (file, directory): &(Option<PathBuf>, Option<PathBuf>)| {
            let elsewhere = !before.is_some_and(|(_, was_directory)| was_directory == directory);
            if elsewhere {
                // Read again here, where a directory arrives, so the answer is in hand
                // before either press can ask for a server and whether or not one is ever
                // started -- the Project view lists them either way.
                write_if(language, |held| held.forget_settings());
                if let Some(directory) = directory.clone() {
                    jobs.send(LspJob::ReadSettings { directory });
                }
            }
            // Bound to a `let` of its own: the write at the foot is to the state this was
            // read from.
            let agreed = proj.peek().trusted;
            if elsewhere || !agreed {
                stop_server(language, &jobs);
            }
            let moved = before.is_some_and(|(was_file, was_directory)| {
                was_file == file && was_directory != directory
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
    // Four fields out of one read, and not a clone of the whole project. The read ends
    // with the block, before either path below writes.
    let (asking, trusted) = {
        let open = proj.peek();
        // Nothing to run one over.
        let Some(directory) = open.workspace() else {
            return;
        };
        let asking = Asking {
            directory,
            serving: open.serving(),
        };
        (asking, open.trusted)
    };
    if trusted {
        run_server(language, jobs, asking);
        return;
    }
    write_if(language, |held| held.ask_to_start(asking));
}

/// Start the server or stop it, whichever the state asks for -- and put the question
/// first where a start needs one ([`start_server`]).
///
/// One function because there are two ways to ask for the same thing: the control in the
/// top bar, and the window's chord. A second spelling of the toggle is a second place for
/// the two halves to drift apart.
pub(crate) fn toggle_server(language: State<Language>, proj: State<OpenProject>, jobs: &LspJobs) {
    // Bound to a `let` of its own: a `match` holds its scrutinee's guard to the end of
    // the statement, and both arms below write the state it was read from.
    let started = language.peek().started();
    match started {
        true => stop_server(language, jobs),
        false => start_server(language, proj, jobs),
    }
}

/// Start what was asked for, leaving whatever was running stopped.
///
/// **A settings file that could not be read starts nothing.** What it would otherwise
/// reach the server as is a name it ignores or a path that is not there, and a server
/// reading the wrong project is worse than one that says why it did not start. The check
/// is here because this is where a start happens, so neither press nor the agreement can
/// grow a path around it -- the same reason the trust gate is in `start_server`.
fn run_server(mut language: State<Language>, jobs: &LspJobs, asking: Asking) {
    // Written whatever it answers, `starting` always counting the run up. The guard ends
    // with the statement, before the send.
    let starting = language.write().starting(asking.serving.clone());
    let Some((run, settings)) = starting else {
        return;
    };
    jobs.send(LspJob::Start {
        run,
        directory: asking.directory,
        program: asking.serving.program,
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

/// Ask `want` about the place `at`. The answer is the worker's, and arrives under the
/// [`Ticket`] minted here, which is what this hands back: its run tells one server from
/// another, and its id tells this question from the next one the same caller puts.
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
) -> Option<Ticket> {
    let ticket = jobs.ticket(language.peek().current()?);
    jobs.send(LspJob::Ask { ticket, at, want });
    Some(ticket)
}

/// Ask what the name at `at` is. [`ask_where`]'s rules, in every respect: the answer
/// arrives under the ticket minted here, there is nobody to ask with no server, and a
/// question put while one is starting waits for it.
pub(crate) fn ask_hover(language: State<Language>, jobs: &LspJobs, at: Lookup) -> Option<Ticket> {
    let ticket = jobs.ticket(language.peek().current()?);
    jobs.send(LspJob::Hover { ticket, at });
    Some(ticket)
}

#[cfg(test)]
mod tests;

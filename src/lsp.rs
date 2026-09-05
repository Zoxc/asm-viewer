//! The language server: rust-analyzer started over the project's directory and asked four
//! questions of one shape -- where the thing under a source position is defined, where it
//! is declared, what implements it, and where it is used -- and one of another: what every
//! name in a file **is**, which is what says which of them are links at all.
//!
//! Hand-rolled over `serde_json` rather than a protocol crate. What is spoken here is
//! eight messages wide -- the handshake's two, those five, and a reply to whatever the
//! server asks of us -- and a crate for it would bring a type for every request in the
//! specification and an async runtime's worth of machinery to drive them (`AGENTS.md`'s
//! pinning rules; `serde_json` is already in the tree and the manifest already blesses it
//! for a protocol rather than a file).
//!
//! **One request is in flight at a time**, so there is no table of outstanding ids: a
//! request writes its message and waits for the answer to that id. Two callers wanting
//! answers at once is what would end that, so which question an answer is to is the
//! caller's to keep (`src/ui/language.rs`).
//!
//! What the server says when nothing was asked is the other half. A reader thread owns the
//! server's output: an answer goes to whoever is waiting for it, a request is replied to,
//! and a notification is acted on -- which is how the app knows the server is busy reading
//! the project, since that arrives as `$/progress` and at no other time. Both threads
//! write to the server, so its input is behind a lock; the reader has to write because
//! declaring an interest in progress is what makes rust-analyzer ask this app to make a
//! progress token.
//!
//! [`Talk`] is generic over the two streams, so the conversation is tested against a fake
//! server over a pipe and the only part needing a real program is [`start_in`].
//!
//! What a server is told about the project is the other half of the handshake. [`wanted`]
//! is what this app asks of every server; a project's own `.vscode/settings.json` is read
//! by [`settings_in`] and laid over it, since some trees -- `rust-lang/rust` is the one
//! the notes use -- cannot be read by a server that was told nothing.
//!
//! The process is owned the way a scratchpad's run is (`src/scratchpad.rs`): a
//! [`Group`] arranged before the spawn, and a stop that kills the group rather than asking
//! the server to leave. A `shutdown` request is what the specification offers, and a
//! server that is indexing may take seconds to answer it; a stop must be over when it
//! returns, and rust-analyzer has nothing to lose by being killed.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::process::Group;

/// The program a project that has not said otherwise is read with. A project on a
/// toolchain of its own says (`Project::language_server`), since nothing here could guess
/// where such a thing lives.
pub const SERVER: &str = "rust-analyzer";

/// The largest message that will be read, so a server that says it is about to send a
/// gigabyte is a broken conversation and not an allocation.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// How much of what the server writes to stderr is kept. The **first** of it, since what
/// is wanted is why a program that would not run said no.
const MAX_SAID: usize = 4096;

/// How long a handshake that failed waits for the program to be gone before deciding it is
/// still there. A pipe that has closed means it is on its way out, and this is only the
/// moment between that and the kernel agreeing.
const ENDING: Duration = Duration::from_millis(200);

/// Why there is no answer. All three are ordinary and none is a bug here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The program could not be started -- not installed, most often.
    NoServer(String),
    /// The conversation ended: the pipe closed, or what came back was not a message.
    Broken(String),
    /// The server answered, with an error: the code it gave and what it said.
    Refused { code: i64, said: String },
}

/// The two error codes that mean "not now" rather than "no": ContentModified, which is
/// what a server still reading the project says, and RequestCancelled. Both are answers a
/// reader gets by asking again, and neither is worth reporting.
const NOT_NOW: [i64; 2] = [-32801, -32800];

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoServer(error) => {
                write!(formatter, "could not start the language server: {error}")
            }
            Failure::Broken(error) => {
                write!(formatter, "the language server stopped answering: {error}")
            }
            Failure::Refused { said, .. } => {
                write!(formatter, "the language server refused: {said}")
            }
        }
    }
}

/// What the server said while nothing was being asked of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Whether it is working: reading the project, priming its caches, or anything else it
    /// reports progress on. An answer asked for while this is true can be empty because
    /// the server has not read the file yet.
    Busy(bool),
}

/// Where something is: a file, a **1-based** line in it, and the columns of the name on
/// that line.
///
/// The protocol counts lines from zero and this counts from one, the unit line information
/// is in everywhere else in the app (`Object::symbols_from_lines`), so the conversion
/// happens here and once. The columns are already the UTF-16 units a pane counts in
/// (`src/chars.rs`) and are left as they came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: PathBuf,
    pub line: u32,
    /// The columns of the name on `line`. Empty where the answer's range spans lines: a
    /// name does not, and an empty run selects nothing.
    pub columns: Range<u32>,
}

/// The names a server gives the semantic token types and modifiers it will send, in the
/// order their indices count from.
///
/// **Read off the handshake and never assumed.** The order is the server's own -- for
/// rust-analyzer it is the order an enum happens to be written in -- so an index means
/// nothing without the list it was sent with, and a version that adds a type renumbers
/// everything after it. Asking by name is also what makes a server that sends fewer types
/// than another simply say less rather than say something wrong.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Legend {
    types: Vec<String>,
    modifiers: Vec<String>,
}

impl Legend {
    /// What the server calls `token`'s type, and `None` for an index it never declared.
    pub fn kind<'a>(&'a self, token: &Token) -> Option<&'a str> {
        self.types.get(token.kind as usize).map(String::as_str)
    }

    /// Whether the server said `modifier` of `token`. A modifier it never declared is one
    /// it cannot have said.
    pub fn says(&self, token: &Token, modifier: &str) -> bool {
        let Some(at) = self.modifiers.iter().position(|name| name == modifier) else {
            return false;
        };
        // The bitset is 32 bits wide on the wire, so a legend longer than that has
        // modifiers no answer can carry.
        u32::try_from(at).is_ok_and(|at| at < 32 && token.modifiers & (1 << at) != 0)
    }

    /// Whether the server declared any of this at all: an empty legend is a server that
    /// answers no semantic tokens, and every question about one is nothing found.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// A legend as a test spells one, the real ones coming off a handshake.
    #[cfg(test)]
    pub fn of(types: &[&str], modifiers: &[&str]) -> Legend {
        let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Legend {
            types: owned(types),
            modifiers: owned(modifiers),
        }
    }
}

/// One name the server classified, as [`Legend`] spells out: where it is, and the type
/// and modifier indices it was sent under.
///
/// The indices are kept rather than the names: a file is thousands of these, the names are
/// a few dozen, and what asks about one has the legend to hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// 1-based, as a [`Place`]'s line is and for its reason.
    pub line: u32,
    /// The columns of the name on `line`, in UTF-16 units.
    pub columns: Range<u32>,
    /// Which of the legend's types, by index.
    pub kind: u32,
    /// Which of its modifiers, one bit each, by index.
    pub modifiers: u32,
}

/// A started server: the conversation, and the process it is with.
pub struct Server {
    talk: Talk<ChildStdin>,
    process: Arc<Process>,
    /// What it wrote to stderr, which is where a program that will not run says why.
    said: Arc<Mutex<String>>,
    /// The thread filling `said`. Kept so a handshake that failed can wait for it to
    /// reach EOF before reading what it collected.
    stderr: Option<std::thread::JoinHandle<()>>,
}

/// Start rust-analyzer over `directory` and hand back the conversation and a handle that
/// can end it.
///
/// The handle is registered here, so [`stop_all`] reaches a server whose [`Server`] has
/// been lost -- the window's close hook can read no UI state and has only this.
pub fn start_in(
    program: &str,
    directory: &Path,
    told: impl FnMut(Note) + Send + 'static,
) -> Result<(Server, Handle), Failure> {
    start_program_in(program, directory, told)
}

/// [`start_in`] under the name the tests use, which is what lets the failing half of it be
/// tested without a language server anywhere on the machine.
fn start_program_in(
    program: &str,
    directory: &Path,
    told: impl FnMut(Note) + Send + 'static,
) -> Result<(Server, Handle), Failure> {
    let mut command = Command::new(program);
    command
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Read on a thread of its own and kept, up to a point: a pipe nobody reads fills
        // and then blocks the program in a write, and what a program that will not run
        // writes there is the only account of why. `rust-analyzer` is often a rustup
        // proxy, and a toolchain without the component is a line on stderr and an exit.
        .stderr(Stdio::piped());
    Group::arrange(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| Failure::NoServer(error.to_string()))?;
    let group = Group::of(&child);

    // Taken before the child goes behind the mutex: the conversation owns its two pipes
    // outright and must never need the lock a stop is waiting on.
    let (to, from) = (child.stdin.take(), child.stdout.take());
    let said = Arc::new(Mutex::new(String::new()));
    let stderr = keep_stderr(child.stderr.take(), &said);
    let process = Arc::new(Process {
        child: Mutex::new(Some((child, group))),
        over: AtomicBool::new(false),
    });
    let handle = Handle(process.clone());
    {
        let mut list = SERVERS.lock().unwrap_or_else(|held| held.into_inner());
        list.retain(|other| !other.finished());
        list.push(handle.clone());
    }

    // `Stdio::piped()` was asked for above, so both are there; a server started without
    // them could not be talked to at all.
    let (Some(to), Some(from)) = (to, from) else {
        handle.stop();
        return Err(Failure::NoServer("it has no pipes".to_owned()));
    };

    let server = Server {
        talk: Talk::over(to, BufReader::new(from), told),
        process,
        said,
        stderr,
    };
    Ok((server, handle))
}

/// Read the program's stderr on a thread of its own, keeping the first [`MAX_SAID`] bytes
/// of it and dropping the rest on the floor.
///
/// The thread is handed back, since when it has reached EOF is what says the program's
/// last words are all in.
fn keep_stderr(
    pipe: Option<impl Read + Send + 'static>,
    said: &Arc<Mutex<String>>,
) -> Option<std::thread::JoinHandle<()>> {
    let mut pipe = pipe?;
    let said = said.clone();
    Some(std::thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            let Ok(read) = pipe.read(&mut buffer) else {
                return;
            };
            if read == 0 {
                return;
            }
            let mut said = said.lock().unwrap_or_else(|held| held.into_inner());
            if said.len() < MAX_SAID {
                said.push_str(&String::from_utf8_lossy(&buffer[..read]));
            }
        }
    }))
}

/// Wait for the stderr thread to reach EOF, up to [`ENDING`], and let it go either way.
///
/// Bounded and not a plain join: stderr is inherited, so a grandchild the program left
/// behind holds the pipe open after the program itself is gone, and this is the failure
/// path of a handshake rather than somewhere to wait for ever.
fn all_said(reader: std::thread::JoinHandle<()>) {
    let until = std::time::Instant::now() + ENDING;
    while !reader.is_finished() {
        if std::time::Instant::now() >= until {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = reader.join();
}

impl Server {
    /// The handshake. Until it returns, the server has been asked nothing else.
    ///
    /// A handshake against a program that has already ended is not a conversation that
    /// broke: it is a program that would not run, and saying so is the difference between
    /// "rust-analyzer stopped answering" and the line it wrote on its way out.
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        self.talk.initialize(directory, options).map_err(|failure| {
            // In this order. The program's last words reach `said` on a thread of its
            // own, and both its pipes close at the same instant, so reading `said` first
            // -- and holding its lock over the wait -- is a race the stderr thread loses
            // about half the time. What it costs is the one line saying why the program
            // would not run, which is what this path is here to carry.
            let ended = self.process.ending();
            if ended.is_some() {
                if let Some(reader) = self.stderr.take() {
                    all_said(reader);
                }
            }
            let said = self.said.lock().unwrap_or_else(|held| held.into_inner());
            gone_instead(failure, ended, &said)
        })
    }

    /// Where what is at `line` and `column` of `file` is defined.
    ///
    /// `line` counts from zero and `column` is in UTF-16 units, which is what the protocol
    /// takes and what `src/chars.rs` already counts in.
    pub fn definition(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.talk.definition(file, line, column)
    }

    /// Where what is at `line` and `column` of `file` is declared, in the same units.
    pub fn declaration(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.talk.declaration(file, line, column)
    }

    /// What implements what is at `line` and `column` of `file`, in the same units.
    pub fn implementations(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.talk.implementations(file, line, column)
    }

    /// Every name in `file`, as the server classifies them.
    pub fn semantic_tokens(&mut self, file: &Path) -> Result<Vec<Token>, Failure> {
        self.talk.semantic_tokens(file)
    }

    /// What it said it would spell those with.
    pub fn legend(&self) -> &Legend {
        self.talk.legend()
    }

    /// Every use of what is at `line` and `column` of `file`, in the same units.
    pub fn references(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.talk.references(file, line, column)
    }
}

/// The failure a handshake really was, given what became of the program and what it said.
fn gone_instead(failure: Failure, ended: Option<String>, said: &str) -> Failure {
    let Some(status) = ended else {
        return failure;
    };
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    Failure::NoServer(match said.is_empty() {
        true => format!("it ended at once ({status})"),
        false => elided(&said),
    })
}

/// What a program said, cut to a length a line of the interface can hold.
fn elided(said: &str) -> String {
    const MOST: usize = 200;
    match said.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}...", &said[..at]),
        None => said.to_owned(),
    }
}

impl Drop for Server {
    /// Kill it and reap it. `Child`'s own `Drop` neither waits nor kills, so a server
    /// merely dropped would go on running with nothing left that could find it.
    fn drop(&mut self) {
        Handle(self.process.clone()).stop();
    }
}

/// A started server, as anything that is not the worker holds it: enough to end it, and
/// nothing to talk with.
#[derive(Clone)]
pub struct Handle(Arc<Process>);

impl Handle {
    /// Kill the server and everything it started, and wait for it to be gone.
    ///
    /// Also how a worker parked in a read is let go: the pipes close with the process, so
    /// the read it is blocked in ends instead of waiting for a server that will never
    /// answer.
    ///
    /// The second stop of a server does nothing: the first took the process out from under
    /// the lock, so no stop can name a pid the system has since given to somebody else.
    pub fn stop(&self) {
        self.0.over.store(true, Ordering::SeqCst);
        let mut held = self.0.child.lock().unwrap_or_else(|held| held.into_inner());
        let Some((mut child, group)) = held.take() else {
            return;
        };
        group.kill();
        // The child's own kill after the group's: it is what a platform with no group, or
        // a job object the system refused, still gets.
        let _ = child.kill();
        // It has been killed, so this returns at once, and it is what keeps a stopped
        // server from sitting in the process table until the app ends.
        let _ = child.wait();
    }

    /// Whether it has been stopped.
    pub fn finished(&self) -> bool {
        self.0.over.load(Ordering::SeqCst)
    }

    /// A handle with no process behind it, for the tests: everything a handle is asked
    /// about a server it has stopped is bookkeeping, and only the killing needs one.
    #[cfg(test)]
    pub fn to_nothing() -> Handle {
        Handle(Arc::new(Process {
            child: Mutex::new(None),
            over: AtomicBool::new(false),
        }))
    }
}

impl Process {
    /// How it ended, if it has, waiting [`ENDING`] for it to finish doing so.
    ///
    /// Asked only of a conversation that has already failed, so the wait is the price of
    /// telling a program that would not start from a server that stopped answering.
    fn ending(&self) -> Option<String> {
        let until = std::time::Instant::now() + ENDING;
        loop {
            {
                let mut held = self.child.lock().unwrap_or_else(|held| held.into_inner());
                let (child, _) = held.as_mut()?;
                match child.try_wait() {
                    Ok(Some(status)) => return Some(status.to_string()),
                    Ok(None) => {}
                    Err(error) => return Some(error.to_string()),
                }
            }
            if std::time::Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The process behind a [`Server`], shared by the conversation and by every [`Handle`].
struct Process {
    /// The process and the group it was started in -- what a stop kills, and what makes it
    /// reach further than [`Child::kill`] would. Behind a `Mutex` because two stops, and a
    /// stop and a drop, race by construction; **taken** by the stop that kills it, so the
    /// second stop is the no-op the first made it and a killed server is not waited for
    /// twice.
    child: Mutex<Option<(Child, Group)>>,
    over: AtomicBool,
}

/// Every server started in this run of the app that has not been stopped.
static SERVERS: Mutex<Vec<Handle>> = Mutex::new(Vec::new());

/// Stop every language server this app started.
///
/// For the window's close hook, which is a `Send` callback that can read no `State` --
/// `scratchpad::stop_all` is there for the same reason.
pub fn stop_all() {
    let servers = {
        let mut list = SERVERS.lock().unwrap_or_else(|held| held.into_inner());
        std::mem::take(&mut *list)
    };
    for server in servers {
        server.stop();
    }
}

/// The conversation itself: what is written to the server, what is read back, and the
/// messages this app knows how to say.
///
/// Generic over the two streams so it can be held against a fake server over a pipe, which
/// is what the tests do; the real one is over the process's own two.
pub struct Talk<W> {
    /// Behind a lock because the reader writes too: a request from the server is answered
    /// on its thread, whether or not this one is in the middle of asking something. An
    /// `Option` because the reader holds it as well, so this is what closes the server's
    /// input when the conversation is dropped.
    to: Arc<Mutex<Option<W>>>,
    /// The answers the reader has picked out of what the server said. A closed channel is
    /// a reader that has stopped, which is a conversation that is over.
    answers: std::sync::mpsc::Receiver<Result<Value, Failure>>,
    /// The id of the last request. Ids are this side's alone and only have to be distinct.
    id: i64,
    /// What the server said it would spell its semantic tokens with, from the handshake.
    /// Empty until then, and empty for a server that answers none.
    legend: Legend,
}

impl<W: Write + Send + 'static> Talk<W> {
    /// A conversation over two streams that are already connected to a server, with `told`
    /// called for whatever the server says that nobody asked for.
    pub fn over(
        to: W,
        from: impl BufRead + Send + 'static,
        told: impl FnMut(Note) + Send + 'static,
    ) -> Self {
        let to = Arc::new(Mutex::new(Some(to)));
        let (answered, answers) = std::sync::mpsc::channel();
        read_from(from, to.clone(), answered, told);
        Talk {
            to,
            answers,
            id: 0,
            legend: Legend::default(),
        }
    }

    /// The handshake: `initialize`, then the `initialized` notification, which the
    /// server waits for and which nothing may come before.
    ///
    /// The capabilities are **one line long**, and what is left out is the decision. Every
    /// request rust-analyzer would make of a client -- for configuration, to register a
    /// watcher -- is opt-in through a capability, so declaring none of those leaves a
    /// conversation this app only ever speaks first in. Nothing is said about positions or
    /// about definitions either: UTF-16 and plain locations are the defaults, both are
    /// what is wanted, and naming them would only be a chance to name them wrongly.
    ///
    /// The one thing asked for is progress, because it is the only way to know the server
    /// is still reading the project -- an answer before that is done is empty and says
    /// nothing about why. It costs the `window/workDoneProgress/create` requests the
    /// reader answers.
    ///
    /// Semantic tokens are **not** declared either, though they are asked for: rust-analyzer
    /// offers them and sends its whole legend to a client that says nothing, which was
    /// measured against a real one before this was written. What the reply says it will
    /// send is kept ([`legend_of`]), since the indices in an answer mean nothing without
    /// it.
    ///
    /// The options are the caller's: [`wanted`] with whatever the project's own
    /// `.vscode/settings.json` said laid over it ([`settings_from`]).
    ///
    /// The directory is made absolute first ([`rooted`]). The box it was typed into takes
    /// any spelling, and a `rootUri` built out of `dev/viewer` or `.` names a place that
    /// is not there -- which a server reports through `window/showMessage` and this
    /// client only logs, leaving a control that says it is running and every question
    /// answering nothing.
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        let directory = rooted(directory);
        let root = uri_of(&directory);
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let said = self.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "clientInfo": { "name": "Assembly Viewer" },
                "rootUri": root,
                "workspaceFolders": [{ "uri": root, "name": name }],
                "capabilities": { "window": { "workDoneProgress": true } },
                "initializationOptions": options,
            }),
        )?;
        self.legend = legend_of(&said);
        self.notify("initialized", json!({}))
    }

    /// Where what is at `line` and `column` of `file` is defined.
    ///
    /// The file is not opened first. rust-analyzer reads the project's files itself, and
    /// this app only ever shows what is on disk, so telling it about one would put an
    /// overlay over the file that has to be taken back off again and can only go stale.
    /// The cost is that a file outside the project answers nothing, which is the same
    /// nothing a question with no answer gets.
    pub fn definition(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.places_at("textDocument/definition", asked_at(file, line, column))
    }

    /// Where what is at `line` and `column` of `file` is **declared**, in the same units.
    ///
    /// A different question from [`Talk::definition`] and not a fallback for it: an item
    /// in a trait `impl` is defined where it is written and declared in the trait, and a
    /// call to a trait method is defined in the `impl` that runs and declared in the trait
    /// as well. So the two disagree wherever a trait is involved, and which of them a name
    /// asks is `src/links.rs`'s to say.
    pub fn declaration(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.places_at("textDocument/declaration", asked_at(file, line, column))
    }

    /// What implements what is at `line` and `column` of `file`, in the same units.
    pub fn implementations(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.places_at("textDocument/implementation", asked_at(file, line, column))
    }

    /// Every use of what is at `line` and `column` of `file`, in the same units.
    ///
    /// Where it is **defined** is not one: a reader who is looking at the name has that
    /// under the pointer already, and following the link is the door to it.
    pub fn references(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        let mut params = asked_at(file, line, column);
        params["context"] = json!({ "includeDeclaration": false });
        self.places_at("textDocument/references", params)
    }

    /// Every name in `file`, as the server classifies them.
    ///
    /// One request for the whole file rather than one per name: it is the only way to be
    /// told what a name **is** without asking about each in turn, and asking about each
    /// would be a round trip per name down a conversation that holds one question at a
    /// time. The file is not opened first, for [`Talk::definition`]'s reason.
    ///
    /// A server that declared no legend is one that answers none of this, and is not
    /// asked.
    ///
    /// **A refusal is passed on and not read as an empty answer.** The two "not now"
    /// codes and the one a server gives for a file it has not read yet all mean "ask
    /// again", which only the caller can do -- and does, once the server says it has read
    /// more of the project (`src/ui/linking.rs`).
    pub fn semantic_tokens(&mut self, file: &Path) -> Result<Vec<Token>, Failure> {
        if self.legend.is_empty() {
            return Ok(Vec::new());
        }
        let params = json!({ "textDocument": { "uri": uri_of(file) } });
        self.request("textDocument/semanticTokens/full", params)
            .map(|value| tokens(&value))
    }

    /// What the server said it would spell its semantic tokens with.
    pub fn legend(&self) -> &Legend {
        &self.legend
    }

    /// The half every question about a place shares: the places an answer names, and the
    /// codes that are not an answer at all.
    fn places_at(&mut self, method: &str, params: Value) -> Result<Vec<Place>, Failure> {
        match self.request(method, params) {
            Ok(value) => Ok(places(&value)),
            // "Ask again": the server is still reading the project, or what was asked
            // about changed under the question. Not a failure to report and not an
            // answer -- a click is a question, not a promise.
            Err(Failure::Refused { code, .. }) if NOT_NOW.contains(&code) => Ok(Vec::new()),
            Err(failure) => Err(failure),
        }
    }

    /// One request, and the answer to it.
    ///
    /// Everything else the server says is the reader's: this waits for an answer carrying
    /// the id it asked under, and a conversation that ended arrives here as the reader
    /// letting go of its end.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.id += 1;
        let id = self.id;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        loop {
            let message = self
                .answers
                .recv()
                .map_err(|_| Failure::Broken("it closed the connection".to_owned()))??;
            // An older request's answer cannot happen with one request in flight, and is
            // dropped rather than mistaken for this one's.
            if message.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(Failure::Refused {
                    code: error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                    said: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("no reason given")
                        .to_owned(),
                });
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// One notification: no id, so no answer is coming and none is waited for.
    fn notify(&mut self, method: &str, params: Value) -> Result<(), Failure> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn write(&mut self, body: &Value) -> Result<(), Failure> {
        write_to(&self.to, body)
    }
}

/// Write one message to a server two threads talk to, and that one of them may already
/// have said goodbye to.
fn write_to(to: &Mutex<Option<impl Write>>, body: &Value) -> Result<(), Failure> {
    let mut to = to.lock().unwrap_or_else(|held| held.into_inner());
    let Some(to) = to.as_mut() else {
        return Err(Failure::Broken("the conversation is over".to_owned()));
    };
    write_message(to, body).map_err(|error| Failure::Broken(error.to_string()))
}

/// Read everything the server says, on a thread of its own, until it stops saying
/// anything.
///
/// An answer goes to whoever asked; a request is replied to here, since the server may ask
/// while nothing is being asked of it; and a notification is what `told` is for. The
/// thread ends when the server's output does, and the closed channel is what tells a
/// waiting request that the conversation is over.
fn read_from<W: Write + Send + 'static>(
    mut from: impl BufRead + Send + 'static,
    to: Arc<Mutex<Option<W>>>,
    answered: std::sync::mpsc::Sender<Result<Value, Failure>>,
    mut told: impl FnMut(Note) + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut working = std::collections::HashSet::new();
        loop {
            let message = match read_message(&mut from) {
                Ok(message) => message,
                // The last word: whoever is waiting is told, and whoever asks next finds
                // the channel closed.
                Err(failure) => {
                    let _ = answered.send(Err(failure));
                    return;
                }
            };
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned);
            match method {
                // A request of us if it carries an id, a notification if it does not.
                Some(method) => match message.get("id") {
                    Some(asked) => {
                        let answer = answer_to(&method, &message);
                        if write_to(&to, &reply(asked.clone(), answer)).is_err() {
                            return;
                        }
                    }
                    None => {
                        if let Some(busy) = busy_after(&method, &message, &mut working) {
                            told(Note::Busy(busy));
                        }
                    }
                },
                // An answer, for whoever is waiting on one.
                None if answered.send(Ok(message)).is_err() => return,
                None => {}
            }
        }
    });
}

/// Whether the server is working, if this notification changed the answer.
///
/// Progress arrives as a token that begins and ends, and several are open at once while
/// rust-analyzer reads a project -- so what is kept is the set of them, and what is said is
/// only that it went from empty to not or back.
fn busy_after(
    method: &str,
    message: &Value,
    working: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    // The one notification a client with no capabilities of its own is told when the
    // server cannot make sense of the project. Every definition after it will be empty,
    // and this is the only place it is said.
    if method == "window/showMessage" {
        log::warn!("the language server said: {message}");
        return None;
    }
    if method != "$/progress" {
        return None;
    }

    let params = message.get("params")?;
    let token = match params.get("token")? {
        Value::String(token) => token.clone(),
        token => token.to_string(),
    };
    let was = !working.is_empty();
    match params.get("value")?.get("kind")?.as_str()? {
        "begin" => working.insert(token),
        "end" => working.remove(&token),
        // A report is progress within a token that has already begun.
        _ => false,
    };
    let now = !working.is_empty();
    (was != now).then_some(now)
}

/// What this app asks of a language server whatever the project: the options it sends at
/// every handshake, and what a project's own settings are laid over.
///
/// **One line, and it turns something off.** Nothing is turned on: what navigation needs is
/// what rust-analyzer already does -- build scripts run and proc macros expand unless a
/// client says otherwise, and a name inside a macro that was not expanded resolves to
/// nothing -- and saying so again would only be a chance to say it wrongly, which is the
/// rule the capabilities follow too.
///
/// The check is off because the server runs one **on loading the workspace**, and not only
/// when a document is saved: watched, it opens a `rust-analyzer/flycheck/0` progress token
/// over a client that has opened no document and saved nothing. This app runs cargo itself
/// from the Project view and shows what came of it, so leaving it alone is a second build
/// of the reader's project whose output goes nowhere. The server's own diagnostics need no
/// turning off beside it: they are published for open documents, and this client opens
/// none.
///
/// What a project needs beyond this -- which manifests are its workspaces, where a tree
/// keeps its own proc-macro server, sysroot sources or toolchain -- depends on the tree and
/// not on this app, and nothing here can guess it: that is what a project's own settings
/// file is read for.
pub fn wanted() -> Value {
    json!({ "checkOnSave": false })
}

/// The file a project's own settings for the server are in, which is VS Code's:
/// `.vscode/settings.json` under the project's directory. Most projects have none, and
/// that is not a failure.
pub const SETTINGS: &str = ".vscode/settings.json";

/// The prefix a key in that file carries when it is meant for the server. Everything else
/// there is the editor's (`git.*`, `files.associations`) and is passed over in silence.
const PREFIX: &str = "rust-analyzer.";

/// The one variable a value may be written with. `${workspaceFolder}` is what a tree uses
/// to point at its own proc-macro server and its own toolchain, and it is the only one
/// this app has an answer for.
const FOLDER: &str = "${workspaceFolder}";

/// The most parts a name may be spelled in. Nothing rust-analyzer takes is more than four
/// deep, and the tree the names build is walked by recursion: how deep that goes is not
/// for a file to say (`AGENTS.md`, never panic on file input).
const DEEPEST: usize = 16;

/// What a project's own settings file said, ready to be handed to a server.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// One per key taken from the file, in name order: the name with `rust-analyzer.` off
    /// it, and the value written back out as it will be sent. What the Project view lists.
    pub overrides: Vec<(String, String)>,
    /// The same, as a server takes it: names split on their dots into a tree, laid over
    /// [`wanted`].
    options: Value,
}

impl Settings {
    /// A project that said nothing, which is what one with no such file has.
    pub fn none() -> Settings {
        Settings {
            overrides: Vec::new(),
            options: wanted(),
        }
    }

    /// What to send as `initializationOptions`.
    pub fn options(&self) -> &Value {
        &self.options
    }
}

/// Why a settings file could not be used. Every one of these **stops a start**: what a
/// server would otherwise be given is a name it ignores or a path that silently does not
/// exist, and either is worse than saying so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// It could not be read at all. A file that is not there is not this: that is the
    /// ordinary case and answers with [`Settings::none`].
    Unread(String),
    /// Not JSON, once the comments and trailing commas an editor allows are taken out
    /// of it ([`as_json`]).
    NotJson(String),
    /// JSON, but not an object.
    NotAnObject,
    /// A name given a value and made a table by a longer name: `cargo` beside
    /// `cargo.features`. Which was meant is not for this app to pick.
    Both(String),
    /// A `${...}` that is not `${workspaceFolder}`.
    Variable(String),
    /// A name spelled in more parts than [`DEEPEST`].
    Deep(String),
}

impl fmt::Display for Unreadable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{SETTINGS}: ")?;
        match self {
            Unreadable::Unread(error) => write!(formatter, "{error}"),
            Unreadable::NotJson(error) => write!(formatter, "not JSON ({error})"),
            Unreadable::NotAnObject => write!(formatter, "not an object"),
            Unreadable::Both(name) => {
                write!(formatter, "{PREFIX}{name} is given a value and a table")
            }
            Unreadable::Variable(name) => {
                write!(formatter, "${{{name}}} is not a variable this can resolve")
            }
            Unreadable::Deep(name) => write!(formatter, "{PREFIX}{name} has too many parts"),
        }
    }
}

/// Read the project's own settings out of `directory`.
///
/// The thin half: everything below this is a function of the file's text. **No file is no
/// overrides**, since most projects have none and a viewer that warned about it would be
/// warning about every project.
pub fn settings_in(directory: &Path) -> Result<Settings, Unreadable> {
    let file = directory.join(".vscode").join("settings.json");
    match std::fs::read_to_string(&file) {
        Ok(text) => settings_from(&text, directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::none()),
        Err(error) => Err(Unreadable::Unread(error.to_string())),
    }
}

/// What that file says, as a server would be told it.
///
/// The two halves that matter are both silent when they are wrong, which is why they are
/// done here and tested rather than trusted: a server ignores a key that kept its
/// `rust-analyzer.` prefix, and ignores one whose dots were not split into a tree. Both
/// were watched happening against a real server. The rest of the file is the editor's own
/// keys, and they are skipped without a word.
pub fn settings_from(text: &str, directory: &Path) -> Result<Settings, Unreadable> {
    let read: Value = serde_json::from_str(&as_json(text))
        .map_err(|error| Unreadable::NotJson(error.to_string()))?;
    let Value::Object(read) = read else {
        return Err(Unreadable::NotAnObject);
    };

    let mut overrides = Vec::new();
    let mut root = BTreeMap::new();
    for (key, value) in &read {
        let Some(name) = key.strip_prefix(PREFIX) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if name.split('.').count() > DEEPEST {
            return Err(Unreadable::Deep(name.to_owned()));
        }
        let value = substituted(value, directory)?;
        overrides.push((name.to_owned(), value.to_string()));
        put(&mut root, name, value)?;
    }

    Ok(Settings {
        overrides,
        options: merged(wanted(), object_of(root)),
    })
}

/// The JSON in a settings file.
///
/// VS Code reads that file as **JSONC**, and the files in the wild are written as one: the
/// tree the whole feature is for opens with nine lines of `//`. `serde_json` takes neither
/// comments nor a trailing comma, so both are taken out here, before it sees the text.
///
/// Comments become spaces rather than nothing, and a newline inside a block comment is
/// kept, so what `serde_json` says about the line and column of a real mistake is about
/// the file the reader wrote.
fn as_json(text: &str) -> String {
    without_trailing_commas(&without_comments(text))
}

/// Comments blanked. **Nothing inside a string is touched**: a `//` is half of every URL,
/// and a string can end in an escaped quote (`"a \" // b"`) or hold a backslash before its
/// closing one (`"c:\\"`), so this tracks whether it is inside a string and whether the
/// last character was an escape. Getting that wrong cuts a path short without a word, which
/// is the failure this whole feature is against.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut string = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if string {
            out.push(character);
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match (character, chars.peek()) {
            ('"', _) => {
                string = true;
                out.push('"');
            }
            // To the end of the line, which is left where it is.
            ('/', Some('/')) => {
                out.push_str("  ");
                chars.next();
                while chars.peek().is_some_and(|next| *next != '\n') {
                    out.push(' ');
                    chars.next();
                }
            }
            // To the next `*/`, keeping the newlines so the lines below still count.
            ('/', Some('*')) => {
                out.push_str("  ");
                chars.next();
                let mut star = false;
                for character in chars.by_ref() {
                    out.push(match character {
                        '\n' => '\n',
                        _ => ' ',
                    });
                    if star && character == '/' {
                        break;
                    }
                    star = character == '*';
                }
            }
            _ => out.push(character),
        }
    }
    out
}

/// The comma before a `}` or a `]` taken out, which VS Code's own parser allows and
/// `serde_json` does not. Over text the pass above has already blanked the comments in, so
/// what is between the comma and the bracket is whitespace or nothing.
fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut string = false;
    let mut escaped = false;
    // Where the last comma was written, while nothing but whitespace has followed it.
    let mut comma: Option<usize> = None;
    for character in text.chars() {
        if string {
            out.push(character);
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => {
                string = true;
                comma = None;
            }
            ',' => comma = Some(out.len()),
            '}' | ']' => {
                if let Some(at) = comma.take() {
                    out.replace_range(at..at + 1, " ");
                }
            }
            character if character.is_whitespace() => {}
            _ => comma = None,
        }
        out.push(character);
    }
    out
}

/// A name being built out of the file's dotted keys: the value the file gave under exactly
/// this name, or the table a longer name made of it. The two are what tells a clash from a
/// merge -- `cargo.features` and `cargo.noDeps` make one table between them, and `cargo`
/// with a value of its own beside either of them is a file saying two things.
enum Node {
    Value(Value),
    Table(BTreeMap<String, Node>),
}

/// Put one of the file's keys in the tree, under the name split on its dots.
///
/// Iterative, and not for elegance: the name comes from a file, and a recursion whose depth
/// it decided is a stack overflow, which cannot be caught.
fn put(root: &mut BTreeMap<String, Node>, name: &str, value: Value) -> Result<(), Unreadable> {
    let mut parts = name.split('.').peekable();
    let mut table = root;
    // Where the name being put reaches to, which is the name a clash is about however
    // the two keys were written and in whichever order the file wrote them.
    let mut at = 0;
    while let Some(part) = parts.next() {
        let clash = || Unreadable::Both(name[..at + part.len()].to_owned());
        if parts.peek().is_none() {
            // A name the file gave twice cannot reach here -- JSON keeps one of them --
            // so anything already under this name is the table a longer name made.
            if table.contains_key(part) {
                return Err(clash());
            }
            table.insert(part.to_owned(), Node::Value(value));
            return Ok(());
        }
        let node = table
            .entry(part.to_owned())
            .or_insert_with(|| Node::Table(BTreeMap::new()));
        let Node::Table(under) = node else {
            return Err(clash());
        };
        table = under;
        at += part.len() + 1;
    }
    Ok(())
}

/// The tree as JSON. Recursion bounded by [`DEEPEST`], which is what `put` refused a
/// deeper name for.
fn object_of(table: BTreeMap<String, Node>) -> Value {
    Value::Object(
        table
            .into_iter()
            .map(|(name, node)| {
                let value = match node {
                    Node::Value(value) => value,
                    Node::Table(under) => object_of(under),
                };
                (name, value)
            })
            .collect(),
    )
}

/// `over` laid on `base`, **leaf by leaf**: two objects are merged key by key and anything
/// else replaces what was under it.
///
/// Per leaf and not per name, so a project setting `cargo.features` keeps whatever else
/// this app sent under `cargo` rather than standing in for the whole of it. Recursion is
/// bounded by the two values' own depth, and a parsed one is bounded by `serde_json`'s
/// nesting limit.
fn merged(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Object(mut base), Value::Object(over)) => {
            for (name, value) in over {
                let under = base.remove(&name).unwrap_or(Value::Null);
                base.insert(name, merged(under, value));
            }
            Value::Object(base)
        }
        // Anything that is not two objects is a leaf, and the project's own stands.
        (_, over) => over,
    }
}

/// Every string in a value with its variables resolved, in place: this walks objects and
/// arrays and changes nothing but strings, which is what VS Code's own pass does.
fn substituted(value: &Value, directory: &Path) -> Result<Value, Unreadable> {
    Ok(match value {
        Value::String(text) => Value::String(resolved(text, directory)?),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| substituted(value, directory))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| Ok((name.clone(), substituted(value, directory)?)))
                .collect::<Result<_, _>>()?,
        ),
        value => value.clone(),
    })
}

/// One string with its variables resolved.
///
/// `${workspaceFolder}` becomes the project's directory and **every other variable is a
/// failure**. VS Code leaves a name it does not know as it was written, which here would
/// be a path reaching the server that silently does not exist; saying so is the better of
/// the two. A `${` that is never closed is not a variable and is left alone.
fn resolved(text: &str, directory: &Path) -> Result<String, Unreadable> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        let Some(end) = rest[at..].find('}').map(|end| at + end) else {
            break;
        };
        let variable = &rest[at..=end];
        if variable != FOLDER {
            return Err(Unreadable::Variable(rest[at + 2..end].to_owned()));
        }
        out.push_str(&rest[..at]);
        out.push_str(&directory.to_string_lossy());
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// What to answer a request the server made of us, as a result or as an error.
///
/// A client that declared no capabilities should be asked nothing, so every arm here is a
/// server going beyond what it was told: the two that have a harmless empty answer get it,
/// and the rest are told the method is not there rather than being left waiting.
fn answer_to(method: &str, message: &Value) -> Result<Value, Value> {
    match method {
        // One setting object per item asked about, each of them "nothing to override".
        "workspace/configuration" => {
            let items = message
                .get("params")
                .and_then(|params| params.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Ok(Value::Array(vec![json!({}); items]))
        }
        // Registering a capability and making a progress token both answer with nothing.
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => Ok(Value::Null),
        _ => Err(json!({ "code": -32601, "message": "not a method this client has" })),
    }
}

/// A response to a request the server made: what `answer_to` decided, under the id it was
/// asked with.
fn reply(id: Value, answer: Result<Value, Value>) -> Value {
    match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

/// The position a question is about, as every question about one sends it.
fn asked_at(file: &Path, line: u32, column: u32) -> Value {
    json!({
        "textDocument": { "uri": uri_of(file) },
        "position": { "line": line, "character": column },
    })
}

/// What the handshake's reply said it would spell semantic tokens with, and an empty
/// legend where it offered none.
///
/// Both lists are taken as they came and in the order they came: an index in an answer is
/// a position in these.
fn legend_of(said: &Value) -> Legend {
    let names = |of: &str| -> Vec<String> {
        said.get("capabilities")
            .and_then(|value| value.get("semanticTokensProvider"))
            .and_then(|value| value.get("legend"))
            .and_then(|value| value.get(of))
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .map(|name| name.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    Legend {
        types: names("tokenTypes"),
        modifiers: names("tokenModifiers"),
    }
}

/// The tokens an answer holds, out of the flat array of numbers it sends them as.
///
/// Five numbers each, and **every one relative to the token before it**: the lines since
/// the last, the columns since the last where that is zero and from the start of the line
/// where it is not, the length, the type, and the modifiers as a bitset. The first token
/// counts from line zero, column zero.
///
/// A length that is not a multiple of five is a message this cannot read the end of, so
/// what it could read is kept and the rest dropped; that and a number too big for a `u32`
/// are the only ways an answer here is not an answer, and neither is worth a word to the
/// reader (`AGENTS.md`: never panic on any file input, and a server's answer is one).
fn tokens(answer: &Value) -> Vec<Token> {
    let Some(data) = answer
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| answer.as_array())
    else {
        return Vec::new();
    };
    let mut tokens = Vec::with_capacity(data.len() / 5);
    let (mut line, mut column) = (0u32, 0u32);
    for five in data.chunks_exact(5) {
        let read = |at: usize| -> Option<u32> { u32::try_from(five.get(at)?.as_u64()?).ok() };
        let (Some(down), Some(along), Some(length), Some(kind), Some(modifiers)) =
            (read(0), read(1), read(2), read(3), read(4))
        else {
            break;
        };
        line = line.saturating_add(down);
        // A token on the same line as the one before it carries on from where that one
        // started; one on a later line counts from the start of its own.
        column = match down {
            0 => column.saturating_add(along),
            _ => along,
        };
        tokens.push(Token {
            // The protocol counts lines from zero and everything else here counts from
            // one, as `places` converts them.
            line: line.saturating_add(1),
            columns: column..column.saturating_add(length),
            kind,
            modifiers,
        });
    }
    tokens
}

/// The places an answer names, whichever of the shapes it came in.
///
/// No `linkSupport` was declared, so a list of plain locations is what should arrive; the
/// bare location and the link are read too, since a server that sends one costs a `match`
/// arm here and would otherwise cost the answer.
fn places(answer: &Value) -> Vec<Place> {
    let one = |value: &Value| {
        let uri = value
            .get("uri")
            .or_else(|| value.get("targetUri"))
            .and_then(Value::as_str)?;
        let range = value.get("range").or_else(|| value.get("targetRange"))?;
        let start = range.get("start")?;
        let line = start.get("line")?.as_u64()?;
        let at = |place: &Value| -> u32 {
            place
                .get("character")
                .and_then(Value::as_u64)
                .and_then(|column| u32::try_from(column).ok())
                .unwrap_or(0)
        };
        let from = at(start);
        // The columns are a name's only where the range is one line's: one that ends on
        // another names more than a name, and the empty run is what says so.
        let ends_here = |end: &&Value| end.get("line").and_then(Value::as_u64) == Some(line);
        let to = range.get("end").filter(ends_here).map_or(from, at);
        Some(Place {
            file: path_of(uri)?,
            // The protocol counts from zero and everything else here counts from one.
            line: u32::try_from(line).ok()?.saturating_add(1),
            columns: from..to.max(from),
        })
    };

    match answer {
        Value::Array(values) => values.iter().filter_map(one).collect(),
        Value::Object(_) => one(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

impl<W> Drop for Talk<W> {
    /// Close the server's input. It is how a server is told there is nothing more coming
    /// -- a language server reads its input to the end and then leaves -- and it is also
    /// what lets the reader thread go, since its own read ends when the server does.
    fn drop(&mut self) {
        *self.to.lock().unwrap_or_else(|held| held.into_inner()) = None;
    }
}

/// Write one message: the header the protocol frames with, and the body.
pub fn write_message(to: &mut impl Write, body: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(body)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    message.extend_from_slice(&body);
    // One write and not three: a message the server reads half of is one it waits on.
    to.write_all(&message)?;
    to.flush()
}

/// Read one message. The headers up to the blank line, then exactly the length they said.
pub fn read_message(from: &mut impl BufRead) -> Result<Value, Failure> {
    let mut length = None;
    loop {
        let mut header = String::new();
        let read = from
            .read_line(&mut header)
            .map_err(|error| Failure::Broken(error.to_string()))?;
        if read == 0 {
            return Err(Failure::Broken("it closed the connection".to_owned()));
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }

    let Some(length) = length.filter(|length| *length <= MAX_MESSAGE) else {
        return Err(Failure::Broken(
            "a message with no usable length".to_owned(),
        ));
    };
    let mut body = vec![0; length];
    from.read_exact(&mut body)
        .map_err(|error| Failure::Broken(error.to_string()))?;
    serde_json::from_slice(&body).map_err(|error| Failure::Broken(error.to_string()))
}

/// The project's directory as the server is told about it: absolute, since a `rootUri` is
/// a URI and a relative one names a place nobody has.
///
/// `path::absolute` and not the `fs::canonicalize` `src/cargo.rs` uses on Unix, because
/// nothing here has to match a spelling something else prints back: the server's answers
/// are reconciled against the tabs already open (`ui::follow`). Resolving would only cost:
/// the reader's own spelling of their project, and on Windows a verbatim prefix
/// (`\\?\C:\work`) that no `file:` URI can carry.
///
/// The process needs none of this: [`start_program_in`] hands the same relative directory
/// to `current_dir`, which the spawn resolves against the same working directory this
/// does.
fn rooted(directory: &Path) -> PathBuf {
    std::path::absolute(directory).unwrap_or_else(|_| directory.to_path_buf())
}

/// A path as the `file:` URI the protocol names files by.
///
/// Percent-encoded by hand rather than by a crate: what has to be escaped is every byte
/// that is not unreserved, and a path is the only thing this app ever puts in a URI.
fn uri_of(path: &Path) -> String {
    let path = path.to_string_lossy();
    let mut uri = String::from("file://");
    // A Windows path starts with a drive letter and not with a separator, and the
    // authority-less form needs the third slash either way.
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                uri.push(byte as char)
            }
            b'\\' => uri.push('/'),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The path a `file:` URI names, or nothing if it names something else.
fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // The path begins at the third slash. Anything between the second and the third is an
    // authority, and that names a file on somebody else's machine.
    if !rest.starts_with('/') {
        return None;
    }

    let mut bytes = Vec::with_capacity(rest.len());
    let mut characters = rest.bytes();
    while let Some(byte) = characters.next() {
        match byte {
            b'%' => {
                let (high, low) = (characters.next()?, characters.next()?);
                let digits = [high, low];
                let text = std::str::from_utf8(&digits).ok()?;
                bytes.push(u8::from_str_radix(text, 16).ok()?);
            }
            byte => bytes.push(byte),
        }
    }

    let path = String::from_utf8(bytes).ok()?;
    Some(PathBuf::from(spelled(&path).as_ref()))
}

/// A decoded URI path as the platform it names spells one.
///
/// `/C:/x/y.rs` is how a Windows path comes back: both the leading slash and the
/// separators are the URI's, where the app spells that file `C:\x\y.rs`. A
/// [`Document::Source`](crate::project::Document) is compared as text and never
/// canonicalised, so the two spellings are two tabs of one file.
///
/// The drive letter is what says a path is Windows', not a `cfg`, so the rule is the same
/// everywhere and can be tested from either platform -- no Unix path begins with one, and
/// one keeps its leading slash and every character after it.
fn spelled(path: &str) -> Cow<'_, str> {
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => {
            Cow::Owned(path[1..].replace('/', "\\"))
        }
        _ => Cow::Borrowed(path),
    }
}

#[cfg(test)]
mod tests;

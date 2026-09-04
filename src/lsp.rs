//! The language server: rust-analyzer started over the project's directory and asked, so
//! far, one question -- where the thing under a source position is defined.
//!
//! Hand-rolled over `serde_json` rather than a protocol crate. What is spoken here is four
//! messages wide, and a crate for it would bring a type for every request in the
//! specification and an async runtime's worth of machinery to drive them (`AGENTS.md`'s
//! pinning rules; `serde_json` is already in the tree and the manifest already blesses it
//! for a protocol rather than a file).
//!
//! **One request is in flight at a time**, so there is no table of outstanding ids: a
//! request writes its message and waits for the answer to that id. It is also the whole
//! reason the surface is one call wide -- a second consumer wanting answers at the same
//! time is what would end it.
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

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
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

/// Where something is defined: a file, a **1-based** line in it, and the column the name
/// starts at.
///
/// The protocol counts lines from zero and this counts from one, the unit line information
/// is in everywhere else in the app (`Object::symbols_from_lines`), so the conversion
/// happens here and once. The column is left as the protocol gives it, a UTF-16 unit
/// counted from zero, which is what a pane counts columns in too (`src/chars.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

/// A started server: the conversation, and the process it is with.
pub struct Server {
    talk: Talk<ChildStdin>,
    process: Arc<Process>,
    /// What it wrote to stderr, which is where a program that will not run says why.
    said: Arc<Mutex<String>>,
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
    keep_stderr(child.stderr.take(), &said);
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
    };
    Ok((server, handle))
}

/// Read the program's stderr on a thread of its own, keeping the first [`MAX_SAID`] bytes
/// of it and dropping the rest on the floor.
fn keep_stderr(pipe: Option<impl Read + Send + 'static>, said: &Arc<Mutex<String>>) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let said = said.clone();
    std::thread::spawn(move || {
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
    });
}

impl Server {
    /// The handshake. Until it returns, the server has been asked nothing else.
    ///
    /// A handshake against a program that has already ended is not a conversation that
    /// broke: it is a program that would not run, and saying so is the difference between
    /// "rust-analyzer stopped answering" and the line it wrote on its way out.
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        self.talk.initialize(directory, options).map_err(|failure| {
            let said = self.said.lock().unwrap_or_else(|held| held.into_inner());
            gone_instead(failure, self.process.ending(), &said)
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

/// The conversation itself: what is written to the server, what is read back, and the four
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
        Talk { to, answers, id: 0 }
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
    /// The options are the caller's: [`wanted`] with whatever the project's own
    /// `.vscode/settings.json` said laid over it ([`settings_from`]).
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        let root = uri_of(directory);
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.request(
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
        let answer = self.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri_of(file) },
                "position": { "line": line, "character": column },
            }),
        );
        match answer {
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

/// The places a definition answer names, whichever of the shapes it came in.
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
        // A column the answer leaves out, or one no `u32` holds, is column 0: the line is
        // what opens the file, and the column only says where the caret goes.
        let column = start
            .get("character")
            .and_then(Value::as_u64)
            .and_then(|character| u32::try_from(character).ok())
            .unwrap_or(0);
        Some(Place {
            file: path_of(uri)?,
            // The protocol counts from zero and everything else here counts from one.
            line: u32::try_from(line).ok()?.saturating_add(1),
            column,
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
    // `/C:/x` is how a Windows path comes back; a Unix one keeps its leading slash.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => &path[..],
    };
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests;

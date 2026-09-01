//! A scratchpad: one Rust source file, the cargo package generated around it, and the
//! `cargo build` that turns the two into something this app can open.
//!
//! The generated package is the storage — [`Scratchpad::load_from`] is the exact inverse
//! of [`Scratchpad::write_to`], so there is no second format to disagree with what cargo
//! is handed. [`Scratchpad::opened_in`], [`Scratchpad::write_to`] and
//! [`Scratchpad::build_in`] block and [`run_in`] forks, so all four belong on a worker
//! thread.

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    ffi::OsString,
    fmt, fs,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::project::{base, write_atomically};

const SCRATCHPADS_DIR: &str = "scratchpads";

const MANIFEST_NAME: &str = "Cargo.toml";
const SOURCE_DIR: &str = "src";
const SOURCE_NAME: &str = "main.rs";

/// Pinned rather than left to cargo's default, so a scratchpad written today still
/// compiles the way it did when a later cargo changes what a new package gets.
const EDITION: &str = "2021";
const PACKAGE_VERSION: &str = "0.1.0";

/// crates.io's own limit.
const MAX_NAME: usize = 64;

/// How much of one line of a program's output is kept before it is cut and continued on
/// the next: a program writing megabytes with no newline in them is still *delivered*
/// rather than accumulated into a string nobody ever sees.
const MAX_LINE: u64 = 4096;

/// How many lines of a program's output are kept, oldest first out. A line cap and not a
/// byte cap, because the view is a list of rows; [`RunOutput::dropped`] is what lets it
/// say the story is missing its beginning.
const MAX_OUTPUT_LINES: usize = 5000;

/// How often a program whose output has ended is asked whether it has exited. Polled
/// rather than waited on: a blocking `wait` needs the `Child`, and holding it is what
/// would make [`Running::stop`] wait for the process it is trying to kill.
const REAP_POLL: Duration = Duration::from_millis(20);

/// What the one scratchpad the app opens is called, and so the directory it lives in.
/// Checked against [`check_name`] by a test, which is what lets [`Scratchpad::default`]
/// hand it out without a `Result`.
pub const DEFAULT_NAME: &str = "scratch";

/// What a new scratchpad starts with. `#[inline(never)]` because the point of a scratchpad
/// is a symbol of the reader's own in the assembly pane.
pub const DEFAULT_SOURCE: &str = "\
#[inline(never)]
pub fn scratch(x: u64) -> u64 {
    x * 3 + 1
}

fn main() {
    println!(\"{}\", scratch(2));
}
";

/// One scratchpad: the package's name, the source it holds, and the crates it asks for.
///
/// The name is both the crate name and the directory name, and the crate-name rules are
/// strictly stronger than what a safe path component needs, so validating once covers
/// both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scratchpad {
    name: String,
    pub source: String,
    /// In the order the reader put them in — the manifest sorts them, this list does not,
    /// since reordering under an edit is the one thing a list of text boxes must not do.
    pub dependencies: Vec<Dependency>,
}

/// One `[dependencies]` row. Both halves are the raw text of a box the reader is typing
/// in, so both are trimmed at the accessor rather than on the way in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

/// What is wrong with one dependency row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    NoName,
    /// A crate name begins with a letter.
    NameStart,
    NameCharacter(char),
    NameTooLong,
    /// Two rows naming the same crate. `[dependencies]` is a table, so the second would
    /// silently replace the first.
    Repeated,
    NoVersion,
    /// `*`, `1.*`, `>=1, <2.*` — a requirement whose answer changes with the day.
    Wildcard,
    NotAVersion,
}

/// Which of a row's two boxes a [`Problem`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Name,
    Version,
}

/// Why nothing was written or nothing was built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Rows to fix first, each an index into [`Scratchpad::dependencies`].
    Dependencies(Vec<(usize, Problem)>),
    /// No state directory and no local data directory on this system.
    NoDirectory,
    Write(String),
    /// `cargo` could not be started at all — not on the `PATH`, or not executable.
    NoCargo(String),
    /// cargo reported success and named no executable.
    NoArtifact,
    /// The built program could not be started — deleted since the build, or on a
    /// filesystem mounted `noexec`.
    NoProgram(String),
}

/// What a build came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Build {
    /// `diagnostics` is whatever cargo said on the way — warnings and notes.
    Built {
        executable: PathBuf,
        diagnostics: Vec<Diagnostic>,
    },
    /// cargo ran and refused. `message` is cargo's own stderr, which is the only place
    /// some failures are said at all: `no matching package named ... found` for a
    /// dependency row that does not resolve, and a manifest error, both arrive with no
    /// compiler diagnostics behind them.
    Rejected {
        diagnostics: Vec<Diagnostic>,
        message: String,
    },
    /// Nothing was compiled.
    Unavailable(Failure),
}

/// One thing the compiler said, flattened out of cargo's JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    pub message: String,
    /// cargo's own rendered block, asked for without colour.
    pub rendered: String,
    /// The primary span, since a UI marking every span of every diagnostic marks most of
    /// the file.
    pub span: Option<Span>,
}

/// A place in a file the compiler named. `file` is as cargo gave it — `src/main.rs` for
/// the scratchpad's own source, a registry path for a dependency's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub file: String,
    /// One-based, as rustc counts.
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    /// Everything else rustc emits — `note`, `help`, `failure-note`.
    Note,
}

impl Problem {
    pub fn half(&self) -> Half {
        match self {
            Problem::NoName
            | Problem::NameStart
            | Problem::NameCharacter(_)
            | Problem::NameTooLong
            | Problem::Repeated => Half::Name,
            Problem::NoVersion | Problem::Wildcard | Problem::NotAVersion => Half::Version,
        }
    }
}

impl Dependency {
    pub fn name(&self) -> &str {
        self.name.trim()
    }

    pub fn version(&self) -> &str {
        self.version.trim()
    }

    /// What is wrong with this row on its own. [`Problem::Repeated`] is a property of the
    /// list and is answered by [`Scratchpad::problems`].
    pub fn check(&self) -> Result<(), Problem> {
        check_name(self.name())?;
        check_version(self.version())
    }
}

impl Default for Scratchpad {
    fn default() -> Scratchpad {
        Scratchpad::new(DEFAULT_NAME).expect("DEFAULT_NAME is a crate name")
    }
}

impl Scratchpad {
    pub fn new(name: impl Into<String>) -> Result<Scratchpad, Problem> {
        let name = name.into();
        check_name(name.trim())?;
        Ok(Scratchpad {
            name: name.trim().to_owned(),
            source: DEFAULT_SOURCE.to_owned(),
            dependencies: Vec::new(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every dependency row that cannot be written, in list order — all of them, since
    /// the editor marks rows.
    pub fn problems(&self) -> Vec<(usize, Problem)> {
        let mut seen = HashSet::new();
        let mut problems = Vec::new();
        for (row, dependency) in self.dependencies.iter().enumerate() {
            match dependency.check() {
                Err(problem) => problems.push((row, problem)),
                // Only a row that is otherwise good can be a duplicate: two empty rows
                // are two empty rows.
                Ok(()) if !seen.insert(dependency.name()) => {
                    problems.push((row, Problem::Repeated))
                }
                Ok(()) => {}
            }
        }
        problems
    }

    /// The `Cargo.toml` this scratchpad generates, as text. The empty `[workspace]` makes
    /// the package its own workspace root wherever the state directory turns out to be.
    pub fn manifest(&self) -> Result<String, Failure> {
        let problems = self.problems();
        if !problems.is_empty() {
            return Err(Failure::Dependencies(problems));
        }

        let manifest = Manifest {
            package: Package {
                name: self.name.clone(),
                version: PACKAGE_VERSION.to_owned(),
                edition: EDITION.to_owned(),
            },
            dependencies: self
                .dependencies
                .iter()
                .map(|row| (row.name().to_owned(), row.version().to_owned()))
                .collect(),
            workspace: Workspace {},
        };

        toml::to_string_pretty(&manifest).map_err(|error| Failure::Write(error.to_string()))
    }

    /// Where this scratchpad lives, or `None` on a system with no state or local data
    /// directory to put it in.
    pub fn directory(&self) -> Option<PathBuf> {
        Some(base()?.join(SCRATCHPADS_DIR).join(&self.name))
    }

    /// Write the package into `directory`, creating it and its `src/`.
    ///
    /// Both files go down through `.tmp` + rename, which `src/main.rs` earns: it is the
    /// reader's document, so an interrupted write must leave the last good version behind.
    /// The manifest goes first, so a directory that exists at all is a package.
    pub fn write_to(&self, directory: &Path) -> Result<(), Failure> {
        let manifest = self.manifest()?;
        let source = directory.join(SOURCE_DIR);

        let write = || -> io::Result<()> {
            fs::create_dir_all(&source)?;
            write_atomically(&directory.join(MANIFEST_NAME), manifest.as_bytes())?;
            write_atomically(&source.join(SOURCE_NAME), self.source.as_bytes())
        };
        write().map_err(|error| Failure::Write(error.to_string()))
    }

    /// Read a scratchpad back out of its directory, or `None` if there is not one there.
    ///
    /// The exact inverse of [`Scratchpad::write_to`] and nothing more: a manifest naming a
    /// dependency this module would refuse to write reads back as those rows, so a
    /// hand-edited scratchpad opens with the bad row visible rather than not opening.
    pub fn load_from(directory: &Path) -> Option<Scratchpad> {
        let manifest = fs::read_to_string(directory.join(MANIFEST_NAME)).ok()?;
        let manifest: Manifest = toml::from_str(&manifest).ok()?;
        let source = fs::read_to_string(directory.join(SOURCE_DIR).join(SOURCE_NAME)).ok()?;

        Some(Scratchpad {
            name: manifest.package.name,
            source,
            dependencies: manifest
                .dependencies
                .into_iter()
                .map(|(name, version)| Dependency { name, version })
                .collect(),
        })
    }

    /// This scratchpad as `directory` has it, or this one where there is nothing there.
    /// Blocking.
    ///
    /// The name kept is **this** scratchpad's and never the manifest's: `directory()` is
    /// derived from the name, so a hand-edited `Cargo.toml` naming another crate would
    /// otherwise send the next write somewhere the reader never opened.
    pub fn opened_in(self, directory: &Path) -> Scratchpad {
        match Scratchpad::load_from(directory) {
            Some(loaded) => Scratchpad {
                name: self.name,
                ..loaded
            },
            None => self,
        }
    }

    /// Write the package into `directory` and build it. Blocking, and never from a UI
    /// thread.
    ///
    /// The subprocess gets a null stdin, so a cargo that decides to ask a question cannot
    /// sit waiting for an answer no one can give it.
    pub fn build_in(&self, directory: &Path) -> Build {
        if let Err(failure) = self.write_to(directory) {
            return Build::Unavailable(failure);
        }

        let output =
            Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
                .current_dir(directory)
                // The artifact path is *asked for* rather than guessed at: deriving
                // `target/debug/<name>` from the crate name and the profile is silently wrong
                // under a `CARGO_TARGET_DIR`, a `.cargo/config` above the directory, or an
                // executable suffix. cargo names it.
                .args(["build", "--message-format=json", "--color=never"])
                .stdin(Stdio::null())
                .output();

        let output = match output {
            Ok(output) => output,
            Err(error) => return Build::Unavailable(Failure::NoCargo(error.to_string())),
        };

        outcome(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.success(),
        )
    }
}

/// Start the program a build made in `directory`, streaming what it writes into `emit`
/// until it ends.
///
/// The artifact is run, not `cargo run`: re-entering cargo would rebuild to a path that
/// may differ from the one the diagnostics on screen are about, interleave cargo's own
/// progress into the program's output, and make stopping meaningless — killing a
/// `cargo run` leaves its child running. Not blocking: it forks, wires up two threads to
/// the process's two pipes, and returns.
pub fn run_in(
    executable: &Path,
    directory: &Path,
    emit: impl FnMut(RunEvent) + Send + 'static,
) -> Result<Running, Failure> {
    let mut child = Command::new(executable)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failure::NoProgram(error.to_string()))?;

    // Taken before the child goes behind the mutex, since a reader thread owns its pipe
    // outright and must never need the lock a stop is waiting on.
    let out = child.stdout.take();
    let err = child.stderr.take();

    let process = Arc::new(Process {
        child: Mutex::new(child),
        over: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
    });
    let running = Running(process.clone());
    {
        let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
        list.retain(|other| !other.finished());
        list.push(running.clone());
    }

    // One `emit` behind one lock, so the two streams interleave in the order the program
    // wrote them. Holding it across the call is deliberate: a consumer that has fallen
    // behind blocks a reader thread, which fills a pipe, which blocks the program itself —
    // the only backpressure there is against a program printing in a tight loop.
    let emit: Emit = Arc::new(Mutex::new(Box::new(emit)));

    // A run is over when both pipes have reached the end **and** the process has been
    // reaped, and the last pipe to finish says so. A program that hands its output to a
    // grandchild outliving it therefore reads as still running, which is honest: the
    // output is still coming.
    let unfinished = Arc::new(AtomicUsize::new(2));
    pipe_thread(out, Stream::Out, &emit, &unfinished, &process);
    pipe_thread(err, Stream::Err, &emit, &unfinished, &process);

    Ok(running)
}

/// Stop every program any scratchpad has started and that has not ended by itself.
///
/// For the window's close hook, which is a `Send` callback that can read no `State` —
/// `project.rs`'s `flush` is there for the same reason. A child outliving the app holds a
/// terminal, a port or a file the next run will want, with nothing able to find it again.
pub fn stop_all() {
    let running = {
        let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
        std::mem::take(&mut *list)
    };
    for running in running {
        running.stop();
    }
}

/// Which of a program's two output streams a line came from. `stderr` is not an error, it
/// is the other stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// One line a running program wrote. The text is an `Arc<str>` because the app keeps
/// thousands of these in a value it clones whenever a line is added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: Arc<str>,
}

/// What a running program has written, bounded by [`MAX_OUTPUT_LINES`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOutput {
    lines: VecDeque<OutputLine>,
    dropped: usize,
}

impl RunOutput {
    /// Keep one more line, letting the oldest go if that is what it costs.
    pub fn push(&mut self, line: OutputLine) {
        if self.lines.len() >= MAX_OUTPUT_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The line at `index`, counting from the oldest one still kept.
    pub fn line(&self, index: usize) -> Option<&OutputLine> {
        self.lines.get(index)
    }

    /// How many lines were let go to make room, so the view can say the story is missing
    /// its beginning.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

/// What a run says as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Wrote(OutputLine),
    /// The last thing any run says, and it is said exactly once.
    Ended(Ended),
}

/// How a run finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The program returned by itself. `None` where the system ended it without a code.
    Exited(Option<i32>),
    /// [`Running::stop`] was asked for.
    Stopped,
    /// It could not be waited for.
    Failed(String),
}

/// A program that was started, and the only thing that can stop it. Cloneable and cheap:
/// the app holds one in a state it clones on every render and [`stop_all`] holds another.
#[derive(Clone)]
pub struct Running(Arc<Process>);

impl Running {
    /// Kill it — `SIGKILL` on Unix, `TerminateProcess` on Windows.
    ///
    /// Dropping the handle would do nothing: `Child`'s own `Drop` neither waits nor kills,
    /// so a run abandoned rather than stopped goes on running with nothing left that could
    /// find it. What this does not reach is a *grandchild*, which would need the run in a
    /// process group of its own and a `libc` this crate does not carry.
    ///
    /// Ends immediately if the run is already over, so a stop that races an exit cannot
    /// name a pid the system has since given to somebody else.
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        if self.0.over.load(Ordering::SeqCst) {
            return;
        }

        let mut child = self.0.child.lock().unwrap_or_else(|held| held.into_inner());
        let _ = child.kill();
    }

    /// Whether it has ended and been reaped.
    pub fn finished(&self) -> bool {
        self.0.over.load(Ordering::SeqCst)
    }
}

/// The process behind a [`Running`], shared by the handle, the two pipe threads and the
/// list [`stop_all`] walks.
struct Process {
    /// Behind a `Mutex` because a stop and the reap race by construction; every operation
    /// taken under it is a syscall that returns at once (see [`REAP_POLL`]).
    child: Mutex<Child>,
    over: AtomicBool,
    stopped: AtomicBool,
}

impl Process {
    /// Wait for the process to be gone, and say how it went.
    fn reap(&self) -> Ended {
        loop {
            {
                let mut child = self.child.lock().unwrap_or_else(|held| held.into_inner());
                match child.try_wait() {
                    Ok(Some(status)) => {
                        self.over.store(true, Ordering::SeqCst);
                        return match self.stopped.load(Ordering::SeqCst) {
                            true => Ended::Stopped,
                            false => Ended::Exited(status.code()),
                        };
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.over.store(true, Ordering::SeqCst);
                        return Ended::Failed(error.to_string());
                    }
                }
            }
            thread::sleep(REAP_POLL);
        }
    }
}

/// One boxed callback behind one lock: what both pipe threads write a line through.
type Emit = Arc<Mutex<Box<dyn FnMut(RunEvent) + Send>>>;

/// Read one of the process's pipes on a thread of its own, and let the last of the two to
/// finish say the run is over.
fn pipe_thread<R: Read + Send + 'static>(
    pipe: Option<R>,
    stream: Stream,
    emit: &Emit,
    unfinished: &Arc<AtomicUsize>,
    process: &Arc<Process>,
) {
    let (emit, unfinished, process) = (emit.clone(), unfinished.clone(), process.clone());
    thread::spawn(move || {
        // A pipe that is not there is a pipe with nothing on it, which keeps the
        // two-must-finish count honest either way.
        if let Some(pipe) = pipe {
            stream_lines(BufReader::new(pipe), stream, |line| {
                let mut emit = emit.lock().unwrap_or_else(|held| held.into_inner());
                emit(RunEvent::Wrote(line));
            });
        }

        if unfinished.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }

        let ended = process.reap();
        {
            let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
            list.retain(|other| !Arc::ptr_eq(&other.0, &process));
        }
        let mut emit = emit.lock().unwrap_or_else(|held| held.into_inner());
        emit(RunEvent::Ended(ended));
    });
}

/// Split what a program writes into lines and hand each one over as it arrives, cut at
/// [`MAX_LINE`]. Invalid UTF-8 is taken lossily: what a program writes is not this app's
/// to reject.
fn stream_lines(mut reader: impl BufRead, stream: Stream, mut emit: impl FnMut(OutputLine)) {
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader
            .by_ref()
            .take(MAX_LINE)
            .read_until(b'\n', &mut buffer)
        {
            // The end of the pipe, or a pipe that will not read: either way there is
            // nothing more to say.
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        // The terminator, and a `\r` in front of it: the rows are drawn one line each, so
        // a carriage return left in would be a control character in the middle of a label.
        while matches!(buffer.last(), Some(b'\n' | b'\r')) {
            buffer.pop();
        }

        emit(OutputLine {
            stream,
            text: Arc::from(String::from_utf8_lossy(&buffer).as_ref()),
        });
    }
}

/// Everything started and not yet ended, so that [`stop_all`] can reach it without a
/// handle. A `static` because the window's close hook can be handed nothing.
static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

/// Turn one `cargo build --message-format=json` run into an answer. A pure function of
/// what cargo said, so a failed build is a test over a canned stream.
///
/// Every line of stdout is one JSON message and cargo's own progress goes to stderr, so a
/// line that does not parse is skipped rather than failed on.
fn outcome(stdout: &str, stderr: &str, success: bool) -> Build {
    let mut diagnostics = Vec::new();
    let mut executable = None;

    for line in stdout.lines() {
        match serde_json::from_str::<Report>(line) {
            Ok(Report::Message { message }) => diagnostics.push(message.into()),
            // The generated package has one binary and cargo builds no dependency's
            // binaries, so the one artifact carrying an executable is ours; a build
            // script's has none. A rebuild that recompiled nothing still names it.
            Ok(Report::Artifact {
                executable: Some(path),
            }) => executable = Some(path),
            _ => {}
        }
    }

    if !success {
        return Build::Rejected {
            diagnostics,
            message: stderr.trim().to_owned(),
        };
    }

    match executable {
        Some(executable) => Build::Built {
            executable,
            diagnostics,
        },
        None => Build::Unavailable(Failure::NoArtifact),
    }
}

/// The messages cargo emits, as much of each as this module reads. `#[serde(other)]` is
/// what makes an unknown `reason` a message that is skipped rather than a parse failure
/// that would throw the artifact path away with it.
#[derive(Deserialize)]
#[serde(tag = "reason")]
enum Report {
    #[serde(rename = "compiler-message")]
    Message { message: CompilerMessage },
    #[serde(rename = "compiler-artifact")]
    Artifact { executable: Option<PathBuf> },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct CompilerMessage {
    level: String,
    message: String,
    #[serde(default)]
    rendered: Option<String>,
    #[serde(default)]
    spans: Vec<MessageSpan>,
}

#[derive(Deserialize)]
struct MessageSpan {
    file_name: String,
    line_start: usize,
    column_start: usize,
    #[serde(default)]
    is_primary: bool,
}

impl From<CompilerMessage> for Diagnostic {
    fn from(message: CompilerMessage) -> Diagnostic {
        // The primary span, falling back to the first: a diagnostic with secondary spans
        // and no primary is rare, and pointing at one of them beats pointing nowhere.
        let span = message
            .spans
            .iter()
            .find(|span| span.is_primary)
            .or(message.spans.first())
            .map(|span| Span {
                file: span.file_name.clone(),
                line: span.line_start,
                column: span.column_start,
            });

        // `starts_with`, because rustc spells an ICE `error: internal compiler error` and
        // a lint's level can arrive as `warning: ...`.
        let level = if message.level.starts_with("error") {
            Level::Error
        } else if message.level.starts_with("warning") {
            Level::Warning
        } else {
            Level::Note
        };

        Diagnostic {
            level,
            rendered: message.rendered.unwrap_or_else(|| message.message.clone()),
            message: message.message,
            span,
        }
    }
}

/// The generated manifest, as a serializable shape. **Field order is load-bearing**: TOML
/// cannot reopen a table once a later one has begun, so every plain value of a table must
/// be emitted before its first sub-table.
#[derive(Serialize, Deserialize)]
struct Manifest {
    package: Package,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    dependencies: BTreeMap<String, String>,
    /// `default` on the way back in, since a hand-edited scratchpad that dropped it is
    /// still a scratchpad.
    #[serde(default)]
    workspace: Workspace,
}

#[derive(Serialize, Deserialize)]
struct Package {
    name: String,
    version: String,
    edition: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Workspace {}

/// Whether `name` could be a crate name — a letter, then letters, digits, `-` and `_`.
///
/// Deliberately the rule for a name crates.io could hold rather than the looser one cargo
/// would accept, since every one of these names is about to be asked for at a registry.
fn check_name(name: &str) -> Result<(), Problem> {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(Problem::NoName);
    };
    if name.len() > MAX_NAME {
        return Err(Problem::NameTooLong);
    }
    if !first.is_ascii_alphabetic() {
        return Err(Problem::NameStart);
    }
    match characters.find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_')) {
        Some(character) => Err(Problem::NameCharacter(character)),
        None => Ok(()),
    }
}

/// Whether `version` could be a cargo version requirement: a comma-separated list of
/// comparators, each an optional operator in front of a version. The *shape* only —
/// whether it resolves is a question about the registry.
///
/// A wildcard is refused outright though cargo takes it: it is what makes a scratchpad
/// build differently on a different day, and it gets its own [`Problem`] so the row can
/// say that rather than "not a version".
fn check_version(version: &str) -> Result<(), Problem> {
    if version.is_empty() {
        return Err(Problem::NoVersion);
    }
    if version.contains('*') {
        return Err(Problem::Wildcard);
    }
    for comparator in version.split(',') {
        check_comparator(comparator.trim())?;
    }
    Ok(())
}

fn check_comparator(comparator: &str) -> Result<(), Problem> {
    // Longest first, or `>=1` would be read as `>` in front of `=1`.
    let version = [">=", "<=", "^", "~", "=", ">", "<"]
        .into_iter()
        .find_map(|operator| comparator.strip_prefix(operator))
        .unwrap_or(comparator)
        .trim_start();

    // `1.2.3-rc.1+build`: the core is what has to be numbers, and the rest is an
    // identifier charset. Splitting on the first of the two markers is what keeps a
    // pre-release's own `-` out of the core.
    let (core, tail) = match version.find(['-', '+']) {
        Some(marker) => version.split_at(marker),
        None => (version, ""),
    };

    let mut parts = core.split('.');
    let mut counted = 0;
    for part in &mut parts {
        counted += 1;
        if counted > 3 || part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Problem::NotAVersion);
        }
    }
    if counted == 0 {
        return Err(Problem::NotAVersion);
    }

    // The tail is `-<identifiers>`, `+<identifiers>` or both, and an identifier is
    // alphanumerics, `-` and `.`. A tail that is only its marker is a half-typed one.
    let bad_tail = tail.len() == 1
        || tail
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'+')));
    if bad_tail {
        return Err(Problem::NotAVersion);
    }

    Ok(())
}

impl fmt::Display for Problem {
    /// One sentence per row, shown beside the row it belongs to — so none of them names
    /// the crate or repeats the value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoName => write!(formatter, "name a crate"),
            Problem::NameStart => write!(formatter, "a crate name starts with a letter"),
            Problem::NameCharacter(character) => write!(
                formatter,
                "a crate name holds letters, digits, - and _, not {character:?}"
            ),
            Problem::NameTooLong => write!(formatter, "no crate name is longer than {MAX_NAME}"),
            Problem::Repeated => write!(formatter, "this crate is already asked for above"),
            Problem::NoVersion => write!(
                formatter,
                "require a version, so this builds the same way twice"
            ),
            Problem::Wildcard => write!(
                formatter,
                "a wildcard builds differently on a different day; require a version"
            ),
            Problem::NotAVersion => write!(formatter, "not a version, such as 1, 1.2 or ^1.2.3"),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The rows say the detail; this is the sentence over the top of them.
            Failure::Dependencies(problems) => match problems.len() {
                1 => write!(formatter, "1 dependency to fix"),
                count => write!(formatter, "{count} dependencies to fix"),
            },
            Failure::NoDirectory => write!(formatter, "nowhere to keep a scratchpad"),
            Failure::Write(error) => write!(formatter, "could not write the package: {error}"),
            Failure::NoCargo(error) => write!(formatter, "could not run cargo: {error}"),
            Failure::NoArtifact => write!(formatter, "cargo built nothing to open"),
            Failure::NoProgram(error) => write!(formatter, "could not start it: {error}"),
        }
    }
}

#[cfg(test)]
mod tests;

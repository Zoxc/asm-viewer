//! A scratchpad: one Rust source file, the cargo package generated around it, and the
//! `cargo build` that turns the two into something this app can open.
//!
//! Framework-free — no freya types appear here — like `project.rs` and `settings.rs`
//! beside it, so the whole of it is unit-tested without a window. What is *not* here is
//! any UI: the editable source view, the dependency rows on screen and the build button
//! are the Scratchpad view in `ui.rs`, and every one of them reads this and nothing else
//! about what a scratchpad is. Three of the five functions it calls are documented here
//! as never running on a UI thread — [`Scratchpad::opened`], [`Scratchpad::write`] and
//! [`Scratchpad::build`] — which is why the view has a worker thread of its own and why
//! that thread is the only thing that ever touches a scratchpad's directory. The fourth,
//! [`Scratchpad::run`], goes to the same thread for a weaker reason: it does not block,
//! but it forks, and the one thread that draws has no business doing that either.
//!
//! **A program the reader wrote is one that may never stop**, so what [`Scratchpad::run`]
//! hands back is a [`Running`] — a handle whose whole purpose is [`Running::stop`], which
//! kills the process rather than dropping anything. Its output is *streamed* through a
//! callback as it is written rather than collected and returned the way a build's is: a
//! program that prints and then loops for ever has said something, and a shape that
//! answers only at exit would never say it. Everything still running is also reachable
//! without a handle, through [`stop_all`], because the window's close hook can read no
//! state — the same reason `project.rs` keeps its save policy in a `static`.
//!
//! **The generated package is the storage.** A scratchpad is a name, a source and a list
//! of `(crate, version)` rows, and every one of those is already a field of the package
//! it generates — the package name, `src/main.rs`, and `[dependencies]`. So nothing here
//! writes a second file describing a scratchpad, and [`Scratchpad::load_from`] is the
//! exact inverse of [`Scratchpad::write_to`] rather than a parallel format that could
//! disagree with what cargo is actually handed. It also means the reader can open a
//! scratchpad in any other editor and lose nothing.
//!
//! **A version is required on every row**, which is the point of the goal rather than a
//! detail of it: a scratchpad is a thing you come back to, and `*` means "whatever was
//! newest the day you built it". A requirement plus the `Cargo.lock` cargo leaves in the
//! scratchpad's own directory is what makes the second build the same as the first.
//!
//! **Rows are validated here and never at crates.io.** What this module can answer is
//! whether a row is a *possible* crate name and a *possible* version requirement — both
//! are grammars, and both are what a half-typed row fails. Whether a crate by that name
//! exists at that version is cargo's answer and arrives as a [`Build::Rejected`] with
//! cargo's own words in it; asking the network here would put a spinner and a failure
//! mode into a text box.

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

/// The directory this app keeps its state in, the same one `project.rs` and
/// `settings.rs` name, and a `scratchpads/` under it — one directory per scratchpad.
/// Spelled out again rather than shared, for their reason: the three modules are meant
/// to be separable and a constant is a cheaper duplicate than a dependency between them.
const APP_DIR: &str = "assembly-viewer";
const SCRATCHPADS_DIR: &str = "scratchpads";

const MANIFEST_NAME: &str = "Cargo.toml";
const SOURCE_DIR: &str = "src";
const SOURCE_NAME: &str = "main.rs";

/// Pinned rather than left to cargo's default, so a scratchpad written today still
/// compiles the way it did when a later cargo changes what a new package gets.
const EDITION: &str = "2021";
const PACKAGE_VERSION: &str = "0.1.0";

/// crates.io's own limit, and the only length rule worth having: a name longer than this
/// cannot be a crate whatever else is true of it.
const MAX_NAME: usize = 64;

/// How much of one line of a program's output is kept before it is cut and continued on
/// the next.
///
/// The bound that has to exist *first*, and it is about a single `String` rather than
/// about the list: a program that writes a gigabyte and never a newline would otherwise
/// grow one line until the machine gave out, and `read_line` would never hand it over on
/// the way. 4 KiB is far past any line a person reads and far short of a problem.
const MAX_LINE: u64 = 4096;

/// How many lines of a program's output are kept.
///
/// A line cap and not a byte cap, because what the view is is a list of rows and what a
/// cap has to answer is "how many rows" — a byte budget would make the row count a
/// function of how long the lines happened to be, so the same tight loop would keep
/// twenty thousand short lines and two hundred long ones. With [`MAX_LINE`] above it the
/// two together bound the memory anyway, at a worst case of about 20 MB and a realistic
/// one of a few hundred KB. What is dropped is the *oldest*, because the interesting end
/// of a program that will not stop is the end it is still writing; [`RunOutput::dropped`]
/// is what lets the view say so rather than silently showing a truncated story.
const MAX_OUTPUT_LINES: usize = 5000;

/// How often a program whose output has ended is asked whether it has exited.
///
/// Polled rather than waited on, and the reason is [`Running::stop`]: a blocking `wait`
/// needs the `Child`, and holding it is exactly what would make a stop wait for the
/// process it is trying to kill. Normally the first ask succeeds — both pipes reaching
/// the end *is* the process exiting — so this interval is only paid by a program that
/// closes its own output and lives on.
const REAP_POLL: Duration = Duration::from_millis(20);

/// What the one scratchpad the app opens is called, and so the directory it lives in.
///
/// A constant rather than something the reader names, because 10c ships one scratchpad
/// and not a list of them: a name that can be edited is a name that can be edited into
/// another scratchpad's directory, which is a picker and a rename with nowhere yet to
/// draw either. Everything below is written in terms of a name all the same -- the model
/// has always held several -- so the picker, when it comes, adds a list and changes
/// nothing here. It is checked against [`check_name`] by a test, which is what lets
/// [`Scratchpad::default`] hand it out without a `Result`.
pub const DEFAULT_NAME: &str = "scratch";

/// What a new scratchpad starts with.
///
/// Not `fn main() {}`, which is the obvious answer and the useless one: a scratchpad
/// exists to be *looked at* in the assembly pane, and an empty `main` compiles to a
/// symbol the reader did not write. One named function with `#[inline(never)]` on it is
/// the smallest thing that puts something of theirs in the listing, and the attribute is
/// there because the first edit is usually to make the body more interesting — at which
/// point an inlined one-liner would vanish into `main` and read as the build being
/// broken.
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
/// The name is both the crate name and the directory name, and [`Scratchpad::new`]
/// checks it against the crate-name rules — which are strictly stronger than what a safe
/// path component needs (no separators, no `.`, no `..`, ASCII only), so validating once
/// covers both and there is no second place a name could get through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scratchpad {
    name: String,
    /// The whole of `src/main.rs`. Public because the editor owns it: this module has no
    /// opinion about Rust source beyond writing it out verbatim.
    pub source: String,
    /// `[dependencies]`, in the order the reader put them in — the manifest sorts them,
    /// this list does not, because a row is a place on screen and reordering under an
    /// edit is the one thing a list of text boxes must not do.
    pub dependencies: Vec<Dependency>,
}

/// One `[dependencies]` row: a crate and the version required of it.
///
/// Both halves are the raw text of a box the reader is typing in, so both are trimmed at
/// the accessor rather than on the way in (`settings.rs` does the same with a font
/// family): a row is edited far more often than it is read, and a half-typed row must
/// stay exactly what was typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

/// What is wrong with one dependency row.
///
/// A value and not a `bool`, because the whole point of validating locally is that the
/// UI can say *why* against the row that is wrong — a row that is silently dropped from
/// the generated manifest is a scratchpad that builds differently from the one on
/// screen, which is the failure this is here to prevent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The row names no crate at all.
    NoName,
    /// A crate name begins with a letter; cargo's own `restricted_names` says so, and a
    /// leading digit or `-` is what a half-typed version in the wrong box looks like.
    NameStart,
    /// A character no crate name may hold.
    NameCharacter(char),
    NameTooLong,
    /// Two rows naming the same crate. `[dependencies]` is a table, so the second would
    /// silently replace the first — the omission this module refuses to make quietly.
    Repeated,
    /// The row requires no version. Required, not defaulted: see the module docs.
    NoVersion,
    /// `*`, `1.*`, `>=1, <2.*` — a requirement whose answer changes with the day.
    Wildcard,
    /// Not a version requirement at all.
    NotAVersion,
}

/// Which of a row's two boxes a [`Problem`] is about.
///
/// A value and not a guess at the editor: the rows are two text boxes and every problem
/// is about exactly one of them, so which box to mark is a property of the problem
/// rather than something a UI can work out from its wording. [`Problem::Repeated`] is
/// the one that is not obvious -- two rows asking for one crate at two versions is a
/// *name* collision, since `[dependencies]` is keyed by the name and the second version
/// is what would be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    Name,
    Version,
}

/// Why nothing was written or nothing was built. Every variant is something the reader
/// can be shown as it stands; none of them is a panic and none of them is a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Rows that have to be fixed first, each as an index into
    /// [`Scratchpad::dependencies`] so the editor can point at the row rather than at
    /// the build.
    Dependencies(Vec<(usize, Problem)>),
    /// No state directory and no local data directory on this system, so there is
    /// nowhere a scratchpad may live. The same answer `project.rs` gives.
    NoDirectory,
    /// The package could not be written.
    Write(String),
    /// `cargo` could not be started at all — not on the `PATH`, or not executable.
    NoCargo(String),
    /// cargo reported success and named no executable, which nothing in a generated
    /// package should be able to do. A third answer rather than an `unwrap`.
    NoArtifact,
    /// The built program could not be started at all — deleted since the build, or on a
    /// filesystem mounted `noexec`. Distinct from [`Failure::NoCargo`] because it is the
    /// reader's own program and not the toolchain that is missing.
    NoProgram(String),
}

/// What a build came back with.
///
/// Three answers and not two, because "the compiler said no" and "there was no compiler"
/// are different things to a reader: one is a page of diagnostics to read and one is a
/// machine to fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Build {
    /// cargo built it. `diagnostics` is whatever it said on the way — warnings and
    /// notes; a successful build's errors do not exist.
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

/// One thing the compiler said, flattened out of cargo's JSON into what a UI can draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    /// The one-line message, for a list.
    pub message: String,
    /// cargo's own rendered block — the source excerpt with the carets under it — asked
    /// for without colour, so it is text and not escape codes.
    pub rendered: String,
    /// Where it points, when it points anywhere. The primary span, since a UI marking
    /// every span of every diagnostic marks most of the file.
    pub span: Option<Span>,
}

/// A place in a file the compiler named. `file` is as cargo gave it — `src/main.rs` for
/// the scratchpad's own source, and a registry path for a diagnostic from a dependency,
/// which is exactly the distinction a UI needs to decide whether it can point at it.
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
    /// Everything else rustc emits — `note`, `help`, `failure-note`. Not worth a variant
    /// each: nothing decides anything on the difference.
    Note,
}

impl Problem {
    /// Which box of the row this is about.
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
    /// The crate this row asks for, trimmed.
    pub fn name(&self) -> &str {
        self.name.trim()
    }

    /// The version requirement, trimmed.
    pub fn version(&self) -> &str {
        self.version.trim()
    }

    /// What is wrong with this row on its own. [`Problem::Repeated`] is not answered
    /// here and cannot be — it is a property of the list, and [`Scratchpad::problems`]
    /// is where the list is.
    pub fn check(&self) -> Result<(), Problem> {
        check_name(self.name())?;
        check_version(self.version())
    }
}

impl Default for Scratchpad {
    /// The scratchpad the app opens with: [`DEFAULT_NAME`] over [`DEFAULT_SOURCE`], and
    /// no crates asked for.
    ///
    /// No `Result`, unlike [`Scratchpad::new`], because the name is this module's own
    /// constant rather than something typed. The `expect` is the only one in this module
    /// and it is not a claim about anything read from a file: it is
    /// `the_default_scratchpad_is_one_this_module_would_write` that holds it, by putting
    /// `DEFAULT_NAME` through the very check this would fail.
    fn default() -> Scratchpad {
        Scratchpad::new(DEFAULT_NAME).expect("DEFAULT_NAME is a crate name")
    }
}

impl Scratchpad {
    /// A new, empty scratchpad, or the reason its name is not a name.
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

    /// Every dependency row that cannot be written, in list order.
    ///
    /// All of them and not the first: the editor marks rows, and a reader who fixes one
    /// row only to be told about the next is being shown the list one item at a time.
    pub fn problems(&self) -> Vec<(usize, Problem)> {
        let mut seen = HashSet::new();
        let mut problems = Vec::new();
        for (row, dependency) in self.dependencies.iter().enumerate() {
            match dependency.check() {
                Err(problem) => problems.push((row, problem)),
                // Only a row that is otherwise good can be a duplicate: two empty rows
                // are two empty rows, and saying so twice over would be noise.
                Ok(()) if !seen.insert(dependency.name()) => {
                    problems.push((row, Problem::Repeated))
                }
                Ok(()) => {}
            }
        }
        problems
    }

    /// The `Cargo.toml` this scratchpad generates, as text.
    ///
    /// The one interesting thing about it is `[workspace]`: an empty table makes the
    /// package its own workspace root, so a scratchpad that happens to sit under a
    /// directory holding a workspace manifest still builds. Nothing decides where a
    /// state directory is, so that is a guarantee rather than a prediction.
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

        // Nothing here can fail to serialize — it is three strings and a map of strings
        // — but the error is turned into a value rather than unwrapped, for the reason
        // `project.rs` gives: writing a file must not be a way to crash the app.
        toml::to_string_pretty(&manifest).map_err(|error| Failure::Write(error.to_string()))
    }

    /// Where this scratchpad lives, or `None` on a system with no state or local data
    /// directory to put it in.
    pub fn directory(&self) -> Option<PathBuf> {
        Some(scratchpads()?.join(&self.name))
    }

    /// Write the package out where it belongs.
    pub fn write(&self) -> Result<(), Failure> {
        let directory = self.directory().ok_or(Failure::NoDirectory)?;
        self.write_to(&directory)
    }

    /// Write the package into `directory`, creating it and its `src/`.
    ///
    /// Both files go down through the same `.tmp` + rename the session and the settings
    /// use, and here it is the source that earns it: `src/main.rs` is the reader's
    /// document, so an interrupted write must leave the last good version behind rather
    /// than a truncated one. The manifest is written first, so a directory that exists
    /// at all is a package cargo can be pointed at.
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

    /// Read a scratchpad back out of its directory, or `None` if there is not one there:
    /// no manifest, a manifest that is not this app's, or no source beside it.
    ///
    /// The exact inverse of [`Scratchpad::write_to`], and deliberately nothing more —
    /// a manifest naming a dependency this module would refuse to write is still read
    /// back as those rows, so a hand-edited scratchpad opens with the bad row visible
    /// rather than not opening at all.
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

    /// This scratchpad as its own directory has it, or this one where there is nothing
    /// there. **Blocking, and never from a UI thread**, for [`Scratchpad::build`]'s
    /// reason at a much smaller scale: it is two `read`s of a file whose size is the
    /// reader's business, and the one thread that draws has no business waiting on any
    /// of them.
    ///
    /// The name kept is **this** scratchpad's and never the manifest's, which is the one
    /// decision in here. `directory()` is derived from the name, so a hand-edited
    /// `Cargo.toml` naming another crate would otherwise send the next write somewhere
    /// the reader never opened -- and the directory, not the manifest, is what the reader
    /// asked for.
    pub fn opened(self) -> Scratchpad {
        match self.directory() {
            Some(directory) => self.opened_in(&directory),
            None => self,
        }
    }

    /// [`Scratchpad::opened`], from a directory of the caller's choosing. Blocking.
    pub fn opened_in(self, directory: &Path) -> Scratchpad {
        match Scratchpad::load_from(directory) {
            Some(loaded) => Scratchpad {
                name: self.name,
                ..loaded
            },
            None => self,
        }
    }

    /// Write the package and build it. **Blocking, and never from a UI thread.**
    ///
    /// A `cargo build` is seconds at best and a full dependency tree at worst, so this
    /// is the same discipline `analysis::open_files` already follows and for the same
    /// reason: the work is a plain blocking function that hands back a *value*, and
    /// `ui.rs` puts it on a `std::thread` with an `async_channel` to carry the answer
    /// home. Nothing here prints, nothing here holds a lock, and nothing here needs the
    /// caller to be a particular kind of thread — which is exactly what makes it safe to
    /// move off the one thread that must not block.
    ///
    /// The subprocess gets a null stdin, so a cargo that decides to ask a question
    /// cannot sit waiting for an answer no one can give it.
    pub fn build(&self) -> Build {
        match self.directory() {
            Some(directory) => self.build_in(&directory),
            None => Build::Unavailable(Failure::NoDirectory),
        }
    }

    /// [`Scratchpad::build`], in a directory of the caller's choosing. Blocking.
    pub fn build_in(&self, directory: &Path) -> Build {
        if let Err(failure) = self.write_to(directory) {
            return Build::Unavailable(failure);
        }

        let output = Command::new(cargo())
            .current_dir(directory)
            // The artifact path is *asked for* rather than guessed at. Deriving
            // `target/debug/<name>` from the crate name and the profile is wrong the
            // moment there is a `CARGO_TARGET_DIR` in the environment, a `.cargo/config`
            // above the directory, or an executable suffix — and it would be wrong
            // silently, by handing the viewer a path that is not there. cargo names it.
            .args(["build", "--message-format=json", "--color=never"])
            .stdin(Stdio::null())
            .output();

        let output = match output {
            Ok(output) => output,
            Err(error) => return Build::Unavailable(Failure::NoCargo(error.to_string())),
        };

        // Lossy rather than a decode error: a diagnostic quoting a source line this app
        // read as UTF-8 cannot itself be invalid, and a build must not fail on the way
        // its own failure was spelt.
        outcome(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.success(),
        )
    }

    /// Start the program a build made, streaming what it writes into `emit` until it
    /// ends.
    ///
    /// **The artifact is run, not `cargo run`**, and that is the one design decision in
    /// here. `build_in` already asked cargo where it put the executable and cargo already
    /// answered, so re-entering cargo would redo dependency resolution and a build to
    /// arrive back at the path this is handed — and it could arrive at a *different* one,
    /// since the reader has very likely typed since. What runs would then not be what the
    /// diagnostics on screen are about. It is also the only shape in which stopping means
    /// anything: killing a `cargo run` kills cargo, whose child goes on running with
    /// nothing left holding it, whereas the process spawned here **is** the reader's
    /// program. And cargo's own progress lines would arrive interleaved into the stream
    /// the reader is reading as the output of what they wrote.
    ///
    /// The working directory is the scratchpad's own, which is what `cargo run` would
    /// have given it — a program that opens a relative path finds the package it was
    /// written in. stdin is null for [`Scratchpad::build`]'s reason: nothing can answer a
    /// question asked of a process with no terminal.
    pub fn run(
        &self,
        executable: &Path,
        emit: impl FnMut(RunEvent) + Send + 'static,
    ) -> Result<Running, Failure> {
        match self.directory() {
            Some(directory) => run_in(executable, &directory, emit),
            None => Err(Failure::NoDirectory),
        }
    }
}

/// [`Scratchpad::run`], in a directory of the caller's choosing.
///
/// Not blocking, unlike everything else this module hands the worker: it forks, wires up
/// two threads to the process's two pipes, and returns. Every wait after that is on one
/// of those threads.
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
    remember(&running);

    // One `emit` behind one lock, so the two streams interleave in the order the program
    // actually wrote them rather than in two lists a view would have to merge. Holding it
    // across the call is deliberate: the callback is what carries a line to the UI, so a
    // consumer that has fallen behind blocks a reader thread, which fills a pipe, which
    // blocks the program itself. That is the *only* backpressure available against a
    // program printing in a tight loop, and it is exactly the right one — the alternative
    // is a queue that grows as fast as the program can write.
    let emit: Emit = Arc::new(Mutex::new(Box::new(emit)));

    // A run is over when both of its pipes have reached the end **and** the process has
    // been reaped, and the last pipe to finish is what says so. No third thread and no
    // polling in the ordinary case: the pipes end because the process exited. A program
    // that hands its output to a grandchild outliving it is therefore reported as still
    // running until that grandchild lets go, which is the honest answer rather than a
    // wrong one — the output is still coming.
    let unfinished = Arc::new(AtomicUsize::new(2));
    pipe_thread(out, Stream::Out, &emit, &unfinished, &process);
    pipe_thread(err, Stream::Err, &emit, &unfinished, &process);

    Ok(running)
}

/// Stop every program any scratchpad has started and that has not ended by itself.
///
/// For the window's close hook, which is a `Send` callback that can read no `State` —
/// `project.rs`'s `flush` is there for the same reason and this sits beside it. **A child
/// process outliving the app is a bug and not an untidiness**: it holds a terminal, a
/// port or a file the next run will want, and nothing in the app would ever be able to
/// find it again. What it cannot cover is the app being killed rather than closed, where
/// no hook of ours runs at all; that is the same bound `project.rs` accepts for a save.
pub fn stop_all() {
    let running = {
        let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
        std::mem::take(&mut *list)
    };
    for running in running {
        running.stop();
    }
}

/// Which of a program's two output streams a line came from. The whole of what the view
/// needs to draw them differently, and the whole of what this module claims about them:
/// `stderr` is *not* an error, it is the other stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// One line a running program wrote.
///
/// The text is an `Arc<str>` rather than a `String` because the app keeps thousands of
/// these in a value it clones whenever a line is added to it; a clone is then a run of
/// refcount bumps instead of a run of allocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: Arc<str>,
}

/// What a running program has written, bounded.
///
/// Bounded *here*, in the model, and not by the view trimming a list it was handed: how
/// much of a program's output is kept is a decision with a reason (see
/// [`MAX_OUTPUT_LINES`]), and a decision with a reason belongs where it can be tested
/// without a window.
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

    /// How many lines were let go to make room. Not a detail: a reader looking at a list
    /// that starts in the middle of a sentence has to be told that is what it is.
    pub fn dropped(&self) -> usize {
        self.dropped
    }
}

/// What a run says as it goes. Every one of them is something the view appends to what it
/// is already showing, which is what makes streaming a matter of *when* they arrive
/// rather than of what they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Wrote(OutputLine),
    /// The last thing any run says, and it is said exactly once.
    Ended(Ended),
}

/// How a run finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The program returned by itself. `None` where the system ended it without one — a
    /// signal on Unix — which is a different thing from exiting with a code of zero.
    Exited(Option<i32>),
    /// [`Running::stop`] was asked for. Reported as its own answer rather than as the
    /// exit status a killed process leaves behind, because "you stopped it" and "it died"
    /// are different things to the person who pressed the button.
    Stopped,
    /// It could not be waited for. Nothing normal reaches this.
    Failed(String),
}

/// A program that was started, and the only thing that can stop it.
///
/// Cloneable and cheap, because the app holds one of these in a state it clones on every
/// render and [`stop_all`] holds another.
#[derive(Clone)]
pub struct Running(Arc<Process>);

impl Running {
    /// Kill it.
    ///
    /// **This really kills the process** — `SIGKILL` on Unix, `TerminateProcess` on
    /// Windows — rather than dropping the handle, which would do nothing at all: `Child`'s
    /// own `Drop` deliberately does not wait and deliberately does not kill, so a run
    /// abandoned rather than stopped goes on running with nothing left that could ever
    /// find it. What it does not reach is a *grandchild*: a process the reader's program
    /// spawned is not this app's to kill without putting the run in a process group of
    /// its own, which is a platform-specific piece of `libc` this does not carry.
    ///
    /// Ends immediately if the run is already over, so a stop that races an exit cannot
    /// name a pid the system has since given to somebody else — and `Child::kill` refuses
    /// a reaped child for the same reason, so the guard is doubled and neither half is
    /// load-bearing on its own.
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
    /// Behind a `Mutex` because a stop and the reap race by construction, and every
    /// operation taken under it is a syscall that returns at once — which is why the reap
    /// polls rather than waits (see [`REAP_POLL`]).
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
        // `None` cannot happen for a `Stdio::piped()` child that was just spawned, but a
        // pipe that is not there is simply a pipe with nothing on it, and answering that
        // way keeps the two-must-finish count honest either way.
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
        forget(&process);
        let mut emit = emit.lock().unwrap_or_else(|held| held.into_inner());
        emit(RunEvent::Ended(ended));
    });
}

/// Split what a program writes into lines and hand each one over as it arrives.
///
/// A function of its own rather than three lines inside the thread, because both of the
/// bounds that matter are in it and neither needs a process to be tested: a line is cut
/// at [`MAX_LINE`] and continues on the next, so output with no newline in it at all is
/// still *delivered* rather than accumulating for ever, and the delivery is per line
/// rather than per read so the view is told what it has as soon as there is a line of it.
/// Invalid UTF-8 is taken lossily, for the reason a diagnostic is: what a program writes
/// is not this app's to reject.
fn stream_lines(mut reader: impl BufRead, stream: Stream, mut emit: impl FnMut(OutputLine)) {
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader
            .by_ref()
            .take(MAX_LINE)
            .read_until(b'\n', &mut buffer)
        {
            // The end of the pipe, which is the end of the program's output.
            Ok(0) => return,
            Ok(_) => {}
            // A pipe that will not read is one with nothing more to say.
            Err(_) => return,
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
/// handle. A `static` for the same reason `project.rs`'s save policy is one: the window's
/// close hook is outside the component tree and can be handed nothing.
static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

fn remember(running: &Running) {
    let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
    // Anything that ended while nobody was looking. The list is walked on every start and
    // every end, so it is as long as the number of programs running at once, which is one.
    list.retain(|other| !other.finished());
    list.push(running.clone());
}

fn forget(process: &Arc<Process>) {
    let mut list = RUNNING.lock().unwrap_or_else(|held| held.into_inner());
    list.retain(|other| !Arc::ptr_eq(&other.0, process));
}

/// The directory scratchpads live in, beside `settings.toml` and the `projects/`
/// directory.
fn scratchpads() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(APP_DIR).join(SCRATCHPADS_DIR))
}

/// Which cargo to run.
///
/// `$CARGO` is set by cargo itself for anything it launches, so a development build run
/// with `cargo run` builds its scratchpads with the very cargo that built it — the same
/// toolchain, without going back through a rustup shim. An installed copy of the app has
/// no such parent and falls back to the `PATH`, which is where a user's rustup shim is.
fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Write `path` by writing `path.tmp` first and renaming it over the top, so an
/// interrupted write cannot leave a half-written file behind. The same two lines
/// `project.rs` and `settings.rs` each have; they are duplicated rather than shared for
/// the same reason the directory constant is.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}

/// Turn one `cargo build --message-format=json` run into an answer.
///
/// Split out of [`Scratchpad::build_in`] so it is a pure function of what cargo said:
/// what a failed build reports is then a test over a canned stream rather than a test
/// that needs a broken compiler to hand.
///
/// Every line of stdout is one JSON message; cargo's own progress goes to stderr, so a
/// line that does not parse is not something to report, it is a cargo that has started
/// saying something new. Those are skipped rather than failed on.
fn outcome(stdout: &str, stderr: &str, success: bool) -> Build {
    let mut diagnostics = Vec::new();
    let mut executable = None;

    for line in stdout.lines() {
        match serde_json::from_str::<Report>(line) {
            Ok(Report::Message { message }) => diagnostics.push(message.into()),
            // The generated package has exactly one binary and cargo builds no
            // dependency's binaries, so the one artifact carrying an executable is ours.
            // A build script's artifact has none, which is what `Option` is doing here.
            // A rebuild that recompiled nothing still names it (the artifact comes back
            // `fresh`), so pressing build twice is not a build that produced nothing.
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

/// The messages cargo emits, as much of each as this module reads.
///
/// `#[serde(other)]` is what makes an unknown `reason` — `build-script-executed`,
/// `build-finished`, whatever cargo adds next — a message that is skipped rather than a
/// parse failure that would throw away the artifact path with it.
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
        // The primary span, and the first one only as a fallback: a diagnostic with
        // secondary spans and no primary is rare, and pointing at one of them beats
        // pointing nowhere.
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

        Diagnostic {
            level: Level::from(message.level.as_str()),
            rendered: message.rendered.unwrap_or_else(|| message.message.clone()),
            message: message.message,
            span,
        }
    }
}

impl Level {
    fn from(level: &str) -> Level {
        // `starts_with`, because rustc spells an ICE `error: internal compiler error`
        // and a lint's level can arrive as `warning: ...` — both of which are the level
        // they start with.
        if level.starts_with("error") {
            Level::Error
        } else if level.starts_with("warning") {
            Level::Warning
        } else {
            Level::Note
        }
    }
}

/// The generated manifest, as a serializable shape.
///
/// **Field order is load-bearing**, the rule this codebase keeps hitting: TOML cannot
/// reopen a table once a later one has begun, so a serializer must emit every plain
/// value of a table before its first sub-table. Everything here is a table, so the order
/// is only the order the file reads in — but `Package`'s own fields are all plain values
/// and a table added to it later would have to go last. The generated-manifest test is
/// what holds both.
#[derive(Serialize, Deserialize)]
struct Manifest {
    package: Package,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    dependencies: BTreeMap<String, String>,
    /// An empty `[workspace]`, so the package is its own workspace root wherever it
    /// happens to sit. `default` on the way back in, since a hand-edited scratchpad that
    /// dropped it is still a scratchpad.
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

/// Whether `name` could be a crate name — cargo's own rule: a letter, then letters,
/// digits, `-` and `_`.
///
/// It is deliberately the rule for a name crates.io could hold rather than the rule for
/// a name cargo would *accept*, which is looser (a leading `_`, non-ASCII), because
/// every one of these names is about to be asked for at a registry.
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

/// Whether `version` could be a cargo version requirement.
///
/// A requirement is a comma-separated list of comparators, each an optional operator in
/// front of a version. This checks the *shape* of each and nothing about what it would
/// resolve to — which is the honest division of labour, since resolution is a question
/// about the registry and about every other crate in the tree.
///
/// The one judgement in here is that a wildcard is refused outright rather than parsed.
/// `*` is a legal requirement and cargo takes it; it is also precisely the thing this
/// feature exists to prevent, since it makes what a scratchpad builds a function of the
/// day it is built on. It gets its own [`Problem`] so the row can say that rather than
/// "not a version".
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
    /// One sentence per row, written to be shown *beside the row it belongs to* — so
    /// none of them names the crate or repeats the value, which the row is already
    /// showing.
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
            // The rows say the detail; this is the sentence over the top of them, for a
            // place that has no rows to point at.
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
mod tests {
    use super::*;

    /// A directory of this test's own under the system temporary directory, named after
    /// the line that asked for it — `project.rs`'s and `settings.rs`'s file tests do the
    /// same, so a failing test leaves something identifiable behind.
    fn directory(line: u32) -> PathBuf {
        std::env::temp_dir().join(format!(
            "assembly-viewer-scratchpad-test-{}-{line}",
            std::process::id()
        ))
    }

    fn scratchpad() -> Scratchpad {
        Scratchpad::new("sketch").expect("a name")
    }

    /// One row, as a reader would have left the two boxes. A test helper and not a
    /// constructor on [`Dependency`]: the editor builds its rows a field at a time out of
    /// two text boxes, so nothing outside these tests ever has both halves at once.
    fn dependency(name: impl Into<String>, version: impl Into<String>) -> Dependency {
        Dependency {
            name: name.into(),
            version: version.into(),
        }
    }

    #[test]
    fn a_new_scratchpad_is_a_name_and_something_to_look_at() {
        let scratchpad = scratchpad();
        assert_eq!(scratchpad.name(), "sketch");
        assert_eq!(scratchpad.source, DEFAULT_SOURCE);
        assert!(scratchpad.dependencies.is_empty());
        assert!(scratchpad.problems().is_empty());

        // The name is a path component as well as a crate name, and the crate-name rules
        // are what keeps it one.
        assert_eq!(Scratchpad::new("../escape"), Err(Problem::NameStart));
        assert_eq!(Scratchpad::new("a/b"), Err(Problem::NameCharacter('/')));
        assert_eq!(Scratchpad::new(""), Err(Problem::NoName));
        // Trimmed, because it comes from a text box.
        assert_eq!(
            Scratchpad::new("  sketch  ").map(|s| s.name),
            Ok("sketch".into())
        );
    }

    /// The whole generated manifest, asserted as text rather than as a value: the field
    /// order rule this codebase keeps hitting is a property of the *serializer*, and a
    /// round trip through a struct would not see it. `[workspace]` being emitted at all
    /// is here for the same reason — an empty table is the one thing a serializer might
    /// reasonably drop.
    #[test]
    fn a_package_is_a_manifest_and_a_main() {
        let mut scratchpad = scratchpad();
        scratchpad.dependencies = vec![
            dependency("rand", "0.8"),
            // Out of order and untrimmed on purpose: the manifest sorts and trims, the
            // list does not.
            dependency(" anyhow ", " 1.0.86 "),
        ];

        assert_eq!(
            scratchpad.manifest().expect("a manifest"),
            "\
[package]
name = \"sketch\"
version = \"0.1.0\"
edition = \"2021\"

[dependencies]
anyhow = \"1.0.86\"
rand = \"0.8\"

[workspace]
"
        );
    }

    /// The empty case is the one that actually ships, so it is asserted whole too: no
    /// `[dependencies]` header at all rather than an empty one.
    #[test]
    fn a_scratchpad_with_no_crates_has_no_dependencies_table() {
        let manifest = scratchpad().manifest().expect("a manifest");
        assert!(!manifest.contains("[dependencies]"), "{manifest}");

        let package = manifest.find("[package]").expect("the package table");
        let workspace = manifest.find("[workspace]").expect("the workspace table");
        assert!(package < workspace, "{manifest}");
    }

    #[test]
    fn a_row_that_is_not_a_crate_name_says_which_row() {
        let mut scratchpad = scratchpad();
        scratchpad.dependencies = vec![
            dependency("serde", "1"),
            dependency("", "1"),
            dependency("1password", "1"),
            dependency("hello world", "1"),
            dependency("a".repeat(MAX_NAME + 1), "1"),
        ];

        assert_eq!(
            scratchpad.problems(),
            vec![
                (1, Problem::NoName),
                (2, Problem::NameStart),
                (3, Problem::NameCharacter(' ')),
                (4, Problem::NameTooLong),
            ]
        );
    }

    #[test]
    fn a_version_that_is_not_a_version_says_so() {
        for good in [
            "1",
            "1.2",
            "1.2.3",
            "^1.2.3",
            "~1.2",
            "=1.2.3",
            ">=1.2, <2.0",
            "1.0.0-rc.1",
            "1.0.0-alpha+build.5",
            " 1.0 ",
        ] {
            assert_eq!(dependency("serde", good).check(), Ok(()), "{good}");
        }

        for (bad, problem) in [
            ("", Problem::NoVersion),
            ("   ", Problem::NoVersion),
            // The whole point of the requirement, and its own answer.
            ("*", Problem::Wildcard),
            ("1.*", Problem::Wildcard),
            (">=1, <2.*", Problem::Wildcard),
            ("latest", Problem::NotAVersion),
            ("v1.2", Problem::NotAVersion),
            ("1.2.3.4", Problem::NotAVersion),
            ("1..2", Problem::NotAVersion),
            ("1.2-", Problem::NotAVersion),
            ("1.2-rc/1", Problem::NotAVersion),
            (">=1,", Problem::NotAVersion),
        ] {
            assert_eq!(dependency("serde", bad).check(), Err(problem), "{bad:?}");
        }
    }

    /// A table cannot hold a key twice, so the second row would silently win. That is
    /// exactly the "silently different build" this module exists to refuse.
    #[test]
    fn the_same_crate_twice_is_a_row_that_says_so() {
        let mut scratchpad = scratchpad();
        scratchpad.dependencies = vec![
            dependency("serde", "1"),
            dependency(" serde ", "2"),
            // A second empty row is empty, not a duplicate: it has nothing to duplicate.
            dependency("", ""),
            dependency("", ""),
        ];

        assert_eq!(
            scratchpad.problems(),
            vec![
                (1, Problem::Repeated),
                (2, Problem::NoName),
                (3, Problem::NoName),
            ]
        );
    }

    #[test]
    fn a_scratchpad_with_a_bad_row_will_not_write() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.dependencies = vec![dependency("rand", "")];

        let failure = scratchpad.write_to(&directory).expect_err("a refusal");
        assert_eq!(
            failure,
            Failure::Dependencies(vec![(0, Problem::NoVersion)])
        );
        // And nothing was written on the way to refusing.
        assert!(!directory.exists());

        // A build refuses in the same terms rather than in cargo's.
        assert_eq!(scratchpad.build_in(&directory), Build::Unavailable(failure));
        assert!(!directory.exists());
    }

    /// The package is the storage, so this is the whole of the persistence test: what
    /// was written comes back, dependencies and all, with no second file involved.
    #[test]
    fn writes_and_reads_back() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.source = "fn main() { /* edited */ }\n".to_owned();
        scratchpad.dependencies = vec![dependency("anyhow", "1.0.86")];

        scratchpad.write_to(&directory).expect("writing");
        assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

        // The temporaries were renamed, not left behind.
        assert!(!directory.join("Cargo.toml.tmp").exists());
        assert!(!directory.join("src").join("main.rs.tmp").exists());

        // A directory with nothing in it is not a scratchpad, and neither is one with a
        // manifest and no source.
        assert_eq!(Scratchpad::load_from(&directory.join("src")), None);
        fs::remove_file(directory.join("src").join("main.rs")).expect("removing the source");
        assert_eq!(Scratchpad::load_from(&directory), None);

        let _ = fs::remove_dir_all(&directory);
    }

    /// What the app opens with, and the reason [`Scratchpad::default`] may hand out a
    /// name without a `Result`: it is a name this module would accept if it were typed,
    /// and it is a package it would agree to write.
    #[test]
    fn the_default_scratchpad_is_one_this_module_would_write() {
        let scratchpad = Scratchpad::default();

        assert_eq!(scratchpad.name(), DEFAULT_NAME);
        assert_eq!(Scratchpad::new(DEFAULT_NAME), Ok(scratchpad.clone()));
        assert!(scratchpad.problems().is_empty());
        assert!(scratchpad.manifest().is_ok());
    }

    /// Reopening: what is on disk wins over what the caller was holding, except for the
    /// name -- which is the directory the next write goes back to, and so cannot be
    /// something a hand-edited manifest gets to choose.
    #[test]
    fn a_scratchpad_opens_as_its_directory_has_it() {
        let directory = directory(line!());

        // Nothing there yet: what the caller was holding, unchanged, so a first run opens
        // on the default source rather than on nothing.
        let fresh = Scratchpad::default().opened_in(&directory);
        assert_eq!(fresh, Scratchpad::default());

        let mut written = scratchpad();
        written.source = "fn main() { /* saved */ }\n".to_owned();
        written.dependencies = vec![dependency("anyhow", "1.0.86")];
        written.write_to(&directory).expect("writing");

        let opened = Scratchpad::default().opened_in(&directory);
        assert_eq!(opened.source, written.source);
        assert_eq!(opened.dependencies, written.dependencies);
        // The manifest says `sketch` and the caller asked for `scratch`. The caller wins:
        // the name is where the next write lands.
        assert_eq!(opened.name(), DEFAULT_NAME);

        let _ = fs::remove_dir_all(&directory);
    }

    /// Which box a row's problem belongs against. The editor marks one of the two, so
    /// this is the model's answer rather than the view guessing from the wording.
    #[test]
    fn a_problem_is_about_one_half_of_its_row() {
        for problem in [
            Problem::NoName,
            Problem::NameStart,
            Problem::NameCharacter('/'),
            Problem::NameTooLong,
            // A repeat is a name collision: `[dependencies]` is keyed by the name, and
            // the version of the second row is what would silently go missing.
            Problem::Repeated,
        ] {
            assert_eq!(problem.half(), Half::Name, "{problem:?}");
        }

        for problem in [Problem::NoVersion, Problem::Wildcard, Problem::NotAVersion] {
            assert_eq!(problem.half(), Half::Version, "{problem:?}");
        }
    }

    /// Writing twice is what the editor does on every build, so the second write has to
    /// be the new source and not a merge of the two.
    #[test]
    fn writing_again_replaces_what_was_there() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.write_to(&directory).expect("writing");
        scratchpad.source = "fn main() {}\n".to_owned();
        scratchpad.dependencies = vec![dependency("rand", "0.8")];
        scratchpad.write_to(&directory).expect("writing again");

        assert_eq!(Scratchpad::load_from(&directory), Some(scratchpad));

        let _ = fs::remove_dir_all(&directory);
    }

    /// What a failed build reports, over a canned cargo stream — which is why `outcome`
    /// is a function of its own. Nothing here shells out.
    #[test]
    fn a_failed_build_reports_the_compilers_diagnostics() {
        let stdout = concat!(
            r#"{"reason":"compiler-artifact","executable":null}"#,
            "\n",
            r#"{"reason":"compiler-message","package_id":"sketch","message":{"#,
            r#""level":"error","message":"cannot find value `x` in this scope","#,
            r#""rendered":"error[E0425]: cannot find value `x`\n --> src/main.rs:2:5\n","#,
            r#""spans":[{"file_name":"src/main.rs","line_start":2,"column_start":5,"is_primary":true}]}}"#,
            "\n",
            r#"{"reason":"build-finished","success":false}"#,
            "\n",
        );
        let stderr = "   Compiling sketch v0.1.0\nerror: could not compile `sketch`\n";

        let Build::Rejected {
            diagnostics,
            message,
        } = outcome(stdout, stderr, false)
        else {
            panic!("a rejection");
        };

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level, Level::Error);
        assert_eq!(
            diagnostics[0].message,
            "cannot find value `x` in this scope"
        );
        assert!(diagnostics[0].rendered.contains("E0425"));
        assert_eq!(
            diagnostics[0].span,
            Some(Span {
                file: "src/main.rs".into(),
                line: 2,
                column: 5,
            })
        );
        // cargo's own stderr is kept whole: some failures are said there and nowhere
        // else.
        assert!(message.contains("could not compile"));
    }

    /// The failure with no diagnostics behind it at all — a dependency row that names a
    /// crate nothing has heard of. cargo says it on stderr and emits no compiler
    /// message, so a build result that only carried diagnostics would report nothing.
    #[test]
    fn a_dependency_that_does_not_resolve_is_cargos_own_words() {
        let stderr = "error: no matching package named `not-a-real-crate` found\n\
                      location searched: crates.io index\n";

        assert_eq!(
            outcome("", stderr, false),
            Build::Rejected {
                diagnostics: Vec::new(),
                message: stderr.trim().to_owned(),
            }
        );
    }

    #[test]
    fn a_successful_build_reports_the_artifact_and_its_warnings() {
        let stdout = concat!(
            r#"{"reason":"compiler-artifact","executable":null}"#,
            "\n",
            r#"{"reason":"build-script-executed","package_id":"libc"}"#,
            "\n",
            r#"{"reason":"compiler-message","message":{"level":"warning","#,
            r#""message":"unused variable: `y`","rendered":"warning: unused variable","#,
            r#""spans":[]}}"#,
            "\n",
            r#"{"reason":"compiler-artifact","executable":"/tmp/sketch/target/debug/sketch"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
            "\n",
            "warning: some future cargo says something new here\n",
        );

        let Build::Built {
            executable,
            diagnostics,
        } = outcome(stdout, "", true)
        else {
            panic!("a build");
        };

        assert_eq!(executable, PathBuf::from("/tmp/sketch/target/debug/sketch"));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level, Level::Warning);
        assert_eq!(diagnostics[0].span, None);
    }

    /// A cargo that succeeded and named nothing is a third answer, not an `unwrap`.
    #[test]
    fn a_build_that_names_no_artifact_says_so() {
        assert_eq!(
            outcome(r#"{"reason":"build-finished","success":true}"#, "", true),
            Build::Unavailable(Failure::NoArtifact)
        );
    }

    /// The one test that shells out. It is hermetic and needs no network: a scratchpad
    /// with no dependencies never touches the registry, so this is one rustc invocation
    /// in a temporary directory. `$CARGO` is set for anything cargo launches, this test
    /// included, so the cargo running the suite is the cargo that runs here.
    #[test]
    fn an_empty_scratchpad_really_builds() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.source = "fn main() {}\n".to_owned();

        let build = scratchpad.build_in(&directory);
        let Build::Built { executable, .. } = &build else {
            panic!("a build, got {build:?}");
        };
        // The path cargo named, and not one derived from the crate name: this is the
        // whole argument for `--message-format=json` in one assertion.
        assert!(executable.is_file(), "{}", executable.display());

        let _ = fs::remove_dir_all(&directory);
    }

    /// The line cap, over a reader that never says anything: a program writing megabytes
    /// with no newline in it must still be *delivered*, in pieces, rather than kept in one
    /// growing string nobody ever sees.
    #[test]
    fn a_line_with_no_end_to_it_is_cut_rather_than_kept() {
        let written = "x".repeat(MAX_LINE as usize * 2 + 7);
        let mut lines = Vec::new();
        stream_lines(io::Cursor::new(written), Stream::Out, |line| {
            lines.push(line)
        });

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text.len(), MAX_LINE as usize);
        assert_eq!(lines[1].text.len(), MAX_LINE as usize);
        assert_eq!(lines[2].text.len(), 7);
        assert!(lines.iter().all(|line| line.stream == Stream::Out));
    }

    /// And the ordinary case, including the two things a naive `read_line` gets wrong: a
    /// Windows line ending left in the text, and a last line with no terminator at all
    /// being dropped instead of delivered.
    #[test]
    fn lines_arrive_without_their_terminators() {
        let mut lines = Vec::new();
        stream_lines(
            io::Cursor::new("first\r\nsecond\n\nlast"),
            Stream::Err,
            |line| lines.push(line),
        );

        let text: Vec<&str> = lines.iter().map(|line| &*line.text).collect();
        assert_eq!(text, ["first", "second", "", "last"]);
        assert!(lines.iter().all(|line| line.stream == Stream::Err));
    }

    /// The other bound. A program printing in a tight loop is not an edge case in a
    /// scratchpad, so what has to be true is that the *oldest* goes and that the view can
    /// say how much of the story it is missing.
    #[test]
    fn output_keeps_the_newest_and_counts_what_it_dropped() {
        let mut output = RunOutput::default();
        assert_eq!(output.len(), 0);

        for line in 0..MAX_OUTPUT_LINES + 12 {
            output.push(OutputLine {
                stream: Stream::Out,
                text: Arc::from(line.to_string().as_str()),
            });
        }

        assert_eq!(output.len(), MAX_OUTPUT_LINES);
        assert_eq!(output.dropped(), 12);
        // The oldest kept is the twelfth written, and the newest is the last.
        assert_eq!(&*output.line(0).expect("a line").text, "12");
        assert_eq!(
            &*output.line(MAX_OUTPUT_LINES - 1).expect("a line").text,
            (MAX_OUTPUT_LINES + 11).to_string()
        );
        assert_eq!(output.line(MAX_OUTPUT_LINES), None);
    }

    /// Build a scratchpad whose source is `source` and hand back what to run. Hermetic
    /// and needs no network for `an_empty_scratchpad_really_builds`'s reason: no
    /// dependencies means no registry, so it is one rustc invocation in a temporary
    /// directory.
    fn program(directory: &Path, source: &str) -> PathBuf {
        let mut scratchpad = scratchpad();
        scratchpad.source = source.to_owned();

        let build = scratchpad.build_in(directory);
        let Build::Built { executable, .. } = &build else {
            panic!("a build, got {build:?}");
        };
        executable.clone()
    }

    /// Whether [`stop_all`] would still reach this run. Not called here -- other tests
    /// have programs of their own running in parallel threads of this same binary, and
    /// stopping *all* of them is exactly what it does.
    fn listed(running: &Running) -> bool {
        RUNNING
            .lock()
            .expect("the list")
            .iter()
            .any(|other| Arc::ptr_eq(&other.0, &running.0))
    }

    /// Collect a run's events until it ends, or give up saying so.
    fn until_ended(events: &std::sync::mpsc::Receiver<RunEvent>) -> Vec<RunEvent> {
        let mut collected = Vec::new();
        loop {
            let event = events
                .recv_timeout(Duration::from_secs(30))
                .expect("the run never ended");
            let ended = matches!(event, RunEvent::Ended(_));
            collected.push(event);
            if ended {
                return collected;
            }
        }
    }

    /// A program that prints and exits: both streams arrive, the end comes last, and the
    /// exit status is the program's own.
    ///
    /// Asserted without an order *between* the streams, which is not a promise this module
    /// makes and could not keep: two pipes read by two threads deliver in whatever order
    /// the kernel woke them. Within a stream the order is the program's own, which is what
    /// the other run tests rest on.
    #[test]
    fn a_program_that_prints_and_exits_is_streamed_and_reported() {
        let directory = directory(line!());
        let executable = program(
            &directory,
            "fn main() {\n\
             \x20   println!(\"to stdout\");\n\
             \x20   eprintln!(\"to stderr\");\n\
             \x20   std::process::exit(3);\n\
             }\n",
        );

        let (events, arrived) = std::sync::mpsc::channel();
        let running = run_in(&executable, &directory, move |event| {
            let _ = events.send(event);
        })
        .expect("it started");

        let collected = until_ended(&arrived);
        let (ended, written) = collected.split_last().expect("something happened");
        // The program's own status, not a zero for having run at all, and said last.
        assert_eq!(ended, &RunEvent::Ended(Ended::Exited(Some(3))));

        let mut written: Vec<(Stream, String)> = written
            .iter()
            .map(|event| match event {
                RunEvent::Wrote(line) => (line.stream, line.text.to_string()),
                RunEvent::Ended(_) => panic!("it ended twice"),
            })
            .collect();
        written.sort_by(|left, right| left.1.cmp(&right.1));
        assert_eq!(
            written,
            vec![
                (Stream::Err, "to stderr".to_owned()),
                (Stream::Out, "to stdout".to_owned()),
            ]
        );
        assert!(running.finished());

        let _ = fs::remove_dir_all(&directory);
    }

    /// The hazard this whole sub-step is about: a program that does not exit.
    ///
    /// Two things have to be true and only a real process can say either. What it printed
    /// **before** it stopped exiting is on screen — which is the difference between
    /// streaming and collecting an output at exit, since this one has no exit — and asking
    /// it to stop really ends it. `Ended::Stopped` arriving is itself the proof of the
    /// second: it is emitted only after the process has been *reaped*, so a run that
    /// reports it is a run the system no longer has.
    #[test]
    fn a_program_that_never_exits_still_says_something_and_can_be_killed() {
        let directory = directory(line!());
        let executable = program(
            &directory,
            "fn main() {\n\
             \x20   println!(\"before the loop\");\n\
             \x20   loop { std::thread::sleep(std::time::Duration::from_millis(50)); }\n\
             }\n",
        );

        let (events, arrived) = std::sync::mpsc::channel();
        let running = run_in(&executable, &directory, move |event| {
            let _ = events.send(event);
        })
        .expect("it started");

        // Said while it is still going, which is the whole point.
        assert_eq!(
            arrived.recv_timeout(Duration::from_secs(30)),
            Ok(RunEvent::Wrote(OutputLine {
                stream: Stream::Out,
                text: Arc::from("before the loop"),
            }))
        );
        assert!(!running.finished(), "it exited on its own");
        // On the list the window's close hook walks, which is the only way a program
        // still going when the app goes away can be reached at all.
        assert!(listed(&running), "nothing would have stopped it at exit");

        running.stop();
        assert_eq!(until_ended(&arrived), vec![RunEvent::Ended(Ended::Stopped)]);
        assert!(running.finished());
        // And off it again, so a long session of runs is not a list of dead handles.
        assert!(!listed(&running), "it stayed on the list after it ended");

        let _ = fs::remove_dir_all(&directory);
    }

    /// Nothing to run is an answer and not a panic — the executable a build named can be
    /// gone by the time the reader presses the button.
    #[test]
    fn a_program_that_is_not_there_says_so() {
        let directory = directory(line!());
        let failure = run_in(&directory.join("not-a-program"), &directory, |_| {})
            .err()
            .expect("a refusal");

        assert!(matches!(failure, Failure::NoProgram(_)), "{failure:?}");
    }

    /// And the same directory built again with source that does not compile: what a
    /// failed build reports, end to end, once.
    #[test]
    fn a_scratchpad_that_does_not_compile_reports_it() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.source = "fn main() { let _: u32 = \"not a number\"; }\n".to_owned();

        let build = scratchpad.build_in(&directory);
        let Build::Rejected { diagnostics, .. } = &build else {
            panic!("a rejection, got {build:?}");
        };
        let error = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.level == Level::Error)
            .expect("an error");
        assert_eq!(
            error.span.as_ref().map(|span| span.file.as_str()),
            Some("src/main.rs")
        );

        let _ = fs::remove_dir_all(&directory);
    }
}

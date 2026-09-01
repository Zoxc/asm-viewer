//! A scratchpad: one Rust source file, the cargo package generated around it, and the
//! `cargo build` that turns the two into something this app can open.
//!
//! Framework-free — no freya types appear here — like `project.rs` and `settings.rs`
//! beside it, so the whole of it is unit-tested without a window. What is *not* here is
//! any UI: the editable source view, the dependency rows on screen and the build button
//! are still to come, and they are what will read this.
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

// Nothing in the app reaches this module yet: what is built here is the model and the
// disk, and the editor, the rows on screen and the build action come with the scratchpad
// view. The allow is on the module rather than on each item because *every* item is in
// that state, and it comes off whole the moment `ui.rs` grows that view.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

impl Dependency {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Dependency {
        Dependency {
            name: name.into(),
            version: version.into(),
        }
    }

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
            Failure::Dependencies(problems) => {
                write!(formatter, "{} dependencies to fix", problems.len())
            }
            Failure::NoDirectory => write!(formatter, "nowhere to keep a scratchpad"),
            Failure::Write(error) => write!(formatter, "could not write the package: {error}"),
            Failure::NoCargo(error) => write!(formatter, "could not run cargo: {error}"),
            Failure::NoArtifact => write!(formatter, "cargo built nothing to open"),
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
            Dependency::new("rand", "0.8"),
            // Out of order and untrimmed on purpose: the manifest sorts and trims, the
            // list does not.
            Dependency::new(" anyhow ", " 1.0.86 "),
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
            Dependency::new("serde", "1"),
            Dependency::new("", "1"),
            Dependency::new("1password", "1"),
            Dependency::new("hello world", "1"),
            Dependency::new("a".repeat(MAX_NAME + 1), "1"),
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
            assert_eq!(Dependency::new("serde", good).check(), Ok(()), "{good}");
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
            assert_eq!(
                Dependency::new("serde", bad).check(),
                Err(problem),
                "{bad:?}"
            );
        }
    }

    /// A table cannot hold a key twice, so the second row would silently win. That is
    /// exactly the "silently different build" this module exists to refuse.
    #[test]
    fn the_same_crate_twice_is_a_row_that_says_so() {
        let mut scratchpad = scratchpad();
        scratchpad.dependencies = vec![
            Dependency::new("serde", "1"),
            Dependency::new(" serde ", "2"),
            // A second empty row is empty, not a duplicate: it has nothing to duplicate.
            Dependency::new("", ""),
            Dependency::new("", ""),
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
        scratchpad.dependencies = vec![Dependency::new("rand", "")];

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
        scratchpad.dependencies = vec![Dependency::new("anyhow", "1.0.86")];

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

    /// Writing twice is what the editor does on every build, so the second write has to
    /// be the new source and not a merge of the two.
    #[test]
    fn writing_again_replaces_what_was_there() {
        let directory = directory(line!());
        let mut scratchpad = scratchpad();
        scratchpad.write_to(&directory).expect("writing");
        scratchpad.source = "fn main() {}\n".to_owned();
        scratchpad.dependencies = vec![Dependency::new("rand", "0.8")];
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

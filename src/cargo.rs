//! Running cargo, and reading what it said.
//!
//! Everything here is about someone else's file: cargo's JSON message stream, and the
//! manifest of the package being built. [`run`] blocks and [`add_debug_lines`] writes, so
//! both belong on a worker thread.
//!
//! Two callers: a scratchpad's generated package (`src/scratchpad.rs`), which has one
//! binary, and the project's own workspace (`src/ui/building.rs`), which has as many
//! artifacts as it has targets. [`outcome`] is a pure function of what cargo printed, which
//! is what lets a build be a test over a canned stream.

use std::{
    borrow::Cow,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

use crate::project::write_atomically;

/// Which of cargo's two built-in profiles to build.
///
/// `Debug` is cargo's `dev`: the profile is named `dev` in a manifest and puts its output
/// in `target/debug`, which is why the name is asked for rather than derived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Debug,
    /// The default: a reader inspecting a binary is usually asking what the optimiser did.
    #[default]
    Release,
}

impl Profile {
    /// What a manifest calls this profile.
    pub fn name(&self) -> &'static str {
        match self {
            Profile::Debug => "dev",
            Profile::Release => "release",
        }
    }

    /// Whether cargo puts debug information in this profile when the manifest says
    /// nothing.
    fn debug_by_default(&self) -> bool {
        matches!(self, Profile::Debug)
    }
}

/// One file a build produced, as cargo named it.
///
/// **Never derived.** `target/<profile>/<name>` worked out from the target and the profile
/// is silently wrong under a `CARGO_TARGET_DIR`, a config above the directory, or an
/// executable suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub path: PathBuf,
    /// The target it came from, as the manifest names it.
    pub target: String,
    /// `bin`, `lib`, `test` — the first of cargo's kinds for that target.
    pub kind: String,
}

/// What a build came back with. Three answers, not two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    /// `diagnostics` is whatever cargo said on the way — warnings and notes.
    Built {
        artifacts: Vec<Artifact>,
        diagnostics: Vec<Diagnostic>,
    },
    /// cargo ran and refused. `message` is cargo's own stderr, which is the only place
    /// some failures are said at all: `no matching package named ... found` for a
    /// dependency that does not resolve, and a manifest error, both arrive with no
    /// compiler diagnostics behind them.
    Rejected {
        diagnostics: Vec<Diagnostic>,
        message: String,
    },
    /// cargo could not be started at all — not on the `PATH`, or not executable.
    NoCargo(String),
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

/// A place in a file the compiler named. `file` is as cargo gave it — relative to where
/// cargo ran for the package's own source, a registry path for a dependency's.
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

impl Span {
    /// Where in `text` this span points, counted in **UTF-16 code units** from the start of
    /// the text — which is how a cursor position is counted, and the only unit that makes
    /// the answer independent of who is counting lines.
    ///
    /// rustc counts a line and a column from one and counts a column in *characters*, so a
    /// tab is one and an accented letter is one. Lines are separated by `\n` and nothing
    /// else, which is rustc's own rule: it normalises `\r\n` before it numbers anything.
    ///
    /// Two clamps, and they are the same decision twice. `text` is the source **as it is
    /// now**, which is not necessarily the source the build was told about — the reader has
    /// usually typed since — so a column past the end of its line lands at the end of that
    /// line, and a line past the end of the text at the end of the text. Being taken to
    /// roughly the right place beats not being taken anywhere, and there is nothing here
    /// that can fail.
    pub fn offset_in(&self, text: &str) -> usize {
        let line = self.line.saturating_sub(1);
        let column = self.column.saturating_sub(1);
        let units =
            |text: &str, take: usize| text.chars().take(take).map(char::len_utf16).sum::<usize>();

        let mut offset = 0;
        for (index, row) in text.split_inclusive('\n').enumerate() {
            if index == line {
                // The line break is no part of the line: a column past the end of the text
                // on it stops before the break rather than landing on the line below.
                let row = row.trim_end_matches('\n').trim_end_matches('\r');
                return offset + units(row, column);
            }
            offset += units(row, usize::MAX);
        }

        offset
    }
}

/// Build the package in `directory`, blocking until cargo is done.
pub fn run(directory: &Path, profile: Profile) -> Run {
    let mut command =
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    command
        .current_dir(directory)
        // The artifact paths are *asked for* rather than guessed at; see [`Artifact`].
        .args(["build", "--message-format=json", "--color=never"])
        // A cargo that asks a question would otherwise wait for an answer nobody can give.
        .stdin(Stdio::null());
    if profile == Profile::Release {
        command.arg("--release");
    }

    let output = match command.output() {
        Ok(output) => output,
        Err(error) => return Run::NoCargo(error.to_string()),
    };

    // The reader typed this directory; the artifacts are matched against cargo's spelling
    // of it. See `as_cargo_names_it`.
    let directory = as_cargo_names_it(directory);

    outcome(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
        output.status.success(),
        &directory,
    )
}

/// Turn what cargo printed into an answer. Pure, so a build is a test over a canned stream.
///
/// Every line of stdout is one JSON message and cargo's progress goes to stderr, so a line
/// that does not parse is skipped rather than failing the run.
fn outcome(stdout: &str, stderr: &str, success: bool, directory: &Path) -> Run {
    let mut diagnostics = Vec::new();
    let mut artifacts = Vec::new();

    for line in stdout.lines() {
        match serde_json::from_str::<Report>(line) {
            Ok(Report::Message { message }) => diagnostics.push(message.into()),
            Ok(Report::Artifact(artifact)) => artifacts.extend(artifact.built(directory)),
            _ => {}
        }
    }

    if !success {
        return Run::Rejected {
            diagnostics,
            message: stderr.trim().to_owned(),
        };
    }

    Run::Built {
        artifacts,
        diagnostics,
    }
}

/// The directory as cargo will name it, which is what its artifacts are matched against.
///
/// cargo is handed a working directory rather than a path, and builds every
/// `manifest_path` by walking up from its own `env::current_dir()`. Meeting that is the
/// whole rule, and the two platforms answer differently.
///
/// On Unix it is `getcwd(2)`, which the kernel walks out of the directory tree: symlinks
/// resolved and `..` gone. Only `fs::canonicalize` reaches the same path, so a `..` or a
/// symlink in what the reader typed would match nothing without it.
///
/// On Windows it is `GetCurrentDirectoryW`, which is logical: the prefix stays plain, `..`
/// is collapsed by spelling, and a junction is left as it was written. `path::absolute` is
/// `GetFullPathNameW` and answers in exactly that form. `fs::canonicalize` there answers
/// verbatim (`\\?\C:\work\app`) and resolves the junction cargo kept, so it agrees with
/// cargo on neither.
fn as_cargo_names_it(directory: &Path) -> PathBuf {
    #[cfg(windows)]
    let full = std::path::absolute(directory);
    #[cfg(not(windows))]
    let full = fs::canonicalize(directory);

    full.unwrap_or_else(|_| directory.to_owned())
}

/// Whether `path` is inside `directory`, each reduced to a plain path first.
///
/// By path component and not by text, so `/work/apple` is not inside `/work/app`.
fn inside(path: &Path, directory: &Path) -> bool {
    simplified(path).starts_with(simplified(directory))
}

/// A Windows verbatim path as the plain path it names, and anything else unchanged.
///
/// [`as_cargo_names_it`] normally leaves the two sides agreeing. But a reader can
/// type `\\?\C:\work\app` into the project box, and `path::absolute` hands a verbatim path
/// back as given, so that spelling reaches this side whole. Whether it reaches cargo's is
/// Windows' business: the working directory a child is started in goes through the loader,
/// which may hand `GetCurrentDirectoryW` back either form. `Path` compares the two as
/// different components -- `VerbatimDisk` is not `Disk` -- and a directory that holds
/// nothing cargo named drops every artifact of the build. Reducing both sides costs nothing
/// where they already match and saves the build where they do not.
///
/// The strip is textual, so it is the same on every platform and can be tested from any of
/// them; no Unix path begins `\\?\`. Only the two forms with a plain spelling are reduced:
/// `\\?\pipe\...` and `\\?\Volume{...}` name what no drive letter can, and are left alone.
fn simplified(path: &Path) -> Cow<'_, Path> {
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };

    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(PathBuf::from(format!(r"\\{share}")));
    }

    match text.strip_prefix(r"\\?\") {
        Some(rest) if drive(rest) => Cow::Borrowed(Path::new(rest)),
        _ => Cow::Borrowed(path),
    }
}

/// Whether `text` starts with a drive letter and a colon, ending there or at a separator.
fn drive(text: &str) -> bool {
    let text = text.as_bytes();
    text.first().is_some_and(u8::is_ascii_alphabetic)
        && text.get(1) == Some(&b':')
        && matches!(text.get(2), None | Some(b'\\'))
}

/// The manifest in `directory`, or `None` when there is none to build.
pub fn manifest(directory: &Path) -> Option<PathBuf> {
    let path = directory.join(MANIFEST);
    path.is_file().then_some(path)
}

/// Whether a binary built with `profile` would carry the line information the source side
/// is drawn from.
///
/// A manifest that says nothing gets cargo's own default, which is *no* debug information
/// under `release` — the reason the view offers to add it.
pub fn debug_lines(directory: &Path, profile: Profile) -> bool {
    let Some(value) = read_manifest(directory)
        .as_ref()
        .and_then(|manifest| manifest.get("profile"))
        .and_then(|profiles| profiles.get(profile.name()))
        .and_then(|profile| profile.get("debug"))
        .cloned()
    else {
        return profile.debug_by_default();
    };

    // The three spellings cargo accepts, and the values of each that mean none. Anything
    // else it accepts -- `1`, `2`, `"limited"`, `"full"` -- carries lines.
    match value {
        toml::Value::Boolean(on) => on,
        toml::Value::Integer(level) => level > 0,
        toml::Value::String(name) => !matches!(name.as_str(), "none" | "false" | "0"),
        _ => false,
    }
}

/// Ask `profile` for line tables, in the manifest in `directory`.
///
/// `line-tables-only` and not `true`: the source side wants the line table and nothing
/// else, and it is the cheapest debug information to build.
pub fn add_debug_lines(directory: &Path, profile: Profile) -> Result<(), String> {
    let path = directory.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| error.to_string())?;

    // Made if it is not there, and made *implicit* in that case so the file gains a
    // `[profile.release]` header and no empty `[profile]` above it.
    let profiles = document
        .as_table_mut()
        .entry("profile")
        .or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| format!("`profile` in {} is not a table", path.display()))?;

    let one = profiles
        .entry(profile.name())
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| format!("`profile.{}` is not a table", profile.name()))?;
    one["debug"] = toml_edit::value("line-tables-only");

    write_atomically(&path, document.to_string().as_bytes()).map_err(|error| error.to_string())
}

const MANIFEST: &str = "Cargo.toml";

/// The manifest as a value, or `None` when there is none or it does not parse. Neither is
/// an error here: what cargo makes of its own file is cargo's answer, said when a build is
/// asked for.
fn read_manifest(directory: &Path) -> Option<toml::Table> {
    // A `Table` and not a `Value`: `Value`'s own `FromStr` parses one TOML *value*, where
    // a manifest is a whole document.
    let text = fs::read_to_string(directory.join(MANIFEST)).ok()?;
    toml::from_str::<toml::Table>(&text).ok()
}

/// The messages cargo emits, as much of each as this module reads. `#[serde(other)]` is
/// what makes an unknown `reason` a message that is skipped rather than a parse failure
/// that would throw the artifacts away with it.
#[derive(Deserialize)]
#[serde(tag = "reason")]
enum Report {
    #[serde(rename = "compiler-message")]
    Message { message: CompilerMessage },
    #[serde(rename = "compiler-artifact")]
    Artifact(ArtifactMessage),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ArtifactMessage {
    manifest_path: PathBuf,
    target: TargetMessage,
    #[serde(default)]
    executable: Option<PathBuf>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct TargetMessage {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
}

impl ArtifactMessage {
    /// What of this message is worth listing, which is usually nothing: cargo reports an
    /// artifact for **every** crate in the graph, dependencies and build scripts included
    /// -- 449 of them for this app's own workspace, of which two are its own.
    ///
    /// A target's own file is its `executable` where it has one, and its `filenames`
    /// otherwise, which is what puts a library's `.rlib` in the list -- an archive this app
    /// opens like any other. `.rmeta` is dropped: it is what cargo hands the next compiler
    /// so it can start early, it holds no code, and a row for it could only ever fail to
    /// parse. The one place here a file is judged by its name.
    fn built(self, directory: &Path) -> Vec<Artifact> {
        if !inside(&self.manifest_path, directory) {
            return Vec::new();
        }

        let kind = self.target.kind.first().cloned().unwrap_or_default();
        let paths = match self.executable {
            Some(executable) => vec![executable],
            None => self
                .filenames
                .into_iter()
                .filter(|path| path.extension().is_none_or(|end| end != "rmeta"))
                .collect(),
        };

        paths
            .into_iter()
            .map(|path| Artifact {
                path,
                target: self.target.name.clone(),
                kind: kind.clone(),
            })
            .collect()
    }
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

#[cfg(test)]
mod tests;

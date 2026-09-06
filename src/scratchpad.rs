//! A scratchpad: one Rust source file, the cargo package generated around it, and the
//! `cargo build` that turns the two into something this app can open.
//!
//! The generated package is the storage — [`Scratchpad::load_from`] is the exact inverse
//! of [`Scratchpad::write_to`], so there is no second format to disagree with what cargo
//! is handed. [`Scratchpad::opened_in`], [`Scratchpad::write_to`] and
//! [`Scratchpad::build_in`] block and [`run_in`] forks, so all four belong on a worker
//! thread.

use std::{
    collections::{BTreeMap, HashSet},
    fmt, fs,
    io::{self, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::cargo::{self, Diagnostic};
use crate::process::{self, RunEvent, Stream};
use crate::project::{base, write_atomically, write_toml};

const SCRATCHPADS_DIR: &str = "scratchpads";

const MANIFEST_NAME: &str = "Cargo.toml";
const SOURCE_DIR: &str = "src";
const SOURCE_NAME: &str = "main.rs";

/// The file a scratchpad's source is, as cargo and rustc spell it: the two names above, in
/// the order the package puts them. What a diagnostic's span says, and what the end of a
/// program's own name for it is.
pub const SOURCE_FILE: &str = "src/main.rs";

/// Whether the file a **diagnostic** names is the pad's own source.
///
/// **Either separator, on every platform.** cargo hands rustc the path it built with
/// `Path::join` and rustc echoes it back as it was given, so on Windows the span says
/// `src\main.rs` and a comparison against the string above makes every diagnostic in the
/// pad's own file look like a dependency's. Nothing on Unix names a built file with a
/// backslash in it, so accepting both costs nothing and leaves the rule one function a
/// test can put Windows' spelling to wherever it runs.
pub fn is_source_file(file: &str) -> bool {
    file.split(['/', '\\']).eq(SOURCE_FILE.split('/'))
}

/// Whether the file a **program's debug info** names is the pad's own source: a path
/// *ending* in [`SOURCE_FILE`].
///
/// A suffix where a diagnostic's is the whole string, and that is the difference between
/// the two spellings. rustc records the file as it was handed it and the name a reader of
/// the debug info gets back is that joined onto the unit's `DW_AT_comp_dir` -- the
/// directory rustc ran in, which is where the pad's own directory *resolved* to and not
/// how this app spells it. So neither the bare name nor a path built from
/// [`Scratchpad::directory`] matches, and the tail is what both spellings share.
pub fn ends_in_source_file(file: &str) -> bool {
    let mut theirs = file.rsplit(['/', '\\']);
    SOURCE_FILE
        .rsplit('/')
        .all(|part| theirs.next() == Some(part))
}

/// Which of the files a program has code from is the pad's own, out of what the program
/// says its files are. [`None`] where it names none: a build with no debug info, or one
/// whose paths were remapped.
///
/// Where a program names two the first wins, and that is arbitrary and said to be
/// arbitrary (`compiled::pick`'s tie-break in a second place): a generated package with
/// one binary in it has one `src/main.rs`, and a dependency of the pad's that happens to
/// have one of its own is a program the reader is not asking about.
pub fn own_source<'a>(files: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    files.into_iter().find(|file| ends_in_source_file(file))
}

/// What a build was *of*: the parts of a scratchpad that decide the program.
///
/// The **name is not one of them**. It lives in `[package.metadata]`, which cargo compiles
/// nothing from, so a rename must not make a program out of date -- the same separation of
/// the id from the name that makes a rename a value changing and not a directory moving.
/// The dependency rows *are*: a row changed is a different program, whatever the source
/// says.
///
/// A value and not a counter, so a reader who types a character and takes it back is
/// building the same program and is told so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compiled {
    source: String,
    dependencies: Vec<Dependency>,
}

impl Compiled {
    /// This, as the sixteen hex digits a build writes down beside its artifact.
    ///
    /// The bytes hashed are the source and then each row's two halves, every one of them
    /// ended by a byte that cannot appear in what it follows, so no two different lists
    /// hash the same by running together.
    pub fn digest(&self) -> String {
        let mut bytes = self.source.clone().into_bytes();
        bytes.push(0);
        for dependency in &self.dependencies {
            bytes.extend_from_slice(dependency.name.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(dependency.version.as_bytes());
            bytes.push(0);
        }
        analysis::FileDigest::of(&bytes).to_string()
    }
}

/// Pinned rather than left to cargo's default, so a scratchpad written today still
/// compiles the way it did when a later cargo changes what a new package gets.
const EDITION: &str = "2021";
const PACKAGE_VERSION: &str = "0.1.0";

/// crates.io's own limit.
const MAX_NAME: usize = 64;

/// What a new pad's id starts with, before the `-N`. The same stem as [`DEFAULT_ID`] and
/// deliberately without its number: the pad a first run opens is `pad` and the ones New
/// makes are `pad-1`, `pad-2`, so New can never hand out the id of the pad the app is
/// already holding — which it could if that one were `pad-1`, since a pad nobody has typed
/// in has claimed no directory for `create_dir` to fail on. It is a crate name because
/// [`check_name`] wants a letter first, which is why this is not `1-pad`.
const NEW_STEM: &str = "pad";

/// The file beside the pads holding the order they were last shown in.
const RECENTS_FILE: &str = "recents.toml";

/// How many names the order **file** keeps. What is lost past this is an *order*, never a
/// pad: [`pads_in`] lists a pad the order has forgotten just as it lists one made out of
/// band. It is the file's bound and not [`PadOrder`]'s, since the list a reader picks a pad
/// from is that listing, and a pad the panel does not draw cannot be opened at all.
pub const MAX_PAD_RECENTS: usize = 50;

/// The id of the pad a first run opens, and so the directory it lives in. Checked against
/// [`check_name`] by a test, which is what lets [`Scratchpad::default`] hand it out without
/// an `Option`.
pub const DEFAULT_ID: &str = "pad";

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

/// A scratchpad's identity: the name of the directory its package lives in, which is also
/// the crate's own name.
///
/// **Generated, never typed, and never shown.** It is what the app files a pad under — the
/// directory, the order, the table the UI keeps — and the reader deals in
/// [`Scratchpad::name`] instead, which is free text they can change without anything
/// moving. That separation is the whole reason this is not the name: a name that is also a
/// path is a rename that is also a directory move, has to be unique, and has to be spelt in
/// the alphabet a crate name allows.
///
/// A newtype for [`ProjectId`](crate::project::ProjectId)'s reason all the same: it is
/// interpolated into a path and it is read back out of files a user can edit — the order
/// beside the pads, and every pad's own `Cargo.toml`. [`PadId::new`] is the only way to make
/// one and `Deserialize` goes through it, so an id out of either file cannot be `..`, an
/// absolute path or a name with a separator in it. The rules are [`check_name`]'s, since the
/// id is what `[package] name` says, and they are strictly stronger than what a safe path
/// component needs. No `Display`, deliberately: an id has no business being written into
/// anything a reader looks at.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct PadId(String);

impl<'de> Deserialize<'de> for PadId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PadId, D::Error> {
        let text = String::deserialize(deserializer)?;
        PadId::new(text).ok_or_else(|| serde::de::Error::custom("not a scratchpad id"))
    }
}

impl PadId {
    /// The id this text spells, or `None` when it is not one. An `Option` and not a
    /// [`Problem`], where a dependency row's name gets one: nobody types an id, so there is
    /// nobody to tell what is wrong with it.
    pub fn new(text: impl Into<String>) -> Option<PadId> {
        let text = text.into();
        check_name(&text).ok()?;
        Some(PadId(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One scratchpad: what the app files it under, what the reader calls it, the source it
/// holds, and the crates it asks for.
///
/// [`PadId`] is the identity and `name` is free text — so two pads may be called the same
/// thing, a rename moves nothing, and a name may be empty, hold spaces or be written in any
/// alphabet at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scratchpad {
    id: PadId,
    /// What the reader calls this pad. Raw text out of a box, so it is trimmed at the
    /// accessor rather than on the way in, exactly as a dependency row's two halves are.
    pub name: String,
    pub source: String,
    /// In the order the reader put them in — the manifest sorts them, this list does not,
    /// since reordering under an edit is the one thing a list of text boxes must not do.
    pub dependencies: Vec<Dependency>,
    /// What the last build made, so a pad opened in a later run shows its program without
    /// being built again. [`None`] for a pad nothing has built.
    ///
    /// It is in the package like everything else here, under `[package.metadata]`, so
    /// `load_from` stays the exact inverse of `write_to` and nothing describes a pad
    /// beside its own directory. The **path** and not a derivation of it: `target/debug/`
    /// under the package is silently wrong beneath a `CARGO_TARGET_DIR`, a config above
    /// the directory, or an executable suffix, so what is kept is what cargo named
    /// ([`cargo::Artifact`]).
    pub built: Option<Built>,
}

/// What a build left behind: where cargo put it, and what it was a build *of*.
///
/// The digest and not the source itself: the source is already in the package a line away,
/// and what this has to answer is only whether the two are still the same. Sixteen
/// lowercase hex digits, [`analysis::FileDigest`]'s own written form, compared as text --
/// text this app did not write is simply not equal, which reads as "changed", the rule the
/// session's own digests follow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Built {
    pub path: PathBuf,
    pub digest: String,
}

/// One row of the pad list: a scratchpad that can be shown, described by its own package
/// read at the moment the list is asked for.
///
/// A name is never copied into the order file beside the ids — it lives in exactly one
/// place, the package the reader edits, which is `recent_projects`' rule for a project's
/// name and holds here for the same reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadListing {
    pub id: PadId,
    pub name: String,
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
    /// The package could not be removed, or what is under the id is not a package this
    /// module would have written.
    Delete(String),
    /// `cargo` could not be started at all — not on the `PATH`, or not executable.
    NoCargo(String),
    /// cargo reported success and named no executable.
    NoArtifact,
    /// The built program could not be started — deleted since the build, or on a
    /// filesystem mounted `noexec`.
    NoProgram(String),
    /// There is a package under the pad's id and this module cannot read it back. Said
    /// rather than stepped over, because the answer to a package that will not load is to
    /// leave it alone: see [`Scratchpad::opened_in`].
    Unreadable,
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
        Scratchpad::new(DEFAULT_ID).expect("DEFAULT_ID is a crate name")
    }
}

impl Scratchpad {
    pub fn new(id: impl Into<String>) -> Option<Scratchpad> {
        Some(Scratchpad::of(PadId::new(id)?))
    }

    /// A pad under `id`, with nothing in it yet — **no name included**: a pad nobody has
    /// named has an empty one, and what stands in for it on screen is the UI's business,
    /// not something written into the package.
    pub fn of(id: PadId) -> Scratchpad {
        Scratchpad {
            id,
            name: String::new(),
            source: DEFAULT_SOURCE.to_owned(),
            dependencies: Vec::new(),
            built: None,
        }
    }

    /// What the app files this pad under. Never drawn.
    pub fn id(&self) -> &PadId {
        &self.id
    }

    /// What the reader calls it, trimmed — **empty when they have not said**, which is a
    /// real answer and not a missing one: what a pad with no name of its own is called on
    /// screen is the UI's to decide.
    pub fn name(&self) -> &str {
        self.name.trim()
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

    /// What building this scratchpad now would be a build *of*, to be compared against what
    /// a build already made ([`Compiled`]).
    pub fn compiled(&self) -> Compiled {
        Compiled {
            source: self.source.clone(),
            dependencies: self.dependencies.clone(),
        }
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
                name: self.id.clone(),
                version: PACKAGE_VERSION.to_owned(),
                edition: EDITION.to_owned(),
                metadata: Metadata {
                    scratchpad: PadMetadata {
                        name: self.name().to_owned(),
                        built: self.built.clone(),
                    },
                },
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
        Some(pad_in(&base()?, &self.id))
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
    /// hand-edited scratchpad opens with the bad row visible rather than not opening. A
    /// manifest with no name in its metadata reads back as a pad nobody has named, which is
    /// what one written by hand is.
    pub fn load_from(directory: &Path) -> Option<Scratchpad> {
        let manifest = fs::read_to_string(directory.join(MANIFEST_NAME)).ok()?;
        let manifest: Manifest = toml::from_str(&manifest).ok()?;
        let source = fs::read_to_string(directory.join(SOURCE_DIR).join(SOURCE_NAME)).ok()?;

        Some(Scratchpad {
            id: manifest.package.name,
            name: manifest.package.metadata.scratchpad.name,
            source,
            dependencies: manifest
                .dependencies
                .into_iter()
                .map(|(name, version)| Dependency { name, version })
                .collect(),
            built: manifest.package.metadata.scratchpad.built,
        })
    }

    /// This scratchpad as `directory` has it, or this one where there is nothing there.
    /// Blocking.
    ///
    /// The id kept is **this** scratchpad's and never the manifest's: `directory()` is
    /// derived from the id, so a hand-edited `Cargo.toml` naming another crate would
    /// otherwise send the next write somewhere the reader never opened. The *name* comes
    /// off the disk like everything else, being a value and not a place.
    ///
    /// **A package this module cannot read is not an empty directory**, and the two are
    /// told apart here. Falling back to what was handed in for either would answer with
    /// the caller's own scratchpad, and the next keystroke would write that over the
    /// reader's files -- which a manifest with one hand-written dependency table entry is
    /// enough to bring about. So a directory holding either file is [`Failure::Unreadable`]
    /// rather than a scratchpad.
    pub fn opened_in(self, directory: &Path) -> Result<Scratchpad, Failure> {
        match Scratchpad::load_from(directory) {
            Some(loaded) => Ok(Scratchpad {
                id: self.id,
                ..loaded
            }),
            None if holds_package(directory) => Err(Failure::Unreadable),
            None => Ok(self),
        }
    }

    /// Write the package into `directory` and build it. Blocking, and never from a UI
    /// thread.
    ///
    /// The package is written first, so what is built is what is on screen.
    pub fn build_in(&self, directory: &Path) -> Build {
        if let Err(failure) = self.write_to(directory) {
            return Build::Unavailable(failure);
        }

        // Always `dev`: a scratchpad is compiled to be read and run, not to be measured,
        // and the wait is the reader's.
        match cargo::run(directory, cargo::Profile::Debug) {
            cargo::Run::Built {
                artifacts,
                diagnostics,
            } => match artifacts
                .into_iter()
                .find(|artifact| artifact.kind == "bin")
            {
                // The generated package has exactly one binary, so the one executable
                // cargo named for it is this pad's.
                Some(artifact) => Build::Built {
                    executable: artifact.path,
                    diagnostics,
                },
                // cargo succeeded and named nothing: a third answer, not an `unwrap`.
                None => Build::Unavailable(Failure::NoArtifact),
            },
            cargo::Run::Rejected {
                diagnostics,
                message,
            } => Build::Rejected {
                diagnostics,
                message,
            },
            cargo::Run::NoCargo(error) => Build::Unavailable(Failure::NoCargo(error)),
        }
    }
}

/// Where the pads are kept: one directory each, under one directory of their own beside
/// the projects and the settings.
fn scratchpads_in(base: &Path) -> PathBuf {
    base.join(SCRATCHPADS_DIR)
}

fn pad_in(base: &Path, id: &PadId) -> PathBuf {
    scratchpads_in(base).join(id.as_str())
}

/// Whether `directory` holds either half of a package. What tells "nothing there" from
/// "something there this module cannot read"; see [`Scratchpad::opened_in`].
fn holds_package(directory: &Path) -> bool {
    directory.join(MANIFEST_NAME).exists() || directory.join(SOURCE_DIR).join(SOURCE_NAME).exists()
}

/// The order file sits **beside the pads** rather than at the top of the state directory,
/// so it is not a second `recents.toml` to tell apart from the projects' one. It is a file
/// where every sibling is a directory, so [`pads_in`] steps over it with no special case.
fn pad_recents_in(base: &Path) -> PathBuf {
    scratchpads_in(base).join(RECENTS_FILE)
}

/// The pads the reader has had open, most recently first.
///
/// `project.rs`'s `Recents` rules, for the same reasons: which pad to open is the first
/// entry and not a field of its own, and this is an *order* rather than an index of what
/// exists — the directories are that — which is why nothing here prunes an id whose
/// directory has gone. [`pads_in`] repairs it at the point of use, where the repair is free.
///
/// Ids and never names: a name is a value the reader edits, and a copy of one here would be
/// a second copy to keep in step with the package's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadOrder {
    #[serde(default)]
    scratchpads: Vec<PadId>,
}

impl PadOrder {
    pub fn ids(&self) -> &[PadId] {
        &self.scratchpads
    }

    pub fn first(&self) -> Option<&PadId> {
        self.scratchpads.first()
    }

    /// The order a listing states, whole and in its own order.
    ///
    /// Not a `touch` per row: this is the order being replaced by what the disk says, where
    /// a touch is what *using* a pad does to it. Nothing is dropped, either. The listing
    /// names every pad there is, the ones the order forgot included, and this is the list
    /// the panel draws — an id missing from it is a pad with no way back to it.
    pub fn of(listing: &[PadListing]) -> PadOrder {
        PadOrder {
            scratchpads: listing.iter().map(|listed| listed.id.clone()).collect(),
        }
    }

    /// Put `id` at the front, and say whether that changed anything — which is what keeps a
    /// startup that reopens the pad already at the front from writing a file.
    ///
    /// Nothing falls off the end here: the cap is [`PadOrder::capped`]'s, applied to what
    /// goes to disk, so a pad this order is holding for the panel is not dropped by
    /// someone else being shown.
    pub fn touch(&mut self, id: &PadId) -> bool {
        if self.first() == Some(id) {
            return false;
        }
        self.scratchpads.retain(|other| other != id);
        self.scratchpads.insert(0, id.clone());
        true
    }

    /// The front of the order, at most [`MAX_PAD_RECENTS`] of it: what is written out.
    ///
    /// The bound is the file's alone. What it drops is the tail of an order and never a
    /// pad, [`pads_in`] appending every pad the file does not name.
    fn capped(mut self) -> PadOrder {
        self.scratchpads.truncate(MAX_PAD_RECENTS);
        self
    }

    /// Drop `id`, for a pad that has just been deleted.
    ///
    /// The list the panel draws and not the file: an id whose directory has gone is one
    /// [`pads_in`] already steps over, so nothing goes back to disk to say so.
    pub fn forget(&mut self, id: &PadId) {
        self.scratchpads.retain(|other| other != id);
    }

    fn load_from(path: &Path) -> PadOrder {
        fs::read_to_string(path)
            .ok()
            .and_then(|data| toml::from_str(&data).ok())
            .unwrap_or_default()
    }
}

/// Put `name` at the front of the order on disk, if there is a directory for it.
///
/// The condition is what keeps the rule "nothing is written until there is something to
/// say": the pad a first run opens is held in memory until something is typed into it, and
/// recording it before then would leave a `recents.toml` behind on a machine where the
/// reader never touched the scratchpad at all.
pub fn remember(id: &PadId) {
    if let Some(base) = base() {
        remember_in(&base, id);
    }
}

/// The whole of the above except finding the state directory. A read-modify-write of the
/// whole file, and a failure is logged and swallowed: losing an order is not losing a pad.
fn remember_in(base: &Path, id: &PadId) {
    if !pad_in(base, id).is_dir() {
        return;
    }

    let path = pad_recents_in(base);
    let mut order = PadOrder::load_from(&path);
    if !order.touch(id) {
        return;
    }
    if let Err(error) = write_toml(&path, &order.capped()) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Every scratchpad there is, in the order they were last opened, then the ones the order
/// does not name in id order — or an empty list on a system with nowhere to keep them.
///
/// Each row carries the name out of that pad's **own package**, read at the moment the list
/// is asked for, which is what lets the panel draw a pad it has never opened. It is also
/// why the order file holds ids alone: a name lives in one place, the one the reader edits.
pub fn pads() -> Vec<PadListing> {
    base().map(|base| pads_in(&base)).unwrap_or_default()
}

/// The whole of the above except finding the state directory.
///
/// A directory [`Scratchpad::load_from`] answers for is a pad and anything else is not, so
/// an id in the order whose directory has gone — or was never a package — is dropped here
/// rather than repaired on load. The strays are appended because this is the list a reader
/// picks from and every pad has to be reachable: one that fell off the end of the order, or
/// one made outside the app, is still a scratchpad. That is the difference from
/// `recent_projects`, which lists the projects a reader has *opened*.
fn pads_in(base: &Path) -> Vec<PadListing> {
    let scratchpads = scratchpads_in(base);
    let listing = |id: PadId| {
        let name = Scratchpad::load_from(&pad_in(base, &id))?.name;
        Some(PadListing { id, name })
    };

    let mut listed: Vec<PadListing> = PadOrder::load_from(&pad_recents_in(base))
        .scratchpads
        .into_iter()
        .filter_map(listing)
        .collect();

    let mut strays: Vec<PadListing> = match fs::read_dir(&scratchpads) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().and_then(PadId::new))
            .filter(|id| !listed.iter().any(|listed| &listed.id == id))
            .filter_map(listing)
            .collect(),
        Err(_) => Vec::new(),
    };
    strays.sort_by(|one, other| one.id.cmp(&other.id));

    listed.append(&mut strays);
    listed
}

/// Claim a directory for a new pad, write the default package into it, and hand back the
/// scratchpad. Blocking.
///
/// The claim *is* the `create_dir`: one atomic operation that fails with `AlreadyExists`
/// rather than opening what is there, so two copies of the app cannot hand out one id.
/// Bounded at a thousand tries, so a directory refusing every `create_dir` for a reason
/// other than collision cannot spin. The package is written **at once** rather than at the
/// first edit: pressing New is a deliberate act, and a claimed directory with no package in
/// it is not a pad and would be repaired away by [`pads_in`].
pub fn new_pad() -> Result<Scratchpad, Failure> {
    new_pad_in(&base().ok_or(Failure::NoDirectory)?)
}

fn new_pad_in(base: &Path) -> Result<Scratchpad, Failure> {
    let scratchpads = scratchpads_in(base);
    fs::create_dir_all(&scratchpads).map_err(|error| Failure::Write(error.to_string()))?;

    for n in 1..=1000 {
        let id = PadId(format!("{NEW_STEM}-{n}"));
        let directory = scratchpads.join(id.as_str());
        match fs::create_dir(&directory) {
            Ok(()) => {
                let scratchpad = Scratchpad::of(id);
                scratchpad.write_to(&directory)?;
                remember_in(base, scratchpad.id());
                return Ok(scratchpad);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Failure::Write(error.to_string())),
        }
    }

    Err(Failure::Write(format!(
        "no free id under {}",
        scratchpads.display()
    )))
}

/// Delete a pad: its directory and everything in it, cargo's `target/` included. Blocking,
/// and the only thing here that destroys what the reader wrote — which is why the app asks
/// before calling it.
///
/// **The path is narrowed twice.** It is [`pad_in`]'s and nothing else, and an id is a
/// checked crate name, so it can be neither `..`, nor a separator, nor an absolute path.
/// Then the directory must still be one [`Scratchpad::load_from`] answers for, so a
/// `remove_dir_all` can only reach a directory holding a manifest this module wrote.
/// [`fs::symlink_metadata`] is what makes that the directory itself rather than whatever a
/// link put in its place.
///
/// A pad with no directory is already deleted and says so: the pad a first run holds has
/// none until something is typed into it.
pub fn delete_pad(id: &PadId) -> Result<(), Failure> {
    delete_pad_in(&base().ok_or(Failure::NoDirectory)?, id)
}

fn delete_pad_in(base: &Path, id: &PadId) -> Result<(), Failure> {
    let directory = pad_in(base, id);
    let entry = match fs::symlink_metadata(&directory) {
        Ok(entry) => entry,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Failure::Delete(error.to_string())),
    };

    if !entry.is_dir() || Scratchpad::load_from(&directory).is_none() {
        return Err(Failure::Delete(format!(
            "{} is not a scratchpad",
            directory.display()
        )));
    }

    fs::remove_dir_all(&directory).map_err(|error| Failure::Delete(error.to_string()))
}

/// Start the program a build made in `directory`, streaming what it writes into `emit`
/// until it ends.
///
/// The artifact is run, not `cargo run`: re-entering cargo would rebuild to a path that
/// may differ from the one the diagnostics on screen are about, interleave cargo's own
/// progress into the program's output, and make stopping meaningless -- killing a
/// `cargo run` leaves its child running. Not blocking: it forks, wires up two threads to
/// the process's two pipes, and returns.
///
/// What comes back is a [`process::Handle`], whose one job is to stop the program and
/// everything it forked (`agents/Process.md`).
pub fn run_in(
    executable: &Path,
    directory: &Path,
    emit: impl FnMut(RunEvent) + Send + 'static,
) -> Result<process::Handle, Failure> {
    let mut command = Command::new(executable);
    command
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (running, pipes) =
        process::start(&mut command).map_err(|error| Failure::NoProgram(error.to_string()))?;

    // One `emit` behind one lock, so the two streams interleave in the order the program
    // wrote them. Holding it across the call is deliberate: a consumer that has fallen
    // behind blocks a reader thread, which fills a pipe, which blocks the program itself --
    // the only backpressure there is against a program printing in a tight loop.
    let emit: Emit = Arc::new(Mutex::new(Box::new(emit)));

    // A run is over when both pipes have reached the end **and** the process has been
    // reaped, and the last pipe to finish says so. A program that hands its output to a
    // grandchild outliving it therefore reads as still running, which is honest: the
    // output is still coming.
    let unfinished = Arc::new(AtomicUsize::new(2));
    pipe_thread(pipes.stdout, Stream::Out, &emit, &unfinished, &running);
    pipe_thread(pipes.stderr, Stream::Err, &emit, &unfinished, &running);

    Ok(running)
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
    running: &process::Handle,
) {
    match pipe {
        Some(pipe) => reading(pipe, stream, emit, unfinished, running),
        // A pipe that is not there is a pipe with nothing on it. Still a thread of its
        // own, so the count reaches zero and the reap happens off the caller's.
        None => reading(io::empty(), stream, emit, unfinished, running),
    }
}

/// The whole of the above with a pipe in hand.
///
/// A thread that will not start is a reader that has finished with nothing, and the run is
/// stopped rather than left with a stream nobody is reading.
fn reading<R: Read + Send + 'static>(
    pipe: R,
    stream: Stream,
    emit: &Emit,
    unfinished: &Arc<AtomicUsize>,
    running: &process::Handle,
) {
    let started = process::read_on_thread("a scratchpad's output reader", pipe, {
        let (emit, unfinished, running) = (emit.clone(), unfinished.clone(), running.clone());
        move |pipe| {
            process::stream_lines(BufReader::new(pipe), stream, |line| {
                let mut emit = emit.lock().unwrap_or_else(|held| held.into_inner());
                emit(RunEvent::Wrote(line));
            });

            reader_finished(&emit, &unfinished, &running);
        }
    });
    if let Err(error) = started {
        log::warn!("a scratchpad's output reader could not be started: {error}");
        // The pipe went with the closure that could not be started, so nothing will read
        // that stream and the program's own writes to it can only fail. End the run: the
        // count has to reach zero however a reader ends, or the process is never reaped,
        // the one `Ended` is never said, and the pad reads "Running" for ever over a
        // zombie. The stop also bounds the reap below, which is this thread's when the
        // other reader has already finished.
        running.stop();
        reader_finished(emit, unfinished, running);
    }
}

/// One reader is done with its pipe. The last of the two reaps the process and says how it
/// ended.
///
/// A process no longer under its lock was taken by a stop, which waited for it, so that is
/// what [`process::Handle::ended`] reads as [`process::Ended::Stopped`] -- and the reap
/// is also
/// what takes the run off the list a shutdown walks.
fn reader_finished(emit: &Emit, unfinished: &AtomicUsize, running: &process::Handle) {
    if unfinished.fetch_sub(1, Ordering::SeqCst) != 1 {
        return;
    }

    let ended = running.ended();
    let mut emit = emit.lock().unwrap_or_else(|held| held.into_inner());
    emit(RunEvent::Ended(ended));
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

/// `metadata` is last because it is a table and the three above it are not — the field
/// order rule again, and here it is also the only place cargo lets a tool of its own keep
/// anything: `[package.metadata]` is reserved for exactly this and cargo itself ignores it.
#[derive(Serialize, Deserialize)]
struct Package {
    name: PadId,
    version: String,
    edition: String,
    #[serde(default)]
    metadata: Metadata,
}

#[derive(Default, Serialize, Deserialize)]
struct Metadata {
    #[serde(default)]
    scratchpad: PadMetadata,
}

/// What the package cannot say for itself: the name the reader gave this pad, which is not
/// the crate's name and is under no obligation to be a crate name at all.
#[derive(Default, Serialize, Deserialize)]
struct PadMetadata {
    #[serde(default)]
    name: String,
    /// After `name`, which is a plain value: TOML puts a table after every value of the
    /// table it is in, so a struct written before it would take the fields under it with
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    built: Option<Built>,
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
            Failure::Delete(error) => write!(formatter, "could not delete the package: {error}"),
            Failure::NoCargo(error) => write!(formatter, "could not run cargo: {error}"),
            Failure::NoArtifact => write!(formatter, "cargo built nothing to open"),
            Failure::NoProgram(error) => write!(formatter, "could not start it: {error}"),
            Failure::Unreadable => write!(formatter, "the package could not be read"),
        }
    }
}

#[cfg(test)]
mod tests;

//! Projects: what the user gave — a name, a directory, the binaries in it — and what the
//! app noticed while they read them — the open documents, where each side of each was
//! left, which one was on screen and where the reader has been.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it
//! can move into a crate of its own.
//!
//! **A project is a directory, and its name is its identity on disk.** More than one can
//! exist; each lives in `projects/<id>/` under the app's own state directory, and
//! [`ProjectId`] *is* that directory's name. Nothing else identifies a project: not the
//! files in it (a binary can be open in two projects), not its given name (which may be
//! absent, may repeat, and may be changed), and not its associated directory (likewise).
//! An **anonymous** project — one the reader never named, which is every project this
//! build can currently make — is one whose `name` key is simply absent, exactly as an
//! unspecified font is an absent key in `settings.rs`. Its id is allocated by
//! [`ProjectId::anonymous`]: the first free `project-N`, claimed with a `create_dir` that
//! fails rather than overwrites, so it cannot collide with an id already on disk or with
//! a second copy of the app racing for one, it survives a restart because it is a
//! directory, and it costs the reader no decision at all. The `project-N` spelling is a
//! convenience for anyone looking at the directory and never a claim: what makes a
//! project anonymous is the missing name, not the shape of its id.
//!
//! **The two halves are two files**, which is the storage split `notes/Goals.md` asks
//! for and `settings.rs` took the first slice of. `project.toml` is what the user *said*:
//! the name, the associated directory and the binaries they opened. `session.toml` is
//! what the app *noticed*: the tabs, the rows each side of them was left at, the active
//! document, the history, and the digest each binary hashed to. The line is drawn where the save policy
//! already drew one — [`Saves`] writes a binaries change at once and leaves everything
//! else pending — so the file a user might reasonably keep, copy or edit is exactly the
//! file that is written only when they do something, and the file rewritten every thirty
//! seconds holds nothing they would miss. Three consequences follow and are the reason it
//! is worth two files rather than two tables in one: a `session.toml` that will not parse
//! loses a scroll position and not the list of binaries; a project can be copied
//! somewhere without a stranger's cursor coming with it; and the thirty-second write
//! touches a file nothing else is trying to read.
//!
//! Identity *inside* those files is not pointers — the UI's identity is `Arc` pointer
//! identity, which does not survive a restart — but *path + names + address* for a place
//! in a binary, and the path itself for a source file. That
//! mapping lives in exactly two places: [`SavedDocument::from_document`] going out and
//! [`SavedDocument::resolve`] coming back.
//!
//! The *when* of saving lives here too, in [`Saves`]: [`record`] is told what the app is
//! now showing and either writes it at once or marks it pending, and [`flush`] writes
//! whatever is pending. The app calls [`record`] from one observer of its state, [`flush`]
//! from a timer every [`AUTOSAVE_INTERVAL`], and [`flush`] once more when the window is
//! closed.
//!
//! *Which* project is open is [`Saves`]' too, and changing it is [`switch`] or
//! [`start_new`]: both flush the project being left, remember the one being entered and
//! re-point every baseline at it. Emptying the app of what belonged to the old project is
//! the caller's half of that, since the states are the UI's.
//!
//! There is no published version of this app, so a schema change is just a schema change:
//! a file that no longer parses is the default, not a migration.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use analysis::{Object, Symbol, SymbolData};
use serde::{Deserialize, Deserializer, Serialize};

use crate::history::History;
use crate::tabs::Positions;

/// The directory this app keeps its state in, under the platform's state directory
/// (falling back to its local data directory). The same constant `settings.rs` and
/// `scratchpad.rs` spell out for themselves, so the three stay separable.
const APP_DIR: &str = "assembly-viewer";
/// Every project's directory sits under this one, so the app's own files —
/// `settings.toml`, `recents.toml` — are never mistaken for a project.
const PROJECTS_DIR: &str = "projects";
const PROJECT_FILE: &str = "project.toml";
const SESSION_FILE: &str = "session.toml";
const RECENTS_FILE: &str = "recents.toml";

/// The stem an anonymous project's id is built from; see the module docs for why the
/// spelling carries no meaning.
const ANONYMOUS_STEM: &str = "project";

/// How many ids [`Recents`] keeps. What is lost past this is an *order*, never a project:
/// every project is a directory that is still there, and a view of them can list the
/// directory. A bound is worth having anyway, because this is a file the app appends to
/// for as long as it is ever used.
const MAX_RECENTS: usize = 50;

/// The longest an id may be, so a hand-edited or hostile `recents.toml` cannot ask for a
/// path component no filesystem will take.
const MAX_ID: usize = 64;

/// How many ids [`ProjectId::anonymous`] will try before giving up. A bound rather than a
/// loop, so a directory that refuses every `create_dir` for a reason other than collision
/// — read-only, out of inodes — cannot spin.
const MAX_ANONYMOUS: usize = 1000;

/// What is currently selected in the UI.
///
/// Lives here rather than in `ui.rs` because it is plain data over the analysis types —
/// no freya involved — and both persistence directions need to speak it.
/// **There is no "nothing" variant.** A `Selection` is always a place, and having none is
/// an absent one — `Option<Selection>` — which is the only spelling that stays honest once
/// a selection is one of the things a tab can hold: a variant meaning "nothing" would be a
/// tab for nowhere, and every list, every saved entry and every comparison would have to
/// know that one of its values is not really one.
#[derive(Clone)]
pub enum Selection {
    Object(Arc<Object>),
    Symbol(Symbol),
}

impl Selection {
    /// Whether this points into the file at `path` — the whole of what closing a file
    /// has to ask of a selection, of an open tab and of a history entry, which is why it
    /// lives on [`Selection`] rather than three times over at the sites that ask it.
    ///
    /// A symbol answers for the file its *object* came out of, so a file takes the
    /// symbols in it with it. `path` is [`Object::path`] and never an object's name: an
    /// archive member is not something the reader opened, so the unit that closes is the
    /// file, members and all.
    pub fn in_file(&self, path: &Path) -> bool {
        match self {
            Selection::Object(object) => object.path == path,
            Selection::Symbol(symbol) => symbol.object.path == path,
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Selection::Object(a), Selection::Object(b)) => Arc::ptr_eq(a, b),
            (Selection::Symbol(a), Selection::Symbol(b)) => a == b,
            _ => false,
        }
    }
}

/// One of the places the reader has open: a place in a binary, or a file.
///
/// **A document is a place in a binary *or* a file**, which is the doctrine Step 1
/// replaced "a document is a place in a binary" with. A tab holds one of these and has
/// two sides — assembly and source — and the variant says which side the tab is *about*
/// and therefore which one drives the other. An assembly-driven document has the function
/// on the subject side and the source it was compiled from beside it; a source-driven one
/// has the file on the subject side and the assembly for the line clicked in it beside
/// that. Opening a file and opening a function then produce the same kind of thing,
/// differing only in which way the mapping runs.
///
/// Here rather than in `ui.rs` for [`Selection`]'s reason: plain data over the analysis
/// types, with both persistence directions needing to speak it.
///
/// A file is the string the debug info said and never a path this filesystem was asked
/// about — it may well name the machine that compiled the binary — which is why it is an
/// `Arc<str>` and not a `PathBuf`.
#[derive(Clone)]
pub enum Document {
    Assembly(Selection),
    Source(Arc<str>),
}

impl Document {
    /// Whether this points into the file at `path` — what closing a binary asks of a tab
    /// and of a history entry.
    ///
    /// A source-driven document answers **false** whatever the path: a file chip outlives
    /// the binary that led the reader to it, because the text stands on its own and
    /// nothing records which object opened it. That is the rule the Source pane's own
    /// strip used to hold by simply not being consulted, kept now that the two strips are
    /// one.
    pub fn in_file(&self, path: &Path) -> bool {
        match self {
            Document::Assembly(selection) => selection.in_file(path),
            Document::Source(_) => false,
        }
    }

    /// The place in a binary this is about, or `None` for a file.
    pub fn selection(&self) -> Option<&Selection> {
        match self {
            Document::Assembly(selection) => Some(selection),
            Document::Source(_) => None,
        }
    }

    /// The symbol this is about: a document that is a function, and not one that is an
    /// object or a file. What the analysis worker is asked for.
    pub fn symbol(&self) -> Option<&Symbol> {
        match self.selection() {
            Some(Selection::Symbol(symbol)) => Some(symbol),
            _ => None,
        }
    }
}

impl PartialEq for Document {
    /// Each variant by its own rule — `Arc` pointer identity for a selection, text for a
    /// file — and never across the two. Which is what keeps "no two open tabs are ever
    /// equal" true of the one strip: it was already true within each of the two lists
    /// this merged, and a function and a file cannot be confused for each other.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Document::Assembly(a), Document::Assembly(b)) => a == b,
            (Document::Source(a), Document::Source(b)) => a == b,
            _ => false,
        }
    }
}

/// A project's identity: the name of the directory its two files live in.
///
/// A newtype rather than a `String` because it is interpolated into a path, and because
/// the one thing that must be true of it — that it is a single, ordinary path component
/// — is then true by construction. [`ProjectId::new`] is the only way to make one, and
/// `Deserialize` goes through it, so an id read out of a hand-edited `recents.toml`
/// cannot be `..`, an absolute path or a name with a separator in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl<'de> Deserialize<'de> for ProjectId {
    /// Validating on the way in, which is what keeps the invariant a property of the type
    /// rather than of the code that happens to have built one. A file holding an id that
    /// is not an id therefore fails to parse as a whole and [`Recents`] falls back to the
    /// default, which is this module's rule everywhere else — and the cost of it here is
    /// only that the *order* is forgotten, since every project is still its own directory.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ProjectId, D::Error> {
        let text = String::deserialize(deserializer)?;
        ProjectId::new(text).ok_or_else(|| serde::de::Error::custom("not a project id"))
    }
}

impl ProjectId {
    /// The id this text names, or `None` when it is not a single ordinary path component.
    ///
    /// Deliberately stricter than the filesystem: ASCII letters, digits, `-` and `_`
    /// only, starting with a letter or digit. A separator, a `.` or a `..` is what makes
    /// this a safety check, and the rest is so an id is the same string on every platform
    /// the app runs on — Windows refuses characters Linux takes, and a project written on
    /// one must be readable on the other.
    pub fn new(text: impl Into<String>) -> Option<ProjectId> {
        let text = text.into();
        if text.is_empty() || text.len() > MAX_ID {
            return None;
        }
        if !text.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            return None;
        }
        match text.contains(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            true => None,
            false => Some(ProjectId(text)),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Claim a directory for a project nobody named, and hand back its id.
    ///
    /// The claim *is* the `create_dir`: it is one atomic filesystem operation that fails
    /// with `AlreadyExists` rather than opening what is there, so the loop cannot hand out
    /// an id another run of the app — or another copy of it running right now — is already
    /// using. That is why the numbers are tried in order rather than derived from a listing
    /// of the directory, which would be a check followed by a race. The reader is asked
    /// nothing, which is the whole point: a project has to exist the moment they open a
    /// file, and demanding a name before that would put a dialog in front of the app.
    ///
    /// Bounded rather than unbounded; see [`MAX_ANONYMOUS`].
    fn anonymous(projects: &Path) -> Option<ProjectId> {
        fs::create_dir_all(projects).ok()?;
        for n in 1..=MAX_ANONYMOUS {
            let id = ProjectId(format!("{ANONYMOUS_STEM}-{n}"));
            match fs::create_dir(projects.join(id.as_str())) {
                Ok(()) => return Some(id),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    log::warn!(
                        "could not create a project directory in {}: {error}",
                        projects.display()
                    );
                    return None;
                }
            }
        }
        None
    }
}

/// The two things a user can give a project that are not files: what to call it, and
/// which directory it is about.
///
/// A type of its own because it is the half of [`Project`] that is *said* rather than
/// derived. The binaries follow from the objects that happen to be open; a name follows
/// from nothing at all, so until something on screen held one there was nowhere for
/// [`record`] to read it from and [`Saves`] had to carry it across the calls. The project
/// view holds one now, which is what lets this arrive at `record` like everything else —
/// and what makes [`Saves::given`] an ordinary baseline rather than a carried value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub name: Option<String>,
    pub directory: Option<PathBuf>,
}

impl Details {
    const fn new() -> Details {
        Details {
            name: None,
            directory: None,
        }
    }
}

/// The user-given half of a project: `project.toml`.
///
/// **The field order is load-bearing**, as it is in every struct in this module and in
/// `settings.rs`: TOML has no way to reopen a table once a later one has begun, so a
/// serializer must emit every plain value of a table before its first sub-table, and a
/// value written after a table fails at *runtime* with "values must be emitted before
/// tables". Every field here is a plain value, which is not an accident worth relying on
/// — the round-trip test is what holds it, here as everywhere else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What the reader called it, or **absent** when they never called it anything.
    ///
    /// `None` is a real third state and not an empty string, exactly as an unspecified
    /// font is in `settings.rs`: it is what makes a project anonymous, and a project view
    /// has to be able to tell "unnamed" from "named the empty string" in order to show
    /// something sensible in its place. `skip_serializing_if` is what writes it as a key
    /// that is not there — TOML has no null, so it is also the only way to write it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The directory the project is about — where its sources live, where a build would
    /// be run. Absent until something associates one; nothing in this build does yet.
    ///
    /// Not the directory the project is *stored* in, which is `projects/<id>/` and is
    /// never written down inside the file it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    /// The paths that were opened, deduplicated, in the order they were opened.
    pub binaries: Vec<PathBuf>,
}

impl Project {
    /// The name and directory of this project, which is what [`Saves`] carries across the
    /// [`record`] calls that cannot know them.
    fn details(&self) -> Details {
        Details {
            name: self.name.clone(),
            directory: self.directory.clone(),
        }
    }

    fn load_from(path: &Path) -> Option<Project> {
        let data = fs::read_to_string(path).ok()?;
        toml::from_str(&data).ok()
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_atomically(path, self)
    }
}

/// Every binary the loaded objects came out of, deduplicated, in the order they were
/// opened — which is [`Project::binaries`], derived rather than tracked.
///
/// The other half of the one mapping from live state to saved state, beside
/// [`Session::from_state`]; it is a free function only because its answer belongs to a
/// different file than the rest of what the objects say about themselves.
pub fn binaries(objects: &[Arc<Object>]) -> Vec<PathBuf> {
    let mut binaries: Vec<PathBuf> = Vec::new();
    for object in objects {
        if !binaries.contains(&object.path) {
            binaries.push(object.path.clone());
        }
    }
    binaries
}

/// The app-noticed half of a project: `session.toml`.
///
/// Everything here is a consequence of reading rather than a decision about what to read
/// — it changes on every click, it is rewritten on a timer, and losing it costs a few
/// clicks. That is the line between this file and [`Project`]; see the module docs.
///
/// **One list of tabs, not two.** Until Step 1 this held the content area's `tabs` beside
/// the Source pane's `sources` and a `shown` index into the second of them, because the
/// app had two strips with two notions of what was open. It has one, so this has one: the
/// strip's interleaved order is what the reader made and is what comes back, and `active`
/// is the one document that was on screen whichever kind it is.
///
/// **The field order is load-bearing.** Nothing here is a plain value any more, so the
/// rule has nothing to bite on directly — but that is a property of the current fields
/// and not a licence, and the round-trip test is what holds it, here as everywhere else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// What each opened binary's bytes hashed to when the session was saved, keyed by the
    /// path [`Project::binaries`] holds — the same identity every other saved thing is
    /// expressed in.
    ///
    /// In *this* file and not beside the binaries, although it is keyed by them, because
    /// the two are different jobs and the split is exactly that difference: `binaries` is
    /// the list of files to *open*, which the user chose, while a digest says nothing
    /// about what to open and everything about what to believe once it is open — the
    /// first piece of cached inspection data `notes/Goals.md` asks to keep here. It is
    /// also not a parallel array — the thing this module refuses everywhere else — since
    /// it is keyed rather than positional, so a path dropped from the other file cannot
    /// shift this one under it.
    ///
    /// The values are [`analysis::FileDigest`]'s own written form, sixteen lowercase hex
    /// digits, compared as text: a digest is only ever asked whether it is the same one,
    /// and text that this build did not write is simply not equal, which reads as
    /// "changed" — the answer that assumes the least.
    ///
    /// A path with **no** entry here is a third state and not a mismatch: a session
    /// written before digests existed, or a hand-edited file, says nothing about the
    /// bytes, so nothing new is done with it and the restore behaves exactly as it did
    /// before there was a digest to consult. See [`Rebuilt`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub digests: BTreeMap<PathBuf, String>,
    /// The document that was on screen.
    ///
    /// Written out in full rather than as an index into `tabs`, although the active
    /// document is by construction one of them, because the two are read back under
    /// different rules: a tab that no longer resolves is *dropped*, which would shift
    /// every later index, and this one *degrades* — a symbol to its object, an object to
    /// nothing — because there is one of it and the app has to open somewhere.
    ///
    /// `skip_serializing_if` because the `toml` crate cannot write a bare `None` at all
    /// — there is no null in TOML — so a session with nothing open has to leave the key
    /// out, and `default` to read that file back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<SavedDocument>,
    /// The open tabs, in strip order, of both kinds.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// `serde(default)` so a partial file — one written by hand, or trimmed — loads with
    /// an empty history rather than failing and taking the tabs and the active document
    /// down with it. The fields above carry it for the same reason.
    #[serde(default)]
    pub history: SavedHistory,
}

/// One of the open tabs: a place, and the row each of its two sides was left at.
///
/// The rows travel *with* the tab they belong to rather than in lists of their own beside
/// [`Session::tabs`], and that is the whole of why this type exists. A parallel array of
/// rows would be a second list to keep in step with the first, and it could not survive
/// the one thing that certainly happens to the first: [`Session::resolve_tabs`] drops the
/// tabs that no longer resolve, which would silently shift every later row onto the wrong
/// tab.
///
/// **Two rows, because a tab has two sides.** A document is a function beside its source
/// or a file beside the assembly for a line in it, and the reader leaves each side
/// somewhere; keying a source position by the *file* — which is what the Source pane's
/// own strip did — made two functions compiled from one file share a position they have
/// no reason to share.
///
/// The field order is load-bearing here too: both rows are plain values and `document` an
/// externally tagged enum, which TOML writes as a sub-table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Which row was at the top of the assembly side, `0` being the first instruction.
    ///
    /// A row and not a pixel offset, for [`crate::tabs::Positions`]' reasons — and one
    /// more that is only true of a saved one: the row height follows the fonts, so a
    /// session saved before the reader changed the assembly font would have every pixel
    /// offset land somewhere else, while every saved row still names the instruction it
    /// named.
    ///
    /// `serde(default)` because it is a hint and not a fact: a hand-written or trimmed
    /// file that names a tab without saying where in it simply opens that tab at the top.
    #[serde(default)]
    pub asm_row: usize,
    /// Which line was at the top of the source side, `0` being the file's first line. A
    /// hint like `asm_row` and defaulted for the same reason — and a file that has been
    /// edited shorter since is exactly what the clamp in
    /// [`crate::tabs::Positions::row`] is for.
    #[serde(default)]
    pub src_row: usize,
    pub document: SavedDocument,
}

/// The navigation history in saved form: the index of the entry that was on screen, and
/// every visited selection, oldest first.
///
/// The field order is load-bearing for the same reason [`Session`]'s is: `entries` is a
/// `Vec` of externally tagged enums, so TOML writes it as an array of tables, and the
/// plain `cursor` has to be emitted before it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    /// An index into `entries`, and `0` — meaning nothing — while it is empty.
    pub cursor: usize,
    #[serde(default)]
    pub entries: Vec<SavedDocument>,
}

impl SavedHistory {
    /// The empty history, as a `const fn` so [`Saves`] can be a `static`.
    pub const fn new() -> SavedHistory {
        SavedHistory {
            cursor: 0,
            entries: Vec::new(),
        }
    }

    /// The saved form of `history`. Every entry is a place, so nothing is dropped here
    /// and the cursor stays pointing at the same entry — a visited source file included,
    /// which is what makes the history panel able to list one.
    fn from_history(history: &History) -> SavedHistory {
        SavedHistory {
            cursor: history.cursor().unwrap_or(0),
            entries: history
                .entries()
                .iter()
                .map(SavedDocument::from_document)
                .collect(),
        }
    }
}

/// The projects the reader has had open, most recently first: `recents.toml`.
///
/// **Which project to reopen is the first entry and not a field of its own.** A `last`
/// beside the list would be a second answer to a question the order already answers, and
/// two answers is what this codebase refuses in `Tabs` and in `Session::selection` alike.
///
/// This is an *order*, not an index of what exists: the projects are the directories, and
/// a project that has fallen off the end of this list is still one of them. That is what
/// makes [`MAX_RECENTS`] safe, and it is why nothing here prunes an id whose directory has
/// gone — a listing that repaired itself on load would write a file on a startup where the
/// reader did nothing, and the repair is free at the point of use anyway.
///
/// The recent-projects view reads a name per row out of each project's own
/// `project.toml` ([`recent_projects`]) rather than out of this file: a name copied in
/// here would be a second copy to keep in step with the one the user edits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recents {
    #[serde(default)]
    pub projects: Vec<ProjectId>,
}

impl Recents {
    /// The project to reopen: the one most recently opened.
    fn first(&self) -> Option<&ProjectId> {
        self.projects.first()
    }

    /// Put `id` at the front, and say whether that changed anything.
    ///
    /// The answer is what keeps a startup that reopens the project already at the front
    /// from writing a file to say so. Trimming happens here rather than at the write, so
    /// the value in memory is the value on disk.
    fn touch(&mut self, id: &ProjectId) -> bool {
        if self.first() == Some(id) {
            return false;
        }
        self.projects.retain(|other| other != id);
        self.projects.insert(0, id.clone());
        self.projects.truncate(MAX_RECENTS);
        true
    }

    fn load_from(path: &Path) -> Recents {
        fs::read_to_string(path)
            .ok()
            .and_then(|data| toml::from_str(&data).ok())
            .unwrap_or_default()
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_atomically(path, self)
    }
}

/// One row of the recent-projects view: a project that can be switched to, described by
/// its own `project.toml`.
///
/// Everything but the id is read out of that file at the moment the list is asked for,
/// which is the whole point of the type — `recents.toml` is an *order* and says nothing
/// about what any of these projects is called, and a name cached beside the order would
/// be a second copy of the one the reader edits. The cost is a small read per row, paid
/// when the view is opened and when the open project changes rather than per render.
///
/// A row is a project the reader can be *put into*, so a project whose file will not
/// parse still gets one: it comes back as [`Project::default`], which is an unnamed
/// project with no binaries — exactly what it will behave as once opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub id: ProjectId,
    pub name: Option<String>,
    pub directory: Option<PathBuf>,
    /// How many binaries it would open, which is the one thing about a project that says
    /// how much is *in* it without opening it.
    pub binaries: usize,
}

/// The projects the reader has had open, most recently first, each described by its own
/// file — or an empty list on a system with nowhere to keep them.
pub fn recent_projects() -> Vec<Recent> {
    base()
        .map(|base| recent_projects_in(&base))
        .unwrap_or_default()
}

/// The whole of the above except finding the state directory, so a test can point it at
/// one of its own.
///
/// An id whose directory has gone is **dropped here rather than repaired**: [`Recents`]
/// deliberately never prunes itself on load, because that would write a file on a startup
/// where the reader did nothing, and this is the point of use where the repair is free.
fn recent_projects_in(base: &Path) -> Vec<Recent> {
    Recents::load_from(&recents_in(base))
        .projects
        .into_iter()
        .filter_map(|id| {
            let directory = project_in(base, &id);
            if !directory.is_dir() {
                return None;
            }
            let project = Project::load_from(&directory.join(PROJECT_FILE)).unwrap_or_default();
            Some(Recent {
                id,
                name: project.name,
                directory: project.directory,
                binaries: project.binaries.len(),
            })
        })
        .collect()
}

/// The binaries that are no longer the files the session was saved against.
///
/// This is the whole of what a digest *does*, and it is deliberately not much. A mismatch
/// is not an error, not a dialog and not a refusal to open — the file the reader asked for
/// is opened exactly as before — it only decides how much of a saved place may still be
/// believed:
///
/// - **The name is believed.** A rebuild keeps most of its function names, so a session
///   whose binary has moved on is mostly still valid and dropping it wholesale would throw
///   away far more than it protects. A saved place names a function the reader was
///   reading, and that name is what they meant by it.
/// - **The address is not.** It is not part of what the reader meant; it is only how the
///   symbol was found again, and it is the half a rebuild invalidates — the function is
///   somewhere else now and something else is where it was. So under a rebuilt file the
///   address stops being a *requirement*, which recovers every symbol that merely moved
///   (today those silently degrade to their object), and stops being *evidence*, which is
///   the half that fixes the bug: where a name is not unique in an object, a stale address
///   is exactly what lands the reader on the wrong one of them. A name that names two
///   symbols and no longer names an address therefore resolves to neither.
/// - **The row is not either.** A saved viewing position is a claim about a listing —
///   "row 57 of this function" — and a recompiled function has a different listing, so
///   that row is a different instruction and the pane would come back pointing
///   confidently at nothing. Such a tab opens at the top, where a tab opened for the
///   first time already opens.
///
/// A path this does not hold is not the same claim as a path known to be unchanged: it may
/// simply never have been hashed. Nothing here needs the difference, because both believe
/// the address, which is what the app did before there were digests at all.
#[derive(Debug, Default)]
struct Rebuilt(HashSet<PathBuf>);

impl Rebuilt {
    /// Compare every saved digest against the file that is loaded under that path now.
    ///
    /// Only a digest present on both sides and *different* is a rebuild: a saved path
    /// nothing was loaded for has nothing to compare against, and an object whose path
    /// was never hashed is the third state [`Session::digests`] describes. The comparison
    /// is per saved path rather than per object, so an archive's 196 members ask it once.
    fn of(session: &Session, objects: &[Arc<Object>]) -> Rebuilt {
        let mut rebuilt = HashSet::new();
        for (path, digest) in &session.digests {
            let Some(object) = objects.iter().find(|object| object.path == *path) else {
                continue;
            };
            if object.data.digest().to_string() != *digest {
                // Not a warning: a rebuilt binary is the normal thing to find after a
                // build, and the reader is told by what the restore does rather than by
                // being interrupted about it.
                log::debug!(
                    "{} has changed since the session was saved; matching by name",
                    path.display()
                );
                rebuilt.insert(path.clone());
            }
        }
        Rebuilt(rebuilt)
    }

    /// Whether the file at `path` has been rebuilt since the session named it.
    fn changed(&self, path: &Path) -> bool {
        self.0.contains(path)
    }
}

/// A [`Document`] expressed in terms that survive a restart.
///
/// **Flat, one table per entry, and not a document wrapping a saved selection.** The two
/// binary variants are what the file has always spelt and the third is a file, so a
/// nested enum would buy a second level of TOML table and one more thing for a
/// hand-edited file to get wrong, to express a distinction the variant names already
/// make.
///
/// `object_name` is [`Object::name`] — the archive member name, or the file name for a
/// plain object — and is needed because one path can contribute many `Object`s (every
/// member of an archive, plus the file itself), so `path` alone is ambiguous.
///
/// [`SavedDocument::Source`]'s `path` is a `String` rather than a `PathBuf` because it is
/// what the debug info said and not something this filesystem was asked about: it may
/// well name the machine that compiled the binary, and writing it as a path would only
/// invite the non-UTF-8 refusal in [`write_atomically`] on a value that was UTF-8 all
/// along.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedDocument {
    Object {
        path: PathBuf,
        object_name: String,
    },
    Symbol {
        path: PathBuf,
        object_name: String,
        symbol_name: String,
        address: u64,
    },
    Source {
        path: String,
    },
}

impl SavedDocument {
    /// The saved form of `document`. Total, because a [`Document`] is always a place:
    /// having none open is an absent one, which is the caller's `Option` and not a
    /// variant here.
    pub fn from_document(document: &Document) -> SavedDocument {
        match document {
            Document::Assembly(Selection::Object(object)) => SavedDocument::Object {
                path: object.path.clone(),
                object_name: object.name.clone(),
            },
            Document::Assembly(Selection::Symbol(symbol)) => SavedDocument::Symbol {
                path: symbol.object.path.clone(),
                object_name: symbol.object.name.clone(),
                symbol_name: symbol.data.name.clone(),
                address: symbol.data.address,
            },
            Document::Source(file) => SavedDocument::Source {
                path: file.to_string(),
            },
        }
    }

    /// The binary this names, or `None` for a file — which is not one of this app's
    /// binaries and is not a thing [`Rebuilt`] has anything to say about.
    fn binary_path(&self) -> Option<&Path> {
        match self {
            SavedDocument::Object { path, .. } | SavedDocument::Symbol { path, .. } => Some(path),
            SavedDocument::Source { .. } => None,
        }
    }

    fn object_name(&self) -> Option<&str> {
        match self {
            SavedDocument::Object { object_name, .. }
            | SavedDocument::Symbol { object_name, .. } => Some(object_name),
            SavedDocument::Source { .. } => None,
        }
    }

    /// The loaded object this names, if it is still there.
    fn find_object<'a>(&self, objects: &'a [Arc<Object>]) -> Option<&'a Arc<Object>> {
        let (path, name) = (self.binary_path()?, self.object_name()?);
        objects
            .iter()
            .find(|object| object.path == path && object.name == name)
    }

    /// Exactly what this names, or `None` when the object — or, for a symbol, the
    /// symbol — is no longer loaded.
    ///
    /// This is what history entries want: an entry that no longer points where it did
    /// is dropped, rather than quietly turned into a different destination that the
    /// user never visited.
    ///
    /// `rebuilt` is what decides whether the saved address is a fact about this file or
    /// only a memory of the one it was saved against; see [`Rebuilt`] and
    /// [`SavedDocument::find_symbol`].
    ///
    /// **A source-driven entry resolves against nothing and so cannot fail.** It is a
    /// path a compiler wrote down, this app never asked the filesystem about it, and a
    /// file that has since been deleted still comes back as a tab over the pane's own
    /// "Source file not found" — which is the true answer and a visible one, where
    /// dropping the tab would lose a file the reader had open without ever saying so.
    fn resolve(&self, objects: &[Arc<Object>], rebuilt: &Rebuilt) -> Option<Document> {
        if let SavedDocument::Source { path } = self {
            return Some(Document::Source(Arc::from(path.as_str())));
        }

        let object = self.find_object(objects)?;
        let selection = match self {
            SavedDocument::Object { .. } => Selection::Object(object.clone()),
            SavedDocument::Symbol {
                path,
                symbol_name,
                address,
                ..
            } => SavedDocument::find_symbol(object, symbol_name, *address, rebuilt.changed(path))
                .map(|data| {
                Selection::Symbol(Symbol {
                    object: object.clone(),
                    data: data.clone(),
                })
            })?,
            SavedDocument::Source { .. } => unreachable!("answered above"),
        };
        Some(Document::Assembly(selection))
    }

    /// The symbol a saved place names, under a file that either is or is not the one it
    /// was saved against.
    ///
    /// **Unchanged** (or never hashed): the name *and* the address, which is what this
    /// has always done — the address is what tells two same-named symbols apart, and in a
    /// file that has not moved it is exact.
    ///
    /// **Rebuilt**: the name, with the address as a tie-breaker only. The exact match is
    /// still preferred where there is one, since a symbol that did not move is the same
    /// symbol however much of the file around it changed. Failing that, a name that names
    /// exactly one symbol *is* an identity and resolves to it — this is where a session
    /// whose functions have all shifted comes back rather than collapsing onto its
    /// objects. A name that names several and matches no address resolves to **nothing**:
    /// the address that would have chosen between them describes a layout this file no
    /// longer has, and picking one of them on the strength of it is precisely how a
    /// reader ends up on a function they never opened.
    ///
    /// The unchanged case keeps the search it always was — a `find` that stops at the
    /// symbol, over a `symbols_sorted` that is 115k entries on the repo's own binary and
    /// is asked once per tab and once per history entry. Only a rebuilt file pays for the
    /// whole pass, and only because "does this name name anything else" cannot be
    /// answered by stopping at the first one.
    fn find_symbol<'a>(
        object: &'a Object,
        name: &str,
        address: u64,
        rebuilt: bool,
    ) -> Option<&'a Arc<SymbolData>> {
        if !rebuilt {
            return object
                .symbols_sorted
                .iter()
                .find(|data| data.name == name && data.address == address);
        }

        let mut exact = None;
        let mut named = None;
        let mut names = 0usize;

        for data in &object.symbols_sorted {
            if data.name != name {
                continue;
            }
            names += 1;
            if data.address == address {
                exact = Some(data);
            }
            named.get_or_insert(data);
        }

        exact.or(match names == 1 {
            true => named,
            false => None,
        })
    }

    /// The same, degrading instead of failing: a symbol that is gone falls back to its
    /// object and an object that is gone to nothing at all.
    ///
    /// This is what the *active document* wants. There is only one of it and it is where
    /// the app opens, so landing near the last session's place beats landing nowhere;
    /// a history entry, of which there are many, is better dropped. A source-driven entry
    /// never reaches the fallback, having never failed.
    fn resolve_or_degrade(&self, objects: &[Arc<Object>], rebuilt: &Rebuilt) -> Option<Document> {
        self.resolve(objects, rebuilt).or_else(|| {
            self.find_object(objects)
                .cloned()
                .map(|object| Document::Assembly(Selection::Object(object)))
        })
    }
}

impl Session {
    /// The empty session, as a `const fn` so [`Saves`] can be a `static`.
    pub const fn new() -> Session {
        Session {
            digests: BTreeMap::new(),
            active: None,
            tabs: Vec::new(),
            history: SavedHistory::new(),
        }
    }

    /// The session described by the state the app is currently in: the loaded `objects`,
    /// the open `tabs` with the `active` one, the `history`, and the row each side of
    /// each tab was left at.
    ///
    /// The one place the app's state is turned into what would be saved — [`binaries`]
    /// being the other half of it, for the other file — so the save policy in [`Saves`]
    /// never has to know where any of it came from. It takes the tab list as a plain
    /// slice rather than a `Tabs<T>` for exactly that reason: this is a mapping over what
    /// is open, not a party to how the list is kept. The positions come as two
    /// [`Positions`] rather than slices only because that is the shape of the question —
    /// "where was this side of this tab left" — and a side that was never scrolled has no
    /// entry in one at all, which is written out as row `0`.
    pub fn from_state(
        objects: &[Arc<Object>],
        tabs: &[Document],
        asm_rows: &Positions<Document>,
        src_rows: &Positions<Document>,
        active: Option<&Document>,
        history: &History,
    ) -> Session {
        let mut digests: BTreeMap<PathBuf, String> = BTreeMap::new();
        for object in objects {
            // Read off the object rather than computed here: the hash was taken once, on
            // the parse worker thread, while the file's bytes were in hand, and every
            // object out of one file answers the same thing — so an archive's members
            // all write the same entry over each other rather than costing a pass each.
            digests
                .entry(object.path.clone())
                .or_insert_with(|| object.data.digest().to_string());
        }
        Session {
            digests,
            active: active.map(SavedDocument::from_document),
            tabs: tabs
                .iter()
                .map(|tab| SavedTab {
                    asm_row: asm_rows.at(tab).unwrap_or(0),
                    src_row: src_rows.at(tab).unwrap_or(0),
                    document: SavedDocument::from_document(tab),
                })
                .collect(),
            history: SavedHistory::from_history(history),
        }
    }

    /// Turn the saved active document back into a live one against the objects that are
    /// now loaded. Binaries change between runs, so this degrades silently: a symbol that
    /// is gone falls back to its object, and an object that is gone to nothing at all.
    ///
    /// A binary that has been *rebuilt* since it was saved changes what "is gone" means
    /// — see [`Rebuilt`], which is where the digests are compared.
    pub fn resolve(&self, objects: &[Arc<Object>]) -> Option<Document> {
        let saved = self.active.as_ref()?;
        saved.resolve_or_degrade(objects, &Rebuilt::of(self, objects))
    }

    /// Turn the saved tabs back into live documents against the objects that are now
    /// loaded, in strip order, each with the rows its two sides were left at. A tab that
    /// no longer resolves is **dropped**, the way a history entry is and pointedly not
    /// the way the active document is.
    ///
    /// The active document degrades because there is one of it and the app has to open
    /// somewhere; a tab is one of many, and a strip whose tabs lead to places that are
    /// no longer there — or, worse, that all degraded onto the same object and so
    /// collapsed into one tab — is worse than a shorter strip. A source-driven tab is
    /// never dropped, having nothing to resolve against.
    ///
    /// Duplicates need no attention here: `Tabs::open` already refuses to open a second
    /// tab for something that is open, so two saved tabs that degrade onto one live
    /// document could not both be opened even if they got this far.
    ///
    /// Each surviving tab comes back with the rows it was left at, which is why they are
    /// fields of the tab rather than lists beside it: the dropping here is exactly what a
    /// parallel array could not have survived.
    pub fn resolve_tabs(&self, objects: &[Arc<Object>]) -> Vec<(Document, usize, usize)> {
        let rebuilt = Rebuilt::of(self, objects);
        self.tabs
            .iter()
            .filter_map(|saved| {
                let document = saved.document.resolve(objects, &rebuilt)?;
                // A row is a claim about a listing, so a listing that has been rebuilt
                // takes both its rows with it; see [`Rebuilt`]. The tab itself survives —
                // it is the function that is being read, not the offset into it. A file
                // has no binary path and so is never rebuilt, which is the honest answer:
                // the app did not compile it and knows nothing about whether it moved.
                let changed = saved
                    .document
                    .binary_path()
                    .is_some_and(|path| rebuilt.changed(path));
                match changed {
                    true => Some((document, 0, 0)),
                    false => Some((document, saved.asm_row, saved.src_row)),
                }
            })
            .collect()
    }

    /// Turn the saved history back into a live one against the objects that are now
    /// loaded. An entry that no longer resolves is dropped, so a session whose binaries
    /// have changed comes back with the entries that still mean something rather than
    /// with none at all.
    ///
    /// The cursor follows the drops, and *how* is [`History::rebuilt`]'s business rather
    /// than this function's: all that happens here is the resolving, one saved entry at a
    /// time, with `None` where an entry no longer points anywhere. Closing a file in a
    /// running session loses entries for the same reason and goes through the same walk
    /// ([`History::retaining`]), which is what makes the two behave identically rather
    /// than merely alike.
    ///
    /// A binary that has been rebuilt since it was saved is matched by name rather than
    /// by name and address ([`Rebuilt`]), so a history over a recompiled file comes back
    /// pointing at the functions it named rather than at their old offsets.
    ///
    /// Duplicates are [`History::restored`]'s business, and neither this function's nor
    /// `rebuilt`'s: two saved entries naming the same destination resolve to the same
    /// `Arc` and so to equal entries, which a saved history written before entries were
    /// bumped rather than appended is full of.
    pub fn resolve_history(&self, objects: &[Arc<Object>]) -> History {
        let rebuilt = Rebuilt::of(self, objects);
        History::rebuilt(
            self.history
                .entries
                .iter()
                .map(|saved| saved.resolve(objects, &rebuilt)),
            self.history.cursor,
        )
    }

    fn load_from(path: &Path) -> Option<Session> {
        let data = fs::read_to_string(path).ok()?;
        toml::from_str(&data).ok()
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_atomically(path, self)
    }
}

/// The directory the app keeps everything in, or `None` on a system with no state or
/// local data directory to put it in.
fn base() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(APP_DIR))
}

/// Where every project's directory lives.
fn projects_in(base: &Path) -> PathBuf {
    base.join(PROJECTS_DIR)
}

/// Where one project's two files live.
fn project_in(base: &Path, id: &ProjectId) -> PathBuf {
    projects_in(base).join(id.as_str())
}

fn recents_in(base: &Path) -> PathBuf {
    base.join(RECENTS_FILE)
}

/// Write `value` as TOML to `path`, by writing `path.tmp` first and renaming it over the
/// top, so an interrupted write cannot leave a half-written file behind — and so a file
/// being read by another copy of the app is either the old one or the new one, never a
/// truncated one that would silently load as the default.
///
/// TOML has no way to spell a path that is not UTF-8, and serde's `PathBuf` impl fails
/// rather than mangling one, so such a project is simply not written: the error becomes an
/// IO error here and is logged and swallowed by the caller, which leaves the previous good
/// file in place. Nothing panics, and nothing lossy reaches the disk to be loaded back as
/// a different path.
fn write_atomically(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let data = toml::to_string_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    fs::write(&temporary, data)?;
    fs::rename(&temporary, path)
}

/// How often [`flush`] is worth calling.
///
/// The write is a few hundred bytes of TOML, so the cost of a tick is a comparison that
/// almost always finds nothing pending. Thirty seconds is far coarser than the rate a
/// user clicks through symbols at — a long burst of navigation collapses into one write
/// — while bounding what an unclean exit can lose to half a minute of history and one
/// selection, neither of which is expensive to redo. A clean window close flushes
/// anyway, so this only ever covers a kill or a crash.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// The save policy: which project is open, what has been written into it, and what is
/// waiting to be.
///
/// A `static` rather than UI state because two of the three things that drive it — the
/// periodic flush and the window's close hook — sit outside the component tree, and the
/// close hook cannot reach into it at all.
static SAVES: Mutex<Saves> = Mutex::new(Saves::new());

/// What one [`Saves::record`] decided has to reach the disk now, and which file each half
/// of it goes in.
///
/// Two fields rather than a pair because the session is not always owed: a change to the
/// user-given half of a project is a `project.toml` write and nothing else, while a
/// binaries change is both files at once. Saying which is which here rather than at the
/// call site keeps the decision in the one function that is allowed to make it.
#[derive(Debug, PartialEq, Eq)]
struct Written {
    project: Project,
    session: Option<Session>,
}

struct Saves {
    /// The project everything is written into, or `None` until one has been reopened or
    /// created. Set by [`reopen`] at startup and otherwise allocated on the first write
    /// that has anything to say, which is what makes a run where nothing was ever opened
    /// leave no directory behind.
    open: Option<ProjectId>,
    /// The name and directory as last written — the baseline a rename is measured
    /// against, exactly as `binaries` below is the baseline an open or a close is
    /// measured against.
    ///
    /// **Every baseline here is the state the app boots into**, and that is why this one
    /// is seeded from the loaded project while the two below are pointedly empty. The
    /// binaries and the session are restored *asynchronously* — the app boots with
    /// nothing open and fills in when the parse lands — so a baseline holding the loaded
    /// values would read the boot state as a change and write an empty project over a
    /// good one. The name and directory are restored *synchronously*, into the state the
    /// project view renders, before a single effect has run; so the state the app boots
    /// into is the loaded one, and the baseline has to say so or the first record would
    /// write the name straight back out again.
    ///
    /// It used to be a value carried across the `record` calls rather than a baseline,
    /// because nothing on screen held a name for one to derive. The project view does
    /// (`ui.rs`, `Proj`), which is what collapsed the special case.
    given: Details,
    /// The binaries the app was last seen holding.
    ///
    /// Empty to start with — deliberately not the ones loaded at startup. The state the
    /// app boots into equals this baseline, so nothing is ever pending before something is
    /// actually opened: a run in which nothing is opened, one whose restore finds no
    /// readable binary, and a flush that fires while the startup parse is still in flight
    /// all leave a good file on disk untouched. Seeding it from the loaded project would
    /// invert that, making the first comparison see the still-empty state as a change and
    /// write an empty project over a good one.
    binaries: Vec<PathBuf>,
    /// What `project.toml` currently *says* the binaries are.
    ///
    /// The same list as `binaries` above in every state but one, and that one is the
    /// reason this exists: between a project being reopened and its parse landing — or
    /// for good, when every binary in it has been deleted or will not parse — the app
    /// holds none while the file names several, and the restore deliberately writes
    /// nothing so that a binary which is only temporarily missing is not forgotten. A
    /// rename in that window is an immediate write, and writing the *app's* list would
    /// forget them after all, through a change that had nothing to do with them. So a
    /// write that is not about the binaries writes back the ones already in the file.
    ///
    /// Seeded by [`Saves::opened`] from the project just entered, and replaced by every
    /// write that is about the binaries — which is the only kind that may replace it.
    listed: Vec<PathBuf>,
    /// The session as last written, empty for the same reason.
    session: Session,
    /// A newer session that has not been written yet; `None` when there is nothing to
    /// write. Only ever a *session*: a change to the other file is written at once, so
    /// there is never a `project.toml` waiting.
    pending: Option<Session>,
}

impl Saves {
    const fn new() -> Saves {
        Saves {
            open: None,
            given: Details::new(),
            binaries: Vec::new(),
            listed: Vec::new(),
            session: Session::new(),
            pending: None,
        }
    }

    /// The newest session this knows about, whether or not it reached the disk.
    fn latest(&self) -> &Session {
        self.pending.as_ref().unwrap_or(&self.session)
    }

    /// The `project.toml` that `binaries` and the carried [`Details`] describe.
    fn project(&self, binaries: Vec<PathBuf>) -> Project {
        Project {
            name: self.given.name.clone(),
            directory: self.given.directory.clone(),
            binaries,
        }
    }

    /// Note that `id` is the project the app is now in, and set every baseline to the
    /// state the app will be in the instant afterwards — see [`Saves::given`], which is
    /// where the asymmetry between the three of them is written out.
    ///
    /// Called at startup by [`reopen`] and at runtime by [`switch`] and [`start_new`],
    /// which is the reason the two empty baselines are *assigned* rather than assumed:
    /// at startup they are already empty, but a project switched away from leaves its own
    /// binaries and its own pending session behind, and neither of those describes the
    /// project being entered. Anything the old one had left pending is [`switch`]'s to
    /// flush before it gets here.
    fn opened(&mut self, id: ProjectId, project: &Project) {
        self.open = Some(id);
        self.given = project.details();
        self.binaries = Vec::new();
        self.listed = project.binaries.clone();
        self.session = Session::new();
        self.pending = None;
    }

    /// Take note of the state the app is now in. Hands back what has to be written right
    /// now, or `None` when nothing changed or the change can wait for a [`Saves::flush`].
    ///
    /// Which binaries are open is a user project change: it is the result of a deliberate
    /// action, it is what every other part of the session is expressed against, and it is
    /// the one thing that is annoying to redo — so it goes to disk at once, carrying
    /// whatever session was pending with it, which is what keeps the two files from
    /// disagreeing across a crash: `session.toml` must never name a tab into a binary
    /// `project.toml` no longer lists. A selection or a history entry only marks the
    /// session dirty.
    ///
    /// A tab is pending too, and deliberately so although opening one is every bit as
    /// deliberate an action as opening a binary. It fails the other two tests: a tab is
    /// expressed *against* the binaries rather than the other way round, and it costs one
    /// click to make again, where a lost binary costs a file dialog and a reparse. It
    /// also arrives far too often to write — `activate` opens a tab on the way to every
    /// selection change, so an immediate write here would put a file on the disk for
    /// every symbol the reader clicks, which is exactly the traffic the pending/flush
    /// split exists to collapse. Nothing in this function has to say so: which file a
    /// field lives in is what decides it.
    ///
    /// A rename is the other immediate write, and for the first of those three reasons
    /// alone: naming a project or pointing it at a directory is exactly as deliberate as
    /// opening a binary, and `notes/Goals.md` asks that a user project change be on disk
    /// before the next click. It is immediate *per keystroke*, which is what "before the
    /// next click" means for a text box — a few hundred bytes written atomically, against
    /// a rename being something a reader does once a project.
    ///
    /// The one thing it does **not** do is write the session with it, which a binaries
    /// change must. That rule exists so `session.toml` can never name a tab into a binary
    /// `project.toml` has already let go of; a rename lets go of nothing, so the session
    /// it was recorded beside stays exactly as pending as it was.
    fn record(
        &mut self,
        details: Details,
        binaries: Vec<PathBuf>,
        session: Session,
    ) -> Option<Written> {
        let binaries_changed = self.binaries != binaries;
        let details_changed = self.given != details;

        if !binaries_changed && !details_changed {
            if *self.latest() != session {
                self.pending = Some(session);
            }
            return None;
        }

        self.given = details;
        self.binaries = binaries.clone();
        // A write that is not about the binaries keeps the ones already in the file; see
        // [`Saves::listed`], which is the whole of that rule.
        let project = match binaries_changed {
            true => {
                self.listed = binaries.clone();
                self.project(binaries)
            }
            false => self.project(self.listed.clone()),
        };

        if binaries_changed {
            self.session = session.clone();
            self.pending = None;
            return Some(Written {
                project,
                session: Some(session),
            });
        }

        if *self.latest() != session {
            self.pending = Some(session);
        }
        Some(Written {
            project,
            session: None,
        })
    }

    /// Take whatever was recorded but not written, or `None` when the two already agree.
    /// Only the session: see [`Saves::pending`].
    fn flush(&mut self) -> Option<Session> {
        let session = self.pending.take()?;
        self.session = session.clone();
        Some(session)
    }
}

fn saves() -> MutexGuard<'static, Saves> {
    // Nothing under this lock can panic short of an allocation failure, but take the
    // state back rather than propagate if something ever does: a poisoned lock must not
    // turn a failed save into a crashed app.
    SAVES.lock().unwrap_or_else(|error| error.into_inner())
}

/// The project everything is written into, creating an anonymous one — and remembering it
/// as the most recent — if there is not one yet.
///
/// Called from the write paths and nowhere else, which is what makes a project appear on
/// disk exactly when there is something to put in it. `None` is a system with no state
/// directory, or one where the directory could not be made — the same silence as any other
/// failed save.
///
/// The reopened case needs no [`remember`]: it *is* the front of the list, that being how
/// it was chosen. [`switch`] and [`start_new`] are the two that pick a different one, and
/// both of them do remember it — which is why neither goes through here.
fn open_project(saves: &mut Saves, base: &Path) -> Option<ProjectId> {
    if let Some(id) = &saves.open {
        return Some(id.clone());
    }
    let id = ProjectId::anonymous(&projects_in(base))?;
    log::debug!("started the anonymous project {}", id.as_str());
    remember(base, &id);
    saves.open = Some(id.clone());
    Some(id)
}

/// Put `id` at the front of `recents.toml`, writing the file only when that moved it.
fn remember(base: &Path, id: &ProjectId) {
    let path = recents_in(base);
    let mut recents = Recents::load_from(&path);
    if !recents.touch(id) {
        return;
    }
    if let Err(error) = recents.save_to(&path) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Reopen the project the app was last in: the first entry of `recents.toml`.
///
/// Hands back both halves for the caller to restore, and tells the save policy which
/// directory it is writing into — but seeds it with nothing else, so the app still boots
/// into a state the policy sees as unwritten (see [`Saves::binaries`]).
///
/// **The directory is the project**, and either of the two files in it being missing or
/// unreadable is simply the default half — which is the split earning its keep twice over.
/// A `session.toml` that will not parse costs a scroll position and not the list of
/// binaries; and a directory that was created a moment before the app was killed, so that
/// neither file was written, is reopened as the empty project it is rather than orphaned
/// while a second one is allocated beside it. Only "no recent project" and "that directory
/// is gone" are `None`.
pub fn reopen() -> Option<(ProjectId, Project, Session)> {
    let (id, project, session) = reopen_in(&base()?)?;
    saves().opened(id.clone(), &project);
    Some((id, project, session))
}

/// The whole of the above except telling [`Saves`], which is what makes it testable
/// against a directory of the test's own rather than against the user's real one.
fn reopen_in(base: &Path) -> Option<(ProjectId, Project, Session)> {
    let recents = Recents::load_from(&recents_in(base));
    let id = recents.first()?.clone();
    let (project, session) = load_project(base, &id)?;
    Some((id, project, session))
}

/// Both halves of the project `id` names, or `None` when its directory is gone.
///
/// The directory is the only thing that has to be there: either file being missing or
/// unreadable is simply the default half, which is the storage split earning its keep.
fn load_project(base: &Path, id: &ProjectId) -> Option<(Project, Session)> {
    let directory = project_in(base, id);
    if !directory.is_dir() {
        log::debug!("the project {} is no longer there", id.as_str());
        return None;
    }

    let project = Project::load_from(&directory.join(PROJECT_FILE)).unwrap_or_default();
    let session = Session::load_from(&directory.join(SESSION_FILE)).unwrap_or_default();
    Some((project, session))
}

/// Leave the project the app is in and enter the one `id` names, handing back both halves
/// for the caller to restore. `None` — and nothing changed at all — when its directory has
/// gone since the recent list named it.
///
/// Three things happen in an order that matters. The project being left is **flushed
/// first**, while [`Saves`] still points at it, because everything pending belongs to it
/// and a moment later there will be nowhere to put it. The new project is then
/// **remembered**, since it is now the one a restart should reopen — the one caller
/// [`open_project`]'s doc anticipated. And [`Saves::opened`] **empties the baselines**,
/// because the caller is about to empty the app: every binary, tab and history entry on
/// screen belongs to the project being left, and a baseline still describing them would
/// read that emptying as a change and write it into the project just entered.
///
/// What this cannot do is empty the app, which is the other half of a switch and is
/// `ui.rs`'s: the states are the UI's and are put back through the same functions a
/// restore goes through.
pub fn switch(id: &ProjectId) -> Option<(Project, Session)> {
    flush();
    let base = base()?;
    let (project, session) = load_project(&base, id)?;
    remember(&base, id);
    saves().opened(id.clone(), &project);
    log::debug!("switched to the project {}", id.as_str());
    Some((project, session))
}

/// Start a project nobody has named yet and enter it, handing back its id.
///
/// [`switch`] with nothing to load: an anonymous project is a directory and two files
/// that do not exist yet, so entering one is claiming the directory and pointing the save
/// policy at it. The two files appear when there is something to put in them, which is
/// the same rule that governs the one the app allocates for itself.
pub fn start_new() -> Option<ProjectId> {
    flush();
    let base = base()?;
    let id = ProjectId::anonymous(&projects_in(&base))?;
    remember(&base, &id);
    saves().opened(id.clone(), &Project::default());
    log::debug!("started the project {}", id.as_str());
    Some(id)
}

/// Take note of the project the app is now in, writing it out immediately if it is a
/// change that must not be lost and marking it pending otherwise.
///
/// Cheap enough to call on every state change: an unchanged project does nothing at all.
pub fn record(details: Details, binaries: Vec<PathBuf>, session: Session) {
    // The write happens under the lock, so two writes can never reach the file out of
    // the order they were decided in. Everything that calls this is on the main thread
    // today, so nothing ever waits on it.
    let mut saves = saves();
    let Some(written) = saves.record(details, binaries, session) else {
        return;
    };
    let Some(directory) = writing_into(&mut saves) else {
        log::warn!("no state directory to save the project in");
        return;
    };
    write_or_warn(&directory.join(PROJECT_FILE), |path| {
        written.project.save_to(path)
    });
    if let Some(session) = written.session {
        write_or_warn(&directory.join(SESSION_FILE), |path| session.save_to(path));
    }
}

/// Write out anything recorded but not yet written. A no-op when nothing has changed,
/// which is what makes it safe to call on a timer.
pub fn flush() {
    let mut saves = saves();
    let Some(session) = saves.flush() else {
        return;
    };
    let Some(directory) = writing_into(&mut saves) else {
        log::warn!("no state directory to save the session in");
        return;
    };
    write_or_warn(&directory.join(SESSION_FILE), |path| session.save_to(path));
}

/// The directory the two files go in, allocating a project for them if this is the first
/// write of the run and nothing was reopened.
fn writing_into(saves: &mut Saves) -> Option<PathBuf> {
    let base = base()?;
    let id = open_project(saves, &base)?;
    Some(project_in(&base, &id))
}

/// Any IO failure is logged and swallowed: failing to persist is never worth interrupting
/// the user for.
fn write_or_warn(path: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) {
    if let Err(error) = write(path) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests;

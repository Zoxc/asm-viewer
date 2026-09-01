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
mod tests {
    use std::collections::HashMap;

    use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolData};

    use super::*;

    /// A bare `Object` with the given text symbols. The analysis crate's own fixtures
    /// go through `parse_object`; here only the fields the mapping reads matter, and
    /// every one of them is public, so the objects are built directly.
    fn object(path: &str, name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
        built(path, name, symbols, b"the first build")
    }

    /// The same, out of a named build of the file. `bytes` is only ever hashed here —
    /// these objects were not parsed from it — so "the file was rebuilt" is spelt as two
    /// calls with different bytes.
    fn built(path: &str, name: &str, symbols: &[(&str, u64)], bytes: &[u8]) -> Arc<Object> {
        let section = Arc::new(Section {
            index: SectionIndex(0),
            name: ".text".into(),
            data: vec![0xC3; symbols.len()],
            address: 0,
            relocations: HashMap::new(),
            symbols: symbols.iter().map(|(_, address)| *address).collect(),
        });

        let symbols_sorted: Vec<Arc<SymbolData>> = symbols
            .iter()
            .map(|(name, address)| {
                Arc::new(SymbolData {
                    name: (*name).to_owned(),
                    demangled: None,
                    address: *address,
                    section: Some(section.clone()),
                    size: 0,
                })
            })
            .collect();

        Arc::new(Object {
            path: PathBuf::from(path),
            name: name.to_owned(),
            format: BinaryFormat::Elf,
            architecture: Architecture::X86_64,
            symbols: HashMap::new(),
            symbols_sorted,
            sections: vec![section],
            // The mapping never looks at the bytes themselves; what it does look at is
            // the digest they hash to, which is what says whether this is still the file
            // the session was saved against.
            data: ObjectData::from(bytes),
            dwarf: Default::default(),
        })
    }

    /// [`Session::from_state`] over a session whose only open tab is the active document
    /// and whose panes are at the top of what they show — the state the tests written
    /// before there were tabs to save were already describing, now spelt out.
    ///
    /// It takes a [`Selection`] because every test that reaches for it is about a place in
    /// a binary; a source-driven tab has its own tests below.
    fn from_state(
        objects: &[Arc<Object>],
        selection: Option<&Selection>,
        history: &History,
    ) -> Session {
        let document = selection.cloned().map(Document::Assembly);
        Session::from_state(
            objects,
            document
                .as_ref()
                .map(std::slice::from_ref)
                .unwrap_or_default(),
            &Positions::default(),
            &Positions::default(),
            document.as_ref(),
            history,
        )
    }

    /// What [`Session::resolve`] answers, as the selection inside it. Every use of this
    /// is a test about a place in a binary, which is what the app had before a document
    /// could also be a file.
    fn resolve_selection(session: &Session, objects: &[Arc<Object>]) -> Option<Selection> {
        session
            .resolve(objects)
            .and_then(|document| document.selection().cloned())
    }

    fn objects() -> Vec<Arc<Object>> {
        vec![
            object("/tmp/lib.a", "a.o", &[("caller", 0), ("target", 6)]),
            // Same path, different member: `path` alone cannot tell these apart.
            object("/tmp/lib.a", "b.o", &[("caller", 0)]),
        ]
    }

    /// The one question closing a file asks. A member is not a file, so both members of
    /// `/tmp/lib.a` answer for it and a symbol answers for the file its object came out
    /// of — closing the archive takes every one of them.
    #[test]
    fn everything_in_a_file_says_so() {
        let objects = objects();
        let lib = Path::new("/tmp/lib.a");
        let other = Path::new("/tmp/some.dll");

        let member = Selection::Object(objects[1].clone());
        assert!(member.in_file(lib));
        assert!(!member.in_file(other));

        let symbol = Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[0].clone(),
        });
        assert!(symbol.in_file(lib));
        assert!(!symbol.in_file(other));

        // Nothing selected is in no file, so a close never has to special-case it.
    }

    #[test]
    fn saves_and_resolves_a_symbol() {
        let objects = objects();
        let selection = Selection::Symbol(Symbol {
            object: objects[1].clone(),
            data: objects[1].symbols_sorted[0].clone(),
        });

        let session = from_state(&objects, Some(&selection), &History::default());
        assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
        assert_eq!(
            session.active,
            Some(SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "b.o".into(),
                symbol_name: "caller".into(),
                address: 0,
            })
        );

        // The duplicate `caller` in `a.o` must not win.
        assert!(resolve_selection(&session, &objects) == Some(selection));
    }

    #[test]
    fn saves_and_resolves_an_object() {
        let objects = objects();
        let selection = Selection::Object(objects[0].clone());
        let session = from_state(&objects, Some(&selection), &History::default());
        assert!(resolve_selection(&session, &objects) == Some(selection));
    }

    #[test]
    fn no_selection_round_trips_as_none() {
        let objects = objects();
        let session = from_state(&objects, None, &History::default());
        assert_eq!(session.active, None);
        assert!(resolve_selection(&session, &objects).is_none());
    }

    #[test]
    fn a_missing_symbol_falls_back_to_its_object() {
        let objects = objects();
        let session = Session {
            active: Some(SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
                symbol_name: "gone".into(),
                address: 12,
            }),
            history: SavedHistory::default(),
            ..Session::new()
        };
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
    }

    #[test]
    fn a_moved_symbol_falls_back_to_its_object() {
        let objects = objects();
        let session = Session {
            active: Some(SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
                // Right name, recompiled to a different address.
                symbol_name: "target".into(),
                address: 999,
            }),
            history: SavedHistory::default(),
            ..Session::new()
        };
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
    }

    #[test]
    fn a_missing_object_falls_back_to_nothing() {
        let objects = objects();
        for saved in [
            SavedDocument::Object {
                path: PathBuf::from("/tmp/other.a"),
                object_name: "a.o".into(),
            },
            // Right path, but that member is no longer in the archive.
            SavedDocument::Object {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "c.o".into(),
            },
            SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "c.o".into(),
                symbol_name: "caller".into(),
                address: 0,
            },
        ] {
            let session = Session {
                active: Some(saved),
                history: SavedHistory::default(),
                ..Session::new()
            };
            assert!(resolve_selection(&session, &objects).is_none());
        }
    }

    /// Serialize to TOML and read it straight back, which is the only way to catch the
    /// `toml` crate's runtime failures: a bare `None`, and a value emitted after a table.
    /// Generic over the two files, because the trap is a property of the serializer and
    /// both halves are equally subject to it.
    fn round_trip<T>(value: &T) -> String
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let text = toml::to_string_pretty(value).expect("serializing");
        let back: T = toml::from_str(&text).unwrap_or_else(|error| {
            panic!("deserializing\n--- {text}--- failed: {error}");
        });
        assert_eq!(*value, back);
        text
    }

    #[test]
    fn toml_round_trips() {
        let session = Session {
            active: Some(SavedDocument::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "b.o".into(),
                symbol_name: "caller".into(),
                address: 0x1234,
            }),
            history: SavedHistory::default(),
            ..Session::new()
        };
        let text = round_trip(&session);
        // The externally tagged enum is a table named after its variant.
        assert!(text.contains("[active.Symbol]"), "{text}");
    }

    #[test]
    fn an_empty_session_round_trips() {
        // Nothing selected and nothing visited: the `None` the `toml` crate cannot write
        // has to be left out of the file entirely, and read back as `None`.
        let session = Session::new();
        let text = round_trip(&session);
        assert!(!text.contains("active"), "{text}");
    }

    #[test]
    fn a_session_with_no_selection_round_trips() {
        let objects = objects();
        let session = from_state(&objects, None, &History::default());
        assert_eq!(session.active, None);
        let text = round_trip(&session);
        assert!(!text.contains("active"), "{text}");
    }

    #[test]
    fn a_multi_entry_history_round_trips_as_an_array_of_tables() {
        let objects = objects();
        let session = from_state(&objects, None, &history(&objects, 1));
        assert_eq!(session.history.entries.len(), 3);
        let text = round_trip(&session);
        assert!(text.contains("[[history.entries]]"), "{text}");
    }

    #[test]
    fn writes_atomically_and_reads_back() {
        let directory = std::env::temp_dir().join(format!(
            "assembly-viewer-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = directory.join("nested").join(SESSION_FILE);

        let session = Session {
            active: Some(SavedDocument::Object {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
            }),
            history: SavedHistory::default(),
            ..Session::new()
        };
        session.save_to(&path).expect("saving");

        assert_eq!(Session::load_from(&path), Some(session));
        // The temporary was renamed, not left behind.
        assert!(!path.with_extension("toml.tmp").exists());

        let _ = fs::remove_dir_all(&directory);
    }

    /// A history over the fixture objects: object `a.o`, then its `target` symbol, then
    /// object `b.o`, with the cursor wherever `back` calls leave it.
    fn history(objects: &[Arc<Object>], back: usize) -> History {
        let mut history = History::default();
        history.push(Document::Assembly(Selection::Object(objects[0].clone())));
        history.push(Document::Assembly(Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        })));
        history.push(Document::Assembly(Selection::Object(objects[1].clone())));
        for _ in 0..back {
            history.back();
        }
        history
    }

    #[test]
    fn saves_and_restores_the_history() {
        let objects = objects();
        let history = history(&objects, 0);

        let session = from_state(&objects, None, &history);
        assert_eq!(session.history.entries.len(), 3);
        assert_eq!(session.history.cursor, 2);

        let restored = session.resolve_history(&objects);
        assert!(restored.entries() == history.entries());
        assert_eq!(restored.cursor(), Some(2));
        assert!(restored.can_back());
        assert!(!restored.can_forward());
    }

    #[test]
    fn a_history_the_user_walked_back_through_keeps_its_cursor() {
        let objects = objects();
        let history = history(&objects, 2);
        assert_eq!(history.cursor(), Some(0));

        let session = from_state(&objects, None, &history);
        let restored = session.resolve_history(&objects);

        assert_eq!(restored.cursor(), Some(0));
        // The two entries in front of the cursor survived, so they are still there to
        // go forward to.
        assert!(restored.can_forward());
        assert!(!restored.can_back());
    }

    /// Building a saved history by hand, since these are entries no live `History`
    /// could have produced against these objects.
    fn saved_history(entries: &[SavedDocument], cursor: usize) -> Session {
        Session {
            active: None,
            history: SavedHistory {
                entries: entries.to_vec(),
                cursor,
            },
            ..Session::new()
        }
    }

    fn saved_object(name: &str) -> SavedDocument {
        SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: name.to_owned(),
        }
    }

    #[test]
    fn history_entries_that_no_longer_resolve_are_dropped() {
        let objects = objects();
        let session = saved_history(
            &[
                saved_object("a.o"),
                // A member that is no longer in the archive.
                saved_object("c.o"),
                // The object is there but the symbol is gone. Unlike the selection,
                // which would degrade to the object, an entry is dropped: the user
                // never visited the object, and a list of places they did not go is
                // worse than a shorter list.
                SavedDocument::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: "gone".into(),
                    address: 12,
                },
                saved_object("b.o"),
            ],
            3,
        );

        let restored = session.resolve_history(&objects);
        assert!(restored.entries() == [tab(&objects[0]), tab(&objects[1]),]);
        // The cursor was on the last entry, which survived: it must still be on it,
        // at its new index, and not on the entry that moved into its old one.
        assert_eq!(restored.cursor(), Some(1));
        assert!(!restored.can_forward());
    }

    #[test]
    fn a_saved_history_with_duplicates_restores_without_them() {
        let objects = objects();
        // What a saved history written before entries were bumped rather than appended
        // looks like: the same destination visited twice, saved twice.
        let session = saved_history(
            &[
                saved_object("a.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            2,
        );

        let restored = session.resolve_history(&objects);
        assert!(restored.entries() == [tab(&objects[1]), tab(&objects[0]),]);
        // The cursor was on the newest `a.o`, which is where the collapse left it.
        assert_eq!(restored.cursor(), Some(1));
        assert!(!restored.can_forward());
    }

    #[test]
    fn duplicates_collapse_around_the_entries_that_were_dropped() {
        let objects = objects();
        let session = saved_history(
            &[
                saved_object("a.o"),
                // Gone, so it is dropped before anything is collapsed.
                saved_object("c.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            3,
        );

        let restored = session.resolve_history(&objects);
        assert!(restored.entries() == [tab(&objects[1]), tab(&objects[0]),]);
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn the_restored_cursor_follows_its_entry_through_the_collapse() {
        let objects = objects();
        // The cursor is on `b.o`, in the middle, and the collapse of the two `a.o`s
        // moves it to the front of the list.
        let session = saved_history(
            &[
                saved_object("a.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            1,
        );

        let restored = session.resolve_history(&objects);
        assert!(restored.current() == Some(&tab(&objects[1])));
        assert_eq!(restored.cursor(), Some(0));
        // The newest `a.o` is still in front of it to go forward to.
        assert!(restored.can_forward());
        assert!(!restored.can_back());
    }

    #[test]
    fn two_saved_symbols_naming_the_same_one_restore_as_one_entry() {
        let objects = objects();
        let symbol = || SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            symbol_name: "target".into(),
            address: 6,
        };
        let session = saved_history(&[symbol(), saved_object("b.o"), symbol()], 2);

        // Both resolve through the same lookup to the same `Arc`, so they are equal
        // entries however far apart they were saved.
        let restored = session.resolve_history(&objects);
        assert!(
            restored.entries()
                == [
                    tab(&objects[1]),
                    Document::Assembly(Selection::Symbol(Symbol {
                        object: objects[0].clone(),
                        data: objects[0].symbols_sorted[1].clone(),
                    })),
                ]
        );
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn a_collapsed_cursor_entry_is_still_the_restored_selection() {
        let objects = objects();
        // Every cursor position over a saved history that holds a duplicate.
        for cursor in 0..3 {
            let mut session = saved_history(
                &[
                    saved_object("a.o"),
                    saved_object("b.o"),
                    saved_object("a.o"),
                ],
                cursor,
            );
            session.active = Some(session.history.entries[cursor].clone());

            let restored_history = session.resolve_history(&objects);
            let restored_active = session.resolve(&objects);

            assert!(restored_history.current() == restored_active.as_ref());
            assert!(!restored_history
                .would_push(restored_active.as_ref().expect("a restored document")));
        }
    }

    #[test]
    fn a_dropped_cursor_entry_falls_back_to_the_nearest_older_survivor() {
        let objects = objects();
        let session = saved_history(
            &[
                saved_object("a.o"),
                saved_object("c.o"),
                saved_object("b.o"),
            ],
            1,
        );

        let restored = session.resolve_history(&objects);
        assert!(restored.cursor() == Some(0));
        assert!(restored.current() == Some(&tab(&objects[0])));
        // `b.o` was in front of the cursor and still is.
        assert!(restored.can_forward());
    }

    #[test]
    fn a_cursor_with_no_older_survivor_lands_on_the_oldest_entry_left() {
        let objects = objects();
        let session = saved_history(&[saved_object("c.o"), saved_object("b.o")], 0);

        let restored = session.resolve_history(&objects);
        assert!(restored.cursor() == Some(0));
        assert!(restored.current() == Some(&tab(&objects[1])));
    }

    #[test]
    fn a_history_that_resolves_to_nothing_restores_as_empty() {
        let objects = objects();
        let session = saved_history(&[saved_object("c.o"), saved_object("d.o")], 1);

        let restored = session.resolve_history(&objects);
        assert!(restored.entries().is_empty());
        assert!(restored.cursor().is_none());
        assert!(!restored.can_back());
        assert!(!restored.can_forward());
    }

    #[test]
    fn a_hand_edited_cursor_past_the_end_is_clamped() {
        let objects = objects();
        let session = saved_history(&[saved_object("a.o"), saved_object("b.o")], 99);

        let restored = session.resolve_history(&objects);
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn the_restored_cursor_entry_is_the_restored_selection() {
        let objects = objects();

        // Every position the cursor can be in, including one the user walked back to.
        for back in 0..3 {
            let history = history(&objects, back);
            let current = history.current().expect("a current entry").clone();
            let selection = current.selection().expect("a place in a binary").clone();
            let session = from_state(&objects, Some(&selection), &history);

            let restored_history = session.resolve_history(&objects);
            let restored_active = session.resolve(&objects);

            assert!(restored_history.current() == restored_active.as_ref());
            // Which is what keeps the recording effect from pushing a duplicate the
            // moment the restore sets the active document.
            assert!(!restored_history
                .would_push(restored_active.as_ref().expect("a restored document")));
        }
    }

    #[test]
    fn a_file_with_no_history_still_loads() {
        // Hand-written, or trimmed: `serde(default)` is what keeps the missing table
        // from taking the binaries and the selection down with it.
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
        let session: Session = toml::from_str(text).expect("deserializing");

        assert_eq!(session.history, SavedHistory::new());

        // And it restores exactly as it would have: the selection back, no history.
        let objects = objects();
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
        assert!(session.resolve_history(&objects).entries().is_empty());
    }

    #[test]
    fn a_history_with_no_entries_still_loads() {
        let text = r#"
            binaries = []

            [history]
            cursor = 0
        "#;
        let session: Session = toml::from_str(text).expect("deserializing");
        assert_eq!(session, Session::new());
    }

    /// A path TOML cannot spell is refused rather than mangled, in *both* files: the
    /// binaries are the project half and the digests are keyed by the same paths, so the
    /// one refusal has to hold on either side of the split.
    #[test]
    fn a_non_utf8_path_is_not_written_rather_than_mangled() {
        // Only Unix has a `PathBuf` that can hold one at all.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;

            let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe.a"));
            let project = Project {
                binaries: vec![path.clone()],
                ..Project::default()
            };
            let session = Session {
                digests: BTreeMap::from([(path, digest_of(b"whatever"))]),
                ..Session::new()
            };
            // An error, not a panic and not a lossy path silently written in its place.
            assert!(toml::to_string_pretty(&project).is_err());
            assert!(toml::to_string_pretty(&session).is_err());

            let directory = std::env::temp_dir().join(format!(
                "assembly-viewer-test-{}-{}",
                std::process::id(),
                line!()
            ));
            assert!(project.save_to(&directory.join(PROJECT_FILE)).is_err());
            assert!(session.save_to(&directory.join(SESSION_FILE)).is_err());
            // Nothing reached the disk, so a good earlier file would still be there.
            assert!(!directory.join(PROJECT_FILE).exists());
            assert!(!directory.join(SESSION_FILE).exists());

            let _ = fs::remove_dir_all(&directory);
        }
    }

    #[test]
    fn the_history_round_trips_through_toml() {
        let objects = objects();
        let session = from_state(&objects, None, &history(&objects, 1));
        round_trip(&session);
    }

    // --- the open tabs ------------------------------------------------------

    /// Where the panes were left, in the map the UI keeps it in.
    fn positions<T: Clone + PartialEq>(at: &[(&T, usize)]) -> Positions<T> {
        let mut positions = Positions::default();
        for (tab, row) in at {
            positions.remember((*tab).clone(), *row);
        }
        positions
    }

    /// An assembly-driven tab over a whole object.
    fn tab(object: &Arc<Object>) -> Document {
        Document::Assembly(Selection::Object(object.clone()))
    }

    /// A source-driven tab, in the `Arc<str>` the UI holds a file in.
    fn file_tab(path: &str) -> Document {
        Document::Source(Arc::from(path))
    }

    fn saved_tab(object_name: &str, asm_row: usize) -> SavedTab {
        SavedTab {
            asm_row,
            src_row: 0,
            document: saved_object(object_name),
        }
    }

    fn saved_file_tab(path: &str, asm_row: usize, src_row: usize) -> SavedTab {
        SavedTab {
            asm_row,
            src_row,
            document: SavedDocument::Source {
                path: path.to_owned(),
            },
        }
    }

    /// One strip of both kinds goes out in the reader's own order and comes back in it,
    /// through the very mapping the history already uses — which is the whole reason a
    /// saved tab costs no new one.
    #[test]
    fn saves_and_resolves_the_open_tabs() {
        let objects = objects();
        let tabs = vec![
            tab(&objects[0]),
            file_tab("/src/main.rs"),
            Document::Assembly(Selection::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            })),
            tab(&objects[1]),
        ];

        let session = Session::from_state(
            &objects,
            &tabs,
            &Positions::default(),
            &Positions::default(),
            Some(&tabs[3]),
            &History::default(),
        );

        assert_eq!(
            session.tabs,
            [
                saved_tab("a.o", 0),
                saved_file_tab("/src/main.rs", 0, 0),
                SavedTab {
                    asm_row: 0,
                    src_row: 0,
                    document: SavedDocument::Symbol {
                        path: PathBuf::from("/tmp/lib.a"),
                        object_name: "a.o".into(),
                        symbol_name: "target".into(),
                        address: 6,
                    },
                },
                saved_tab("b.o", 0),
            ]
        );
        assert!(
            session.resolve_tabs(&objects)
                == [
                    (tabs[0].clone(), 0, 0),
                    (tabs[1].clone(), 0, 0),
                    (tabs[2].clone(), 0, 0),
                    (tabs[3].clone(), 0, 0),
                ]
        );
    }

    /// The active tab is not written twice: it is `active`, and a saved session says
    /// which tab is on screen only by naming it there.
    #[test]
    fn the_active_tab_is_only_the_active_document() {
        let objects = objects();
        let tabs = [tab(&objects[0])];
        let session = Session::from_state(
            &objects,
            &tabs,
            &Positions::default(),
            &Positions::default(),
            Some(&tabs[0]),
            &History::default(),
        );

        assert_eq!(session.active, Some(saved_object("a.o")));
        assert_eq!(session.tabs, [saved_tab("a.o", 0)]);
    }

    /// A tab is dropped exactly where the active document would degrade. There is one
    /// active document and the app has to open somewhere, but a strip whose tabs lead to
    /// places that are no longer there is worse than a shorter strip — and degrading
    /// would be worse still, since two symbols of one object would degrade onto the
    /// same tab and `Tabs::open` would collapse them into one.
    ///
    /// A **source-driven tab is never dropped**: it resolves against nothing, so there is
    /// nothing for it to fail against, and a file that has been deleted since is the
    /// pane's own "Source file not found" rather than a tab that quietly went away.
    ///
    /// The rows go with the tabs they belong to, which is the whole reason they are
    /// fields of one: the second and third tabs here are dropped, and a parallel array of
    /// rows would have handed `b.o` the rows of a tab that vanished before it.
    #[test]
    fn open_tabs_that_no_longer_resolve_are_dropped() {
        let objects = objects();
        let session = Session {
            tabs: vec![
                saved_tab("a.o", 3),
                // A member that is no longer in the archive.
                saved_tab("c.o", 4),
                // The object is still there; the symbol is not. The active document
                // would fall back to `a.o` here, and a tab must not.
                SavedTab {
                    asm_row: 5,
                    src_row: 0,
                    document: SavedDocument::Symbol {
                        path: PathBuf::from("/tmp/lib.a"),
                        object_name: "a.o".into(),
                        symbol_name: "gone".into(),
                        address: 12,
                    },
                },
                saved_file_tab("/no/such/file.rs", 0, 9),
                saved_tab("b.o", 6),
            ],
            ..Session::new()
        };

        assert!(
            session.resolve_tabs(&objects)
                == [
                    (tab(&objects[0]), 3, 0),
                    (file_tab("/no/such/file.rs"), 0, 9),
                    (tab(&objects[1]), 6, 0),
                ]
        );
    }

    /// Where each *side* of each tab was left goes out with the tab it belongs to, and a
    /// side that was never scrolled is written as the top rather than left out.
    ///
    /// Two rows and not one, because a tab has two sides: keying the source position by
    /// the file — which is what the Source pane's own strip did — made two functions
    /// compiled from one file share a position they have no reason to share.
    #[test]
    fn saves_the_rows_each_side_of_a_tab_was_left_at() {
        let objects = objects();
        let tabs = vec![tab(&objects[0]), tab(&objects[1])];

        let session = Session::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[1], 42)]),
            &positions(&[(&tabs[0], 7)]),
            Some(&tabs[0]),
            &History::default(),
        );

        assert_eq!(
            session.tabs,
            [
                SavedTab {
                    asm_row: 0,
                    src_row: 7,
                    document: saved_object("a.o"),
                },
                saved_tab("b.o", 42),
            ]
        );
    }

    /// The round trip the app actually makes: out of the two maps, through TOML, and back
    /// into them the way `use_restore_on_startup` does it.
    #[test]
    fn the_rows_come_back_against_the_tabs_they_belong_to() {
        let objects = objects();
        let tabs = vec![tab(&objects[0]), tab(&objects[1])];

        let session = Session::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[0], 12), (&tabs[1], 900)]),
            &positions(&[(&tabs[1], 4)]),
            Some(&tabs[0]),
            &History::default(),
        );
        let session: Session = toml::from_str(&round_trip(&session)).expect("reading back");

        let (mut asm, mut src): (Positions<Document>, Positions<Document>) =
            (Positions::default(), Positions::default());
        for (tab, asm_row, src_row) in session.resolve_tabs(&objects) {
            asm.remember(tab.clone(), asm_row);
            src.remember(tab, src_row);
        }
        assert_eq!(asm.at(&tabs[0]), Some(12));
        assert_eq!(asm.at(&tabs[1]), Some(900));
        assert_eq!(src.at(&tabs[1]), Some(4));
        // And a hint it is: a listing that has since shrunk clamps to what it holds now.
        assert_eq!(asm.row(&tabs[1], 100), 99);
    }

    /// A row is a hint and not a fact, so a saved tab that does not name one is a tab at
    /// the top rather than a file that will not load.
    #[test]
    fn a_saved_tab_with_no_rows_opens_at_the_top() {
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [[tabs]]
            [tabs.document.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"

            [[tabs]]
            [tabs.document.Source]
            path = "/src/main.rs"
        "#;
        let session: Session = toml::from_str(text).expect("deserializing");

        let objects = objects();
        assert!(
            session.resolve_tabs(&objects)
                == [(tab(&objects[0]), 0, 0), (file_tab("/src/main.rs"), 0, 0),]
        );
    }

    /// Nothing about a source file is resolved against this filesystem, on purpose:
    /// the pane's own "Source file not found" is the right answer for one that has been
    /// deleted, and dropping the tab would lose a file the reader had open without ever
    /// saying so.
    #[test]
    fn a_source_file_that_is_no_longer_there_still_comes_back() {
        let path = "/no/such/directory/gone.rs";
        assert!(!Path::new(path).exists());

        let objects = objects();
        let tabs = [file_tab(path)];
        let session = Session::from_state(
            &objects,
            &tabs,
            &Positions::default(),
            &Positions::default(),
            Some(&tabs[0]),
            &History::default(),
        );

        assert_eq!(session.tabs, [saved_file_tab(path, 0, 0)]);
        assert_eq!(
            session.active,
            Some(SavedDocument::Source { path: path.into() })
        );
        assert!(session.resolve(&objects) == Some(file_tab(path)));
    }

    /// The field-order trap, which only a real serialization catches: a saved tab's two
    /// rows are plain values and have to reach the file before the `document` sub-table
    /// under them. A session with every field set at once is the one that fails when they
    /// do not.
    #[test]
    fn a_full_session_round_trips_through_toml() {
        let objects = objects();
        let tabs = vec![
            tab(&objects[0]),
            Document::Assembly(Selection::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            })),
            file_tab("/src/main.rs"),
        ];
        let session = Session::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[0], 12), (&tabs[1], 34)]),
            &positions(&[(&tabs[0], 56)]),
            Some(&tabs[1]),
            &history(&objects, 1),
        );

        let text = round_trip(&session);
        assert!(text.contains("[[tabs]]"), "{text}");
        assert!(text.contains("[tabs.document.Source]"), "{text}");

        // Inside a tab, the rows before the table its document is written as.
        let asm_row = text.find("asm_row = 12").expect("the first tab's row");
        let src_row = text
            .find("src_row = 56")
            .expect("the first tab's source row");
        let document = text
            .find("[tabs.document")
            .expect("the first tab's document");
        assert!(asm_row < document, "asm_row after its document\n{text}");
        assert!(src_row < document, "src_row after its document\n{text}");
    }

    #[test]
    fn a_file_with_no_tabs_still_loads() {
        // The `serde(default)`s, from the other side: a hand-written or trimmed file is
        // a session with an empty strip rather than a load failure.
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
        let session: Session = toml::from_str(text).expect("deserializing");

        assert!(session.tabs.is_empty());

        // And the active document still restores, opening its own tab through `activate`
        // the way a session saved before there were tabs to save would.
        let objects = objects();
        assert!(session.resolve(&objects) == Some(tab(&objects[0])));
        assert!(session.resolve_tabs(&objects).is_empty());
    }

    // --- the binaries' digests ---------------------------------------------

    /// The digest of the objects the fixtures are built from, as it is written down.
    fn digest_of(bytes: &[u8]) -> String {
        analysis::FileDigest::of(bytes).to_string()
    }

    /// A saved session naming one binary at `row`, with the digest of `bytes` — what a
    /// run that had this file open would have written.
    fn saved_against(bytes: Option<&[u8]>, saved: SavedDocument, row: usize) -> Session {
        Session {
            digests: bytes
                .map(|bytes| BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(bytes))]))
                .unwrap_or_default(),
            active: Some(saved.clone()),
            tabs: vec![SavedTab {
                asm_row: row,
                src_row: 0,
                document: saved.clone(),
            }],
            history: SavedHistory {
                cursor: 0,
                entries: vec![saved],
            },
            ..Session::new()
        }
    }

    fn saved_symbol(object_name: &str, symbol_name: &str, address: u64) -> SavedDocument {
        SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: object_name.to_owned(),
            symbol_name: symbol_name.to_owned(),
            address,
        }
    }

    /// One digest per *file*, however many objects came out of it: the members of an
    /// archive share one `ObjectData` and so one hash, and the map is keyed by the path
    /// the rest of the session is expressed in.
    #[test]
    fn saves_one_digest_per_binary_however_many_objects_it_holds() {
        let objects = objects();
        let session = from_state(&objects, None, &History::default());

        assert_eq!(binaries(&objects), vec![PathBuf::from("/tmp/lib.a")]);
        assert_eq!(
            session.digests,
            BTreeMap::from([(PathBuf::from("/tmp/lib.a"), digest_of(b"the first build"))])
        );
    }

    /// The file is the one the session was saved against, so the saved address is a fact
    /// about it: an exact match resolves, a symbol that is not where it was said to be
    /// does not, and the row the tab was left at is still that tab's row. All of which is
    /// what the app did before there were digests — an unchanged file changes nothing.
    #[test]
    fn an_unchanged_binary_is_still_matched_on_the_address() {
        let objects = objects();

        let session = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "target", 6),
            42,
        );
        assert!(
            resolve_selection(&session, &objects)
                == Some(Selection::Symbol(Symbol {
                    object: objects[0].clone(),
                    data: objects[0].symbols_sorted[1].clone(),
                }))
        );
        assert_eq!(session.resolve_tabs(&objects).len(), 1);
        assert_eq!(session.resolve_tabs(&objects)[0].1, 42);

        // The same name at an address it is not at. Nothing about this file explains
        // that, so it degrades exactly as it always has.
        let moved = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "target", 999),
            42,
        );
        assert!(resolve_selection(&moved, &objects) == Some(Selection::Object(objects[0].clone())));
        assert!(moved.resolve_tabs(&objects).is_empty());
        assert!(moved.resolve_history(&objects).entries().is_empty());
    }

    /// The file has been rebuilt under the session. The name is what the reader meant, so
    /// a symbol that has merely moved comes back — where an unchanged file would have
    /// dropped it — and the saved row goes, because it named a row of a listing this
    /// build no longer has.
    #[test]
    fn a_rebuilt_binary_matches_by_name_and_forgets_the_row() {
        let objects = vec![
            built(
                "/tmp/lib.a",
                "a.o",
                &[("caller", 0), ("target", 96)],
                b"the second build",
            ),
            built("/tmp/lib.a", "b.o", &[("caller", 0)], b"the second build"),
        ];

        // Saved when `target` was at 6; it is at 96 now.
        let session = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "target", 6),
            42,
        );

        let expected = Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        });
        assert!(resolve_selection(&session, &objects) == Some(expected.clone()));
        let document = Document::Assembly(expected.clone());
        assert!(session.resolve_tabs(&objects) == [(document.clone(), 0, 0)]);
        assert!(session.resolve_history(&objects).entries() == [document]);
    }

    /// The half that is a refusal rather than a recovery: two symbols of one name in a
    /// rebuilt object, and a saved address that is now neither of theirs. The address is
    /// the only thing that could choose between them and it describes a layout this file
    /// no longer has, so nothing is chosen — landing the reader on a function they never
    /// opened is the failure this whole step exists to stop.
    #[test]
    fn a_rebuilt_binary_will_not_guess_between_two_symbols_of_one_name() {
        let objects = vec![built(
            "/tmp/lib.a",
            "a.o",
            &[("helper", 32), ("helper", 64)],
            b"the second build",
        )];

        let session = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "helper", 6),
            42,
        );
        // The selection degrades to the object; the tab and the history entry drop.
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
        assert!(session.resolve_tabs(&objects).is_empty());
        assert!(session.resolve_history(&objects).entries().is_empty());

        // And where the address still names one of them, it is still the tie-breaker.
        let exact = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "helper", 64),
            42,
        );
        assert!(
            resolve_selection(&exact, &objects)
                == Some(Selection::Symbol(Symbol {
                    object: objects[0].clone(),
                    data: objects[0].symbols_sorted[1].clone(),
                }))
        );
    }

    /// A session that never wrote a digest — a hand-edited file, or one saved before
    /// there were any — says nothing about the bytes, so nothing new is done with it.
    /// "Not known to be unchanged" is not "known to have changed".
    #[test]
    fn a_binary_with_no_saved_digest_is_believed_exactly_as_before() {
        let objects = vec![built(
            "/tmp/lib.a",
            "a.o",
            &[("caller", 0), ("target", 96)],
            b"the second build",
        )];

        let session = saved_against(None, saved_symbol("a.o", "target", 6), 42);
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
        assert!(session.resolve_tabs(&objects).is_empty());
    }

    /// A digest for a path that is not loaded, and a loaded path with no digest, are both
    /// "nothing to compare" rather than a mismatch.
    #[test]
    fn a_digest_for_a_binary_that_is_not_open_says_nothing() {
        let objects = objects();
        let mut session = saved_against(
            Some(b"the first build"),
            saved_symbol("a.o", "target", 6),
            42,
        );
        session
            .digests
            .insert(PathBuf::from("/tmp/some.dll"), digest_of(b"whatever"));

        assert_eq!(session.resolve_tabs(&objects)[0].1, 42);
    }

    /// The digests are a TOML *table*, and a hex string is what it holds, since a `u64`
    /// digest does not fit TOML's signed integers at all.
    #[test]
    fn the_digests_round_trip_through_toml() {
        let objects = objects();
        let tabs = vec![tab(&objects[0]), file_tab("/src/main.rs")];
        let session = Session::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[0], 12)]),
            &Positions::default(),
            Some(&tabs[0]),
            &history(&objects, 1),
        );

        let text = round_trip(&session);
        assert!(text.contains("[digests]"), "{text}");
        assert!(
            text.contains(&format!("{}\"", digest_of(b"the first build"))),
            "{text}"
        );
    }

    /// And a file with no digests at all still loads, the way one with no tabs does.
    #[test]
    fn a_file_with_no_digests_still_loads() {
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [active.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
        let session: Session = toml::from_str(text).expect("deserializing");
        assert!(session.digests.is_empty());

        let objects = objects();
        assert!(
            resolve_selection(&session, &objects) == Some(Selection::Object(objects[0].clone()))
        );
    }

    // --- the save policy ---------------------------------------------------

    fn paths(binaries: &[&str]) -> Vec<PathBuf> {
        binaries.iter().map(PathBuf::from).collect()
    }

    fn session_with(selection: Option<&str>) -> Session {
        Session {
            active: selection.map(saved_object),
            history: SavedHistory::new(),
            ..Session::new()
        }
    }

    /// What `record` hands back to be written: the project half, the session half where
    /// one is owed, or nothing at all.
    ///
    /// The details handed in are the ones the project already has, so every test below
    /// that uses this is asking about a change to the binaries or the session and nothing
    /// else — which is what they were all asking before a rename could reach `record` at
    /// all. The rename tests spell theirs out.
    fn recorded(
        saves: &mut Saves,
        binaries: Vec<PathBuf>,
        session: Session,
    ) -> Option<(Project, Option<Session>)> {
        let unchanged = saves.given.clone();
        saves
            .record(unchanged, binaries, session)
            .map(|written| (written.project, written.session))
    }

    fn written(
        saves: &mut Saves,
        binaries: &[&str],
        selection: Option<&str>,
    ) -> Option<(Project, Option<Session>)> {
        recorded(saves, paths(binaries), session_with(selection))
    }

    #[test]
    fn the_state_the_app_boots_into_is_never_written() {
        let mut saves = Saves::new();
        // The save observer runs once on mount, before anything is restored, and this
        // is what it records. Nothing may come of it: the files on disk are the good
        // ones — and no project directory is allocated either, since only a write
        // allocates one.
        assert_eq!(recorded(&mut saves, Vec::new(), Session::new()), None);
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn opening_a_binary_is_written_at_once() {
        let mut saves = Saves::new();

        let written = written(&mut saves, &["/tmp/lib.a"], None);
        assert_eq!(
            written,
            Some((
                Project {
                    name: None,
                    directory: None,
                    binaries: paths(&["/tmp/lib.a"]),
                },
                Some(session_with(None)),
            ))
        );
        // And is not written a second time by the next flush.
        assert_eq!(saves.flush(), None);
    }

    /// Closing one takes the same path opening one does, which is the whole of what
    /// makes 6d's "the save is immediate" true: `binaries` is what `record` looks at,
    /// and it does not care in which direction the list changed.
    #[test]
    fn closing_a_binary_is_written_at_once() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));

        // The selection is still pending from the open above; closing writes the lot,
        // so `session.toml` never names a place inside a binary `project.toml` has
        // already let go of.
        let written = written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
        assert_eq!(
            written.as_ref().map(|(project, _)| &project.binaries),
            Some(&paths(&["/tmp/lib.a"]))
        );
        assert_eq!(
            written.and_then(|(_, session)| session),
            Some(session_with(Some("a.o")))
        );
        assert_eq!(saves.flush(), None);
    }

    /// Closing the last one is not "nothing changed": the empty project is a project,
    /// and it has to reach the disk or the next run reopens what was just closed.
    #[test]
    fn closing_the_only_binary_is_written_too() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

        let written = recorded(&mut saves, Vec::new(), Session::new());
        assert_eq!(written, Some((Project::default(), Some(Session::new()))));
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn a_selection_change_waits_for_the_flush() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], None);

        assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
        assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn recording_the_same_project_again_changes_nothing() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], None);

        // A pending change re-recorded unchanged, as the save observer does whenever
        // something it does not persist wakes it.
        written(&mut saves, &["/tmp/lib.a"], Some("a.o"));
        assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
        // Still pending, and still exactly one write.
        assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));
        assert_eq!(saves.flush(), None);

        // And once written, re-recording it is not a second write either.
        assert_eq!(written(&mut saves, &["/tmp/lib.a"], Some("a.o")), None);
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn opening_a_binary_carries_the_pending_change_with_it() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

        // The selection is pending; opening a second binary writes the lot.
        let written = written(&mut saves, &["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));
        assert_eq!(
            written.and_then(|(_, session)| session),
            Some(session_with(Some("a.o")))
        );
        assert_eq!(saves.flush(), None);
    }

    /// A tab is pending and not an immediate write, and nothing in `record` says so:
    /// which file a field lives in is what decides it, and a tab lives in the session.
    /// That is the answer wanted — `activate` opens a tab on the way to every selection
    /// change, so an immediate write here would be one file per click.
    #[test]
    fn opening_a_tab_waits_for_the_flush() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], None);

        let mut session = session_with(Some("a.o"));
        session.tabs = vec![saved_tab("a.o", 0)];
        assert_eq!(
            recorded(&mut saves, paths(&["/tmp/lib.a"]), session.clone()),
            None
        );
        assert_eq!(saves.flush(), Some(session));
        assert_eq!(saves.flush(), None);
    }

    /// And so is a source file, by the same route: the pane opens one whenever the
    /// selection lands on a symbol with line info.
    #[test]
    fn opening_a_source_file_waits_for_the_flush() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], None);

        let mut session = session_with(None);
        session.tabs = vec![
            saved_file_tab("/src/main.rs", 0, 0),
            saved_file_tab("/src/lib.rs", 0, 0),
        ];
        session.active = Some(SavedDocument::Source {
            path: "/src/lib.rs".into(),
        });
        assert_eq!(
            recorded(&mut saves, paths(&["/tmp/lib.a"]), session.clone()),
            None
        );
        assert_eq!(saves.flush(), Some(session));
        assert_eq!(saves.flush(), None);
    }

    /// Closing a binary still writes at once, and now carries the tabs it closed with
    /// it: they were pending, and `record` takes everything pending along with the
    /// binaries change, so the session file never names a tab into a binary the project
    /// file has already let go of.
    #[test]
    fn closing_a_binary_carries_the_tabs_it_closed_with_it() {
        let mut saves = Saves::new();
        let mut opened = session_with(Some("a.o"));
        opened.tabs = vec![saved_tab("a.o", 0)];
        recorded(&mut saves, paths(&["/tmp/lib.a", "/tmp/some.dll"]), opened);

        let closed = session_with(None);
        assert_eq!(
            recorded(&mut saves, paths(&["/tmp/lib.a"]), closed.clone()),
            Some((
                Project {
                    binaries: paths(&["/tmp/lib.a"]),
                    ..Project::default()
                },
                Some(closed)
            ))
        );
        assert_eq!(saves.flush(), None);
    }

    /// The name and the directory survive a record that is not about them: they are the
    /// baseline, so a record handing back the same ones is handing back "unchanged" and
    /// the write carries them rather than the absence a derived project would have.
    #[test]
    fn a_record_keeps_the_name_the_project_was_given() {
        let mut saves = Saves::new();
        let named = Project {
            name: Some("kernel".into()),
            directory: Some(PathBuf::from("/src/kernel")),
            binaries: paths(&["/tmp/vmlinux"]),
        };
        saves.opened(ProjectId::new("kernel-1").expect("an id"), &named);

        let (project, _) = written(&mut saves, &["/tmp/lib.a"], None).expect("a write");
        assert_eq!(project.name.as_deref(), Some("kernel"));
        assert_eq!(project.directory, Some(PathBuf::from("/src/kernel")));
        // And the binaries are the ones the app is showing, not the ones it was opened
        // with: that half *is* derived, on every record.
        assert_eq!(project.binaries, paths(&["/tmp/lib.a"]));
    }

    /// The other half of that: a reopen seeds the *name* and not the contents. Both are
    /// the same rule — a baseline is the state the app boots into — applied to two fields
    /// that are restored at different moments. The name is put on screen synchronously,
    /// so the baseline holds it; the binaries arrive when a worker thread has finished
    /// parsing them, so a baseline holding them would read the still-empty boot state as a
    /// change and write an empty project over a good one.
    #[test]
    fn reopening_seeds_the_name_but_not_the_baseline() {
        let mut saves = Saves::new();
        let loaded = Project {
            name: Some("kernel".into()),
            directory: None,
            binaries: paths(&["/tmp/vmlinux"]),
        };
        saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

        // The boot state, recorded by the observer's first run: it equals the baseline,
        // so nothing is written and the good files are left alone.
        assert_eq!(recorded(&mut saves, Vec::new(), Session::new()), None);
        // And the restore that follows is an ordinary change, written at once.
        let (project, _) =
            recorded(&mut saves, paths(&["/tmp/vmlinux"]), Session::new()).expect("a write");
        assert_eq!(project, loaded);
    }

    /// Naming a project is a user project change, so it is on disk before the next
    /// click — and it is a `project.toml` write and nothing else, since a rename lets go
    /// of no binary and so cannot leave the two files disagreeing.
    #[test]
    fn a_rename_is_written_at_once_and_leaves_the_session_pending() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], None);
        // A selection, pending as ever.
        written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

        let named = Details {
            name: Some("kernel".into()),
            directory: Some(PathBuf::from("/src/kernel")),
        };
        let written = saves
            .record(
                named.clone(),
                paths(&["/tmp/lib.a"]),
                session_with(Some("a.o")),
            )
            .expect("a write");
        assert_eq!(
            written.project,
            Project {
                name: named.name.clone(),
                directory: named.directory.clone(),
                binaries: paths(&["/tmp/lib.a"]),
            }
        );
        // The session was not owed and so was not written — and is still pending, which
        // is the half that says the rename did not quietly take it along.
        assert_eq!(written.session, None);
        assert_eq!(saves.flush(), Some(session_with(Some("a.o"))));

        // And the same name recorded again is not a second write: `given` is a baseline
        // like the binaries, so a re-render costs nothing.
        assert_eq!(
            saves.record(named, paths(&["/tmp/lib.a"]), session_with(Some("a.o"))),
            None
        );
    }

    /// Clearing a name is a change like any other, and writes the key away rather than
    /// leaving the old one on disk.
    #[test]
    fn clearing_a_name_is_a_change_too() {
        let mut saves = Saves::new();
        saves.opened(
            ProjectId::new("kernel-1").expect("an id"),
            &Project {
                name: Some("kernel".into()),
                ..Project::default()
            },
        );

        let written = saves
            .record(Details::new(), Vec::new(), Session::new())
            .expect("a write");
        assert_eq!(written.project.name, None);
        assert_eq!(written.session, None);
    }

    /// A rename while the binaries are still being parsed — or after a restore that
    /// opened none of them at all — writes back the list the file already holds.
    ///
    /// The app holds no binary in that window and deliberately writes nothing to say so,
    /// since a file that is only temporarily missing must not be forgotten. A rename is
    /// an immediate write all the same, and writing the app's own empty list would forget
    /// them through a change that had nothing to do with them.
    #[test]
    fn a_rename_before_the_binaries_have_loaded_does_not_forget_them() {
        let mut saves = Saves::new();
        let loaded = Project {
            name: None,
            directory: None,
            binaries: paths(&["/tmp/vmlinux", "/tmp/lib.a"]),
        };
        saves.opened(ProjectId::new("kernel-1").expect("an id"), &loaded);

        let named = Details {
            name: Some("kernel".into()),
            directory: None,
        };
        let written = saves
            .record(named, Vec::new(), Session::new())
            .expect("a write");
        assert_eq!(written.project.name.as_deref(), Some("kernel"));
        assert_eq!(written.project.binaries, loaded.binaries);

        // And once the parse lands, the binaries are the app's own again: that write *is*
        // about them, so it is the one kind that may replace the list.
        let written = saves
            .record(
                saves.given.clone(),
                paths(&["/tmp/vmlinux"]),
                Session::new(),
            )
            .expect("a write");
        assert_eq!(written.project.binaries, paths(&["/tmp/vmlinux"]));
        // Closing the last one is still a real change and still empties the file.
        let written = recorded(&mut saves, Vec::new(), Session::new()).expect("a write");
        assert_eq!(written.0.binaries, Vec::<PathBuf>::new());
    }

    /// Entering another project empties every baseline, because the app is about to be
    /// emptied of everything that belonged to the last one. A baseline still describing
    /// those binaries would read that emptying as a change and write it into the project
    /// just entered.
    #[test]
    fn entering_a_project_empties_every_baseline() {
        let mut saves = Saves::new();
        written(&mut saves, &["/tmp/lib.a"], Some("a.o"));

        let entered = Project {
            name: Some("other".into()),
            ..Project::default()
        };
        saves.opened(ProjectId::new("other-2").expect("an id"), &entered);

        // The state a switch leaves the app in: nothing open, nothing selected, and the
        // name of the project just entered. Every one of those is the baseline, so
        // nothing is written into the new project before its own restore has run.
        assert_eq!(
            saves.record(entered.details(), Vec::new(), Session::new()),
            None
        );
        // Nor is the old project's pending session waiting to be written into the new
        // one: `switch` flushed it, and entering dropped whatever was left.
        assert_eq!(saves.flush(), None);
    }

    // --- the two files, the ids and the recent list -------------------------

    /// A directory of this test's own under the system temporary directory, named after
    /// the line that asked for it, exactly as the file tests above are.
    fn directory(line: u32) -> PathBuf {
        std::env::temp_dir().join(format!(
            "assembly-viewer-project-test-{}-{line}",
            std::process::id()
        ))
    }

    fn a_project() -> Project {
        Project {
            name: Some("kernel".into()),
            directory: Some(PathBuf::from("/src/kernel")),
            binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        }
    }

    /// The field-order trap for the project half. Everything in it is a plain value
    /// today, which is exactly the kind of thing that stops being true when a field is
    /// added, so it is asserted against a real serializer rather than read off the struct.
    #[test]
    fn a_project_round_trips_through_toml() {
        let project = a_project();
        let text = round_trip(&project);
        assert!(!text.contains("\n["), "a table in the project file\n{text}");

        let name = text.find("name = ").expect("the name");
        let directory = text.find("directory = ").expect("the directory");
        let binaries = text.find("binaries = ").expect("the binaries");
        assert!(name < directory && directory < binaries, "{text}");
    }

    /// Anonymous is an *absent* key, the way an unspecified font is in `settings.rs`:
    /// it is what makes the project anonymous, so it must not be spelt as an empty name
    /// that a later reader could mistake for one the user chose.
    #[test]
    fn an_anonymous_project_writes_no_name() {
        let project = Project {
            binaries: paths(&["/tmp/lib.a"]),
            ..Project::default()
        };
        let text = round_trip(&project);
        assert!(!text.contains("name"), "{text}");
        assert!(!text.contains("directory"), "{text}");
    }

    /// The whole of the split, seen from the disk: each half in its own file, neither
    /// holding a word of the other's.
    #[test]
    fn the_two_halves_are_written_to_their_own_files() {
        let directory = directory(line!());
        let project = a_project();
        let session = Session {
            active: Some(saved_object("a.o")),
            tabs: vec![saved_tab("a.o", 7)],
            ..Session::new()
        };

        project
            .save_to(&directory.join(PROJECT_FILE))
            .expect("saving the project");
        session
            .save_to(&directory.join(SESSION_FILE))
            .expect("saving the session");

        let project_text = fs::read_to_string(directory.join(PROJECT_FILE)).expect("reading");
        let session_text = fs::read_to_string(directory.join(SESSION_FILE)).expect("reading");
        assert!(project_text.contains("/tmp/lib.a"), "{project_text}");
        assert!(!project_text.contains("active"), "{project_text}");
        assert!(session_text.contains("active"), "{session_text}");
        assert!(!session_text.contains("binaries"), "{session_text}");

        assert_eq!(
            Project::load_from(&directory.join(PROJECT_FILE)),
            Some(project)
        );
        assert_eq!(
            Session::load_from(&directory.join(SESSION_FILE)),
            Some(session)
        );

        let _ = fs::remove_dir_all(&directory);
    }

    /// The reason the split is worth two files rather than two tables: the half the app
    /// rewrites every thirty seconds cannot take the half the user gave down with it.
    #[test]
    fn a_corrupt_session_leaves_the_project_readable() {
        let directory = directory(line!());
        let project = a_project();
        project
            .save_to(&directory.join(PROJECT_FILE))
            .expect("saving the project");
        fs::write(directory.join(SESSION_FILE), b"{ not toml").expect("writing the corrupt half");

        assert_eq!(
            Project::load_from(&directory.join(PROJECT_FILE)),
            Some(project)
        );
        assert_eq!(Session::load_from(&directory.join(SESSION_FILE)), None);

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_or_corrupt_file_is_none() {
        let directory = directory(line!());
        let path = directory.join(SESSION_FILE);

        assert_eq!(Session::load_from(&path), None);
        assert_eq!(Project::load_from(&directory.join(PROJECT_FILE)), None);

        fs::create_dir_all(&directory).expect("creating the test directory");
        fs::write(&path, b"{ not toml").expect("writing the corrupt file");
        assert_eq!(Session::load_from(&path), None);

        let _ = fs::remove_dir_all(&directory);
    }

    /// An id is interpolated into a path, so what it may be is the whole of what keeps
    /// `recents.toml` from naming somewhere else on the disk.
    #[test]
    fn an_id_is_one_ordinary_path_component() {
        for good in ["project-1", "kernel_2", "a", "9lives"] {
            assert_eq!(
                ProjectId::new(good).map(|id| id.as_str().to_owned()),
                Some(good.to_owned())
            );
        }
        for bad in [
            "", "..", ".", "-leading", "a/b", "a\\b", "a b", "a.toml", "é",
        ] {
            assert_eq!(ProjectId::new(bad), None, "{bad}");
        }
        assert_eq!(ProjectId::new("x".repeat(MAX_ID + 1)), None);
    }

    /// Deserializing goes through the same check, so an id out of a hand-edited file
    /// cannot be a path — and a file holding one is a corrupt file, which is the default.
    #[test]
    fn a_hand_edited_recent_that_is_not_an_id_is_refused() {
        assert!(toml::from_str::<Recents>(r#"projects = ["../elsewhere"]"#).is_err());
        assert!(toml::from_str::<Recents>(r#"projects = ["project-1"]"#).is_ok());

        let directory = directory(line!());
        let path = directory.join(RECENTS_FILE);
        fs::create_dir_all(&directory).expect("creating the test directory");
        fs::write(&path, br#"projects = ["../elsewhere"]"#).expect("writing");
        assert_eq!(Recents::load_from(&path), Recents::default());

        let _ = fs::remove_dir_all(&directory);
    }

    fn id(text: &str) -> ProjectId {
        ProjectId::new(text).expect("an id")
    }

    /// The order *is* the answer to "which project was last open", so touching the one
    /// already at the front changes nothing — which is what keeps a startup that reopens
    /// it from writing a file to say so.
    #[test]
    fn touching_a_project_moves_it_to_the_front_once() {
        let mut recents = Recents::default();
        assert!(recents.touch(&id("a")));
        assert!(recents.touch(&id("b")));
        assert_eq!(recents.projects, vec![id("b"), id("a")]);
        assert_eq!(recents.first(), Some(&id("b")));

        // Already first: no change, and so no write.
        assert!(!recents.touch(&id("b")));
        // And one that is in the list is moved rather than repeated.
        assert!(recents.touch(&id("a")));
        assert_eq!(recents.projects, vec![id("a"), id("b")]);
    }

    /// Bounded, because this file is appended to for as long as the app is ever used.
    /// What falls off the end is a place in the order and never a project: every one of
    /// them is still a directory.
    #[test]
    fn the_recent_list_is_bounded() {
        let mut recents = Recents::default();
        for n in 0..MAX_RECENTS + 10 {
            recents.touch(&id(&format!("project-{n}")));
        }
        assert_eq!(recents.projects.len(), MAX_RECENTS);
        assert_eq!(
            recents.first(),
            Some(&id(&format!("project-{}", MAX_RECENTS + 9)))
        );
    }

    #[test]
    fn the_recent_list_round_trips_through_toml() {
        let mut recents = Recents::default();
        recents.touch(&id("project-1"));
        recents.touch(&id("kernel_2"));
        let text = round_trip(&recents);
        assert!(text.contains(r#""kernel_2""#), "{text}");

        // A missing or unreadable file is the empty list, never an error.
        assert_eq!(
            Recents::load_from(Path::new("/no/such/recents.toml")),
            Recents::default()
        );
    }

    /// The claim is the `create_dir`, so two allocations in the same directory cannot
    /// land on the same name however the numbers are counted — and the directory is what
    /// makes the id survive a restart.
    #[test]
    fn anonymous_projects_do_not_collide() {
        let directory = directory(line!());

        let first = ProjectId::anonymous(&directory).expect("an id");
        let second = ProjectId::anonymous(&directory).expect("a second id");
        assert_ne!(first, second);
        assert!(directory.join(first.as_str()).is_dir());
        assert!(directory.join(second.as_str()).is_dir());

        // A directory that is already there is stepped over rather than opened, whether
        // this app made it or not.
        fs::create_dir(directory.join(format!("{ANONYMOUS_STEM}-3"))).expect("a squatter");
        let third = ProjectId::anonymous(&directory).expect("a third id");
        assert_ne!(third.as_str(), format!("{ANONYMOUS_STEM}-3"));

        let _ = fs::remove_dir_all(&directory);
    }

    /// The whole of "a project appears when there is something to put in it": nothing on
    /// disk until the first write, then a directory, and the recent list pointing at it.
    #[test]
    fn the_first_write_creates_a_project_and_remembers_it() {
        let base = directory(line!());
        let mut saves = Saves::new();

        let id = open_project(&mut saves, &base).expect("a project");
        assert!(project_in(&base, &id).is_dir());
        assert_eq!(
            Recents::load_from(&recents_in(&base)).projects,
            vec![id.clone()]
        );

        // Every later write of the run goes into the same one rather than allocating
        // another, which is what makes the id the run's identity and not the write's.
        assert_eq!(open_project(&mut saves, &base), Some(id));

        let _ = fs::remove_dir_all(&base);
    }

    /// Startup: the front of the recent list, both halves of it, through the same
    /// defaulting every other read here uses.
    #[test]
    fn the_last_project_is_the_one_reopened() {
        let base = directory(line!());
        let project = a_project();
        let session = Session {
            active: Some(saved_object("a.o")),
            ..Session::new()
        };

        for id in ["other-1", "wanted-2"] {
            let id = self::id(id);
            project
                .save_to(&project_in(&base, &id).join(PROJECT_FILE))
                .expect("saving the project");
            session
                .save_to(&project_in(&base, &id).join(SESSION_FILE))
                .expect("saving the session");
            remember(&base, &id);
        }

        let (id, reopened, restored) = reopen_in(&base).expect("a project to reopen");
        assert_eq!(id, self::id("wanted-2"));
        assert_eq!(reopened, project);
        assert_eq!(restored, session);

        let _ = fs::remove_dir_all(&base);
    }

    /// Three ways for there to be nothing to reopen, all of them silence.
    #[test]
    fn nothing_to_reopen_is_not_an_error() {
        let base = directory(line!());
        // No recent list at all: a first run, or one whose file was deleted.
        assert!(reopen_in(&base).is_none());

        // A recent list naming a project whose directory has gone — deleted by hand, or
        // on another machine the state directory is synced from.
        remember(&base, &id("gone-1"));
        assert!(reopen_in(&base).is_none());

        let _ = fs::remove_dir_all(&base);
    }

    /// The directory *is* the project, so a run that was killed between creating one and
    /// writing either file into it reopens as the empty project it is — rather than being
    /// orphaned while a second one is allocated beside it. A corrupt session is the same
    /// answer for the same reason, and this is the split earning its keep: the half the
    /// app rewrites every thirty seconds cannot take the other half down with it.
    #[test]
    fn a_project_missing_a_half_still_reopens() {
        let base = directory(line!());
        let mut saves = Saves::new();
        let id = open_project(&mut saves, &base).expect("a project");

        // Neither file written yet.
        let (reopened, project, session) = reopen_in(&base).expect("a project to reopen");
        assert_eq!(reopened, id);
        assert_eq!(project, Project::default());
        assert_eq!(session, Session::new());

        // The user's half good, the app's half corrupt.
        let project = a_project();
        project
            .save_to(&project_in(&base, &id).join(PROJECT_FILE))
            .expect("saving the project");
        fs::write(project_in(&base, &id).join(SESSION_FILE), b"{ not toml")
            .expect("writing the corrupt half");

        let (_, reopened, session) = reopen_in(&base).expect("a project to reopen");
        assert_eq!(reopened, project);
        assert_eq!(session, Session::new());

        let _ = fs::remove_dir_all(&base);
    }

    /// The recent-projects view reads each row's name out of that project's own file,
    /// in the order the list keeps, and not out of the list — which holds ids and nothing
    /// else precisely so there is one copy of a name.
    #[test]
    fn the_recent_view_names_each_project_from_its_own_file() {
        let base = directory(line!());
        for (id, name) in [("first-1", "kernel"), ("second-2", "loader")] {
            let id = self::id(id);
            Project {
                name: Some(name.to_owned()),
                directory: Some(PathBuf::from("/src").join(name)),
                binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
            }
            .save_to(&project_in(&base, &id).join(PROJECT_FILE))
            .expect("saving the project");
            remember(&base, &id);
        }

        let recents = recent_projects_in(&base);
        assert_eq!(
            recents
                .iter()
                .map(|row| row.id.as_str())
                .collect::<Vec<_>>(),
            ["second-2", "first-1"]
        );
        assert_eq!(recents[0].name.as_deref(), Some("loader"));
        assert_eq!(recents[0].directory, Some(PathBuf::from("/src/loader")));
        assert_eq!(recents[0].binaries, 2);

        let _ = fs::remove_dir_all(&base);
    }

    /// Two things that are not errors and not empty rows. A project whose directory has
    /// gone is dropped here — the list never prunes itself on load, and this is the point
    /// of use where the repair is free — while one that has a directory and no readable
    /// file at all is a real project the reader can be put into, so it keeps its row and
    /// describes itself as the empty project it is.
    #[test]
    fn a_recent_project_that_is_gone_is_dropped_and_an_empty_one_is_not() {
        let base = directory(line!());
        let mut saves = Saves::new();
        let empty = open_project(&mut saves, &base).expect("a project");
        remember(&base, &id("gone-1"));

        let recents = recent_projects_in(&base);
        assert_eq!(recents.len(), 1);
        assert_eq!(recents[0].id, empty);
        assert_eq!(recents[0].name, None);
        assert_eq!(recents[0].binaries, 0);

        let _ = fs::remove_dir_all(&base);
    }

    /// Anonymity is the missing name and not the shape of the id: a project the reader
    /// later names keeps the directory it was allocated.
    #[test]
    fn an_anonymous_id_says_nothing_about_the_name() {
        let directory = directory(line!());
        let id = ProjectId::anonymous(&directory).expect("an id");

        let named = Project {
            name: Some("kernel".into()),
            ..Project::default()
        };
        let path = directory.join(id.as_str()).join(PROJECT_FILE);
        named.save_to(&path).expect("saving");
        assert_eq!(Project::load_from(&path), Some(named));

        let _ = fs::remove_dir_all(&directory);
    }
}

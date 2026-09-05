//! Projects: what the user gave — a directory, the binaries in it — and what the
//! app noticed while they read them — the open documents, where each side of each was
//! left, which one was on screen and where the reader has been.
//!
//! Framework-free: no freya types appear here.
//!
//! **A project is its project file's path.** The file is what the user said (directory,
//! binaries, bookmarks) and is written at once; beside it, named after it, is the session
//! the app noticed (tabs with their trails and rows, active document, visits, digests),
//! written on a timer. An *unsaved* project is one whose file is under the app's own
//! `projects/`; nothing else distinguishes it from one the reader gave a place.
//! The *when* of saving is
//! [`Saves`]: [`record`] writes or marks pending, [`flush`] writes what is pending.
//!
//! [`ProjectId`] is not where a project is but *which* project it is: a large random
//! number in the project file, carried by every file the app keeps beside it, so a session
//! left next to a project file that has since been replaced is not read with it.
//!
//! There is no published version of this app, so a schema change is just a schema change:
//! a file that no longer parses is the default, not a migration. It is moved aside first
//! (`rescue.rs`), the one thing owed to a reader whose file the next write would replace.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use analysis::{MadeUp, Object, Symbol, SymbolData};
use serde::{Deserialize, Deserializer, Serialize};

use crate::bookmarks::Bookmark;
use crate::cargo::Profile;
use crate::docs::{DocId, Entry};
use crate::history::{History, Stop};
use crate::rescue;
use crate::tabs::{Driven, Page, Positions, Spot};
use crate::visits::Visits;

/// The one directory everything this app stores lives under: the projects, the recent
/// list, the settings and the scratchpads.
const APP_DIR: &str = "assembly-viewer";

/// The variable that says where all of that goes, in place of the desktop's own state
/// directory.
///
/// It is there because **more than one copy of this app otherwise shares one directory**:
/// two checkouts, or a build somebody is trying something in beside the window the reader
/// actually uses. They do not merely take turns -- one writing a file the other's build
/// cannot parse is one moving the reader's file aside as unreadable, since that is what
/// every load on the way to a write does (`rescue`). Pointing a second copy somewhere of
/// its own is the whole of the answer, and it is a variable rather than a flag because it
/// has to reach every process the app starts.
pub const STATE_VARIABLE: &str = "ASSEMBLY_VIEWER_STATE";
const PROJECTS_DIR: &str = "projects";
const RECENTS_FILE: &str = "recents.toml";

/// What a project file is called. TOML inside, like everything else this app writes; the
/// extension is its own so that a file can be recognised as a project without reading it.
pub const PROJECT_EXTENSION: &str = "avproj";

/// What the app's own file for a project is called: the project file's whole name and
/// this. So the files beside a project file are named after it and one ignore rule covers
/// them.
const SESSION_EXTENSION: &str = "session";

/// How many paths [`Recents`] keeps. What is lost past this is an *order*, never a project.
const MAX_RECENTS: usize = 50;

/// How many names an unsaved project may try before giving up, so a `projects/` directory
/// that refuses every create for a reason other than collision cannot spin. [`rescue`]'s
/// bound and its reasoning.
const MAX_UNSAVED: u32 = 1000;

/// What is currently selected in the UI. There is no "nothing" variant: having none is an
/// absent one, `Option<Selection>`.
#[derive(Clone)]
pub enum Selection {
    Object(Arc<Object>),
    Symbol(Symbol),
}

impl Selection {
    /// Whether this points into the file at `path`. A symbol answers for the file its
    /// *object* came out of, and `path` is [`Object::path`] and never an object's name,
    /// so an archive closes members and all.
    pub fn in_file(&self, path: &Path) -> bool {
        match self {
            Selection::Object(object) => object.path == path,
            Selection::Symbol(symbol) => symbol.object.path == path,
        }
    }

    /// The file it came out of: an archive for a member, and never an object's name.
    pub fn file(&self) -> &Path {
        match self {
            Selection::Object(object) => &object.path,
            Selection::Symbol(symbol) => &symbol.object.path,
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
/// A tab holds one of these and has two sides — assembly and source — and the variant
/// says which side the tab is *about* and therefore which one drives the other. A file is
/// a string and not a `PathBuf`: the spelling the debug info said, or the project directory
/// joined with a Files row's entries, which is deliberately the same spelling and is never
/// canonicalised, since the two are compared as text and a file reached both ways is one tab.
///
/// [`Code`](Document::Code) is a third kind: **all of one object's code** as one listing,
/// the symbols drawn as labels inside it where they start. It is assembly-driven like a
/// symbol's tab, and one per object rather than one per place in it — where the reader
/// was in it is the tab's position, not its identity.
#[derive(Clone)]
pub enum Document {
    Assembly(Selection),
    Source(Arc<str>),
    Code(Arc<Object>),
}

impl Document {
    /// Whether this points into the file at `path`. A source-driven document answers
    /// **false** whatever the path: a file chip outlives the binary that led the reader
    /// to it.
    pub fn in_file(&self, path: &Path) -> bool {
        match self {
            Document::Assembly(selection) => selection.in_file(path),
            Document::Source(_) => false,
            Document::Code(object) => object.path == path,
        }
    }

    /// The file on disk this is a place in: the binary for the two assembly-driven
    /// kinds, and the source file itself for a file. Spelled the way the document is,
    /// so a relative one stays relative.
    pub fn file(&self) -> &Path {
        match self {
            Document::Assembly(selection) => selection.file(),
            Document::Source(file) => Path::new(&**file),
            Document::Code(object) => &object.path,
        }
    }

    /// The symbol this is about — a document that is a function, and not one that is an
    /// object or a file. What the analysis worker is asked for.
    pub fn symbol(&self) -> Option<&Symbol> {
        match self {
            Document::Assembly(Selection::Symbol(symbol)) => Some(symbol),
            _ => None,
        }
    }
}

impl PartialEq for Document {
    /// Each variant by its own rule — `Arc` pointer identity for a selection and for an
    /// object's code, text for a file — and never across the kinds: an object's code and
    /// the object itself are two documents.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Document::Assembly(a), Document::Assembly(b)) => a == b,
            (Document::Source(a), Document::Source(b)) => a == b,
            (Document::Code(a), Document::Code(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Which project this is: a large random number, made with the project and never shown.
///
/// It is not *where* a project is — that is the path of its file. It is in the project
/// file and in every file the app keeps beside it, so a session found next to a project
/// file can be asked whether it belongs to the project now in that file, rather than only
/// to whatever used to be.
///
/// Random rather than a counter: a counter is only unique to the machine that kept it, and
/// two projects made on two machines end up beside each other the moment one is checked
/// in. Sixty-four bits, written as sixteen lowercase hex digits — [`analysis::FileDigest`]'s
/// own form, and a string because TOML's only integer is signed and 64-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(u64);

impl Serialize for ProjectId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ProjectId, D::Error> {
        let text = String::deserialize(deserializer)?;
        ProjectId::parse(&text).ok_or_else(|| serde::de::Error::custom("not a project id"))
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl ProjectId {
    /// A new one, or `None` where the system will not answer for randomness — which is a
    /// project that cannot be told from another and so is not made at all.
    pub fn new() -> Option<ProjectId> {
        match getrandom::u64() {
            Ok(bits) => Some(ProjectId(bits)),
            Err(error) => {
                log::warn!("could not make a project id: {error}");
                None
            }
        }
    }

    /// The id this text spells, or `None` when it is not sixteen hex digits. Strict, since
    /// what it guards is whether a session is believed: a text this build did not write is
    /// simply not this project's, which is the answer a mismatch already gets.
    fn parse(text: &str) -> Option<ProjectId> {
        match text.len() {
            16 => u64::from_str_radix(text, 16).ok().map(ProjectId),
            _ => None,
        }
    }
}

/// The things a user can give a project that are not files: which directory it is about,
/// what to read it with, and what to build it with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub directory: Option<PathBuf>,
    pub language_server: Option<String>,
    pub cargo: Option<Cargo>,
}

impl Details {
    /// The half of a project that is what the user said.
    fn of(project: &Project) -> Details {
        Details {
            directory: project.directory.clone(),
            language_server: project.language_server.clone(),
            cargo: project.cargo.clone(),
        }
    }
}

/// What the reader chose about building, in `project.toml`'s `[cargo]`.
///
/// A table of its own, so it is **absent** until the reader has chosen something and has
/// room for what a later step adds. Absent means the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cargo {
    #[serde(default)]
    pub profile: Profile,
}

/// What the last build produced, in `session.toml`'s `[cargo]`.
///
/// Kept for one reason: the next build replaces the artifacts of the build before it, and
/// the build before it may have been in another run of the app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCargo {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<PathBuf>,
}

/// The user-given half of a project: `project.toml`.
///
/// **Field order is load-bearing** in every serde struct in this module: TOML cannot
/// reopen a table once a later one has begun, so every plain value must be emitted before
/// the first sub-table, and getting it wrong fails at *runtime* rather than at compile
/// time. The round-trip tests are what hold it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Which project this is, and what the files beside it are matched against. **Absent**
    /// in a file written by hand or by a build that had no ids: such a project opens, and
    /// nothing beside it is believed, since nothing can be matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ProjectId>,
    /// The directory the project is about, not the one it is stored in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    /// The language server this project is read with, when it is not the usual one:
    /// a program to run, found on the path or named outright. **Absent** means
    /// rust-analyzer, which is what a Rust project has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_server: Option<String>,
    /// The paths that were opened, deduplicated, in the order they were opened.
    ///
    /// `serde(default)` for the reason the session's fields have it, and for one more: a
    /// project file is **claimed empty** and filled by the first write, so between those
    /// two moments the file holds no keys at all and has to read as the empty project it
    /// is. Written always, empty or not, since it is the list and not a hint.
    #[serde(default)]
    pub binaries: Vec<PathBuf>,
    /// What to build the directory with. A table, so it comes after every plain value
    /// above and before the array of tables below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<Cargo>,
    /// The places the reader bookmarked, in the order they did. The one array of tables
    /// in this file, so it comes last; absent rather than empty when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<Bookmark>,
}

/// `skip_serializing_if` for a plain `bool`, so a false one writes no key at all.
fn is_false(value: &bool) -> bool {
    !*value
}

impl Project {
    /// Turn every path in this project the way `spelling` says, against the directory the
    /// project file is in.
    ///
    /// The **project file alone** does this, and it is what makes one worth checking in:
    /// a `binaries` naming `target/debug/viewer` is a claim about the tree the file sits
    /// in, where `/home/john/dev/viewer-a/target/debug/viewer` is a claim about one
    /// machine. A path outside that tree has nothing to be relative to and stays as it is.
    ///
    /// The session beside it is **not** turned: it is the app's own file, it never travels,
    /// and its digests are keyed by the paths the app is holding.
    fn against(&mut self, directory: &Path, spelling: Spelling) {
        let turn = |path: &mut PathBuf| match spelling {
            Spelling::Stored => {
                if let Ok(relative) = path.strip_prefix(directory) {
                    *path = relative.to_path_buf();
                }
            }
            Spelling::Working => {
                if path.is_relative() {
                    *path = directory.join(&path);
                }
            }
        };

        if let Some(about) = &mut self.directory {
            turn(about);
        }
        for binary in &mut self.binaries {
            turn(binary);
        }
        for bookmark in &mut self.bookmarks {
            if let Some(path) = bookmark.document.binary_path_mut() {
                turn(path);
            }
        }
    }

    /// Read one, or `None` if it is not there or will not parse. The plain read, and the
    /// only one: it is what draws a row for a project that is **not open**
    /// ([`recent_projects_in`]), and listing a project must not move its file aside.
    /// [`load_project`], which opens one, goes through [`rescue`].
    fn load_from(path: &Path) -> Option<Project> {
        let data = fs::read_to_string(path).ok()?;
        let mut project: Project = toml::from_str(&data).ok()?;
        if let Some(directory) = path.parent() {
            project.against(directory, Spelling::Working);
        }
        Some(project)
    }

    /// The other half: written out at `path`, with its paths turned the way the file
    /// spells them. A copy, since what the app goes on holding is the absolute form.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let mut stored = self.clone();
        if let Some(directory) = path.parent() {
            stored.against(directory, Spelling::Stored);
        }
        write_toml(path, &stored)
    }
}

/// Which way a path in a project file is being turned, [`Project::against`]'s question.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spelling {
    /// On the way out: a path under the project file's directory is written **relative to
    /// it**, so a project checked in beside the code it is about opens on another machine.
    /// Everything else stays absolute, there being nothing to be relative to.
    Stored,
    /// On the way in: a relative path is joined onto the project file's directory. The app
    /// works in absolute paths and always has -- a binary is opened by path, and two
    /// spellings of one file would be two entries in the list.
    Working,
}

/// Every binary the loaded objects came out of, deduplicated, in the order they were
/// opened — which is [`Project::binaries`], derived rather than tracked.
pub fn binaries(objects: &[Arc<Object>]) -> Vec<PathBuf> {
    let mut binaries: Vec<PathBuf> = Vec::new();
    for object in objects {
        if !binaries.contains(&object.path) {
            binaries.push(object.path.clone());
        }
    }
    binaries
}

/// How the window was arranged, in the session's `[ui]`.
///
/// Every field is an `Option` and absent means "as it comes": a window nobody has dragged
/// anything in writes no section at all, and a build that has not got one of these reads
/// the rest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedUi {
    /// How wide the sidebar was, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<f32>,
    /// How wide the **leading** side of a document was, as a percentage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<f32>,
    /// The sidebar's panels and the groups they were in. A table, so it comes after the
    /// two plain values above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<SavedDock>,
}

/// One node of the sidebar's arrangement: a row or column of others, or a group of panels.
///
/// A mirror of what the docking model holds and not that type itself, which is freya's and
/// derives no serde -- and a mirror is what keeps this module framework-free besides. The
/// panels are **strings** for [`SavedTab`]'s reason: an unknown name is a parse error where
/// a string is one panel this build does not have, and a session that will not parse is
/// moved aside whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedDock {
    /// Children side by side (`horizontal`) or stacked.
    Split {
        horizontal: bool,
        children: Vec<SavedDock>,
    },
    /// One group: the panels in it, in the order their names sit across its top, and which
    /// of them was showing.
    Group {
        panels: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        showing: Option<String>,
    },
}

/// The app-noticed half of a project: `session.toml`. Field order is load-bearing; see
/// [`Project`].
///
/// **`PartialEq` and not `Eq`**: the widths in `[ui]` are `f32`s, which is what a dragged
/// handle is. Nothing here wants the total ordering `Eq` promises -- what a session is
/// compared for is "did this change", which `PartialEq` answers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// The id of the project this was written for. A session is found by the project
    /// file's name, which says nothing about whether that file still holds the project it
    /// held — so one whose id is not the project's is ignored whole. **Absent** counts as
    /// another id: a session that cannot say which project it belongs to is not this one's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ProjectId>,
    /// The page that was on screen, where one was. A plain value, so it comes before
    /// every table below; `active` beside it is a document, and the two cannot both be
    /// set, the tab on screen being one tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    /// Whether the reader has agreed to a language server being run over the project's
    /// directory. One reads the whole project and runs its build scripts and proc macros,
    /// so it is asked about once and the answer kept.
    ///
    /// **Here and not in the project file**, which is the one thing about it that is not
    /// obvious: a project file is something a reader may check in, and a `trusted = true`
    /// travelling with it would run a language server over a stranger's tree without ever
    /// asking. The agreement is this machine's. **Absent** is no, which is what a directory
    /// nobody has been asked about has to be.
    #[serde(default, skip_serializing_if = "is_false")]
    pub trusted: bool,
    /// How the window was arranged. Absent until something in it is dragged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<SavedUi>,
    /// What the last build produced, so a build after a restart still replaces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<SessionCargo>,
    /// What each opened binary's bytes hashed to when the session was saved, keyed by the
    /// path [`Project::binaries`] holds.
    ///
    /// The values are [`analysis::FileDigest`]'s own written form, sixteen lowercase hex
    /// digits, compared as text: text this build did not write is simply not equal, which
    /// reads as "changed". A path with **no** entry here is a third state and not a
    /// mismatch — nothing new is done with it. See [`Rebuilt`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub digests: BTreeMap<PathBuf, String>,
    /// The document that was on screen, written out in full rather than as an index into
    /// `tabs`: a tab that no longer resolves is *dropped*, which would shift every later
    /// index, while this one *degrades*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<SavedDocument>,
    /// The open tabs, in strip order, of both kinds.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// `serde(default)` so a partial file — one written by hand, or trimmed — loads with
    /// an empty history rather than failing and taking the tabs down with it. The fields
    /// above carry it for the same reason.
    #[serde(default)]
    pub history: SavedHistory,
}

/// One tab as the app holds it, on its way into a [`SavedTab`]: the bar's order is the
/// order of these, and a document's trail is borrowed rather than cloned.
pub enum SavingTab<'a> {
    Page(Page),
    Document {
        id: DocId,
        trail: &'a History,
        temporal: bool,
    },
}

/// What was on screen when the session was written. Three answers and not an
/// `Option<&Document>`: a page is on screen, a document is, or nothing is.
#[derive(Clone, Copy)]
pub enum OnScreen<'a> {
    Nothing,
    Page(Page),
    Document(&'a Document),
}

/// One of the open tabs: a page, or a document's trail, oldest place first, with the
/// cursor on the place it showed and whether it was the temporal tab.
///
/// The whole trail and not the current place alone, so that Back works across a
/// restart: reopening after a rebuild is this app's daily loop, and a trail lost on every
/// restart would be worth little. The entries travel *with* the tab rather than in lists
/// beside [`Session::tabs`], because [`Session::resolve_tabs`] drops the entries and the
/// tabs that no longer resolve, which would shift every later row of a parallel array
/// onto the wrong tab. Field order is load-bearing: `page`, `temporal` and `cursor` are
/// plain values and `entries` is written as an array of tables.
///
/// A row with a `page` is a page tab and has no trail; every other field is what a
/// document tab is made of. The name is written as a **string** and not as a serde enum
/// because an unknown variant is a parse error, and a session that will not parse is
/// moved aside whole (`rescue`): a name this build does not have costs one tab, where an
/// error would cost every tab, every trail and the record of visits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Which page this tab is, and absent for a document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Whether this was the temporal tab, the preview a sidebar row opens in.
    #[serde(default)]
    pub temporal: bool,
    /// An index into `entries`: the place the tab showed.
    #[serde(default)]
    pub cursor: usize,
    #[serde(default)]
    pub entries: Vec<SavedEntry>,
}

/// One place on a saved tab's trail, and the row each of its two sides was left at.
///
/// Field order is load-bearing: the rows, the line and the address are plain values and
/// `document` is written as a sub-table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedEntry {
    /// Which row was at the top of the assembly side, `0` being the first instruction.
    /// `serde(default)` because it is a hint and not a fact.
    #[serde(default)]
    pub asm_row: usize,
    /// Which line was at the top of the source side, `0` being the file's first line.
    #[serde(default)]
    pub src_row: usize,
    /// Which line of the file a source-driven place's assembly side was driven from, and
    /// absent for every other kind. It is what makes `asm_row` mean anything for such a
    /// place: without it the listing that row is a row of is not there to come back to.
    ///
    /// Nothing resolves it -- it is a number, not a place -- so a rebuilt binary simply
    /// answers it again out of what is loaded now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The placed address at the top of an object's **code** tab, and absent for every
    /// other kind: that listing's rows are counted afresh as it is decoded, so a row
    /// there is no place to come back to and an address is. A claim about a layout, so a
    /// rebuilt binary takes it with the rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asm_address: Option<u64>,
    /// The line of the file this place *is*, for a stop in a source file, and absent for
    /// every other kind. Not the same thing as `line` above, which is what a
    /// source-driven place's assembly side follows: this is where the reader arrived and
    /// what Back comes back to, and the two part company the moment they click elsewhere
    /// in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_line: Option<u32>,
    pub document: SavedDocument,
}

/// One tab a restore opens: a page, or a document with something left on its trail. What
/// [`Session::resolve_tabs`] hands back, in the order the bar was in.
///
/// A document's trail is live, its cursor carried past the entries that no longer
/// resolve; `entries` holds the rows of every place still on it, in the trail's own order.
#[derive(Clone, PartialEq)]
pub enum RestoredTab {
    Page(Page),
    Document {
        temporal: bool,
        trail: History,
        entries: Vec<RestoredEntry>,
    },
}

/// One place of a restored tab that still points somewhere, with the rows its two sides
/// were left at.
///
/// Named rather than a tuple because it is six things, and because the rows and the
/// address drop under a rebuilt binary while the two lines do not.
#[derive(Clone, PartialEq)]
pub struct RestoredEntry {
    pub document: Document,
    pub asm_row: usize,
    pub src_row: usize,
    pub line: Option<u32>,
    pub address: Option<u64>,
    /// The line the place itself is, where it is a place in a source file.
    pub src_line: Option<u32>,
}

/// The record of visits in saved form: every place visited, oldest first. No cursor --
/// the cursors are the tabs' -- so nothing has to precede the array of tables.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    #[serde(default)]
    pub entries: Vec<SavedDocument>,
}

impl SavedHistory {
    fn from_visits(visits: &Visits) -> SavedHistory {
        SavedHistory {
            entries: visits
                .entries()
                .iter()
                .map(SavedDocument::from_document)
                .collect(),
        }
    }
}

/// The projects the reader has had open, most recently first: `recents.toml`.
///
/// Which project to reopen is the first entry and not a field of its own. This is an
/// *order*, not an index of what exists — the project files are that — which is why
/// nothing here prunes a path whose file has gone; `recent_projects_in` does that at
/// the point of use.
///
/// A path under the app's own storage is written **relative to it** and every other path
/// absolutely, so that moving the state directory — a different user, a restored backup —
/// does not lose every unsaved project. In memory they are all absolute: the relative
/// spelling belongs to the file and nowhere else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Recents {
    #[serde(default)]
    projects: Vec<PathBuf>,
}

impl Recents {
    fn first(&self) -> Option<&Path> {
        self.projects.first().map(PathBuf::as_path)
    }

    /// Put `path` at the front, and say whether that changed anything — which is what keeps
    /// a startup that reopens the project already at the front from writing a file.
    fn touch(&mut self, path: &Path) -> bool {
        if self.first() == Some(path) {
            return false;
        }
        self.projects.retain(|other| other != path);
        self.projects.insert(0, path.to_path_buf());
        self.projects.truncate(MAX_RECENTS);
        true
    }

    /// Drop `path` from the order, and say whether it was there. Nothing else prunes this
    /// file, so a project that has gone for good is taken out here.
    fn forget(&mut self, path: &Path) -> bool {
        let before = self.projects.len();
        self.projects.retain(|other| other != path);
        self.projects.len() != before
    }

    /// The stored order, with every path made absolute. A file that will not parse is
    /// moved aside first: the next [`remember`] writes this file, so ignoring it would
    /// lose the order without the reader ever hearing about it.
    fn load_in(base: &Path) -> Recents {
        let mut recents: Recents = rescue::parse(base, &recents_in(base)).unwrap_or_default();
        for path in &mut recents.projects {
            if path.is_relative() {
                *path = base.join(&path);
            }
        }
        recents
    }

    /// The same the other way, for the write: what is under `base` goes back to relative.
    fn stored_in(&self, base: &Path) -> Recents {
        Recents {
            projects: self
                .projects
                .iter()
                .map(|path| match path.strip_prefix(base) {
                    Ok(relative) => relative.to_path_buf(),
                    Err(_) => path.clone(),
                })
                .collect(),
        }
    }
}

/// One row of the recent-projects view: a project that can be switched to, described by
/// its own file read at the moment the list is asked for, so nothing about a project is
/// copied beside the order. A project whose file will not parse still gets a row, as the
/// [`Project::default`] it will behave as once opened — and the file stays where it is
/// until it is opened, a row being a reading of a project and not a claim on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    /// The project file: what a project is, what it is called by, and what opening this
    /// row opens.
    pub path: PathBuf,
    pub directory: Option<PathBuf>,
    pub binaries: usize,
}

/// The projects the reader has had open, most recently first, each described by its own
/// file — or an empty list on a system with nowhere to keep them.
pub fn recent_projects() -> Vec<Recent> {
    base()
        .map(|base| recent_projects_in(&base))
        .unwrap_or_default()
}

/// The whole of the above except finding the state directory. A path whose file has gone
/// is dropped here rather than repaired, since [`Recents`] never prunes itself on load and
/// this is the point of use where the repair is free.
fn recent_projects_in(base: &Path) -> Vec<Recent> {
    Recents::load_in(base)
        .projects
        .into_iter()
        .filter_map(|path| {
            if !path.is_file() {
                return None;
            }
            let project = Project::load_from(&path).unwrap_or_default();
            Some(Recent {
                path,
                directory: project.directory,
                binaries: project.binaries.len(),
            })
        })
        .collect()
}

/// The binaries that are no longer the files the session was saved against.
///
/// A mismatch is not an error and not a refusal to open; it only decides how much of a
/// saved place may still be believed. The **name** is believed, since a rebuild keeps
/// most of its function names. The **address** is not: under a rebuilt file it stops
/// being a requirement (recovering a symbol that merely moved) and stops being evidence
/// (a name that names two symbols and no longer names an address resolves to neither).
/// The saved **row** is not either, being a claim about a listing this build no longer
/// has.
#[derive(Debug)]
enum Rebuilt {
    /// The paths whose saved digest no longer matches the file loaded under them.
    Paths(HashSet<PathBuf>),
    /// Every path, whatever the digests say: what a bookmark resolves under
    /// ([`SavedDocument::resolve_by_name`]), being saved against no digest at all.
    Every,
}

impl Rebuilt {
    /// Compare every saved digest against the file loaded under that path now. Only a
    /// digest present on both sides and *different* is a rebuild. Per saved path rather
    /// than per object, so an archive's 196 members ask it once.
    fn of(session: &Session, objects: &[Arc<Object>]) -> Rebuilt {
        let mut rebuilt = HashSet::new();
        for (path, digest) in &session.digests {
            let Some(object) = objects.iter().find(|object| object.path == *path) else {
                continue;
            };
            if object.data.digest().to_string() != *digest {
                log::debug!(
                    "{} has changed since the session was saved; matching by name",
                    path.display()
                );
                rebuilt.insert(path.clone());
            }
        }
        Rebuilt::Paths(rebuilt)
    }

    fn changed(&self, path: &Path) -> bool {
        match self {
            Rebuilt::Paths(paths) => paths.contains(path),
            Rebuilt::Every => true,
        }
    }
}

/// A [`Document`] expressed in terms that survive a restart.
///
/// `object_name` is [`Object::name`] — the archive member name, or the file name for a
/// plain object — and is needed because one path can contribute many `Object`s, so `path`
/// alone is ambiguous. [`SavedDocument::Source`]'s `path` is a `String` rather than a
/// `PathBuf` because it is what the debug info said and not something this filesystem was
/// asked about; writing it as a path would invite [`write_toml`]'s non-UTF-8
/// refusal on a value that was UTF-8 all along.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedDocument {
    Object {
        path: PathBuf,
        object_name: String,
    },
    Symbol {
        path: PathBuf,
        object_name: String,
        address: u64,
        /// Last, and after `address`: a name the file stated is written as a table of its
        /// own, and a table cannot precede a plain value (`write_toml`).
        symbol_name: SavedName,
    },
    Source {
        path: String,
    },
    /// All of an object's code, named the way its object is.
    Code {
        path: PathBuf,
        object_name: String,
    },
}

/// What a saved symbol is called: the file's own name for it, or, for one the app named
/// itself, **which** name the app made up. The address is saved beside it either way
/// ([`SavedDocument::Symbol`]).
///
/// The spelling of a made-up name is not saved. It is a function of which name it is and
/// the symbol's address ([`MadeUp`]), so those two go in the file and the name is rendered
/// again on the way back. A bookmark on `<function 0x140001000>` therefore survives the app
/// deciding to spell that some other way: a saved string would quietly stop matching the
/// symbol it was made on, and a bookmark that resolves to nothing is a bookmark gone.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedName {
    /// The file's own name, spelled as the file spells it.
    File(String),
    /// The app's name for the entry point.
    EntryPoint,
    /// The app's name for a function at the saved address.
    Function,
    /// The app's name for a fragment at the saved address.
    Fragment,
}

impl SavedName {
    /// The saved form of `name`, borne by a symbol at `address`. Every variant of
    /// [`MadeUp`] is named here, so a fourth made-up name is a compile error and not a
    /// spelling silently written to a file.
    pub fn of(name: &str, address: u64) -> SavedName {
        match MadeUp::of(name, address) {
            None => SavedName::File(name.to_owned()),
            Some(MadeUp::EntryPoint) => SavedName::EntryPoint,
            Some(MadeUp::Function(_)) => SavedName::Function,
            Some(MadeUp::Fragment(_)) => SavedName::Fragment,
        }
    }

    /// The name a symbol at `address` carries now: the file's own as it was saved, or a
    /// made-up one spelled the way the app spells it today. What the symbol is looked up
    /// by.
    pub fn text(&self, address: u64) -> Cow<'_, str> {
        match (self.made_up(address), self) {
            (Some(made_up), _) => Cow::Owned(made_up.to_string()),
            (None, SavedName::File(name)) => Cow::Borrowed(name),
            // [`SavedName::made_up`] answers `None` for `File` and for nothing else.
            (None, _) => Cow::Borrowed(""),
        }
    }

    /// Which name the app made up, for a symbol at `address`; [`None`] where the name is
    /// the file's own. The other half of [`SavedName::of`], and what says a saved place
    /// can spell itself without the file it points into being open.
    pub fn made_up(&self, address: u64) -> Option<MadeUp> {
        match self {
            SavedName::File(_) => None,
            SavedName::EntryPoint => Some(MadeUp::EntryPoint),
            SavedName::Function => Some(MadeUp::Function(address)),
            SavedName::Fragment => Some(MadeUp::Fragment(address)),
        }
    }
}

impl SavedDocument {
    /// The saved form of `document`.
    pub fn from_document(document: &Document) -> SavedDocument {
        match document {
            Document::Code(object) => SavedDocument::Code {
                path: object.path.clone(),
                object_name: object.name.clone(),
            },
            Document::Assembly(Selection::Object(object)) => SavedDocument::Object {
                path: object.path.clone(),
                object_name: object.name.clone(),
            },
            Document::Assembly(Selection::Symbol(symbol)) => SavedDocument::Symbol {
                path: symbol.object.path.clone(),
                object_name: symbol.object.name.clone(),
                address: symbol.data.address,
                symbol_name: SavedName::of(&symbol.data.name, symbol.data.address),
            },
            Document::Source(file) => SavedDocument::Source {
                path: file.to_string(),
            },
        }
    }

    /// The name this place spells for itself: a made-up symbol name, rendered from what
    /// was saved ([`SavedName`]). [`None`] for every other place, whose name is the
    /// file's or the reader's and has to be stored to be drawn.
    pub fn made_up_name(&self) -> Option<String> {
        match self {
            SavedDocument::Symbol {
                address,
                symbol_name,
                ..
            } => symbol_name.made_up(*address).map(|name| name.to_string()),
            _ => None,
        }
    }

    /// The binary this names, or `None` for a file.
    fn binary_path(&self) -> Option<&Path> {
        match self {
            SavedDocument::Object { path, .. }
            | SavedDocument::Symbol { path, .. }
            | SavedDocument::Code { path, .. } => Some(path),
            SavedDocument::Source { .. } => None,
        }
    }

    /// The same to write into: what [`Project::against`] rewrites, and `None` for a source
    /// file, whose path is what the debug information said rather than something this
    /// filesystem was asked about.
    fn binary_path_mut(&mut self) -> Option<&mut PathBuf> {
        match self {
            SavedDocument::Object { path, .. }
            | SavedDocument::Symbol { path, .. }
            | SavedDocument::Code { path, .. } => Some(path),
            SavedDocument::Source { .. } => None,
        }
    }

    /// The loaded object this names, if it is still there.
    fn find_object<'a>(&self, objects: &'a [Arc<Object>]) -> Option<&'a Arc<Object>> {
        let path = self.binary_path()?;
        let name = match self {
            SavedDocument::Object { object_name, .. }
            | SavedDocument::Symbol { object_name, .. }
            | SavedDocument::Code { object_name, .. } => object_name.as_str(),
            SavedDocument::Source { .. } => return None,
        };
        objects
            .iter()
            .find(|object| object.path == path && object.name == name)
    }

    /// Exactly what this names, or `None` when the object — or, for a symbol, the symbol
    /// — is no longer loaded. What history entries want: an entry that no longer points
    /// where it did is dropped rather than turned into a destination the user never
    /// visited.
    ///
    /// A source-driven entry resolves against nothing and so cannot fail: a deleted file
    /// comes back as a tab over the pane's own "Source file not found".
    fn resolve(&self, objects: &[Arc<Object>], rebuilt: &Rebuilt) -> Option<Document> {
        if let SavedDocument::Source { path } = self {
            return Some(Document::Source(Arc::from(path.as_str())));
        }

        let object = self.find_object(objects)?;
        let selection = match self {
            SavedDocument::Code { .. } => return Some(Document::Code(object.clone())),
            SavedDocument::Object { .. } => Selection::Object(object.clone()),
            SavedDocument::Symbol {
                path,
                symbol_name,
                address,
                ..
            } => SavedDocument::find_symbol(
                object,
                &symbol_name.text(*address),
                *address,
                rebuilt.changed(path),
            )
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

    /// What this names against whatever is loaded, believing the **name** over the address
    /// whether or not the file is known to have changed: [`Rebuilt::Every`]'s reading of
    /// [`SavedDocument::resolve`]. What a bookmark resolves by.
    ///
    /// It is the answer the digest-aware rule gives wherever the two could be compared. An
    /// unchanged file still holds the exact name-and-address pair the place was saved
    /// with, so the exact match wins there as it does under the strict rule; a rebuilt file
    /// is read exactly as a rebuilt file is. And a bookmark cannot be read the other way:
    /// `Session::digests` is the digest at the last *session* save, not at the bookmark's
    /// making, so on the second launch after a rebuild the file would read as unchanged and
    /// a stale address would drop a bookmark the reader made on purpose.
    pub fn resolve_by_name(&self, objects: &[Arc<Object>]) -> Option<Document> {
        self.resolve(objects, &Rebuilt::Every)
    }

    /// The symbol a saved place names, under a file that either is or is not the one it
    /// was saved against.
    ///
    /// The candidates are the run of `symbols_sorted` that carries the name, found by two
    /// binary searches over a list that is sorted by name — 115k entries on the repo's own
    /// binary.
    ///
    /// **Unchanged** (or never hashed): the name *and* the address, which is what tells two
    /// same-named symbols apart.
    ///
    /// **Rebuilt**: the name, with the address as a tie-breaker only. An exact match is
    /// still preferred; failing that, a name that names exactly one symbol resolves to
    /// it. A name that names several and matches no address resolves to **nothing** —
    /// picking one on the strength of a stale address is how a reader ends up on a
    /// function they never opened.
    fn find_symbol<'a>(
        object: &'a Object,
        name: &str,
        address: u64,
        rebuilt: bool,
    ) -> Option<&'a Arc<SymbolData>> {
        let sorted = &object.symbols_sorted;
        let from = sorted.partition_point(|data| data.name.as_str() < name);
        let named = &sorted[from..];
        let named = &named[..named.partition_point(|data| data.name == name)];

        let exact = named.iter().find(|data| data.address == address);
        match (exact, rebuilt, named) {
            (Some(data), _, _) => Some(data),
            (None, true, [one]) => Some(one),
            (None, _, _) => None,
        }
    }

    /// The same, degrading instead of failing: a symbol that is gone falls back to its
    /// object and an object that is gone to nothing at all. What the *active document*
    /// wants, there being one of it and the app having to open somewhere.
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
            id: None,
            active_page: None,
            trusted: false,
            ui: None,
            cargo: None,
            digests: BTreeMap::new(),
            active: None,
            tabs: Vec::new(),
            // Spelt out rather than `SavedHistory::default()`, which is not a `const fn`.
            history: SavedHistory {
                entries: Vec::new(),
            },
        }
    }

    /// The session described by the state the app is currently in — the one place the
    /// app's state is turned into what would be saved, [`binaries`] being the other half
    /// of it for the other file. `tabs` is each open tab in strip order: its id, its
    /// trail, and whether it is the temporal one. A side that was never scrolled has no
    /// entry in its [`Positions`] at all and is written out as row `0`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_state(
        objects: &[Arc<Object>],
        tabs: &[SavingTab<'_>],
        asm_rows: &Positions<Entry>,
        src_rows: &Positions<Entry>,
        places: &Positions<Entry, Spot>,
        driven: &Driven,
        shown: OnScreen<'_>,
        visits: &Visits,
        artifacts: &[PathBuf],
        trusted: bool,
        ui: SavedUi,
    ) -> Session {
        let mut digests: BTreeMap<PathBuf, String> = BTreeMap::new();
        for object in objects {
            // Read off the object rather than computed here: the hash was taken once, on
            // the parse worker thread, and every object out of one file answers the same
            // thing — so an archive's members cost one pass rather than one each.
            digests
                .entry(object.path.clone())
                .or_insert_with(|| object.data.digest().to_string());
        }
        Session {
            // Absent here and stamped by [`Saves::record`]: which project this is belongs
            // to the save policy, not to the state the app is in.
            id: None,
            active_page: match shown {
                OnScreen::Page(page) => Some(page.stored().to_owned()),
                OnScreen::Document(_) | OnScreen::Nothing => None,
            },
            trusted,
            // Absent rather than empty, so a window nobody has arranged writes no section.
            ui: (ui != SavedUi::default()).then_some(ui),
            // Absent rather than empty, so a project nothing was ever built in writes no
            // section at all.
            cargo: (!artifacts.is_empty()).then(|| SessionCargo {
                artifacts: artifacts.to_vec(),
            }),
            digests,
            active: match shown {
                OnScreen::Document(document) => Some(SavedDocument::from_document(document)),
                OnScreen::Page(_) | OnScreen::Nothing => None,
            },
            tabs: tabs
                .iter()
                .map(|tab| match tab {
                    SavingTab::Page(page) => SavedTab {
                        page: Some(page.stored().to_owned()),
                        temporal: false,
                        cursor: 0,
                        entries: Vec::new(),
                    },
                    SavingTab::Document {
                        id,
                        trail,
                        temporal,
                    } => SavedTab {
                        page: None,
                        temporal: *temporal,
                        cursor: trail.cursor().unwrap_or(0),
                        entries: trail
                            .entries()
                            .iter()
                            .map(|stop| {
                                let entry = (*id, stop.clone());
                                SavedEntry {
                                    asm_row: asm_rows.at(&entry).unwrap_or(0),
                                    src_row: src_rows.at(&entry).unwrap_or(0),
                                    line: driven.line(&entry),
                                    asm_address: places.at(&entry).map(|spot| spot.address),
                                    src_line: stop.line,
                                    document: SavedDocument::from_document(&stop.document),
                                }
                            })
                            .collect(),
                    },
                })
                .collect(),
            history: SavedHistory::from_visits(visits),
        }
    }

    /// The saved active document against the objects that are now loaded. Degrades
    /// silently: a symbol that is gone falls back to its object, an object that is gone
    /// to nothing.
    pub fn resolve(&self, objects: &[Arc<Object>]) -> Option<Document> {
        let saved = self.active.as_ref()?;
        saved.resolve_or_degrade(objects, &Rebuilt::of(self, objects))
    }

    /// The page that was on screen, where one was and this build still has it.
    pub fn shown_page(&self) -> Option<Page> {
        Page::from_stored(self.active_page.as_deref()?)
    }

    /// The saved pages with the place each had in the bar. A page resolves against no
    /// object, so these are what a restore can put back before any binary has been read
    /// -- and all it has to put back for a project with no binaries at all.
    pub fn pages(&self) -> impl Iterator<Item = (usize, Page)> + '_ {
        self.tabs.iter().enumerate().filter_map(|(position, tab)| {
            Some((position, Page::from_stored(tab.page.as_deref()?)?))
        })
    }

    /// The saved tabs as live trails, in strip order, each place with the rows its two
    /// sides were left at. A place that no longer resolves is **dropped** from its trail
    /// rather than degraded, the cursor carried the way [`History::rebuilt`] carries it
    /// -- the same walk closing a file goes through, so the two cannot drift -- and a tab
    /// with nothing left on its trail is dropped: a strip whose tabs all degraded onto
    /// the same object would collapse into one.
    pub fn resolve_tabs(&self, objects: &[Arc<Object>]) -> Vec<RestoredTab> {
        let rebuilt = Rebuilt::of(self, objects);
        self.tabs
            .iter()
            .filter_map(|saved| {
                // A page resolves against nothing, and one this build does not have is
                // dropped as a place that no longer resolves is.
                if let Some(page) = &saved.page {
                    return Some(RestoredTab::Page(Page::from_stored(page)?));
                }
                let resolved: Vec<Option<RestoredEntry>> = saved
                    .entries
                    .iter()
                    .map(|entry| {
                        let document = entry.document.resolve(objects, &rebuilt)?;
                        // A row is a claim about a listing, so a rebuilt listing takes
                        // both its rows with it; the place itself survives. A file has
                        // no binary path and so is never rebuilt. The driven line is a
                        // claim about a *file* rather than about a listing, so it
                        // survives a rebuild and is simply asked again.
                        let changed = entry
                            .document
                            .binary_path()
                            .is_some_and(|path| rebuilt.changed(path));
                        let (asm_row, src_row, address) = match changed {
                            true => (0, 0, None),
                            false => (entry.asm_row, entry.src_row, entry.asm_address),
                        };
                        Some(RestoredEntry {
                            document,
                            asm_row,
                            src_row,
                            line: entry.line,
                            address,
                            src_line: entry.src_line,
                        })
                    })
                    .collect();
                let trail = History::rebuilt(
                    resolved.iter().map(|entry| {
                        entry.as_ref().map(|entry| Stop {
                            document: entry.document.clone(),
                            address: entry.address,
                            line: entry.src_line,
                        })
                    }),
                    saved.cursor,
                );
                trail.current()?;
                // The rows of what survived, in the trail's order; `rebuilt` keeps the
                // survivors in the order they were given, so the two agree.
                let entries = resolved.into_iter().flatten().collect();
                Some(RestoredTab::Document {
                    temporal: saved.temporal,
                    trail,
                    entries,
                })
            })
            .collect()
    }

    /// The saved record of visits as a live one. A place that no longer resolves is
    /// dropped: a list of places the reader cannot get back to is worse than a short
    /// list.
    pub fn resolve_history(&self, objects: &[Arc<Object>]) -> Visits {
        let rebuilt = Rebuilt::of(self, objects);
        Visits::restored(
            self.history
                .entries
                .iter()
                .filter_map(|saved| saved.resolve(objects, &rebuilt))
                .collect(),
        )
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_toml(path, self)
    }
}

/// The directory the app keeps everything in, or `None` on a system with no state or
/// local data directory to put it in.
/// Read on **every** call and not cached, so nothing anywhere has to be sequenced against
/// the moment it is first asked for. It is an environment lookup, and the app asks at most
/// once per save.
pub fn base() -> Option<PathBuf> {
    given_base(std::env::var_os(STATE_VARIABLE)).or_else(desktop_base)
}

/// The directory the variable names, or `None` where it names nothing.
///
/// **Unset and empty are one answer.** A variable set to nothing is what a script that
/// meant to set it and did not looks like, and taking that as a path would put the reader's
/// projects in whatever directory the app was started from.
fn given_base(given: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let given = given?;
    match given.is_empty() {
        true => None,
        false => Some(PathBuf::from(given)),
    }
}

/// Where the desktop says an application's state goes, which is where this app keeps it
/// when nobody has said otherwise.
fn desktop_base() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(APP_DIR))
}

fn projects_in(base: &Path) -> PathBuf {
    base.join(PROJECTS_DIR)
}

fn recents_in(base: &Path) -> PathBuf {
    base.join(RECENTS_FILE)
}

/// Where the session for the project at `path` is: beside it, under its whole name. The
/// name and not the stem, so the two files sort together and one ignore rule reaches both.
fn session_beside(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(SESSION_EXTENSION);
    PathBuf::from(name)
}

/// Whether the project at `path` is one the app is keeping for want of anywhere else: an
/// **unsaved** project. Being under `projects/` is the whole of it, since that is the one
/// place the app puts a project the reader has not given a place.
fn is_unsaved(base: &Path, path: &Path) -> bool {
    path.starts_with(projects_in(base))
}

/// Claim a file for a project the reader has not given a place, and hand back its path.
///
/// The claim *is* the `create_new`: one atomic operation that fails with `AlreadyExists`
/// rather than opening what is there, so the loop cannot hand out a name another copy of
/// the app is already using. The file is left empty; the first write fills it.
fn unsaved_project(projects: &Path) -> Option<PathBuf> {
    fs::create_dir_all(projects).ok()?;
    for n in 1..=MAX_UNSAVED {
        let path = projects.join(format!("{n}.{PROJECT_EXTENSION}"));
        match fs::File::create_new(&path) {
            Ok(_) => return Some(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                log::warn!(
                    "could not make a project file in {}: {error}",
                    projects.display()
                );
                return None;
            }
        }
    }
    None
}

/// The number an unsaved project's file is named by. `None` for a project the reader gave
/// a place, which is called by that file instead.
fn unsaved_number(base: &Path, path: &Path) -> Option<String> {
    match is_unsaved(base, path) {
        true => Some(path.file_stem()?.to_string_lossy().into_owned()),
        false => None,
    }
}

/// Whether `path` is a project file at all, which is the whole of what is asked of one
/// before it is opened: the extension and nothing else, so a file can be recognised without
/// being read. What is *in* it is [`load_project`]'s answer.
pub fn is_project_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == PROJECT_EXTENSION)
}

/// Whether the project kept at `path` is one the app is keeping for want of anywhere else.
/// The question a view asks before drawing a Save where a close would be.
pub fn unsaved(path: &Path) -> bool {
    base().is_some_and(|base| is_unsaved(&base, path))
}

/// What to call the project kept at `path`: the file's name, or `Unsaved project 3` for one
/// the app is keeping for want of anywhere else. The whole of the naming rule, and here
/// rather than in a view because more than one draws it.
pub fn label(path: &Path) -> String {
    if let Some(number) = base().and_then(|base| unsaved_number(&base, path)) {
        return format!("Unsaved project {number}");
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Write `contents` to `path` by writing `path.tmp` first and renaming it over the top,
/// so an interrupted write cannot leave a half-written file behind and a concurrent reader
/// sees either the old file or the new one, never a truncated one. The parent directory is
/// made if it is not there, which is what lets a project's first write create its
/// directory.
///
/// The temporary is **synced before the rename**. A rename is atomic against a crash of
/// the process, but not against a power loss: the directory entry can reach the disk
/// before the data does, and the file the next launch then reads is zero bytes or a
/// truncated tail -- which will not parse, so `rescue` moves the reader's project or
/// session aside and answers a default. The cost is one fsync per save, at most one every
/// 30 s. The directory entry itself is left unsynced: losing the rename costs the last
/// save, where losing the data costs the file.
///
/// The one atomic writer for everything the app stores: the two project files, the recent
/// list, the settings and a scratchpad's package.
pub fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let mut file = fs::File::create(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temporary, path)
}

/// The same for a value written as TOML.
///
/// TOML cannot spell a path that is not UTF-8 and serde's `PathBuf` impl fails rather
/// than mangling one, so such a project is simply not written: the error is logged and
/// swallowed by the caller, leaving the previous good file in place. This is a *runtime*
/// failure, not a compile-time one.
pub fn write_toml(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let data = toml::to_string_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_atomically(path, data.as_bytes())
}

/// How often [`flush`] is worth calling. Far coarser than the rate a user clicks through
/// symbols at, while bounding what an unclean exit can lose; a clean window close flushes
/// anyway.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// A `static` rather than UI state because two of the three things that drive it — the
/// periodic flush and the window's close hook — sit outside the component tree.
static SAVES: Mutex<Saves> = Mutex::new(Saves::new());

struct Saves {
    /// The project file everything is written into, or `None` until one has been reopened
    /// or created. Otherwise claimed on the first write that has anything to say, so a run
    /// where nothing was ever opened leaves no file behind.
    open: Option<PathBuf>,
    /// Which project that file holds. Kept here rather than asked of the file, because it
    /// is stamped onto both halves of every write and the two must agree: a session
    /// carrying another id is one the next load throws away.
    id: Option<ProjectId>,
    /// The name and directory as last written: the baseline a rename is measured against.
    ///
    /// Seeded by [`Saves::opened`] where the two below are pointedly empty, because every
    /// baseline is the state the app boots into and only this one is restored
    /// synchronously.
    given: Details,
    /// The bookmarks as last written: the other baseline seeded by [`Saves::opened`], since
    /// they too are restored synchronously, out of the file and not out of a parse.
    bookmarks: Vec<Bookmark>,
    /// The binaries the app was last seen holding, out of a moment when nothing was
    /// loading. Empty to start with, deliberately not the ones loaded at startup: they
    /// arrive asynchronously, so a baseline holding them would read the still-empty boot
    /// state as a change and write an empty project over a good one.
    binaries: Vec<PathBuf>,
    /// What `project.toml` currently *says* the binaries are.
    ///
    /// The same list as `binaries` in every state but one: while a load is in flight, the
    /// app holds what has landed so far while the file names the whole list. A write that
    /// is not about the binaries writes this back rather than the app's own list, so a
    /// rename in that window cannot forget them.
    listed: Vec<PathBuf>,
    /// The session as last written, empty for `binaries`' reason.
    session: Session,
    /// A newer session that has not been written yet. Only ever a *session*: a change to
    /// the other file is written at once.
    pending: Option<Session>,
}

impl Saves {
    const fn new() -> Saves {
        Saves {
            open: None,
            id: None,
            given: Details {
                directory: None,
                language_server: None,
                cargo: None,
            },
            bookmarks: Vec::new(),
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

    /// Note that `id` is the project the app is now in, and set every baseline to the
    /// state the app will be in the instant afterwards. The two empty baselines are
    /// *assigned* rather than assumed because a project switched away from leaves its own
    /// binaries and pending session behind.
    fn opened(&mut self, path: PathBuf, project: &Project, trusted: bool) {
        self.open = Some(path);
        self.id = project.id;
        self.given = Details::of(project);
        self.bookmarks = project.bookmarks.clone();
        self.binaries = Vec::new();
        self.listed = project.binaries.clone();
        // The id and the agreement, and nothing else. Both are restored *synchronously*
        // -- the one from the file being opened, the other into `Proj` beside it -- so a
        // baseline without them would read the state the app boots into as a change.
        self.session = Session {
            id: project.id,
            trusted,
            ..Session::new()
        };
        self.pending = None;
    }

    /// Take note of the state the app is now in. Hands back the `project.toml` to write
    /// now and the `session.toml` beside it where one is owed, or `None` when nothing
    /// changed or the change can wait for a flush.
    ///
    /// A **binaries** change goes to disk at once and carries whatever session was
    /// pending with it, which is what keeps `session.toml` from ever naming a tab into a
    /// binary `project.toml` no longer lists. A **rename** is immediate too but writes
    /// `project.toml` alone, since it lets go of no binary, and so is a change to the
    /// **bookmarks**, for the same reason. Everything else — a selection, a tab, a history
    /// entry — only marks the session pending. Nothing here has to say which is which:
    /// which file a field lives in is what decides it.
    ///
    /// While a load is in flight `binaries` is not the app's list but the part of it that
    /// has landed, so it is neither compared nor written: a project naming only what has
    /// arrived, and the session the app holds until a restore has resolved its tabs,
    /// would otherwise go to disk over the good ones. Both baselines are left where they
    /// were for the record that follows the load, which is the one that sees the change.
    /// A binaries change the reader makes in that window waits for the same record.
    ///
    /// **No baseline moves here**, since a baseline is what the *file* holds and the file
    /// has not been written yet. The caller moves them with [`Saves::wrote_project`] and
    /// [`Saves::wrote_session`] once the write has landed, so a write that fails leaves
    /// the change for the next record to see again. The pending session is not a baseline
    /// and is set here as ever: it is what has not been written.
    fn record(
        &mut self,
        details: Details,
        binaries: Vec<PathBuf>,
        loading: bool,
        bookmarks: Vec<Bookmark>,
        session: Session,
    ) -> Option<Recorded> {
        // Stamped here rather than by the caller: which project this is belongs to the
        // policy and not to the UI, and stamping before the comparison is what keeps the
        // baseline and what arrives comparable.
        let session = Session {
            id: self.id,
            ..session
        };
        let binaries_changed = !loading && self.binaries != binaries;
        let details_changed = self.given != details;
        let bookmarks_changed = self.bookmarks != bookmarks;

        if !binaries_changed && !details_changed && !bookmarks_changed {
            if *self.latest() != session {
                self.pending = Some(session);
            }
            return None;
        }

        // A write that is not about the binaries keeps the ones already in the file; see
        // [`Saves::listed`].
        let listed = match binaries_changed {
            true => binaries,
            false => self.listed.clone(),
        };
        let project = Project {
            id: self.id,
            directory: details.directory,
            language_server: details.language_server,
            binaries: listed,
            cargo: details.cargo,
            bookmarks,
        };

        if binaries_changed {
            return Some(Recorded {
                project,
                binaries_changed,
                session: Some(session),
            });
        }

        if *self.latest() != session {
            self.pending = Some(session);
        }
        Some(Recorded {
            project,
            binaries_changed,
            session: None,
        })
    }

    /// Whatever was recorded but not written, or `None` when the two already agree. Left
    /// pending until [`Saves::wrote_session`] says it reached the disk.
    fn owing(&self) -> Option<Session> {
        self.pending.clone()
    }

    /// Note that `project` reached `project.toml`. The details, the bookmarks and the
    /// binaries the file lists are what it says they are. The app's own list moves only
    /// when the change was to the binaries: any other write put back the list the file
    /// already held.
    fn wrote_project(&mut self, project: &Project, binaries_changed: bool) {
        self.given = Details::of(project);
        self.bookmarks = project.bookmarks.clone();
        self.listed = project.binaries.clone();
        if binaries_changed {
            self.binaries = project.binaries.clone();
        }
    }

    /// Note that `session` reached `session.toml`: it is what the file holds, and nothing
    /// is owed.
    fn wrote_session(&mut self, session: Session) {
        self.session = session;
        self.pending = None;
    }

    /// Note that the project is now kept at `path` under `id`. Only *where* it is has
    /// changed, so every baseline but the id stays: the app is holding what it was holding
    /// a moment ago, and the files just written say the same.
    fn moved_to(&mut self, path: PathBuf, id: Option<ProjectId>) {
        self.open = Some(path);
        self.id = id;
        self.session.id = id;
        if let Some(pending) = &mut self.pending {
            pending.id = id;
        }
    }

    /// Note that there is no project open. Every baseline back to what the app boots into,
    /// because the caller is about to empty the app -- one still describing the project
    /// just left would read that emptying as a change and write it back into it.
    fn closed(&mut self) {
        *self = Saves::new();
    }

    /// The other answer: the write did not happen, so the session is owed again and the
    /// next flush tries it rather than finding nothing to do.
    fn owes_session(&mut self, session: Session) {
        self.pending = Some(session);
    }
}

/// What a [`Saves::record`] decided to write, and what it takes to note that it landed.
struct Recorded {
    /// The `project.toml` to write now.
    project: Project,
    /// The `session.toml` to write beside it, which only a binaries change carries.
    session: Option<Session>,
    /// Whether that change was to the binaries: the one baseline `project.toml`'s write
    /// moves only sometimes.
    binaries_changed: bool,
}

fn saves() -> MutexGuard<'static, Saves> {
    // Take the state back rather than propagate: a poisoned lock must not turn a failed
    // save into a crashed app.
    SAVES.lock().unwrap_or_else(|error| error.into_inner())
}

/// The project file everything is written into, or `None` when there is no project open.
///
/// **It creates nothing.** It used to claim a file for an unsaved project on the first write
/// that had anything to say, which was how "opening files with no project makes one" worked
/// — but with no project the reader can still arrange the window and open Settings, and a
/// lazy claim turns any of that into a project appearing on disk behind their back. A
/// project is made where the reader asks for one ([`start_new`], reached from the menu), and
/// with none open the two write paths do nothing at all.

/// Take `path` out of `recents.toml`, writing the file only when it was there. What a
/// project deleted, or moved somewhere else, leaves behind.
fn forget(base: &Path, path: &Path) {
    let mut recents = Recents::load_in(base);
    if !recents.forget(path) {
        return;
    }
    write_recents(base, &recents);
}

/// Put `path` at the front of `recents.toml`, writing the file only when that moved it.
fn remember(base: &Path, path: &Path) {
    let mut recents = Recents::load_in(base);
    if !recents.touch(path) {
        return;
    }
    write_recents(base, &recents);
}

/// The one write of that file, which is where the paths under `base` go back to relative.
fn write_recents(base: &Path, recents: &Recents) {
    let path = recents_in(base);
    if let Err(error) = write_toml(&path, &recents.stored_in(base)) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Reopen the project the app was last in: the first entry of `recents.toml`. Hands back
/// both halves for the caller to restore, and points the save policy at it — but seeds it
/// with nothing else (see [`Saves::binaries`]).
pub fn reopen() -> Option<(PathBuf, Project, Session)> {
    let (path, project, session) = reopen_in(&base()?)?;
    saves().opened(path.clone(), &project, session.trusted);
    Some((path, project, session))
}

/// The whole of the above except telling [`Saves`], so a test can point it at a directory
/// of its own.
fn reopen_in(base: &Path) -> Option<(PathBuf, Project, Session)> {
    let recents = Recents::load_in(base);
    let path = recents.first()?.to_path_buf();
    let (project, session) = load_project(base, &path)?;
    Some((path, project, session))
}

/// Both halves of the project the file at `path` holds, or `None` when it is not there or
/// will not parse.
///
/// **The project file is never moved aside**, however it fails: it may be the reader's own
/// file, sitting in their tree beside the code, and the app has no business taking one
/// away. `None` here therefore means the project does not open at all, and since nothing
/// opens, nothing writes over what could not be read. That is the whole of the rule — the
/// plain read is [`Project::load_from`], which the recent list has always used for the same
/// reason.
///
/// The session beside it *is* the app's own, and goes through [`rescue`] like everything
/// else the app stores. One written for another project is dropped rather than believed:
/// the file is found by the project file's name, which says nothing about whether that file
/// still holds the project it did.
fn load_project(base: &Path, path: &Path) -> Option<(Project, Session)> {
    let project = match Project::load_from(path) {
        Some(project) => project,
        None => {
            log::debug!("the project {} will not open", path.display());
            return None;
        }
    };

    let session: Session = rescue::parse(base, &session_beside(path)).unwrap_or_default();
    let session = match session.id == project.id && project.id.is_some() {
        true => session,
        false => {
            if session != Session::new() {
                log::debug!("the session beside {} is another project's", path.display());
            }
            Session::new()
        }
    };
    Some((project, session))
}

/// Leave the project the app is in and enter the one the file at `path` holds, handing back
/// both halves for the caller to restore. `None` — and nothing changed at all — when it is
/// not there or will not parse.
///
/// The order matters. The project being left is flushed **first**, while [`Saves`] still
/// points at it. The new one is then remembered, and [`Saves::opened`] empties the
/// baselines because the caller is about to empty the app — a baseline still describing
/// the old binaries would read that emptying as a change and write it into the project
/// just entered. Emptying the app is the caller's half, the states being the UI's.
pub fn switch(path: &Path) -> Option<(Project, Session)> {
    flush();
    let (_, project, session) = open_at(path)?;
    log::debug!("switched to the project {}", path.display());
    Some((project, session))
}

/// Open the project the file at `path` holds without leaving one first: what a startup
/// given a project file on the command line does, where there is nothing to flush.
/// [`switch`] is this with the flush in front of it.
pub fn open_at(path: &Path) -> Option<(PathBuf, Project, Session)> {
    let base = base()?;
    let (project, session) = load_project(&base, path)?;
    remember(&base, path);
    saves().opened(path.to_path_buf(), &project, session.trusted);
    Some((path.to_path_buf(), project, session))
}

/// Start a project the reader has not given a place and enter it: [`switch`] with nothing
/// to load.
pub fn start_new() -> Option<PathBuf> {
    flush();
    let base = base()?;
    let path = unsaved_project(&projects_in(&base))?;
    let project = Project {
        id: ProjectId::new(),
        ..Project::default()
    };
    remember(&base, &path);
    saves().opened(path.clone(), &project, false);
    log::debug!("started the project {}", path.display());
    Some(path)
}

/// Whether putting a project somewhere leaves the old place behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Put {
    /// Save as: the project is **copied** to the new place under an id of its own, the app
    /// is then in the copy, and what was copied is left as it was. A new id because the two
    /// are now two projects, and one id across both would mean each matched the other's
    /// session -- so shuffling the files around would silently pick up the wrong tabs.
    Copy,
    /// Save: the project is **moved**, keeping its id, and nothing is left behind. What an
    /// unsaved project has instead of Save as, there being no second project afterwards.
    Move,
}

/// Put the open project in the file at `path`. Answers whether it was written.
///
/// Read and written rather than copied byte for byte, because a path in a project file is
/// relative to the file's own directory ([`Project::against`]): the same bytes in another
/// directory would be a claim about *that* tree. The session beside it holds absolute paths
/// and is only carried across.
///
/// The pending session is flushed **first**, while [`Saves`] still points at the old place,
/// so what is carried across is what the app holds and not what the disk happened to have.
pub fn put_in(path: &Path, put: Put) -> bool {
    flush();
    let mut saves = saves();
    let (Some(base), Some(from)) = (base(), saves.open.clone()) else {
        log::warn!("no project to save");
        return false;
    };
    let Some((project, session)) = load_project(&base, &from) else {
        return false;
    };

    let id = match put {
        Put::Copy => ProjectId::new(),
        Put::Move => project.id,
    };
    let project = Project { id, ..project };
    let session = Session { id, ..session };

    if !write_or_warn(path, |path| project.save_to(path)) {
        return false;
    }
    // The session is the app's own and regenerable, so a failure here is worth a line in
    // the log and nothing more: the project itself is already where the reader asked.
    write_or_warn(&session_beside(path), |path| session.save_to(path));

    if put == Put::Move {
        for leaving in [from.clone(), session_beside(&from)] {
            if let Err(error) = fs::remove_file(&leaving) {
                // The copy is made and the app has moved on; a file left behind is untidy
                // and not lost work.
                log::warn!("could not remove {}: {error}", leaving.display());
            }
        }
        forget(&base, &from);
    }
    remember(&base, path);
    saves.moved_to(path.to_path_buf(), id);
    log::debug!("the project is now {}", path.display());
    true
}

/// Leave the project the app is in, with nothing open afterwards. What is pending is
/// written **first**, while [`Saves`] still points at it.
pub fn close() {
    flush();
    let mut saves = saves();
    saves.closed();
}

/// The same, and take the project away with it. Answers whether it was removed.
///
/// **Only ever a project in app storage**: one the reader gave a place is their own file
/// and this app has no business deleting it, whatever asked. Nothing is flushed, the
/// project being about to go.
pub fn delete() -> bool {
    let mut saves = saves();
    let (Some(base), Some(path)) = (base(), saves.open.clone()) else {
        return false;
    };
    if !is_unsaved(&base, &path) {
        log::warn!("{} is not the app's to delete", path.display());
        return false;
    }

    for going in [path.clone(), session_beside(&path)] {
        if let Err(error) = fs::remove_file(&going) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not remove {}: {error}", going.display());
            }
        }
    }
    forget(&base, &path);
    saves.closed();
    log::debug!("deleted the project {}", path.display());
    true
}

/// Take note of the project the app is now in, writing it out immediately if it is a
/// change that must not be lost and marking it pending otherwise. Cheap enough to call on
/// every state change. `loading` says the binaries are still arriving, which is what
/// keeps a half-read list off the disk.
pub fn record(
    details: Details,
    binaries: Vec<PathBuf>,
    loading: bool,
    bookmarks: Vec<Bookmark>,
    session: Session,
) {
    // The write happens under the lock, so two writes can never reach the file out of
    // the order they were decided in, and nothing can slip between a write and the
    // baseline it moves.
    let mut saves = saves();
    let Some(recorded) = saves.record(details, binaries, loading, bookmarks, session) else {
        return;
    };
    let Some(file) = writing_into(&saves) else {
        log::warn!("no state directory to save the project in");
        if let Some(session) = recorded.session {
            saves.owes_session(session);
        }
        return;
    };
    // The id is only minted when the file is claimed, so a project that was not open when
    // the record was decided has one now and both halves take it.
    let project = Project {
        id: saves.id,
        ..recorded.project
    };

    if write_or_warn(&file, |path| project.save_to(path)) {
        saves.wrote_project(&project, recorded.binaries_changed);
    }
    if let Some(session) = recorded.session {
        let session = Session {
            id: saves.id,
            ..session
        };
        match write_or_warn(&session_beside(&file), |path| session.save_to(path)) {
            true => saves.wrote_session(session),
            false => saves.owes_session(session),
        }
    }
}

/// Write out anything recorded but not yet written. A no-op when nothing has changed,
/// which is what makes it safe to call on a timer.
pub fn flush() {
    let mut saves = saves();
    let Some(session) = saves.owing() else {
        return;
    };
    let Some(file) = writing_into(&saves) else {
        log::warn!("no state directory to save the session in");
        return;
    };
    let session = Session {
        id: saves.id,
        ..session
    };
    if write_or_warn(&session_beside(&file), |path| session.save_to(path)) {
        saves.wrote_session(session);
    }
}

/// The project file to write, or `None` when there is no project to write into — in which
/// case nothing is written and nothing is made. The session goes beside it.
fn writing_into(saves: &Saves) -> Option<PathBuf> {
    saves.open.clone()
}

/// Any IO failure is logged and swallowed: failing to persist is never worth interrupting
/// the user for. Answers whether the file was written, which is what says whether the
/// baseline behind it may move: a save recorded as done is a save nothing retries.
fn write_or_warn(path: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> bool {
    if let Err(error) = write(path) {
        log::warn!("could not save {}: {error}", path.display());
        return false;
    }
    true
}

#[cfg(test)]
mod tests;

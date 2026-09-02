//! Projects: what the user gave — a name, a directory, the binaries in it — and what the
//! app noticed while they read them — the open documents, where each side of each was
//! left, which one was on screen and where the reader has been.
//!
//! Framework-free: no freya types appear here.
//!
//! A project is a directory under `projects/`, and [`ProjectId`] is that directory's
//! name. It is two files: `project.toml` is what the user said (name, directory,
//! binaries) and is written at once; `session.toml` is what the app noticed (tabs, rows,
//! active document, history, digests) and is written on a timer. The *when* of saving is
//! [`Saves`]: [`record`] writes or marks pending, [`flush`] writes what is pending.
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

use crate::bookmarks::Bookmark;
use crate::history::History;
use crate::tabs::{Driven, Positions, Spot};

/// The one directory everything this app stores lives under: the projects, the recent
/// list, the settings and the scratchpads.
const APP_DIR: &str = "assembly-viewer";
const PROJECTS_DIR: &str = "projects";
const PROJECT_FILE: &str = "project.toml";
const SESSION_FILE: &str = "session.toml";
const RECENTS_FILE: &str = "recents.toml";

/// The stem an anonymous project's id is built from. The spelling carries no meaning:
/// what makes a project anonymous is the missing name.
const ANONYMOUS_STEM: &str = "project";

/// How many ids [`Recents`] keeps. What is lost past this is an *order*, never a project.
const MAX_RECENTS: usize = 50;

/// The longest an id may be, so a hand-edited or hostile `recents.toml` cannot ask for a
/// path component no filesystem will take.
const MAX_ID: usize = 64;

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

/// A project's identity: the name of the directory its two files live in.
///
/// A newtype because it is interpolated into a path and is read back out of a file a user
/// can edit. [`ProjectId::new`] is the only way to make one and `Deserialize` goes
/// through it, so an id out of a hand-edited `recents.toml` cannot be `..`, an absolute
/// path or a name with a separator in it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl<'de> Deserialize<'de> for ProjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ProjectId, D::Error> {
        let text = String::deserialize(deserializer)?;
        ProjectId::new(text).ok_or_else(|| serde::de::Error::custom("not a project id"))
    }
}

impl ProjectId {
    /// The id this text names, or `None` when it is not a single ordinary path component.
    ///
    /// Deliberately stricter than the filesystem: ASCII letters, digits, `-` and `_`
    /// only, starting with a letter or digit — so an id is the same string on every
    /// platform the app runs on.
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
    /// The claim *is* the `create_dir`: one atomic operation that fails with
    /// `AlreadyExists` rather than opening what is there, so the loop cannot hand out an
    /// id another copy of the app is already using. Bounded at a thousand tries, so a
    /// directory that refuses every `create_dir` for a reason other than collision cannot
    /// spin.
    fn anonymous(projects: &Path) -> Option<ProjectId> {
        fs::create_dir_all(projects).ok()?;
        for n in 1..=1000 {
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub name: Option<String>,
    pub directory: Option<PathBuf>,
}

/// The user-given half of a project: `project.toml`.
///
/// **Field order is load-bearing** in every serde struct in this module: TOML cannot
/// reopen a table once a later one has begun, so every plain value must be emitted before
/// the first sub-table, and getting it wrong fails at *runtime* rather than at compile
/// time. The round-trip tests are what hold it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// What the reader called it, or **absent** when they never called it anything —
    /// which is what makes a project anonymous, so it must not be an empty string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The directory the project is about, not the one it is stored in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    /// The paths that were opened, deduplicated, in the order they were opened.
    pub binaries: Vec<PathBuf>,
    /// The places the reader bookmarked, in the order they did. The one array of tables
    /// in this file, so it comes last; absent rather than empty when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<Bookmark>,
}

impl Project {
    fn load_from(path: &Path) -> Option<Project> {
        let data = fs::read_to_string(path).ok()?;
        toml::from_str(&data).ok()
    }
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

/// The app-noticed half of a project: `session.toml`. Field order is load-bearing; see
/// [`Project`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
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

/// One of the open tabs: a place, and the row each of its two sides was left at.
///
/// The rows travel *with* the tab rather than in lists beside [`Session::tabs`], because
/// [`Session::resolve_tabs`] drops the tabs that no longer resolve, which would shift
/// every later row of a parallel array onto the wrong tab. Field order is load-bearing:
/// both rows are plain values and `document` is written as a sub-table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Which row was at the top of the assembly side, `0` being the first instruction.
    /// `serde(default)` because it is a hint and not a fact.
    #[serde(default)]
    pub asm_row: usize,
    /// Which line was at the top of the source side, `0` being the file's first line.
    #[serde(default)]
    pub src_row: usize,
    /// Which line of the file a source-driven tab's assembly side was driven from, and
    /// absent for every other tab. It is what makes `asm_row` mean anything for such a
    /// tab: without it the listing that row is a row of is not there to come back to.
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
    pub document: SavedDocument,
}

/// A saved tab that still points somewhere: what [`Session::resolve_tabs`] hands back.
///
/// Named rather than a tuple because it is five things now, and because the rows and the
/// address drop under a rebuilt binary while the line does not.
#[derive(Clone, PartialEq)]
pub struct RestoredTab {
    pub document: Document,
    pub asm_row: usize,
    pub src_row: usize,
    pub line: Option<u32>,
    pub address: Option<u64>,
}

/// The navigation history in saved form: the index of the entry that was on screen, and
/// every visited selection, oldest first. Field order is load-bearing: `entries` is
/// written as an array of tables, so the plain `cursor` has to precede it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    /// An index into `entries`, and `0` — meaning nothing — while it is empty.
    pub cursor: usize,
    #[serde(default)]
    pub entries: Vec<SavedDocument>,
}

impl SavedHistory {
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
/// Which project to reopen is the first entry and not a field of its own. This is an
/// *order*, not an index of what exists — the projects are the directories — which is why
/// nothing here prunes an id whose directory has gone; `recent_projects_in` does that at
/// the point of use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Recents {
    #[serde(default)]
    projects: Vec<ProjectId>,
}

impl Recents {
    fn first(&self) -> Option<&ProjectId> {
        self.projects.first()
    }

    /// Put `id` at the front, and say whether that changed anything — which is what keeps
    /// a startup that reopens the project already at the front from writing a file.
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
}

/// One row of the recent-projects view: a project that can be switched to, described by
/// its own `project.toml` read at the moment the list is asked for, so a name is never
/// copied beside the order. A project whose file will not parse still gets a row, as the
/// [`Project::default`] it will behave as once opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    pub id: ProjectId,
    pub name: Option<String>,
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

/// The whole of the above except finding the state directory. An id whose directory has
/// gone is dropped here rather than repaired, since [`Recents`] never prunes itself on
/// load and this is the point of use where the repair is free.
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
        symbol_name: String,
        address: u64,
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
                symbol_name: symbol.data.name.clone(),
                address: symbol.data.address,
            },
            Document::Source(file) => SavedDocument::Source {
                path: file.to_string(),
            },
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
            digests: BTreeMap::new(),
            active: None,
            tabs: Vec::new(),
            // Spelt out rather than `SavedHistory::default()`, which is not a `const fn`.
            history: SavedHistory {
                cursor: 0,
                entries: Vec::new(),
            },
        }
    }

    /// The session described by the state the app is currently in — the one place the
    /// app's state is turned into what would be saved, [`binaries`] being the other half
    /// of it for the other file. A side that was never scrolled has no entry in its
    /// [`Positions`] at all and is written out as row `0`.
    pub fn from_state(
        objects: &[Arc<Object>],
        tabs: &[Document],
        asm_rows: &Positions<Document>,
        src_rows: &Positions<Document>,
        places: &Positions<Document, Spot>,
        driven: &Driven,
        active: Option<&Document>,
        history: &History,
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
            digests,
            active: active.map(SavedDocument::from_document),
            tabs: tabs
                .iter()
                .map(|tab| SavedTab {
                    asm_row: asm_rows.at(tab).unwrap_or(0),
                    src_row: src_rows.at(tab).unwrap_or(0),
                    line: driven.line(tab),
                    asm_address: places.at(tab).map(|spot| spot.address),
                    document: SavedDocument::from_document(tab),
                })
                .collect(),
            history: SavedHistory::from_history(history),
        }
    }

    /// The saved active document against the objects that are now loaded. Degrades
    /// silently: a symbol that is gone falls back to its object, an object that is gone
    /// to nothing.
    pub fn resolve(&self, objects: &[Arc<Object>]) -> Option<Document> {
        let saved = self.active.as_ref()?;
        saved.resolve_or_degrade(objects, &Rebuilt::of(self, objects))
    }

    /// The saved tabs as live documents, in strip order, each with the rows its two sides
    /// were left at. A tab that no longer resolves is **dropped** rather than degraded: a
    /// strip whose tabs all degraded onto the same object would collapse into one.
    pub fn resolve_tabs(&self, objects: &[Arc<Object>]) -> Vec<RestoredTab> {
        let rebuilt = Rebuilt::of(self, objects);
        self.tabs
            .iter()
            .filter_map(|saved| {
                let document = saved.document.resolve(objects, &rebuilt)?;
                // A row is a claim about a listing, so a rebuilt listing takes both its
                // rows with it; the tab itself survives. A file has no binary path and so
                // is never rebuilt. The driven line is a claim about a *file* rather than
                // about a listing, so it survives a rebuild and is simply asked again.
                let changed = saved
                    .document
                    .binary_path()
                    .is_some_and(|path| rebuilt.changed(path));
                let (asm_row, src_row, address) = match changed {
                    true => (0, 0, None),
                    false => (saved.asm_row, saved.src_row, saved.asm_address),
                };
                Some(RestoredTab {
                    document,
                    asm_row,
                    src_row,
                    line: saved.line,
                    address,
                })
            })
            .collect()
    }

    /// The saved history as a live one. An entry that no longer resolves is dropped;
    /// where the cursor lands is [`History::rebuilt`]'s business, the same walk closing a
    /// file goes through, so the two cannot drift.
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

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        write_toml(path, self)
    }
}

/// The directory the app keeps everything in, or `None` on a system with no state or
/// local data directory to put it in.
pub fn base() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(APP_DIR))
}

fn projects_in(base: &Path) -> PathBuf {
    base.join(PROJECTS_DIR)
}

fn project_in(base: &Path, id: &ProjectId) -> PathBuf {
    projects_in(base).join(id.as_str())
}

fn recents_in(base: &Path) -> PathBuf {
    base.join(RECENTS_FILE)
}

/// Write `contents` to `path` by writing `path.tmp` first and renaming it over the top,
/// so an interrupted write cannot leave a half-written file behind and a concurrent reader
/// sees either the old file or the new one, never a truncated one. The parent directory is
/// made if it is not there, which is what lets a project's first write create its
/// directory.
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

    fs::write(&temporary, contents)?;
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
    /// The project everything is written into, or `None` until one has been reopened or
    /// created. Otherwise allocated on the first write that has anything to say, so a run
    /// where nothing was ever opened leaves no directory behind.
    open: Option<ProjectId>,
    /// The name and directory as last written: the baseline a rename is measured against.
    ///
    /// Seeded by [`Saves::opened`] where the two below are pointedly empty, because every
    /// baseline is the state the app boots into and only this one is restored
    /// synchronously.
    given: Details,
    /// The bookmarks as last written: the other baseline seeded by [`Saves::opened`], since
    /// they too are restored synchronously, out of the file and not out of a parse.
    bookmarks: Vec<Bookmark>,
    /// The binaries the app was last seen holding. Empty to start with, deliberately not
    /// the ones loaded at startup: they arrive asynchronously, so a baseline holding them
    /// would read the still-empty boot state as a change and write an empty project over
    /// a good one.
    binaries: Vec<PathBuf>,
    /// What `project.toml` currently *says* the binaries are.
    ///
    /// The same list as `binaries` in every state but one: between a project being
    /// reopened and its parse landing, the app holds none while the file names several. A
    /// write that is not about the binaries writes this back rather than the app's own
    /// list, so a rename in that window cannot forget them.
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
            given: Details {
                name: None,
                directory: None,
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

    fn project(&self, binaries: Vec<PathBuf>) -> Project {
        Project {
            name: self.given.name.clone(),
            directory: self.given.directory.clone(),
            binaries,
            bookmarks: self.bookmarks.clone(),
        }
    }

    /// Note that `id` is the project the app is now in, and set every baseline to the
    /// state the app will be in the instant afterwards. The two empty baselines are
    /// *assigned* rather than assumed because a project switched away from leaves its own
    /// binaries and pending session behind.
    fn opened(&mut self, id: ProjectId, project: &Project) {
        self.open = Some(id);
        self.given = Details {
            name: project.name.clone(),
            directory: project.directory.clone(),
        };
        self.bookmarks = project.bookmarks.clone();
        self.binaries = Vec::new();
        self.listed = project.binaries.clone();
        self.session = Session::new();
        self.pending = None;
    }

    /// Take note of the state the app is now in. Hands back the `project.toml` to write
    /// now and the `session.toml` beside it where one is owed, or `None` when nothing
    /// changed or the change can wait for a [`Saves::flush`].
    ///
    /// A **binaries** change goes to disk at once and carries whatever session was
    /// pending with it, which is what keeps `session.toml` from ever naming a tab into a
    /// binary `project.toml` no longer lists. A **rename** is immediate too but writes
    /// `project.toml` alone, since it lets go of no binary, and so is a change to the
    /// **bookmarks**, for the same reason. Everything else — a selection, a tab, a history
    /// entry — only marks the session pending. Nothing here has to say which is which:
    /// which file a field lives in is what decides it.
    fn record(
        &mut self,
        details: Details,
        binaries: Vec<PathBuf>,
        bookmarks: Vec<Bookmark>,
        session: Session,
    ) -> Option<(Project, Option<Session>)> {
        let binaries_changed = self.binaries != binaries;
        let details_changed = self.given != details;
        let bookmarks_changed = self.bookmarks != bookmarks;

        if !binaries_changed && !details_changed && !bookmarks_changed {
            if *self.latest() != session {
                self.pending = Some(session);
            }
            return None;
        }

        self.given = details;
        self.bookmarks = bookmarks;
        self.binaries = binaries.clone();
        // A write that is not about the binaries keeps the ones already in the file; see
        // [`Saves::listed`].
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
            return Some((project, Some(session)));
        }

        if *self.latest() != session {
            self.pending = Some(session);
        }
        Some((project, None))
    }

    /// Take whatever was recorded but not written, or `None` when the two already agree.
    fn flush(&mut self) -> Option<Session> {
        let session = self.pending.take()?;
        self.session = session.clone();
        Some(session)
    }
}

fn saves() -> MutexGuard<'static, Saves> {
    // Take the state back rather than propagate: a poisoned lock must not turn a failed
    // save into a crashed app.
    SAVES.lock().unwrap_or_else(|error| error.into_inner())
}

/// The project everything is written into, creating an anonymous one — and remembering it
/// as the most recent — if there is not one yet. Called from the write paths and nowhere
/// else, which is what makes a project appear on disk exactly when there is something to
/// put in it.
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
    if let Err(error) = write_toml(&path, &recents) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Reopen the project the app was last in: the first entry of `recents.toml`. Hands back
/// both halves for the caller to restore, and points the save policy at it — but seeds it
/// with nothing else (see [`Saves::binaries`]).
pub fn reopen() -> Option<(ProjectId, Project, Session)> {
    let (id, project, session) = reopen_in(&base()?)?;
    saves().opened(id.clone(), &project);
    Some((id, project, session))
}

/// The whole of the above except telling [`Saves`], so a test can point it at a directory
/// of its own.
fn reopen_in(base: &Path) -> Option<(ProjectId, Project, Session)> {
    let recents = Recents::load_from(&recents_in(base));
    let id = recents.first()?.clone();
    let (project, session) = load_project(base, &id)?;
    Some((id, project, session))
}

/// Both halves of the project `id` names, or `None` when its directory is gone. The
/// directory is the only thing that has to be there: either file being missing or
/// unreadable is simply the default half.
fn load_project(base: &Path, id: &ProjectId) -> Option<(Project, Session)> {
    let directory = project_in(base, id);
    if !directory.is_dir() {
        log::debug!("the project {} is no longer there", id.as_str());
        return None;
    }

    let project = Project::load_from(&directory.join(PROJECT_FILE)).unwrap_or_default();
    let session: Session = fs::read_to_string(directory.join(SESSION_FILE))
        .ok()
        .and_then(|data| toml::from_str(&data).ok())
        .unwrap_or_default();
    Some((project, session))
}

/// Leave the project the app is in and enter the one `id` names, handing back both halves
/// for the caller to restore. `None` — and nothing changed at all — when its directory has
/// gone since the recent list named it.
///
/// The order matters. The project being left is flushed **first**, while [`Saves`] still
/// points at it. The new one is then remembered, and [`Saves::opened`] empties the
/// baselines because the caller is about to empty the app — a baseline still describing
/// the old binaries would read that emptying as a change and write it into the project
/// just entered. Emptying the app is the caller's half, the states being the UI's.
pub fn switch(id: &ProjectId) -> Option<(Project, Session)> {
    flush();
    let base = base()?;
    let (project, session) = load_project(&base, id)?;
    remember(&base, id);
    saves().opened(id.clone(), &project);
    log::debug!("switched to the project {}", id.as_str());
    Some((project, session))
}

/// Start a project nobody has named yet and enter it: [`switch`] with nothing to load.
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
/// change that must not be lost and marking it pending otherwise. Cheap enough to call on
/// every state change.
pub fn record(
    details: Details,
    binaries: Vec<PathBuf>,
    bookmarks: Vec<Bookmark>,
    session: Session,
) {
    // The write happens under the lock, so two writes can never reach the file out of
    // the order they were decided in.
    let mut saves = saves();
    let Some((project, session)) = saves.record(details, binaries, bookmarks, session) else {
        return;
    };
    let Some(directory) = writing_into(&mut saves) else {
        log::warn!("no state directory to save the project in");
        return;
    };
    write_or_warn(&directory.join(PROJECT_FILE), |path| {
        write_toml(path, &project)
    });
    if let Some(session) = session {
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

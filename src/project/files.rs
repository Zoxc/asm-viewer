//! The two files a project is stored in, as serde reads and writes them: the project
//! file the reader may check in, the session the app keeps beside it, and the id that
//! ties the two together.
//!
//! The schema and nothing else — the keys, how a path in a project file is spelled, the
//! one read and the two writes. Resolving a saved place against what is loaded is
//! [`super::restore`]; when a file is written is [`super::saves`].

use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use analysis::{MadeUp, SectionAddress, SymbolData};
use serde::{Deserialize, Deserializer, Serialize};

use crate::bookmarks::Bookmark;
use crate::cargo::Profile;
use crate::document::Kind;
use crate::store::Store;

/// What a project file is called. TOML inside, like everything else this app writes; the
/// extension is its own so that a file can be recognised as a project without reading it.
pub const PROJECT_EXTENSION: &str = "avproj";

/// What the app's own file for a project is called: the project file's whole name and
/// this. So the files beside a project file are named after it and one ignore rule covers
/// them.
const SESSION_EXTENSION: &str = "session";

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
    pub(super) fn parse(text: &str) -> Option<ProjectId> {
        match text.len() {
            16 => u64::from_str_radix(text, 16).ok().map(ProjectId),
            _ => None,
        }
    }
}

/// The things a user can give a project that are not files: which directory it is about,
/// what to read it with, and what to build it with.
///
/// The project file's own fields, flattened into [`Project`] rather than repeated there,
/// so a new one is added here and nowhere else.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Details {
    /// The directory the project is about, not the one it is stored in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    /// The language server this project is read with, when it is not the usual one:
    /// a program to run, found on the path or named outright. **Absent** means
    /// rust-analyzer, which is what a Rust project has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_server: Option<String>,
    /// Which of the project's files that server is for, as extensions separated by
    /// whatever the reader typed between them: `c h cpp`, or `rs`. **Absent** means the
    /// program's own answer -- Rust for rust-analyzer, and every language this app knows
    /// for a program it cannot guess about (`language_files`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_files: Option<String>,
    /// What to build the directory with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cargo: Option<Cargo>,
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Which project this is, and what the files beside it are matched against. **Absent**
    /// in a file written by hand or by a build that had no ids: such a project opens, and
    /// nothing beside it is believed, since nothing can be matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ProjectId>,
    /// What the reader said about the project, which is the one thing the app does not
    /// decide for itself. Flattened, so these are keys of the file like the rest.
    #[serde(flatten)]
    pub details: Details,
    /// The paths that were opened, deduplicated, in the order they were opened.
    ///
    /// `serde(default)` for the reason the session's fields have it, and for one more: a
    /// project file is **claimed empty** and filled by the first write, so between those
    /// two moments the file holds no keys at all and has to read as the empty project it
    /// is. Written always, empty or not, since it is the list and not a hint.
    #[serde(default)]
    pub binaries: Vec<PathBuf>,
    /// The places the reader bookmarked, in the order they did. Absent rather than empty
    /// when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<Bookmark>,
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

        if let Some(about) = &mut self.details.directory {
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

    /// Read one, or [`Reason`] if it is not there or will not parse. The plain read, and
    /// the only one: it is what draws a row for a project that is **not open**
    /// ([`super::recent_projects`]), and listing a project must not move its file aside.
    /// [`super::load_project`], which opens one, goes through [`Store::read`].
    ///
    /// The **bytes** and not a string, so that a file which is not text is told apart from
    /// one the system would not hand over: they fail as one `io::Error` otherwise, and
    /// what the reader is told about a project that will not open is the point of this
    /// answering a reason at all.
    pub(super) fn load_from(path: &Path) -> Result<Project, Reason> {
        let data = fs::read(path).map_err(Reason::reading)?;
        let text = std::str::from_utf8(&data).map_err(|_| Reason::NotText)?;
        let mut project: Project =
            toml::from_str(text).map_err(|error| Reason::of(&error, text))?;
        if let Some(directory) = path.parent() {
            project.against(directory, Spelling::Working);
        }
        Ok(project)
    }

    /// The other half: written out at `path`, with its paths turned the way the file
    /// spells them. A copy, since what the app goes on holding is the absolute form.
    pub(super) fn save_to(&self, store: &Store, path: &Path) -> std::io::Result<()> {
        let mut stored = self.clone();
        if let Some(directory) = path.parent() {
            stored.against(directory, Spelling::Stored);
        }
        store.write_toml(path, &stored)
    }
}

/// A project that would not open: the file the reader asked for, and what was wrong with
/// it.
///
/// **The project file is never moved aside** ([`super::load_project`]), so telling the reader is
/// the whole of what happens to a project that will not open -- which is why the reason is
/// carried this far rather than logged and dropped. The path travels with it because
/// [`super::reopen`] picks the file itself: the one open nobody asked for is the one whose failure
/// names a file the reader has not seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The project file, as it was asked for.
    pub path: PathBuf,
    pub reason: Reason,
}

/// What was wrong with a project file. Its [`fmt::Display`] is the sentence the reader is
/// shown, so each is a whole one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// There is nothing at that path.
    Missing,
    /// The system would not hand the file over: no permission, a directory, a broken
    /// link.
    Unreadable(String),
    /// Not text at all, so nothing can be said about where it goes wrong.
    NotText,
    /// There is nowhere for the app to keep its own files, so there is nothing to open a
    /// project into. Nothing to do with the file, and the one reason this module does not
    /// find for itself: it is what a caller with no [`Store`] has instead of one.
    NoStore,
    /// Text, but not TOML this app can read: what the parser said, and where it stopped
    /// when it said where.
    Malformed {
        /// The line and the column, both counted from one and in characters rather than
        /// bytes -- what an editor puts in its corner.
        at: Option<(usize, usize)>,
        message: String,
    },
}

impl Reason {
    /// What an `io::Error` from the read was about. Not being there is its own answer:
    /// it is the one failure here that is nobody's mistake, and the only one a startup
    /// keeps quiet about.
    fn reading(error: std::io::Error) -> Reason {
        match error.kind() {
            std::io::ErrorKind::NotFound => Reason::Missing,
            _ => Reason::Unreadable(error.to_string()),
        }
    }

    /// What a TOML error said, and where in `text` it said it.
    ///
    /// Taken apart rather than printed: the error's own [`fmt::Display`] is a three-line
    /// diagram with a caret under the column, which lines up only in a fixed-width font
    /// and only while nothing wraps -- neither of which a window this wide can promise.
    /// A span that is not a character boundary, or is past the end, costs the position
    /// and not the message.
    fn of(error: &toml::de::Error, text: &str) -> Reason {
        let at = error
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| {
                let line = before.matches('\n').count() + 1;
                let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                (line, column)
            });
        Reason::Malformed {
            at,
            message: error.message().to_owned(),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Missing => write!(formatter, "There is no file there."),
            Reason::Unreadable(error) => write!(formatter, "It could not be read: {error}."),
            Reason::NotText => write!(formatter, "It is not a text file."),
            Reason::NoStore => write!(formatter, "The app has nowhere to keep its own files."),
            Reason::Malformed {
                at: Some((line, column)),
                message,
            } => write!(
                formatter,
                "It will not parse: {message}, at line {line}, column {column}."
            ),
            Reason::Malformed { at: None, message } => {
                write!(formatter, "It will not parse: {message}.")
            }
        }
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
    /// The sidebar's panels and the groups they were in.
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

/// The app-noticed half of a project: `session.toml`.
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
    /// The page that was on screen, where one was. `active` beside it is a document, and
    /// the two cannot both be set, the tab on screen being one tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    /// Whether the reader has agreed to a language server being run over the project's
    /// directory. One reads the whole project and runs its build scripts and proc macros,
    /// so it is asked about once and the answer kept.
    ///
    /// **Never in either file**, since both can arrive with the project -- checked in, or
    /// in an archive -- and a `trusted = true` travelling with them would run a language
    /// server over a stranger's tree without ever asking. The agreement is this machine's:
    /// it is kept in the store ([`super::trust`]) and carried here only on its way in and
    /// out.
    #[serde(skip)]
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
    /// mismatch — nothing new is done with it. See [`super::restore::Changed`].
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

impl Session {
    pub(super) fn save_to(&self, store: &Store, path: &Path) -> std::io::Result<()> {
        store.write_toml(path, self)
    }
}

/// One of the open tabs: a page, or a document's trail, newest place first, with the
/// cursor on the place it showed and whether it was the temporal tab.
///
/// The whole trail and not the current place alone, so that Back works across a
/// restart: reopening after a rebuild is this app's daily loop, and a trail lost on every
/// restart would be worth little. The entries travel *with* the tab rather than in lists
/// beside [`Session::tabs`], because a restore drops the entries and the tabs that no
/// longer resolve, which would shift every later row of a parallel array onto the wrong
/// tab.
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedEntry {
    /// Which row was at the top of the assembly side, `0` being the first instruction.
    /// `serde(default)` because it is a hint and not a fact. For an object's code, how
    /// many rows past `asm_address`'s own row.
    #[serde(default)]
    pub asm_row: usize,
    /// How far into that row, in 65536ths of it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub asm_into: u16,
    /// Which line was at the top of the source side, `0` being the file's first line.
    #[serde(default)]
    pub src_row: usize,
    /// How far into that line, in 65536ths of it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub src_into: u16,
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
    /// rebuilt binary takes it with the rows. Where the tab was *scrolled* to and nothing
    /// else; the place it is at is `code_address` below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asm_address: Option<u64>,
    /// The address this place *is*: placed, for a stop in an object's code, and the
    /// symbol's own, for an instruction of a symbol; absent for every other kind. Not the same thing as `asm_address` above, which is where that
    /// listing was scrolled to: this is where the reader arrived and what Back comes back
    /// to, and the two part company the moment they scroll.
    ///
    /// A claim about a layout as `asm_address` is, so a rebuilt binary takes it too and
    /// the place comes back as the whole listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_address: Option<u64>,
    /// The line of the file this place *is*, for a stop in a source file, and absent for
    /// every other kind. Not the same thing as `line` above, which is what a
    /// source-driven place's assembly side follows: this is where the reader arrived and
    /// what Back comes back to, and the two part company the moment they click elsewhere
    /// in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_line: Option<u32>,
    pub document: SavedDocument,
}

/// The record of visits in saved form: every place visited, newest first. No cursor: the
/// cursors are the tabs'.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    #[serde(default)]
    pub entries: Vec<SavedDocument>,
}

/// A [`crate::document::Document`] expressed in terms that survive a restart.
///
/// `object_name` is [`analysis::Object::name`] — the archive member name, or the file name for a
/// plain object — and is needed because one path can contribute many `Object`s, so `path`
/// alone is ambiguous. [`SavedDocument::Source`]'s `path` is written by [`any_path`] and
/// not as serde writes a `PathBuf`, which refuses one that is not UTF-8 and so would stop
/// the whole file being written for one tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedDocument {
    /// The whole of an object, shown one of the two ways it can be.
    Object {
        path: PathBuf,
        object_name: String,
        /// Which of the two ([`SavedShown`]): the path and the name say both, and this
        /// is all that tells them apart.
        shown: SavedShown,
    },
    Symbol {
        path: PathBuf,
        object_name: String,
        address: u64,
        symbol_name: SavedName,
    },
    Source {
        #[serde(with = "any_path")]
        path: PathBuf,
    },
}

/// A path as a project file spells it: its text where it is UTF-8, and otherwise, on Unix,
/// its bytes, which TOML writes as an array of numbers. Elsewhere such a path is written
/// lossily, and names no file when it is read back.
mod any_path {
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        match path.to_str() {
            Some(text) => serializer.serialize_str(text),
            #[cfg(unix)]
            None => {
                use std::os::unix::ffi::OsStrExt;
                serializer.collect_seq(path.as_os_str().as_bytes())
            }
            #[cfg(not(unix))]
            None => serializer.serialize_str(&path.to_string_lossy()),
        }
    }

    /// The two ways a path is written.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Spelled {
        Text(String),
        Bytes(Vec<u8>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        Ok(match Spelled::deserialize(deserializer)? {
            Spelled::Text(text) => PathBuf::from(text),
            #[cfg(unix)]
            Spelled::Bytes(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(bytes))
            }
            #[cfg(not(unix))]
            Spelled::Bytes(bytes) => PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()),
        })
    }
}

/// Which of the two ways the whole of an object is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedShown {
    /// The symbols it holds, which is what a binary's own tab lists.
    Symbols,
    /// All of its code, as one listing.
    Code,
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
///
/// The two are separate types because they are the two answers, and neither has the
/// other's half: a name the app made up has no string to save, and one the file stated
/// has no [`MadeUp`] to render.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedName {
    /// The file's own name, spelled as the file spells it.
    File(String),
    /// A name the app made up: which one, the spelling being a function of that and the
    /// address ([`SavedMadeUp`]).
    MadeUp(SavedMadeUp),
}

/// Which name the app made up: [`MadeUp`] without the address, which is saved beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SavedMadeUp {
    /// The app's name for the entry point.
    EntryPoint,
    /// The app's name for a function at the saved address.
    Function,
    /// The app's name for a fragment at the saved address.
    Fragment,
}

impl SavedMadeUp {
    /// The saved form of `made_up`. Every variant of [`MadeUp`] is named here, so a
    /// fourth made-up name is a compile error and not a spelling silently written to a
    /// file.
    fn of(made_up: MadeUp) -> SavedMadeUp {
        match made_up {
            MadeUp::EntryPoint => SavedMadeUp::EntryPoint,
            MadeUp::Function(_) => SavedMadeUp::Function,
            MadeUp::Fragment(_) => SavedMadeUp::Fragment,
        }
    }

    /// The name this is, borne by a symbol at `address`: the other half of
    /// [`SavedMadeUp::of`], and where the address the spelling needs comes back.
    fn at(self, address: SectionAddress) -> MadeUp {
        match self {
            SavedMadeUp::EntryPoint => MadeUp::EntryPoint,
            SavedMadeUp::Function => MadeUp::Function(address),
            SavedMadeUp::Fragment => MadeUp::Fragment(address),
        }
    }
}

impl SavedName {
    /// The saved form of `symbol`'s name.
    pub fn of(symbol: &SymbolData) -> SavedName {
        match symbol.made_up {
            None => SavedName::File(symbol.name.clone()),
            Some(made_up) => SavedName::MadeUp(SavedMadeUp::of(made_up)),
        }
    }

    /// The name a symbol at `address` carries now: the file's own as it was saved, or a
    /// made-up one spelled the way the app spells it today. What the symbol is looked up
    /// by.
    pub fn text(&self, address: SectionAddress) -> Cow<'_, str> {
        match self {
            SavedName::File(name) => Cow::Borrowed(name),
            SavedName::MadeUp(made_up) => Cow::Owned(made_up.at(address).to_string()),
        }
    }

    /// Which name the app made up, for a symbol at `address`; [`None`] where the name is
    /// the file's own. What says a saved place can spell itself without the file it
    /// points into being open.
    pub fn made_up(&self, address: SectionAddress) -> Option<MadeUp> {
        match self {
            SavedName::File(_) => None,
            SavedName::MadeUp(made_up) => Some(made_up.at(address)),
        }
    }
}

impl SavedDocument {
    /// Which of the three kinds of place this is: the same answer [`crate::document::Document::kind`]
    /// gives for the live one, so a bookmark wears the glyph its tab would.
    pub fn kind(&self) -> Kind {
        match self {
            SavedDocument::Object {
                shown: SavedShown::Code,
                ..
            } => Kind::Code,
            SavedDocument::Object { .. } | SavedDocument::Symbol { .. } => Kind::Binary,
            SavedDocument::Source { .. } => Kind::Source,
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
            } => symbol_name
                .made_up(SectionAddress::new(*address))
                .map(|name| name.to_string()),
            _ => None,
        }
    }

    /// The object this names: its file, and the name it is known in that file by.
    /// `None` for a source file, which names no binary. One question, so nothing that
    /// wants the object answers for a source file as well.
    pub(super) fn binary(&self) -> Option<(&Path, &str)> {
        match self {
            SavedDocument::Object {
                path, object_name, ..
            }
            | SavedDocument::Symbol {
                path, object_name, ..
            } => Some((path, object_name)),
            SavedDocument::Source { .. } => None,
        }
    }

    /// The binary this names, or `None` for a file.
    pub(super) fn binary_path(&self) -> Option<&Path> {
        self.binary().map(|(path, _)| path)
    }

    /// The same to write into: what [`Project::against`] rewrites, and `None` for a source
    /// file, whose path is what the debug information said rather than something this
    /// filesystem was asked about.
    fn binary_path_mut(&mut self) -> Option<&mut PathBuf> {
        match self {
            SavedDocument::Object { path, .. } | SavedDocument::Symbol { path, .. } => Some(path),
            SavedDocument::Source { .. } => None,
        }
    }
}

/// Where the session for the project at `path` is: beside it, under its whole name. The
/// name and not the stem, so the two files sort together and one ignore rule reaches both.
pub(super) fn session_beside(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(SESSION_EXTENSION);
    PathBuf::from(name)
}

// `pub(super)` so the rest of the module's tests build on the fixtures declared here.
/// No part of a row, which a saved entry leaves out.
fn is_zero(into: &u16) -> bool {
    *into == 0
}

#[cfg(test)]
pub(super) mod tests;

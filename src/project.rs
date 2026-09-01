//! Session persistence: the binaries that were open, the tabs they were open in, the
//! symbol that was selected and where the selection has been, written to a single TOML
//! file so a rerun of the app comes back where it left off.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it
//! can move into a crate of its own once the full project model of Step 8 arrives.
//!
//! Identity in the UI is `Arc` pointer identity, but pointers do not survive a restart,
//! so everything persisted here is identified by *path + names + address* instead. That
//! mapping lives in exactly two places: [`SavedSelection::from_selection`] going out and
//! [`SavedSelection::resolve`] coming back.
//!
//! The *when* of saving lives here too, in [`Saves`]: [`record`] is told the project the
//! app is now in and either writes it at once or marks it pending, and [`flush`] writes
//! whatever is pending. The app calls [`record`] from one observer of its state, [`flush`]
//! from a timer every [`AUTOSAVE_INTERVAL`], and [`flush`] once more when the window is
//! closed.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use analysis::{Object, Symbol};
use serde::{Deserialize, Serialize};

use crate::history::History;
use crate::tabs::Positions;

/// The directory this app keeps its state in, under the platform's state directory
/// (falling back to its local data directory).
const APP_DIR: &str = "assembly-viewer";
const FILE_NAME: &str = "project.toml";

/// What is currently selected in the UI.
///
/// Lives here rather than in `ui.rs` because it is plain data over the analysis types —
/// no freya involved — and both persistence directions need to speak it.
#[derive(Clone)]
pub enum Selection {
    None,
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
    /// file, members and all (`notes/Plan.md`, 6d).
    pub fn in_file(&self, path: &Path) -> bool {
        match self {
            Selection::None => false,
            Selection::Object(object) => object.path == path,
            Selection::Symbol(symbol) => symbol.object.path == path,
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Selection::None, Selection::None) => true,
            (Selection::Object(a), Selection::Object(b)) => Arc::ptr_eq(a, b),
            (Selection::Symbol(a), Selection::Symbol(b)) => a == b,
            _ => false,
        }
    }
}

/// The persisted session.
///
/// **The field order is load-bearing.** TOML has no way to reopen a table once a later
/// one has begun, so a serializer must emit every plain value of a table before the
/// first sub-table of it; a `Vec<PathBuf>` written after `selection` fails at runtime
/// with "values must be emitted before tables". The two plain values here — `binaries`,
/// an array of strings, and the `shown` index — therefore come first, and everything
/// table-valued follows: `selection` is a table, and `tabs`, `sources` and
/// `history.entries` are arrays of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The paths that were opened, deduplicated, in the order they were opened.
    pub binaries: Vec<PathBuf>,
    /// An index into `sources`, and `0` — meaning nothing — while it is empty, exactly
    /// as [`SavedHistory::cursor`] indexes its own entries.
    ///
    /// An index and not a path because the pane shows one *of the open files*: a path
    /// would be a second place for the name to be spelt and a second thing that could
    /// disagree with the list.
    #[serde(default)]
    pub shown: usize,
    /// `skip_serializing_if` because the `toml` crate cannot write a bare `None` at all
    /// — there is no null in TOML — so an unselected session has to leave the key out,
    /// and `default` to read that file back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<SavedSelection>,
    /// The content area's open tabs, in strip order.
    ///
    /// Which of them was active is not recorded here: `selection` already is it, so a
    /// field for it would be a second answer to the same question — the very thing
    /// `Tabs` refuses to hold in memory. The restore's only extra rule is an ordering,
    /// tabs before selection, and it lives at the call site.
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
    /// The Source pane's open files, in strip order.
    #[serde(default)]
    pub sources: Vec<SavedSource>,
    /// `serde(default)` so a partial file — one written by hand, or trimmed — loads with
    /// an empty history rather than failing and taking the binaries and the selection
    /// down with it. The three fields above carry it for the same reason.
    #[serde(default)]
    pub history: SavedHistory,
}

/// One of the content area's open tabs: a place, and the row the reader left it at.
///
/// The row travels *with* the tab it belongs to rather than in a list of its own beside
/// `Project::tabs`, and that is the whole of why this type exists. A parallel array of
/// rows would be a second list to keep in step with the first, and it could not survive
/// the one thing that certainly happens to the first: [`Project::resolve_tabs`] drops the
/// tabs that no longer resolve, which would silently shift every later row onto the wrong
/// tab.
///
/// The field order is load-bearing here too: `row` is a plain value and `selection` an
/// externally tagged enum, which TOML writes as a sub-table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    /// Which row was at the top of the assembly pane, `0` being the first instruction.
    ///
    /// A row and not a pixel offset, for [`crate::tabs::Positions`]' reasons — and one
    /// more that is only true of a saved one: `ROW_HEIGHT` is a compile-time constant of
    /// the UI, so a build that changes it would move every saved pixel offset while every
    /// saved row still names the instruction it named.
    ///
    /// `serde(default)` because it is a hint and not a fact: a hand-written or trimmed
    /// file that names a tab without saying where in it simply opens that tab at the top.
    #[serde(default)]
    pub row: usize,
    pub selection: SavedSelection,
}

/// One of the Source pane's open files: its path, and the row the reader left it at.
///
/// The path is a `String` rather than a `PathBuf` because it is what the debug info said
/// and not something this filesystem was asked about: it is the string `LineInfo` handed
/// the pane, it may well name the machine that compiled the binary rather than this one,
/// and writing it as a path would only invite `save_to`'s non-UTF-8 refusal on
/// a value that was UTF-8 all along.
///
/// A file that has since been deleted still comes back as a tab. The pane already draws
/// "Source file not found" for one, which is the true answer and a visible one; dropping
/// the tab instead would lose a file the reader had open without ever saying so. Nothing
/// here is therefore resolved against anything — unlike a [`SavedTab`], these come back
/// exactly as they went out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSource {
    /// Which line was at the top of the Source pane, `0` being the file's first line.
    /// A hint like [`SavedTab::row`] and defaulted for the same reason — and a file that
    /// has been edited shorter since is exactly what the clamp in
    /// [`crate::tabs::Positions::row`] is for.
    #[serde(default)]
    pub row: usize,
    pub path: String,
}

/// The navigation history in saved form: the index of the entry that was on screen, and
/// every visited selection, oldest first.
///
/// The field order is load-bearing for the same reason [`Project`]'s is: `entries` is a
/// `Vec` of externally tagged enums, so TOML writes it as an array of tables, and the
/// plain `cursor` has to be emitted before it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    /// An index into `entries`, and `0` — meaning nothing — while it is empty.
    pub cursor: usize,
    #[serde(default)]
    pub entries: Vec<SavedSelection>,
}

impl SavedHistory {
    /// The empty history, as a `const fn` so [`Saves`] can be a `static`.
    pub const fn new() -> SavedHistory {
        SavedHistory {
            cursor: 0,
            entries: Vec::new(),
        }
    }

    /// The saved form of `history`.
    ///
    /// [`History`] never holds a [`Selection::None`] — `push` refuses one and
    /// [`History::restored`] is only ever handed entries that resolved — so nothing is
    /// dropped here and the cursor stays pointing at the same entry.
    fn from_history(history: &History) -> SavedHistory {
        SavedHistory {
            cursor: history.cursor().unwrap_or(0),
            entries: history
                .entries()
                .iter()
                .filter_map(SavedSelection::from_selection)
                .collect(),
        }
    }
}

/// A [`Selection`] expressed in terms that survive a restart.
///
/// `object_name` is [`Object::name`] — the archive member name, or the file name for a
/// plain object — and is needed because one path can contribute many `Object`s (every
/// member of an archive, plus the file itself), so `path` alone is ambiguous.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SavedSelection {
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
}

impl SavedSelection {
    /// The saved form of `selection`, or `None` when nothing is selected.
    pub fn from_selection(selection: &Selection) -> Option<SavedSelection> {
        match selection {
            Selection::None => None,
            Selection::Object(object) => Some(SavedSelection::Object {
                path: object.path.clone(),
                object_name: object.name.clone(),
            }),
            Selection::Symbol(symbol) => Some(SavedSelection::Symbol {
                path: symbol.object.path.clone(),
                object_name: symbol.object.name.clone(),
                symbol_name: symbol.data.name.clone(),
                address: symbol.data.address,
            }),
        }
    }

    fn path(&self) -> &Path {
        match self {
            SavedSelection::Object { path, .. } | SavedSelection::Symbol { path, .. } => path,
        }
    }

    fn object_name(&self) -> &str {
        match self {
            SavedSelection::Object { object_name, .. }
            | SavedSelection::Symbol { object_name, .. } => object_name,
        }
    }

    /// The loaded object this names, if it is still there.
    fn find_object<'a>(&self, objects: &'a [Arc<Object>]) -> Option<&'a Arc<Object>> {
        objects
            .iter()
            .find(|object| object.path == self.path() && object.name == self.object_name())
    }

    /// Exactly what this names, or `None` when the object — or, for a symbol, the
    /// symbol — is no longer loaded.
    ///
    /// This is what history entries want: an entry that no longer points where it did
    /// is dropped, rather than quietly turned into a different destination that the
    /// user never visited.
    fn resolve(&self, objects: &[Arc<Object>]) -> Option<Selection> {
        let object = self.find_object(objects)?;
        match self {
            SavedSelection::Object { .. } => Some(Selection::Object(object.clone())),
            SavedSelection::Symbol {
                symbol_name,
                address,
                ..
            } => object
                .symbols_sorted
                .iter()
                .find(|data| data.name == *symbol_name && data.address == *address)
                .map(|data| {
                    Selection::Symbol(Symbol {
                        object: object.clone(),
                        data: data.clone(),
                    })
                }),
        }
    }

    /// The same, degrading instead of failing: a symbol that is gone falls back to its
    /// object and an object that is gone to nothing at all.
    ///
    /// This is what the *selection* wants. There is only one of it and it is where the
    /// app opens, so landing near the last session's place beats landing nowhere;
    /// a history entry, of which there are many, is better dropped.
    fn resolve_or_degrade(&self, objects: &[Arc<Object>]) -> Selection {
        self.resolve(objects).unwrap_or_else(|| {
            match self.find_object(objects) {
                Some(object) => Selection::Object(object.clone()),
                None => Selection::None,
            }
        })
    }
}

impl Project {
    /// The empty project, as a `const fn` so [`Saves`] can be a `static`.
    pub const fn new() -> Project {
        Project {
            binaries: Vec::new(),
            shown: 0,
            selection: None,
            tabs: Vec::new(),
            sources: Vec::new(),
            history: SavedHistory::new(),
        }
    }

    /// The project described by the state the app is currently in: the loaded `objects`,
    /// the content area's open `tabs` and the active one (`selection`), the `history`,
    /// and the Source pane's open `sources` with the one it is `shown` — each strip
    /// beside the `rows` its panes were left at.
    ///
    /// The one place the app's state is turned into what would be saved, so the save
    /// policy in [`Saves`] never has to know where any of it came from. It takes the two
    /// tab lists as plain slices rather than a `Tabs<T>` for exactly that reason: this is
    /// a mapping over what is open, not a party to how the lists are kept. The positions
    /// come as a [`Positions`] rather than a slice only because that is the shape of the
    /// question — "where was this tab left" — and a tab that was never scrolled has no
    /// entry in one at all, which is written out as row `0`.
    ///
    /// A `shown` naming a file that is not in `sources` cannot happen — `open_file` puts
    /// it there — and lands on `0` rather than being reported, because there is nothing
    /// for a saver to do about it and the restore clamps the same way.
    pub fn from_state(
        objects: &[Arc<Object>],
        tabs: &[Selection],
        tab_rows: &Positions<Selection>,
        selection: &Selection,
        history: &History,
        sources: &[Arc<str>],
        source_rows: &Positions<Arc<str>>,
        shown: Option<&str>,
    ) -> Project {
        let mut binaries: Vec<PathBuf> = Vec::new();
        for object in objects {
            if !binaries.contains(&object.path) {
                binaries.push(object.path.clone());
            }
        }
        Project {
            binaries,
            shown: shown
                .and_then(|file| sources.iter().position(|open| &**open == file))
                .unwrap_or(0),
            selection: SavedSelection::from_selection(selection),
            // `filter_map` for the same reason [`SavedHistory::from_history`] uses it and
            // with the same result: `Selection::None` is the app's placeholder state and
            // never a tab, so nothing is actually dropped here.
            tabs: tabs
                .iter()
                .filter_map(|tab| {
                    Some(SavedTab {
                        row: tab_rows.at(tab).unwrap_or(0),
                        selection: SavedSelection::from_selection(tab)?,
                    })
                })
                .collect(),
            sources: sources
                .iter()
                .map(|file| SavedSource {
                    row: source_rows.at(file).unwrap_or(0),
                    path: file.to_string(),
                })
                .collect(),
            history: SavedHistory::from_history(history),
        }
    }

    /// Turn the saved selection back into a live one against the objects that are now
    /// loaded. Binaries change between runs, so this degrades silently: a symbol that
    /// is gone falls back to its object, and an object that is gone to nothing at all.
    pub fn resolve(&self, objects: &[Arc<Object>]) -> Selection {
        match &self.selection {
            Some(saved) => saved.resolve_or_degrade(objects),
            None => Selection::None,
        }
    }

    /// Turn the saved tabs back into live selections against the objects that are now
    /// loaded, in strip order. A tab that no longer resolves is **dropped**, the way a
    /// history entry is and pointedly not the way the selection is.
    ///
    /// The selection degrades because there is one of it and the app has to open
    /// somewhere; a tab is one of many, and a strip whose chips lead to places that are
    /// no longer there — or, worse, that all degraded onto the same object and so
    /// collapsed into one chip — is worse than a shorter strip.
    ///
    /// Duplicates need no attention here: `Tabs::open` already refuses to open a second
    /// tab for something that is open, so two saved tabs that degrade onto one live
    /// selection could not both be opened even if they got this far.
    ///
    /// Each surviving tab comes back with the row it was left at, which is why the row is
    /// a field of the tab rather than a list beside it: the dropping here is exactly what
    /// a parallel array could not have survived.
    pub fn resolve_tabs(&self, objects: &[Arc<Object>]) -> Vec<(Selection, usize)> {
        self.tabs
            .iter()
            .filter_map(|saved| Some((saved.selection.resolve(objects)?, saved.row)))
            .collect()
    }

    /// The Source pane's open files with the row each was left at, in strip order.
    ///
    /// Nothing is resolved and nothing is dropped — see [`SavedSource`] — so this is only
    /// the change of type, done here rather than at the call site because the mapping
    /// between what is saved and what the app holds belongs to this module.
    pub fn resolve_sources(&self) -> Vec<(Arc<str>, usize)> {
        self.sources
            .iter()
            .map(|saved| (Arc::from(saved.path.as_str()), saved.row))
            .collect()
    }

    /// The source file the pane was showing, or `None` when it had none open.
    ///
    /// Clamped rather than trusted: a `shown` past the end of `sources` — hand-edited, or
    /// left behind by a trimmed list — falls back to the first open file, because "a file
    /// is shown exactly when one is open" is an invariant of the pane and "none of these"
    /// is not one of its states.
    pub fn shown_source(&self) -> Option<&str> {
        self.sources
            .get(self.shown)
            .or_else(|| self.sources.first())
            .map(|saved| saved.path.as_str())
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
    /// Duplicates are [`History::restored`]'s business, and neither this function's nor
    /// `rebuilt`'s: two saved entries naming the same destination resolve to the same
    /// `Arc` and so to equal entries, which a saved history written before entries were
    /// bumped rather than appended is full of.
    pub fn resolve_history(&self, objects: &[Arc<Object>]) -> History {
        History::rebuilt(
            self.history
                .entries
                .iter()
                .map(|saved| saved.resolve(objects)),
            self.history.cursor,
        )
    }

    /// The file the session is stored in, or `None` on a system with no state or local
    /// data directory to put it in.
    pub fn path() -> Option<PathBuf> {
        let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
        Some(base.join(APP_DIR).join(FILE_NAME))
    }

    /// Read the saved session. A missing, unreadable or corrupt file is simply `None`:
    /// this must never surface as an error to the user.
    pub fn load() -> Option<Project> {
        Project::load_from(&Project::path()?)
    }

    /// Write the session out, atomically. Any IO failure is logged and swallowed —
    /// failing to persist a session is never worth interrupting the user for.
    ///
    /// Private on purpose: everything goes through [`record`] and [`flush`], so the
    /// policy of *when* to write has no bypass.
    fn save(&self) {
        let Some(path) = Project::path() else {
            log::warn!("no state directory to save the project in");
            return;
        };
        if let Err(error) = self.save_to(&path) {
            log::warn!("could not save {}: {error}", path.display());
        }
    }

    fn load_from(path: &Path) -> Option<Project> {
        let data = fs::read_to_string(path).ok()?;
        toml::from_str(&data).ok()
    }

    /// Write `path` by writing `path.tmp` first and renaming it over the top, so an
    /// interrupted write cannot leave a half-written file behind.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }

        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);

        // TOML has no way to spell a path that is not UTF-8, and serde's `PathBuf`
        // impl fails rather than mangling one, so such a project is simply not written:
        // the error is turned into an IO error here and logged and swallowed by
        // `save`, which leaves the previous good file in place. Nothing panics, and
        // nothing lossy reaches the disk to be loaded back as a different path.
        let data = toml::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        fs::write(&temporary, data)?;
        fs::rename(&temporary, path)
    }
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

/// The save policy: what has been written, and what is waiting to be.
///
/// A `static` rather than UI state because two of the three things that drive it — the
/// periodic flush and the window's close hook — sit outside the component tree, and the
/// close hook cannot reach into it at all.
static SAVES: Mutex<Saves> = Mutex::new(Saves::new());

struct Saves {
    /// The project as last written.
    ///
    /// It starts as the *empty* project — deliberately not as the one loaded at startup.
    /// The state the app boots into equals this baseline, so nothing is ever pending
    /// before something is actually opened: a run in which nothing is opened, one whose
    /// restore finds no readable binary, and a flush that fires while the startup parse
    /// is still in flight all leave a good file on disk untouched. Seeding it from the
    /// loaded project would invert that, making the first comparison see the still-empty
    /// state as a change and write an empty project over a good one.
    written: Project,
    /// A newer project that has not been written yet; `None` when there is nothing to
    /// write.
    pending: Option<Project>,
}

impl Saves {
    const fn new() -> Saves {
        Saves {
            written: Project::new(),
            pending: None,
        }
    }

    /// The newest project this knows about, whether or not it reached the disk.
    fn latest(&self) -> &Project {
        self.pending.as_ref().unwrap_or(&self.written)
    }

    /// Take note of the state the app is now in. Hands back the project to write right
    /// now, or `None` when nothing changed or the change can wait for a [`Saves::flush`].
    ///
    /// Which binaries are open is a user project change: it is the result of a deliberate
    /// action, it is what every other part of the session is expressed against, and it is
    /// the one thing that is annoying to redo — so it goes to disk at once, carrying
    /// whatever selection and history were pending with it. A selection or a history
    /// entry only marks the project dirty.
    ///
    /// A tab is pending too, and deliberately so although opening one is every bit as
    /// deliberate an action as opening a binary. It fails the other two tests: a tab is
    /// expressed *against* the binaries rather than the other way round, and it costs one
    /// click to make again, where a lost binary costs a file dialog and a reparse. It
    /// also arrives far too often to write — `activate` opens a tab on the way to every
    /// selection change, so an immediate write here would put a file on the disk for
    /// every symbol the reader clicks, which is exactly the traffic the pending/flush
    /// split exists to collapse. Nothing in this function has to say so: `binaries` is
    /// all it looks at, and the new fields fall on the pending side by not being it.
    fn record(&mut self, project: Project) -> Option<Project> {
        if *self.latest() == project {
            return None;
        }

        if self.latest().binaries == project.binaries {
            self.pending = Some(project);
            return None;
        }

        self.written = project.clone();
        self.pending = None;
        Some(project)
    }

    /// Take whatever was recorded but not written, or `None` when the two already agree.
    fn flush(&mut self) -> Option<Project> {
        let project = self.pending.take()?;
        self.written = project.clone();
        Some(project)
    }
}

fn saves() -> MutexGuard<'static, Saves> {
    // Nothing under this lock can panic short of an allocation failure, but take the
    // state back rather than propagate if something ever does: a poisoned lock must not
    // turn a failed save into a crashed app.
    SAVES.lock().unwrap_or_else(|error| error.into_inner())
}

/// Take note of the project the app is now in, writing it out immediately if it is a
/// change that must not be lost and marking it pending otherwise.
///
/// Cheap enough to call on every state change: an unchanged project does nothing at all.
pub fn record(project: Project) {
    // The write happens under the lock, so two writes can never reach the file out of
    // the order they were decided in. Everything that calls this is on the main thread
    // today, so nothing ever waits on it.
    let mut saves = saves();
    if let Some(project) = saves.record(project) {
        project.save();
    }
}

/// Write out anything recorded but not yet written. A no-op when nothing has changed,
/// which is what makes it safe to call on a timer.
pub fn flush() {
    let mut saves = saves();
    if let Some(project) = saves.flush() {
        project.save();
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
            // The mapping never looks at the bytes; these objects were never parsed
            // from any.
            data: ObjectData::from(&b""[..]),
            dwarf: Default::default(),
        })
    }

    /// [`Project::from_state`] over a session whose only open tab is the selection, whose
    /// Source pane has nothing open and whose panes are at the top of what they show — the
    /// state the tests written before there were tabs to save were already describing, now
    /// spelt out.
    fn from_state(objects: &[Arc<Object>], selection: &Selection, history: &History) -> Project {
        Project::from_state(
            objects,
            std::slice::from_ref(selection),
            &Positions::default(),
            selection,
            history,
            &[],
            &Positions::default(),
            None,
        )
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
        assert!(!Selection::None.in_file(lib));
    }

    #[test]
    fn saves_and_resolves_a_symbol() {
        let objects = objects();
        let selection = Selection::Symbol(Symbol {
            object: objects[1].clone(),
            data: objects[1].symbols_sorted[0].clone(),
        });

        let project = from_state(&objects, &selection, &History::default());
        assert_eq!(project.binaries, vec![PathBuf::from("/tmp/lib.a")]);
        assert_eq!(
            project.selection,
            Some(SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "b.o".into(),
                symbol_name: "caller".into(),
                address: 0,
            })
        );

        // The duplicate `caller` in `a.o` must not win.
        assert!(project.resolve(&objects) == selection);
    }

    #[test]
    fn saves_and_resolves_an_object() {
        let objects = objects();
        let selection = Selection::Object(objects[0].clone());
        let project = from_state(&objects, &selection, &History::default());
        assert!(project.resolve(&objects) == selection);
    }

    #[test]
    fn no_selection_round_trips_as_none() {
        let objects = objects();
        let project = from_state(&objects, &Selection::None, &History::default());
        assert_eq!(project.selection, None);
        assert!(project.resolve(&objects) == Selection::None);
    }

    #[test]
    fn a_missing_symbol_falls_back_to_its_object() {
        let objects = objects();
        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a")],
            selection: Some(SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
                symbol_name: "gone".into(),
                address: 12,
            }),
            history: SavedHistory::default(),
            ..Project::new()
        };
        assert!(project.resolve(&objects) == Selection::Object(objects[0].clone()));
    }

    #[test]
    fn a_moved_symbol_falls_back_to_its_object() {
        let objects = objects();
        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a")],
            selection: Some(SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
                // Right name, recompiled to a different address.
                symbol_name: "target".into(),
                address: 999,
            }),
            history: SavedHistory::default(),
            ..Project::new()
        };
        assert!(project.resolve(&objects) == Selection::Object(objects[0].clone()));
    }

    #[test]
    fn a_missing_object_falls_back_to_nothing() {
        let objects = objects();
        for saved in [
            SavedSelection::Object {
                path: PathBuf::from("/tmp/other.a"),
                object_name: "a.o".into(),
            },
            // Right path, but that member is no longer in the archive.
            SavedSelection::Object {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "c.o".into(),
            },
            SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "c.o".into(),
                symbol_name: "caller".into(),
                address: 0,
            },
        ] {
            let project = Project {
                binaries: vec![PathBuf::from("/tmp/lib.a")],
                selection: Some(saved),
                history: SavedHistory::default(),
            ..Project::new()
            };
            assert!(project.resolve(&objects) == Selection::None);
        }
    }

    /// Serialize to TOML and read it straight back, which is the only way to catch the
    /// `toml` crate's runtime failures: a bare `None`, and a value emitted after a table.
    fn round_trip(project: &Project) -> String {
        let text = toml::to_string_pretty(project).expect("serializing");
        let back: Project = toml::from_str(&text).unwrap_or_else(|error| {
            panic!("deserializing\n--- {text}--- failed: {error}");
        });
        assert_eq!(*project, back);
        text
    }

    #[test]
    fn toml_round_trips() {
        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a"), PathBuf::from("/tmp/some.dll")],
            selection: Some(SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "b.o".into(),
                symbol_name: "caller".into(),
                address: 0x1234,
            }),
            history: SavedHistory::default(),
            ..Project::new()
        };
        let text = round_trip(&project);
        // The externally tagged enum is a table named after its variant.
        assert!(text.contains("[selection.Symbol]"), "{text}");
    }

    #[test]
    fn an_empty_project_round_trips() {
        // Nothing selected and nothing visited: the `None` the `toml` crate cannot write
        // has to be left out of the file entirely, and read back as `None`.
        let project = Project::new();
        let text = round_trip(&project);
        assert!(!text.contains("selection"), "{text}");
    }

    #[test]
    fn a_project_with_no_selection_but_open_binaries_round_trips() {
        let objects = objects();
        let project = from_state(&objects, &Selection::None, &History::default());
        assert_eq!(project.selection, None);
        let text = round_trip(&project);
        assert!(!text.contains("selection"), "{text}");
    }

    #[test]
    fn a_multi_entry_history_round_trips_as_an_array_of_tables() {
        let objects = objects();
        let project = from_state(&objects, &Selection::None, &history(&objects, 1));
        assert_eq!(project.history.entries.len(), 3);
        let text = round_trip(&project);
        assert!(text.contains("[[history.entries]]"), "{text}");
    }

    #[test]
    fn writes_atomically_and_reads_back() {
        let directory = std::env::temp_dir().join(format!(
            "assembly-viewer-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = directory.join("nested").join(FILE_NAME);

        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a")],
            selection: Some(SavedSelection::Object {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "a.o".into(),
            }),
            history: SavedHistory::default(),
            ..Project::new()
        };
        project.save_to(&path).expect("saving");

        assert_eq!(Project::load_from(&path), Some(project));
        // The temporary was renamed, not left behind.
        assert!(!path.with_extension("toml.tmp").exists());

        let _ = fs::remove_dir_all(&directory);
    }

    /// A history over the fixture objects: object `a.o`, then its `target` symbol, then
    /// object `b.o`, with the cursor wherever `back` calls leave it.
    fn history(objects: &[Arc<Object>], back: usize) -> History {
        let mut history = History::default();
        history.push(Selection::Object(objects[0].clone()));
        history.push(Selection::Symbol(Symbol {
            object: objects[0].clone(),
            data: objects[0].symbols_sorted[1].clone(),
        }));
        history.push(Selection::Object(objects[1].clone()));
        for _ in 0..back {
            history.back();
        }
        history
    }

    #[test]
    fn saves_and_restores_the_history() {
        let objects = objects();
        let history = history(&objects, 0);

        let project = from_state(&objects, &Selection::None, &history);
        assert_eq!(project.history.entries.len(), 3);
        assert_eq!(project.history.cursor, 2);

        let restored = project.resolve_history(&objects);
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

        let project = from_state(&objects, &Selection::None, &history);
        let restored = project.resolve_history(&objects);

        assert_eq!(restored.cursor(), Some(0));
        // The two entries in front of the cursor survived, so they are still there to
        // go forward to.
        assert!(restored.can_forward());
        assert!(!restored.can_back());
    }

    /// Building a saved history by hand, since these are entries no live `History`
    /// could have produced against these objects.
    fn saved_history(entries: &[SavedSelection], cursor: usize) -> Project {
        Project {
            binaries: vec![PathBuf::from("/tmp/lib.a")],
            selection: None,
            history: SavedHistory {
                entries: entries.to_vec(),
                cursor,
            },
            ..Project::new()
        }
    }

    fn saved_object(name: &str) -> SavedSelection {
        SavedSelection::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: name.to_owned(),
        }
    }

    #[test]
    fn history_entries_that_no_longer_resolve_are_dropped() {
        let objects = objects();
        let project = saved_history(
            &[
                saved_object("a.o"),
                // A member that is no longer in the archive.
                saved_object("c.o"),
                // The object is there but the symbol is gone. Unlike the selection,
                // which would degrade to the object, an entry is dropped: the user
                // never visited the object, and a list of places they did not go is
                // worse than a shorter list.
                SavedSelection::Symbol {
                    path: PathBuf::from("/tmp/lib.a"),
                    object_name: "a.o".into(),
                    symbol_name: "gone".into(),
                    address: 12,
                },
                saved_object("b.o"),
            ],
            3,
        );

        let restored = project.resolve_history(&objects);
        assert!(
            restored.entries()
                == [
                    Selection::Object(objects[0].clone()),
                    Selection::Object(objects[1].clone()),
                ]
        );
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
        let project = saved_history(
            &[
                saved_object("a.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            2,
        );

        let restored = project.resolve_history(&objects);
        assert!(
            restored.entries()
                == [
                    Selection::Object(objects[1].clone()),
                    Selection::Object(objects[0].clone()),
                ]
        );
        // The cursor was on the newest `a.o`, which is where the collapse left it.
        assert_eq!(restored.cursor(), Some(1));
        assert!(!restored.can_forward());
    }

    #[test]
    fn duplicates_collapse_around_the_entries_that_were_dropped() {
        let objects = objects();
        let project = saved_history(
            &[
                saved_object("a.o"),
                // Gone, so it is dropped before anything is collapsed.
                saved_object("c.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            3,
        );

        let restored = project.resolve_history(&objects);
        assert!(
            restored.entries()
                == [
                    Selection::Object(objects[1].clone()),
                    Selection::Object(objects[0].clone()),
                ]
        );
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn the_restored_cursor_follows_its_entry_through_the_collapse() {
        let objects = objects();
        // The cursor is on `b.o`, in the middle, and the collapse of the two `a.o`s
        // moves it to the front of the list.
        let project = saved_history(
            &[
                saved_object("a.o"),
                saved_object("b.o"),
                saved_object("a.o"),
            ],
            1,
        );

        let restored = project.resolve_history(&objects);
        assert!(restored.current() == Some(&Selection::Object(objects[1].clone())));
        assert_eq!(restored.cursor(), Some(0));
        // The newest `a.o` is still in front of it to go forward to.
        assert!(restored.can_forward());
        assert!(!restored.can_back());
    }

    #[test]
    fn two_saved_symbols_naming_the_same_one_restore_as_one_entry() {
        let objects = objects();
        let symbol = || SavedSelection::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            symbol_name: "target".into(),
            address: 6,
        };
        let project = saved_history(&[symbol(), saved_object("b.o"), symbol()], 2);

        // Both resolve through the same lookup to the same `Arc`, so they are equal
        // entries however far apart they were saved.
        let restored = project.resolve_history(&objects);
        assert!(
            restored.entries()
                == [
                    Selection::Object(objects[1].clone()),
                    Selection::Symbol(Symbol {
                        object: objects[0].clone(),
                        data: objects[0].symbols_sorted[1].clone(),
                    }),
                ]
        );
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn a_collapsed_cursor_entry_is_still_the_restored_selection() {
        let objects = objects();
        // Every cursor position over a saved history that holds a duplicate.
        for cursor in 0..3 {
            let mut project = saved_history(
                &[
                    saved_object("a.o"),
                    saved_object("b.o"),
                    saved_object("a.o"),
                ],
                cursor,
            );
            project.selection = Some(project.history.entries[cursor].clone());

            let restored_history = project.resolve_history(&objects);
            let restored_selection = project.resolve(&objects);

            assert!(restored_history.current() == Some(&restored_selection));
            assert!(!restored_history.would_push(&restored_selection));
        }
    }

    #[test]
    fn a_dropped_cursor_entry_falls_back_to_the_nearest_older_survivor() {
        let objects = objects();
        let project = saved_history(
            &[
                saved_object("a.o"),
                saved_object("c.o"),
                saved_object("b.o"),
            ],
            1,
        );

        let restored = project.resolve_history(&objects);
        assert!(restored.cursor() == Some(0));
        assert!(restored.current() == Some(&Selection::Object(objects[0].clone())));
        // `b.o` was in front of the cursor and still is.
        assert!(restored.can_forward());
    }

    #[test]
    fn a_cursor_with_no_older_survivor_lands_on_the_oldest_entry_left() {
        let objects = objects();
        let project = saved_history(&[saved_object("c.o"), saved_object("b.o")], 0);

        let restored = project.resolve_history(&objects);
        assert!(restored.cursor() == Some(0));
        assert!(restored.current() == Some(&Selection::Object(objects[1].clone())));
    }

    #[test]
    fn a_history_that_resolves_to_nothing_restores_as_empty() {
        let objects = objects();
        let project = saved_history(&[saved_object("c.o"), saved_object("d.o")], 1);

        let restored = project.resolve_history(&objects);
        assert!(restored.entries().is_empty());
        assert!(restored.cursor().is_none());
        assert!(!restored.can_back());
        assert!(!restored.can_forward());
    }

    #[test]
    fn a_hand_edited_cursor_past_the_end_is_clamped() {
        let objects = objects();
        let project = saved_history(&[saved_object("a.o"), saved_object("b.o")], 99);

        let restored = project.resolve_history(&objects);
        assert_eq!(restored.cursor(), Some(1));
    }

    #[test]
    fn the_restored_cursor_entry_is_the_restored_selection() {
        let objects = objects();

        // Every position the cursor can be in, including one the user walked back to.
        for back in 0..3 {
            let history = history(&objects, back);
            let selection = history.current().expect("a current entry").clone();
            let project = from_state(&objects, &selection, &history);

            let restored_history = project.resolve_history(&objects);
            let restored_selection = project.resolve(&objects);

            assert!(restored_history.current() == Some(&restored_selection));
            // Which is what keeps the recording effect from pushing a duplicate the
            // moment the restore sets the selection.
            assert!(!restored_history.would_push(&restored_selection));
        }
    }

    #[test]
    fn a_file_with_no_history_still_loads() {
        // Hand-written, or trimmed: `serde(default)` is what keeps the missing table
        // from taking the binaries and the selection down with it.
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [selection.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
        let project: Project = toml::from_str(text).expect("deserializing");

        assert_eq!(project.binaries, vec![PathBuf::from("/tmp/lib.a")]);
        assert_eq!(project.history, SavedHistory::new());

        // And it restores exactly as it would have: the selection back, no history.
        let objects = objects();
        assert!(project.resolve(&objects) == Selection::Object(objects[0].clone()));
        assert!(project.resolve_history(&objects).entries().is_empty());
    }

    #[test]
    fn a_history_with_no_entries_still_loads() {
        let text = r#"
            binaries = []

            [history]
            cursor = 0
        "#;
        let project: Project = toml::from_str(text).expect("deserializing");
        assert_eq!(project, Project::new());
    }

    #[test]
    fn a_non_utf8_path_is_not_written_rather_than_mangled() {
        // Only Unix has a `PathBuf` that can hold one at all.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;

            let project = Project {
                binaries: vec![PathBuf::from(std::ffi::OsStr::from_bytes(
                    b"/tmp/\xff\xfe.a",
                ))],
                selection: None,
                history: SavedHistory::new(),
                ..Project::new()
            };
            // An error, not a panic and not a lossy path silently written in its place.
            assert!(toml::to_string_pretty(&project).is_err());

            let directory = std::env::temp_dir().join(format!(
                "assembly-viewer-test-{}-{}",
                std::process::id(),
                line!()
            ));
            let path = directory.join(FILE_NAME);
            assert!(project.save_to(&path).is_err());
            // Nothing reached the disk, so a good earlier file would still be there.
            assert!(!path.exists());

            let _ = fs::remove_dir_all(&directory);
        }
    }

    #[test]
    fn the_history_round_trips_through_toml() {
        let objects = objects();
        let project = from_state(&objects, &Selection::None, &history(&objects, 1));
        round_trip(&project);
    }

    // --- the open tabs and the source files ---------------------------------

    /// The Source pane's strip, in the `Arc<str>` the UI holds it in.
    fn sources(files: &[&str]) -> Vec<Arc<str>> {
        files.iter().map(|file| Arc::from(*file)).collect()
    }

    /// Where the panes were left, in the map the UI keeps it in.
    fn positions<T: Clone + PartialEq>(at: &[(&T, usize)]) -> Positions<T> {
        let mut positions = Positions::default();
        for (tab, row) in at {
            positions.remember((*tab).clone(), *row);
        }
        positions
    }

    fn saved_tab(object_name: &str, row: usize) -> SavedTab {
        SavedTab {
            row,
            selection: saved_object(object_name),
        }
    }

    fn saved_source(path: &str, row: usize) -> SavedSource {
        SavedSource {
            row,
            path: path.to_owned(),
        }
    }

    /// A strip goes out in order and comes back in it, through the very mapping the
    /// history already uses — which is the whole reason a saved tab costs no new one.
    #[test]
    fn saves_and_resolves_the_open_tabs() {
        let objects = objects();
        let tabs = vec![
            Selection::Object(objects[0].clone()),
            Selection::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            }),
            Selection::Object(objects[1].clone()),
        ];

        let project = Project::from_state(
            &objects,
            &tabs,
            &Positions::default(),
            &tabs[2],
            &History::default(),
            &[],
            &Positions::default(),
            None,
        );

        assert_eq!(
            project.tabs,
            [
                saved_tab("a.o", 0),
                SavedTab {
                    row: 0,
                    selection: SavedSelection::Symbol {
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
            project.resolve_tabs(&objects)
                == [
                    (tabs[0].clone(), 0),
                    (tabs[1].clone(), 0),
                    (tabs[2].clone(), 0),
                ]
        );
    }

    /// The active tab is not written twice: it is the selection, and a saved session
    /// says which chip is on screen only by naming it there.
    #[test]
    fn the_active_tab_is_only_the_selection() {
        let objects = objects();
        let tabs = [Selection::Object(objects[0].clone())];
        let project = Project::from_state(
            &objects,
            &tabs,
            &Positions::default(),
            &tabs[0],
            &History::default(),
            &[],
            &Positions::default(),
            None,
        );

        assert_eq!(project.selection, Some(saved_object("a.o")));
        assert_eq!(project.tabs, [saved_tab("a.o", 0)]);
    }

    /// A tab is dropped exactly where the selection would degrade. There is one
    /// selection and the app has to open somewhere, but a strip whose chips lead to
    /// places that are no longer there is worse than a shorter strip — and degrading
    /// would be worse still, since two symbols of one object would degrade onto the
    /// same tab and `Tabs::open` would collapse them into one.
    ///
    /// The rows go with the tabs they belong to, which is the whole reason a row is a
    /// field of one: the second and third tabs here are dropped, and a parallel array of
    /// rows would have handed `b.o` the row of the tab that vanished before it.
    #[test]
    fn open_tabs_that_no_longer_resolve_are_dropped() {
        let objects = objects();
        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a")],
            tabs: vec![
                saved_tab("a.o", 3),
                // A member that is no longer in the archive.
                saved_tab("c.o", 4),
                // The object is still there; the symbol is not. The selection would
                // fall back to `a.o` here, and a tab must not.
                SavedTab {
                    row: 5,
                    selection: SavedSelection::Symbol {
                        path: PathBuf::from("/tmp/lib.a"),
                        object_name: "a.o".into(),
                        symbol_name: "gone".into(),
                        address: 12,
                    },
                },
                saved_tab("b.o", 6),
            ],
            ..Project::new()
        };

        assert!(
            project.resolve_tabs(&objects)
                == [
                    (Selection::Object(objects[0].clone()), 3),
                    (Selection::Object(objects[1].clone()), 6),
                ]
        );
    }

    /// Where each pane was left goes out with the tab it belongs to, and a tab that was
    /// never scrolled is written as the top rather than left out.
    #[test]
    fn saves_the_row_each_tab_was_left_at() {
        let objects = objects();
        let tabs = vec![
            Selection::Object(objects[0].clone()),
            Selection::Object(objects[1].clone()),
        ];
        let files = sources(&["/src/main.rs", "/src/lib.rs"]);

        let project = Project::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[1], 42)]),
            &tabs[0],
            &History::default(),
            &files,
            &positions(&[(&files[0], 7)]),
            Some("/src/main.rs"),
        );

        assert_eq!(project.tabs, [saved_tab("a.o", 0), saved_tab("b.o", 42)]);
        assert_eq!(
            project.sources,
            [
                saved_source("/src/main.rs", 7),
                saved_source("/src/lib.rs", 0)
            ]
        );
    }

    /// The round trip the app actually makes: out of the two maps, through TOML, and back
    /// into them the way `use_restore_on_startup` does it.
    #[test]
    fn the_rows_come_back_against_the_tabs_they_belong_to() {
        let objects = objects();
        let tabs = vec![
            Selection::Object(objects[0].clone()),
            Selection::Object(objects[1].clone()),
        ];
        let files = sources(&["/src/main.rs"]);

        let project = Project::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[0], 12), (&tabs[1], 900)]),
            &tabs[0],
            &History::default(),
            &files,
            &positions(&[(&files[0], 4)]),
            Some("/src/main.rs"),
        );
        let project: Project = toml::from_str(&round_trip(&project)).expect("reading back");

        let mut restored: Positions<Selection> = Positions::default();
        for (tab, row) in project.resolve_tabs(&objects) {
            restored.remember(tab, row);
        }
        assert_eq!(restored.at(&tabs[0]), Some(12));
        assert_eq!(restored.at(&tabs[1]), Some(900));
        // And a hint it is: a listing that has since shrunk clamps to what it holds now.
        assert_eq!(restored.row(&tabs[1], 100), 99);

        assert_eq!(project.resolve_sources(), [(files[0].clone(), 4)]);
    }

    /// A row is a hint and not a fact, so a saved tab that does not name one is a tab at
    /// the top rather than a file that will not load.
    #[test]
    fn a_saved_tab_with_no_row_opens_at_the_top() {
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [[tabs]]
            [tabs.selection.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"

            [[sources]]
            path = "/src/main.rs"
        "#;
        let project: Project = toml::from_str(text).expect("deserializing");

        let objects = objects();
        assert!(project.resolve_tabs(&objects) == [(Selection::Object(objects[0].clone()), 0)]);
        assert_eq!(project.resolve_sources(), [(Arc::from("/src/main.rs"), 0)]);
    }

    #[test]
    fn saves_and_restores_the_source_files() {
        let objects = objects();
        let files = sources(&["/src/main.rs", "/src/lib.rs"]);
        let project = Project::from_state(
            &objects,
            &[],
            &Positions::default(),
            &Selection::None,
            &History::default(),
            &files,
            &Positions::default(),
            Some("/src/lib.rs"),
        );

        assert_eq!(
            project.sources,
            [
                saved_source("/src/main.rs", 0),
                saved_source("/src/lib.rs", 0)
            ]
        );
        assert_eq!(project.shown, 1);
        assert_eq!(project.shown_source(), Some("/src/lib.rs"));
    }

    /// Nothing about a source file is resolved against this filesystem, on purpose:
    /// the pane's own "Source file not found" is the right answer for one that has been
    /// deleted, and dropping the tab would lose a file the reader had open without ever
    /// saying so.
    #[test]
    fn a_source_file_that_is_no_longer_there_still_comes_back() {
        let path = "/no/such/directory/gone.rs";
        assert!(!Path::new(path).exists());

        let project = Project::from_state(
            &objects(),
            &[],
            &Positions::default(),
            &Selection::None,
            &History::default(),
            &sources(&[path]),
            &Positions::default(),
            Some(path),
        );

        assert_eq!(project.sources, [saved_source(path, 0)]);
        assert_eq!(project.shown_source(), Some(path));
    }

    #[test]
    fn a_shown_index_past_the_end_falls_back_to_the_first_open_file() {
        // Hand-edited, or left behind by a trimmed list. "A file is shown exactly when
        // one is open" leaves no room for a fourth answer, so this clamps rather than
        // reporting nothing.
        let project = Project {
            sources: vec![
                saved_source("/src/main.rs", 0),
                saved_source("/src/lib.rs", 0),
            ],
            shown: 99,
            ..Project::new()
        };
        assert_eq!(project.shown_source(), Some("/src/main.rs"));
    }

    #[test]
    fn no_open_source_files_shows_nothing() {
        assert_eq!(Project::new().shown_source(), None);

        // A file open but none shown cannot happen; `from_state` writes index 0 for it
        // and it reads back as the first of them, not as a state of its own.
        let project = Project::from_state(
            &objects(),
            &[],
            &Positions::default(),
            &Selection::None,
            &History::default(),
            &sources(&["/src/main.rs"]),
            &Positions::default(),
            None,
        );
        assert_eq!(project.shown, 0);
        assert_eq!(project.shown_source(), Some("/src/main.rs"));
    }

    /// The field-order trap, which only a real serialization catches: `binaries` and the
    /// `shown` index are the plain values and have to reach the file before the first
    /// table opens, and a saved tab's own `row` before the `selection` sub-table under
    /// it. A project with every field set at once is the one that fails when they do not.
    #[test]
    fn a_full_project_round_trips_through_toml() {
        let objects = objects();
        let tabs = vec![
            Selection::Object(objects[0].clone()),
            Selection::Symbol(Symbol {
                object: objects[0].clone(),
                data: objects[0].symbols_sorted[1].clone(),
            }),
        ];
        let files = sources(&["/src/main.rs", "/src/lib.rs"]);
        let project = Project::from_state(
            &objects,
            &tabs,
            &positions(&[(&tabs[0], 12), (&tabs[1], 34)]),
            &tabs[1],
            &history(&objects, 1),
            &files,
            &positions(&[(&files[1], 56)]),
            Some("/src/lib.rs"),
        );

        let text = round_trip(&project);
        assert!(text.contains("[[tabs]]"), "{text}");
        assert!(text.contains("[[sources]]"), "{text}");

        let first_table = text.find("\n[").expect("a table");
        for plain in ["binaries = ", "shown = "] {
            let at = text
                .find(plain)
                .unwrap_or_else(|| panic!("{plain}\n{text}"));
            assert!(at < first_table, "{plain} after a table\n{text}");
        }
        // And inside a tab, the row before the table its selection is written as.
        let row = text.find("row = 12").expect("the first tab's row");
        let selection = text
            .find("[tabs.selection")
            .expect("the first tab's selection");
        assert!(row < selection, "row after its selection\n{text}");
    }

    #[test]
    fn a_file_with_no_tabs_or_source_files_still_loads() {
        // The `serde(default)`s, from the other side: a hand-written or trimmed file is
        // a session with an empty strip rather than a load failure.
        let text = r#"
            binaries = ["/tmp/lib.a"]

            [selection.Object]
            path = "/tmp/lib.a"
            object_name = "a.o"
        "#;
        let project: Project = toml::from_str(text).expect("deserializing");

        assert!(project.tabs.is_empty());
        assert!(project.sources.is_empty());
        assert_eq!(project.shown, 0);
        assert_eq!(project.shown_source(), None);

        // And the selection still restores, opening its own tab through `activate` the
        // way a session saved before there were tabs to save would.
        let objects = objects();
        assert!(project.resolve(&objects) == Selection::Object(objects[0].clone()));
        assert!(project.resolve_tabs(&objects).is_empty());
    }

    // --- the save policy ---------------------------------------------------

    fn project_with(binaries: &[&str], selection: Option<&str>) -> Project {
        Project {
            binaries: binaries.iter().map(PathBuf::from).collect(),
            selection: selection.map(saved_object),
            history: SavedHistory::new(),
            ..Project::new()
        }
    }

    #[test]
    fn the_state_the_app_boots_into_is_never_written() {
        let mut saves = Saves::new();
        // The save observer runs once on mount, before anything is restored, and this
        // is what it records. Nothing may come of it: the file on disk is the good one.
        assert_eq!(saves.record(Project::new()), None);
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn opening_a_binary_is_written_at_once() {
        let mut saves = Saves::new();
        let project = project_with(&["/tmp/lib.a"], None);

        assert_eq!(saves.record(project.clone()), Some(project));
        // And is not written a second time by the next flush.
        assert_eq!(saves.flush(), None);
    }

    /// Closing one takes the same path opening one does, which is the whole of what
    /// makes 6d's "the save is immediate" true: `binaries` is what `record` looks at,
    /// and it does not care in which direction the list changed.
    #[test]
    fn closing_a_binary_is_written_at_once() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a", "/tmp/some.dll"], Some("a.o")));

        // The selection is still pending from the open above; closing writes the lot,
        // so the file on disk never names a binary the app no longer has open.
        let project = project_with(&["/tmp/lib.a"], Some("a.o"));
        assert_eq!(saves.record(project.clone()), Some(project));
        assert_eq!(saves.flush(), None);
    }

    /// Closing the last one is not "nothing changed": the empty project is a project,
    /// and it has to reach the disk or the next run reopens what was just closed.
    #[test]
    fn closing_the_only_binary_is_written_too() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], Some("a.o")));

        let project = Project::new();
        assert_eq!(saves.record(project.clone()), Some(project));
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn a_selection_change_waits_for_the_flush() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], None));

        let project = project_with(&["/tmp/lib.a"], Some("a.o"));
        assert_eq!(saves.record(project.clone()), None);
        assert_eq!(saves.flush(), Some(project));
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn recording_the_same_project_again_changes_nothing() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], None));

        // A pending change re-recorded unchanged, as the save observer does whenever
        // something it does not persist wakes it.
        let project = project_with(&["/tmp/lib.a"], Some("a.o"));
        saves.record(project.clone());
        assert_eq!(saves.record(project.clone()), None);
        // Still pending, and still exactly one write.
        assert_eq!(saves.flush(), Some(project.clone()));
        assert_eq!(saves.flush(), None);

        // And once written, re-recording it is not a second write either.
        assert_eq!(saves.record(project), None);
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn opening_a_binary_carries_the_pending_change_with_it() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], Some("a.o")));

        // The selection is pending; opening a second binary writes the lot.
        let project = project_with(&["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));
        assert_eq!(saves.record(project.clone()), Some(project));
        assert_eq!(saves.flush(), None);
    }


    /// A tab is pending and not an immediate write, and nothing in `record` says so:
    /// `binaries` is all it compares, so the new fields fall on the pending side by not
    /// being it. That is the answer wanted — `activate` opens a tab on the way to every
    /// selection change, so an immediate write here would be one file per click.
    #[test]
    fn opening_a_tab_waits_for_the_flush() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], None));

        let mut project = project_with(&["/tmp/lib.a"], Some("a.o"));
        project.tabs = vec![saved_tab("a.o", 0)];
        assert_eq!(saves.record(project.clone()), None);
        assert_eq!(saves.flush(), Some(project));
        assert_eq!(saves.flush(), None);
    }

    /// And so is a source file, by the same route: the pane opens one whenever the
    /// selection lands on a symbol with line info.
    #[test]
    fn opening_a_source_file_waits_for_the_flush() {
        let mut saves = Saves::new();
        saves.record(project_with(&["/tmp/lib.a"], None));

        let mut project = project_with(&["/tmp/lib.a"], None);
        project.sources = vec![
            saved_source("/src/main.rs", 0),
            saved_source("/src/lib.rs", 0),
        ];
        project.shown = 1;
        assert_eq!(saves.record(project.clone()), None);
        assert_eq!(saves.flush(), Some(project));
        assert_eq!(saves.flush(), None);
    }

    /// Closing a binary still writes at once, and now carries the tabs it closed with
    /// it: they were pending, and `record` takes everything pending along with the
    /// binaries change, so the file on disk never names a tab into a binary the app has
    /// already let go of.
    #[test]
    fn closing_a_binary_carries_the_tabs_it_closed_with_it() {
        let mut saves = Saves::new();
        let mut opened = project_with(&["/tmp/lib.a", "/tmp/some.dll"], Some("a.o"));
        opened.tabs = vec![saved_tab("a.o", 0)];
        saves.record(opened);

        let closed = project_with(&["/tmp/lib.a"], None);
        assert_eq!(saves.record(closed.clone()), Some(closed));
        assert_eq!(saves.flush(), None);
    }

    #[test]
    fn a_missing_or_corrupt_file_is_none() {
        let directory = std::env::temp_dir().join(format!(
            "assembly-viewer-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = directory.join(FILE_NAME);

        assert_eq!(Project::load_from(&path), None);

        fs::create_dir_all(&directory).expect("creating the test directory");
        fs::write(&path, b"{ not toml").expect("writing the corrupt file");
        assert_eq!(Project::load_from(&path), None);

        let _ = fs::remove_dir_all(&directory);
    }
}

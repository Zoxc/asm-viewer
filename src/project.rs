//! Session persistence: the binaries that were open, the symbol that was selected and
//! where the selection has been, written to a single JSON file so a rerun of the app
//! comes back where it left off.
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

/// The directory this app keeps its state in, under the platform's state directory
/// (falling back to its local data directory).
const APP_DIR: &str = "assembly-viewer";
const FILE_NAME: &str = "project.json";

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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// The paths that were opened, deduplicated, in the order they were opened.
    pub binaries: Vec<PathBuf>,
    pub selection: Option<SavedSelection>,
    /// `serde(default)` because this arrived after the first `project.json` files did:
    /// a file written before it exists loads with an empty history rather than failing
    /// and taking the binaries and the selection down with it.
    #[serde(default)]
    pub history: SavedHistory,
}

/// The navigation history in saved form: every visited selection, oldest first, and the
/// index of the one that was on screen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHistory {
    pub entries: Vec<SavedSelection>,
    /// An index into `entries`, and `0` — meaning nothing — while it is empty.
    pub cursor: usize,
}

impl SavedHistory {
    /// The empty history, as a `const fn` so [`Saves`] can be a `static`.
    pub const fn new() -> SavedHistory {
        SavedHistory {
            entries: Vec::new(),
            cursor: 0,
        }
    }

    /// The saved form of `history`.
    ///
    /// [`History`] never holds a [`Selection::None`] — `push` refuses one and
    /// [`History::restored`] is only ever handed entries that resolved — so nothing is
    /// dropped here and the cursor stays pointing at the same entry.
    fn from_history(history: &History) -> SavedHistory {
        SavedHistory {
            entries: history
                .entries()
                .iter()
                .filter_map(SavedSelection::from_selection)
                .collect(),
            cursor: history.cursor().unwrap_or(0),
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
            selection: None,
            history: SavedHistory::new(),
        }
    }

    /// The project described by the currently loaded `objects`, `selection` and
    /// `history`. The binaries are the objects' paths, deduplicated, in order.
    ///
    /// The one place the app's state is turned into what would be saved, so the save
    /// policy in [`Saves`] never has to know where any of it came from.
    pub fn from_state(
        objects: &[Arc<Object>],
        selection: &Selection,
        history: &History,
    ) -> Project {
        let mut binaries: Vec<PathBuf> = Vec::new();
        for object in objects {
            if !binaries.contains(&object.path) {
                binaries.push(object.path.clone());
            }
        }
        Project {
            binaries,
            selection: SavedSelection::from_selection(selection),
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

    /// Turn the saved history back into a live one against the objects that are now
    /// loaded. An entry that no longer resolves is dropped, so a session whose binaries
    /// have changed comes back with the entries that still mean something rather than
    /// with none at all.
    ///
    /// The cursor follows the drops: walking the entries up to and including the saved
    /// cursor, it is left on the last one that survived. So it stays on the saved entry
    /// when that entry survived, falls back to the nearest older survivor when it did
    /// not, and to the oldest surviving entry when nothing older survived either.
    pub fn resolve_history(&self, objects: &[Arc<Object>]) -> History {
        let mut entries = Vec::new();
        let mut cursor = 0;

        for (index, saved) in self.history.entries.iter().enumerate() {
            let Some(entry) = saved.resolve(objects) else {
                continue;
            };
            if index <= self.history.cursor {
                cursor = entries.len();
            }
            entries.push(entry);
        }

        History::restored(entries, cursor)
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
        let data = fs::read(path).ok()?;
        serde_json::from_slice(&data).ok()
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

        let data = serde_json::to_vec_pretty(self)?;
        fs::write(&temporary, data)?;
        fs::rename(&temporary, path)
    }
}

/// How often [`flush`] is worth calling.
///
/// The write is a few hundred bytes of JSON, so the cost of a tick is a comparison that
/// almost always finds nothing pending. Five seconds is far coarser than the rate a user
/// clicks through symbols at — a burst of navigation collapses into one write — while
/// bounding what an unclean exit can lose to five seconds of history and one selection,
/// neither of which is expensive to redo. A clean window close flushes anyway.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(5);

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

    use analysis::{BinaryFormat, Section, SymbolData};

    use super::*;

    /// A bare `Object` with the given text symbols. The analysis crate's own fixtures
    /// go through `parse_object`; here only the fields the mapping reads matter, and
    /// every one of them is public, so the objects are built directly.
    fn object(path: &str, name: &str, symbols: &[(&str, u64)]) -> Arc<Object> {
        let section = Arc::new(Section {
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
            symbols: HashMap::new(),
            symbols_sorted,
            sections: vec![section],
        })
    }

    fn objects() -> Vec<Arc<Object>> {
        vec![
            object("/tmp/lib.a", "a.o", &[("caller", 0), ("target", 6)]),
            // Same path, different member: `path` alone cannot tell these apart.
            object("/tmp/lib.a", "b.o", &[("caller", 0)]),
        ]
    }

    #[test]
    fn saves_and_resolves_a_symbol() {
        let objects = objects();
        let selection = Selection::Symbol(Symbol {
            object: objects[1].clone(),
            data: objects[1].symbols_sorted[0].clone(),
        });

        let project = Project::from_state(&objects, &selection, &History::default());
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
        let project = Project::from_state(&objects, &selection, &History::default());
        assert!(project.resolve(&objects) == selection);
    }

    #[test]
    fn no_selection_round_trips_as_none() {
        let objects = objects();
        let project = Project::from_state(&objects, &Selection::None, &History::default());
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
            };
            assert!(project.resolve(&objects) == Selection::None);
        }
    }

    #[test]
    fn json_round_trips() {
        let project = Project {
            binaries: vec![PathBuf::from("/tmp/lib.a"), PathBuf::from("/tmp/some.dll")],
            selection: Some(SavedSelection::Symbol {
                path: PathBuf::from("/tmp/lib.a"),
                object_name: "b.o".into(),
                symbol_name: "caller".into(),
                address: 0x1234,
            }),
            history: SavedHistory::default(),
        };
        let json = serde_json::to_string(&project).expect("serializing");
        let back: Project = serde_json::from_str(&json).expect("deserializing");
        assert_eq!(project, back);
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
        };
        project.save_to(&path).expect("saving");

        assert_eq!(Project::load_from(&path), Some(project));
        // The temporary was renamed, not left behind.
        assert!(!path.with_extension("json.tmp").exists());

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

        let project = Project::from_state(&objects, &Selection::None, &history);
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

        let project = Project::from_state(&objects, &Selection::None, &history);
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
            let project = Project::from_state(&objects, &selection, &history);

            let restored_history = project.resolve_history(&objects);
            let restored_selection = project.resolve(&objects);

            assert!(restored_history.current() == Some(&restored_selection));
            // Which is what keeps the recording effect from pushing a duplicate the
            // moment the restore sets the selection.
            assert!(!restored_history.would_push(&restored_selection));
        }
    }

    #[test]
    fn a_file_written_before_the_history_existed_still_loads() {
        let json = r#"{
            "binaries": ["/tmp/lib.a"],
            "selection": { "Object": { "path": "/tmp/lib.a", "object_name": "a.o" } }
        }"#;
        let project: Project = serde_json::from_str(json).expect("deserializing");

        assert_eq!(project.binaries, vec![PathBuf::from("/tmp/lib.a")]);
        assert_eq!(project.history, SavedHistory::new());

        // And it restores exactly as it did before: the selection back, no history.
        let objects = objects();
        assert!(project.resolve(&objects) == Selection::Object(objects[0].clone()));
        assert!(project.resolve_history(&objects).entries().is_empty());
    }

    #[test]
    fn the_history_round_trips_through_json() {
        let objects = objects();
        let project = Project::from_state(&objects, &Selection::None, &history(&objects, 1));
        let json = serde_json::to_string(&project).expect("serializing");
        assert_eq!(
            serde_json::from_str::<Project>(&json).expect("deserializing"),
            project
        );
    }

    // --- the save policy ---------------------------------------------------

    fn project_with(binaries: &[&str], selection: Option<&str>) -> Project {
        Project {
            binaries: binaries.iter().map(PathBuf::from).collect(),
            selection: selection.map(saved_object),
            history: SavedHistory::new(),
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
        fs::write(&path, b"{ not json").expect("writing the corrupt file");
        assert_eq!(Project::load_from(&path), None);

        let _ = fs::remove_dir_all(&directory);
    }
}

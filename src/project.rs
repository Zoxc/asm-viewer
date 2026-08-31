//! Session persistence: the binaries that were open and the symbol that was selected,
//! written to a single JSON file so a rerun of the app comes back where it left off.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it
//! can move into a crate of its own once the full project model of Step 8 arrives.
//!
//! Identity in the UI is `Arc` pointer identity, but pointers do not survive a restart,
//! so everything persisted here is identified by *path + names + address* instead. That
//! mapping lives in exactly two places: [`SavedSelection::from_selection`] going out and
//! [`Project::resolve`] coming back.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{Object, Symbol};
use serde::{Deserialize, Serialize};

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
}

impl Project {
    /// The project described by the currently loaded `objects` and `selection`. The
    /// binaries are the objects' paths, deduplicated, in order.
    pub fn from_state(objects: &[Arc<Object>], selection: &Selection) -> Project {
        let mut binaries: Vec<PathBuf> = Vec::new();
        for object in objects {
            if !binaries.contains(&object.path) {
                binaries.push(object.path.clone());
            }
        }
        Project {
            binaries,
            selection: SavedSelection::from_selection(selection),
        }
    }

    /// Turn the saved selection back into a live one against the objects that are now
    /// loaded. Binaries change between runs, so this degrades silently: a symbol that
    /// is gone falls back to its object, and an object that is gone to nothing at all.
    // Saving (Step 2b) only goes the other way; this direction is Step 2c's.
    #[allow(dead_code)]
    pub fn resolve(&self, objects: &[Arc<Object>]) -> Selection {
        let Some(saved) = &self.selection else {
            return Selection::None;
        };

        let Some(object) = objects
            .iter()
            .find(|object| object.path == saved.path() && object.name == saved.object_name())
        else {
            return Selection::None;
        };

        match saved {
            SavedSelection::Object { .. } => Selection::Object(object.clone()),
            SavedSelection::Symbol {
                symbol_name,
                address,
                ..
            } => match object
                .symbols_sorted
                .iter()
                .find(|data| data.name == *symbol_name && data.address == *address)
            {
                Some(data) => Selection::Symbol(Symbol {
                    object: object.clone(),
                    data: data.clone(),
                }),
                None => Selection::Object(object.clone()),
            },
        }
    }

    /// The file the session is stored in, or `None` on a system with no state or local
    /// data directory to put it in.
    pub fn path() -> Option<PathBuf> {
        let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
        Some(base.join(APP_DIR).join(FILE_NAME))
    }

    /// Read the saved session. A missing, unreadable or corrupt file is simply `None`:
    /// this must never surface as an error to the user.
    // Only the tests read a session back so far; Step 2c is what restores one at startup.
    #[allow(dead_code)]
    pub fn load() -> Option<Project> {
        Project::load_from(&Project::path()?)
    }

    /// Write the session out, atomically. Any IO failure is logged and swallowed —
    /// failing to persist a session is never worth interrupting the user for.
    pub fn save(&self) {
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

        let project = Project::from_state(&objects, &selection);
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
        let project = Project::from_state(&objects, &selection);
        assert!(project.resolve(&objects) == selection);
    }

    #[test]
    fn no_selection_round_trips_as_none() {
        let objects = objects();
        let project = Project::from_state(&objects, &Selection::None);
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
        };
        project.save_to(&path).expect("saving");

        assert_eq!(Project::load_from(&path), Some(project));
        // The temporary was renamed, not left behind.
        assert!(!path.with_extension("json.tmp").exists());

        let _ = fs::remove_dir_all(&directory);
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

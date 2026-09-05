//! The shape the Objects list is drawn in: the files that were opened, and the objects
//! each of them contributed. Framework-free.
//!
//! [`ObjectTree`] groups objects into the **consecutive runs** sharing a [`Object::path`] —
//! runs rather than a map keyed by path, so the rows keep the order the files were opened
//! in. One file opened twice therefore folds into one row over both copies. A file that
//! contributed exactly one object is its own row and grows no parent. [`Loads`] is the
//! other half: the files being read right now, which have a row before they have an object.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{BinaryFormat, Object};

use crate::filter::Matcher;

/// Which load asked for a file. A counter and not a path, because the same path can be
/// loading twice — a file closed and reopened mid-parse is two loads, and the first one's
/// objects must not arrive into the second's row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoadId(u64);

/// The files being read and parsed right now, one entry per (load, path).
///
/// **Cancelling is by path and never by load**: closing a file is `close_binary`'s business
/// and its unit is the path. Leaving a project is [`Loads::clear`].
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Loads {
    entries: Vec<(LoadId, PathBuf)>,
    next: u64,
}

impl Loads {
    /// Register `paths` as being read, and hand back the id the answers will be checked
    /// against. Called *before* the work starts, so the row is on screen from the click.
    pub fn begin(&mut self, paths: &[PathBuf]) -> LoadId {
        let id = LoadId(self.next);
        self.next += 1;
        self.entries
            .extend(paths.iter().map(|path| (id, path.clone())));
        id
    }

    /// This load has nothing more to say about `path`.
    pub fn finished(&mut self, id: LoadId, path: &Path) {
        self.entries
            .retain(|(entry, loading)| *entry != id || loading != path);
    }

    /// Whether this load's answers about `path` are still wanted.
    pub fn holds(&self, id: LoadId, path: &Path) -> bool {
        self.entries
            .iter()
            .any(|(entry, loading)| *entry == id && loading == path)
    }

    /// Whether this load has any path left at all, which is what tells the worker feeding
    /// it to stop rather than to skip one answer.
    pub fn active(&self, id: LoadId) -> bool {
        self.entries.iter().any(|(entry, _)| *entry == id)
    }

    /// Whether nothing is being read at all, which is what the save policy asks: a list
    /// of binaries still filling in is not the list the app holds.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether anything is still producing objects for `path`, which is what a row draws.
    pub fn is_loading(&self, path: &Path) -> bool {
        self.entries.iter().any(|(_, loading)| loading == path)
    }

    /// Stop reading `path`, whoever asked for it.
    pub fn cancel(&mut self, path: &Path) {
        self.entries.retain(|(_, loading)| loading != path);
    }

    /// Stop everything, which is a project being left.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The paths still being read, in the order they were asked for and without repeats:
    /// one file is one row however many loads are producing it.
    pub fn paths(&self) -> Vec<&Path> {
        let mut paths: Vec<&Path> = Vec::new();
        for (_, path) in &self.entries {
            if !paths.contains(&path.as_path()) {
                paths.push(path);
            }
        }
        paths
    }
}

/// Whether a file row's members are on screen, and whether the reader decided that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expansion {
    Collapsed,
    Expanded,
    /// Held open by the filter rather than by the reader, because the file matched only
    /// through its members. A third state and not a `true` in the expansion set: the set is
    /// what the reader asked for and outlives the filter, and a forced row draws no
    /// disclosure triangle, since folding it would hide the matches it is pointing at.
    Forced,
}

/// One row of the flattened objects list. Flattened because a `VirtualScrollView` is told a
/// length and asked for row *n*: the tree is a shape in the data, never in the element tree.
#[derive(Clone)]
pub enum TreeRow {
    /// A file its members fold under. Not an [`Object`] itself: an `.a`/`.lib` does not
    /// parse as one, so this row has a path and a count and nothing to select.
    File {
        /// The file's name, without its directory.
        name: String,
        /// The whole path, which is what the row's tooltip says.
        path: PathBuf,
        /// The group's identity and the key the expansion set holds: the pointer of the
        /// first object the file contributed. [`None`] for a file that has contributed
        /// nothing yet, which is exactly the row that can never be folded.
        group: Option<usize>,
        /// How many objects are under this row *now*, which under a filter is how many
        /// of them matched.
        members: usize,
        expansion: Expansion,
        /// Whether more objects may still arrive out of this file.
        loading: bool,
    },
    /// One object: an archive member indented under its file, or a file that contributed
    /// exactly one object and so is a row of its own.
    Object { object: Arc<Object>, member: bool },
}

/// The rows the Objects list draws, in order. Built in a memo and shared by an `Arc`,
/// compared by that pointer.
#[derive(Clone)]
pub struct ObjectTree(Arc<Vec<TreeRow>>);

impl PartialEq for ObjectTree {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ObjectTree {
    /// Group `objects` by the file they came from, drop what the filter does not match,
    /// and flatten what is left into rows.
    ///
    /// A file row is never hidden while a row under it is visible, so a file is shown when
    /// its own name matches *or* any member's does, and the two differ:
    ///
    /// - The **file's name matched**: every member is under it, folded the way the reader
    ///   left it.
    /// - Only **members matched**: only those members are under it, and it is held open
    ///   ([`Expansion::Forced`]).
    /// - **Neither**, and the file is not there at all.
    ///
    /// Matching is on the name each row shows, so the directory is not read.
    ///
    /// **A file still being read is always a file row**, whatever it has contributed so
    /// far: "one object is its own row" needs to know the one is all there will be, and a
    /// row that promoted itself to a parent as the second member landed would move the
    /// list under a reader already reading it.
    pub fn new(
        objects: &[Arc<Object>],
        loads: &Loads,
        matcher: &Matcher,
        expanded: &HashSet<usize>,
    ) -> Self {
        // Nothing may be forced open while the filter is asking nothing.
        let filtering = !matches!(matcher, Matcher::Everything);
        let mut rows = Vec::new();
        let mut rest = objects;

        while let Some(first) = rest.first() {
            let count = rest.iter().take_while(|o| o.path == first.path).count();
            let (group, tail) = rest.split_at(count);
            rest = tail;

            let loading = loads.is_loading(&first.path);

            if let ([object], false) = (group, loading) {
                if matcher.matches(&object.name) {
                    rows.push(TreeRow::Object {
                        object: object.clone(),
                        member: false,
                    });
                }
                continue;
            }

            let name = file_name(&first.path);
            let whole = matcher.matches(&name);
            let members: Vec<&Arc<Object>> = group
                .iter()
                .filter(|object| whole || matcher.matches(&object.name))
                .collect();
            if members.is_empty() {
                continue;
            }

            let group = Arc::as_ptr(first).addr();
            let expansion = if filtering && !whole {
                Expansion::Forced
            } else if expanded.contains(&group) {
                Expansion::Expanded
            } else {
                Expansion::Collapsed
            };

            rows.push(TreeRow::File {
                name,
                path: first.path.clone(),
                group: Some(group),
                members: members.len(),
                expansion,
                loading,
            });

            if expansion != Expansion::Collapsed {
                rows.extend(members.into_iter().map(|object| TreeRow::Object {
                    object: object.clone(),
                    member: true,
                }));
            }
        }

        // The files that have produced nothing yet cannot come out of the walk above, which
        // is over objects. Appended rather than interleaved: there is no object to place
        // them next to, and a file's row moves into the walk above once its first one
        // lands. Only the file's own name is matched, there being no members yet.
        for path in loads.paths() {
            if objects.iter().any(|object| object.path == path) {
                continue;
            }
            let name = file_name(path);
            if !matcher.matches(&name) {
                continue;
            }
            rows.push(TreeRow::File {
                name,
                path: path.to_path_buf(),
                group: None,
                members: 0,
                expansion: Expansion::Collapsed,
                loading: true,
            });
        }

        ObjectTree(Arc::new(rows))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn row(&self, index: usize) -> &TreeRow {
        &self.0[index]
    }

    #[cfg(test)]
    fn rows(&self) -> &[TreeRow] {
        &self.0
    }
}

/// What a file is called without its directory, falling back to the whole path when it has
/// no file name at all — a path ending in `..`, say, which must not come out empty.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The short tag a row wears to say what kind of file it is. Text and not an icon: nothing
/// in Lucide's set names an object file format.
pub fn format_tag(format: BinaryFormat) -> &'static str {
    match format {
        BinaryFormat::Elf => "ELF",
        BinaryFormat::Pe => "PE",
        BinaryFormat::Coff => "COFF",
        BinaryFormat::MachO => "MACH",
        BinaryFormat::Wasm => "WASM",
        BinaryFormat::Xcoff => "XCOF",
        // `BinaryFormat` is `#[non_exhaustive]`, so a format this build has never heard of
        // is still a row that has to say something.
        _ => "OBJ",
    }
}

/// The tag on a file row: the archive holding them is a format `object` does not parse and
/// so has no `BinaryFormat`.
pub const ARCHIVE_TAG: &str = "AR";

#[cfg(test)]
mod tests;

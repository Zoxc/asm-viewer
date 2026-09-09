//! The shape the Objects list is drawn in: the files that were opened, and the objects
//! each of them contributed. Framework-free.
//!
//! [`ObjectTree`] groups objects into the **consecutive runs** sharing a [`Object::path`] —
//! runs rather than a map keyed by path, so the rows keep the order the files were opened
//! in. One file opened twice therefore folds into one row over both copies. The run is the
//! writer's to keep: an object is put after the last one of its own file, so two loads
//! arriving at once cannot split a file in two. A file that contributed exactly one object
//! is its own row and grows no parent. [`Loads`] is the other half: the files being read
//! right now, which have a row before they have an object.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{BinaryFormat, Object};

use crate::filter::Matcher;
use crate::shared::Shared;
use crate::source;

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

/// Whether the app holds `path` already: an object read from it is in the list, or a load
/// of it is still on its way. The two halves are one question -- a file is in the app from
/// the moment it is asked for, not from the moment its first object lands -- and opening a
/// path a second time would put a second copy of each of its objects in the list.
///
/// What a Files row's menu turns on (Close file or Open file) and what an artefact row's
/// press asks before it starts a load.
pub fn holds(objects: &[Arc<Object>], loads: &Loads, path: &Path) -> bool {
    objects_have(objects, path) || loads.is_loading(path)
}

/// Whether any object in the list came out of `path`.
fn objects_have(objects: &[Arc<Object>], path: &Path) -> bool {
    objects.iter().any(|object| object.path == path)
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
        /// first object the file contributed.
        group: usize,
        /// How many objects are under this row *now*, which under a filter is how many
        /// of them matched.
        members: usize,
        expansion: Expansion,
        /// Whether more objects may still arrive out of this file.
        loading: bool,
    },
    /// A file being read that has contributed nothing yet: a row so the reader can see it
    /// was opened, with nothing under it to fold and no format until it has been parsed.
    Pending {
        /// The file's name, without its directory.
        name: String,
        /// The whole path, which is what the row's tooltip says.
        path: PathBuf,
    },
    /// One object: an archive member indented under its file, or a file that contributed
    /// exactly one object and so is a row of its own.
    Object { object: Arc<Object>, member: bool },
}

/// The rows the Objects list draws, in order.
pub type ObjectTree = Shared<TreeRow>;

impl ObjectTree {
    /// Group `objects` by the file they came from, drop what the filter does not match,
    /// and flatten what is left into rows: the files that have produced objects, in the
    /// order their objects are in, and then the files still working on their first.
    ///
    /// Matching is on the name each row shows, so the directory is not read.
    pub fn new(
        objects: &[Arc<Object>],
        loads: &Loads,
        matcher: &Matcher,
        expanded: &HashSet<usize>,
    ) -> Self {
        let mut rows = opened(objects, loads, matcher, expanded);
        rows.extend(pending(objects, loads, matcher));
        rows.into()
    }
}

/// The rows for the files that have produced objects, one file's run of them at a time.
fn opened(
    objects: &[Arc<Object>],
    loads: &Loads,
    matcher: &Matcher,
    expanded: &HashSet<usize>,
) -> Vec<TreeRow> {
    // Nothing may be forced open while the filter is asking nothing.
    let filtering = !matches!(matcher, Matcher::Everything);
    objects
        .chunk_by(|a, b| a.path == b.path)
        .flat_map(|group| file(group, loads, matcher, expanded, filtering))
        .collect()
}

/// The rows one file's `group` of objects makes: none, if the filter kept none of them;
/// the object alone, if that is all the file will ever contribute; a file row otherwise,
/// with what the filter kept under it unless the row is folded.
///
/// A file row is never hidden while a row under it is visible, so a file is shown when its
/// own name matches *or* any member's does, and the two differ:
///
/// - The **file's name matched**: every member is under it, folded the way the reader left
///   it.
/// - Only **members matched**: only those members are under it, and it is held open
///   ([`Expansion::Forced`]).
/// - **Neither**, and the file is not there at all.
///
/// **A file still being read is always a file row**, even at one object: "one object is its
/// own row" needs to know the one is all there will be, and a row that promoted itself to a
/// parent as the second member landed would move the list under a reader already reading
/// it.
fn file(
    group: &[Arc<Object>],
    loads: &Loads,
    matcher: &Matcher,
    expanded: &HashSet<usize>,
    filtering: bool,
) -> Vec<TreeRow> {
    let first = &group[0];
    let loading = loads.is_loading(&first.path);

    if let ([object], false) = (group, loading) {
        if !matcher.matches(&object.name) {
            return Vec::new();
        }
        return vec![TreeRow::Object {
            object: object.clone(),
            member: false,
        }];
    }

    let name = source::name_of(&first.path);
    let whole = matcher.matches(&name);
    let members: Vec<&Arc<Object>> = group
        .iter()
        .filter(|object| whole || matcher.matches(&object.name))
        .collect();
    if members.is_empty() {
        return Vec::new();
    }

    let key = Arc::as_ptr(first).addr();
    let expansion = if filtering && !whole {
        Expansion::Forced
    } else if expanded.contains(&key) {
        Expansion::Expanded
    } else {
        Expansion::Collapsed
    };

    let mut rows = vec![TreeRow::File {
        name,
        path: first.path.clone(),
        group: key,
        members: members.len(),
        expansion,
        loading,
    }];

    if expansion != Expansion::Collapsed {
        rows.extend(members.into_iter().map(|object| TreeRow::Object {
            object: object.clone(),
            member: true,
        }));
    }

    rows
}

/// The rows for the files being read that have produced nothing yet
/// ([`TreeRow::Pending`]), in the order they were asked for.
///
/// They cannot come out of [`opened`], which is a walk over objects, and they go after it
/// rather than among it: there is no object to place them next to, and a file's row moves
/// into that walk once its first one lands. Only the file's own name is matched, there
/// being no members yet.
fn pending(objects: &[Arc<Object>], loads: &Loads, matcher: &Matcher) -> Vec<TreeRow> {
    loads
        .paths()
        .into_iter()
        .filter(|path| !objects_have(objects, path))
        .filter_map(|path| {
            let name = source::name_of(path);
            matcher.matches(&name).then(|| TreeRow::Pending {
                name,
                path: path.to_path_buf(),
            })
        })
        .collect()
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

//! The shape the Objects list is drawn in: the files that were opened, and the objects
//! each of them contributed. Framework-free.
//!
//! [`ObjectTree`] groups objects into the **consecutive runs** sharing a [`Object::path`] —
//! runs rather than a map keyed by path, so the rows keep the order the files were opened
//! in. One file opened twice therefore folds into one row over both copies. The run is the
//! writer's to keep: an object is put after the last one of its own file, so two loads
//! arriving at once cannot split a file in two ([`place`]). A file that contributed exactly
//! one object is its own row and grows no parent. A file asked for is in the list at once,
//! as a [placeholder](Object::placeholder) its first object replaces. [`Loads`] is the
//! other half: the files being read right now.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{BinaryFormat, ByteSlice, Object, Severity};

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
    /// The first id the last [`Loads::clear`] did not stop: every load below it was begun
    /// for a project since left.
    left: u64,
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

    /// This load has nothing more to say about any path: its worker is gone, whether or
    /// not it said so for each of them.
    pub fn end(&mut self, id: LoadId) {
        self.entries.retain(|(entry, _)| *entry != id);
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

    /// Whether nothing is being read at all, which is what the save policy asks: the
    /// session has no tabs until a restore's load is over.
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
        self.left = self.next;
    }

    /// Whether this load was begun for a project since left. Not the same as no longer
    /// [`Loads::active`]: a load also ends when it finishes or its files are closed, and
    /// what waits on it then is still about the project on screen.
    pub fn left(&self, id: LoadId) -> bool {
        id.0 < self.left
    }
}

/// Whether the app holds `path` already: an object read from it is in the list, or the
/// placeholder of a load still on its way. Opening a path a second time would put a second
/// copy of each of its objects in the list.
///
/// What a Files row's menu turns on (Close file or Open file) and what an artefact row's
/// press asks before it starts a load.
pub fn holds(objects: &[Arc<Object>], path: &Path) -> bool {
    objects.iter().any(|object| object.path == path)
}

/// Put `object` into `objects`: in place of its file's placeholder where there is one, and
/// at [`slot`] otherwise.
pub fn place(objects: &mut Vec<Arc<Object>>, object: Arc<Object>, order: &[PathBuf]) {
    let placeholder = objects
        .iter()
        .position(|held| held.path == object.path && held.is_placeholder());
    match placeholder {
        Some(at) => objects[at] = object,
        None => {
            let at = slot(objects, &object.path, order);
            objects.insert(at, object);
        }
    }
}

/// Where an object read from `path` goes in `objects`.
///
/// After the last object of its own file, so a file stays one run however loads
/// interleave. A file's first object -- its placeholder -- goes before the first object of
/// a file `order` lists after it, or of one it does not list at all, since that was opened
/// later; with no such object, or a `path` that `order` does not list, it goes at the end.
/// `order` is the binaries as they were listed when a load closed them to read them again,
/// so a rebuilt file goes back to its own place in the list; any other load hands in an
/// empty one.
pub fn slot(objects: &[Arc<Object>], path: &Path, order: &[PathBuf]) -> usize {
    if let Some(last) = objects.iter().rposition(|held| held.path == path) {
        return last + 1;
    }
    let rank = |path: &Path| order.iter().position(|listed| listed == path);
    let Some(own) = rank(path) else {
        return objects.len();
    };
    objects
        .iter()
        .position(|held| rank(&held.path).is_none_or(|other| other > own))
        .unwrap_or(objects.len())
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
        /// The whole path, which is what the row's tooltip says, and the key the expansion
        /// set holds: the tree makes one row per path.
        path: PathBuf,
        /// How many objects are under this row *now*, which under a filter is how many
        /// of them matched.
        members: usize,
        expansion: Expansion,
        /// Whether more objects may still arrive out of this file.
        loading: bool,
        /// The worst of what went wrong reading the objects counted in `members`
        /// ([`Object::worst`]), so a folded file still shows that one of them is wrong.
        worst: Option<Severity>,
    },
    /// A file whose [placeholder](Object::placeholder) no object has replaced yet: a row so
    /// the reader can see it was opened, with nothing under it to fold and no format until
    /// it has been parsed.
    Pending {
        /// The file's name, without its directory.
        name: String,
        /// The whole path, which is what the row's tooltip says.
        path: PathBuf,
        /// Whether it is still being read. Not where its load ended without an answer:
        /// its worker would not start, or died.
        loading: bool,
    },
    /// One object: an archive member indented under its file, or a file that contributed
    /// exactly one object and so is a row of its own.
    Object { object: Arc<Object>, member: bool },
}

/// The rows the Objects list draws, in order.
pub type ObjectTree = Shared<TreeRow>;

impl ObjectTree {
    /// Group `objects` by the file they came from, drop what the filter does not match,
    /// and flatten what is left into rows, one file's run of objects at a time, in the
    /// order they are in.
    ///
    /// Matching is on the name each row shows, so the directory is not read.
    pub fn new(
        objects: &[Arc<Object>],
        loads: &Loads,
        matcher: &Matcher,
        expanded: &HashSet<PathBuf>,
    ) -> Self {
        opened(objects, loads, matcher, expanded).into()
    }
}

/// The rows of every file in `objects`.
fn opened(
    objects: &[Arc<Object>],
    loads: &Loads,
    matcher: &Matcher,
    expanded: &HashSet<PathBuf>,
) -> Vec<TreeRow> {
    // Nothing may be forced open while the filter is asking nothing.
    let filtering = !matches!(matcher, Matcher::Everything);
    objects
        .chunk_by(|a, b| a.path == b.path)
        .flat_map(|group| file(group, loads, matcher, expanded, filtering))
        .collect()
}

/// The rows one file's `group` of objects makes: none, if the filter kept none of them;
/// a pending row for a placeholder; the object alone, if that is all the file will ever
/// contribute; a file row otherwise, with what the filter kept under it unless the row is
/// folded.
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
    expanded: &HashSet<PathBuf>,
    filtering: bool,
) -> Vec<TreeRow> {
    let first = &group[0];
    let loading = loads.is_loading(&first.path);

    // A placeholder has no members, so only the file's own name is matched.
    if first.is_placeholder() {
        let name = source::name_of(&first.path);
        if !matcher.matches(&name) {
            return Vec::new();
        }
        return vec![TreeRow::Pending {
            name,
            path: first.path.clone(),
            loading,
        }];
    }

    if let ([object], false) = (group, loading) {
        if !matcher.matches(&object.name.to_str_lossy()) {
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
        .filter(|object| whole || matcher.matches(&object.name.to_str_lossy()))
        .collect();
    if members.is_empty() {
        return Vec::new();
    }

    let expansion = if filtering && !whole {
        Expansion::Forced
    } else if expanded.contains(&first.path) {
        Expansion::Expanded
    } else {
        Expansion::Collapsed
    };

    let mut rows = vec![TreeRow::File {
        name,
        path: first.path.clone(),
        members: members.len(),
        expansion,
        loading,
        worst: members.iter().filter_map(|object| object.worst()).max(),
    }];

    if expansion != Expansion::Collapsed {
        rows.extend(members.into_iter().map(|object| TreeRow::Object {
            object: object.clone(),
            member: true,
        }));
    }

    rows
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

/// The tag a row wears for what `object` is: its format's, the archive's for an archive
/// none of whose members is shown, and a question mark for a file of no known kind, which
/// is shown only to say why it shows nothing.
pub fn object_tag(object: &Object) -> &'static str {
    match object.format {
        Some(format) => format_tag(format),
        None if object.is_archive() => ARCHIVE_TAG,
        None => "?",
    }
}

#[cfg(test)]
mod tests;

//! The shape the Objects list is drawn in: the files that were opened, and the objects
//! each of them contributed.
//!
//! This module is **framework-free** — no freya types appear here — for the reason
//! `filter.rs` and `history.rs` are: it can move into a crate alongside the rest of the
//! non-UI code, and the rules below can be asserted without mounting a UI.
//!
//! The list was flat until now, but the model under it never was. One *file* contributes
//! one [`Object`] and an archive contributes one per member (`analysis`'s parse pipeline),
//! so opening `libanalysis-sample.rlib` put 196 sibling rows in the sidebar with nothing
//! on screen saying they were one file. [`ObjectTree`] groups them back: the rows are the
//! consecutive runs of objects sharing a [`Object::path`] — consecutive because
//! `open_files` emits a file's objects together, and runs rather than a map keyed by path
//! so that the rows keep the order the files were opened in rather than a hash order.
//! Opening one file twice therefore folds into one row holding both copies of its
//! members, the two runs being adjacent; that is the objects list holding a file twice
//! showing through, and the row still says truthfully how many objects are under it.
//!
//! A file is also in this list *before* it has contributed anything. `open_files_streaming`
//! hands objects over as they are parsed, so between the moment a file is asked for and
//! the moment its last member lands there is a row with nothing behind it yet -- which is
//! the state `notes/Goals.md` asks for an indicator for, and which nothing could be in
//! while the parse handed back one `Vec` at the end. [`Loads`] is that half of the model:
//! the files being read right now, which is a list only the app can keep (the crate is
//! told the paths and hands back objects; it has no opinion about what a reader is
//! looking at meanwhile).
//!
//! A file that contributed exactly **one** object is its own row and grows no parent: the
//! parent would be named after the same file, carry the same tooltip, and fold away a
//! single child. That rule is about the count and not about the archive-ness, so a
//! one-member archive collapses to one row named after its member — nothing is lost, the
//! path is still in the row's tooltip, and the alternative is a disclosure triangle that
//! never has more than one thing behind it.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{BinaryFormat, Object};

use crate::filter::Matcher;

/// Which load asked for a file, so that one abandoned load cannot speak for another.
///
/// A plain counter and not a path, because the same path can be loading twice -- a reader
/// who opens a file, closes it and opens it again before the first parse has finished has
/// two loads of it, and the first one's objects must not arrive into the second's row.
/// Compare this with the analysis worker (`ui.rs`, `use_analysis`), which deliberately
/// has *no* counter: an answer there is about a `Symbol` that already existed and so
/// carries its own identity, while a load is about work that has not produced anything to
/// be identified by.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoadId(u64);

/// The files being read and parsed right now.
///
/// One entry per (load, path) rather than per path, and a `Vec` rather than a set,
/// because both questions asked of it are ordered ones: the rows are drawn in the order
/// the files were asked for, and an arriving object has to be checked against the load
/// that produced it rather than against the path alone.
///
/// **Cancelling is by path and never by load**, which is not an oversight: closing a file
/// is `close_binary`'s business (in `ui.rs`) and its unit is the path, so a path that is
/// closed stops loading however many loads happened to be producing it. Leaving *this*
/// project is `clear`, which is every load at once.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Loads {
    entries: Vec<(LoadId, PathBuf)>,
    /// The next id to hand out. Per-state rather than a process-wide counter: two states
    /// never compare ids with each other, and a test wants to start from zero.
    next: u64,
}

impl Loads {
    /// Register `paths` as being read, and hand back the id the answers will be checked
    /// against.
    ///
    /// Called by whoever is about to start the work and *before* it starts, so the row
    /// saying "this file is being read" is on screen from the click rather than from the
    /// first byte read -- which for a 331 MB file is a second and a half later.
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

    /// Whether this load's answers about `path` are still wanted. `false` for a file that
    /// has since been closed, and for one whose project has been left.
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

    /// Whether anything is still producing objects for `path`, which is what a row draws.
    pub fn is_loading(&self, path: &Path) -> bool {
        self.entries.iter().any(|(_, loading)| loading == path)
    }

    /// Stop reading `path`, whoever asked for it. See the type's note on why the unit is
    /// the path.
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
    /// Opened by the filter rather than by the reader, because this file matched only
    /// through its members: the rows under it *are* the search results, and a search that
    /// leaves its own results folded away has answered nothing.
    ///
    /// A third state rather than a `true` written into the expansion set, because the two
    /// differ in what a click can do. The set is what the reader asked for and outlives
    /// the filter; while a filter is holding a file open, folding it would hide the
    /// matches the filter is pointing at, so the row draws no disclosure triangle at all
    /// and nothing invites the click. The triangle comes back when the filter is cleared,
    /// on whichever side of it the reader had left the file.
    Forced,
}

/// One row of the flattened objects list.
///
/// Flattened because a `VirtualScrollView` is told a length and asked for row *n*: the
/// tree is a shape in the data and never in the element tree.
#[derive(Clone)]
pub enum TreeRow {
    /// A file that contributed more than one object — an archive — and the row its
    /// members fold under. It is not an [`Object`] itself: an `.a`/`.lib` does not parse
    /// as one, so this row has a path and a count and nothing to select.
    ///
    /// It is also what a file being read is drawn as before anything has come out of it,
    /// and what one stays as while it is still being read even if only one object has:
    /// the alternative is a top-level object row that turns into a parent under the
    /// reader the moment a second member lands.
    File {
        /// What the row is called: the file's name, without its directory.
        name: String,
        /// The whole path, which is what the row's tooltip says.
        path: PathBuf,
        /// The group's identity, and the key the expansion set holds. The pointer of the
        /// first object the file contributed, which is `Arc` pointer identity the way the
        /// rest of the UI keys things — a path would collide with the same file opened
        /// twice, and an index would move under the reader as files are opened.
        ///
        /// [`None`] for a file that has contributed nothing yet: there is no object to
        /// point at, and there is equally nothing to fold, so the row that has no key is
        /// exactly the row that never needs one.
        group: Option<usize>,
        /// How many objects are under this row *now*, which under a filter is how many
        /// of them matched.
        members: usize,
        expansion: Expansion,
        /// Whether more objects may still arrive out of this file. It is the indicator
        /// `notes/Goals.md` asks for, and it is a property of the **file** rather than of
        /// an object because an object that has not been parsed does not exist: the unit
        /// that is part-way through is the one the reader opened, the one `close_binary`
        /// closes, and the one that already has a row.
        loading: bool,
    },
    /// One object: an archive member indented under its file, or a file that contributed
    /// exactly one object and so is a row of its own.
    Object { object: Arc<Object>, member: bool },
}

/// The rows the Objects list draws, in order.
///
/// Built in a memo over the objects, the filter and the expansion set — never per row —
/// and shared by an `Arc` so that handing it to a scroll view costs a pointer. Compared
/// by that pointer for the reason every other `Arc` in the UI is: a `Vec` of rows holding
/// `Arc<Object>`s has no meaningful structural equality.
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
    /// **What a match on a member does to its parent**, which is the part worth stating:
    /// a file row is never hidden while a row under it is visible, because a member
    /// indented under nothing is not a tree. So a file is shown when its own name matches
    /// *or* any of its members' names do, and the two cases differ in what comes with it:
    ///
    /// - The **file's name matched**, so the whole file is the answer: every member is
    ///   under it, and whether they are on screen is left to whatever the reader had the
    ///   row folded to. The result being pointed at is the file row itself, so opening it
    ///   would bury it under 196 members it did not ask about.
    /// - Only **members matched**, so those members are the answer: the file is drawn as
    ///   the context they hang under, only the matching ones are under it, and it is held
    ///   open ([`Expansion::Forced`]) whatever the reader had it folded to.
    /// - **Neither**, and the file is not there at all.
    ///
    /// Matching is on the name each row shows — the file's own name, not the directory
    /// above it, and the member's name — for the reason the symbol list matches on the
    /// demangled name: a filter whose effect cannot be seen on screen is not one.
    /// **A file still being read is always a file row**, whatever it has contributed so
    /// far — nothing, one object or fifty. The "one object is its own row" rule needs to
    /// know that the one is all there will be, which is exactly what is not known yet,
    /// and a row that promoted itself to a parent as the second member landed would move
    /// the list under a reader who is already reading it.
    pub fn new(
        objects: &[Arc<Object>],
        loads: &Loads,
        matcher: &Matcher,
        expanded: &HashSet<usize>,
    ) -> Self {
        // Whether the filter is asking anything at all. Nothing may be forced open while
        // it is not: an untouched list is exactly the list, folded the way it was left.
        let filtering = !matches!(matcher, Matcher::Everything);
        let mut rows = Vec::new();
        let mut rest = objects;

        while let Some(first) = rest.first() {
            let count = rest.iter().take_while(|o| o.path == first.path).count();
            let (group, tail) = rest.split_at(count);
            rest = tail;

            let loading = loads.is_loading(&first.path);

            // One object from this file: it *is* the row. See the module comment — and
            // only once the file is done with, since "one" is not yet an answer while
            // more may be coming.
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

        // The files that have been asked for and have produced nothing yet. They cannot
        // come out of the walk above, which is over objects, and they are the whole
        // difference between a window that says it is working and one that sits empty for
        // the second and a half it takes to read 331 MB. Appended rather than interleaved:
        // there is no object to place them next to, and a file's row moves into the walk
        // above the moment its first one lands.
        //
        // The filter reads the file's name and nothing else here, there being no member
        // names to match yet. A file nothing matches is simply not there, exactly as a
        // loaded one is not.
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

    /// How many rows there are, which is what the `VirtualScrollView` is given.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The row at `index`, which the scroll view only ever asks for in range.
    pub fn row(&self, index: usize) -> &TreeRow {
        &self.0[index]
    }

    #[cfg(test)]
    fn rows(&self) -> &[TreeRow] {
        &self.0
    }
}

/// What a file is called without its directory. The whole path when it has no file name
/// at all — a path ending in `..`, say, which nothing here would open but which must not
/// come out as an empty row.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// The short tag a row wears to say what kind of file it is.
///
/// Text and not a picture, which is the answer the filter toggles' glyphs already gave
/// for the same question. The dependency half of that reasoning is gone -- the dock tab
/// bar draws Lucide icons now -- and the other half was checked again against all 1640 of
/// them: nothing in the set names an object file format, so every row would wear one
/// generic page and the column would stop answering the question it exists to answer.
/// Four characters of the format's own name say all of it, and say it differently for
/// each format.
pub fn format_tag(format: BinaryFormat) -> &'static str {
    match format {
        BinaryFormat::Elf => "ELF",
        BinaryFormat::Pe => "PE",
        BinaryFormat::Coff => "COFF",
        BinaryFormat::MachO => "MACH",
        BinaryFormat::Wasm => "WASM",
        BinaryFormat::Xcoff => "XCOF",
        // `BinaryFormat` is `#[non_exhaustive]`, so a format this build has never heard
        // of is still a row that has to say something.
        _ => "OBJ",
    }
}

/// The tag on a file row. Its children have formats; the archive holding them is an
/// archive, which is a file format `object` does not parse and so has no `BinaryFormat`.
pub const ARCHIVE_TAG: &str = "AR";

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use analysis::{Architecture, DwarfCache, ObjectData};

    use crate::filter::Filter;

    use super::*;

    /// An `Object` with nothing in it but the two fields the tree reads. Parsing a real
    /// one would be testing `analysis` instead of this file.
    fn object(path: &str, name: &str) -> Arc<Object> {
        Arc::new(Object {
            path: PathBuf::from(path),
            name: name.to_owned(),
            format: BinaryFormat::Elf,
            architecture: Architecture::X86_64,
            symbols: HashMap::new(),
            symbols_sorted: Vec::new(),
            sections: Vec::new(),
            data: ObjectData::from(&[][..]),
            dwarf: DwarfCache::default(),
        })
    }

    fn plain(pattern: &str) -> Filter {
        Filter {
            pattern: pattern.to_owned(),
            ..Filter::default()
        }
    }

    /// The rows as `(name, kind)`, which is what every assertion below is about.
    fn described(tree: &ObjectTree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|row| match row {
                TreeRow::File {
                    name,
                    members,
                    expansion,
                    loading,
                    ..
                } => {
                    let reading = if *loading { " reading" } else { "" };
                    format!("file {name} ({members}) {expansion:?}{reading}")
                }
                TreeRow::Object { object, member } => {
                    let indent = if *member { "  " } else { "" };
                    format!("{indent}object {}", object.name)
                }
            })
            .collect()
    }

    fn tree(objects: &[Arc<Object>], filter: &Filter, expanded: &[usize]) -> ObjectTree {
        loading_tree(objects, &Loads::default(), filter, expanded)
    }

    fn loading_tree(
        objects: &[Arc<Object>],
        loads: &Loads,
        filter: &Filter,
        expanded: &[usize],
    ) -> ObjectTree {
        ObjectTree::new(
            objects,
            loads,
            &filter.matcher(),
            &expanded.iter().copied().collect(),
        )
    }

    /// A `Loads` with one load over `paths`, which is what every load in the app is
    /// except the multi-file one the Open dialog can make.
    fn reading(paths: &[&str]) -> Loads {
        let mut loads = Loads::default();
        loads.begin(&paths.iter().map(PathBuf::from).collect::<Vec<_>>());
        loads
    }

    fn archive() -> Vec<Arc<Object>> {
        vec![
            object("/tmp/libfoo.rlib", "foo.o"),
            object("/tmp/libfoo.rlib", "bar.o"),
            object("/tmp/libfoo.rlib", "baz.o"),
        ]
    }

    #[test]
    fn a_file_with_one_object_is_its_own_row() {
        let objects = vec![object("/tmp/hello", "hello")];
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[])),
            ["object hello"]
        );
    }

    #[test]
    fn an_archive_folds_its_members_away() {
        let objects = archive();
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[])),
            ["file libfoo.rlib (3) Collapsed"]
        );

        let group = Arc::as_ptr(&objects[0]).addr();
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[group])),
            [
                "file libfoo.rlib (3) Expanded",
                "  object foo.o",
                "  object bar.o",
                "  object baz.o",
            ]
        );
    }

    /// Grouping is by consecutive run, so a file opened twice is one row over both
    /// copies rather than two rows the reader would have to tell apart. What that row
    /// counts is what is under it.
    #[test]
    fn one_path_opened_twice_is_one_group() {
        let objects = [archive(), archive()].concat();
        let group = Arc::as_ptr(&objects[0]).addr();
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[])),
            ["file libfoo.rlib (6) Collapsed"]
        );
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[group])).len(),
            7
        );
    }

    /// A member matched, so the member is the answer: its file comes with it as the thing
    /// it hangs under, held open, and the members that did not match are not there.
    #[test]
    fn a_matching_member_opens_its_file() {
        let objects = archive();
        assert_eq!(
            described(&tree(&objects, &plain("ba"), &[])),
            [
                "file libfoo.rlib (2) Forced",
                "  object bar.o",
                "  object baz.o",
            ]
        );
    }

    /// The file matched, so the file is the answer: every member is under it and the row
    /// stays folded the way it was left. Opening it would bury the row that matched.
    #[test]
    fn a_matching_file_keeps_its_own_expansion() {
        let objects = archive();
        assert_eq!(
            described(&tree(&objects, &plain("rlib"), &[])),
            ["file libfoo.rlib (3) Collapsed"]
        );

        let group = Arc::as_ptr(&objects[0]).addr();
        assert_eq!(
            described(&tree(&objects, &plain("rlib"), &[group])),
            [
                "file libfoo.rlib (3) Expanded",
                "  object foo.o",
                "  object bar.o",
                "  object baz.o",
            ]
        );
    }

    /// The file's name matches nothing a member is called, so this is the case where a
    /// member-only rule would have shown an empty file row.
    #[test]
    fn a_matching_file_carries_members_that_match_nothing() {
        let objects = archive();
        let group = Arc::as_ptr(&objects[0]).addr();
        assert_eq!(
            described(&tree(&objects, &plain("libfoo"), &[group])),
            [
                "file libfoo.rlib (3) Expanded",
                "  object foo.o",
                "  object bar.o",
                "  object baz.o",
            ]
        );
    }

    /// Nothing under a file matched and the file did not either, so the file is gone —
    /// not an empty row that folds open onto nothing.
    #[test]
    fn a_file_nothing_matches_is_not_there() {
        let objects = [archive(), vec![object("/tmp/hello", "hello")]].concat();
        assert_eq!(
            described(&tree(&objects, &plain("hell"), &[])),
            ["object hello"]
        );
    }

    /// The directory is not what the row shows, so it is not what the filter reads.
    #[test]
    fn the_directory_is_not_matched() {
        let objects = archive();
        assert!(described(&tree(&objects, &plain("tmp"), &[])).is_empty());
    }

    /// A pattern that will not compile matches nothing, so the list is empty — the bar
    /// above it is what says why, and a file row with no reason to be there would be the
    /// one thing left on screen.
    #[test]
    fn an_invalid_pattern_empties_the_tree() {
        let objects = [archive(), vec![object("/tmp/hello", "hello")]].concat();
        let filter = Filter {
            regex: true,
            ..plain("foo(")
        };
        assert!(described(&tree(&objects, &filter, &[])).is_empty());
    }

    /// The row that could not exist before objects streamed: a file that has been asked
    /// for and has produced nothing yet is still on screen, saying so. This is the whole
    /// of `Goals.md`'s "an indicator for an object still being processed".
    #[test]
    fn a_file_being_read_is_a_row_before_it_has_an_object() {
        assert_eq!(
            described(&loading_tree(
                &[],
                &reading(&["/tmp/libfoo.rlib"]),
                &Filter::default(),
                &[]
            )),
            ["file libfoo.rlib (0) Collapsed reading"]
        );
    }

    /// A file with nothing behind it has no group either, since the group is the first
    /// object's pointer -- so nothing can fold it, and nothing draws a triangle inviting
    /// the reader to try.
    #[test]
    fn a_file_with_nothing_behind_it_has_no_group() {
        let tree = loading_tree(
            &[],
            &reading(&["/tmp/libfoo.rlib"]),
            &Filter::default(),
            &[],
        );
        assert!(matches!(tree.row(0), TreeRow::File { group: None, .. }));
    }

    /// While a file is being read, one object is not yet an answer: the row stays a file
    /// row so that the second member landing does not turn a top-level row into a parent
    /// under a reader who is already reading it.
    #[test]
    fn one_object_of_a_file_still_being_read_stays_under_its_file() {
        let objects = vec![object("/tmp/libfoo.rlib", "foo.o")];
        let loads = reading(&["/tmp/libfoo.rlib"]);
        let group = Arc::as_ptr(&objects[0]).addr();

        assert_eq!(
            described(&loading_tree(
                &objects,
                &loads,
                &Filter::default(),
                &[group]
            )),
            ["file libfoo.rlib (1) Expanded reading", "  object foo.o"]
        );

        // And once the read is over it collapses into the one row the rule has always
        // given it.
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[group])),
            ["object foo.o"]
        );
    }

    /// The members that have arrived are ordinary rows under an ordinary file row; only
    /// the indicator says the rest are still coming.
    #[test]
    fn a_part_read_archive_shows_what_it_has() {
        let objects = archive();
        let loads = reading(&["/tmp/libfoo.rlib"]);
        let group = Arc::as_ptr(&objects[0]).addr();

        assert_eq!(
            described(&loading_tree(
                &objects,
                &loads,
                &Filter::default(),
                &[group]
            )),
            [
                "file libfoo.rlib (3) Expanded reading",
                "  object foo.o",
                "  object bar.o",
                "  object baz.o",
            ]
        );
        assert_eq!(
            described(&tree(&objects, &Filter::default(), &[group])),
            [
                "file libfoo.rlib (3) Expanded",
                "  object foo.o",
                "  object bar.o",
                "  object baz.o",
            ]
        );
    }

    /// A file being read is filtered like any other row, on the only name it has. It has
    /// no members to match through, so its own name is the whole question.
    #[test]
    fn a_file_being_read_is_filtered_on_its_name() {
        let loads = reading(&["/tmp/libfoo.rlib"]);
        assert_eq!(
            described(&loading_tree(&[], &loads, &plain("foo"), &[])),
            ["file libfoo.rlib (0) Collapsed reading"]
        );
        assert!(described(&loading_tree(&[], &loads, &plain("bar"), &[])).is_empty());
    }

    /// Two files asked for at once are two rows, in the order they were asked for, and
    /// the one that has started producing objects is drawn where its objects are.
    #[test]
    fn every_file_being_read_gets_a_row_of_its_own() {
        let objects = vec![object("/tmp/hello", "hello")];
        let loads = reading(&["/tmp/hello", "/tmp/libfoo.rlib"]);
        assert_eq!(
            described(&loading_tree(&objects, &loads, &Filter::default(), &[])),
            [
                "file hello (1) Collapsed reading",
                "file libfoo.rlib (0) Collapsed reading",
            ]
        );
    }

    /// Closing a file stops every load of it, whoever asked: the unit is the path, which
    /// is what `close_binary` closes by and what the saved binaries are a list of.
    #[test]
    fn cancelling_a_path_stops_every_load_of_it() {
        let mut loads = Loads::default();
        let first = loads.begin(&[PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
        let second = loads.begin(&[PathBuf::from("/tmp/a")]);

        loads.cancel(Path::new("/tmp/a"));

        assert!(!loads.holds(first, Path::new("/tmp/a")));
        assert!(!loads.holds(second, Path::new("/tmp/a")));
        // The other path of the first load is untouched: one file closing is not the
        // request closing.
        assert!(loads.holds(first, Path::new("/tmp/b")));
        assert!(loads.active(first));
        assert!(!loads.active(second));
    }

    /// The reason a load has an id at all: a path closed and immediately reopened is two
    /// loads, and the abandoned one's objects must not arrive into the new one's row.
    #[test]
    fn one_load_does_not_answer_for_another_over_the_same_path() {
        let mut loads = Loads::default();
        let first = loads.begin(&[PathBuf::from("/tmp/a")]);
        loads.cancel(Path::new("/tmp/a"));
        let second = loads.begin(&[PathBuf::from("/tmp/a")]);

        assert!(!loads.holds(first, Path::new("/tmp/a")));
        assert!(loads.holds(second, Path::new("/tmp/a")));
        // The row does not care which load it is: the file is being read.
        assert!(loads.is_loading(Path::new("/tmp/a")));
    }

    /// Finishing is per path, so a load over several files goes quiet one file at a time
    /// — which is what lets each row stop saying it is being read at the right moment.
    #[test]
    fn finishing_one_path_leaves_the_rest_of_its_load() {
        let mut loads = Loads::default();
        let id = loads.begin(&[PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);

        loads.finished(id, Path::new("/tmp/a"));

        assert!(!loads.is_loading(Path::new("/tmp/a")));
        assert!(loads.is_loading(Path::new("/tmp/b")));
        assert!(loads.active(id));

        loads.finished(id, Path::new("/tmp/b"));
        assert!(!loads.active(id));
        assert!(loads.paths().is_empty());
    }

    /// One file is one row however many loads are producing it, and the order is the
    /// order it was asked for in.
    #[test]
    fn the_paths_being_read_do_not_repeat() {
        let mut loads = Loads::default();
        loads.begin(&[PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
        loads.begin(&[PathBuf::from("/tmp/a")]);

        assert_eq!(loads.paths(), [Path::new("/tmp/a"), Path::new("/tmp/b")]);
    }

    /// Leaving a project abandons every load at once, including the ones whose files have
    /// contributed nothing and so are not in the objects list to be closed one by one.
    #[test]
    fn clearing_abandons_every_load() {
        let mut loads = Loads::default();
        let id = loads.begin(&[PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
        loads.clear();

        assert!(!loads.active(id));
        assert!(loads.paths().is_empty());
    }

    #[test]
    fn tags_name_the_format() {
        assert_eq!(format_tag(BinaryFormat::Elf), "ELF");
        assert_eq!(format_tag(BinaryFormat::Pe), "PE");
        assert_eq!(format_tag(BinaryFormat::Coff), "COFF");
    }
}

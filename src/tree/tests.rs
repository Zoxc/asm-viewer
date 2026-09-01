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

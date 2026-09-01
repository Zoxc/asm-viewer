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
    File {
        /// What the row is called: the file's name, without its directory.
        name: String,
        /// The whole path, which is what the row's tooltip says.
        path: PathBuf,
        /// The group's identity, and the key the expansion set holds. The pointer of the
        /// first object the file contributed, which is `Arc` pointer identity the way the
        /// rest of the UI keys things — a path would collide with the same file opened
        /// twice, and an index would move under the reader as files are opened.
        group: usize,
        /// How many objects are under this row *now*, which under a filter is how many
        /// of them matched.
        members: usize,
        expansion: Expansion,
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
    pub fn new(objects: &[Arc<Object>], matcher: &Matcher, expanded: &HashSet<usize>) -> Self {
        // Whether the filter is asking anything at all. Nothing may be forced open while
        // it is not: an untouched list is exactly the list, folded the way it was left.
        let filtering = !matches!(matcher, Matcher::Everything);
        let mut rows = Vec::new();
        let mut rest = objects;

        while let Some(first) = rest.first() {
            let count = rest.iter().take_while(|o| o.path == first.path).count();
            let (group, tail) = rest.split_at(count);
            rest = tail;

            // One object from this file: it *is* the row. See the module comment.
            if let [object] = group {
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
                group,
                members: members.len(),
                expansion,
            });

            if expansion != Expansion::Collapsed {
                rows.extend(members.into_iter().map(|object| TreeRow::Object {
                    object: object.clone(),
                    member: true,
                }));
            }
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

    use analysis::{DwarfCache, ObjectData};

    use crate::filter::Filter;

    use super::*;

    /// An `Object` with nothing in it but the two fields the tree reads. Parsing a real
    /// one would be testing `analysis` instead of this file.
    fn object(path: &str, name: &str) -> Arc<Object> {
        Arc::new(Object {
            path: PathBuf::from(path),
            name: name.to_owned(),
            format: BinaryFormat::Elf,
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
                    ..
                } => format!("file {name} ({members}) {expansion:?}"),
                TreeRow::Object { object, member } => {
                    let indent = if *member { "  " } else { "" };
                    format!("{indent}object {}", object.name)
                }
            })
            .collect()
    }

    fn tree(objects: &[Arc<Object>], filter: &Filter, expanded: &[usize]) -> ObjectTree {
        ObjectTree::new(
            objects,
            &filter.matcher(),
            &expanded.iter().copied().collect(),
        )
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

    #[test]
    fn tags_name_the_format() {
        assert_eq!(format_tag(BinaryFormat::Elf), "ELF");
        assert_eq!(format_tag(BinaryFormat::Pe), "PE");
        assert_eq!(format_tag(BinaryFormat::Coff), "COFF");
    }
}

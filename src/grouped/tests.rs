use super::*;
use crate::filter::Filter;

/// The rows as they are spelled: a file row by its name and count, an item row by its item.
fn spelled(rows: &Rows<u32>) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::File {
                name,
                count,
                folded,
                ..
            } => format!("{name} {count}{}", if *folded { " folded" } else { "" }),
            Row::Item { item, .. } => item.to_string(),
        })
        .collect()
}

fn all(grouped: &Grouped<u32>) -> Rows<u32> {
    grouped.rows(&Matcher::Everything)
}

/// A file's path as a caller holds it: one `Arc` for every item pushed under it.
fn path(spelling: &str) -> Arc<Path> {
    Arc::from(Path::new(spelling))
}

/// The path a push is given is the path the rows are built from, and not a copy of it:
/// the search makes one per file, and nothing after it allocates another. Fails on a
/// `push` that takes `&Path` and makes its own `Arc`.
#[test]
fn a_files_rows_are_built_from_the_arc_it_was_pushed_under() {
    let a = path("/p/a.rs");
    let mut grouped = Grouped::default();
    grouped.push(&a, 1);
    grouped.push(&a, 7);

    for row in all(&grouped).iter() {
        let (Row::File { path, .. } | Row::Item { path, .. }) = row;
        assert!(
            Arc::ptr_eq(path, &a),
            "the row copied the path it was given"
        );
    }
}

/// Two `Arc`s spelling the same path are the same file: `push` shortcuts through
/// `Arc::ptr_eq`, but what it means by the same file is what the path says.
#[test]
fn the_same_path_under_another_arc_is_the_same_file() {
    let mut grouped = Grouped::default();
    grouped.push(&path("/p/a.rs"), 1);
    grouped.push(&path("/p/a.rs"), 7);

    assert_eq!(grouped.files(), 1);
    assert_eq!(spelled(&all(&grouped)), ["a.rs 2", "1", "7"]);
}

/// Items are grouped under the file they came with, the files in the order they arrived,
/// and each row carries the path it is under.
#[test]
fn items_are_grouped_under_their_file_in_the_order_they_arrived() {
    let (a, b) = (path("/p/a.rs"), path("/p/b.rs"));
    let mut grouped = Grouped::default();
    grouped.push(&a, 1);
    grouped.push(&a, 7);
    grouped.push(&b, 2);

    assert_eq!((grouped.count(), grouped.files()), (3, 2));
    let rows = all(&grouped);
    assert_eq!(spelled(&rows), ["a.rs 2", "1", "7", "b.rs 1", "2"]);
    assert_eq!(
        rows[1],
        Row::Item {
            path: Arc::from(Path::new("/p/a.rs")),
            item: Arc::new(1),
        }
    );
}

/// A file pushed again after another one is a second group of its own: the comparison is
/// against the last file and not a lookup, since a walk reports a file's items together.
#[test]
fn a_file_that_comes_back_later_is_a_group_of_its_own() {
    let (a, b) = (path("/p/a.rs"), path("/p/b.rs"));
    let mut grouped = Grouped::default();
    grouped.push(&a, 1);
    grouped.push(&b, 2);
    grouped.push(&a, 3);

    assert_eq!(grouped.files(), 3);
    assert_eq!(
        spelled(&all(&grouped)),
        ["a.rs 1", "1", "b.rs 1", "2", "a.rs 1", "3"]
    );
}

/// A folded file draws its own row and none of its items, and its count is still what
/// was found and not what is drawn. Folding a file that is not there changes nothing.
#[test]
fn a_folded_file_keeps_its_row_and_its_count() {
    let mut grouped = Grouped::from_files([
        (Path::new("/p/a.rs"), vec![1, 7]),
        (Path::new("/p/b.rs"), vec![2]),
    ]);

    assert!(grouped.toggle(Path::new("/p/a.rs")));
    assert_eq!(spelled(&all(&grouped)), ["a.rs 2 folded", "b.rs 1", "2"]);
    assert_eq!(grouped.count(), 3, "the fold hides rows and finds nothing");

    // And back, since the same press unfolds it.
    assert!(grouped.toggle(Path::new("/p/a.rs")));
    assert_eq!(spelled(&all(&grouped)), ["a.rs 2", "1", "7", "b.rs 1", "2"]);

    assert!(!grouped.toggle(Path::new("/p/gone.rs")));
}

/// The filter is asked about the file's path, and a file it does not keep takes its items
/// with it.
#[test]
fn the_filter_keeps_files_by_their_path() {
    let grouped = Grouped::from_files([
        (Path::new("/p/src/a.rs"), vec![1]),
        (Path::new("/p/tests/b.rs"), vec![2]),
    ]);

    let filter = Filter {
        pattern: "tests".to_owned(),
        ..Filter::default()
    };
    assert_eq!(spelled(&grouped.rows(&filter.matcher())), ["b.rs 1", "2"]);
}

/// The rows are shared by an `Arc` and compared by it, so handing ten thousand of them to
/// a scroll view is one comparison.
#[test]
fn rows_are_compared_by_pointer() {
    let grouped: Grouped<u32> = Grouped::default();
    let rows = all(&grouped);

    assert!(rows == rows.clone());
    assert!(rows != all(&grouped));
}

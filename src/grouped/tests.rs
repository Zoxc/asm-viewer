use super::*;
use crate::filter::Filter;

/// The rows as they are drawn: a file row by its name and count, an item row by its item.
fn drawn(rows: &Rows<u32>) -> Vec<String> {
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

/// Items are grouped under the file they came with, the files in the order they arrived,
/// and each row carries the path it is under.
#[test]
fn items_are_grouped_under_their_file_in_the_order_they_arrived() {
    let mut grouped = Grouped::default();
    grouped.push(Path::new("/p/a.rs"), 1);
    grouped.push(Path::new("/p/a.rs"), 7);
    grouped.push(Path::new("/p/b.rs"), 2);

    assert_eq!((grouped.count(), grouped.files()), (3, 2));
    let rows = all(&grouped);
    assert_eq!(drawn(&rows), ["a.rs 2", "1", "7", "b.rs 1", "2"]);
    assert_eq!(
        rows[1],
        Row::Item {
            path: PathBuf::from("/p/a.rs"),
            item: Arc::new(1),
        }
    );
}

/// A file pushed again after another one is a second group of its own: the comparison is
/// against the last file and not a lookup, since a walk reports a file's items together.
#[test]
fn a_file_that_comes_back_later_is_a_group_of_its_own() {
    let mut grouped = Grouped::default();
    grouped.push(Path::new("/p/a.rs"), 1);
    grouped.push(Path::new("/p/b.rs"), 2);
    grouped.push(Path::new("/p/a.rs"), 3);

    assert_eq!(grouped.files(), 3);
    assert_eq!(
        drawn(&all(&grouped)),
        ["a.rs 1", "1", "b.rs 1", "2", "a.rs 1", "3"]
    );
}

/// A folded file draws its own row and none of its items, and its count is still what
/// was found and not what is drawn. Folding a file that is not there changes nothing.
#[test]
fn a_folded_file_keeps_its_row_and_its_count() {
    let mut grouped = Grouped::from_files([
        (PathBuf::from("/p/a.rs"), vec![1, 7]),
        (PathBuf::from("/p/b.rs"), vec![2]),
    ]);

    assert!(grouped.toggle(Path::new("/p/a.rs")));
    assert_eq!(drawn(&all(&grouped)), ["a.rs 2 folded", "b.rs 1", "2"]);
    assert_eq!(grouped.count(), 3, "the fold hides rows and finds nothing");

    // And back, since the same press unfolds it.
    assert!(grouped.toggle(Path::new("/p/a.rs")));
    assert_eq!(drawn(&all(&grouped)), ["a.rs 2", "1", "7", "b.rs 1", "2"]);

    assert!(!grouped.toggle(Path::new("/p/gone.rs")));
}

/// The filter is asked about the file's path, and a file it does not keep takes its items
/// with it.
#[test]
fn the_filter_keeps_files_by_their_path() {
    let grouped = Grouped::from_files([
        (PathBuf::from("/p/src/a.rs"), vec![1]),
        (PathBuf::from("/p/tests/b.rs"), vec![2]),
    ]);

    let filter = Filter {
        pattern: "tests".to_owned(),
        ..Filter::default()
    };
    assert_eq!(drawn(&grouped.rows(&filter.matcher())), ["b.rs 1", "2"]);
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

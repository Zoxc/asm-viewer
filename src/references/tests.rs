use super::*;
use crate::filter::Matcher;
use crate::grouped::Row;
use std::path::PathBuf;

/// A place as the server answers one.
fn place(file: &str, line: u32, columns: Range<u32>) -> lsp::Place {
    lsp::Place {
        file: PathBuf::from(file),
        line,
        columns,
    }
}

/// Every row, with no filter over the files.
fn all(references: &References) -> ReferenceRows {
    references.rows(&Matcher::Everything)
}

/// A grouping over places whose files hold nothing: what the rows are without any text.
fn grouped(places: &[lsp::Place]) -> References {
    of(places, |_| None)
}

/// The text of the reference rows, in the order they are drawn.
fn texts(rows: &ReferenceRows) -> Vec<String> {
    (0..rows.len())
        .filter_map(|at| match &rows[at] {
            Row::Item { item, .. } => Some(item.text.clone()),
            Row::File { .. } => None,
        })
        .collect()
}

/// The paths of the file rows, in the order they are drawn.
fn files(rows: &ReferenceRows) -> Vec<String> {
    (0..rows.len())
        .filter_map(|at| match &rows[at] {
            Row::File { path, .. } => Some(path.display().to_string()),
            Row::Item { .. } => None,
        })
        .collect()
}

/// The lines of the reference rows, in the order they are drawn.
fn lines(rows: &ReferenceRows) -> Vec<u32> {
    (0..rows.len())
        .filter_map(|at| match &rows[at] {
            Row::Item { item, .. } => Some(item.line),
            Row::File { .. } => None,
        })
        .collect()
}

#[test]
fn the_answers_places_are_grouped_by_file_whatever_order_they_came_in() {
    let references = grouped(&[
        place("/p/src/b.rs", 9, 0..3),
        place("/p/src/a.rs", 4, 8..11),
        place("/p/src/b.rs", 2, 1..4),
    ]);

    assert_eq!(references.count(), 3);
    let rows = all(&references);
    // The files by path, and inside one the references by line.
    assert_eq!(files(&rows), vec!["/p/src/a.rs", "/p/src/b.rs"]);
    assert_eq!(lines(&rows), vec![4, 2, 9]);
    assert_eq!(
        &rows[0],
        &Row::File {
            path: PathBuf::from("/p/src/a.rs"),
            name: "a.rs".to_owned(),
            count: 1,
            folded: false,
        }
    );
}

#[test]
fn a_name_used_twice_on_one_line_is_two_rows_each_with_its_own_columns() {
    let references = grouped(&[
        place("/p/src/a.rs", 7, 20..23),
        place("/p/src/a.rs", 7, 4..7),
    ]);

    let rows = all(&references);
    assert_eq!(references.count(), 2);
    assert_eq!(lines(&rows), vec![7, 7]);
    let columns: Vec<Range<usize>> = (0..rows.len())
        .filter_map(|at| match &rows[at] {
            Row::Item { item, .. } => Some(item.columns.clone()),
            Row::File { .. } => None,
        })
        .collect();
    assert_eq!(columns, vec![4..7, 20..23]);
}

#[test]
fn a_reference_carries_its_line_marked_where_the_name_is() {
    let source = "fn main() {\n    let n = helper(1);\n}\n";
    let references = of(&[place("/p/src/main.rs", 2, 12..18)], |path| {
        (path == Path::new("/p/src/main.rs")).then(|| source.to_owned())
    });

    let rows = all(&references);
    let Row::Item { item, .. } = &rows[1] else {
        panic!("the second row is the use");
    };
    // The line as a row draws it: its indentation gone, and the name marked where it is
    // in what is left.
    assert_eq!(item.text, "let n = helper(1);");
    assert_eq!(item.spans, vec![8..14]);
    assert_eq!(&item.text[item.spans[0].clone()], "helper");
    // And the columns are still the file's own line's, which is what opening it selects.
    assert_eq!(item.columns, 12..18);
}

#[test]
fn every_reference_in_one_file_costs_one_read() {
    let reads = std::cell::Cell::new(0);
    let references = of(
        &[
            place("/p/src/main.rs", 1, 3..7),
            place("/p/src/main.rs", 2, 0..4),
            place("/p/src/other.rs", 1, 0..4),
        ],
        |_| {
            reads.set(reads.get() + 1);
            Some("main here\nmain there\n".to_owned())
        },
    );

    assert_eq!(
        reads.get(),
        2,
        "a file is read once however many references are in it"
    );
    assert_eq!(
        texts(&all(&references)),
        vec!["main here", "main there", "main here"]
    );
}

#[test]
fn a_line_the_file_does_not_have_is_the_number_alone() {
    // The file changed under the answer, or would not read at all.
    let references = of(&[place("/p/src/main.rs", 9, 0..4)], |_| {
        Some("one\ntwo\n".to_owned())
    });

    let rows = all(&references);
    let Row::Item { item, .. } = &rows[1] else {
        panic!("the second row is the use");
    };
    assert_eq!(item.line, 9);
    assert!(item.text.is_empty());
    assert!(item.spans.is_empty());
}

#[test]
fn a_name_after_a_wide_character_is_marked_in_bytes_and_counted_in_units() {
    // The answer's columns are bytes and a pane's are UTF-16 units, and an emoji is
    // where the two part: four bytes, two units. `// ` is three of each, then the crab,
    // then a space, so the name begins at byte 8 and at column 6.
    let references = of(&[place("/p/src/main.rs", 1, 8..14)], |_| {
        Some("// \u{1f980} helper\n".to_owned())
    });

    let rows = all(&references);
    let Row::Item { item, .. } = &rows[1] else {
        panic!("the second row is the use");
    };
    assert_eq!(&item.text[item.spans[0].clone()], "helper");
    assert_eq!(item.columns, 6..12);
}

#[test]
fn columns_the_line_has_no_such_bytes_for_mark_nothing() {
    // Half of a character, and a run past the end of the line: a line that has changed
    // under the answer, or a server counting some other way. Both are marks that are not
    // drawn, and neither is a slice taken off a character boundary.
    let inside = of(&[place("/p/src/main.rs", 1, 4..6)], |_| {
        Some("// \u{1f980} helper\n".to_owned())
    });
    let beyond = of(&[place("/p/src/main.rs", 1, 8..99)], |_| {
        Some("// \u{1f980} helper\n".to_owned())
    });

    for references in [inside, beyond] {
        let rows = all(&references);
        let Row::Item { item, .. } = &rows[1] else {
            panic!("the second row is the use");
        };
        assert_eq!(item.text, "// \u{1f980} helper");
        assert!(item.spans.is_empty());
    }
}

#[test]
fn a_line_of_zero_is_the_number_alone_and_not_a_panic() {
    // Places are 1-based by the server's answer, so a 0 is a line no file has. The
    // subtraction that finds it must not wrap: in release it would read `usize::MAX`.
    let references = of(&[place("/p/src/main.rs", 0, 0..4)], |_| {
        Some("one\ntwo\n".to_owned())
    });

    let rows = all(&references);
    let Row::Item { item, .. } = &rows[1] else {
        panic!("the second row is the use");
    };
    assert_eq!(item.line, 0);
    assert!(item.text.is_empty());
    assert!(item.spans.is_empty());
}

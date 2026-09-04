use super::*;

/// A place as the server answers one.
fn place(file: &str, line: u32, columns: Range<u32>) -> lsp::Place {
    lsp::Place {
        file: PathBuf::from(file),
        line,
        columns,
    }
}

/// Every row, with no filter over the files.
fn all(uses: &Uses) -> UseRows {
    uses.rows_matching(&Matcher::Everything)
}

/// A grouping over places whose files hold nothing: what the rows are without any text.
fn grouped(places: &[lsp::Place]) -> Uses {
    Uses::of(places, |_| None)
}

/// The text of the use rows, in the order they are drawn.
fn texts(rows: &UseRows) -> Vec<String> {
    (0..rows.len())
        .filter_map(|at| match rows.row(at) {
            UseRow::Use { used, .. } => Some(used.text.clone()),
            UseRow::File { .. } => None,
        })
        .collect()
}

/// The paths of the file rows, in the order they are drawn.
fn files(rows: &UseRows) -> Vec<String> {
    (0..rows.len())
        .filter_map(|at| match rows.row(at) {
            UseRow::File { path, .. } => Some(path.display().to_string()),
            UseRow::Use { .. } => None,
        })
        .collect()
}

/// The lines of the use rows, in the order they are drawn.
fn lines(rows: &UseRows) -> Vec<u32> {
    (0..rows.len())
        .filter_map(|at| match rows.row(at) {
            UseRow::Use { used, .. } => Some(used.line),
            UseRow::File { .. } => None,
        })
        .collect()
}

#[test]
fn the_answers_places_are_grouped_by_file_whatever_order_they_came_in() {
    let uses = grouped(&[
        place("/p/src/b.rs", 9, 0..3),
        place("/p/src/a.rs", 4, 8..11),
        place("/p/src/b.rs", 2, 1..4),
    ]);

    assert_eq!(uses.count(), 3);
    let rows = all(&uses);
    // The files by path, and inside one the uses by line.
    assert_eq!(files(&rows), vec!["/p/src/a.rs", "/p/src/b.rs"]);
    assert_eq!(lines(&rows), vec![4, 2, 9]);
    assert_eq!(
        rows.row(0),
        &UseRow::File {
            path: PathBuf::from("/p/src/a.rs"),
            name: "a.rs".to_owned(),
            count: 1,
            folded: false,
        }
    );
}

#[test]
fn a_name_used_twice_on_one_line_is_two_rows_each_with_its_own_columns() {
    let uses = grouped(&[
        place("/p/src/a.rs", 7, 20..23),
        place("/p/src/a.rs", 7, 4..7),
    ]);

    let rows = all(&uses);
    assert_eq!(uses.count(), 2);
    assert_eq!(lines(&rows), vec![7, 7]);
    let columns: Vec<Range<u32>> = (0..rows.len())
        .filter_map(|at| match rows.row(at) {
            UseRow::Use { used, .. } => Some(used.columns.clone()),
            UseRow::File { .. } => None,
        })
        .collect();
    assert_eq!(columns, vec![4..7, 20..23]);
}

#[test]
fn folding_a_file_hides_its_uses_and_leaves_its_count() {
    let mut uses = grouped(&[place("/p/src/a.rs", 4, 0..1), place("/p/src/b.rs", 9, 0..1)]);

    assert!(uses.toggle(Path::new("/p/src/a.rs")));
    let rows = all(&uses);
    assert_eq!(files(&rows), vec!["/p/src/a.rs", "/p/src/b.rs"]);
    assert_eq!(lines(&rows), vec![9]);
    assert_eq!(
        rows.row(0),
        &UseRow::File {
            path: PathBuf::from("/p/src/a.rs"),
            name: "a.rs".to_owned(),
            count: 1,
            folded: true,
        }
    );

    // And back, since the same press unfolds it.
    assert!(uses.toggle(Path::new("/p/src/a.rs")));
    assert_eq!(lines(&all(&uses)), vec![4, 9]);
}

#[test]
fn folding_a_file_that_is_not_there_changes_nothing() {
    let mut uses = grouped(&[place("/p/src/a.rs", 4, 0..1)]);

    assert!(!uses.toggle(Path::new("/p/src/gone.rs")));
    assert_eq!(lines(&all(&uses)), vec![4]);
}

#[test]
fn an_answer_naming_nowhere_is_no_rows() {
    let uses = grouped(&[]);

    assert_eq!(uses.count(), 0);
    assert_eq!(all(&uses).len(), 0);
}

#[test]
fn a_use_carries_its_line_marked_where_the_name_is() {
    let source = "fn main() {\n    let n = helper(1);\n}\n";
    let uses = Uses::of(&[place("/p/src/main.rs", 2, 12..18)], |path| {
        (path == Path::new("/p/src/main.rs")).then(|| source.to_owned())
    });

    let rows = all(&uses);
    let UseRow::Use { used, .. } = rows.row(1) else {
        panic!("the second row is the use");
    };
    // The line as a row draws it: its indentation gone, and the name marked where it is
    // in what is left.
    assert_eq!(used.text, "let n = helper(1);");
    assert_eq!(used.spans, vec![8..14]);
    assert_eq!(&used.text[used.spans[0].clone()], "helper");
    // And the columns are still the file's own line's, which is what opening it selects.
    assert_eq!(used.columns, 12..18);
}

#[test]
fn every_use_in_one_file_costs_one_read() {
    let reads = std::cell::Cell::new(0);
    let uses = Uses::of(
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
        "a file is read once however many uses are in it"
    );
    assert_eq!(
        texts(&all(&uses)),
        vec!["main here", "main there", "main here"]
    );
}

#[test]
fn a_line_the_file_does_not_have_is_the_number_alone() {
    // The file changed under the answer, or would not read at all.
    let uses = Uses::of(&[place("/p/src/main.rs", 9, 0..4)], |_| {
        Some("one\ntwo\n".to_owned())
    });

    let rows = all(&uses);
    let UseRow::Use { used, .. } = rows.row(1) else {
        panic!("the second row is the use");
    };
    assert_eq!(used.line, 9);
    assert!(used.text.is_empty());
    assert!(used.spans.is_empty());
}

#[test]
fn a_name_after_a_wide_character_is_marked_where_it_is_in_the_bytes() {
    // The columns are UTF-16 units and the spans are bytes: an emoji is two units and
    // four bytes, and a name after one is marked wrongly by anything that confuses them.
    // `// ` is three units, the crab is two more, then a space: the name starts at six.
    let uses = Uses::of(&[place("/p/src/main.rs", 1, 6..12)], |_| {
        Some("// \u{1f980} helper\n".to_owned())
    });

    let rows = all(&uses);
    let UseRow::Use { used, .. } = rows.row(1) else {
        panic!("the second row is the use");
    };
    assert_eq!(&used.text[used.spans[0].clone()], "helper");
}

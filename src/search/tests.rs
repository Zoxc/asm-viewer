use std::{
    fs,
    sync::atomic::{AtomicU32, Ordering as Atomic},
};

use super::*;

/// A directory of this test's own, empty, under the system's temp directory.
fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Atomic::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "viewer-search-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("the temp directory is writable");
    path
}

fn write(path: &Path, text: &str) {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).expect("the temp directory is writable");
    }
    fs::write(path, text).expect("the temp directory is writable");
}

/// A plain search for `pattern` under `root`, every hit collected.
fn found(root: &Path, pattern: &str) -> Vec<Hit> {
    hits(root, filter(pattern))
}

fn filter(pattern: &str) -> Filter {
    Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    }
}

fn hits(root: &Path, filter: Filter) -> Vec<Hit> {
    let query = SearchQuery {
        root: root.to_path_buf(),
        filter,
    };
    let mut hits = Vec::new();
    let mut finished = false;
    search(&query, &mut |event| {
        match event {
            SearchEvent::Hit(hit) => hits.push(hit),
            SearchEvent::Finished => finished = true,
        }
        ControlFlow::Continue(())
    });
    assert!(finished, "a search that ends says so");
    hits
}

/// Each hit as `path:line`, the path relative to the root, which is what the order
/// assertions are about.
fn places(root: &Path, hits: &[Hit]) -> Vec<String> {
    hits.iter()
        .map(|hit| {
            let path = hit.path.strip_prefix(root).unwrap_or(&hit.path);
            format!(
                "{}:{}",
                path.display().to_string().replace('\\', "/"),
                hit.line
            )
        })
        .collect()
}

/// A file's own hits come before the directories under it, and each level is by name.
/// The order is the order the panel's list grows in, so it is pinned.
#[test]
fn a_directorys_files_come_before_the_directories_under_it() {
    let root = temp_dir("order");
    write(&root.join("b.rs"), "needle\n");
    write(&root.join("a/inner.rs"), "needle\n");
    write(&root.join("a.rs"), "needle\n");
    write(&root.join("z/deep/last.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(
        places(&root, &hits) == ["a.rs:1", "b.rs:1", "a/inner.rs:1", "z/deep/last.rs:1"],
        "{:?}",
        places(&root, &hits)
    );
    let _ = fs::remove_dir_all(&root);
}

/// Every line of a file that matches is its own hit, numbered from one.
#[test]
fn every_matching_line_is_a_hit_numbered_from_one() {
    let root = temp_dir("lines");
    write(&root.join("x.rs"), "one\nneedle\nthree\nneedle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["x.rs:2", "x.rs:4"]);
    let _ = fs::remove_dir_all(&root);
}

/// What `.gitignore` names is not searched, and it is honoured outside a git working
/// tree, which is where a project directory usually is.
#[test]
fn what_git_is_told_to_ignore_is_not_searched() {
    let root = temp_dir("ignored");
    write(&root.join(".gitignore"), "target\n");
    write(&root.join("target/build.rs"), "needle\n");
    write(&root.join("kept.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["kept.rs:1"]);
    let _ = fs::remove_dir_all(&root);
}

/// A hidden file is not searched, unlike the Files panel, which lists one.
#[test]
fn a_hidden_file_is_not_searched() {
    let root = temp_dir("hidden");
    write(&root.join(".secret.rs"), "needle\n");
    write(&root.join("open.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["open.rs:1"]);
    let _ = fs::remove_dir_all(&root);
}

/// A file with a NUL in it is a binary file, and is left where it was found: the match
/// before the NUL is not reported either, since the file is abandoned whole.
#[test]
fn a_binary_file_is_skipped() {
    let root = temp_dir("binary");
    fs::write(root.join("object.o"), b"needle\n\x00 needle\n").expect("writable");
    write(&root.join("source.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["source.rs:1"]);
    let _ = fs::remove_dir_all(&root);
}

/// Nothing typed is no search at all, and neither is a pattern that will not compile:
/// both say so under the box instead.
#[test]
fn nothing_typed_and_a_broken_pattern_are_not_questions() {
    let root = temp_dir("askable");
    write(&root.join("x.rs"), "needle\n");

    let query = |pattern: &str, regex: bool| SearchQuery {
        root: root.clone(),
        filter: Filter {
            pattern: pattern.to_owned(),
            regex,
            ..Filter::default()
        },
    };

    assert!(!query("", false).is_askable());
    assert!(!query("(", true).is_askable());
    assert!(query("needle", false).is_askable());
    assert!(found(&root, "").is_empty());
    let _ = fs::remove_dir_all(&root);
}

/// The three toggles mean what they mean in a filter bar, the expression being the same
/// one: a literal pattern is escaped, Word is `\b` and not something looser, and case is
/// the builder's flag.
#[test]
fn the_toggles_mean_what_they_mean_in_a_filter_bar() {
    let root = temp_dir("toggles");
    write(
        &root.join("x.rs"),
        "Needle\nneedles\na.c\nabc\nfoo -2 bar\n",
    );

    let case = Filter {
        pattern: "needle".to_owned(),
        case_sensitive: true,
        ..Filter::default()
    };
    assert!(places(&root, &hits(&root, case)) == ["x.rs:2"]);

    let word = Filter {
        pattern: "needle".to_owned(),
        whole_word: true,
        ..Filter::default()
    };
    assert!(places(&root, &hits(&root, word)) == ["x.rs:1"]);

    // `-2` under Word is `\b(?:\-2)\b`, which does not match `foo -2 bar`. `grep-regex`'s
    // own `word` option would, which is why the expression is written here instead.
    let looser = Filter {
        pattern: "-2".to_owned(),
        whole_word: true,
        ..Filter::default()
    };
    assert!(hits(&root, looser).is_empty());

    // A literal `.` is escaped and does not match any character.
    let literal = filter("a.c");
    assert!(places(&root, &hits(&root, literal)) == ["x.rs:3"]);

    let expression = Filter {
        pattern: "a.c".to_owned(),
        regex: true,
        ..Filter::default()
    };
    assert!(places(&root, &hits(&root, expression)) == ["x.rs:3", "x.rs:4"]);
    let _ = fs::remove_dir_all(&root);
}

/// A pattern anchored to the line's start is answered about the whole line, not the
/// trimmed text the row draws: the trimming happens after the match is found.
#[test]
fn an_anchored_pattern_is_asked_of_the_whole_line() {
    let root = temp_dir("anchored");
    write(&root.join("x.rs"), "    needle\nneedle\n");

    let anchored = Filter {
        pattern: "^needle".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, anchored);

    assert!(places(&root, &hits) == ["x.rs:2"]);
    let _ = fs::remove_dir_all(&root);
}

/// A match that starts in the whitespace the row does not draw is marked for the part of
/// it that is drawn. The matches are found over the whole line -- a pattern that needs the
/// indentation finds it -- and are moved to the drawn text afterwards.
#[test]
fn a_match_reaching_into_the_indentation_is_marked_for_what_is_drawn() {
    let root = temp_dir("indent");
    write(&root.join("x.rs"), "    needle;\n");

    let indented = Filter {
        pattern: r"\s+needle".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, indented);

    assert!(hits.len() == 1);
    assert!(hits[0].text == "needle;", "{:?}", hits[0].text);
    assert!(hits[0].spans == vec![0..6], "{:?}", hits[0].spans);
    let _ = fs::remove_dir_all(&root);
}

/// The row's text is the line without its leading whitespace or its terminator, and the
/// spans point into that text and not into the line as it was read.
#[test]
fn the_spans_are_where_the_matches_are_in_the_text_drawn() {
    let root = temp_dir("spans");
    write(&root.join("x.rs"), "\tlet needle = needle;\r\n");

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    let hit = &hits[0];
    assert!(hit.text == "let needle = needle;", "{:?}", hit.text);
    assert!(hit.spans == vec![4..10, 13..19], "{:?}", hit.spans);
    assert!(hit
        .spans
        .iter()
        .all(|span| &hit.text[span.clone()] == "needle"));
    let _ = fs::remove_dir_all(&root);
}

/// A hit knows where its first match is in the **file's** line, in the UTF-16 units a
/// pane counts columns in: what opening the hit selects. Counted over the whole line, so
/// the indentation the row does not draw is still in it, and in units and not bytes, so a
/// multi-byte character before the match does not move it.
#[test]
fn a_hit_knows_where_its_match_is_in_the_files_line() {
    let root = temp_dir("columns");
    write(&root.join("x.rs"), "  \u{e9}\u{1f600} needle here\n");

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    // Two spaces, `\u{e9}` (one unit), an emoji (two) and a space: the match starts at 6.
    assert!(hits[0].columns == Some(6..12), "{:?}", hits[0].columns);
    let _ = fs::remove_dir_all(&root);
}

/// A line longer than the bound is cut on a character boundary, and a match past the cut
/// is dropped rather than pointing off the end of the text.
#[test]
fn a_long_line_is_cut_on_a_character_boundary() {
    let root = temp_dir("cut");
    let long = format!("needle{}needle\n", "\u{e9}".repeat(MAX_LINE));
    write(&root.join("x.rs"), &long);

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    let hit = &hits[0];
    assert!(hit.text.chars().count() == MAX_LINE);
    assert!(hit.spans == vec![0..6], "{:?}", hit.spans);
    let _ = fs::remove_dir_all(&root);
}

/// A zero-width match marks nothing, so it is not a span, and the line is a hit all the
/// same.
#[test]
fn a_zero_width_match_is_a_hit_with_nothing_marked() {
    let root = temp_dir("empty");
    write(&root.join("x.rs"), "word\n");

    let empty = Filter {
        pattern: r"\b".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, empty);

    assert!(hits.len() == 1);
    assert!(hits[0].spans.is_empty());
    let _ = fs::remove_dir_all(&root);
}

/// The callback saying stop stops the walk where it stands, and nothing is emitted after
/// it -- not even the end of the search, which nobody is listening for.
#[test]
fn a_break_stops_the_walk_where_it_stands() {
    let root = temp_dir("break");
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(&root.join(name), "needle\nneedle\n");
    }

    let query = SearchQuery {
        root: root.clone(),
        filter: filter("needle"),
    };
    let mut seen = 0;
    let mut finished = false;
    search(&query, &mut |event| {
        match event {
            SearchEvent::Hit(_) => seen += 1,
            SearchEvent::Finished => finished = true,
        }
        if seen == 2 {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });

    assert!(seen == 2);
    assert!(!finished);
    let _ = fs::remove_dir_all(&root);
}

/// The search stops at the cap, and says it ended: a capped search is over, where a
/// search whose reader has gone is not worth saying anything to.
#[test]
fn the_search_stops_at_the_cap() {
    let root = temp_dir("cap");
    let lines = "needle\n".repeat(MAX_HITS + 5);
    write(&root.join("many.rs"), &lines);

    let hits = found(&root, "needle");

    assert!(hits.len() == MAX_HITS);

    let mut held = SearchHits::default();
    for hit in hits {
        held.push(hit);
    }
    assert!(held.capped());
    let _ = fs::remove_dir_all(&root);
}

/// Hits are grouped under the file they are in, in the order they arrived, and a folded
/// file draws its own row and none of theirs.
#[test]
fn hits_are_grouped_under_their_file_and_fold() {
    let mut hits = SearchHits::default();
    let hit = |path: &str, line: u32| Hit {
        path: PathBuf::from(path),
        line,
        text: "needle".to_owned(),
        spans: vec![0..6],
        columns: Some(0..6),
    };
    hits.push(hit("a.rs", 1));
    hits.push(hit("a.rs", 7));
    hits.push(hit("b.rs", 2));

    assert!(hits.counts() == (3, 2));
    assert!(!hits.capped());

    let rows = hits.rows();
    assert!(rows.len() == 5);
    assert!(
        rows.row(0)
            == &SearchRow::File {
                path: PathBuf::from("a.rs"),
                name: "a.rs".to_owned(),
                count: 2,
                folded: false,
            }
    );
    assert!(rows.row(1) == &SearchRow::Match(hit("a.rs", 1)));

    assert!(hits.toggle(Path::new("a.rs")));
    let folded = hits.rows();
    assert!(folded.len() == 3);
    assert!(matches!(
        folded.row(0),
        SearchRow::File { folded: true, .. }
    ));
    assert!(matches!(folded.row(1), SearchRow::File { name, .. } if name == "b.rs"));

    assert!(!hits.toggle(Path::new("nothing.rs")));
}

/// The rows are shared by an `Arc` and compared by it, so handing ten thousand of them to
/// a scroll view is one comparison.
#[test]
fn rows_are_compared_by_pointer() {
    let hits = SearchHits::default();
    let rows = hits.rows();

    assert!(rows == rows.clone());
    assert!(rows != hits.rows());
}

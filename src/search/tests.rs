use std::fs;

use super::*;
use crate::temporary::Temporary;

fn write(path: &Path, text: &str) {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).expect("the temp directory is writable");
    }
    fs::write(path, text).expect("the temp directory is writable");
}

/// A plain search for `pattern` under `root`: every hit, with the file it was found in.
fn found(root: &Path, pattern: &str) -> Vec<(Arc<Path>, Hit)> {
    hits(root, filter(pattern))
}

fn filter(pattern: &str) -> Filter {
    Filter {
        pattern: pattern.to_owned(),
        ..Filter::default()
    }
}

fn hits(root: &Path, filter: Filter) -> Vec<(Arc<Path>, Hit)> {
    let query = SearchQuery {
        root: root.to_path_buf(),
        filter,
    };
    let mut hits = Vec::new();
    let mut finished = false;
    search(&query, &mut |event| {
        match event {
            SearchEvent::Hit(path, hit) => hits.push((path, hit)),
            SearchEvent::Finished => finished = true,
        }
        ControlFlow::Continue(())
    });
    assert!(finished, "a search that ends says so");
    hits
}

/// Each hit as `path:line`, the path relative to the root, which is what the order
/// assertions are about.
fn places(root: &Path, hits: &[(Arc<Path>, Hit)]) -> Vec<String> {
    hits.iter()
        .map(|(path, hit)| {
            let path = path.strip_prefix(root).unwrap_or(path);
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
    let root = Temporary::fresh_directory("search-order");
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
}

/// A file's hits all carry the one `Arc` the search made for it, so a capped search
/// allocates a path per file and not per hit. Fails on a `PathBuf` built for each.
#[test]
fn every_hit_of_a_file_carries_the_one_path() {
    let root = Temporary::fresh_directory("search-one-path");
    write(&root.join("a.rs"), "needle\nneedle\nneedle\n");
    write(&root.join("b.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert_eq!(hits.len(), 4);
    let first = &hits[0].0;
    for (path, _) in &hits[..3] {
        assert!(Arc::ptr_eq(path, first), "a.rs allocated a path per hit");
    }
    assert!(!Arc::ptr_eq(&hits[3].0, first), "b.rs is another file");
}

/// Every line of a file that matches is its own hit, numbered from one.
#[test]
fn every_matching_line_is_a_hit_numbered_from_one() {
    let root = Temporary::fresh_directory("search-lines");
    write(&root.join("x.rs"), "one\nneedle\nthree\nneedle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["x.rs:2", "x.rs:4"]);
}

/// What `.gitignore` names is not searched, and it is honoured outside a git working
/// tree, which is where a project directory usually is.
#[test]
fn what_git_is_told_to_ignore_is_not_searched() {
    let root = Temporary::fresh_directory("search-ignored");
    write(&root.join(".gitignore"), "target\n");
    write(&root.join("target/build.rs"), "needle\n");
    write(&root.join("kept.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["kept.rs:1"]);
}

/// A hidden file is not searched, unlike the Files panel, which lists one.
#[test]
fn a_hidden_file_is_not_searched() {
    let root = Temporary::fresh_directory("search-hidden");
    write(&root.join(".secret.rs"), "needle\n");
    write(&root.join("open.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["open.rs:1"]);
}

/// A file with a NUL in it is a binary file, and is left where it was found: the match
/// before the NUL is not reported either, since the file is abandoned whole.
#[test]
fn a_binary_file_is_skipped() {
    let root = Temporary::fresh_directory("search-binary");
    fs::write(root.join("object.o"), b"needle\n\x00 needle\n").expect("writable");
    write(&root.join("source.rs"), "needle\n");

    let hits = found(&root, "needle");

    assert!(places(&root, &hits) == ["source.rs:1"]);
}

/// Nothing typed is no search at all, and neither is a pattern that will not compile:
/// both say so under the box instead.
#[test]
fn nothing_typed_and_a_broken_pattern_are_not_questions() {
    let root = Temporary::fresh_directory("search-askable");
    write(&root.join("x.rs"), "needle\n");

    let query = |pattern: &str, regex: bool| SearchQuery {
        root: root.to_path_buf(),
        filter: Filter {
            pattern: pattern.to_owned(),
            regex,
            ..Filter::default()
        },
    };

    assert!(!query("", false).is_askable());
    assert!(!query("(", true).is_askable());
    assert!(query("needle", false).is_askable());

    // Asked anyway, each ends having found nothing: an empty pattern is not one that
    // matches every line, and a pattern that will not build is not searched for.
    assert!(found(&root, "").is_empty());
    let broken = Filter {
        pattern: "(".to_owned(),
        regex: true,
        ..Filter::default()
    };
    assert!(hits(&root, broken).is_empty());
}

/// The three toggles mean what they mean in a filter bar, the expression being the same
/// one: a literal pattern is escaped, Word is `\b` and not something looser, and case is
/// the builder's flag.
#[test]
fn the_toggles_mean_what_they_mean_in_a_filter_bar() {
    let root = Temporary::fresh_directory("search-toggles");
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
}

/// A pattern anchored to the line's start is answered about the whole line, not the
/// trimmed text the row draws: the trimming happens after the match is found.
#[test]
fn an_anchored_pattern_is_asked_of_the_whole_line() {
    let root = Temporary::fresh_directory("search-anchored");
    write(&root.join("x.rs"), "    needle\nneedle\n");

    let anchored = Filter {
        pattern: "^needle".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, anchored);

    assert!(places(&root, &hits) == ["x.rs:2"]);
}

/// A pattern anchored to the line's end finds a line ended by `\r\n` as it finds one ended
/// by `\n`: the `\r` is part of the terminator and not of the line.
#[test]
fn an_end_anchored_pattern_finds_a_crlf_line() {
    let root = Temporary::fresh_directory("search-crlf");
    write(&root.join("x.rs"), "let a = 1;\r\nfn b() {\r\n");

    let anchored = |pattern: &str| Filter {
        pattern: pattern.to_owned(),
        regex: true,
        ..Filter::default()
    };
    let statement = hits(&root, anchored(";$"));
    assert!(places(&root, &statement) == ["x.rs:1"], "{statement:?}");
    let brace = hits(&root, anchored(r"\{$"));
    assert!(places(&root, &brace) == ["x.rs:2"], "{brace:?}");
}

/// A match that starts in the whitespace the row does not draw is marked for the part of
/// it that is drawn. The matches are found over the whole line -- a pattern that needs the
/// indentation finds it -- and are moved to the drawn text afterwards.
#[test]
// Lists of one range, on purpose.
#[allow(clippy::single_range_in_vec_init)]
fn a_match_reaching_into_the_indentation_is_marked_for_what_is_drawn() {
    let root = Temporary::fresh_directory("search-indent");
    write(&root.join("x.rs"), "    needle;\n");

    let indented = Filter {
        pattern: r"\s+needle".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, indented);

    assert!(hits.len() == 1);
    assert!(hits[0].1.text == "needle;", "{:?}", hits[0].1.text);
    assert!(hits[0].1.spans == vec![0..6], "{:?}", hits[0].1.spans);
}

/// The row's text is the line without its leading whitespace or its terminator, and the
/// spans point into that text and not into the line as it was read.
#[test]
fn the_spans_are_where_the_matches_are_in_the_text_drawn() {
    let root = Temporary::fresh_directory("search-spans");
    write(&root.join("x.rs"), "\tlet needle = needle;\r\n");

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    let hit = &hits[0].1;
    assert!(hit.text == "let needle = needle;", "{:?}", hit.text);
    assert!(hit.spans == vec![4..10, 13..19], "{:?}", hit.spans);
    assert!(hit
        .spans
        .iter()
        .all(|span| &hit.text[span.clone()] == "needle"));
}

/// A hit knows where its first match is in the **file's** line, in bytes: what opening
/// the hit selects. Counted over the whole line, so the indentation the row does not draw
/// is still in it.
#[test]
fn a_hit_knows_where_its_match_is_in_the_files_line() {
    let root = Temporary::fresh_directory("search-columns");
    write(&root.join("x.rs"), "  \u{e9}\u{1f600} needle here\n");

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    // Two spaces, `\u{e9}` (two bytes), an emoji (four) and a space: the match starts at 9.
    assert!(hits[0].1.columns == Some(9..15), "{:?}", hits[0].1.columns);
}

/// A BOM is part of the line, as the Source pane reads it: the columns count its three
/// bytes. Fails with the searcher's default, which strips it.
#[test]
fn a_utf8_boms_bytes_are_counted_in_the_columns() {
    let root = Temporary::fresh_directory("search-bom");
    write(&root.join("x.rs"), "\u{feff}needle\n");

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    assert!(hits[0].1.columns == Some(3..9), "{:?}", hits[0].1.columns);
}

/// A UTF-16 file is not decoded: its NULs make it the binary the pane would draw it as.
#[test]
fn a_utf16_file_is_skipped_as_binary() {
    let root = Temporary::fresh_directory("search-utf16");
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend("abc needle\n".encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(root.join("x.rc"), bytes).expect("writable");

    assert!(found(&root, "needle").is_empty());
}

/// A line longer than the bound is cut on a character boundary, and a match past the cut
/// is dropped rather than pointing off the end of the text.
#[test]
// Lists of one range, on purpose.
#[allow(clippy::single_range_in_vec_init)]
fn a_long_line_is_cut_on_a_character_boundary() {
    let root = Temporary::fresh_directory("search-cut");
    let long = format!("needle{}needle\n", "\u{e9}".repeat(grouped::MAX_LINE));
    write(&root.join("x.rs"), &long);

    let hits = found(&root, "needle");

    assert!(hits.len() == 1);
    let hit = &hits[0].1;
    assert!(hit.text.chars().count() == grouped::MAX_LINE);
    assert!(hit.spans == vec![0..6], "{:?}", hit.spans);
}

/// A zero-width match marks nothing, so it is not a span, and the line is a hit all the
/// same.
#[test]
fn a_zero_width_match_is_a_hit_with_nothing_marked() {
    let root = Temporary::fresh_directory("search-empty");
    write(&root.join("x.rs"), "word\n");

    let empty = Filter {
        pattern: r"\b".to_owned(),
        regex: true,
        ..Filter::default()
    };
    let hits = hits(&root, empty);

    assert!(hits.len() == 1);
    assert!(hits[0].1.spans.is_empty());
}

/// The callback saying stop stops the walk where it stands, and nothing is emitted after
/// it -- not even the end of the search, which nobody is listening for.
#[test]
fn a_break_stops_the_walk_where_it_stands() {
    let root = Temporary::fresh_directory("search-break");
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(&root.join(name), "needle\nneedle\n");
    }

    let query = SearchQuery {
        root: root.to_path_buf(),
        filter: filter("needle"),
    };
    let mut seen = 0;
    let mut finished = false;
    search(&query, &mut |event| {
        match event {
            SearchEvent::Hit(..) => seen += 1,
            SearchEvent::Finished => finished = true,
        }
        if seen == 2 {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });

    assert!(seen == 2);
    assert!(!finished);
}

/// The search stops at the cap, and says it ended: a capped search is over, where a
/// search whose reader has gone is not worth saying anything to.
#[test]
fn the_search_stops_at_the_cap() {
    let root = Temporary::fresh_directory("search-cap");
    let lines = "needle\n".repeat(MAX_HITS + 5);
    write(&root.join("many.rs"), &lines);

    let hits = found(&root, "needle");

    assert!(hits.len() == MAX_HITS);

    let mut held = SearchHits::default();
    for (path, hit) in hits {
        held.push(&path, hit);
    }
    assert!(capped(&held));
}

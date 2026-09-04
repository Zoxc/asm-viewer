use super::*;

/// A path as the walk holds one: the name starts after the last separator.
fn hit(query: &str, shown: &str) -> Option<Hit> {
    let name_at = shown.rfind('/').map(|at| at + 1).unwrap_or(0);
    find(query, shown, name_at)
}

/// The marked runs as the text in them, which is what a row draws in the match colour.
fn marked(query: &str, shown: &str) -> Vec<String> {
    hit(query, shown)
        .expect("the query matched")
        .marks
        .iter()
        .map(|run| shown[run.clone()].to_owned())
        .collect()
}

/// Which of two paths the list puts first.
fn better(query: &str, first: &str, second: &str) -> bool {
    let first = hit(query, first).expect("the query matched the first path");
    let second = hit(query, second).expect("the query matched the second path");
    first.score < second.score
}

/// The whole point, and the example the spec is written around.
#[test]
fn characters_in_order_reach_a_path_they_are_spread_through() {
    assert!(hit("srcuivw", "src/ui/files_view.rs").is_some());
}

#[test]
fn a_character_out_of_order_matches_nothing() {
    assert!(hit("cba", "abc.rs").is_none());
    assert!(hit("xyz", "abc.rs").is_none());
}

#[test]
fn nothing_typed_is_not_a_query() {
    assert!(hit("", "abc.rs").is_none());
}

#[test]
fn case_is_ignored() {
    assert!(hit("SRC", "src/x.rs").is_some());
    assert!(hit("src", "SRC/x.rs").is_some());
}

/// The pass back is what makes this true: matched as it is read, `ui` would take the `u`
/// of `src/ui` and the `i` of `view`, and the row would mark two characters a word apart
/// instead of the directory the reader was typing.
#[test]
fn the_marks_are_pulled_together_into_runs() {
    assert_eq!(marked("ui", "src/ui/files_view.rs"), ["ui"]);
    assert_eq!(marked("fv", "src/ui/files_view.rs"), ["f", "v"]);
    assert_eq!(marked("view", "src/ui/files_view.rs"), ["view"]);
}

/// Both placements are tried and the better kept. Read once, `ab` starts on the name's
/// own first character; walked back it would start inside the word, for no fewer runs.
/// Walked back, `sv` keeps both characters in the file's own name, where reading once
/// would have taken the `s` from the directory above it.
#[test]
fn the_marks_are_the_better_of_reading_forward_and_walking_back() {
    assert_eq!(marked("ab", "axaxb.rs"), ["a", "b"]);
    assert_eq!(
        hit("ab", "axaxb.rs").expect("the query matched").marks[0],
        0..1
    );
    assert_eq!(
        hit("sv", "src/ui/files_view.rs")
            .expect("the query matched")
            .marks[0],
        11..12
    );
}

#[test]
fn a_match_in_the_name_beats_one_in_a_directory_above_it() {
    assert!(better("files", "src/files.rs", "src/files/tests.rs"));
}

#[test]
fn a_run_beats_the_same_characters_spread_out() {
    assert!(better("abc", "xabc.rs", "xaxbxc.rs"));
}

#[test]
fn the_start_of_a_word_beats_inside_one() {
    assert!(better("ab", "ab_x.rs", "xab_.rs"));
    assert!(better("ab", "x_ab.rs", "xxab.rs"));
}

/// A camel-cased name's parts start words too, so `FV` reaches `FilesView`.
#[test]
fn a_capital_starts_a_word() {
    assert!(better("ab", "xAb_.rs", "xab_.rs"));
}

#[test]
fn the_shorter_path_wins_a_tie() {
    assert!(better("ab", "x/ab.rs", "xx/ab.rs"));
}

/// The order the four comparisons are made in, pinned as one: a name match wins even
/// where the path is longer and the characters are further apart.
#[test]
fn the_comparisons_are_made_in_the_order_they_are_written() {
    assert!(better("ab", "zzzz/aXXXb.rs", "ab/z.rs"));
}

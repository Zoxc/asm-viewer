//! What [`Coded`] does with an answer.

use super::*;

fn lines(of: &[u32]) -> Arc<HashSet<u32>> {
    Arc::new(of.iter().copied().collect())
}

#[test]
fn the_marks_of_a_file_the_pane_has_left_are_not_taken() {
    let mut state = Coded {
        wanted: Some(Arc::from("now.c")),
        ..Coded::default()
    };
    assert!(!state.take(Arc::from("before.c"), lines(&[1, 2]), vec![7]));
    assert!(state.found.is_none());
}

#[test]
fn the_marks_of_the_file_the_pane_is_showing_are_taken_with_the_objects_they_were_worked_over() {
    let file: Arc<str> = Arc::from("now.c");
    let mut state = Coded {
        wanted: Some(file.clone()),
        ..Coded::default()
    };
    assert!(state.take(file.clone(), lines(&[3]), vec![7]));
    assert!(state.lines_in(&file).is_some());
    assert_eq!(state.over, vec![7]);
    assert!(
        state.pending(&[]).is_some(),
        "asked again once what is open is not what it was worked out over"
    );
}

/// A landing on `line` of `file`, for the tab showing `file` as its own document.
fn landing(file: &Arc<str>, line: u32) -> Landing {
    Landing {
        tab: Document::Source(file.clone()),
        at: Some(LinePos {
            file: file.clone(),
            line,
        }),
        address: None,
        columns: None,
    }
}

#[test]
fn a_pane_reveals_the_first_row_of_its_own_run() {
    let file: Arc<str> = Arc::from("now.c");
    let owing = Owing::Own(3..=7);
    assert_eq!(
        owed_row(&owing, &file, 10, |_| panic!(
            "its own run asks the listing nothing"
        )),
        Some(3)
    );
    assert_eq!(
        owed_row(&owing, &file, 3, |_| Vec::new()),
        None,
        "a row past the end of the file on screen"
    );
}

#[test]
fn a_pane_reveals_the_line_the_other_panes_run_was_compiled_from() {
    let file: Arc<str> = Arc::from("now.c");
    let pair = Owing::Pair(line_pick(file.clone(), 4, None, Owed::default()));
    let at = |file: &str, line| LinePos {
        file: Arc::from(file),
        line,
    };
    assert_eq!(
        owed_row(&pair, &file, 10, |_| vec![at("now.c", 6)]),
        Some(5),
        "the line, as a row"
    );
    assert_eq!(
        owed_row(&pair, &file, 10, |_| vec![
            at("before.h", 6),
            at("now.c", 8)
        ]),
        Some(7),
        "the first place in the file on screen, not the first place"
    );
    assert_eq!(
        owed_row(&pair, &file, 10, |_| vec![at("before.h", 6)]),
        None,
        "a line of a file this pane is not showing"
    );
    assert_eq!(
        owed_row(&pair, &file, 10, |_| vec![at("now.c", 0)]),
        None,
        "no line at all"
    );
    assert_eq!(
        owed_row(&pair, &file, 4, |_| vec![at("now.c", 9)]),
        None,
        "past the end of a file that has moved on since it was compiled"
    );
}

#[test]
fn a_landing_is_taken_only_by_the_pane_drawing_the_place_and_the_file_it_names() {
    let file: Arc<str> = Arc::from("now.c");
    let document = Document::Source(file.clone());
    assert_eq!(
        landing_row(&landing(&file, 3), &document, &file, 10),
        Some(2)
    );

    let other: Arc<str> = Arc::from("before.h");
    assert_eq!(
        landing_row(
            &landing(&file, 3),
            &Document::Source(other.clone()),
            &file,
            10
        ),
        None,
        "a landing on another place of the trail"
    );
    assert_eq!(
        landing_row(&landing(&file, 3), &document, &other, 10),
        None,
        "a companion file the landing does not name"
    );
    assert_eq!(
        landing_row(&landing(&file, 0), &document, &file, 10),
        None,
        "no line at all"
    );
    assert_eq!(
        landing_row(&landing(&file, 11), &document, &file, 10),
        None,
        "a line the file does not have"
    );

    let mut nothing = landing(&file, 3);
    nothing.at = None;
    assert_eq!(
        landing_row(&nothing, &document, &file, 10),
        None,
        "a door that knows an address and no line"
    );
}

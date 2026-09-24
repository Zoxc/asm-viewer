//! What [`Finder`] does with the worker's answer.

use super::*;

fn answered(id: u64, query: &str) -> Answered {
    Answered {
        id,
        query: query.to_owned(),
        rows: Shared::default(),
        walking: false,
    }
}

#[test]
fn the_answer_to_a_walk_the_reader_has_moved_on_from_is_not_taken() {
    let mut state = Finder {
        id: 3,
        walking: true,
        ..Finder::default()
    };
    assert!(!state.take(answered(2, "ma")));
    assert!(state.listed.for_query.is_empty(), "the box is not answered");
    assert!(state.walking, "and the walk that is on is still going");
}

#[test]
fn an_answer_is_taken_whole_so_the_rows_and_their_query_are_one_questions() {
    let mut state = Finder {
        id: 3,
        walking: true,
        ..Finder::default()
    };
    assert!(state.take(answered(3, "main")));
    assert_eq!(state.listed.for_query, "main");
    assert!(!state.walking);
}

fn found(shown: &str) -> Found {
    Found {
        path: PathBuf::from(shown),
        shown: shown.into(),
        name_at: 0,
    }
}

fn found_by(held: &mut Held, id: u64, shown: &str) -> Change {
    held.take(Told::Found {
        id,
        file: found(shown),
    })
}

#[test]
fn a_file_found_by_a_walk_held_back_changes_no_answer() {
    let mut held = Held::default();
    let root = PathBuf::from("/project");
    held.take(Told::Walking {
        id: 1,
        root: root.clone(),
    });
    found_by(&mut held, 1, "main.rs");
    assert!(held.take(Told::Walked { id: 1 }) == Change::Now);

    held.take(Told::Walking { id: 2, root });
    held.take(Told::Asked {
        id: 2,
        query: "mod".to_owned(),
    });
    assert!(
        found_by(&mut held, 2, "mod.rs") == Change::None,
        "the ranking reads the last walk's files until this one ends"
    );
    assert!(held.take(Told::Walked { id: 2 }) == Change::Now);
}

#[test]
fn a_file_the_first_walk_finds_changes_the_answer_only_to_a_query() {
    let mut held = Held::default();
    held.take(Told::Walking {
        id: 1,
        root: PathBuf::from("/project"),
    });
    assert!(
        found_by(&mut held, 1, "main.rs") == Change::None,
        "an empty box ranks nothing"
    );
    held.take(Told::Asked {
        id: 1,
        query: "mod".to_owned(),
    });
    assert!(found_by(&mut held, 1, "mod.rs") == Change::Files);
}

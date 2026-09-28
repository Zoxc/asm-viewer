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

#[test]
fn files_the_throttle_held_back_are_answered_when_the_walk_stalls() {
    let (tells, told) = mpsc::channel::<Told>();
    let (sends, answers) = async_channel::unbounded::<Answered>();
    std::thread::spawn(move || rank_files(told, sends));
    let root = PathBuf::from("/project");
    tells.send(Told::Walking { id: 1, root }).unwrap();
    tells
        .send(Told::Asked {
            id: 1,
            query: "m".to_owned(),
        })
        .unwrap();
    // The answer to the box, which starts the throttle.
    answers.recv_blocking().unwrap();
    tells
        .send(Told::Found {
            id: 1,
            file: found("main.rs"),
        })
        .unwrap();
    // Nothing else is sent: the walk has stalled.
    let answered = std::thread::spawn(move || answers.recv_blocking());
    std::thread::sleep(WALK_REFRESH * 5);
    assert!(answered.is_finished(), "the file is still not answered");
    assert_eq!(answered.join().unwrap().unwrap().rows.len(), 1);
    drop(tells);
}

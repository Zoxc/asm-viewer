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

//! What [`Searched`] does with a batch of hits.

use super::*;
use crate::search::Hit;

/// One hit, and the file it was found in, as a batch carries them.
fn hit(line: u32) -> SearchEvent {
    SearchEvent::Hit(
        PathBuf::from("src/main.rs"),
        Hit {
            line,
            text: "fn main() {}".to_owned(),
            spans: Vec::new(),
            columns: None,
        },
    )
}

#[test]
fn the_batches_of_a_search_the_reader_has_replaced_are_dropped_whole() {
    let mut state = Searched {
        id: 2,
        running: true,
        ..Searched::default()
    };
    assert!(
        !state.take(1, vec![hit(1), SearchEvent::Finished]),
        "the search before this one is nobody's answer"
    );
    assert!(
        (state.hits.count(), state.hits.files()) == (0, 0),
        "not even the hits ahead of the end"
    );
    assert!(state.running, "and it does not stop the search that is on");
}

#[test]
fn the_batches_of_the_search_that_is_on_are_taken_and_its_end_stops_it() {
    let mut state = Searched {
        id: 2,
        running: true,
        ..Searched::default()
    };
    assert!(state.take(2, vec![hit(1), hit(2)]));
    assert!(state.running);
    assert!(state.take(2, vec![SearchEvent::Finished]));
    assert!(!state.running);
    assert!((state.hits.count(), state.hits.files()) == (2, 1));
}

//! What [`Searched`] does with a batch of hits.

use super::*;

fn hit(line: u32) -> Hit {
    Hit {
        path: PathBuf::from("src/main.rs"),
        line,
        text: "fn main() {}".to_owned(),
        spans: Vec::new(),
        columns: None,
    }
}

#[test]
fn the_batches_of_a_search_the_reader_has_replaced_are_dropped_whole() {
    let mut state = Searched {
        id: 2,
        running: true,
        ..Searched::default()
    };
    assert!(
        !state.take(1, vec![SearchEvent::Hit(hit(1)), SearchEvent::Finished]),
        "the search before this one is nobody's answer"
    );
    assert!(
        state.hits.counts() == (0, 0),
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
    assert!(state.take(2, vec![SearchEvent::Hit(hit(1)), SearchEvent::Hit(hit(2))]));
    assert!(state.running);
    assert!(state.take(2, vec![SearchEvent::Finished]));
    assert!(!state.running);
    assert!(state.hits.counts() == (2, 1));
}

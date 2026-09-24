//! What [`Coded`] does with an answer.

use super::*;

fn lines(of: &[u32]) -> Arc<HashSet<u32>> {
    Arc::new(of.iter().copied().collect())
}

/// A real object, parsed per call, so two calls are two objects.
fn fixture() -> Arc<Object> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    analysis::open_files(vec![path])
        .first()
        .expect("the fixture parses")
        .clone()
}

#[test]
fn the_marks_of_a_file_the_pane_has_left_are_not_taken() {
    let showing: Arc<Path> = Arc::from(Path::new("now.c"));
    let mut state = Coded::default();
    assert!(!state.take(
        Some(&showing),
        Arc::from(Path::new("before.c")),
        lines(&[1, 2]),
        Vec::new()
    ));
    assert!(state.found.is_none());
}

#[test]
fn the_marks_of_the_file_the_pane_is_showing_are_taken_with_the_objects_they_were_worked_over() {
    let file: Arc<Path> = Arc::from(Path::new("now.c"));
    let open = [fixture()];
    let mut state = Coded::default();
    assert!(state.take(Some(&file), file.clone(), lines(&[3]), object_ids(&open)));
    // The lines themselves: a take that stored an empty set, or another file's, answers
    // with something either way.
    assert_eq!(state.lines_in(&file), Some(&lines(&[3])));
    assert!(!state.pending(&file, &open));
    assert!(
        state.pending(&file, &[]),
        "asked again once what is open is not what it was worked out over"
    );
    assert!(
        state.pending(&file, &[]),
        "asked again once what is open is not what it was worked out over"
    );
}

/// A closed object's address cannot go to the next one loaded while an answer names it,
/// or that answer would pass for the new object's.
#[test]
fn marks_worked_out_over_a_closed_object_are_not_taken_for_the_next_one() {
    let file: Arc<Path> = Arc::from(Path::new("now.c"));
    let mut state = Coded::default();
    let closed = fixture();
    assert!(state.take(
        Some(&file),
        file.clone(),
        lines(&[3]),
        object_ids(std::slice::from_ref(&closed))
    ));
    drop(closed);

    let loaded = [fixture()];
    assert!(state.pending(&file, &loaded));
}

//! What [`Coded`] does with an answer.

use super::*;

fn lines(of: &[u32]) -> Arc<HashSet<u32>> {
    Arc::new(of.iter().copied().collect())
}

#[test]
fn the_marks_of_a_file_the_pane_has_left_are_not_taken() {
    let showing: Arc<str> = Arc::from("now.c");
    let mut state = Coded::default();
    assert!(!state.take(
        Some(&showing),
        Arc::from("before.c"),
        lines(&[1, 2]),
        vec![7]
    ));
    assert!(state.found.is_none());
}

#[test]
fn the_marks_of_the_file_the_pane_is_showing_are_taken_with_the_objects_they_were_worked_over() {
    let file: Arc<str> = Arc::from("now.c");
    let mut state = Coded::default();
    assert!(state.take(Some(&file), file.clone(), lines(&[3]), vec![7]));
    assert!(state.lines_in(&file).is_some());
    assert_eq!(state.over, vec![7]);
    assert!(
        state.pending(&file, &[]),
        "asked again once what is open is not what it was worked out over"
    );
}

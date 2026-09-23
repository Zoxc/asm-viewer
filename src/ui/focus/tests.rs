//! When the Source pane's run is dropped, and the row a line is.

use super::*;

/// Two tabs, each showing a file of its own. A source stop needs no object, so this stays
/// object-free.
fn tabs() -> (Entry, Entry) {
    let mut docs = Docs::default();
    let (first, second) = (
        Stop::whole(Document::Source(Arc::from(Path::new("a.rs")))),
        Stop::whole(Document::Source(Arc::from(Path::new("b.rs")))),
    );
    let a = (docs.open(first.clone()), first);
    let b = (docs.open(second.clone()), second);
    (a, b)
}

#[test]
fn a_source_run_is_dropped_only_where_the_pane_moved_off_the_file_it_is_in() {
    let (a, b) = tabs();
    let (now, before) = (
        Arc::<Path>::from(Path::new("now.c")),
        Arc::<Path>::from(Path::new("before.h")),
    );

    assert!(
        moved_off(Some(&a), Some(&a), Some(&now), Some(&before), Some(&now)),
        "one place, another file, and the run is in the file left"
    );
    assert!(
        !moved_off(Some(&a), Some(&b), Some(&now), Some(&before), Some(&now)),
        "a switch of place, which `use_land` owns whole"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&now), Some(&now)),
        "the same file still on screen"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&before), Some(&before)),
        "a landing's run, planted in the file arriving"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), Some(&now), Some(&before), None),
        "no run to drop"
    );
    assert!(
        !moved_off(Some(&a), Some(&a), None, Some(&now), Some(&now)),
        "a pane that was drawing no file was on no run's file"
    );
}

/// **A row and a line convert one way each**, and line 0 is no row: debug info writes it
/// for instructions belonging to no source line, and a stored place or a compiler's own
/// message can state it too, so it reaches the panes from a file. Read as row 0 it would
/// pick out the first line of the file, which is somewhere the reader was never sent.
#[test]
fn line_0_is_no_row_of_any_file() {
    let file: Arc<Path> = Arc::from(Path::new("now.c"));

    assert_eq!(LinePos::of_row(file.clone(), 0).line, 1);
    assert_eq!(LinePos::of_row(file.clone(), 39).row(), Some(39));
    assert_eq!(LinePos::line_of(usize::MAX), u32::MAX, "no file has it");

    assert_eq!(LinePos::row_of(0), None);
    assert_eq!(LinePos::row_of(1), Some(0));
    let none = LinePos {
        file: file.clone(),
        line: 0,
    };
    assert_eq!(none.row(), None);

    // And the run a door onto a line makes, which is where the two used to disagree: the
    // panes read the line as a row, so line 0 is a door onto nowhere.
    assert!(line_pick(file.clone(), 0, None, Owed::BOTH).is_none());
    let first = line_pick(file.clone(), 1, None, Owed::BOTH).expect("line 1 is row 0");
    assert!(
        !first.is_line(&file, 0),
        "the run on the first row is not a run of line 0"
    );
    assert!(first.is_line(&file, 1));
}

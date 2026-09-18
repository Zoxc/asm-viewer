//! The recent order and the rows it is drawn as.

use std::fs;

use super::*;
use crate::project::files::tests::{directory, kept_at, paths, round_trip};
use crate::project::files::{Details, PROJECT_EXTENSION};
use crate::project::saves::unsaved_project;
use crate::store::{MAX_ORDER, PROJECTS_DIR};

/// A recent list that will not parse is moved aside, the next `remember` being what would
/// otherwise write over it.
#[test]
fn a_recent_list_that_will_not_parse_is_moved_aside() {
    let directory = directory(line!());
    let path = directory.join(RECENTS_FILE);
    fs::create_dir_all(&directory).expect("creating the test directory");
    fs::write(&path, b"{ not toml").expect("writing");
    assert_eq!(load_recents(&Store::at(&directory)), Recents::default());
    assert!(directory
        .join(crate::store::INCOMPATIBLE_DIR)
        .join(RECENTS_FILE)
        .exists());
}

/// A project the app is keeping is written relative to the state directory and every other
/// path absolutely, so moving that directory does not lose every unsaved project. In
/// memory they are all absolute.
#[test]
fn a_project_in_app_storage_is_remembered_relative_to_it() {
    let base = directory(line!());
    let store = Store::at(&base);
    let unsaved = store.projects().join(format!("1.{PROJECT_EXTENSION}"));
    let elsewhere = PathBuf::from("/src/kernel/kernel.avproj");

    let mut recents = Recents::default();
    recents.touch(elsewhere.as_path());
    recents.touch(unsaved.as_path());

    write_recents(&store, recents);
    let text = fs::read_to_string(store.path(RECENTS_FILE)).expect("the file was written");
    assert!(
        text.contains(&format!("{PROJECTS_DIR}/1.{PROJECT_EXTENSION}")),
        "the unsaved project was not written relative to the store: {text}"
    );
    // Relative and not merely ending that way: an absolute path holds the tail above too,
    // so the store's own prefix is what says which of the two was written.
    assert!(
        !text.contains(&base.to_string_lossy().into_owned()),
        "the unsaved project was written by its whole path: {text}"
    );
    assert!(text.contains("/src/kernel/kernel.avproj"), "{text}");

    // And back: what the file holds is read as the paths the app works in.
    assert_eq!(load_recents(&store).entries(), [unsaved, elsewhere]);
}

/// Bounded, because this file is appended to for as long as the app is ever used. What
/// falls off the end is a place in the order and never a project. The bound is the
/// **file's**, applied on the way out, which is what lets the pads share one order type
/// with a list the panel is holding whole.
#[test]
fn the_recent_list_is_bounded_where_it_is_written() {
    let base = directory(line!());
    let store = Store::at(&base);

    let mut recents = Recents::default();
    for n in 0..MAX_ORDER + 10 {
        recents.touch(kept_at(&format!("{n}")));
    }
    assert_eq!(recents.len(), MAX_ORDER + 10);

    write_recents(&store, recents);
    let stored = load_recents(&store);
    assert_eq!(stored.entries().len(), MAX_ORDER);
    assert_eq!(
        stored.first(),
        Some(&kept_at(&format!("{}", MAX_ORDER + 9)))
    );
}

#[test]
fn the_recent_list_round_trips_through_toml() {
    let mut recents = Recents::default();
    recents.touch(kept_at("1"));
    recents.touch(kept_at("2"));
    let text = round_trip(&recents);
    assert!(text.contains(r#"2.avproj"#), "{text}");

    // A missing or unreadable file is the empty list, never an error.
    assert_eq!(load_recents(&Store::at("/no/such")), Recents::default());
}

/// The recent-projects view describes each row out of that project's own file, in the order
/// the list keeps, so nothing about a project is copied beside the order.
#[test]
fn the_recent_view_describes_each_project_from_its_own_file() {
    let base = directory(line!());
    let store = Store::at(&base);
    for name in ["kernel", "loader"] {
        let path = store.projects().join(format!("{name}.{PROJECT_EXTENSION}"));
        store
            .write_toml(
                &path,
                &Project {
                    id: None,
                    details: Details {
                        directory: Some(PathBuf::from("/src").join(name)),
                        ..Details::default()
                    },
                    binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
                    bookmarks: Vec::new(),
                },
            )
            .expect("saving the project");
        remember(&store, &path);
    }

    let recents = recent_projects(&store);
    assert_eq!(
        recents
            .iter()
            .map(|row| row.path.clone())
            .collect::<Vec<_>>(),
        [
            store.projects().join(format!("loader.{PROJECT_EXTENSION}")),
            store.projects().join(format!("kernel.{PROJECT_EXTENSION}"))
        ]
    );
    assert_eq!(recents[0].directory, Some(PathBuf::from("/src/loader")));
    assert_eq!(recents[0].binaries, 2);
}

/// Reading a project to draw its row is not opening it: a file that will not parse is
/// left where it is, since nothing is about to write over a project nobody has entered.
#[test]
fn listing_a_project_does_not_move_its_file_aside() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = store.projects().join(format!("broken.{PROJECT_EXTENSION}"));
    fs::create_dir_all(store.projects()).expect("creating the test directory");
    fs::write(&path, b"{ not toml").expect("writing the corrupt file");
    remember(&store, &path);

    // The row is drawn, as the project it will behave as once opened.
    let recents = recent_projects(&store);
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].directory, None);

    assert!(path.exists());
    assert!(!base.join(crate::store::INCOMPATIBLE_DIR).exists());
}

/// A project whose file has gone is dropped here, where the repair is free; one whose file
/// is there and holds nothing yet is a real project and keeps its row.
#[test]
fn a_recent_project_that_is_gone_is_dropped_and_an_empty_one_is_not() {
    let base = directory(line!());
    let store = Store::at(&base);
    let empty = unsaved_project(&store).expect("a project");
    remember(&store, &empty);
    remember(
        &store,
        &store.projects().join(format!("gone.{PROJECT_EXTENSION}")),
    );

    let recents = recent_projects(&store);
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].path, empty);
    assert_eq!(recents[0].directory, None);
    assert_eq!(recents[0].binaries, 0);
}

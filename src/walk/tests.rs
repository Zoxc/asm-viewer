use std::{
    fs,
    sync::atomic::{AtomicU32, Ordering as Atomic},
};

use super::*;

/// A directory of this test's own, empty, under the system's temp directory.
fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Atomic::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "viewer-walk-{}-{unique}-{name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("the temp directory is writable");
    path
}

fn write(path: &Path, text: &str) {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).expect("the temp directory is writable");
    }
    fs::write(path, text).expect("the temp directory is writable");
}

/// Every file the walk reports under `root`, as the paths it draws them by.
fn walked(root: &Path) -> Vec<String> {
    files(root)
        .into_iter()
        .map(|found| found.shown.into_string())
        .collect()
}

fn files(root: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    let mut finished = false;
    walk_files(root, &mut |event| {
        match event {
            WalkEvent::File(file) => found.push(file),
            WalkEvent::Finished => finished = true,
        }
        ControlFlow::Continue(())
    });
    assert!(finished, "a walk that ends says so");
    found
}

/// The order the finder's list is built in, and the Search panel's before it: a
/// directory's own files, then the directories under it, each by name.
#[test]
fn a_directorys_files_come_before_the_directories_under_it() {
    let root = temp_dir("order");
    write(&root.join("b.rs"), "");
    write(&root.join("a/inner.rs"), "");
    write(&root.join("a.rs"), "");

    assert_eq!(walked(&root), ["a.rs", "b.rs", "a/inner.rs"]);
}

/// The rule the module exists for: what the search skips, the finder skips.
#[test]
fn what_git_is_told_to_ignore_is_not_walked() {
    let root = temp_dir("ignored");
    write(&root.join(".gitignore"), "skipped.rs\n");
    write(&root.join("skipped.rs"), "");
    write(&root.join("kept.rs"), "");

    assert_eq!(walked(&root), ["kept.rs"]);
}

#[test]
fn a_hidden_file_is_not_walked() {
    let root = temp_dir("hidden");
    write(&root.join(".hidden.rs"), "");
    write(&root.join("kept.rs"), "");

    assert_eq!(walked(&root), ["kept.rs"]);
}

/// The source pane's bound: a file it would refuse to show is one the finder must not
/// offer, since opening it would do nothing.
#[test]
fn a_file_too_big_for_the_source_pane_is_not_walked() {
    let root = temp_dir("oversized");
    let big = " ".repeat(crate::source::MAX_SIZE as usize + 1);
    write(&root.join("big.rs"), &big);
    write(&root.join("kept.rs"), "");

    assert_eq!(walked(&root), ["kept.rs"]);
}

/// A directory is not a file, whatever it is called.
#[test]
fn a_directory_is_not_reported() {
    let root = temp_dir("directories");
    fs::create_dir_all(root.join("empty.rs")).expect("the temp directory is writable");
    write(&root.join("kept.rs"), "");

    assert_eq!(walked(&root), ["kept.rs"]);
}

/// What the row draws and what the query is matched against: one string, `/` separated
/// whatever the platform, cut into the name and the directories above it.
#[test]
fn a_file_is_held_by_the_path_it_is_drawn_by() {
    let root = temp_dir("shown");
    write(&root.join("src/ui/files_view.rs"), "");
    write(&root.join("top.rs"), "");

    let found = files(&root);
    let deep = found
        .iter()
        .find(|file| file.name() == "files_view.rs")
        .expect("the deep file was walked");
    assert_eq!(&*deep.shown, "src/ui/files_view.rs");
    assert_eq!(deep.directory(), "src/ui/");
    assert_eq!(deep.path, root.join("src").join("ui").join("files_view.rs"));

    let top = found
        .iter()
        .find(|file| file.name() == "top.rs")
        .expect("the top file was walked");
    assert_eq!(top.directory(), "");
}

/// A walk nobody is waiting for stops where it stands: the callback's own answer, the
/// only way either reader cancels one.
#[test]
fn a_walk_stops_when_the_callback_says_to() {
    let root = temp_dir("stopped");
    write(&root.join("a.rs"), "");
    write(&root.join("b.rs"), "");
    write(&root.join("c.rs"), "");

    let mut seen = Vec::new();
    let mut finished = false;
    walk_files(&root, &mut |event| match event {
        WalkEvent::File(file) => {
            seen.push(file.shown.into_string());
            ControlFlow::Break(())
        }
        WalkEvent::Finished => {
            finished = true;
            ControlFlow::Continue(())
        }
    });

    assert_eq!(seen, ["a.rs"]);
    assert!(
        !finished,
        "a walk nobody is waiting for does not report an end"
    );
}

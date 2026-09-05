use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::temporary::Temporary;

/// A `root/` directory of this test's own, empty, under the system's temp directory. It
/// and everything above it go when the test ends.
fn temp_dir(name: &str) -> Temporary {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    Temporary::under(
        std::env::temp_dir().join(format!(
            "viewer-files-{}-{unique}-{name}",
            std::process::id()
        )),
        "root",
    )
}

fn touch(path: &Path) {
    fs::write(path, b"").expect("the temp directory is writable");
}

/// The rows as text, which is what every assertion below is about: two spaces per level,
/// a directory marked by its fold — `+` folded, `-` unfolded, `!` failed.
fn described(rows: &FileRows) -> Vec<String> {
    rows.rows()
        .iter()
        .map(|row| {
            let indent = "  ".repeat(row.depth);
            let fold = match row.fold {
                None => "",
                Some(Fold::Folded) => "/ +",
                Some(Fold::Unfolded) => "/ -",
                Some(Fold::Failed) => "/ !",
            };
            format!("{indent}{}{fold}", row.name)
        })
        .collect()
}

/// `root/` holding `src/main.rs`, `src/ui/mod.rs` and `Cargo.toml`.
fn project(name: &str) -> Temporary {
    let root = temp_dir(name);
    fs::create_dir_all(root.join("src/ui")).expect("the temp directory is writable");
    touch(&root.join("src/main.rs"));
    touch(&root.join("src/ui/mod.rs"));
    touch(&root.join("Cargo.toml"));
    root
}

#[test]
fn a_fresh_tree_is_the_root_and_its_own_entries() {
    let root = project("fresh");
    let tree = FileTree::new(&root).expect("a readable directory");

    assert_eq!(
        described(&tree.rows()),
        ["root/ -", "  src/ +", "  Cargo.toml"]
    );
}

#[test]
fn unfolding_reads_one_level_and_folding_drops_it() {
    let root = project("unfold");
    let mut tree = FileTree::new(&root).expect("a readable directory");

    assert!(tree.toggle(&root.join("src")));
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  src/ -",
            "    ui/ +",
            "    main.rs",
            "  Cargo.toml"
        ]
    );

    assert!(tree.toggle(&root.join("src")));
    assert_eq!(
        described(&tree.rows()),
        ["root/ -", "  src/ +", "  Cargo.toml"]
    );
}

#[test]
fn a_refold_reads_the_directory_again() {
    let root = project("refold");
    let mut tree = FileTree::new(&root).expect("a readable directory");
    tree.toggle(&root.join("src"));

    // Nothing watches the disk: a file made after the read is not there until the
    // directory is read again, which is what folding and unfolding it does.
    touch(&root.join("src/lib.rs"));
    let before = described(&tree.rows());
    assert!(!before.iter().any(|row| row.contains("lib.rs")));

    tree.toggle(&root.join("src"));
    tree.toggle(&root.join("src"));
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  src/ -",
            "    ui/ +",
            "    lib.rs",
            "    main.rs",
            "  Cargo.toml"
        ]
    );
}

#[test]
fn toggling_the_root_refreshes_the_top_level() {
    let root = project("root");
    let mut tree = FileTree::new(&root).expect("a readable directory");

    assert!(tree.toggle(&root));
    assert_eq!(described(&tree.rows()), ["root/ +"]);

    touch(&root.join("README.md"));
    assert!(tree.toggle(&root));
    assert_eq!(
        described(&tree.rows()),
        ["root/ -", "  src/ +", "  Cargo.toml", "  README.md"]
    );
}

#[test]
fn directories_come_first_and_names_sort_without_regard_to_case() {
    let root = temp_dir("order");
    fs::create_dir_all(root.join("zdir")).expect("the temp directory is writable");
    fs::create_dir_all(root.join("Mdir")).expect("the temp directory is writable");
    touch(&root.join("b.txt"));
    touch(&root.join("A.txt"));
    touch(&root.join("Makefile"));
    touch(&root.join("main.rs"));

    let tree = FileTree::new(&root).expect("a readable directory");
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  Mdir/ +",
            "  zdir/ +",
            "  A.txt",
            "  b.txt",
            "  main.rs",
            "  Makefile"
        ]
    );
}

#[test]
fn a_root_that_is_not_a_directory_is_nothing() {
    let root = project("noroot");
    assert!(FileTree::new(&root.join("Cargo.toml")).is_none());
    assert!(FileTree::new(&root.join("missing")).is_none());
}

#[test]
fn toggling_a_file_changes_nothing() {
    let root = project("file");
    let mut tree = FileTree::new(&root).expect("a readable directory");
    let before = described(&tree.rows());

    assert!(!tree.toggle(&root.join("Cargo.toml")));
    assert!(!tree.toggle(&root.join("src/main.rs")));
    assert!(!tree.toggle(Path::new("/nowhere/at/all")));
    assert_eq!(described(&tree.rows()), before);
}

#[test]
fn unfolding_one_branch_leaves_another_where_it_was() {
    let root = project("branches");
    fs::create_dir_all(root.join("tests")).expect("the temp directory is writable");
    touch(&root.join("tests/it.rs"));
    let mut tree = FileTree::new(&root).expect("a readable directory");

    tree.toggle(&root.join("src"));
    tree.toggle(&root.join("tests"));
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  src/ -",
            "    ui/ +",
            "    main.rs",
            "  tests/ -",
            "    it.rs",
            "  Cargo.toml"
        ]
    );

    tree.toggle(&root.join("src"));
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  src/ +",
            "  tests/ -",
            "    it.rs",
            "  Cargo.toml"
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_read_is_a_failed_row_that_tries_again() {
    use std::os::unix::fs::PermissionsExt;

    let root = project("failed");
    let locked = root.join("locked");
    fs::create_dir_all(&locked).expect("the temp directory is writable");
    touch(&locked.join("secret.rs"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let mut tree = FileTree::new(&root).expect("a readable directory");
    tree.toggle(&locked);
    let rows = described(&tree.rows());
    if rows.iter().any(|row| row.contains("secret.rs")) {
        // Running as root, which reads anything: nothing here can be asserted.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        return;
    }
    assert_eq!(rows, ["root/ -", "  locked/ !", "  src/ +", "  Cargo.toml"]);

    // Made readable again, the next toggle is a read and not a fold.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(tree.toggle(&locked));
    assert_eq!(
        described(&tree.rows()),
        [
            "root/ -",
            "  locked/ -",
            "    secret.rs",
            "  src/ +",
            "  Cargo.toml"
        ]
    );
}

/// A press opens what the source cache would read: a regular file within its bound. A
/// directory, a missing path, or a file past the bound opens nothing.
#[test]
fn a_press_opens_what_the_source_cache_would_read() {
    let root = project("bound");
    let long = root.join("long.txt");
    fs::write(&long, "x".repeat(100)).expect("writable");

    assert!(shows_as_source_within(&long, 100));
    assert!(!shows_as_source_within(&long, 99));
    assert!(shows_as_source_within(&root.join("Cargo.toml"), 0));
    assert!(!shows_as_source_within(&root, u64::MAX));
    assert!(!shows_as_source_within(&root.join("missing"), u64::MAX));
}

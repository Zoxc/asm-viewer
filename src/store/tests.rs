use serde::Deserialize;

use super::*;
use crate::temporary::Temporary;

/// A directory of this test's own under the system temporary directory, named after the
/// line that asked for it, standing in for the one everything is stored in. Gone when the
/// test ends.
fn base(line: u32) -> Temporary {
    Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-store-test-{}-{line}",
        std::process::id()
    )))
}

/// A file with `data` in it, made along with the directories above it.
fn written(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("creating the test directory");
    fs::write(path, data).expect("writing the test file");
}

/// A schema of one key, so that "TOML, but not this file's shape" can be told from "not
/// TOML at all". [`MOVED`] is not asserted on: the tests share one process and one
/// static, so what any of them drained would be a race. The filesystem is the answer.
#[derive(Debug, Deserialize, PartialEq)]
struct Named {
    name: String,
}

#[test]
fn a_file_that_parses_is_left_alone() {
    let base = base(line!());
    let path = base.join("settings.toml");
    written(&path, b"name = \"a\"\n");

    assert_eq!(
        Store::at(&base).read::<Named>(&path),
        Some(Named { name: "a".into() })
    );
    assert!(path.exists());
    assert!(!base.join(INCOMPATIBLE_DIR).exists());
}

/// The mirror: a project's file keeps its project's directory rather than being flattened
/// into one heap of `session.toml`s.
#[test]
fn a_file_that_will_not_parse_is_moved_under_the_path_it_had() {
    let base = base(line!());
    let path = base.join("projects").join("project-1").join("session.toml");
    written(&path, b"{ not toml");

    assert_eq!(Store::at(&base).read::<Named>(&path), None);

    assert!(!path.exists(), "the original was left behind");
    let moved = base
        .join(INCOMPATIBLE_DIR)
        .join("projects")
        .join("project-1")
        .join("session.toml");
    assert_eq!(fs::read(&moved).ok().as_deref(), Some(&b"{ not toml"[..]));
}

/// TOML that this file's schema does not accept is a file that will not parse: a stale
/// one is exactly what the reader would otherwise lose without hearing about it.
#[test]
fn a_file_of_the_wrong_shape_is_moved_too() {
    let base = base(line!());
    let path = base.join("settings.toml");
    written(&path, b"other = 1\n");

    assert_eq!(Store::at(&base).read::<Named>(&path), None);
    assert!(base.join(INCOMPATIBLE_DIR).join("settings.toml").exists());
}

/// A file that is not text will not parse either, and is lost in the same way, so reading
/// it as bytes is what makes it rescuable at all.
#[test]
fn a_file_that_is_not_text_is_moved() {
    let base = base(line!());
    let path = base.join("recents.toml");
    written(&path, &[0xFF, 0xFE, 0x00]);

    assert_eq!(Store::at(&base).read::<Named>(&path), None);
    assert_eq!(
        fs::read(base.join(INCOMPATIBLE_DIR).join("recents.toml")).ok(),
        Some(vec![0xFF, 0xFE, 0x00])
    );
}

/// Nothing there is ever overwritten, so the second rescue of one name takes another.
#[test]
fn a_name_already_taken_gets_a_number() {
    let base = base(line!());
    let path = base.join("settings.toml");
    let moved = base.join(INCOMPATIBLE_DIR);

    written(&path, b"first");
    assert_eq!(Store::at(&base).read::<Named>(&path), None);
    written(&path, b"second");
    assert_eq!(Store::at(&base).read::<Named>(&path), None);
    written(&path, b"third");
    assert_eq!(Store::at(&base).read::<Named>(&path), None);

    assert_eq!(
        fs::read(moved.join("settings.toml")).ok().as_deref(),
        Some(&b"first"[..])
    );
    assert_eq!(
        fs::read(moved.join("2-settings.toml")).ok().as_deref(),
        Some(&b"second"[..])
    );
    assert_eq!(
        fs::read(moved.join("3-settings.toml")).ok().as_deref(),
        Some(&b"third"[..])
    );
}

/// A file that is not there, or that the system will not hand over, is not a file that
/// will not parse: there is nothing to rescue and nothing is about to write over it.
#[test]
fn a_missing_file_moves_nothing() {
    let base = base(line!());

    assert_eq!(
        Store::at(&base).read::<Named>(&base.join("settings.toml")),
        None
    );
    assert!(!base.join(INCOMPATIBLE_DIR).exists());
}

/// A path this app does not store is one it has no mirror for, and moving it would be
/// taking away a file that is somebody else's.
#[test]
fn a_path_outside_the_base_is_left_where_it_is() {
    let base = base(line!());
    let outside = base.join("elsewhere").join("settings.toml");
    written(&outside, b"{ not toml");

    assert_eq!(Store::at(&base.join("state")).read::<Named>(&outside), None);
    assert!(outside.exists());
}

/// The variable that points this app's storage somewhere of its own, so a second copy does
/// not write over the first's. Its parsing is what is tested here and not the reading of it:
/// an environment is one per process and the tests run many at once, so setting one would be
/// a test that broke whichever others happened to be looking.
#[test]
fn a_state_directory_can_be_given_and_an_empty_one_is_not_given() {
    assert_eq!(
        given_base(Some("/tmp/somewhere".into())),
        Some(PathBuf::from("/tmp/somewhere"))
    );
    // Unset and empty are the same answer: a variable set to nothing is a script that
    // meant to set it and did not, and taking it as a path would put a reader's projects
    // in whatever directory the app was started from.
    assert_eq!(given_base(Some("".into())), None);
    assert_eq!(given_base(None), None);
}

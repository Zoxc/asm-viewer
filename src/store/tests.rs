use serde::Deserialize;

use super::*;
use crate::temporary::Temporary;

impl Store {
    /// A store at a given directory: what a test points at one of its own, in place of
    /// the twin every operation under here used to have for exactly that.
    pub fn at(base: impl AsRef<Path>) -> Store {
        Store::new(base.as_ref().to_path_buf())
    }
}

/// The temporaries [`write_atomically`] left in `directory`, which is none once every write
/// has answered, whether it succeeded or not.
pub fn temporaries(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .expect("reading the test directory")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "tmp"))
        .collect()
}

/// A file with `data` in it, made along with the directories above it.
fn written(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().expect("a parent")).expect("creating the test directory");
    fs::write(path, data).expect("writing the test file");
}

/// A schema of one key, so that "TOML, but not this file's shape" can be told from "not
/// TOML at all". [`MOVED`] is not asserted on: the tests share one process and one
/// static, so what any of them drained would be a race. The filesystem is the answer.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct Named {
    name: String,
}

#[test]
fn a_file_that_parses_is_left_alone() {
    let base = Temporary::fresh("store-test");
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
    let base = Temporary::fresh("store-test");
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
    let base = Temporary::fresh("store-test");
    let path = base.join("settings.toml");
    written(&path, b"other = 1\n");

    assert_eq!(Store::at(&base).read::<Named>(&path), None);
    assert!(base.join(INCOMPATIBLE_DIR).join("settings.toml").exists());
}

/// A file that is not text will not parse either, and is lost in the same way, so reading
/// it as bytes is what makes it rescuable at all.
#[test]
fn a_file_that_is_not_text_is_moved() {
    let base = Temporary::fresh("store-test");
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
    let base = Temporary::fresh("store-test");
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
    let base = Temporary::fresh("store-test");

    assert_eq!(
        Store::at(&base).read::<Named>(&base.join("settings.toml")),
        None
    );
    assert!(!base.join(INCOMPATIBLE_DIR).exists());
}

/// A file the app keeps outside the store -- the session beside a project the reader gave
/// a place -- is the app's all the same, and the next write would replace it just as
/// surely. It is moved aside under `outside/`, at its whole path less the root.
#[test]
fn a_path_outside_the_base_is_moved_aside_too() {
    let base = Temporary::fresh("store-test");
    let outside = base.join("elsewhere").join("settings.toml");
    written(&outside, b"{ not toml");

    let state = base.join("state");
    assert_eq!(Store::at(&state).read::<Named>(&outside), None);
    assert!(!outside.exists());

    let named: PathBuf = outside
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .collect();
    let moved = state.join(INCOMPATIBLE_DIR).join(OUTSIDE_DIR).join(named);
    assert_eq!(fs::read(&moved).ok().as_deref(), Some(&b"{ not toml"[..]));
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

    // A relative one is taken against the directory the app was started from, once: a path
    // the store hands out has to come back through `Store::path` unchanged.
    let relative = given_base(Some("state".into())).expect("a relative directory is given");
    assert_eq!(
        relative,
        std::env::current_dir().expect("a directory").join("state")
    );
    let store = Store::at(&relative);
    assert_eq!(store.path(store.projects()), store.projects());
}

/// The join and its inverse are one rule: what [`Store::path`] made absolute comes back as
/// it was given, and a path the store does not hold is not made relative at all. Path work
/// only, so nothing is written.
#[test]
fn a_path_is_relative_to_the_store_only_where_it_is_under_it() {
    let store = Store::at("/state/assembly-viewer");

    let under = store.path("projects/1.avproj");
    assert_eq!(store.relative(&under), Some(Path::new("projects/1.avproj")));

    // A prefix of the directory's name is not the directory.
    assert_eq!(
        store.relative(Path::new("/state/assembly-viewer-2/x")),
        None
    );
    assert_eq!(store.relative(Path::new("/elsewhere/1.avproj")), None);
}

/// **The cap on an order file is the store's**, applied on the way out. A module that keeps
/// one hands over whatever it is holding and the file stops at [`MAX_ORDER`], so a third
/// order file cannot be the one that forgets the number.
#[test]
fn an_order_is_cut_to_the_cap_as_it_is_written() {
    let base = Temporary::fresh("store-test");
    let store = Store::at(&base);

    let over: Order<String> = (0..MAX_ORDER + 10).map(|n| format!("e{n}")).collect();
    store.save_order(RECENTS_FILE, over);

    let written: Order<String> = store.read(RECENTS_FILE).expect("the order was written");
    assert_eq!(written.len(), MAX_ORDER);
    assert_eq!(written.first(), Some(&"e0".to_owned()));
    assert_eq!(
        written.entries().last(),
        Some(&format!("e{}", MAX_ORDER - 1))
    );
}

/// A save that cannot happen is logged and swallowed, and what it leaves is the good file
/// that was already there. That is what these files are: an order or a setting one save out
/// of date is one the app carries on from, where a project is not.
#[test]
fn a_save_that_fails_leaves_the_file_that_was_there() {
    let base = Temporary::fresh("store-test");
    let store = Store::at(&base);
    written(&base.join("settings.toml"), b"name = \"a\"\n");

    // TOML has no spelling for a file that is one number, so this save cannot happen.
    store.save("settings.toml", &5);

    assert_eq!(
        store.read::<Named>("settings.toml"),
        Some(Named { name: "a".into() })
    );
}

/// Two writes of one file at once, as two apps on one store make of `recents.toml`, each
/// land whole. A temporary they shared was one file both wrote into, and what the rename
/// put in place was the shorter write followed by the tail of the longer one.
#[test]
fn writes_of_one_file_at_once_each_land_whole() {
    let base = Temporary::fresh_directory("store-test");
    let path = base.join(RECENTS_FILE);
    let long = vec![b'a'; 64 * 1024];
    let short = vec![b'b'; 1024];

    std::thread::scope(|scope| {
        for contents in [&long, &short] {
            let path = &path;
            scope.spawn(move || {
                for _ in 0..50 {
                    write_atomically(path, contents).expect("every write lands");
                }
            });
        }
    });

    let landed = fs::read(&path).expect("the file reads");
    assert!(landed == long || landed == short, "a spliced file");
    assert_eq!(temporaries(&base), Vec::<PathBuf>::new());
}

/// A write that fails takes its temporary with it: here the rename, over a directory.
#[test]
fn a_write_that_fails_leaves_no_temporary() {
    let base = Temporary::fresh_directory("store-test");
    let path = base.join("settings.toml");
    written(&path.join("inside"), b"");

    assert!(write_atomically(&path, b"name = \"a\"\n").is_err());
    assert_eq!(temporaries(&base), Vec::<PathBuf>::new());
}

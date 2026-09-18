//! Entering a project, reopening one, putting one somewhere else, and what one that
//! will not open says.

use super::*;
use crate::project::files::tests::*;
use crate::store::PROJECTS_DIR;

/// What is asked of a path before a project is opened from it: the extension and nothing
/// else, since a file has to be recognised before it is read. What is *in* it is a separate
/// answer, and a separate way of saying no.
#[test]
fn a_project_file_is_known_by_its_extension() {
    assert!(is_project_file(Path::new("/src/kernel/kernel.avproj")));
    assert!(is_project_file(Path::new("1.avproj")));
    for bad in [
        "/src/kernel",
        "kernel.toml",
        "kernel.avproj.session",
        ".avproj/x",
    ] {
        assert!(!is_project_file(Path::new(bad)), "{bad}");
    }
}

/// Giving a project a place writes what `Saves` holds, not what the old files hold. The two
/// differ twice over. A project just **started** has an empty file, so its id is the app's
/// alone until a write puts it there -- and a project put somewhere without one is a project
/// whose session the next load throws away, the two being matched by id. A project just
/// **opened** has the other half of it: the baseline every change is measured against is the
/// stub `opened` seeded, and only `stored` says what the session file holds.
///
/// The one test here that goes through the `SAVES` static; every other builds a `Saves` of
/// its own, and nothing else in the suite touches it.
#[test]
fn putting_a_project_somewhere_carries_the_id_and_the_session() {
    let base = directory(line!());
    let store = Store::at(&base);
    let from = start_new(&store).expect("a project is started");
    assert_eq!(fs::read(&from).expect("the claimed file"), b"");

    // A session worth carrying across, left pending until the flush inside `put_in`, and
    // a directory typed in, owed to that flush too: a Save pressed straight after typing
    // keeps what was typed.
    let typed = Details {
        directory: Some(PathBuf::from("/src/kernel")),
        ..Details::default()
    };
    record(&typed, &[], false, &[], session_with(Some("a.o")));

    let to = base.join(format!("kernel.{PROJECT_EXTENSION}"));
    assert!(put_in(&store, &to, Put::Move), "the project was written");

    let (project, session) = load_project(&store, &to).expect("the project reads back");
    assert!(project.id.is_some(), "the id the app gave it");
    assert_eq!(project.details, typed, "the directory came with it");
    assert_eq!(
        session.active,
        Some(saved_object("a.o")),
        "the session came with it"
    );

    // Opened again and copied elsewhere before anything has been recorded, which is the
    // window a load in flight holds open for as long as it runs.
    open_at(&store, &to).expect("the project opens");
    let elsewhere = base.join(format!("copy.{PROJECT_EXTENSION}"));
    assert!(
        put_in(&store, &elsewhere, Put::Copy),
        "the copy was written"
    );

    let (copy, carried) = load_project(&store, &elsewhere).expect("the copy reads back");
    assert_ne!(copy.id, project.id, "a copy is a project of its own");
    assert_eq!(
        carried.active,
        Some(saved_object("a.o")),
        "and it has the session the file held"
    );
}

/// **Nothing makes a project but the reader asking for one.** With none open the two write
/// paths do nothing at all: no file is claimed and none is remembered. The app used to claim
/// one on the first write that had anything to say, which turned arranging the window, or
/// opening Settings, into a project appearing on disk behind the reader's back.
#[test]
fn no_project_open_means_nothing_is_written_and_nothing_is_made() {
    let base = directory(line!());
    let store = Store::at(&base);
    let mut saves = Saves::default();

    // A change the app would otherwise write at once, and a session that would go pending.
    let decided = saves.record(
        &Details {
            directory: Some(PathBuf::from("/src/kernel")),
            ..Details::default()
        },
        &paths(&["/tmp/lib.a"]),
        false,
        &[],
        session_with(Some("a.o")),
    );
    assert!(decided.is_some(), "the change was noticed");
    assert_eq!(
        writing_into(&saves),
        None,
        "but there is nowhere to write it"
    );

    assert!(!store.projects().exists(), "a project was made anyway");
    assert_eq!(load_recents(&store).entries(), Vec::<PathBuf>::new());
}

/// Startup: the front of the recent list, both halves of it.
#[test]
fn the_last_project_is_the_one_reopened() {
    let base = directory(line!());
    let store = Store::at(&base);
    let project = a_project();
    let session = Session {
        id: project.id,
        active: Some(saved_object("a.o")),
        ..Session::default()
    };

    let wanted = store.projects().join(format!("wanted.{PROJECT_EXTENSION}"));
    for name in ["other", "wanted"] {
        let path = store.projects().join(format!("{name}.{PROJECT_EXTENSION}"));
        store
            .write_toml(&path, &project)
            .expect("saving the project");
        session
            .save_to(&store, &session_beside(&path))
            .expect("saving the session");
        remember(&store, &path);
    }

    let (path, reopened, restored) = reopen(&store)
        .expect("a project to reopen")
        .expect("it opens");
    assert_eq!(path, wanted);
    assert_eq!(reopened, project);
    assert_eq!(restored, session);
}

/// A project opens under the path it was named by. Nothing on the way through
/// canonicalises or reduces one, so a path the reader spelled the long way round is the
/// path the project is opened and remembered under -- which is why [`open_at`] hands back
/// the two halves and not the path it was given.
#[test]
fn a_project_opens_under_the_path_it_was_named_by() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = base.join(format!("kernel.{PROJECT_EXTENSION}"));
    let project = a_project();
    store
        .write_toml(&path, &project)
        .expect("saving the project");

    // The same file, named through a directory walked into and back out of: a spelling
    // the system resolves and `canonicalize` would reduce.
    fs::create_dir_all(base.join("sub")).expect("creating the test directory");
    let named = base
        .join("sub")
        .join("..")
        .join(format!("kernel.{PROJECT_EXTENSION}"));
    assert_ne!(
        named, path,
        "the two spellings are the same file, not the same path"
    );

    let (opened, _) = open_at(&store, &named).expect("the project opens");
    assert_eq!(opened, project);
    assert_eq!(
        load_recents(&store).first(),
        Some(&named),
        "the recent list holds a path the caller never gave"
    );
}

/// Two ways for there to be nothing to reopen, both of them silence -- and the one way a
/// startup does have something to say, which is the point of telling them apart. The
/// recent list never prunes itself, so a name in it with nothing behind it is what an
/// ordinary startup after a deleted project looks like; a file that is *there* and will
/// not open is news.
#[test]
fn nothing_to_reopen_is_not_an_error() {
    let base = directory(line!());
    let store = Store::at(&base);
    // No recent list at all: a first run, or one whose file was deleted.
    assert!(reopen(&store).is_none());

    // A recent list naming a project whose file has gone.
    let gone = store.projects().join(format!("gone.{PROJECT_EXTENSION}"));
    remember(&store, &gone);
    assert!(reopen(&store).is_none());

    // The same name, with a file behind it that will not parse.
    fs::create_dir_all(store.projects()).expect("creating the test directory");
    fs::write(&gone, b"{ not toml").expect("writing");
    let failure = reopen(&store)
        .expect("something to say")
        .expect_err("it does not open");
    assert_eq!(failure.path, gone);
}

/// The file *is* the project, so a run killed between claiming one and writing anything
/// into it reopens as the empty project it is rather than being orphaned. A corrupt session
/// beside it is the same answer.
#[test]
fn a_project_missing_a_half_still_reopens() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = unsaved_project(&store).expect("a project");
    remember(&store, &path);

    // The file claimed and nothing written into it yet.
    let (reopened, project, session) = reopen(&store)
        .expect("a project to reopen")
        .expect("it opens");
    assert_eq!(reopened, path);
    assert_eq!(project, Project::default());
    assert_eq!(session, Session::default());

    // The user's half good, the app's half corrupt.
    let project = a_project();
    store
        .write_toml(&path, &project)
        .expect("saving the project");
    fs::write(session_beside(&path), b"{ not toml").expect("writing the corrupt half");

    let (_, reopened, session) = reopen(&store)
        .expect("a project to reopen")
        .expect("it opens");
    assert_eq!(reopened, project);
    assert_eq!(session, Session::default());

    // And the corrupt half was moved aside under the path it had, rather than left for
    // the next flush to write over.
    let moved = base
        .join(crate::store::INCOMPATIBLE_DIR)
        .join(PROJECTS_DIR)
        .join(
            session_beside(&path)
                .file_name()
                .expect("the session's name"),
        );
    assert_eq!(fs::read(&moved).ok().as_deref(), Some(&b"{ not toml"[..]));
}

/// The project file is the reader's own, wherever it is kept, so one that will not parse is
/// **not** moved aside: the project simply does not open, and nothing writes over what
/// could not be read. Telling the reader is therefore the whole of what happens, so what
/// is answered instead is where the parser stopped and what it said there.
#[test]
fn a_project_file_that_will_not_parse_is_left_where_it_is() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = store.projects().join(format!("1.{PROJECT_EXTENSION}"));
    fs::create_dir_all(store.projects()).expect("creating the test directory");
    let text = b"binaries = []\n{ not toml\n";
    fs::write(&path, text).expect("writing");

    let failure = load_project(&store, &path).expect_err("the project does not open");
    assert_eq!(failure.path, path);
    let Reason::Malformed { at, message } = &failure.reason else {
        panic!("a file that will not parse: {}", failure.reason);
    };
    // The second line, where the keys that do parse stop.
    assert_eq!(*at, Some((2, 1)));
    assert!(!message.is_empty(), "the parser said nothing");
    // And the sentence the reader is shown carries both.
    let said = failure.reason.to_string();
    assert!(said.contains("line 2, column 1"), "{said}");
    assert!(said.contains(message.as_str()), "{said}");

    assert_eq!(fs::read(&path).expect("the file is still there"), text);
    assert!(!base.join(crate::store::INCOMPATIBLE_DIR).exists());
}

/// The other ways one does not open, told apart. Which it was decides what the window
/// says, and a file that is not there is the one the startup keeps to itself.
#[test]
fn a_project_that_does_not_open_says_which_way() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = store.projects().join(format!("1.{PROJECT_EXTENSION}"));
    fs::create_dir_all(store.projects()).expect("creating the test directory");

    assert_eq!(
        load_project(&store, &path)
            .expect_err("nothing is there")
            .reason,
        Reason::Missing
    );

    // Not text at all, which is not a parse failure and cannot be given a place in the
    // file: the bytes are what is read for exactly this.
    fs::write(&path, [0xff, 0xfe, 0x00, 0x41]).expect("writing");
    assert_eq!(
        load_project(&store, &path)
            .expect_err("it is not text")
            .reason,
        Reason::NotText
    );
}

/// The session is found by the project file's name, which says nothing about whether that
/// file still holds the project it held. So one carrying another id is dropped whole
/// rather than opened over a project it was never written for.
#[test]
fn a_session_written_for_another_project_is_ignored() {
    let base = directory(line!());
    let store = Store::at(&base);
    let path = store.projects().join(format!("1.{PROJECT_EXTENSION}"));
    fs::create_dir_all(store.projects()).expect("creating the test directory");

    let mine = ProjectId::parse("00000000deadbeef").expect("an id");
    let project = Project {
        id: Some(mine),
        ..a_project()
    };
    store
        .write_toml(&path, &project)
        .expect("saving the project");

    let session = Session {
        active: Some(saved_object("a.o")),
        ..Session::default()
    };
    let theirs = Session {
        id: ProjectId::parse("000000000badcafe"),
        ..session.clone()
    };
    theirs
        .save_to(&store, &session_beside(&path))
        .expect("saving the session");

    let (_, restored) = load_project(&store, &path).expect("the project opens");
    assert_eq!(restored, Session::default());

    // The same session under this project's own id is read.
    Session {
        id: Some(mine),
        ..session.clone()
    }
    .save_to(&store, &session_beside(&path))
    .expect("saving the session");
    let (_, restored) = load_project(&store, &path).expect("the project opens");
    assert_eq!(restored.active, session.active);
}

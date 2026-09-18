//! The two files, as serde reads and writes them.
//!
//! Also the fixtures the rest of the module's tests are built from -- the
//! `pub(in crate::project)` ones below. They are values of the schema in `files.rs`, so
//! they are made here and `restore`, `recents`, `saves` and the lifecycle build on them.

use std::collections::BTreeMap;

use super::*;
use crate::store::write_atomically;
use crate::temporary::Temporary;

/// A store to write a file through where the path is the whole of the question: these
/// files are the reader's own and are given absolutely, so which store writes them makes
/// no difference to what lands.
fn anywhere() -> Store {
    Store::at("/state")
}

/// What `load_project` does with `session.toml`: a missing or corrupt file is `None`.
fn load_session(path: &Path) -> Option<Session> {
    let data = fs::read_to_string(path).ok()?;
    toml::from_str(&data).ok()
}

/// Serialize to TOML and read it straight back, which is the only way to catch the `toml`
/// crate's runtime failures -- a bare `None` among them.
pub(in crate::project) fn round_trip<T>(value: &T) -> String
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let text = toml::to_string_pretty(value).expect("serializing");
    let back: T = toml::from_str(&text).unwrap_or_else(|error| {
        panic!("deserializing\n--- {text}--- failed: {error}");
    });
    assert_eq!(*value, back);
    text
}

/// The digest of the objects the fixtures are built from, as it is written down.
pub(in crate::project) fn digest_of(bytes: &[u8]) -> String {
    analysis::FileDigest::of(bytes).to_string()
}

pub(in crate::project) fn paths(binaries: &[&str]) -> Vec<PathBuf> {
    binaries.iter().map(PathBuf::from).collect()
}

/// A directory of this test's own, named after the line that asked for it, and gone when
/// the test ends.
pub(in crate::project) fn directory(line: u32) -> Temporary {
    Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-project-test-{}-{line}",
        std::process::id()
    )))
}

/// A project file under a `projects/` directory the test never makes: the path is the
/// identity, and nothing here reads it.
pub(in crate::project) fn kept_at(name: &str) -> PathBuf {
    PathBuf::from(format!("/state/projects/{name}.{PROJECT_EXTENSION}"))
}

pub(in crate::project) fn a_project() -> Project {
    Project {
        // A project the app wrote always has one, which is what the session beside it is
        // matched against.
        id: ProjectId::parse("00000000deadbeef"),
        details: Details {
            directory: Some(PathBuf::from("/src/kernel")),
            language_server: Some("ra-multiplex".into()),
            language_files: Some("rs".into()),
            cargo: None,
        },
        binaries: paths(&["/tmp/lib.a", "/tmp/some.dll"]),
        bookmarks: Vec::new(),
    }
}

pub(in crate::project) fn session_with(selection: Option<&str>) -> Session {
    Session {
        active: selection.map(saved_object),
        history: SavedHistory::default(),
        ..Session::default()
    }
}

pub(in crate::project) fn saved_object(name: &str) -> SavedDocument {
    SavedDocument::Object {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: name.to_owned(),
        shown: SavedShown::Symbols,
    }
}

pub(in crate::project) fn saved_symbol(
    object_name: &str,
    symbol_name: &str,
    address: u64,
) -> SavedDocument {
    SavedDocument::Symbol {
        path: PathBuf::from("/tmp/lib.a"),
        object_name: object_name.to_owned(),
        symbol_name: SavedName::File(symbol_name.to_owned()),
        address,
    }
}

/// A saved place with its assembly row and nothing else.
pub(in crate::project) fn saved_entry(document: SavedDocument, asm_row: usize) -> SavedEntry {
    SavedEntry {
        asm_row,
        src_row: 0,
        line: None,
        asm_address: None,
        code_address: None,
        src_line: None,
        document,
    }
}

/// A saved tab that stays, with `entry` alone on its trail.
pub(in crate::project) fn saved_one(entry: SavedEntry) -> SavedTab {
    SavedTab {
        page: None,
        temporal: false,
        cursor: 0,
        entries: vec![entry],
    }
}

pub(in crate::project) fn saved_tab(object_name: &str, asm_row: usize) -> SavedTab {
    saved_one(saved_entry(saved_object(object_name), asm_row))
}

#[test]
fn toml_round_trips() {
    let session = Session {
        active: Some(SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "b.o".into(),
            symbol_name: SavedName::File("caller".into()),
            address: 0x1234,
        }),
        history: SavedHistory::default(),
        ..Session::default()
    };
    let text = round_trip(&session);
    // The externally tagged enum is a table named after its variant.
    assert!(text.contains("[active.Symbol]"), "{text}");
}

#[test]
fn an_empty_session_round_trips() {
    // The `None` the `toml` crate cannot write has to be left out of the file entirely,
    // and read back as `None`.
    let session = Session::default();
    let text = round_trip(&session);
    assert!(!text.contains("active"), "{text}");
}

#[test]
fn writes_atomically_and_reads_back() {
    let directory = Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-test-{}-{}",
        std::process::id(),
        line!()
    )));
    let path = directory.join("nested").join("one.avproj.session");

    let session = Session {
        active: Some(SavedDocument::Object {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            shown: SavedShown::Symbols,
        }),
        history: SavedHistory::default(),
        ..Session::default()
    };
    Store::at(&directory)
        .write_toml(&path, &session)
        .expect("saving");

    assert_eq!(load_session(&path), Some(session));
    // The temporary was renamed, not left behind.
    assert!(!path.with_extension("toml.tmp").exists());
}

/// The rename is the last thing `write_atomically` does, so nothing that goes wrong on the
/// way to it can touch the file already there: the good one is still readable afterwards,
/// and the reader loses the save rather than the file.
///
/// What this cannot see is the `sync_all` itself, which is the point of the ordering: no
/// test in a process can observe whether the data reached the disk before the directory
/// entry did. It pins that a failure before the rename is an error and not a replacement.
#[test]
fn a_write_that_fails_leaves_the_good_file_where_it_is() {
    let directory = Temporary::at(std::env::temp_dir().join(format!(
        "assembly-viewer-test-{}-{}",
        std::process::id(),
        line!()
    )));
    let _ = fs::remove_dir_all(&directory);
    let path = directory.join("one.avproj");

    write_atomically(&path, b"the good file").expect("the first write");
    assert!(
        !path.with_extension("avproj.tmp").exists(),
        "a temporary was left"
    );

    // A temporary that cannot be created at all: a directory is already sitting there.
    fs::create_dir_all(path.with_extension("avproj.tmp")).expect("the temporary's stand-in");
    assert!(write_atomically(&path, b"the new file").is_err());
    assert_eq!(fs::read(&path).expect("the file reads"), b"the good file");
}

/// A path TOML cannot spell is refused rather than mangled, in *both* files.
#[test]
fn a_non_utf8_path_is_not_written_rather_than_mangled() {
    // Only Unix has a `PathBuf` that can hold one at all.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe.a"));
        let project = Project {
            binaries: vec![path.clone()],
            ..Project::default()
        };
        let session = Session {
            digests: BTreeMap::from([(path, digest_of(b"whatever"))]),
            ..Session::default()
        };
        // An error, not a panic and not a lossy path silently written in its place.
        assert!(toml::to_string_pretty(&project).is_err());
        assert!(toml::to_string_pretty(&session).is_err());

        let directory = Temporary::at(std::env::temp_dir().join(format!(
            "assembly-viewer-test-{}-{}",
            std::process::id(),
            line!()
        )));
        let store = Store::at(&directory);
        assert!(store
            .write_toml(directory.join("one.avproj"), &project)
            .is_err());
        assert!(session
            .save_to(&store, &directory.join("one.avproj.session"))
            .is_err());
        // Nothing reached the disk, so a good earlier file would still be there.
        assert!(!directory.join("one.avproj").exists());
        assert!(!directory.join("one.avproj.session").exists());
    }
}

/// The project half through a real serializer. [`Details`] is **flattened**, so what the
/// reader said is keys of the file rather than a `[details]` table of their own, and the
/// file reads back as what was written.
#[test]
fn a_project_round_trips_through_toml() {
    let project = Project {
        details: Details {
            cargo: Some(Cargo {
                profile: Profile::Debug,
            }),
            ..a_project().details
        },
        ..a_project()
    };
    let text = round_trip(&project);
    assert!(!text.contains("[details]"), "{text}");
    for key in [
        "id = ",
        "directory = ",
        "language_server = ",
        "language_files = ",
        "binaries = ",
    ] {
        assert!(text.contains(key), "no {key} in\n{text}");
    }
    // The flattened table is a table of the file, wherever the struct puts it.
    assert!(text.contains("[cargo]"), "{text}");
}

/// What the reader has not said is an *absent* key, never an empty one a later reader could
/// mistake for something they chose.
#[test]
fn what_was_never_said_writes_no_key() {
    let project = Project {
        binaries: paths(&["/tmp/lib.a"]),
        ..Project::default()
    };
    let text = round_trip(&project);
    assert!(!text.contains("directory"), "{text}");
    // And so is the language server: absent means the usual one, and an empty key would
    // be a program named "".
    assert!(!text.contains("language_server"), "{text}");
    // The agreement to run one is not in this file at all: it is the machine's answer and
    // must not travel with a project that is shared.
    assert!(!text.contains("trusted"), "{text}");

    // It is the session's, and absent there is the "no" a directory nobody has been asked
    // about has to have.
    assert!(!round_trip(&Session::default()).contains("trusted"));
    let agreed = Session {
        trusted: true,
        ..Session::default()
    };
    assert!(round_trip(&agreed).contains("trusted = true"));
}

/// A path under the project file's own directory is written **relative to it**, so a
/// project checked in beside the code it is about opens on another machine; a path outside
/// that tree has nothing to be relative to and stays absolute. The app works in absolute
/// paths either way, so what goes in comes back out.
#[test]
fn a_path_under_the_project_file_is_written_relative_to_it() {
    let directory = directory(line!());
    fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("kernel.avproj");

    let project = Project {
        id: ProjectId::parse("00000000deadbeef"),
        details: Details {
            directory: Some(directory.to_path_buf()),
            ..Details::default()
        },
        binaries: vec![
            directory.join("target/debug/vmlinux"),
            "/usr/lib/libc.so".into(),
        ],
        bookmarks: vec![Bookmark {
            name: Some("start".to_owned()),
            document: SavedDocument::Object {
                path: directory.join("target/debug/vmlinux"),
                object_name: "vmlinux".into(),
                shown: SavedShown::Symbols,
            },
        }],
    };
    project
        .save_to(&anywhere(), &path)
        .expect("saving the project");

    let text = fs::read_to_string(&path).expect("reading");
    assert!(text.contains(r#""target/debug/vmlinux""#), "{text}");
    assert!(text.contains(r#""/usr/lib/libc.so""#), "{text}");
    // The project's own directory *is* the file's, which is the empty path.
    assert!(
        !text.contains(&directory.to_string_lossy().into_owned()),
        "{text}"
    );

    assert_eq!(Project::load_from(&path), Ok(project));
}

/// And the point of it: the same file read from somewhere else answers about that place,
/// which is what a project checked in beside its code has to do.
#[test]
fn a_project_file_moved_with_its_tree_points_at_the_new_one() {
    let here = directory(line!());
    let there = directory(line!() + 1000);
    fs::create_dir_all(&here).expect("creating the test directory");
    fs::create_dir_all(&there).expect("creating the second test directory");

    let project = Project {
        details: Details {
            directory: Some(here.to_path_buf()),
            ..Details::default()
        },
        binaries: vec![here.join("target/debug/vmlinux")],
        ..Project::default()
    };
    project
        .save_to(&anywhere(), &here.join("kernel.avproj"))
        .expect("saving");
    fs::copy(here.join("kernel.avproj"), there.join("kernel.avproj")).expect("copying");

    let moved = Project::load_from(&there.join("kernel.avproj")).expect("reading it back");
    assert_eq!(moved.details.directory, Some(there.to_path_buf()));
    assert_eq!(moved.binaries, vec![there.join("target/debug/vmlinux")]);
}

/// The split seen from the disk: each half in its own file, neither holding a word of the
/// other's.
#[test]
fn the_two_halves_are_written_to_their_own_files() {
    let directory = directory(line!());
    let project = a_project();
    let session = Session {
        active: Some(saved_object("a.o")),
        tabs: vec![saved_tab("a.o", 7)],
        ..Session::default()
    };

    Store::at(&directory)
        .write_toml(directory.join("one.avproj"), &project)
        .expect("saving the project");
    session
        .save_to(&anywhere(), &directory.join("one.avproj.session"))
        .expect("saving the session");

    let project_text = fs::read_to_string(directory.join("one.avproj")).expect("reading");
    let session_text = fs::read_to_string(directory.join("one.avproj.session")).expect("reading");
    assert!(project_text.contains("/tmp/lib.a"), "{project_text}");
    assert!(!project_text.contains("active"), "{project_text}");
    assert!(session_text.contains("active"), "{session_text}");
    assert!(!session_text.contains("binaries"), "{session_text}");

    assert_eq!(
        Project::load_from(&directory.join("one.avproj")),
        Ok(project)
    );
    assert_eq!(
        load_session(&directory.join("one.avproj.session")),
        Some(session)
    );
}

/// Why the split is worth two files: the half the app rewrites every thirty seconds cannot
/// take the half the user gave down with it.
#[test]
fn a_corrupt_session_leaves_the_project_readable() {
    let directory = directory(line!());
    let project = a_project();
    Store::at(&directory)
        .write_toml(directory.join("one.avproj"), &project)
        .expect("saving the project");
    fs::write(directory.join("one.avproj.session"), b"{ not toml")
        .expect("writing the corrupt half");

    assert_eq!(
        Project::load_from(&directory.join("one.avproj")),
        Ok(project)
    );
    assert_eq!(load_session(&directory.join("one.avproj.session")), None);
}

/// An id is written and read as sixteen hex digits, and anything else is not one. Strict
/// on the way in because what it decides is whether a session is believed.
#[test]
fn an_id_is_sixteen_hex_digits() {
    let id = ProjectId(0x0123_4567_89ab_cdef);
    assert_eq!(id.to_string(), "0123456789abcdef");
    assert_eq!(ProjectId::parse("0123456789abcdef"), Some(id));
    // Leading zeroes are written, so the text is always the same length.
    assert_eq!(ProjectId(1).to_string(), "0000000000000001");

    for bad in [
        "",
        "1",
        "0123456789abcdefg",
        "0123456789ABCDEF ",
        "zzzzzzzzzzzzzzzz",
    ] {
        assert_eq!(ProjectId::parse(bad), None, "{bad}");
    }
}

/// The id is what says a session belongs to the project it sits beside, so it has to
/// survive the file it is written into.
#[test]
fn an_id_round_trips_through_toml() {
    let project = Project {
        id: ProjectId::parse("00000000deadbeef"),
        ..Project::default()
    };
    let text = round_trip(&project);
    assert!(text.contains("00000000deadbeef"), "{text}");
}

/// A bookmark is what the user said, so it is `project.toml`'s, written as an array of
/// tables and read back as what went in.
#[test]
fn bookmarks_are_written_to_the_project_file_and_read_back() {
    let project = Project {
        bookmarks: vec![
            Bookmark {
                name: Some("kernel::start".into()),
                document: saved_symbol("a.o", "_ZN6kernel5startE", 6),
            },
            Bookmark {
                name: Some("main.rs".into()),
                document: SavedDocument::Source {
                    path: "/src/main.rs".into(),
                },
            },
        ],
        ..a_project()
    };
    let text = round_trip(&project);

    assert!(text.contains("[[bookmarks]]"), "{text}");
    assert!(text.contains("name = \"kernel::start\""), "{text}");
    // Each document is a table named after its variant.
    assert!(text.contains("[bookmarks.document.Symbol]"), "{text}");
    assert!(text.contains("[bookmarks.document.Source]"), "{text}");

    // And none at all is a key that is absent, not an empty list.
    assert!(!round_trip(&a_project()).contains("bookmarks"));
}

/// So nothing of that spelling reaches the file: the bookmark is the address and which name
/// it is, and it carries no name of its own, the place being able to say what it is called.
#[test]
fn a_bookmark_on_a_made_up_name_writes_no_spelling() {
    let spelling = MadeUp::Function(0x10).to_string();
    let bookmark = Bookmark::new(
        SavedDocument::Symbol {
            path: PathBuf::from("/tmp/lib.a"),
            object_name: "a.o".into(),
            address: 0x10,
            symbol_name: SavedName::MadeUp(SavedMadeUp::Function),
        },
        spelling.clone(),
    );
    assert_eq!(bookmark.label(), spelling.as_str());

    let project = Project {
        bookmarks: vec![bookmark],
        ..a_project()
    };
    let text = round_trip(&project);
    let written = &text[text.find("[[bookmarks]]").expect("a bookmark")..];
    assert!(!written.contains(&spelling), "the spelling is in\n{text}");
    assert!(!written.contains("\nname = "), "a name of its own\n{text}");
    assert!(written.contains("MadeUp = \"Function\""), "{text}");
}

/// The section a build puts in each file, as it is spelled: `[cargo]` in both, a profile
/// by name in the project's and the paths in the session's, both read back as they were
/// written. The profile is written as a word rather than as a number, so a file a reader
/// opens says what it means.
#[test]
fn a_cargo_section_is_written_where_toml_can_read_it_back() {
    let project = Project {
        id: None,
        details: Details {
            directory: Some(PathBuf::from("/src/kernel")),
            cargo: Some(Cargo {
                profile: Profile::Debug,
            }),
            ..Details::default()
        },
        binaries: paths(&["/tmp/vmlinux"]),
        bookmarks: vec![Bookmark {
            name: Some("start".to_owned()),
            document: SavedDocument::Source {
                path: "/src/kernel/main.rs".to_owned(),
            },
        }],
    };
    let text = round_trip(&project);
    assert!(text.contains("[cargo]"), "{text}");
    assert!(text.contains("profile = \"debug\""), "{text}");

    let session = Session {
        cargo: Some(SessionCargo {
            artifacts: paths(&["/src/kernel/target/debug/vmlinux"]),
        }),
        ..session_with(None)
    };
    let text = round_trip(&session);
    assert!(text.contains("[cargo]"), "{text}");
}

/// Absent rather than empty: a project whose profile is the default one and a session in
/// which nothing was built each write no section at all, so a file nothing has chosen in
/// says nothing.
#[test]
fn nothing_chosen_and_nothing_built_write_no_section() {
    let project = Project {
        id: None,
        details: Details::default(),
        binaries: Vec::new(),
        bookmarks: Vec::new(),
    };
    assert!(!round_trip(&project).contains("[cargo]"));
    assert!(!round_trip(&Session::default()).contains("[cargo]"));
}

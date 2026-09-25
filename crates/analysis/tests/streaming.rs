//! Objects arriving one at a time: what `open_files_streaming` says, in what order, and
//! what stops it. Order is the point — a `Vec` returned at the end would satisfy every
//! assertion about contents and none about when each answer is delivered.

mod common;

use analysis::{open_data_streaming, open_files, open_files_streaming, FileDigest, Progress};
use common::{archive, caller_and_target, Scratch};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Parsed(String),
    Finished(PathBuf),
}

fn events(paths: Vec<PathBuf>) -> Vec<Event> {
    let mut seen = Vec::new();
    open_files_streaming(paths, |progress| {
        seen.push(taken(progress));
        ControlFlow::Continue(())
    });
    seen
}

/// The same for files this test holds rather than wrote: `named` is what each is to be
/// called by, and nothing is on the disk under that name.
fn data_events(files: Vec<(&str, Vec<u8>)>) -> Vec<Event> {
    let mut seen = Vec::new();
    open_data_streaming(named(files), |progress| {
        seen.push(taken(progress));
        ControlFlow::Continue(())
    });
    seen
}

fn named(files: Vec<(&str, Vec<u8>)>) -> Vec<(PathBuf, Arc<[u8]>)> {
    files
        .into_iter()
        .map(|(name, bytes)| (PathBuf::from(name), Arc::from(bytes)))
        .collect()
}

fn taken(progress: Progress) -> Event {
    match progress {
        Progress::Parsed(object) => Event::Parsed(object.name.clone()),
        Progress::Finished(path) => Event::Finished(path),
    }
}

/// Members arrive one by one, each before the file they came out of is finished with. The
/// archive itself is not an object file and so adds nothing at the end.
#[test]
fn an_archive_streams_its_members_before_it_finishes() {
    let bytes = archive(&[
        ("first.o", &caller_and_target()),
        ("second.o", &caller_and_target()),
        ("third.o", &caller_and_target()),
    ]);

    assert_eq!(
        data_events(vec![("lib.a", bytes)]),
        [
            Event::Parsed("first.o".into()),
            Event::Parsed("second.o".into()),
            Event::Parsed("third.o".into()),
            Event::Finished("lib.a".into()),
        ]
    );
}

/// Each file's objects sit between its predecessor's end and its own, which is what lets
/// a caller draw "this file is still being read" against the right row.
#[test]
fn each_file_is_finished_before_the_next_one_starts() {
    let first = archive(&[("a.o", &caller_and_target())]);

    assert_eq!(
        data_events(vec![("lib.a", first), ("plain.o", caller_and_target())]),
        [
            Event::Parsed("a.o".into()),
            Event::Finished("lib.a".into()),
            Event::Parsed("plain.o".into()),
            Event::Finished("plain.o".into()),
        ]
    );
}

/// A caller drawing a pending file has no other way to learn that nothing is coming.
#[test]
fn a_file_that_yields_nothing_is_still_finished() {
    assert_eq!(
        data_events(vec![(
            "garbage",
            b"not an object file, nor an archive".to_vec()
        )]),
        [Event::Finished("garbage".into())]
    );
}

/// The other way a path yields nothing, and the one only the reading walk has: a file
/// that was not there to read. A path under the temporary directory that this test never
/// makes, so what it asserts is the miss and not what happens to be at some name.
#[test]
fn a_path_that_cannot_be_read_is_still_finished() {
    let missing =
        std::env::temp_dir().join(format!("analysis-was-never-here-{}", std::process::id()));

    assert_eq!(events(vec![missing.clone()]), [Event::Finished(missing)]);
}

/// A caller that breaks on the first object sees nothing after it — not the rest of the
/// archive, not the file's own `Finished`, and not the path behind it.
#[test]
fn breaking_stops_the_walk_where_it_stands() {
    let member = caller_and_target();
    let first = archive(&[("a.o", &member), ("b.o", &member), ("c.o", &member)]);

    let mut seen = Vec::new();
    open_data_streaming(
        named(vec![("lib.a", first), ("plain.o", member)]),
        |progress| {
            if let Progress::Parsed(object) = progress {
                seen.push(object.name.clone());
            }
            ControlFlow::Break(())
        },
    );

    assert_eq!(seen, ["a.o"]);
}

#[test]
fn collecting_the_stream_is_what_open_files_returns() {
    let scratch = Scratch::new("streaming");
    let member = caller_and_target();
    let first = scratch.write("lib.a", &archive(&[("a.o", &member), ("b.o", &member)]));
    let second = scratch.write("plain.o", &member);
    let paths = vec![first, second, scratch.0.join("was-never-here")];

    let collected: Vec<String> = open_files(paths.clone())
        .iter()
        .map(|object| object.name.clone())
        .collect();
    let streamed: Vec<String> = events(paths)
        .into_iter()
        .filter_map(|event| match event {
            Event::Parsed(name) => Some(name),
            Event::Finished(_) => None,
        })
        .collect();

    assert_eq!(collected, streamed);
}

/// A member's digest is the *whole archive's*, which it could only be if the file was
/// hashed as a file: a member hashing its own bytes would answer something else.
#[test]
fn streaming_does_not_hash_a_file_once_per_object() {
    let member = caller_and_target();
    let bytes = archive(&[("a.o", &member), ("b.o", &member), ("c.o", &member)]);

    let mut digests = Vec::new();
    open_data_streaming(named(vec![("lib.a", bytes.clone())]), |progress| {
        if let Progress::Parsed(object) = progress {
            digests.push(object.data.digest());
        }
        ControlFlow::Continue(())
    });

    assert_eq!(digests.len(), 3);
    assert!(digests
        .iter()
        .all(|digest| *digest == FileDigest::of(&bytes)));
    assert_ne!(FileDigest::of(&member), FileDigest::of(&bytes));
}

/// A path with no file name at all is what a plain object's *name* is derived from.
#[test]
fn a_path_with_no_file_name_still_finishes() {
    assert_eq!(
        events(vec![PathBuf::from("..")]),
        [Event::Finished(Path::new("..").to_path_buf())]
    );
}

/// A path a project file lists can name a fifo, and a plain open of one waits for a writer
/// that may never come. It is refused at once, as an unreadable path is, and the walk goes
/// on to the next. The walk runs on a thread of its own so a regression fails here rather
/// than hanging the suite.
#[cfg(unix)]
#[test]
fn a_fifo_is_finished_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let scratch = Scratch::new("fifo");
    let fifo = scratch.0.join("binary.o");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).expect("no NUL in the path");
    // SAFETY: `name` is a valid NUL-terminated string that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let after = scratch.write("after.o", &caller_and_target());

    let (sender, receiver) = std::sync::mpsc::channel();
    let paths = vec![fifo.clone(), after.clone()];
    std::thread::spawn(move || sender.send(events(paths)));
    let seen = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the walk waited on the fifo");
    assert_eq!(
        seen,
        [
            Event::Finished(fifo),
            Event::Parsed("after.o".into()),
            Event::Finished(after),
        ]
    );
}

/// Three objects after a `//` names table, the middle one's name field replaced by `name`.
fn archive_with_bad_middle_name(name: &[u8; 16]) -> Vec<u8> {
    let mut bytes = archive(&[
        // The helper adds the slash: this is the `//` names table.
        ("/", b"unused_long_name.o/\n"),
        ("first.o", &caller_and_target()),
        ("middle.o", &caller_and_target()),
        ("third.o", &caller_and_target()),
    ]);
    let at = bytes
        .windows(9)
        .position(|window| window == b"middle.o/")
        .expect("the middle member's header");
    bytes[at..at + 16].copy_from_slice(name);
    bytes
}

fn objects_of(bytes: Vec<u8>) -> Vec<Arc<analysis::Object>> {
    let mut objects = Vec::new();
    open_data_streaming(named(vec![("lib.a", bytes)]), |progress| {
        if let Progress::Parsed(object) = progress {
            objects.push(object);
        }
        ControlFlow::Continue(())
    });
    objects
}

/// `object` ends its walk of an archive at the first member whose name will not read. The
/// last object shown before it says so, and which member it was.
#[test]
fn an_archive_whose_members_stop_early_says_so_on_the_last_object_shown() {
    // A GNU name past the end of the `//` table, and a BSD name longer than its member.
    for name in [b"/999            ", b"#1/9999         "] {
        let objects = objects_of(archive_with_bad_middle_name(name));
        let names: Vec<&str> = objects.iter().map(|object| object.name.as_str()).collect();
        assert_eq!(names, ["first.o"]);
        assert_eq!(
            objects[0].messages,
            [analysis::LoadMessage::ArchiveCutShort { member: 2 }]
        );
    }
}

/// A clean archive says nothing about its members.
#[test]
fn an_archive_read_to_its_end_says_nothing() {
    let objects = objects_of(archive_with_bad_middle_name(b"middle.o/       "));
    assert_eq!(objects.len(), 3);
    assert!(objects.iter().all(|object| object.messages.is_empty()));
}

/// A thin archive: `ar --thin`'s magic, then a GNU header per member stating the size of a
/// file kept elsewhere, and none of its bytes.
fn thin_archive(members: &[(&str, u64)]) -> Vec<u8> {
    let mut bytes = b"!<thin>\n".to_vec();
    for (name, size) in members {
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            format!("{name}/"),
            0,
            0,
            0,
            644,
            size
        );
        bytes.extend_from_slice(header.as_bytes());
    }
    bytes
}

/// A thin archive's members are in other files. `object` gives each an offset of 0 and the
/// other file's size, so cutting the member from the archive would parse the archive's own
/// first bytes. None is parsed, and the archive is shown alone to say why.
#[test]
fn a_thin_archive_says_its_members_are_elsewhere() {
    // Sizes that fit in the archive, so a member cut from it would be its first bytes.
    let objects = objects_of(thin_archive(&[("first.o", 64), ("second.o", 100)]));
    let [archive] = objects.as_slice() else {
        panic!("one object, the archive: {}", objects.len());
    };
    assert_eq!(archive.name, "lib.a");
    assert_eq!(archive.format, None);
    assert!(archive.symbols.is_empty() && archive.sections.is_empty());
    assert_eq!(
        archive.messages,
        [analysis::LoadMessage::ThinArchive { members: 2 }]
    );
}

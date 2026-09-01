//! Objects arriving one at a time: what [`open_files_streaming`] says, in what order, and
//! what stops it.
//!
//! The point of the shape is that a reader can be looking at an archive's first member
//! while its last is still being parsed, so what these tests are really about is *when*
//! each answer is delivered — a `Vec` returned at the end would satisfy every assertion
//! about contents and none about order.

mod common;

use analysis::{open_files, open_files_streaming, FileDigest, Progress};
use common::{archive, caller_and_target};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

/// One event, flattened to something a test can compare and print.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    Parsed(String),
    Finished(PathBuf),
}

/// Every event `paths` produces, in order, with the walk allowed to run to the end.
fn events(paths: Vec<PathBuf>) -> Vec<Event> {
    let mut seen = Vec::new();
    open_files_streaming(paths, |progress| {
        seen.push(match progress {
            Progress::Parsed(object) => Event::Parsed(object.name.clone()),
            Progress::Finished(path) => Event::Finished(path),
        });
        ControlFlow::Continue(())
    });
    seen
}

/// A directory of this test's own, named after the line that asked for it so two tests
/// cannot collide. The archive fixtures have to be on disk: reading the file is the first
/// half of what is under test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(line: u32) -> Scratch {
        let directory =
            std::env::temp_dir().join(format!("analysis-streaming-{}-{line}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("creating the test directory");
        Scratch(directory)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("writing a fixture");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The whole of the sub-step in one assertion: an archive's members arrive one by one,
/// each of them *before* the file they came out of is finished with. The archive itself
/// is not an object file and so adds nothing at the end.
#[test]
fn an_archive_streams_its_members_before_it_finishes() {
    let scratch = Scratch::new(line!());
    let bytes = archive(&[
        ("first.o", &caller_and_target()),
        ("second.o", &caller_and_target()),
        ("third.o", &caller_and_target()),
    ]);
    let path = scratch.write("lib.a", &bytes);

    assert_eq!(
        events(vec![path.clone()]),
        [
            Event::Parsed("first.o".into()),
            Event::Parsed("second.o".into()),
            Event::Parsed("third.o".into()),
            Event::Finished(path),
        ]
    );
}

/// Files are walked in the order they were asked for, and each one's objects sit between
/// its own predecessor's end and its own — which is what lets a caller draw "this file is
/// still being read" against the right row without the crate having to name the file each
/// object belongs to a second time.
#[test]
fn each_file_is_finished_before_the_next_one_starts() {
    let scratch = Scratch::new(line!());
    let first = scratch.write("lib.a", &archive(&[("a.o", &caller_and_target())]));
    let second = scratch.write("plain.o", &caller_and_target());

    assert_eq!(
        events(vec![first.clone(), second.clone()]),
        [
            Event::Parsed("a.o".into()),
            Event::Finished(first),
            Event::Parsed("plain.o".into()),
            Event::Finished(second),
        ]
    );
}

/// A path that yields nothing is still finished with, both ways it can happen. This is
/// the variant's whole reason for existing: a caller drawing a pending file has no other
/// way to learn that nothing is coming.
#[test]
fn a_path_that_yields_nothing_is_still_finished() {
    let scratch = Scratch::new(line!());
    let garbage = scratch.write("garbage", b"not an object file, nor an archive");
    let missing = scratch.0.join("was-never-here");

    assert_eq!(
        events(vec![garbage.clone(), missing.clone()]),
        [Event::Finished(garbage), Event::Finished(missing)]
    );
}

/// Work nobody is waiting for stops. A caller that breaks on the first object it is
/// handed sees nothing after it — not the rest of the archive, not the file's own
/// `Finished`, and not the path behind it.
#[test]
fn breaking_stops_the_walk_where_it_stands() {
    let scratch = Scratch::new(line!());
    let member = caller_and_target();
    let first = scratch.write(
        "lib.a",
        &archive(&[("a.o", &member), ("b.o", &member), ("c.o", &member)]),
    );
    let second = scratch.write("plain.o", &member);

    let mut seen = Vec::new();
    open_files_streaming(vec![first, second], |progress| {
        if let Progress::Parsed(object) = progress {
            seen.push(object.name.clone());
        }
        ControlFlow::Break(())
    });

    assert_eq!(seen, ["a.o"]);
}

/// The collecting entry point is the streaming one with a `Vec` closed over, so the two
/// cannot disagree about what a file contributes.
#[test]
fn collecting_the_stream_is_what_open_files_returns() {
    let scratch = Scratch::new(line!());
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

/// Streaming must not quietly turn one hash per file into one per object. A member's
/// digest is the *whole archive's* — which it could only be if the file was hashed as a
/// file — and a member hashing its own bytes would answer something else here, since no
/// member is the archive.
#[test]
fn streaming_does_not_hash_a_file_once_per_object() {
    let scratch = Scratch::new(line!());
    let member = caller_and_target();
    let bytes = archive(&[("a.o", &member), ("b.o", &member), ("c.o", &member)]);
    let path = scratch.write("lib.a", &bytes);

    let mut digests = Vec::new();
    open_files_streaming(vec![path], |progress| {
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

/// A relative path, and a path with no file name at all, are what the *name* of a plain
/// object is derived from. Nothing here should be able to panic on one.
#[test]
fn a_path_with_no_file_name_still_finishes() {
    assert_eq!(
        events(vec![PathBuf::from("..")]),
        [Event::Finished(Path::new("..").to_path_buf())]
    );
}

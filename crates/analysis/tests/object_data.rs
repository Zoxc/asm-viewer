//! The bytes an `Object` keeps: an object file is parsed from a slice of a file's bytes
//! and holds on to them, so a later pass can read what parsing did not keep.

mod common;

use analysis::{open_files, parse_object, FileDigest, ObjectData};
use common::caller_and_target;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn a_parsed_object_keeps_the_bytes_it_was_parsed_from() {
    let data = caller_and_target();
    let object = parse_object(
        data[..].into(),
        "fixture.o".into(),
        PathBuf::from("/fixture.o"),
    )
    .expect("fixture parses");

    assert_eq!(object.data.bytes(), &data[..]);
}

#[test]
fn a_member_is_the_slice_of_the_file_it_lives_in() {
    // What an archive member looks like to `open_files`: an object file at some offset
    // inside a larger buffer, addressed by range rather than copied out of it.
    let member = caller_and_target();
    let prefix = b"!<arch>\n....a member header....";
    let mut file = prefix.to_vec();
    file.extend_from_slice(&member);
    file.extend_from_slice(b"...trailing members...");

    let file = ObjectData::whole_file(Arc::from(file));
    let data = ObjectData::member(&file, prefix.len() as u64, member.len() as u64)
        .expect("the member lies inside the file");

    // The member's own bytes, not the whole file's.
    assert_eq!(data.bytes(), &member[..]);

    let object = parse_object(data, "member.o".into(), PathBuf::from("/lib.a"))
        .expect("the member parses on its own");
    assert_eq!(object.data.bytes(), &member[..]);
    assert_eq!(object.symbols_sorted.len(), 2);
}

#[test]
fn a_member_range_outside_the_file_is_rejected() {
    let file = ObjectData::whole_file(Arc::from(vec![0u8; 16]));

    // Every way a member header can lie about where its data is. Each must come back as
    // `None` — `open_files` then skips the member, exactly as a failed `data()` did.
    assert!(ObjectData::member(&file, 0, 17).is_none());
    assert!(ObjectData::member(&file, 16, 1).is_none());
    assert!(ObjectData::member(&file, u64::MAX, 1).is_none());
    assert!(ObjectData::member(&file, 8, u64::MAX).is_none());

    // The boundary cases that are in range stay in range.
    assert_eq!(
        ObjectData::member(&file, 16, 0)
            .expect("empty tail")
            .bytes(),
        &[]
    );
    assert_eq!(
        ObjectData::member(&file, 0, 16)
            .expect("the whole file")
            .bytes(),
        file.bytes()
    );
}

/// The digest is the *file's*, so a member carries the archive's and not its own bytes'.
/// That is what makes it comparable with what a session saved: a session names files.
#[test]
fn a_member_carries_the_digest_of_the_file_it_lives_in() {
    let member = caller_and_target();
    let prefix = b"!<arch>\n....a member header....";
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(&member);

    let file = ObjectData::whole_file(Arc::from(bytes.clone()));
    let data = ObjectData::member(&file, prefix.len() as u64, member.len() as u64)
        .expect("the member lies inside the file");

    assert_eq!(data.digest(), FileDigest::of(&bytes));
    assert_eq!(data.digest(), file.digest());
    // And emphatically not the digest of the slice it addresses, which would answer for
    // a unit nothing names.
    assert_ne!(data.digest(), FileDigest::of(&member));

    let object = parse_object(data, "member.o".into(), PathBuf::from("/lib.a"))
        .expect("the member parses on its own");
    assert_eq!(object.data.digest(), FileDigest::of(&bytes));
}

/// One byte's difference is a different digest; the same bytes are the same digest
/// whatever allocation they arrive in. Both halves are what a restore leans on.
#[test]
fn the_digest_is_of_the_bytes_and_nothing_else() {
    let bytes = caller_and_target();
    let mut rebuilt = bytes.clone();
    *rebuilt.last_mut().expect("a non-empty fixture") ^= 1;

    assert_eq!(FileDigest::of(&bytes), FileDigest::of(&bytes.clone()));
    assert_ne!(FileDigest::of(&bytes), FileDigest::of(&rebuilt));
    // Written as sixteen hex digits, which is the form the session file holds.
    assert_eq!(FileDigest::of(&bytes).to_string().len(), 16);
}

/// A GNU `ar` archive holding `members`, built by hand: the writers this crate's fixtures
/// use can write object files but not the archive around them.
fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut file = b"!<arch>\n".to_vec();
    for (name, data) in members {
        file.extend_from_slice(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                format!("{name}/"),
                0,
                0,
                0,
                644,
                data.len()
            )
            .as_bytes(),
        );
        file.extend_from_slice(data);
        // Members are two-byte aligned.
        if data.len() % 2 == 1 {
            file.push(b'\n');
        }
    }
    file
}

/// The archive case the whole design turns on: 196 members must cost one hash, not 196,
/// and every object out of one file must answer the same thing — a session names the
/// file, and a member that disagreed with its archive would be a second answer to a
/// question with one.
#[test]
fn every_object_out_of_one_archive_shares_one_digest() {
    let bytes = archive(&[("first.o", &caller_and_target()), ("second.o", &caller_and_target())]);

    let directory = std::env::temp_dir().join(format!(
        "analysis-digest-test-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("lib.a");
    std::fs::write(&path, &bytes).expect("writing the archive");

    let objects = open_files(vec![path.clone()]);
    let _ = std::fs::remove_dir_all(&directory);

    // Both members parsed; the archive itself is not an object file, so it adds none.
    assert_eq!(objects.len(), 2);
    for object in &objects {
        assert_eq!(object.path, path);
        assert_eq!(object.data.digest(), FileDigest::of(&bytes));
    }
}

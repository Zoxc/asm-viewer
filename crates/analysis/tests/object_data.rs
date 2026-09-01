//! The bytes an `Object` keeps: it is parsed from a slice of a file's bytes and holds on
//! to them, so a later pass can read what parsing did not keep.

mod common;

use analysis::{open_files, parse_object, FileDigest, ObjectData};
use common::{archive, caller_and_target};
use std::path::PathBuf;
use std::sync::Arc;

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

    assert_eq!(data.bytes(), &member[..]);

    let object = parse_object(data, "member.o".into(), PathBuf::from("/lib.a"))
        .expect("the member parses on its own");
    assert_eq!(object.data.bytes(), &member[..]);
    assert_eq!(object.symbols_sorted.len(), 2);
}

#[test]
fn a_member_range_outside_the_file_is_rejected() {
    let file = ObjectData::whole_file(Arc::from(vec![0u8; 16]));

    // Every way a member header can lie about where its data is, overflow included.
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

/// The digest is the *file's*, which is what makes it comparable with what a session
/// saved: a session names files.
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
    assert_ne!(data.digest(), FileDigest::of(&member));

    let object = parse_object(data, "member.o".into(), PathBuf::from("/lib.a"))
        .expect("the member parses on its own");
    assert_eq!(object.data.digest(), FileDigest::of(&bytes));
}

#[test]
fn the_digest_is_of_the_bytes_and_nothing_else() {
    let bytes = caller_and_target();
    let mut rebuilt = bytes.clone();
    *rebuilt.last_mut().expect("a non-empty fixture") ^= 1;

    assert_eq!(FileDigest::of(&bytes), FileDigest::of(&bytes.clone()));
    assert_ne!(FileDigest::of(&bytes), FileDigest::of(&rebuilt));
    // Sixteen hex digits, which is the form the session file holds.
    assert_eq!(FileDigest::of(&bytes).to_string().len(), 16);
}

/// 196 members must cost one hash, not 196, and every object out of one file must answer
/// the same thing.
#[test]
fn every_object_out_of_one_archive_shares_one_digest() {
    let bytes = archive(&[
        ("first.o", &caller_and_target()),
        ("second.o", &caller_and_target()),
    ]);

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

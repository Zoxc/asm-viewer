//! The bytes an `Object` keeps: an object file is parsed from a slice of a file's bytes
//! and holds on to them, so a later pass can read what parsing did not keep.

mod common;

use analysis::{parse_object, ObjectData};
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

    let file: Arc<[u8]> = Arc::from(file);
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
    let file: Arc<[u8]> = Arc::from(vec![0u8; 16]);

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
        &file[..]
    );
}

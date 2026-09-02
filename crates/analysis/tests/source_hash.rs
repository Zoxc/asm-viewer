//! The checksums debug info records for a source file, and the digests a file read off disk
//! is compared with. The vectors are the published ones for `"abc"`, so a wrong algorithm or
//! byte order fails here and not in a pane.

mod common;

use analysis::{LineInfo, LineRow, SourceDigests, SourceHash};
use std::sync::Arc;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_three_digests_are_the_published_vectors_for_abc() {
    let digests = SourceDigests::of(b"abc");
    let md5 = SourceHash::Md5(hex("900150983cd24fb0d6963f7d28e17f72").try_into().unwrap());
    let sha1 = SourceHash::Sha1(
        hex("a9993e364706816aba3e25717850c26c9cd0d89d")
            .try_into()
            .unwrap(),
    );
    let sha256 = SourceHash::Sha256(
        hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            .try_into()
            .unwrap(),
    );
    for hash in [md5, sha1, sha256] {
        assert!(hash.matches(&digests), "{hash:?}");
        assert!(!hash.matches(&SourceDigests::of(b"abd")), "{hash:?}");
        assert!(!hash.matches(&SourceDigests::of(b"")), "{hash:?}");
    }
}

/// `LineInfo::new` is the backends' own path: rows come out sorted, clipped and coalesced,
/// and each file keeps the hash given with it.
#[test]
fn line_info_built_by_hand_holds_the_invariants() {
    let md5 = SourceHash::Md5([7; 16]);
    let files: Vec<(Arc<str>, Option<SourceHash>)> =
        vec![(Arc::from("a.c"), Some(md5)), (Arc::from("b.c"), None)];
    let row = |start, end, file, line| LineRow {
        range: start..end,
        file: Some(file),
        line: Some(line),
        column: None,
    };
    let info = LineInfo::new(
        vec![
            row(10, 20, 1, 5),
            row(0, 12, 0, 3), // overlaps the first: the earlier start keeps the addresses
            row(20, 20, 0, 9), // empty, dropped
            row(20, 30, 1, 5), // continues the first with the same position: coalesced
        ],
        files,
    )
    .expect("rows remain");

    let rows: Vec<(u64, u64, Option<usize>, Option<u32>)> = info
        .rows()
        .iter()
        .map(|row| (row.range.start, row.range.end, row.file, row.line))
        .collect();
    assert_eq!(
        rows,
        [(0, 12, Some(0), Some(3)), (12, 30, Some(1), Some(5))]
    );
    assert_eq!(info.hash_of(0), Some(md5));
    assert_eq!(info.hash_of(1), None);
    assert_eq!(info.hash_of(2), None);
    assert_eq!(info.file_of(&info.rows()[1]), Some("b.c"));

    assert!(LineInfo::new(vec![row(5, 5, 0, 1)], Vec::new()).is_none());
}

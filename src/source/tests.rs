use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

/// A path of this test run's own, named per process and per call so tests can run in
/// parallel, here and in another checkout at once.
fn temp_path(name: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "viewer-source-{}-{unique}-{name}",
        std::process::id()
    ))
}

fn write(name: &str, bytes: &[u8]) -> PathBuf {
    let path = temp_path(name);
    fs::write(&path, bytes).expect("the temp directory is writable");
    path
}

#[test]
fn reads_a_file_verbatim() {
    let path = write("lines.rs", b"fn main() {\r\n    let x = 1;\n}\n");
    let file = SourceFile::read(&path, MAX_SIZE).expect("a readable file");

    // Line endings included: what splits the text into lines is the highlighter, and it is
    // entitled to see the file as it is.
    assert!(file.text() == "fn main() {\r\n    let x = 1;\n}\n");
    assert!(file.path() == path);
    let _ = fs::remove_file(&path);
}

#[test]
fn invalid_utf8_is_read_lossily() {
    let path = write("latin1.c", b"/* caf\xe9 */\nint main(void) { return 0; }\n");
    let file = SourceFile::read(&path, MAX_SIZE).expect("a readable file");

    assert!(file.text() == "/* caf\u{fffd} */\nint main(void) { return 0; }\n");
    let _ = fs::remove_file(&path);
}

#[test]
fn a_file_over_the_cap_is_refused() {
    let path = write("big.rs", b"fn main() {}\n");
    assert!(SourceFile::read(&path, 4).is_none());
    // And the same file is fine once it fits, so it is the cap that refused it.
    assert!(SourceFile::read(&path, MAX_SIZE).is_some());
    let _ = fs::remove_file(&path);
}

#[test]
fn a_directory_is_not_a_source_file() {
    assert!(SourceFile::read(&std::env::temp_dir(), MAX_SIZE).is_none());
}

#[test]
fn a_file_is_read_once() {
    let path = write("cached.rs", b"fn main() {}\n");
    let first = load(&path).expect("a readable file");

    // Deleting it must not change the answer: the second call never reaches the filesystem.
    let _ = fs::remove_file(&path);
    let second = load(&path).expect("the remembered file");
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn a_missing_file_is_remembered_as_missing() {
    let path = temp_path("never-written.rs");
    assert!(load(&path).is_none());
    assert!(cache().contains_key(&path));

    // Creating it afterwards changes nothing: the pane asks on every render and must not
    // `stat` a missing file every time.
    let _ = fs::write(&path, b"fn main() {}\n");
    assert!(load(&path).is_none());
    let _ = fs::remove_file(&path);
}

/// The digests are of the bytes as read, so a file answers the checksum the compiler took
/// of it — the published vectors for `abc`, here — and not one taken of other bytes.
#[test]
fn a_file_matches_the_checksum_of_its_own_bytes() {
    fn hex<const N: usize>(text: &str) -> [u8; N] {
        let bytes: Vec<u8> = (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect();
        bytes.try_into().unwrap()
    }
    let md5 = SourceHash::Md5(hex("900150983cd24fb0d6963f7d28e17f72"));
    let sha1 = SourceHash::Sha1(hex("a9993e364706816aba3e25717850c26c9cd0d89d"));
    let sha256 = SourceHash::Sha256(hex(
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ));

    let path = write("abc.c", b"abc");
    let file = SourceFile::read(&path, MAX_SIZE).expect("a readable file");
    for hash in [md5, sha1, sha256] {
        assert!(file.matches(hash), "{hash:?}");
    }
    let _ = fs::remove_file(&path);

    let path = write("abd.c", b"abd");
    let edited = SourceFile::read(&path, MAX_SIZE).expect("a readable file");
    for hash in [md5, sha1, sha256] {
        assert!(!edited.matches(hash), "{hash:?}");
    }
    let _ = fs::remove_file(&path);
}

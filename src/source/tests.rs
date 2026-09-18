use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::*;
use crate::temporary::Temporary;

/// A source file [`load`] answers for with nothing on the disk: what a test uses when the
/// file is a fixture and not the thing under test.
///
/// [`Temporary`](crate::temporary::Temporary) is the other half of this, and what a test
/// is about is what decides between them. A real file when the reading is the point — a
/// file read, a miss, a directory forgotten and read again, a path that only reduces
/// through `canonicalize` — and one of these when the pane merely has to have something
/// to draw.
///
/// Nothing is made, so the directory is a name and not a place. The entries come out on
/// `Drop`, which unwinding runs: [`SEEDED`] is a `static` that outlives every test in the
/// process. What the drop takes is the seed and not the parse made of it, so a test that
/// reads a seeded file through `source_text` forgets it with `forget_source_under` as it
/// would a real one.
pub struct Seeded {
    directory: PathBuf,
}

impl Seeded {
    /// A directory of this call's own, named per process and per call so that tests seeding
    /// files can run in parallel, here and in another checkout at once. It is under the
    /// system temporary directory for one reason, that being an absolute path on every
    /// platform; nothing is written there.
    pub fn directory(name: &str) -> Seeded {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        Seeded {
            directory: std::env::temp_dir().join(format!(
                "assembly-viewer-seeded-{}-{unique}-{name}",
                std::process::id()
            )),
        }
    }

    /// Put `text` at `name` under it, and hand back the path a pane asks for it by.
    pub fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.directory.join(name);
        let bytes = text.as_bytes();
        let file = SourceFile {
            path: path.clone(),
            digests: SourceDigests::of(bytes),
            text: text.to_owned(),
        };
        seeds().insert(path.clone(), Arc::new(file));
        path
    }

    /// The same, as the string a `Document::Source` names a file by.
    pub fn named(&self, name: &str, text: &str) -> Arc<str> {
        Arc::from(
            self.file(name, text)
                .to_str()
                .expect("the temporary directory is utf-8"),
        )
    }
}

impl std::ops::Deref for Seeded {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.directory
    }
}

impl Drop for Seeded {
    fn drop(&mut self) {
        seeds().retain(|path, _| !path.starts_with(&self.directory));
    }
}

/// Every file seeded and not yet dropped, by path.
static SEEDED: LazyLock<Mutex<HashMap<PathBuf, Arc<SourceFile>>>> = LazyLock::new(Mutex::default);

fn seeds() -> MutexGuard<'static, HashMap<PathBuf, Arc<SourceFile>>> {
    SEEDED.lock().unwrap_or_else(|error| error.into_inner())
}

/// The file seeded at `path`, which [`load`] answers with before it reads anything.
pub(super) fn seeded(path: &Path) -> Option<Arc<SourceFile>> {
    seeds().get(path).cloned()
}

/// A path of this test run's own, named per process and per call so tests can run in
/// parallel, here and in another checkout at once. Gone when the test ends.
fn temp_path(name: &str) -> Temporary {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    Temporary::at(std::env::temp_dir().join(format!(
        "viewer-source-{}-{unique}-{name}",
        std::process::id()
    )))
}

fn write(name: &str, bytes: &[u8]) -> Temporary {
    let path = temp_path(name);
    fs::write(&path, bytes).expect("the temp directory is writable");
    path
}

/// A file `length` bytes long, made by its length alone: a sparse file costs no blocks and
/// `symlink_metadata` reports the length, so a test about the cap need not write 16 MiB.
fn sized(name: &str, length: u64) -> Temporary {
    let path = temp_path(name);
    fs::File::create(&path)
        .and_then(|file| file.set_len(length))
        .expect("the temp directory is writable");
    path
}

#[test]
fn reads_a_file_verbatim() {
    let path = write("lines.rs", b"fn main() {\r\n    let x = 1;\n}\n");
    let file = SourceFile::read(&path).expect("a readable file");

    // Line endings included: what splits the text into lines is the highlighter, and it is
    // entitled to see the file as it is.
    assert!(file.text() == "fn main() {\r\n    let x = 1;\n}\n");
    assert!(file.path() == &*path);
}

#[test]
fn invalid_utf8_is_read_lossily() {
    let path = write("latin1.c", b"/* caf\xe9 */\nint main(void) { return 0; }\n");
    let file = SourceFile::read(&path).expect("a readable file");

    assert!(file.text() == "/* caf\u{fffd} */\nint main(void) { return 0; }\n");
}

#[test]
fn a_file_over_the_cap_is_refused() {
    let path = sized("big.rs", MAX_SIZE + 1);
    assert!(SourceFile::read(&path).is_none());

    // And the same file is fine once it fits, so it is the cap that refused it.
    fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_len(MAX_SIZE))
        .expect("the temp file can be shrunk");
    assert!(SourceFile::read(&path).is_some());
}

#[test]
fn a_directory_is_not_a_source_file() {
    assert!(SourceFile::read(&std::env::temp_dir()).is_none());
}

/// The gate a press is put through, which is the read's own first step: a regular file
/// within the bound, and nothing else.
#[test]
fn only_a_regular_file_within_the_bound_is_shown() {
    let at_the_cap = sized("at-the-cap.txt", MAX_SIZE);
    let over_it = sized("over-the-cap.txt", MAX_SIZE + 1);
    let empty = write("empty.rs", b"");
    let missing = std::env::temp_dir().join("viewer-source-nothing-here");

    // The cap is inclusive, and one byte past it is not.
    assert!(showable(&at_the_cap));
    assert!(!showable(&over_it));
    // An empty file is within it too.
    assert!(showable(&empty));
    assert!(!showable(&std::env::temp_dir()));
    assert!(!showable(&missing));
}

/// The app follows no symlink. `showable` asks about the path itself, so a link to a file
/// the pane would happily read is refused as the link it is -- the rule the walk and the
/// Files view keep to as well, by listing no symlink.
///
/// The broken link and the pair pointing at each other are the other half: a link is file
/// input, and the answer is no off one `lstat` rather than a chase or a panic.
#[cfg(unix)]
#[test]
fn a_symlink_is_not_shown_whatever_it_points_at() {
    use std::os::unix::fs::symlink;

    let real = write("linked.rs", b"fn main() {}\n");
    let link = temp_path("link.rs");
    symlink(&*real, &*link).expect("the temp directory is writable");

    assert!(showable(&real));
    assert!(!showable(&link));
    // And the read behind the gate, so a caller that skipped it gets the same answer.
    assert!(read_text(&link).is_none());

    let broken = temp_path("broken.rs");
    symlink(temp_path("nothing.rs").to_path_buf(), &*broken)
        .expect("the temp directory is writable");
    assert!(!showable(&broken));

    let (first, second) = (temp_path("loop-a.rs"), temp_path("loop-b.rs"));
    symlink(second.to_path_buf(), &*first).expect("the temp directory is writable");
    symlink(first.to_path_buf(), &*second).expect("the temp directory is writable");
    assert!(!showable(&first));
}

/// `read_text` is the pane's rule without the digests, so what the pane refuses it refuses:
/// a language server answering with a directory must not open it.
#[test]
fn read_text_refuses_a_directory() {
    assert!(read_text(&std::env::temp_dir()).is_none());
}

/// And refuses a file past the cap, the point of the cap being that the bytes are never
/// read.
#[test]
fn read_text_refuses_a_file_over_the_cap() {
    let path = sized("huge.rs", MAX_SIZE + 1);
    assert!(read_text(&path).is_none());
}

/// Lossy like the pane's read, and fresh every time: nothing remembers it, so a file
/// answers what is on the disk now.
#[test]
fn read_text_is_lossy_and_not_remembered() {
    let path = write("answer.c", b"/* caf\xe9 */\n");
    assert_eq!(read_text(&path).as_deref(), Some("/* caf\u{fffd} */\n"));

    fs::write(&path, b"int main(void) { return 0; }\n").expect("the temp file is writable");
    assert_eq!(
        read_text(&path).as_deref(),
        Some("int main(void) { return 0; }\n")
    );
}

/// A seeded file goes when its guard does: `SEEDED` is a `static` and every test after
/// this one would otherwise be holding what this one made up. Nothing is written, so what
/// is left behind is a path with no file at it.
#[test]
fn a_seeded_file_is_forgotten_when_its_guard_goes() {
    let seeded = Seeded::directory("dropped");
    let path = seeded.file("one.rs", "fn one() {}\n");
    assert!(load(&path).expect("the seeded file").text() == "fn one() {}\n");

    drop(seeded);
    assert!(load(&path).is_none());
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
    let file = SourceFile::read(&path).expect("a readable file");
    for hash in [md5, sha1, sha256] {
        assert!(file.matches(hash), "{hash:?}");
    }
    let _ = fs::remove_file(&path);

    let path = write("abd.c", b"abd");
    let edited = SourceFile::read(&path).expect("a readable file");
    for hash in [md5, sha1, sha256] {
        assert!(!edited.matches(hash), "{hash:?}");
    }
}

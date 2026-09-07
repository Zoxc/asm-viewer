use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::temporary::Temporary;

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

#[test]
fn reads_a_file_verbatim() {
    let path = write("lines.rs", b"fn main() {\r\n    let x = 1;\n}\n");
    let file = SourceFile::read(&path, MAX_SIZE).expect("a readable file");

    // Line endings included: what splits the text into lines is the highlighter, and it is
    // entitled to see the file as it is.
    assert!(file.text() == "fn main() {\r\n    let x = 1;\n}\n");
    assert!(file.path() == &*path);
}

#[test]
fn invalid_utf8_is_read_lossily() {
    let path = write("latin1.c", b"/* caf\xe9 */\nint main(void) { return 0; }\n");
    let file = SourceFile::read(&path, MAX_SIZE).expect("a readable file");

    assert!(file.text() == "/* caf\u{fffd} */\nint main(void) { return 0; }\n");
}

#[test]
fn a_file_over_the_cap_is_refused() {
    let path = write("big.rs", b"fn main() {}\n");
    assert!(SourceFile::read(&path, 4).is_none());
    // And the same file is fine once it fits, so it is the cap that refused it.
    assert!(SourceFile::read(&path, MAX_SIZE).is_some());
}

#[test]
fn a_directory_is_not_a_source_file() {
    assert!(SourceFile::read(&std::env::temp_dir(), MAX_SIZE).is_none());
}

/// The gate a press is put through, which is the read's own first step: a regular file
/// within the bound, and nothing else.
#[test]
fn only_a_regular_file_within_the_bound_is_shown() {
    let long = write("long.txt", &b"x".repeat(100));
    let empty = write("empty.rs", b"");
    let missing = std::env::temp_dir().join("viewer-source-nothing-here");

    assert!(fits(&long, 100));
    assert!(!fits(&long, 99));
    // An empty file is within every bound, zero included.
    assert!(fits(&empty, 0));
    assert!(!fits(&std::env::temp_dir(), u64::MAX));
    assert!(!fits(&missing, u64::MAX));
}

/// The app follows no symlink. `fits` asks about the path itself, so a link to a file the
/// pane would happily read is refused as the link it is -- the rule the walk and the Files
/// view keep to as well, by listing no symlink.
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

    assert!(fits(&real, MAX_SIZE));
    assert!(!fits(&link, MAX_SIZE));
    // And the read behind the gate, so a caller that skipped it gets the same answer.
    assert!(read_text(&link).is_none());

    let broken = temp_path("broken.rs");
    symlink(temp_path("nothing.rs").to_path_buf(), &*broken)
        .expect("the temp directory is writable");
    assert!(!fits(&broken, MAX_SIZE));

    let (first, second) = (temp_path("loop-a.rs"), temp_path("loop-b.rs"));
    symlink(second.to_path_buf(), &*first).expect("the temp directory is writable");
    symlink(first.to_path_buf(), &*second).expect("the temp directory is writable");
    assert!(!fits(&first, MAX_SIZE));
}

/// `read_text` is the pane's rule without the cache, so what the pane refuses it refuses:
/// a language server answering with a directory must not open it.
#[test]
fn read_text_refuses_a_directory() {
    assert!(read_text(&std::env::temp_dir()).is_none());
}

/// And refuses a file past the cap, the point of the cap being that the bytes are never
/// read. The file is made by its length alone, so nothing here writes 16 MB to say so.
#[test]
fn read_text_refuses_a_file_over_the_cap() {
    let path = write("huge.rs", b"fn main() {}\n");
    fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_len(MAX_SIZE + 1))
        .expect("the temp file can be grown");

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
    assert!(cache().contains_key(&*path));

    // Creating it afterwards changes nothing: the pane asks on every render and must not
    // `stat` a missing file every time.
    let _ = fs::write(&path, b"fn main() {}\n");
    assert!(load(&path).is_none());
}

/// The other half of reading a file once: a build says the files under a directory have
/// changed, and what was read of them goes -- the misses with the rest, a file the build
/// generated having been missing when the pane first asked for it.
#[test]
fn forgetting_a_directory_re_reads_what_is_under_it() {
    let directory = temp_path("built");
    fs::create_dir_all(&directory).expect("the temp directory is writable");
    let path = directory.join("main.rs");
    fs::write(&path, b"fn main() {}\n").expect("a writable directory");
    let generated = directory.join("generated.rs");
    let outside = write("outside.rs", b"fn outside() {}\n");

    assert!(load(&path).expect("a readable file").text() == "fn main() {}\n");
    assert!(load(&generated).is_none());
    let kept = load(&outside).expect("a readable file");

    fs::write(&path, b"fn main() { one(); }\n").expect("a writable directory");
    fs::write(&generated, b"fn generated() {}\n").expect("a writable directory");
    forget_under(&directory);

    assert!(load(&path).expect("a readable file").text() == "fn main() { one(); }\n");
    assert!(load(&generated).is_some());
    // And a file outside the root is untouched: the same `Arc`, never read again.
    assert!(Arc::ptr_eq(
        &kept,
        &load(&outside).expect("the remembered file")
    ));
}

/// A seeded file goes when its guard does: `CACHE` is a `static` and every test after
/// this one would otherwise be holding what this one made up. Nothing is written, so what
/// is left behind is a path with no file at it.
#[test]
fn a_seeded_file_is_forgotten_when_its_guard_goes() {
    let seeded = Seeded::directory("dropped");
    let path = seeded.file("one.rs", "fn one() {}\n");
    assert!(load(&path).expect("the seeded file").text() == "fn one() {}\n");

    drop(seeded);
    assert!(!cache().contains_key(&path));
    // Read rather than `load`, which would leave the miss in the cache it just left.
    assert!(SourceFile::read(&path, MAX_SIZE).is_none());
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
}

#[test]
fn a_language_is_the_extension_and_nothing_else() {
    let of = |name: &str| Language::of(Path::new(name));
    assert!(of("main.rs") == Some(Language::Rust));
    assert!(of("sum.c") == Some(Language::C));
    assert!(of("sum.h") == Some(Language::C));
    assert!(of("sum.hpp") == Some(Language::Cpp));
    assert!(of("Cargo.toml") == Some(Language::Toml));
    assert!(of("compile_commands.json") == Some(Language::Json));

    // Named for what they compile to, with no grammar behind them.
    assert!(of("main.zig") == Some(Language::Zig));
    assert!(of("main.go") == Some(Language::Go));
    assert!(of("start.s") == Some(Language::Assembly));
    assert!(of("start.S") == Some(Language::Assembly));
    assert!(of("view.m") == Some(Language::ObjC));
    assert!(of("kernel.cu") == Some(Language::Cuda));
    assert!(of("solve.f90") == Some(Language::Fortran));

    // No extension at all, and one nothing here knows.
    assert!(of("Makefile").is_none());
    assert!(of("notes.md").is_none());
    assert!(of("build.py").is_none());
    // The name is not read: a file called `rs` is not Rust.
    assert!(of("rs").is_none());
    // The extension is read as written: `.C` is C++ to one compiler and C to another, so
    // it is nothing here rather than a guess.
    assert!(of("sum.C").is_none());
}

/// What decides whether a tab opens with an assembly side: a language named here that
/// becomes machine code, and nothing else. A file the app cannot place is not one.
#[test]
fn only_a_named_compiled_language_is_compiled() {
    assert!(compiled(Path::new("main.rs")));
    assert!(compiled(Path::new("sum.c")));
    assert!(compiled(Path::new("sum.hpp")));
    assert!(!compiled(Path::new("Cargo.toml")));
    assert!(!compiled(Path::new("compile_commands.json")));

    // A language named for what it compiles to needs no grammar to be answered yes.
    for named in [
        "shader.zig",
        "server.go",
        "start.S",
        "view.mm",
        "solve.f90",
        "runtime.d",
        "App.swift",
        "kernel.cu",
    ] {
        assert!(compiled(Path::new(named)), "{named}");
    }

    // And an extension nothing here names is still no.
    assert!(!compiled(Path::new("Makefile")));
    assert!(!compiled(Path::new("notes.md")));
}

/// The narrow half of the policy above: five of the twenty are coloured, and every other
/// one is answered plainly rather than left out of the match.
#[test]
fn only_five_languages_have_a_grammar() {
    for coloured in [
        Language::Rust,
        Language::C,
        Language::Cpp,
        Language::Toml,
        Language::Json,
    ] {
        assert!(coloured.grammar().is_some(), "{coloured:?}");
    }

    for plain in [
        Language::ObjC,
        Language::Assembly,
        Language::Go,
        Language::Zig,
        Language::D,
        Language::Swift,
        Language::Nim,
        Language::Odin,
        Language::Fortran,
        Language::Ada,
        Language::Pascal,
        Language::Haskell,
        Language::OCaml,
        Language::Crystal,
        Language::Cuda,
    ] {
        assert!(plain.grammar().is_none(), "{plain:?}");
    }
}

/// Which of the three answers a language gets: Rust its own scanner, C and C++ a parse
/// with the grammar, and everything else nothing -- **including** a language that has a
/// grammar, since colouring a file is not finding functions in it.
#[test]
fn functions_follow_the_language_and_not_the_grammar() {
    let names = |found: Vec<Function>| {
        found
            .into_iter()
            .map(|function| function.name)
            .collect::<Vec<_>>()
    };

    assert!(names(Language::Rust.functions("fn one() {}\n")) == ["one"]);
    assert!(names(Language::C.functions("int two(void) { return 2; }\n")) == ["two"]);
    assert!(names(Language::Cpp.functions("struct S { void three() {} };\n")) == ["three"]);

    // Both have a grammar, and a configuration file defines no functions.
    assert!(Language::Toml
        .functions("[package]\nname = \"one\"\n")
        .is_empty());
    assert!(Language::Json.functions("{ \"one\": 1 }\n").is_empty());
    // And a language with no grammar has nothing to parse with.
    assert!(Language::Zig.functions("fn one() void {}\n").is_empty());
}

/// The one language server this app can name, and the one place it is named.
#[test]
fn rust_is_the_only_language_with_a_server() {
    assert!(Language::Rust.server() == Some("rust-analyzer"));
    assert!(Language::C.server().is_none());
    assert!(Language::Toml.server().is_none());
}

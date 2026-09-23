use super::*;

#[test]
fn a_path_becomes_a_uri_and_comes_back() {
    for path in [
        "/home/reader/a project/src/main.rs",
        "/home/reader/hør/lib.rs",
        "/home/reader/plain.rs",
    ] {
        let uri = uri_of(Path::new(path));
        assert!(uri.starts_with("file:///"), "{uri}");
        assert_eq!(path_of(&uri), Some(PathBuf::from(path)), "{uri}");
    }
}

#[test]
fn a_space_in_a_path_is_escaped() {
    assert_eq!(uri_of(Path::new("/a b")), "file:///a%20b");
}

/// A Windows path comes back spelled the way it went out. `uri_of` writes every separator
/// as `/` and the URI carries a leading slash the path has not got, so a path that came
/// back as the URI spelled it named a file the app already had open under another
/// spelling -- and a `Document::Source` is never canonicalised.
///
/// The drive letter is what says the path is Windows', so the rule holds on either
/// platform and this test runs on both.
#[test]
fn a_windows_path_comes_back_with_its_own_separators() {
    let uri = uri_of(Path::new(r"C:\Users\reader\src\main.rs"));
    assert_eq!(uri, "file:///C:/Users/reader/src/main.rs");
    assert_eq!(
        path_of(&uri),
        Some(PathBuf::from(r"C:\Users\reader\src\main.rs"))
    );

    // A drive with nothing after it, and a share, which has no drive letter and keeps the
    // separators it came with.
    assert_eq!(path_of("file:///C:/"), Some(PathBuf::from(r"C:\")));
    assert_eq!(
        path_of("file:///a/b"),
        Some(PathBuf::from("/a/b")),
        "a unix path was respelled"
    );
}

#[test]
fn a_colon_after_a_unix_directory_name_is_no_drive() {
    let uri = uri_of(Path::new("/a:b/x.rs"));
    assert_eq!(uri, "file:///a:b/x.rs");
    assert_eq!(path_of(&uri), Some(PathBuf::from("/a:b/x.rs")));
}

/// A `\` is a separator only on a path with a drive letter. In a Unix name it is a
/// character, and goes out escaped.
#[test]
fn a_backslash_in_a_unix_name_is_a_character() {
    let uri = uri_of(Path::new(r"/x/a\b.rs"));
    assert_eq!(uri, "file:///x/a%5Cb.rs");
    assert_eq!(path_of(&uri), Some(PathBuf::from(r"/x/a\b.rs")));
}

#[test]
fn a_uri_of_something_that_is_not_a_local_file_names_no_path() {
    assert_eq!(path_of("https://example.invalid/x.rs"), None);
    assert_eq!(path_of("file://elsewhere/x.rs"), None);
}

#[test]
fn a_file_uri_is_the_path_with_everything_reserved_encoded() {
    assert_eq!(uri_of(Path::new("/tmp/a.rs")), "file:///tmp/a.rs");
    assert_eq!(uri_of(Path::new("/a b/c.rs")), "file:///a%20b/c.rs");
    assert_eq!(uri_of(Path::new("/-._~/x")), "file:///-._~/x");
    // Not text: one byte in, three out, whatever the byte was.
    assert_eq!(uri_of(Path::new("/é")), "file:///%C3%A9");
}

/// The URI goes inside a GVariant literal and beside a `dbus-send` type, and neither
/// caller quotes it. It may not carry a character that would end either one.
#[test]
fn a_file_uri_carries_nothing_that_would_need_quoting() {
    let hostile = "/a '\"\\ ;$(x)\n/b%c";
    let uri = uri_of(Path::new(hostile));
    let plain = |c: char| c.is_ascii_alphanumeric() || "-._~/%:".contains(c);
    assert!(uri.chars().all(plain), "{uri}");
}

/// A Unix path that is not UTF-8 is still a path, and the encoder never sees text.
#[cfg(unix)]
#[test]
fn a_file_uri_encodes_a_path_that_is_not_text() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff.o"));
    assert_eq!(uri_of(&path), "file:///tmp/%FF.o");
}

/// A server's answer naming that path comes back as the same bytes, not as nothing.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_text_comes_back() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff/a:b\\c.o"));
    assert_eq!(path_of(&uri_of(&path)), Some(path));
}

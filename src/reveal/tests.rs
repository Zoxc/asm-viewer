use super::*;

/// The arguments of the attempt that runs `program`, as text, for a path that is a file.
fn args_of(path: &str, program: &str) -> Vec<String> {
    args_for(path, false, program)
}

/// The same for a path that is a folder.
fn folder_args_of(path: &str, program: &str) -> Vec<String> {
    args_for(path, true, program)
}

fn args_for(path: &str, folder: bool, program: &str) -> Vec<String> {
    let plan = plan(Path::new(path), folder);
    let attempt = plan
        .iter()
        .find(|attempt| attempt.program == program)
        .unwrap_or_else(|| panic!("nothing runs {program}"));
    attempt
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_file_uri_is_the_path_with_everything_reserved_encoded() {
    assert_eq!(file_uri(Path::new("/tmp/a.rs")), "file:///tmp/a.rs");
    assert_eq!(file_uri(Path::new("/a b/c.rs")), "file:///a%20b/c.rs");
    assert_eq!(file_uri(Path::new("/-._~/x")), "file:///-._~/x");
    // Not text: one byte in, three out, whatever the byte was.
    assert_eq!(file_uri(Path::new("/é")), "file:///%C3%A9");
}

/// The URI goes inside a GVariant literal and beside a `dbus-send` type, and neither
/// caller quotes it. It may not carry a character that would end either one.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_file_uri_carries_nothing_that_would_need_quoting() {
    let hostile = "/a '\"\\ ;$(x)\n/b%c";
    let uri = file_uri(Path::new(hostile));
    let plain = |c: char| c.is_ascii_alphanumeric() || "-._~/%:".contains(c);
    assert!(uri.chars().all(plain), "{uri}");
}

/// A path that is not UTF-8 is still a path, and the encoder never sees text.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_file_uri_encodes_a_path_that_is_not_text() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff.o"));
    assert_eq!(file_uri(&path), "file:///tmp/%FF.o");
}

/// The D-Bus call names the file itself, since selecting it is the whole point; the
/// fallback can only name the folder.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn the_dbus_call_names_the_file_and_the_fallback_its_folder() {
    let gdbus = args_of("/home/r/a b/x.rs", "gdbus");
    assert!(
        gdbus.contains(&"['file:///home/r/a%20b/x.rs']".to_owned()),
        "{gdbus:?}"
    );
    // The empty startup id, which gdbus reads as GVariant and not as an argument.
    assert_eq!(gdbus.last().map(String::as_str), Some("''"));

    let send = args_of("/home/r/a b/x.rs", "dbus-send");
    assert!(
        send.contains(&"array:string:file:///home/r/a%20b/x.rs".to_owned()),
        "{send:?}"
    );
    assert_eq!(send.last().map(String::as_str), Some("string:"));

    assert_eq!(args_of("/home/r/a b/x.rs", "xdg-open"), ["/home/r/a b"]);
}

/// The one path with no folder above it opens itself rather than nothing, whichever it
/// is taken for.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn the_root_is_its_own_folder() {
    assert_eq!(args_of("/", "xdg-open"), ["/"]);
    assert_eq!(folder_args_of("/", "xdg-open"), ["/"]);
}

/// A folder is opened, a file is picked out in its folder, and the D-Bus call says which
/// by its method: `ShowItems` opens the window *around* what it names, so given a folder
/// it opens the parent, and `ShowFolders` opens the folder itself. The fallback ends at
/// the same window either way, picking nothing out.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_folder_is_opened_and_a_file_is_picked_out_in_its_folder() {
    let folder = folder_args_of("/home/r/dev/viewer", "gdbus");
    assert!(
        folder.contains(&"org.freedesktop.FileManager1.ShowFolders".to_owned()),
        "{folder:?}"
    );
    let file = args_of("/home/r/dev/viewer/x.rs", "gdbus");
    assert!(
        file.contains(&"org.freedesktop.FileManager1.ShowItems".to_owned()),
        "{file:?}"
    );
    // Both name the path itself; what differs is which call is asked of it.
    assert!(
        folder.contains(&"['file:///home/r/dev/viewer']".to_owned()),
        "{folder:?}"
    );

    // And `dbus-send` is asked the same question as `gdbus`.
    assert!(folder_args_of("/home/r/dev/viewer", "dbus-send")
        .contains(&"org.freedesktop.FileManager1.ShowFolders".to_owned()));

    // The fallback opens the folder each call above ends at.
    assert_eq!(
        folder_args_of("/home/r/dev/viewer", "xdg-open"),
        ["/home/r/dev/viewer"]
    );
    assert_eq!(args_of("/home/r/dev/viewer", "xdg-open"), ["/home/r/dev"]);
}

/// A trailing separator names the same folder, not one below it: it goes before the URI
/// is built, so nothing runs on a path spelled two ways.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_trailing_separator_makes_no_difference() {
    let gdbus = folder_args_of("/home/r/dev/", "gdbus");
    assert!(
        gdbus.contains(&"['file:///home/r/dev']".to_owned()),
        "{gdbus:?}"
    );
    assert_eq!(folder_args_of("/home/r/dev/", "xdg-open"), ["/home/r/dev"]);
}

/// A relative path would be a host name in a `file://` URI, so nothing is ever asked for
/// one.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_relative_path_is_made_absolute_first() {
    let gdbus = args_of("src/main.rs", "gdbus");
    let uri = gdbus
        .iter()
        .find(|arg| arg.starts_with("['file://"))
        .expect("no uri");
    assert!(uri.starts_with("['file:///"), "{uri}");
    assert!(uri.ends_with("/src/main.rs']"), "{uri}");
}

/// `explorer` parses `/select,` itself: the path is quoted inside the switch, and the
/// argument reaches it unchanged.
#[cfg(windows)]
#[test]
fn explorer_is_given_the_switch_and_the_path_as_one_quoted_argument() {
    assert_eq!(
        args_of(r"C:\a b\x.rs", "explorer"),
        [r#"/select,"C:\a b\x.rs""#]
    );
}

/// Its exit status is 1 whether or not it worked, so starting is what counts.
#[cfg(windows)]
#[test]
fn explorer_is_not_judged_by_its_exit_status() {
    let plan = plan(Path::new(r"C:\x.rs"), false);
    assert!(!plan[0].by_status);
}

/// A folder is opened and not selected: the switch would show its parent. Its trailing
/// separator is already gone.
#[cfg(windows)]
#[test]
fn a_folder_is_opened_where_a_file_is_selected() {
    assert_eq!(
        folder_args_of(r"C:\a b\dev\", "explorer"),
        [r#""C:\a b\dev""#]
    );
    assert_eq!(
        args_of(r"C:\a b\dev\x.rs", "explorer"),
        [r#"/select,"C:\a b\dev\x.rs""#]
    );
}

/// A root is the one path still ending in a backslash, and `CommandLineToArgvW` would
/// read that one as escaping the quote after it. Doubled, the path arrives whole.
#[cfg(windows)]
#[test]
fn a_root_keeps_its_backslash_through_the_quoting() {
    assert_eq!(folder_args_of(r"C:\", "explorer"), [r#""C:\\""#]);
}

#[cfg(target_os = "macos")]
#[test]
fn the_finder_is_asked_to_reveal_the_file() {
    assert_eq!(
        args_of("/Users/r/a b/x.rs", "open"),
        ["-R", "/Users/r/a b/x.rs"]
    );
}

/// A folder is opened rather than revealed: `-R` on one shows the parent.
#[cfg(target_os = "macos")]
#[test]
fn the_finder_is_asked_to_open_a_folder() {
    assert_eq!(folder_args_of("/Users/r/a b", "open"), ["/Users/r/a b"]);
}

/// `open -R` takes a folder as readily as a file and means the same by it.
#[cfg(target_os = "macos")]
#[test]
fn the_finder_reveals_a_folder_the_same_way() {
    assert_eq!(
        folder_args_of("/Users/r/dev/", "open"),
        ["-R", "/Users/r/dev"]
    );
}

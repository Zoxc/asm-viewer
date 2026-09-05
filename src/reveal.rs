//! Showing a file, or a folder, where the rest of the reader's tools are: the desktop's
//! file manager, opened on the folder it is in with the item itself picked out.
//!
//! No two desktops answer the same call and none of them offers a library worth linking,
//! so this is a list of programs per platform, run in order until one works. Running one
//! is a spawn and a wait, so it never happens on the UI thread: [`reveal`] starts a thread
//! and returns.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Show `path` in the desktop's file manager, on a thread of its own.
///
/// Returns at once. When nothing answers the reader is told in a box, the same kind a
/// panic uses: they pressed something, and an item that does nothing at all leaves them
/// wondering whether the app heard.
pub fn reveal(path: PathBuf) {
    // Named, so that a panic on it says which thread died (`crate::panics`).
    let started = std::thread::Builder::new()
        .name("the file manager call".to_owned())
        .spawn(move || {
            if !show(&path) {
                log::warn!("no file manager showed {}", path.display());
                tell(&path);
            }
        });
    if let Err(error) = started {
        log::warn!("the file manager call could not be started: {error}");
    }
}

/// The same on **this** thread, for a caller that is about to leave: the panic box, whose
/// shutdown would kill the thread [`reveal`] starts before it had spawned anything.
/// Answers whether a program ran.
///
/// Safe to wait on because each attempt is a program that hands the path to the desktop
/// and exits, rather than the file manager itself ([`run`]); the caller is a reader who
/// has just pressed a button and is waiting either way. Nothing is said in a box when
/// nothing answers -- the caller is already showing one.
pub fn reveal_now(path: &Path) -> bool {
    show(path)
}

/// Try the platform's ways of showing `path` in order, and say whether one worked.
fn show(path: &Path) -> bool {
    // The one look at the disk. A path that is gone answers no and is shown as a file
    // would be, which opens the folder it was in.
    plan(path, path.is_dir()).iter().any(run)
}

/// What would be run for `path`, in order. `folder` says the path is a folder itself,
/// which only the last resort has to know.
///
/// The path is made absolute first: a `file://` URI is read from the root, and a relative
/// path in one would name a host instead. `absolute` is textual, so a file reached
/// through a symlink is still shown where the reader found it. Then it is put back
/// together from its components, which drops a trailing separator: every call here names
/// an item and not a place, and a root is the only thing left ending in one.
fn plan(path: &Path, folder: bool) -> Vec<Attempt> {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let path: PathBuf = path.components().collect();
    attempts(&path, folder)
}

/// Say that nothing answered.
fn tell(path: &Path) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Assembly Viewer")
        .set_description(format!(
            "No file manager answered.\n\n{} could not be shown.",
            path.display()
        ))
        .show();
}

/// One program to run, with what its finishing means.
struct Attempt {
    program: &'static str,
    args: Vec<OsString>,
    /// Whether a zero exit is what "it worked" means. Windows' `explorer` exits 1 either
    /// way, so for that one starting is all there is to go on.
    by_status: bool,
}

impl Attempt {
    fn new(program: &'static str, args: Vec<OsString>) -> Attempt {
        Attempt {
            program,
            args,
            by_status: true,
        }
    }

    /// The same, for a program whose exit status says nothing.
    #[cfg(windows)]
    fn regardless(self) -> Attempt {
        Attempt {
            by_status: false,
            ..self
        }
    }
}

/// Run one attempt and say whether it worked.
fn run(attempt: &Attempt) -> bool {
    let mut command = Command::new(attempt.program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    arguments(&mut command, &attempt.args);

    let Ok(mut child) = command.spawn() else {
        return false;
    };
    // Waited for, always: nothing else here would reap it, and each of these programs
    // hands the file to the desktop and exits rather than being the file manager.
    let finished = child.wait();
    match attempt.by_status {
        true => finished.is_ok_and(|status| status.success()),
        false => true,
    }
}

#[cfg(not(windows))]
fn arguments(command: &mut Command, args: &[OsString]) {
    command.args(args);
}

/// Windows' `explorer` gets its argument exactly as written, quotes and all: it parses
/// `/select,` itself and wants the path quoted inside the switch, where `arg` would quote
/// the whole argument around it for a path with a space in it.
#[cfg(windows)]
fn arguments(command: &mut Command, args: &[OsString]) {
    use std::os::windows::process::CommandExt;

    for arg in args {
        command.raw_arg(arg);
    }
}

/// The freedesktop desktops. `org.freedesktop.FileManager1` is what every file manager
/// meaning to be scripted answers, and it has a call per kind: `ShowItems` opens the
/// window around what it is given and picks it out, which is what showing a file means,
/// and `ShowFolders` opens a folder itself. `gdbus` (glib) and `dbus-send` (dbus's own
/// tools) are the two ways to call it without linking a D-Bus library, and a machine with
/// a desktop on it has at least one. When neither program is there, or nothing is
/// listening, `xdg-open` is the last word: the right window, with nothing picked out.
#[cfg(all(unix, not(target_os = "macos")))]
fn attempts(path: &Path, folder: bool) -> Vec<Attempt> {
    const SERVICE: &str = "org.freedesktop.FileManager1";
    const OBJECT: &str = "/org/freedesktop/FileManager1";

    // The interface has one call per kind and they mean different things. `ShowItems`
    // opens the window *around* what it is given and picks it out, which is what showing
    // a file means; given a folder it opens the parent, which is not. `ShowFolders`
    // opens the folder itself. Checked against a live session bus: the handler runs as
    // `dolphin --new-window --select <path>` for the first and `--new-window <path>` for
    // the second.
    let method = match folder {
        true => "org.freedesktop.FileManager1.ShowFolders",
        false => "org.freedesktop.FileManager1.ShowItems",
    };

    let uri = file_uri(path);
    // What the last resort opens. It can pick nothing out, so it opens the window each
    // call above ends at: a file's own folder, and a folder itself. `/` has no parent and
    // is its own.
    let opened = match folder {
        true => path,
        false => path.parent().unwrap_or(path),
    };
    let opened = opened.as_os_str().to_owned();

    vec![
        Attempt::new(
            "gdbus",
            vec![
                "call".into(),
                "--session".into(),
                "--dest".into(),
                SERVICE.into(),
                "--object-path".into(),
                OBJECT.into(),
                "--method".into(),
                method.into(),
                // GVariant as text: an array of one URI, then the startup id, empty.
                // That one is `''` and not an empty argument, which gdbus would refuse.
                format!("['{uri}']").into(),
                "''".into(),
            ],
        ),
        Attempt::new(
            "dbus-send",
            vec![
                "--session".into(),
                // Without a reply to wait for, dbus-send exits zero whether or not
                // anything was listening.
                "--print-reply".into(),
                format!("--dest={SERVICE}").into(),
                OBJECT.into(),
                method.into(),
                format!("array:string:{uri}").into(),
                // The startup id: everything after the type is the value, so this is
                // the empty string.
                "string:".into(),
            ],
        ),
        Attempt::new("xdg-open", vec![opened]),
    ]
}

/// macOS has one answer and it is in the base system: `open` with `-R` reveals a file in
/// the Finder -- the enclosing window, with the item picked out -- and `open` without it
/// opens what it is given, which is what a folder wants. `-R` on a folder would show the
/// parent, the same mistake the freedesktop call makes. The path is an argument of its
/// own and no shell sees it, so nothing about it needs quoting.
#[cfg(target_os = "macos")]
fn attempts(path: &Path, folder: bool) -> Vec<Attempt> {
    let mut args: Vec<OsString> = Vec::new();
    if !folder {
        args.push("-R".into());
    }
    args.push(path.as_os_str().to_owned());

    vec![Attempt::new("open", args)]
}

/// Windows has one too, and it is the shell itself: `explorer /select,` opens the
/// enclosing folder with the item picked out, and `explorer` on its own opens what it is
/// given. A folder takes the second, the switch on one showing its parent -- the same
/// mistake the other two platforms' reveal calls make.
///
/// The path is quoted inside the switch, and the backslashes it ends in are doubled:
/// `CommandLineToArgvW` halves a run of them before a quote, so an odd run would escape
/// the quote that closes the path and the argument would be read wrong. `plan` drops a
/// trailing separator from everything but a root, so what this doubles is `C:\`'s one
/// backslash. Nothing else in a path needs escaping: `"` cannot appear in one, and
/// `CreateProcess` is not a shell.
#[cfg(windows)]
fn attempts(path: &Path, folder: bool) -> Vec<Attempt> {
    use std::os::windows::ffi::OsStrExt;

    // The run of backslashes the path ends in. Counted forwards, `EncodeWide` being a
    // one-way iterator: the fold's final value is the run that reaches the end.
    let trailing =
        path.as_os_str()
            .encode_wide()
            .fold(0usize, |run, unit| match unit == u16::from(b'\\') {
                true => run + 1,
                false => 0,
            });

    let mut select = OsString::from(match folder {
        true => "\"",
        false => "/select,\"",
    });
    select.push(path);
    select.push("\\".repeat(trailing));
    select.push("\"");

    vec![Attempt::new("explorer", vec![select]).regardless()]
}

/// Somewhere that is neither: nothing to call, so the reader is told.
#[cfg(not(any(unix, windows)))]
fn attempts(_path: &Path, _folder: bool) -> Vec<Attempt> {
    Vec::new()
}

/// `path` as a `file://` URI.
///
/// A path here is bytes and not text, so it is encoded a byte at a time: everything
/// outside RFC 3986's unreserved set, the separator apart, becomes `%XX`. What is left is
/// letters, digits and `-._~/%`, which is why neither caller has to quote what it is
/// given.
#[cfg(all(unix, not(target_os = "macos")))]
fn file_uri(path: &Path) -> String {
    use std::{fmt::Write, os::unix::ffi::OsStrExt};

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char);
            }
            _ => {
                let _ = write!(uri, "%{byte:02X}");
            }
        }
    }
    uri
}

#[cfg(test)]
mod tests;

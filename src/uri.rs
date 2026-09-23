//! A path as the `file:` URI a language server and the desktop's file manager name files
//! by, and back; and the one rule saying a path is Windows' by its drive letter.
//!
//! Percent-encoded by hand rather than by a crate: a path is the only thing this app ever
//! puts in a URI.

use std::{
    borrow::Cow,
    fmt::Write,
    path::{Path, PathBuf},
};

/// Whether `text` starts with a drive letter and a colon, ending there or at a separator.
///
/// The rule is textual and not a `cfg`, so it is the same on every platform and can be
/// tested from any of them. A Unix path can start this way too (`c:/x` is a relative one),
/// and is then read as Windows'.
pub fn drive(text: &[u8]) -> bool {
    text.first().is_some_and(u8::is_ascii_alphabetic)
        && text.get(1) == Some(&b':')
        && matches!(text.get(2), None | Some(b'\\' | b'/'))
}

/// `path` as a `file:` URI.
///
/// Encoded a byte at a time: everything outside RFC 3986's unreserved set, `/` and `:`
/// apart, becomes `%XX`. What is left is letters, digits and `-._~/:%`, so no caller has to
/// quote it. A path on a drive has its `\` written as `/`; any other path keeps a `\` as
/// `%5C`, it being a character of a Unix name. So a UNC path (`\\srv\share`), having no
/// drive, goes out as `file:///%5C%5Csrv%5Cshare`, which [`path_of`] cannot read back; nor
/// could it read the `file://///srv/share` this once wrote.
pub fn uri_of(path: &Path) -> String {
    let bytes = bytes_of(path);
    let windows = drive(&bytes);
    let mut uri = String::from("file://");
    // The authority-less form needs a third slash, and a path on a drive has none.
    if bytes.first() != Some(&b'/') {
        uri.push('/');
    }
    for &byte in bytes.iter() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                uri.push(byte as char)
            }
            b'\\' if windows => uri.push('/'),
            _ => {
                let _ = write!(uri, "%{byte:02X}");
            }
        }
    }
    uri
}

/// A Unix path is bytes and not text, and is encoded as the bytes it is.
#[cfg(unix)]
fn bytes_of(path: &Path) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(path.as_os_str().as_bytes())
}

/// Anywhere else a path is text, as UTF-8.
#[cfg(not(unix))]
fn bytes_of(path: &Path) -> Cow<'_, [u8]> {
    match path.to_string_lossy() {
        Cow::Borrowed(text) => Cow::Borrowed(text.as_bytes()),
        Cow::Owned(text) => Cow::Owned(text.into_bytes()),
    }
}

/// The path a `file:` URI names, or nothing if it names something else.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // The path begins at the third slash. Anything between the second and the third is an
    // authority, and that names a file on somebody else's machine.
    if !rest.starts_with('/') {
        return None;
    }

    let mut bytes = Vec::with_capacity(rest.len());
    let mut characters = rest.bytes();
    while let Some(byte) = characters.next() {
        match byte {
            b'%' => {
                let (high, low) = (characters.next()?, characters.next()?);
                let digits = [high, low];
                let text = std::str::from_utf8(&digits).ok()?;
                bytes.push(u8::from_str_radix(text, 16).ok()?);
            }
            byte => bytes.push(byte),
        }
    }

    path_from(spelled(bytes))
}

/// A decoded URI path as the platform it names spells one.
///
/// `/C:/x/y.rs` is how a Windows path comes back: both the leading slash and the
/// separators are the URI's, where the app spells that file `C:\x\y.rs`. A
/// [`Document::Source`](crate::document::Document) is compared as text and never
/// canonicalised, so the two spellings are two tabs of one file.
///
/// Whether the path is Windows' is [`drive`]'s rule, so a Unix `/a:b/x.rs` keeps its leading
/// slash and every byte after it. The rule reads bytes, and a `/` byte is never part of a
/// longer UTF-8 character, so text comes out as it would have as text.
fn spelled(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.first() == Some(&b'/') && drive(&bytes[1..]) {
        bytes.remove(0);
        for byte in &mut bytes {
            if *byte == b'/' {
                *byte = b'\\';
            }
        }
    }
    bytes
}

/// A Unix path is bytes, so any it decodes to is one.
#[cfg(unix)]
fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// Anywhere else a path is text, so bytes that are not UTF-8 name none.
#[cfg(not(unix))]
fn path_from(bytes: Vec<u8>) -> Option<PathBuf> {
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests;

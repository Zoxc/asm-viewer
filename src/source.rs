//! Source files read off disk, and the one rule for what counts as one.
//!
//! A path out of debug info is a weak thing to trust, so every failure is the same answer,
//! [`None`], and the pane draws a placeholder. Nothing here is cached: the parse over a
//! file is, misses included (`src/ui/highlight.rs`).

use analysis::{SourceDigests, SourceHash};
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::counter;

/// The largest file this will read into memory. A bound on what a bad path can cost, not a
/// guess at what source looks like: a debug-info string that happens to name a disk image
/// must not be loaded to find that out.
pub const MAX_SIZE: u64 = 16 * 1024 * 1024;

/// What a path is called without its directory, and the whole path where it has no name --
/// a root like `/`, which must not come out empty.
pub fn name_of(path: &Path) -> String {
    borrowed_name(path).into_owned()
}

/// [`name_of`] without the copy: borrowed where the name is valid UTF-8. One rule and not
/// two, for a caller that asks per comparison rather than per row (`walk::order`).
pub fn borrowed_name(path: &Path) -> Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

/// One source file: where it came from, and what it says. Splitting it into lines is the
/// UI's syntax highlighter's job, which works in whole files and hands its own line breaks
/// back.
pub struct SourceFile {
    path: PathBuf,
    text: String,
    /// The digests of the bytes as read — before the lossy decode, since the compiler
    /// hashed the bytes too — taken once with the file, so a pane asking on every render
    /// compares two arrays.
    digests: SourceDigests,
}

impl SourceFile {
    /// Where this was read from, i.e. the path the debug info named.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's contents, decoded lossily.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether this file is the one a checksum out of the debug info was taken of: the
    /// file the binary was built from, and not that file edited since.
    pub fn matches(&self, hash: SourceHash) -> bool {
        hash.matches(&self.digests)
    }

    /// Read a file, or [`None`] for anything that is not a readable text-sized regular
    /// file: [`contents`]' rule, and the digests of the bytes it read.
    fn read(path: &Path) -> Option<SourceFile> {
        let (bytes, text) = contents(path)?;
        Some(SourceFile {
            path: path.to_path_buf(),
            digests: SourceDigests::of(&bytes),
            text,
        })
    }
}

/// The text of `path` by [`load`]'s rule -- a regular file within [`MAX_SIZE`], decoded
/// lossily -- without the digests: for a reader that wants many files once
/// (`src/references.rs`).
///
/// It is here and not a `fs::read_to_string` at the caller so that there is one answer to
/// what a source file is. A second rule means the same file read two ways: a line a pane
/// draws that a list of references leaves blank, or a fifo a language server named opened
/// on a worker that then never returns.
pub fn read_text(path: &Path) -> Option<String> {
    contents(path).map(|(_, text)| text)
}

/// Whether [`load`] would read `path`: a regular file within [`MAX_SIZE`], and not a
/// symlink to one. Asked of the metadata and never of the bytes.
///
/// The gate the UI puts in front of opening a file as source, and [`contents`]' own first
/// step, so the two cannot drift apart: a row opens because the reader would read it.
///
/// `is_file` is asked before the size, and both before any read: a directory opens happily
/// on Linux and a fifo blocks the reader until someone writes to it, and neither may reach
/// a UI thread.
///
/// **`symlink_metadata` and not `metadata`**: this answers about the path itself, so a
/// symlink is not a file here whatever it points at. **The app follows none anywhere**,
/// and this is where that is written. The walk of a project's directory (`crate::walk`)
/// and the Files view's read of one level (`crate::files`) list no symlink to match, so a
/// file is offered by all three or by none. Not following also costs one `lstat` on a
/// broken link or a loop, where following would chase the loop to the kernel's limit for
/// the same answer.
pub fn showable(path: &Path) -> bool {
    #[cfg(test)]
    TOUCHES.set(TOUCHES.get() + 1);
    fs::symlink_metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() <= MAX_SIZE)
        .unwrap_or(false)
}

/// The bytes of `path` and those bytes decoded, or [`None`] for anything [`showable`]
/// refuses. **The one rule** for reading a source file by path; both readers above are
/// this plus what they keep.
///
/// The bytes come back beside the text because the digests are of the bytes as read: the
/// compiler hashed those, and a lossy decode is not reversible.
fn contents(path: &Path) -> Option<(Vec<u8>, String)> {
    if !showable(path) {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    // Lossy rather than strict: a file with one bad byte in a comment is still a source
    // file.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some((bytes, text))
}

counter!(
    /// Test-only: how many times this thread has asked the filesystem about a source
    /// file. Every read and every gate above goes through [`showable`], so counting
    /// there counts them all -- which settles what no other test here can, that a render
    /// or an effect made no filesystem call at all.
    pub fn touches() = TOUCHES
);

/// The file at `path`, read now: its text by [`contents`]' rule and the digests of the
/// bytes read. [`None`] means it cannot be shown -- missing, unreadable, not a file, or
/// past [`MAX_SIZE`].
///
/// Nothing here remembers it. What is remembered is the parse made of it, which holds
/// this `Arc` (`src/ui/highlight.rs`), and a build is what forgets that.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    #[cfg(test)]
    if let Some(seeded) = tests::seeded(path) {
        return Some(seeded);
    }
    SourceFile::read(path).map(Arc::new)
}

#[cfg(test)]
mod tests;

/// The test-only way to give [`load`] a file with nothing on the disk, kept with the tests
/// it belongs to.
#[cfg(test)]
pub use tests::Seeded;

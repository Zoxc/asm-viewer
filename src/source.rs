//! Source files, read off disk once and remembered — including the ones that are not
//! there.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it
//! can move into a crate of its own alongside the rest of the non-UI code.
//!
//! What the source pane has to work with is a *path a compiler wrote down*, which is a
//! weak thing to trust: the file may have been built on another machine, moved, deleted
//! or replaced since, and nothing in the debug info says how big it is or that it is text
//! at all. So every failure here is the same answer — [`None`], "no source" — and the
//! pane draws a placeholder rather than an error, exactly as `line.rs` answers "no line
//! info" for every reason at once.
//!
//! The cache remembers that answer too. A pane renders on every selection change, every
//! scroll and every hover, and asking the filesystem each time about a path that is not
//! on this machine would be a `stat` per frame; caching only the successes would make the
//! missing-file case the expensive one, which is backwards.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

/// The largest file this will read into memory.
///
/// Not a guess at what source looks like but a bound on what a bad path can cost: the
/// path comes out of a binary, and a debug-info string that happens to name a disk image
/// must not be loaded to find that out. Generated sources — bindgen output, a big
/// `include!`d table — run to a few megabytes, so the cap is well above anything a person
/// would open and well below anything that would hurt.
const MAX_SIZE: u64 = 16 * 1024 * 1024;

/// One source file: where it came from, and what it says.
///
/// Just the decoded text — the splitting into lines the pane draws is not done here,
/// because the UI has to run the text through a syntax highlighter that works in whole
/// files and hands *its* line breaks back. This is the part that reads a path and knows
/// nothing about how it will be shown.
pub struct SourceFile {
    path: PathBuf,
    text: String,
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

    /// Read and split a file, or [`None`] for anything that is not a readable text-sized
    /// regular file.
    ///
    /// The size is checked *before* the bytes are read, and `is_file` before that: a
    /// directory opens happily on Linux and a fifo blocks the reader until someone writes
    /// to it, and neither may reach a UI thread. `max_size` is a parameter only so the
    /// tests can set a small one.
    fn read(path: &Path, max_size: u64) -> Option<SourceFile> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > max_size {
            return None;
        }

        // Lossy rather than strict: a file with one bad byte in a comment is still a
        // source file, and refusing to show it would be a worse answer than a replacement
        // character in the line that holds it.
        let bytes = fs::read(path).ok()?;

        Some(SourceFile {
            path: path.to_path_buf(),
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

/// Every path asked about so far and what came back, `None` included.
///
/// A `static` rather than something the UI owns for the same reason the save policy is
/// one: the cache is about the machine's filesystem, not about any part of the component
/// tree, and two panes asking for the same file must get the same `Arc` rather than two
/// copies of a megabyte of text.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<SourceFile>>>>> =
    LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<SourceFile>>>> {
    // Nothing under this lock can panic short of an allocation failure, but recover
    // rather than propagate if something ever does: a poisoned lock must not turn an
    // unreadable file into a crashed app.
    CACHE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The contents of `path`, read on the first call and answered from memory afterwards.
///
/// [`None`] means the file cannot be shown — missing, unreadable, not a file, or past
/// [`MAX_SIZE`] — and is remembered as such, so a path that is not on this machine costs
/// one `stat` for the life of the process and nothing after that.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    if let Some(cached) = cache().get(path) {
        return cached.clone();
    }

    // Read outside the lock: this is the one slow step here, and holding the lock across
    // it would make every other pane wait on this file rather than on its own. The cost
    // of not holding it is that two callers racing for one path may both read it, and the
    // second's copy is dropped when it loses the insert.
    let file = SourceFile::read(path, MAX_SIZE).map(Arc::new);

    cache().entry(path.to_path_buf()).or_insert(file).clone()
}

#[cfg(test)]
mod tests;

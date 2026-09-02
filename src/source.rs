//! Source files, read off disk once and remembered — including the ones that are not
//! there.
//!
//! A path out of debug info is a weak thing to trust, so every failure is the same answer,
//! [`None`], and the pane draws a placeholder. The misses are cached too: a pane asks on
//! every render, and caching only the successes would make a path that is not on this
//! machine the expensive case.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

/// The largest file this will read into memory. A bound on what a bad path can cost, not a
/// guess at what source looks like: a debug-info string that happens to name a disk image
/// must not be loaded to find that out.
pub const MAX_SIZE: u64 = 16 * 1024 * 1024;

/// One source file: where it came from, and what it says. Splitting it into lines is the
/// UI's syntax highlighter's job, which works in whole files and hands its own line breaks
/// back.
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

    /// Read a file, or [`None`] for anything that is not a readable text-sized regular
    /// file.
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
        // source file.
        let bytes = fs::read(path).ok()?;

        Some(SourceFile {
            path: path.to_path_buf(),
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

/// Every path asked about so far and what came back, `None` included. A `static` so that
/// two panes asking for one file get the same `Arc` rather than two copies of a megabyte.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<SourceFile>>>>> =
    LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<SourceFile>>>> {
    // A poisoned lock must not turn an unreadable file into a crashed app.
    CACHE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The contents of `path`, read on the first call and answered from memory afterwards.
/// [`None`] means the file cannot be shown — missing, unreadable, not a file, or past
/// [`MAX_SIZE`] — and is remembered as such.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    if let Some(cached) = cache().get(path) {
        return cached.clone();
    }

    // Read outside the lock: holding it across the read would make every other pane wait
    // on this file. The cost is that two callers racing for one path may both read it, and
    // the second's copy is dropped when it loses the insert.
    let file = SourceFile::read(path, MAX_SIZE).map(Arc::new);

    cache().entry(path.to_path_buf()).or_insert(file).clone()
}

#[cfg(test)]
mod tests;

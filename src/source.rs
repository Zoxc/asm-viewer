//! Source files, read off disk once and remembered — including the ones that are not
//! there.
//!
//! A path out of debug info is a weak thing to trust, so every failure is the same answer,
//! [`None`], and the pane draws a placeholder. The misses are cached too: a pane asks on
//! every render, and caching only the successes would make a path that is not on this
//! machine the expensive case.

use analysis::{SourceDigests, SourceHash};
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

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
/// lossily -- read fresh and not remembered: for a reader that wants many files once
/// (`src/references.rs`), which the cache would otherwise hold for the life of the app.
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

/// Test-only: how many times this thread has asked the filesystem about a source file.
///
/// Every read and every gate above goes through [`showable`], so counting there counts
/// them all. A thread-local because `freya-testing` runs the whole app on the test's own
/// thread, which makes this the one thing that can settle what no other test here can:
/// that a render or an effect made no filesystem call at all. Nothing resets it -- a test
/// takes the count before and after what it is about.
#[cfg(test)]
pub fn touches() -> usize {
    TOUCHES.get()
}

#[cfg(test)]
thread_local! {
    static TOUCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every path asked about so far and what came back, `None` included. A `static` so that
/// two panes asking for one file get the same `Arc` rather than two copies of a megabyte.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<SourceFile>>>>> =
    LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<SourceFile>>>> {
    // A poisoned lock must not turn an unreadable file into a crashed app.
    CACHE.lock().unwrap_or_else(|error| error.into_inner())
}

/// What has been forgotten so far: how many times, and the last few directories it was.
///
/// Read before a file is and asked again before what was read is filed, so that a read
/// which began before a [`forget_under`] and finished after it is not put back as what is
/// on disk now. The reading is a worker thread's (`src/ui/highlight.rs`) and the
/// forgetting is a finished build's, on the UI thread, so the two do interleave: the file
/// is read, the build writes it and says so, and the copy from before the build is then
/// filed under the path nothing will ask about again.
///
/// **The directories and not the count alone**, since a forget is about one of them: a
/// file read while some other directory was being forgotten is a file nothing has said
/// anything about, and dropping it would cost a read for every build in a window the
/// reader is not even looking at. Only the last [`KEPT`] are held -- a bound on what this
/// costs, the answer for a read that has been outlived by that many forgets being that it
/// may well have been forgotten.
///
/// One record for both caches, since neither can be forgotten without the other.
static FORGOTTEN: LazyLock<Mutex<Forgets>> = LazyLock::new(Mutex::default);

/// How many directories back [`FORGOTTEN`] remembers.
const KEPT: usize = 16;

#[derive(Default)]
struct Forgets {
    /// How many times anything has been forgotten.
    count: u64,
    /// The roots of the last [`KEPT`] of them, oldest first.
    roots: VecDeque<PathBuf>,
}

fn forgets() -> MutexGuard<'static, Forgets> {
    FORGOTTEN.lock().unwrap_or_else(|error| error.into_inner())
}

/// How many times anything has been forgotten so far.
pub fn forgotten() -> u64 {
    forgets().count
}

/// Whether anything forgotten since `at` covers `path`, which is what says a copy read
/// then must not be filed now.
pub fn forgotten_since(at: u64, path: &Path) -> bool {
    let forgets = forgets();
    let since = forgets.count.saturating_sub(at);
    if since == 0 {
        return false;
    }
    // More forgets than are remembered: the ones this cannot answer for are answered as
    // if they were about this file.
    if since > forgets.roots.len() as u64 {
        return true;
    }
    forgets
        .roots
        .iter()
        .rev()
        .take(since as usize)
        .any(|root| path.starts_with(root))
}

/// The contents of `path`, read on the first call and answered from memory afterwards.
/// [`None`] means the file cannot be shown — missing, unreadable, not a file, or past
/// [`MAX_SIZE`] — and is remembered as such.
///
/// Nothing here notices a file that changed on disk. [`forget_under`] is how it is told.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    if let Some(cached) = cache().get(path) {
        return cached.clone();
    }

    // Read outside the lock: holding it across the read would make every other pane wait
    // on this file. The cost is that two callers racing for one path may both read it, and
    // the second's copy is dropped when it loses the insert.
    let at = forgotten();
    let file = SourceFile::read(path).map(Arc::new);

    let mut cache = cache();
    // Forgotten while it was being read: what came back is the file as it was before
    // whatever said so, and is handed to the caller that asked for it rather than filed
    // for everyone after. Asked under this cache's lock, which [`forget_under`] takes
    // too, so a forget is either counted here or has yet to empty anything.
    if forgotten_since(at, path) {
        return file;
    }
    cache.entry(path.to_path_buf()).or_insert(file).clone()
}

/// Forget every file read from under `root`, misses included, so the next call reads them
/// again.
///
/// Checking on the way in would be a `stat` per lookup, and a pane asks on every render.
/// So a build is what calls this, a build being the app's one word that a directory's
/// files have changed. The parsed copies above these go with them
/// (`src/ui/highlight.rs`).
pub fn forget_under(root: &Path) {
    let mut forgets = forgets();
    forgets.count += 1;
    forgets.roots.push_back(root.to_path_buf());
    if forgets.roots.len() > KEPT {
        forgets.roots.pop_front();
    }
    // Written down and let go of before the cache is taken. A reader takes the two the
    // other way round -- the cache, then this, to ask what happened while it was reading
    // -- so holding both here is the one thing that would deadlock.
    drop(forgets);
    cache().retain(|path, _| !path.starts_with(root));
}

#[cfg(test)]
mod tests;

/// The test-only way into the cache, kept with the tests it belongs to.
#[cfg(test)]
pub use tests::Seeded;

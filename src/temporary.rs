//! A path under the system temporary directory that a test owns and that goes when the
//! test does, and the name it is given.
//!
//! Removing it at the foot of the body is not enough: the common failure is an `assert!`
//! part way down, and the lines after it never run. So the removal is a `Drop`, which
//! unwinding runs. It matters because `/tmp` is memory on many systems and the names carry
//! the process id: each run writes a fresh set rather than over the last one's, so a leak
//! is per run rather than once.
//!
//! The name is made here and not by the test: `assembly-viewer-{name}-{pid}-{n}`, with `n`
//! from one count for the whole process, so no two calls share a path however many tests
//! run at once. The pid is what lets the suite run in two checkouts at once, and what tells
//! one live run's directories from another's.

use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};

/// A path under the system temporary directory no other call in this process is given,
/// and nothing is made there. For a test that needs a name but writes nothing, as `Seeded`
/// does; the rest take a [`Temporary`].
pub fn fresh_path(name: &str) -> PathBuf {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("assembly-viewer-{name}-{}-{n}", std::process::id()))
}

/// A temporary path, removed on drop. Derefs to the `Path`, so it is used as the path it
/// stands for; `to_path_buf` is how a test hands one to something that outlives it.
pub struct Temporary {
    /// What the test uses. May be under `owned`.
    path: PathBuf,
    /// What the drop removes.
    owned: PathBuf,
}

impl Temporary {
    /// A path of this call's own that nothing has made yet. The test writes what it needs
    /// there, or asserts that nothing was written.
    pub fn fresh(name: &str) -> Temporary {
        Temporary::at(fresh_path(name))
    }

    /// The same, made as an empty directory.
    pub fn fresh_directory(name: &str) -> Temporary {
        Temporary::directory(fresh_path(name))
    }

    /// A directory `inner` under one of this call's own, made empty, with the whole of the
    /// outer one removed on drop. For a test that needs its root called something in
    /// particular and still leaves no parent behind.
    pub fn fresh_under(name: &str, inner: &str) -> Temporary {
        Temporary::under(fresh_path(name), inner)
    }

    fn at(path: PathBuf) -> Temporary {
        Temporary {
            owned: path.clone(),
            path,
        }
    }

    /// Made as an empty directory, whatever an earlier run left there.
    fn directory(path: PathBuf) -> Temporary {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the temp directory is writable");
        Temporary::at(path)
    }

    fn under(outer: PathBuf, name: &str) -> Temporary {
        let _ = fs::remove_dir_all(&outer);
        let path = outer.join(name);
        fs::create_dir_all(&path).expect("the temp directory is writable");
        Temporary { path, owned: outer }
    }
}

impl Deref for Temporary {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Temporary {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Temporary {
    /// Either shape, since a test's own path may be a file. Failures are ignored: this is
    /// tidying up after an assertion that has already said what went wrong.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.owned);
        let _ = fs::remove_dir_all(&self.owned);
    }
}

/// A fifo made at `path`, for a test that a read does not wait on one. Removed with the
/// [`Temporary`] it is made under.
#[cfg(unix)]
pub fn make_fifo(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no NUL in the path");
    // SAFETY: `name` is a NUL-terminated string that outlives the call.
    assert_eq!(
        unsafe { libc::mkfifo(name.as_ptr(), 0o600) },
        0,
        "making a fifo"
    );
}

/// What `work` answers, run on a thread of its own and failing the test if that takes more
/// than ten seconds: a read that waits on a fifo then fails the test rather than hanging
/// the suite.
pub fn promptly<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(work()));
    receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("it waited")
}

#[cfg(test)]
mod tests;

//! A path under the system temporary directory that a test owns and that goes when the
//! test does.
//!
//! Removing it at the foot of the body is not enough: the common failure is an `assert!`
//! part way down, and the lines after it never run. So the removal is a `Drop`, which
//! unwinding runs. It matters because `/tmp` is memory on many systems and the names carry
//! the process id: each run writes a fresh set rather than over the last one's, so a leak
//! is per run rather than once.
//!
//! The pid stays in the names. It is what lets the suite run in two checkouts at once, and
//! what tells one live run's directories from another's.

use std::{
    fs,
    ops::Deref,
    path::{Path, PathBuf},
};

/// A temporary path, removed on drop. Derefs to the `Path`, so it is used as the path it
/// stands for; `to_path_buf` is how a test hands one to something that outlives it.
pub struct Temporary {
    /// What the test uses. May be under `owned`.
    path: PathBuf,
    /// What the drop removes.
    owned: PathBuf,
}

impl Temporary {
    /// A path nothing has made yet. The test writes what it needs there, or asserts that
    /// nothing was written.
    pub fn at(path: PathBuf) -> Temporary {
        Temporary {
            owned: path.clone(),
            path,
        }
    }

    /// The same, made as an empty directory, whatever an earlier run left there.
    pub fn directory(path: PathBuf) -> Temporary {
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the temp directory is writable");
        Temporary::at(path)
    }

    /// A directory `name` under `outer`, made empty, with the whole of `outer` removed on
    /// drop. For a test that needs its root called something in particular and still
    /// leaves no parent behind.
    pub fn under(outer: PathBuf, name: &str) -> Temporary {
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

#[cfg(test)]
mod tests;

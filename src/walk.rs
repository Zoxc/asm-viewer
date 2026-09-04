//! The project's directory walked: the rules it is read under, and the files that came
//! back. Framework-free.
//!
//! One place and not two. The Search panel reads the directory to grep it and the file
//! finder reads it to list it, and they have to agree about what is in a project: a file
//! the search finds a hit in but the finder will not offer, or the other way round, is a
//! reader being told two things. So the walker is built here, once, and both take it from
//! here.
//!
//! The walk is ripgrep's own ([`ignore`]), which is what skips what git is told to ignore
//! without being told twice. Files leave here through a **callback** and not a channel,
//! [`crate::search::search`]'s own shape and for its reason: whoever draws the result is
//! who should decide what to do when they arrive faster than they can be drawn. The
//! callback answering [`ControlFlow::Break`] is how a walk nobody is waiting for stops
//! where it stands.

use ignore::{Walk, WalkBuilder};
use std::{cmp::Ordering, ops::ControlFlow, path::Path, path::PathBuf};

/// The walker both readers of a project's directory use.
///
/// The bounds are here rather than at either call site, since a file one reader skips and
/// the other does not is the disagreement this module exists to prevent.
pub fn walker(root: &Path) -> Walk {
    WalkBuilder::new(root)
        // `ignore`'s default is to read `.gitignore` only inside a git working tree, so
        // without this a project directory that is not one has its `target/` walked whole.
        .require_git(false)
        // The bound the source pane reads by: a file it would refuse to show is a file no
        // hit in it could open, and one the finder could not open either.
        .max_filesize(Some(crate::source::MAX_SIZE))
        .sort_by_file_path(order)
        .build()
}

/// One walked file, as the finder needs it.
///
/// The path is the entry's own, never canonicalised, so a file reached here and a file
/// reached through the Files view are one document ([`crate::project::Document`]).
/// `shown` and `name_at` are worked out here, on the walking thread: they are what every
/// keystroke is matched against, and a match that had to take a path apart first would do
/// it once per file per character typed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Found {
    /// What opening the file passes on.
    pub path: PathBuf,
    /// The path written from the project's directory, in `/` whatever the platform: what
    /// is matched and what the row draws.
    pub shown: Box<str>,
    /// Where the file's own name starts in `shown`, as a byte offset. The whole of it for
    /// a file directly in the project's directory.
    pub name_at: usize,
}

impl Found {
    /// The file's own name.
    pub fn name(&self) -> &str {
        &self.shown[self.name_at..]
    }

    /// The directories above it, with the separator that follows them; empty for a file
    /// in the project's directory itself.
    pub fn directory(&self) -> &str {
        &self.shown[..self.name_at]
    }
}

/// What a walk reports.
pub enum WalkEvent {
    /// One file, in the order [`order`] settles.
    File(Found),
    /// The walk reached the end of the directory. Not sent to a callback that has already
    /// asked it to stop.
    Finished,
}

/// Walk `root` and report every file under it that the rules above allow.
///
/// Directories are not reported and neither is anything that is not a plain file: what
/// this answers is which files a reader could open, and the tree they sit in is the Files
/// view's question, not this one.
pub fn walk_files(root: &Path, emit: &mut dyn FnMut(WalkEvent) -> ControlFlow<()>) {
    for entry in walker(root).flatten() {
        // An entry whose kind is unknown is one `ignore` could not stat, so it is skipped.
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(found) = found_at(root, entry.path()) else {
            continue;
        };
        if emit(WalkEvent::File(found)).is_break() {
            return;
        }
    }
    let _ = emit(WalkEvent::Finished);
}

/// `path` as the finder holds it, or [`None`] where it is not one of the project's files
/// at all: a path outside `root`, which is where a source the reader reached through a
/// binary's debug info lives -- the standard library's own, most of it.
///
/// The finder holds a file it did not walk the same way it holds one it did, so this is
/// what a file opened recently goes through before it is listed beside them.
pub fn found_under(root: &Path, path: &Path) -> Option<Found> {
    if !path.starts_with(root) {
        return None;
    }
    found_at(root, path)
}

/// `path` as the finder holds it, or [`None`] for a path with nothing to draw: the root
/// itself, which the walk reports when it is handed a file rather than a directory.
fn found_at(root: &Path, path: &Path) -> Option<Found> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    // Separators are made `/` whatever the platform, so a query is typed one way and the
    // rows read one way on all three.
    let shown: String = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/");
    if shown.is_empty() {
        return None;
    }
    let name_at = shown.rfind('/').map(|at| at + 1).unwrap_or(0);
    Some(Found {
        path: path.to_path_buf(),
        shown: shown.into_boxed_str(),
        name_at,
    })
}

/// One directory's entries, files before the directories under them and each by name
/// without regard to case and then with it. The order the reader sees a list grow in, so
/// it is settled here rather than left to the walker: a hit in a directory's own files
/// arrives before the walk descends, and the list only ever grows at its end.
///
/// The comparator is handed paths and not entries, so the kind costs a `symlink_metadata`
/// per comparison. A path that cannot be stat'ed sorts as a file, which is where a walker
/// that cannot list it does the least.
fn order(a: &Path, b: &Path) -> Ordering {
    let directory = |path: &Path| {
        path.symlink_metadata()
            .map(|data| data.is_dir())
            .unwrap_or(false)
    };
    let (a_name, b_name) = (name_of(a), name_of(b));
    directory(a)
        .cmp(&directory(b))
        .then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase()))
        .then_with(|| a_name.cmp(&b_name))
}

/// What a path is called without its directory, the whole path where it has no name.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;

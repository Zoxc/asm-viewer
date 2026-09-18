//! The projects the reader has had open, most recently first: `recents.toml`, the two
//! ways it changes, and the rows the recent list is drawn from.

use std::path::{Path, PathBuf};

use crate::order::Order;
use crate::store::{Store, RECENTS_FILE};

use super::files::Project;

/// The projects the reader has had open, most recently first: `recents.toml`.
///
/// An *order* and not an index of what exists -- the project files are that -- which is
/// why nothing here prunes a path whose file has gone: [`recent_projects`] does it at the
/// point of use, where the repair is free.
///
/// A path under the app's own storage is written **relative to it** and every other path
/// absolutely, so that moving the state directory — a different user, a restored backup —
/// does not lose every unsaved project. In memory they are all absolute: the relative
/// spelling belongs to the file and nowhere else, which is what [`load_recents`] and
/// [`write_recents`] are for.
pub(super) type Recents = Order<PathBuf>;

/// The stored order, with every path made absolute. A file that will not parse is moved
/// aside first ([`Store::read`]): the next [`remember`] writes this file, so ignoring it
/// would lose the order without the reader ever hearing about it.
pub(super) fn load_recents(store: &Store) -> Recents {
    store
        .read::<Recents>(RECENTS_FILE)
        .unwrap_or_default()
        .into_entries()
        .into_iter()
        .map(|path| match path.is_relative() {
            true => store.path(path),
            false => path,
        })
        .collect()
}

/// The one write of that file, which is where the paths under the store go back to
/// relative. The cut to what the file keeps is [`Store::save_order`]'s, along with the
/// log-and-swallow of a failure.
fn write_recents(store: &Store, recents: Recents) {
    let stored: Recents = recents
        .into_entries()
        .into_iter()
        .map(|path| match store.relative(&path) {
            Some(relative) => relative.to_path_buf(),
            None => path,
        })
        .collect();
    store.save_order(RECENTS_FILE, stored);
}

/// Put `path` at the front of `recents.toml`, writing the file only when that moved it.
pub(super) fn remember(store: &Store, path: &Path) {
    let mut recents = load_recents(store);
    if !recents.touch(path) {
        return;
    }
    write_recents(store, recents);
}

/// Take `path` out of `recents.toml`, writing the file only when it was there. What a
/// project deleted, or moved somewhere else, leaves behind.
pub(super) fn forget(store: &Store, path: &PathBuf) {
    let mut recents = load_recents(store);
    if !recents.forget(path) {
        return;
    }
    write_recents(store, recents);
}

/// One row of the recent-projects view: a project that can be switched to, described by
/// its own file read at the moment the list is asked for, so nothing about a project is
/// copied beside the order. A project whose file will not parse still gets a row, as the
/// [`Project::default`] it will behave as once opened — and the file stays where it is
/// until it is opened, a row being a reading of a project and not a claim on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    /// The project file: what a project is, what it is called by, and what opening this
    /// row opens.
    pub path: PathBuf,
    pub directory: Option<PathBuf>,
    pub binaries: usize,
}

/// The projects the reader has had open, most recently first, each described by its own
/// file.
///
/// A path whose file has gone is dropped here rather than repaired, since a [`Recents`]
/// never prunes itself on load and this is the point of use where the repair is free.
pub fn recent_projects(store: &Store) -> Vec<Recent> {
    load_recents(store)
        .into_entries()
        .into_iter()
        .filter_map(|path| {
            if !path.is_file() {
                return None;
            }
            let project = Project::load_from(&path).unwrap_or_default();
            Some(Recent {
                path,
                directory: project.details.directory,
                binaries: project.binaries.len(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;

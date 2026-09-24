//! Projects: what the user gave — a directory, the binaries in it — and what the
//! app noticed while they read them — the open documents, where each side of each was
//! left, which one was on screen and where the reader has been.
//!
//! Framework-free: no freya types appear here.
//!
//! Four parts, and over them the lifecycle — all that is left in this file:
//!
//! - [`files`] — the two file schemas, and the identity that ties them together.
//! - [`restore`] — live state into a session, and a session back into live state.
//! - [`recents`] — the order the projects were last open in.
//! - [`saves`] — what the two files last held, and when the next write happens.
//!
//! **A project is its project file's path.** The file is what the user said (directory,
//! binaries, bookmarks) and is written at once, or once the reader stops typing in a box;
//! beside it, named after it, is the session the app noticed (tabs with their trails and
//! rows, active document, visits, digests), written on a timer. An *unsaved* project is one whose file is under the app's own
//! `projects/`; nothing else distinguishes it from one the reader gave a place.
//! The *when* of saving is [`saves::Saves`]: [`record`] writes or marks pending, [`flush`]
//! writes what is pending.
//!
//! [`ProjectId`] is not where a project is but *which* project it is: a large random
//! number in the project file, carried by every file the app keeps beside it, so a session
//! left next to a project file that has since been replaced is not read with it.
//!
//! There is no published version of this app, so a schema change is just a schema change:
//! a file that no longer parses is the default, not a migration. It is moved aside first
//! ([`Store::read`]), the one thing owed to a reader whose file the next write would
//! replace.

mod files;
mod recents;
mod restore;
mod saves;

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::bookmarks::Bookmark;
use crate::store::Store;

// Each of the four is re-exported whole: what a module marks `pub` is its share of what
// `project` offers, and the rest of the app reaches all four under this one name.
pub use files::*;
pub use recents::*;
pub use restore::*;
pub use saves::*;

use files::session_beside;
use recents::{forget, load_recents, remember};
use saves::{saves, unsaved_number, unsaved_project, write_or_warn, writing_into};

/// Whether `path` is a project file at all, which is the whole of what is asked of one
/// before it is opened: the extension and nothing else, so a file can be recognised without
/// being read. What is *in* it is [`load_project`]'s answer.
pub fn is_project_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == PROJECT_EXTENSION)
}

/// What to call the project kept at `path`: the file's name, or `Unsaved project 3` for one
/// the app is keeping for want of anywhere else. The whole of the naming rule, and here
/// rather than in a view because more than one draws it.
pub fn label(store: &Store, path: &Path) -> String {
    if let Some(number) = unsaved_number(store, path) {
        return format!("Unsaved project {number}");
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Reopen the project the app was last in: the first entry of `recents.toml`. Hands back
/// the path it picked and both halves for the caller to restore, and points the save policy
/// at it — but seeds it with nothing else (see [`saves::Saves::binaries`]).
///
/// `None` when there is nothing to reopen, which is a first run and not a failure — and a
/// project whose file has **gone** is one of those: the recent list never prunes itself, so
/// a name in it with nothing behind it is an ordinary startup rather than news. A file that
/// is there and will not open is the [`Failure`], for the caller to say.
pub fn reopen(store: &Store) -> Option<Result<(PathBuf, Project, Session), Failure>> {
    let path = load_recents(store).first()?.clone();
    match open_at(store, &path) {
        Err(failure) if failure.reason == Reason::Missing => None,
        Err(failure) => Some(Err(failure)),
        Ok((project, session)) => Some(Ok((path, project, session))),
    }
}

/// Both halves of the project the file at `path` holds, or the [`Failure`] when it is not
/// there or will not parse.
///
/// **The project file is never moved aside**, however it fails: it may be the reader's own
/// file, sitting in their tree beside the code, and the app has no business taking one
/// away. A failure here therefore means the project does not open at all, and since nothing
/// opens, nothing writes over what could not be read. That is the whole of the rule — the
/// plain read is [`files::Project::load_from`], which the recent list has always used for the same
/// reason. It also makes telling the reader everything that is left to do, which is what
/// the [`Reason`] is carried out of here for.
///
/// The session beside it *is* the app's own, and goes through [`Store::read`] like everything
/// else the app stores. One written for another project is dropped rather than believed:
/// the file is found by the project file's name, which says nothing about whether that file
/// still holds the project it did.
fn load_project(store: &Store, path: &Path) -> Result<(Project, Session), Failure> {
    let project = Project::load_from(path).map_err(|reason| {
        log::warn!("the project {} will not open: {reason}", path.display());
        Failure {
            path: path.to_path_buf(),
            reason,
        }
    })?;

    let session: Session = store.read(session_beside(path)).unwrap_or_default();
    let session = match session.id == project.id && project.id.is_some() {
        true => session,
        false => {
            if session != Session::default() {
                log::debug!("the session beside {} is another project's", path.display());
            }
            Session::default()
        }
    };
    Ok((project, session))
}

/// Leave the project the app is in and enter the one the file at `path` holds, handing back
/// both halves for the caller to restore. `None` — and nothing changed at all — when it is
/// not there or will not parse.
///
/// The order matters. The project being left is flushed **first**, while [`saves::Saves`] still
/// points at it. The new one is then remembered, and [`saves::Saves::opened`] empties the
/// baselines because the caller is about to empty the app — a baseline still describing
/// the old binaries would read that emptying as a change and write it into the project
/// just entered. Emptying the app is the caller's half, the states being the UI's.
pub fn switch(store: &Store, path: &Path) -> Result<(Project, Session), Failure> {
    flush();
    let opened = open_at(store, path)?;
    log::debug!("switched to the project {}", path.display());
    Ok(opened)
}

/// Open the project the file at `path` holds without leaving one first: what a startup
/// given a project file on the command line does, where there is nothing to flush.
/// [`switch`] is this with the flush in front of it.
///
/// The path is used as it was given: nothing here canonicalises or reduces it, so the
/// caller's own path stays the project's name.
///
/// A file with no id -- written by hand, or claimed by [`start_new`] and never written --
/// is given one by [`saves::Saves::opened`], and the next flush writes it.
pub fn open_at(store: &Store, path: &Path) -> Result<(Project, Session), Failure> {
    let (project, session) = load_project(store, path)?;
    remember(store, path);
    saves().opened(store, path.to_path_buf(), &project, &session);
    Ok((project, session))
}

/// Start a project the reader has not given a place and enter it: [`switch`] with nothing
/// to load.
pub fn start_new(store: &Store) -> Option<PathBuf> {
    flush();
    let path = unsaved_project(store)?;
    remember(store, &path);
    // The file is empty, so the project it holds has no id: `opened` gives it one.
    saves().opened(
        store,
        path.clone(),
        &Project::default(),
        &Session::default(),
    );
    log::debug!("started the project {}", path.display());
    Some(path)
}

/// Whether putting a project somewhere leaves the old place behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Put {
    /// Save as: the project is **copied** to the new place under an id of its own, the app
    /// is then in the copy, and what was copied is left as it was. A new id because the two
    /// are now two projects, and one id across both would mean each matched the other's
    /// session -- so shuffling the files around would silently pick up the wrong tabs.
    Copy,
    /// Save: the project is **moved**, keeping its id, and nothing is left behind. What an
    /// unsaved project has instead of Save as, there being no second project afterwards.
    Move,
}

/// Put the open project in the file at `path`. Answers whether it was written.
///
/// Serialised afresh rather than copied byte for byte, because a path in a project file is
/// relative to the file's own directory ([`files::Project::against`]): the same bytes in another
/// directory would be a claim about *that* tree. The session beside it holds absolute paths
/// and is only carried across.
///
/// What is pending is flushed **first**, while [`saves::Saves`] still points at the old place,
/// and what travels is then what [`saves::Saves`] holds: `written` and `stored` are the two files
/// as they now stand, so neither is read back. That saves two reads and a parse under the
/// lock, on the UI thread, and drops a failure a Save has no business having -- a project
/// file deleted or mangled underneath a run holding it perfectly well used to make this
/// answer `false` and write nothing. The baselines are the truer answer besides: a project
/// just started ([`start_new`]) whose id the flush could not write has an empty file, and a
/// re-read would hand it to its new place with no id, and so with no session either.
pub fn put_in(store: &Store, path: &Path, put: Put) -> bool {
    flush();
    let mut saves = saves();
    let Some((from, project, session)) = saves.to_put(put) else {
        log::warn!("no project to save");
        return false;
    };
    let id = project.id;

    if !write_or_warn(path, |path| project.save_to(store, path)) {
        return false;
    }
    // A failed session write does not fail the put: the project is already where the
    // reader asked. The session is owed instead, and a move keeps the old copy of it.
    let session_written = write_or_warn(&session_beside(path), |path| session.save_to(store, path));

    // A move onto the file the project is already in has nothing to leave behind, and the
    // two files it would remove are the two just written.
    if put == Put::Move && !same_file(&from, path) {
        let mut leaving = vec![from.clone()];
        if session_written {
            leaving.push(session_beside(&from));
        }
        for leaving in leaving {
            if let Err(error) = fs::remove_file(&leaving) {
                // The copy is made and the app has moved on; a file left behind is untidy
                // and not lost work.
                log::warn!("could not remove {}: {error}", leaving.display());
            }
        }
        forget(store, &from);
    }
    remember(store, path);
    saves.moved_to(path.to_path_buf(), id);
    if !session_written {
        saves.owes_session(session);
    }
    log::debug!("the project is now {}", path.display());
    true
}

/// Whether two paths name one file: the same spelling, or two the system resolves to one.
fn same_file(one: &Path, other: &Path) -> bool {
    if one == other {
        return true;
    }
    match (fs::canonicalize(one), fs::canonicalize(other)) {
        (Ok(one), Ok(other)) => one == other,
        _ => false,
    }
}

/// Leave the project the app is in, with nothing open afterwards. What is pending is
/// written **first**, while [`saves::Saves`] still points at it.
pub fn close() {
    flush();
    let mut saves = saves();
    saves.closed();
}

/// The same, and take the project away with it. Answers whether it was removed.
///
/// **Only ever a project in app storage**: one the reader gave a place is their own file
/// and this app has no business deleting it, whatever asked. Nothing is flushed, the
/// project being about to go.
pub fn delete() -> bool {
    let mut saves = saves();
    let Some((store, path)) = writing_into(&saves) else {
        return false;
    };
    if !unsaved(&store, &path) {
        log::warn!("{} is not the app's to delete", path.display());
        return false;
    }

    for going in [path.clone(), session_beside(&path)] {
        if let Err(error) = fs::remove_file(&going) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not remove {}: {error}", going.display());
            }
        }
    }
    forget(&store, &path);
    saves.closed();
    log::debug!("deleted the project {}", path.display());
    true
}

/// Take note of the project the app is now in, writing it out immediately if it is a
/// change that must not be lost and marking it pending otherwise. Cheap enough to call on
/// every state change. `loading` says the binaries are still arriving, which is what
/// keeps a half-read list off the disk.
pub fn record(
    details: &Details,
    binaries: &[PathBuf],
    loading: bool,
    bookmarks: &[Bookmark],
    session: Session,
) {
    // The write happens under the lock, so two writes can never reach the file out of
    // the order they were decided in, and nothing can slip between a write and the
    // baseline it moves.
    let mut saves = saves();
    let Some(recorded) = saves.record(details, binaries, loading, bookmarks, session) else {
        return;
    };
    let Some((store, file)) = writing_into(&saves) else {
        log::warn!("no state directory to save the project in");
        if let Some(session) = recorded.session {
            saves.owes_session(session);
        }
        return;
    };
    let project = recorded.project;

    // A write that failed is owed, so the next flush tries it again. The session it
    // carries is owed with it rather than written, so it never names a tab into a binary
    // the project file does not list.
    if !write_or_warn(&file, |path| project.save_to(&store, path)) {
        saves.owes_project(OwedProject {
            project,
            binaries_changed: recorded.binaries_changed,
        });
        if let Some(session) = recorded.session {
            saves.owes_session(session);
        }
        return;
    }
    saves.wrote_project(&project, recorded.binaries_changed);
    if let Some(session) = recorded.session {
        match write_or_warn(&session_beside(&file), |path| session.save_to(&store, path)) {
            true => saves.wrote_session(session),
            false => saves.owes_session(session),
        }
    }
}

/// Write out anything recorded but not yet written: the project file that is owed, then
/// the pending session. A no-op when nothing has changed, which is what makes it safe to
/// call on a timer. The session waits while a change to the binaries is still owed, for
/// [`record`]'s reason.
pub fn flush() {
    let mut saves = saves();
    if !write_owed_project(&mut saves) {
        return;
    }
    let Some(session) = saves.take_owing() else {
        return;
    };
    let Some((store, file)) = writing_into(&saves) else {
        log::warn!("no state directory to save the session in");
        saves.owes_session(session);
        return;
    };
    match write_or_warn(&session_beside(&file), |path| session.save_to(&store, path)) {
        true => saves.wrote_session(session),
        false => saves.owes_session(session),
    }
}

/// Write out the project file a change to the details owes, and nothing else: what the UI
/// calls once the reader has stopped typing, the session keeping to its own timer.
pub fn flush_project() {
    write_owed_project(&mut saves());
}

/// Answers whether the session may follow: false only where a change to the binaries is
/// still owed.
fn write_owed_project(saves: &mut Saves) -> bool {
    let Some(owed) = saves.take_owed_project() else {
        return true;
    };
    let binaries_changed = owed.binaries_changed;
    let Some((store, file)) = writing_into(saves) else {
        log::warn!("no state directory to save the project in");
        saves.owes_project(owed);
        return !binaries_changed;
    };
    match write_or_warn(&file, |path| owed.project.save_to(&store, path)) {
        true => {
            saves.wrote_project(&owed.project, binaries_changed);
            true
        }
        false => {
            saves.owes_project(owed);
            !binaries_changed
        }
    }
}

#[cfg(test)]
mod tests;

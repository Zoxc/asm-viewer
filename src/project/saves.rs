//! When the two files are written: what each last held, what has changed since, and which
//! changes go to disk at once rather than waiting for a flush.
//!
//! Also the app's own `projects/`: whether a path is under it, and the claim of a free
//! name in it, which is what an unsaved project's file is.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard},
    time::Duration,
};

use crate::bookmarks::Bookmark;
use crate::store::Store;

use super::files::{Details, Project, ProjectId, Session, PROJECT_EXTENSION};
use super::trust::{self, Agreement};
use super::Put;

/// Whether the project at `path` is one the app is keeping for want of anywhere else: an
/// **unsaved** project. Being under `projects/` is the whole of it, since that is the one
/// place the app puts a project the reader has not given a place. What a view asks before
/// drawing a Save where a close would be.
pub fn unsaved(store: &Store, path: &Path) -> bool {
    path.starts_with(store.projects())
}

/// Claim a file for a project the reader has not given a place, and hand back its path.
/// The file is left empty; the first write fills it. [`Store::claim`]'s rules.
pub(super) fn unsaved_project(store: &Store) -> Option<PathBuf> {
    store.claim(
        store.projects(),
        |n| format!("{n}.{PROJECT_EXTENSION}"),
        |path| fs::File::create_new(path).map(drop),
    )
}

/// The number an unsaved project's file is named by. `None` for a project the reader gave
/// a place, which is called by that file instead.
pub(super) fn unsaved_number(store: &Store, path: &Path) -> Option<String> {
    match unsaved(store, path) {
        true => Some(path.file_stem()?.to_string_lossy().into_owned()),
        false => None,
    }
}

/// How often [`super::flush`] is worth calling. Far coarser than the rate a user clicks through
/// symbols at, while bounding what an unclean exit can lose; a clean window close flushes
/// anyway.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// A `static` rather than UI state because two of the three things that drive it — the
/// periodic flush and the window's close hook — sit outside the component tree. A
/// [`LazyLock`] because it holds a [`Store`], which is a `PathBuf` and so not something a
/// `const fn` can spell; what that buys is one `#[derive(Default)]` in place of a
/// constructor naming every field.
static SAVES: LazyLock<Mutex<Saves>> = LazyLock::new(Mutex::default);

#[derive(Default)]
pub(super) struct Saves {
    /// Where the app's own files go, taken from the store the run opened when a project
    /// was entered. Held so that [`super::record`] and [`super::flush`] — a timer and a close hook,
    /// neither of them in the component tree — have one without being handed one.
    store: Option<Store>,
    /// The project file everything is written into, or `None` while no project is open.
    /// Only [`Saves::opened`] and [`Saves::moved_to`] set it, and nothing is written
    /// without it, so a run where nothing was ever opened leaves no file behind.
    open: Option<PathBuf>,
    /// Which project this is. Stamped onto both halves of every write, since a session
    /// carrying another id is one the next load throws away.
    id: Option<ProjectId>,
    /// `project.toml` as last written: the baseline every change is measured against, the
    /// binaries included. Its id is the one the file holds, which is `None` until a write
    /// gives it [`Saves::id`].
    ///
    /// Seeded whole by [`Saves::opened`], where the session is pointedly empty, because
    /// every baseline is the state the app boots into, and this one is restored
    /// synchronously: the details into `Proj`, the bookmarks, and the binaries as a
    /// placeholder each, their objects landing later in the placeholders' places.
    written: Project,
    /// The session as last written. Empty to start with, deliberately not the one the
    /// project was opened on: its tabs are restored only once the load is over, so a
    /// baseline holding them would read the tabless boot state as a change.
    session: Session,
    /// What `session.toml` holds. The baseline above only becomes that once something has
    /// been written: until then it is the stub [`Saves::opened`] seeded, while the file
    /// holds the session the project was opened on. The two differ until the first
    /// session write, which a load in flight holds back.
    ///
    /// Seeded whole by `opened`, and moved with the baseline by [`Saves::wrote_session`].
    /// [`super::put_in`] asks what the files hold, so it reads this one.
    stored: Session,
    /// A newer session that has not been written yet.
    pending: Option<Session>,
    /// The agreement `agreed.toml` holds for the open project, or `None` where it holds
    /// none.
    agreed: Option<Agreement>,
    /// The program and directory the details named at the last record, agreed to or not.
    pair: Option<Agreement>,
    /// A `project.toml` owed to the next flush: for a change to the details alone, which
    /// waits because a box being typed in changes them on every keystroke, or for a write
    /// made at once that did not land. A write for the binaries or the bookmarks takes it
    /// along, since it carries the details too.
    owed_project: Option<OwedProject>,
}

/// A `project.toml` owed to the next flush, and what it takes to note that it landed.
pub(super) struct OwedProject {
    pub(super) project: Project,
    /// Whether it is a change to the binaries, as [`Recorded::binaries_changed`].
    pub(super) binaries_changed: bool,
}

impl Saves {
    /// The newest session this knows about, whether or not it reached the disk.
    fn latest(&self) -> &Session {
        self.pending.as_ref().unwrap_or(&self.session)
    }

    /// Note that `project` is the file the app is now in, and set every baseline to the
    /// state the app will be in the instant afterwards. The empty session is *assigned*
    /// rather than assumed because a project switched away from leaves its own pending
    /// session behind.
    ///
    /// A `project` with no id -- written by hand, or claimed by [`super::start_new`] and not
    /// written yet -- is given one here, and the file is owed it, like a detail typed in.
    /// Otherwise the file got it only with a change the reader made, and a project they
    /// only read lost its session on every launch: the session went out under an id the
    /// file never held.
    pub(super) fn opened(
        &mut self,
        store: &Store,
        path: PathBuf,
        project: &Project,
        session: &Session,
    ) {
        self.store = Some(store.clone());
        self.open = Some(path);
        self.id = project.id.or_else(ProjectId::new);
        self.written = project.clone();
        // The id and the agreement, and nothing else. Both are restored *synchronously*
        // -- the one from the file being opened, the other into `Proj` beside it -- so a
        // baseline without them would read the state the app boots into as a change.
        self.session = Session {
            id: self.id,
            trusted: session.trusted,
            ..Session::default()
        };
        // What the file holds, which is the whole session and not the stub above.
        self.stored = Session {
            id: self.id,
            ..session.clone()
        };
        self.pending = None;
        self.owed_project = self.owed_for(&project.details);
        self.pair = Agreement::of(&project.details);
        self.agreed = self.pair.clone().filter(|_| session.trusted);
    }

    /// Keep `agreed.toml` in step with the agreement the app holds: the program and the
    /// directory it was given for, taken back where either changed or it went. Written at
    /// once and not with the session, since the session no longer carries it.
    ///
    /// Only a `trusted` over the pair already held is granted. The UI clears `trusted` when
    /// the details change, but after the record that sees the change, and a pair arriving
    /// with it still set would be agreed to without the reader being asked.
    pub(super) fn agreement(&mut self, details: &Details, trusted: bool) {
        let pair = Agreement::of(details);
        let same_pair = pair == self.pair;
        self.pair = pair.clone();
        let now = pair.filter(|_| trusted && same_pair);
        if now == self.agreed {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        if let Some(was) = &self.agreed {
            trust::agree(store, was, false);
        }
        if let Some(now) = &now {
            trust::agree(store, now, true);
        }
        self.agreed = now;
    }

    /// What a [`super::put_in`] writes into the place it is putting the project: the file
    /// the project is in now, and the two files as they stand under the id the put gives
    /// them -- a fresh one for a copy, since the two are afterwards two projects, and the
    /// project's own for a move. [`None`] where there is no project open.
    ///
    /// The baselines and not the files read back: `written` and `stored` are what the two
    /// files hold this instant, the pending session having been flushed first
    /// ([`super::put_in`], which says why that is the truer answer).
    ///
    /// The id is stamped here for [`Saves::record`]'s reason: which project a file is for
    /// belongs to the policy, and this is the one other place a write is made up from the
    /// baselines rather than from what the app is holding.
    pub(super) fn to_put(&self, put: Put) -> Option<(PathBuf, Project, Session)> {
        let from = self.open.clone()?;
        let id = match put {
            Put::Copy => ProjectId::new(),
            Put::Move => self.id,
        };
        let project = Project {
            id,
            ..self.written.clone()
        };
        let session = Session {
            id,
            ..self.stored.clone()
        };
        Some((from, project, session))
    }

    /// Take note of the state the app is now in. Hands back the `project.toml` to write
    /// now and the `session.toml` beside it where one is owed, or `None` when nothing
    /// changed or the change can wait for a flush.
    ///
    /// A **binaries** change goes to disk at once and carries whatever session was
    /// pending with it, which is what keeps `session.toml` from naming a tab into a binary
    /// `project.toml` no longer lists. A change to the **bookmarks** is immediate too but
    /// writes `project.toml` alone, since it lets go of no binary. A change to the
    /// **details** alone is owed to the next flush rather than written: it lets go of no
    /// binary either, and arrives once per keystroke in a box. Everything else — a
    /// selection, a tab, a history entry — only marks the session pending. Nothing here
    /// has to say which is which: which file a field lives in is what decides it.
    ///
    /// `binaries` is the whole list from the moment each was asked for, a file being read
    /// held by its placeholder, so it is compared and written whether or not a load is in
    /// flight. The **session** is not the app's own while one is: it has no tabs until a
    /// restore has resolved them. So it is neither compared, carried nor marked pending
    /// then, or a tabless session would go to disk over the good one -- pending is what the
    /// next flush writes, and a close, a switch or the timer can land inside the load. A
    /// binaries change in that window goes alone; the record after the load sees the
    /// session. A session left pending *before* the load began describes a real state and
    /// stays. A restore begins its load before any record runs, so the boot state is never
    /// seen: not its session, and not its empty list of binaries.
    ///
    /// **No baseline moves here**, since a baseline is what the *file* holds and the file
    /// has not been written yet. The caller moves them with [`Saves::wrote_project`] and
    /// [`Saves::wrote_session`] once the write has landed, so a write that fails leaves
    /// the change for the next record to see again. The pending session is not a baseline
    /// and is set here as ever: it is what has not been written.
    pub(super) fn record(
        &mut self,
        details: &Details,
        binaries: &[PathBuf],
        loading: bool,
        bookmarks: &[Bookmark],
        session: Session,
    ) -> Option<Recorded> {
        // Stamped here rather than by the caller: which project this is belongs to the
        // policy and not to the UI, and stamping before the comparison is what keeps the
        // baseline and what arrives comparable. The only stamp: the writes take both
        // halves as this hands them back.
        let session = Session {
            id: self.id,
            ..session
        };
        let binaries_changed = self.written.binaries != binaries;
        let bookmarks_changed = self.written.bookmarks != bookmarks;
        let session_changed = !loading && *self.latest() != session;

        // A binaries change carries the session to disk with it, outside a load; anything
        // else leaves it pending. Decided here, once, so the two ways out below say nothing
        // about it.
        let carried = match (binaries_changed, loading) {
            (true, false) => Some(session),
            (true, true) => None,
            (false, _) => {
                if session_changed {
                    self.pending = Some(session);
                }
                None
            }
        };

        if !binaries_changed && !bookmarks_changed {
            // The details alone, or nothing: owed, or no longer owed where they have been
            // changed back to what the file holds.
            self.owed_project = self.owed_for(details);
            return None;
        }
        // This write carries the details, so nothing is owed for them any more.
        self.owed_project = None;

        Some(Recorded {
            binaries_changed,
            project: Project {
                id: self.id,
                details: details.clone(),
                binaries: binaries.to_vec(),
                bookmarks: bookmarks.to_vec(),
            },
            session: carried,
        })
    }

    /// The `project.toml` owed where `details` or the id differ from what the file holds,
    /// both being changes that wait for a flush.
    fn owed_for(&self, details: &Details) -> Option<OwedProject> {
        let changed = self.written.details != *details || self.written.id != self.id;
        changed.then(|| OwedProject {
            project: Project {
                id: self.id,
                details: details.clone(),
                ..self.written.clone()
            },
            binaries_changed: false,
        })
    }

    /// Take whatever was recorded but not written, or `None` when the two already agree.
    /// Taken rather than cloned: the caller either notes it written
    /// ([`Saves::wrote_session`]) or hands it back ([`Saves::owes_session`]), so a copy
    /// left behind would only be dropped.
    pub(super) fn take_owing(&mut self) -> Option<Session> {
        self.pending.take()
    }

    /// Take the `project.toml` that is owed, for [`Saves::take_owing`]'s reason.
    pub(super) fn take_owed_project(&mut self) -> Option<OwedProject> {
        self.owed_project.take()
    }

    /// Its write did not happen: owed, for the next flush. Whether it was taken out by a
    /// flush or handed back by a record whose write at once failed: either way the
    /// close hook's flush must not find nothing to do.
    pub(super) fn owes_project(&mut self, owed: OwedProject) {
        self.owed_project = Some(owed);
    }

    /// Note that `project` reached `project.toml`: it is now what the file holds.
    pub(super) fn wrote_project(&mut self, project: &Project) {
        self.written = project.clone();
    }

    /// Note that `session` reached `session.toml`: it is what the file holds, and nothing
    /// is owed.
    pub(super) fn wrote_session(&mut self, session: Session) {
        self.stored = session.clone();
        self.session = session;
        self.pending = None;
    }

    /// Note that the project is now kept at `path` under `id`. Only *where* it is has
    /// changed, so every baseline but the id stays: the app is holding what it was holding
    /// a moment ago, and the files just written say the same.
    pub(super) fn moved_to(&mut self, path: PathBuf, id: Option<ProjectId>) {
        self.open = Some(path);
        self.id = id;
        self.written.id = id;
        self.session.id = id;
        self.stored.id = id;
        if let Some(pending) = &mut self.pending {
            pending.id = id;
        }
        if let Some(owed) = &mut self.owed_project {
            owed.project.id = id;
        }
    }

    /// Note that there is no project open. Every baseline back to what the app boots into,
    /// because the caller is about to empty the app -- one still describing the project
    /// just left would read that emptying as a change and write it back into it.
    pub(super) fn closed(&mut self) {
        *self = Saves {
            store: self.store.take(),
            ..Saves::default()
        };
    }

    /// The other answer: the write did not happen, so the session is owed again and the
    /// next flush tries it rather than finding nothing to do.
    pub(super) fn owes_session(&mut self, session: Session) {
        self.pending = Some(session);
    }
}

/// What a [`Saves::record`] decided to write, and what it takes to note that it landed.
pub(super) struct Recorded {
    /// The `project.toml` to write now.
    pub(super) project: Project,
    /// The `session.toml` to write beside it, which only a binaries change outside a load
    /// carries.
    pub(super) session: Option<Session>,
    /// Whether that change was to the binaries, which a flush does not let the session
    /// out ahead of while the write is owed.
    pub(super) binaries_changed: bool,
}

pub(super) fn saves() -> MutexGuard<'static, Saves> {
    // Take the state back rather than propagate: a poisoned lock must not turn a failed
    // save into a crashed app.
    SAVES.lock().unwrap_or_else(|error| error.into_inner())
}

/// Test-only: put the static back to what the app boots into if it is in a project under
/// `directory`, which a test is about to remove. Otherwise the next flush from any test
/// writes into the removed directory and makes it again. A static in some other test's
/// directory is left alone.
#[cfg(test)]
pub(super) fn forget_under(directory: &Path) {
    let mut saves = saves();
    let store_in = saves
        .store
        .as_ref()
        .is_some_and(|store| store.path("").starts_with(directory));
    let open_in = saves
        .open
        .as_ref()
        .is_some_and(|open| open.starts_with(directory));
    if store_in || open_in {
        *saves = Saves::default();
    }
}

/// The store and the project file the app is in, or `None` when it is in none — in which
/// case nothing is written and nothing is made. The session goes beside the file. Also
/// what a delete takes away.
pub(super) fn writing_into(saves: &Saves) -> Option<(Store, PathBuf)> {
    Some((saves.store.clone()?, saves.open.clone()?))
}

/// Any IO failure is logged and swallowed: failing to persist is never worth interrupting
/// the user for. Answers whether the file was written, which is what says whether the
/// baseline behind it may move: a save recorded as done is a save nothing retries.
pub(super) fn write_or_warn(path: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> bool {
    if let Err(error) = write(path) {
        log::warn!("could not save {}: {error}", path.display());
        return false;
    }
    true
}

#[cfg(test)]
mod tests;

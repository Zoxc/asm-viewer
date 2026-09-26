//! Reading binaries onto the objects list: the placeholder a file is in the list as from
//! the moment it is asked for, the worker thread each load is read on, the batches its
//! answers are drained in, and which load each answer belongs to.
//!
//! [`open_binaries`] is **the one path by which anything is ever added to `objects`**. The
//! Objects panel's "Add binaries...", a Files row's "Open file", a session restore and a
//! build's reopening all go through it or its two halves, so they cannot differ about what
//! opening a file means. Nothing here opens or closes a tab: the opposite number is
//! `close_binary` (`documents.rs`), which cancels the load as its last act.
//!
//! [`Loading`] sits here and not in `state.rs`, a context living with the mechanism that
//! fills it.

use super::*;

/// The files being read into [`Objects`] right now, so the sidebar can say so. A state of
/// its own because it is about what that list has *not* got yet: a file appears here when
/// it is asked for and leaves when nothing more is coming out of it, whether or not it
/// produced anything at all. See [`Loads`] and [`open_binaries`].
#[derive(Clone, Copy)]
pub(crate) struct Loading(pub(crate) State<Loads>);

/// Read and parse `paths` on a worker thread, putting each object into the list as it is
/// parsed.
///
/// The channel is unbounded -- the worker should run flat out -- and drained in batches, a
/// write per member being a re-render per member.
pub(crate) async fn open_binaries(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    paths: Vec<PathBuf>,
) {
    let (id, paths) = begin_load(objects, loading, paths, &[]);
    if paths.is_empty() {
        return;
    }
    read_binaries(objects, loading, id, paths).await;
}

/// The first half of [`open_binaries`]: put a [placeholder](Object::placeholder) for each
/// of `paths` into the list and register them as being read, now rather than at the task's
/// first poll. Hands back the load's id and the paths it is to read.
///
/// So the file is in the app from the moment it is asked for: drawn, held by the project
/// file, and in its place in the list, which is where its first object lands. A caller
/// whose load the save observer must see the next time it runs calls this itself: a
/// restore, and a build's reopen. The task reading it must not be dropped before it runs,
/// or the load is never finished.
///
/// **A file the app holds already is left out** ([`crate::tree::holds`]): opening a path a
/// second time would put a second copy of each of its objects in the list. Checked here,
/// on the one path in, so no caller can forget it -- the Add dialog did. `order` is what a
/// file read again goes back among ([`crate::tree::slot`]).
pub(crate) fn begin_load(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    paths: Vec<PathBuf>,
    order: &[PathBuf],
) -> (LoadId, Vec<PathBuf>) {
    let paths: Vec<PathBuf> = {
        let held = objects.peek();
        let mut wanted: Vec<PathBuf> = Vec::new();
        for path in paths {
            if !crate::tree::holds(&held, &path) && !wanted.contains(&path) {
                wanted.push(path);
            }
        }
        wanted
    };
    if !paths.is_empty() {
        let mut objects = objects.write();
        for path in &paths {
            let placeholder = Arc::new(Object::placeholder(path.clone()));
            crate::tree::place(&mut objects, placeholder, order);
        }
    }
    let id = loading.write().begin(&paths);
    (id, paths)
}

/// The second half: read the load [`begin_load`] registered as `id`.
pub(crate) async fn read_binaries(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    id: LoadId,
    paths: Vec<PathBuf>,
) {
    // Unbounded: the worker should run flat out. What stops it is the receiver going,
    // which is `take_load` deciding that nothing more from this load is wanted. The worker
    // learns that only when a send fails, so a closed archive stops a member or two later,
    // and a closed object file is parsed to the end, its first answer, and dropped.
    let events = stream("the binary reader", None, move |emit| {
        open_files_streaming(paths, emit)
    });

    take_load(objects, loading, id, events).await;
}

/// Take one load's answers until it has nothing left to say.
///
/// An object nobody asked for any more is dropped rather than prevented: the worker is
/// already parsing when the file is closed. It is checked against `Loads::holds` -- the
/// load *and* the path, since a file closed and reopened mid-parse is two loads.
///
/// Returning is what stops the worker: it drops the receiver, the next `send_blocking`
/// fails, and the walk breaks where it stands. A close sends nothing here, so it is
/// noticed only when the worker's next answer wakes this: an object file is always parsed
/// to the end.
pub(crate) async fn take_load(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    id: LoadId,
    events: async_channel::Receiver<Progress>,
) {
    // A batch per wake, so a burst costs one write.
    while let Some(batch) = next_batch(&events).await {
        // Both lists are worked out under one read guard and the guard is gone before
        // anything writes.
        let (parsed, finished) = {
            let held = loading.peek();
            let mut parsed: Vec<Arc<Object>> = Vec::new();
            let mut finished: Vec<PathBuf> = Vec::new();
            for progress in batch {
                match progress {
                    Progress::Parsed(object) if held.holds(id, &object.path) => parsed.push(object),
                    // An object for a file this load no longer holds: the reader closed
                    // it, or left the project, while it was being parsed.
                    Progress::Parsed(_) => {}
                    Progress::Finished(path) => finished.push(path),
                }
            }
            (parsed, finished)
        };

        if !parsed.is_empty() {
            let mut objects = objects.write();
            for object in parsed {
                // In place of the file's placeholder, then after its last object, never
                // simply at the end: two loads running at once interleave their batches,
                // and the Objects list groups a file by the run its objects make
                // (`crate::tree`). Appended, one archive would be drawn as several file
                // rows, each with a fold of its own, for the rest of the session.
                crate::tree::place(&mut objects, object, &[]);
            }
        }
        if !finished.is_empty() {
            let mut held = loading.write();
            for path in finished {
                held.finished(id, &path);
            }
        }

        // Nothing left that this load could be asked about: it is done, or everything it
        // was reading has been closed. Returning drops the receiver, which is what tells
        // the worker.
        if !loading.peek().active(id) {
            return;
        }
    }

    // The worker is gone without finishing every path: its thread would not start, or
    // died. Nothing more is coming, so a path left here would be drawn as loading and
    // would hold off every session save for the rest of the run. A placeholder no object
    // replaced stays: the file is still the project's, and closing it is the reader's.
    let active = loading.peek().active(id);
    if active {
        loading.write().end(id);
    }
}

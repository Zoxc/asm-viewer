//! Reading binaries onto the objects list: the one worker thread, the batches its answers
//! are drained in, and which load each answer belongs to.
//!
//! [`open_binaries`] is **the one path by which anything is ever added to `objects`**. The
//! toolbar's Open, a session restore and a scratchpad's rebuild all go through it, so they
//! cannot differ about what opening a file means. Nothing here opens or closes a tab: the
//! opposite number is `close_binary` (`documents.rs`), which cancels the load as its last
//! act.
//!
//! [`Loading`] sits here and not in `state.rs`, a context living with the mechanism that
//! fills it.

use super::*;

/// The files being read into [`Objects`] right now, so the sidebar can say so. A state of
/// its own because it is about what that list has *not* got: a file appears here when it
/// is asked for and leaves when nothing more is coming out of it, whether or not it
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
    // Registered before a byte is read, so the rows are on screen for the whole wait.
    let id = {
        let mut loading = loading;
        loading.write().begin(&paths)
    };

    // Unbounded: the worker should run flat out. What stops it is the receiver going,
    // which is `take_load` deciding that nothing more from this load is wanted -- and is
    // what keeps a closed 331 MB file from being parsed to the end into a value that will
    // be dropped.
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
/// fails, and the walk breaks where it stands.
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
                // After the last object of the same file, never simply at the end: two
                // loads running at once interleave their batches, and the Objects list
                // groups a file by the run its objects make (`crate::tree`). Appended,
                // one archive would be drawn as several file rows, each with a fold of
                // its own, for the rest of the session.
                match objects.iter().rposition(|held| held.path == object.path) {
                    Some(last) => objects.insert(last + 1, object),
                    None => objects.push(object),
                }
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
}

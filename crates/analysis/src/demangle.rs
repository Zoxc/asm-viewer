//! One object's symbol names, demangled: on stacks big enough for the deepest of them, and
//! on more than one core.
//!
//! Demangling is the last of the open-time cost that is not simply reading the file, and it
//! is embarrassingly parallel — every name is independent of every other. What makes it more
//! than a `map` over threads is the stack: **a demangler's recursion depth is the file's to
//! choose** ([`MAX_MANGLED_NAME`]), so every thread that may touch a name needs
//! [`DEMANGLE_STACK`], and a thread that big is not something to create per object. Hence a
//! [`Pool`]: a fixed, process-wide set of them, started once and handed work.

use std::{
    ops::Range,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Sender},
        Arc, Mutex, OnceLock,
    },
};
use symbolic_demangle::{Demangle, DemangleOptions};

/// The longest mangled name this crate will hand to a demangler.
///
/// **A demangler's recursion depth is the file's to choose**, and a stack overflow is an
/// **abort** no `catch_unwind` turns back into "this symbol has no demangled name", so it has
/// to be headed off before the call. `msvc-demangler` 0.11 has no recursion limit at all
/// (one level per `P` byte) and `cpp_demangle`'s is deep enough that reaching it is megabytes
/// of stack. Measured at roughly 10 KiB of stack per byte of name, so this and
/// [`DEMANGLE_STACK`] are one bound. The longest name in any sample in the repo is 1038
/// bytes; a name past the cap is displayed exactly as the file wrote it.
const MAX_MANGLED_NAME: usize = 2048;

/// The stack every thread that demangles gets; see [`MAX_MANGLED_NAME`]. A *reservation*:
/// pages are committed only as they are touched.
const DEMANGLE_STACK: usize = 64 << 20;

/// A name short enough to demangle on the caller's own stack: measured, the worst-case
/// 64-byte name demangles inside 1 MiB while a 96-byte one overflows it. Below that line a
/// small batch is the caller's own business and a hand-off is pure cost.
const SHORT_MANGLED_NAME: usize = 64;

/// How many names a thread takes off the batch at a time.
///
/// Grains are taken as a thread frees rather than dealt out up front, because the cost of a
/// name is superlinear in its length and where the long ones sit is the file's business: an
/// even split of 115k names hands one thread the object's whole C++ section. Small enough
/// that the tail is short, large enough that the atomic and the hand-back are noise beside
/// the demangling of 256 names.
const GRAIN: usize = 256;

/// The most threads the pool holds. Demangling is a fraction of an open, so the cores past
/// this buy proportionally less and each costs a live thread whose deepest recursion stays
/// committed for the life of the process.
const MAX_THREADS: usize = 8;

/// One object's mangled names, in symbol order: [`None`] for a name with nothing to demangle
/// (the entry point's, which is this crate's own).
///
/// Shared rather than borrowed, because the pool's threads outlive any one batch and a job
/// handed to them cannot borrow the caller's frame. Nothing is copied to make it: the caller
/// moves its names in here and takes them back out afterwards.
pub(crate) type Names = Arc<Vec<Option<String>>>;

/// Demangle a whole object's names. The answer is one entry per name, in the same order,
/// whatever it was demangled by and wherever it was demangled.
///
/// [`None`] out means no demangler recognised the name, it was longer than
/// [`MAX_MANGLED_NAME`], or the demangler panicked — all of which display as the file wrote
/// it, which is what an unrecognised name already did.
pub(crate) fn batch(names: &Names) -> Vec<Option<String>> {
    // The deepest any of them can recurse is the longest of them.
    let deepest = names.iter().flatten().map(|name| name.len()).max();
    match deepest {
        None | Some(0) => return vec![None; names.len()],
        // Short names and few of them: the caller's own stack, and no hand-off at all.
        // Every fixture in the test suite is this.
        Some(deepest) if deepest <= SHORT_MANGLED_NAME && names.len() <= GRAIN => {
            return demangle_range(names, 0..names.len())
        }
        Some(_) => {}
    }

    match pool() {
        Some(pool) => parallel(pool, names),
        // A pool that would not start is one more reason for this to stay what it was.
        None => sequential(names),
    }
}

/// One name, or [`None`] where nothing is to be made of it.
///
/// The `catch_unwind` is not general defensiveness: a demangler panicking on a name out of a
/// string table would otherwise take out the parse, or a pool thread with it.
fn demangle_one(name: &str) -> Option<String> {
    let name = Some(name).filter(|name| name.len() <= MAX_MANGLED_NAME)?;
    std::panic::catch_unwind(|| {
        symbolic_common::Name::from(name).demangle(DemangleOptions::complete())
    })
    .ok()
    .flatten()
}

/// A run of the batch, demangled in place order. The one definition of what a chunk of work
/// is, so the parallel path and the sequential one cannot answer differently.
fn demangle_range(names: &[Option<String>], range: Range<usize>) -> Vec<Option<String>> {
    names[range]
        .iter()
        .map(|name| demangle_one(name.as_deref()?))
        .collect()
}

/// How many of the pool's threads a batch of `len` names is worth asking for: enough that
/// each has a grain to start on, never more than there are threads, never none.
fn jobs_for(len: usize, threads: usize) -> usize {
    len.div_ceil(GRAIN).clamp(1, threads.max(1))
}

/// The whole batch on one thread of the pool's size, for when there is no pool.
fn sequential(names: &Names) -> Vec<Option<String>> {
    let len = names.len();
    let names = names.clone();
    std::thread::Builder::new()
        .stack_size(DEMANGLE_STACK)
        .spawn(move || demangle_range(&names, 0..names.len()))
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or_else(|| vec![None; len])
}

/// The batch across the pool: `jobs_for` jobs pulling grains off one cursor until it is
/// past the end, each handing back what it did and where it did it.
///
/// **The answer does not depend on which thread got which grain.** A result carries the
/// index it starts at and is written there, so the order is the batch's own and two runs
/// over the same names answer identically.
fn parallel(pool: &Pool, names: &Names) -> Vec<Option<String>> {
    let len = names.len();
    let cursor = Arc::new(AtomicUsize::new(0));
    let (done, results) = mpsc::channel::<(usize, Vec<Option<String>>)>();

    let mut dispatched = 0;
    for _ in 0..jobs_for(len, pool.threads) {
        let names = names.clone();
        let cursor = cursor.clone();
        let done = done.clone();
        let job: Job = Box::new(move || {
            loop {
                let start = cursor.fetch_add(GRAIN, Ordering::Relaxed);
                if start >= len {
                    break;
                }
                let end = start.saturating_add(GRAIN).min(len);
                // A panic here is a job that hands back nothing rather than a pool thread
                // that dies; `demangle_one` already guards each name, so this is for the
                // allocation around them.
                let values = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    demangle_range(&names, start..end)
                }))
                .unwrap_or_else(|_| vec![None; end - start]);
                if done.send((start, values)).is_err() {
                    break;
                }
            }
        });
        if pool.jobs.send(job).is_err() {
            break;
        }
        dispatched += 1;
    }
    // Held until here so the receive loop below ends when the jobs' own copies are dropped,
    // and not before.
    drop(done);

    if dispatched == 0 {
        return sequential(names);
    }

    let mut demangled: Vec<Option<Option<String>>> = vec![None; len];
    while let Ok((start, values)) = results.recv() {
        for (slot, value) in demangled[start..].iter_mut().zip(values) {
            *slot = Some(value);
        }
    }
    // The outer `Option` is "a job answered for this name". A grain that never came back —
    // only possible if a pool thread died under it — leaves its names as the file wrote
    // them, which is the same answer an unrecognised name gets.
    demangled.into_iter().map(Option::flatten).collect()
}

/// A job for the pool. `'static` and owning what it touches: the threads it runs on are the
/// process's and outlive whoever submitted it.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed set of threads with stacks sized for the deepest name a file can state
/// ([`DEMANGLE_STACK`]), started on the first batch that needs one and kept for the life of
/// the process.
///
/// Not a general executor and deliberately not a dependency: `rayon` is in the lock already
/// (transitively, under `image`), but its threads would still have to be a pool of this
/// crate's own to get the stack size, which is the whole of what is hard here — what is left
/// is a queue and a cursor. **A job never submits a job**, which is why a bounded pool
/// cannot deadlock itself no matter how many opens are in flight.
struct Pool {
    jobs: Sender<Job>,
    threads: usize,
}

/// The one pool, or [`None`] if not a single thread of it would start.
fn pool() -> Option<&'static Pool> {
    static POOL: OnceLock<Option<Pool>> = OnceLock::new();
    POOL.get_or_init(Pool::start).as_ref()
}

impl Pool {
    fn start() -> Option<Pool> {
        let threads = std::thread::available_parallelism()
            .map_or(1, |threads| threads.get())
            .min(MAX_THREADS);
        let (jobs, queue) = mpsc::channel::<Job>();
        let queue = Arc::new(Mutex::new(queue));

        let started = (0..threads)
            .filter(|_| {
                let queue = queue.clone();
                std::thread::Builder::new()
                    .name("demangle".to_owned())
                    .stack_size(DEMANGLE_STACK)
                    .spawn(move || loop {
                        // The lock is held across the `recv` — the waiting thread is the one
                        // holding it and hands it on the moment a job arrives, so a job is
                        // never left sitting. It is not held while the job runs, which is
                        // the part that would serialise the pool.
                        let Ok(job) = ({
                            let Ok(queue) = queue.lock() else { return };
                            queue.recv()
                        }) else {
                            // Only when the sender is gone, and it is a `static`: this is
                            // the thread's exit and nothing more.
                            return;
                        };
                        job();
                    })
                    .is_ok()
            })
            .count();

        (started > 0).then(|| Pool {
            jobs,
            threads: started,
        })
    }
}

#[cfg(test)]
mod tests;

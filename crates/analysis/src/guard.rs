//! The calls whose panics are caught on purpose, and how anything else can tell.
//!
//! Two of them: a demangler let loose on a name out of a string table, and a debug format
//! read by a dependency that does unchecked arithmetic on what the file says. Both are the
//! crate's guard against a *dependency's* bug on file input, never against its own
//! (`AGENTS.md`), and a caught panic there is nothing gone wrong with the app: a name is
//! left undemangled, or a line has no source.
//!
//! A panic hook runs on the panicking thread **before** the unwind reaches any
//! `catch_unwind`, so a hook cannot see that one of these will be caught. [`guarded`] is
//! how it is told: the call raises a count on its own thread for as long as it runs, and a
//! hook asking during a panic gets the answer for the thread that panicked.

use std::{
    cell::Cell,
    panic::{self, AssertUnwindSafe},
};

thread_local! {
    /// How many guarded calls this thread is inside. A count and not a flag: the demangle
    /// pool's per-job guard has the per-name one inside it.
    static GUARDS: Cell<usize> = const { Cell::new(0) };
}

/// Whether this thread is inside a [`guard`]ed call, and so whether a panic on it now is
/// one the crate hardens against rather than one that has broken the app.
pub fn guarded() -> bool {
    GUARDS.with(|guards| guards.get() > 0)
}

/// Run `f`, answering [`None`] where it panicked, and say so on this thread while it runs.
///
/// The count is put back whichever way `f` leaves, the unwind included, since the value is
/// restored after `catch_unwind` returns and not by a guard object that a panic would
/// unwind through -- there is nothing to unwind through here.
pub fn guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    GUARDS.with(|guards| guards.set(guards.get().saturating_add(1)));
    let answer = panic::catch_unwind(AssertUnwindSafe(f)).ok();
    GUARDS.with(|guards| guards.set(guards.get().saturating_sub(1)));
    answer
}

#[cfg(test)]
mod tests;

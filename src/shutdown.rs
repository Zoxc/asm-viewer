//! The end of the process: everything that has to happen before it, in one place.

use crate::{process, project, scratchpad, settings};

/// Everything that must happen before the process ends: every program the app started
/// stopped, then the project, the settings and the scratchpads saved.
///
/// The window's close hook and the panic hook's shutdown thread are the two ways the app
/// comes down, and nothing else may end the process. Both call this, so neither can drift
/// from the other: a copy of the sequence missing the stop leaves rust-analyzer running
/// after the app is gone, with the `cargo`, `rustc` and proc-macro server it forked behind
/// it, and a scratchpad's program holding whatever it holds.
///
/// One stop because there is one list of started programs (`src/process.rs`): a stop that
/// reaches only half of them is not something this can be written wrongly as any more.
pub fn before_exit() {
    in_order(
        process::stop_all,
        [project::flush, settings::flush, scratchpad::flush],
    );
}

/// `stop`, then each of `saves`, none of them kept from running by a panic in another.
///
/// The stop comes first because a save can fail to return. It can block on a lock the
/// thread that panicked holds, and only that thread's unwind lets it go; or it can panic
/// itself, and the shutdown thread would then unwind past its `exit`. The stop takes only
/// `process`'s own locks. The panic is still written down by the hook; it is caught only
/// so the saves after it and the exit still happen.
fn in_order(stop: fn(), saves: [fn(); 3]) {
    for step in std::iter::once(stop).chain(saves) {
        let _ = std::panic::catch_unwind(step);
    }
}

#[cfg(test)]
mod tests;

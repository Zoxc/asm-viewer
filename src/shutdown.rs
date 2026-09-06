//! The end of the process: everything that has to happen before it, in one place.

use crate::{process, project};

/// Everything that must happen before the process ends: the projects saved, then every
/// program the app started stopped.
///
/// The window's close hook and the panic hook's shutdown thread are the two ways the app
/// comes down, and nothing else may end the process. Both call this, so neither can drift
/// from the other: a copy of the sequence missing the stop leaves rust-analyzer running
/// after the app is gone, with the `cargo`, `rustc` and proc-macro server it forked behind
/// it, and a scratchpad's program holding whatever it holds.
///
/// Two lines because there is one list of started programs (`src/process.rs`): a stop that
/// reaches only half of them is not something this can be written wrongly as any more.
pub fn before_exit() {
    project::flush();
    process::stop_all();
}

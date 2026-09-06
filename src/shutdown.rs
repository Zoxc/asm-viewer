//! The end of the process: everything that has to happen before it, in one place.

use crate::{lsp, project, scratchpad};

/// Everything that must happen before the process ends: the projects saved, then every
/// child the app started stopped.
///
/// The window's close hook and the panic hook's shutdown thread are the two ways the app
/// comes down, and nothing else may end the process. Both call this, so neither can drift
/// from the other: a copy of the sequence missing `lsp::stop_all` leaves rust-analyzer
/// running after the app is gone, with the `cargo`, `rustc` and proc-macro server it
/// forked behind it.
pub fn before_exit() {
    project::flush();
    scratchpad::stop_all();
    lsp::stop_all();
}

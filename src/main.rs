mod bookmarks;
mod cargo;
mod chars;
mod compiled;
mod docs;
mod files;
mod filter;
mod fonts;
mod functions;
mod history;
mod lanes;
mod lsp;
mod naming;
mod panics;
mod pixels;
mod process;
mod project;
mod rescue;
mod reveal;
mod rows;
mod scratchpad;
mod search;
mod section;
mod settings;
mod source;
mod tabs;
mod tree;
mod ui;
mod visits;

use freya::prelude::*;

fn main() {
    env_logger::init();
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(ui::app)
                .with_title("Assembly Viewer")
                .with_size(1200., 800.)
                // The only exit hook freya 0.4 offers, and it is a `Send` callback outside
                // the component tree, so nothing here can read UI state. It covers the
                // window being closed normally, not a kill or a crash; the periodic flush
                // bounds the loss there.
                .with_on_close(|_, _| {
                    project::flush();
                    scratchpad::stop_all();
                    lsp::stop_all();
                    CloseDecision::Close
                }),
        ),
    );
}

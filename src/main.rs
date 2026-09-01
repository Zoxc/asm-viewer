mod docs;
mod filter;
mod fonts;
mod history;
mod lanes;
mod project;
mod rows;
mod scratchpad;
mod settings;
mod source;
mod tabs;
mod tree;
mod ui;

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
                    CloseDecision::Close
                }),
        ),
    );
}

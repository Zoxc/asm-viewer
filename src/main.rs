mod fonts;
mod history;
mod project;
mod ui;

use freya::prelude::*;

fn main() {
    env_logger::init();
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(ui::app)
                .with_title("Assembly Viewer")
                .with_size(1200., 800.)
                // The one exit hook freya 0.4 offers: `WindowConfig::with_on_close`
                // runs on `WindowEvent::CloseRequested`, before the window is dropped
                // and the event loop is told to exit, and its return value decides
                // whether the close goes ahead. It is a `Send` callback outside the
                // component tree, which is exactly why the save policy lives in a
                // `static` in `project` rather than in a `State`: nothing here can
                // read the UI's state, but `flush` needs no argument.
                //
                // It covers the normal way out -- the user closing the window. It does
                // not cover a kill, a crash, or a session logout that does not send a
                // close request; the periodic flush is what bounds the loss there.
                .with_on_close(|_, _| {
                    project::flush();
                    CloseDecision::Close
                }),
        ),
    );
}

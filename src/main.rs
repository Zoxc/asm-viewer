mod fonts;
mod project;
mod ui;

use freya::prelude::*;

fn main() {
    env_logger::init();
    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(ui::app)
                .with_title("Assembly Viewer")
                .with_size(1200., 800.),
        ),
    );
}

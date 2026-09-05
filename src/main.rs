mod bookmarks;
mod cargo;
mod chars;
mod compiled;
mod docs;
mod files;
mod filter;
mod fonts;
mod functions;
mod fuzzy;
mod history;
mod lanes;
mod links;
mod lsp;
mod naming;
mod panics;
mod pixels;
mod process;
mod project;
mod references;
mod rescue;
mod reveal;
mod rows;
mod scratchpad;
mod search;
mod section;
mod settings;
mod source;
mod tabs;
#[cfg(test)]
mod temporary;
mod tree;
mod ui;
mod visits;
mod walk;

use freya::prelude::*;

fn main() {
    env_logger::init();

    // One optional argument: the project to open, in place of the one last open. Checked
    // here rather than in the app so that a path that is not a project can be answered on
    // the command line it came from and the window never opens -- a windowed program that
    // starts and says nothing has said nothing.
    let opening = match std::env::args_os().nth(1).map(std::path::PathBuf::from) {
        Some(path) if project::is_project_file(&path) => Some(path),
        Some(path) => {
            eprintln!("{}: not a project file", path.display());
            return;
        }
        None => None,
    };

    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new(move || ui::app(opening.clone()))
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

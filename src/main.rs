mod bookmarks;
mod cargo;
mod chars;
mod compiled;
mod dialog;
mod docs;
mod document;
mod files;
mod filter;
mod find;
mod fonts;
mod functions;
mod fuzzy;
mod grouped;
mod history;
mod lanes;
mod languages;
mod links;
mod lsp;
mod naming;
mod order;
mod panics;
mod pixels;
mod positions;
mod process;
mod project;
mod references;
mod reveal;
mod scratchpad;
mod search;
mod section;
mod settings;
mod shared;
mod shortcuts;
mod shutdown;
mod source;
mod store;
mod tabs;
#[cfg(test)]
mod temporary;
mod tree;
mod ui;
mod verdict;
mod visits;
mod walk;

use freya::prelude::*;

/// A test-only counter: how many times this thread did the thing the module counts.
///
/// The three pieces every counter in the app is made of, written once -- the cell, the
/// reader that answers it, and the `#[cfg(test)]` on both:
///
/// ```ignore
/// counter!(
///     /// Test-only: how many times this thread has asked the filesystem about a file.
///     pub fn touches() = TOUCHES
/// );
/// ```
///
/// **The bump stays where the thing is done**, one `#[cfg(test)]` line at the top of it:
/// `TOUCHES.set(TOUCHES.get() + 1);`. That line is what keeps the count out of a release
/// build, and what a reader of that function sees.
///
/// A counter is a thread-local because `freya-testing` runs the whole app on the test's
/// own thread, which makes one the only way to settle that a render read no file, copied
/// nothing, or drew no row twice. Nothing resets a counter: a test takes the count before
/// and after what it is about.
///
/// At the crate root because four of the modules that count are not under `ui`, and
/// nothing outside the UI may reach into it.
macro_rules! counter {
    ($(#[$doc:meta])* $vis:vis fn $reader:ident() = $cell:ident) => {
        $(#[$doc])*
        #[cfg(test)]
        $vis fn $reader() -> usize {
            $cell.get()
        }

        #[cfg(test)]
        thread_local! {
            static $cell: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        }
    };
}

pub(crate) use counter;

/// What the app is called, wherever it is spelled: the window's title, the title over a
/// box the reader is shown, and the name the language server is told its client has.
pub const APP_NAME: &str = "Assembly Viewer";

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
                .with_title(APP_NAME)
                .with_size(1200., 800.)
                // The only exit hook freya 0.4 offers, and it is a `Send` callback outside
                // the component tree, so nothing here can read UI state. It covers the
                // window being closed normally, not a kill or a crash; the periodic flush
                // bounds the loss there.
                .with_on_close(|_, _| {
                    shutdown::before_exit();
                    CloseDecision::Close
                }),
        ),
    );
}

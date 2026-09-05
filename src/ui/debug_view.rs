//! The Debug page: the ways to make the app do the things it only does when something has
//! gone wrong, so that what it then does can be looked at.
//!
//! **It exists for `crate::panics`.** What that module does -- the record it writes, the
//! box it puts up, the backtrace it cuts down, the shutdown it starts -- happens on a path
//! nothing can reach on purpose, and a headless test can pin the rules but not the box,
//! which is the desktop's own. So the three panics the hook tells apart are three buttons
//! here, and looking at the box is one press rather than a patched build.
//!
//! The three are the whole of what the hook distinguishes: one on the UI thread, which is
//! the one that leaves no frame to draw a window of the app's in; one on a thread of its
//! own, which is what a worker dying looks like; and one inside `analysis::guard`, which
//! is written down and nothing else.
//!
//! Under them, what those panics left behind: one row per run that has panicked, and a
//! press shows that run's file where the reader's other files are. The box a panic puts
//! up names its file and then the app goes down, which is a path to read once and never
//! find again -- and a guarded panic puts up no box at all, so its file is named here or
//! nowhere.

use super::*;

/// The page's body.
#[derive(Clone, PartialEq)]
pub(crate) struct DebugTab;

impl Component for DebugTab {
    fn render(&self) -> impl IntoElement {
        rect()
            .expanded()
            .background(palette().pane_bg)
            .font(&fonts().ui)
            .color(palette().text_fg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Panics", None))
                        .child(info_line(
                            "Each of these is a real panic. The app writes the record, \
                             says so, and stops -- except the guarded one, which is \
                             written down and nothing more."
                                .to_owned(),
                        ))
                        .child(panic_row("On the UI thread", |_| {
                            panic!("a panic asked for on the Debug page")
                        }))
                        .child(panic_row("On a worker thread", |_| panic_off_thread()))
                        .child(panic_row("Inside analysis::guard", |_| {
                            analysis::guard::guard(|| {
                                panic!("a guarded panic asked for on the Debug page")
                            });
                        }))
                        .child(section_heading("Panic files", None))
                        .children(recorded_rows()),
                ),
            )
    }
}

/// One row: what the panic is on the left, taking whatever the button leaves, and the
/// button on the right.
///
/// **Not `field_row`.** That one's name column is [`field_label_width`] wide, with
/// nothing clipping it, which suits the Settings page's one-word names and not a sentence:
/// the text drew past its box and straight over the button beside it. Here the name takes
/// the room that is left and is cut with an ellipsis where there is not enough, which is
/// [`tree_name`]'s arrangement and for the same reason.
fn panic_row(name: &str, press: impl FnMut(Event<PressEventData>) + 'static) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(tree_name(name.to_owned(), false))
        .child(Button::new().on_press(press).child("Panic"))
}

/// A row per run that has panicked, newest first, or a line saying there are none.
///
/// **Read on every render and not held.** The list changes when this app panics, which is
/// the one moment nothing here will be redrawn afterwards; and it is one `read_dir` of a
/// directory with a handful of files in it, on a page nobody has open by accident.
fn recorded_rows() -> Vec<Element> {
    let files = crate::panics::recorded();
    if files.is_empty() {
        return vec![info_line("Nothing has panicked.".to_owned()).into_element()];
    }
    files
        .into_iter()
        .map(|path| FileRow { path }.into_element())
        .collect()
}

/// One panic file: what it is called, and a press that shows it in the file manager.
///
/// A component and not a helper, for the hover: there is no `.hover()` pseudo-state, so a
/// row that lights under the pointer is a scope with a `use_state` in it.
#[derive(PartialEq)]
struct FileRow {
    path: PathBuf,
}

impl Component for FileRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let path = self.path.clone();
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());

        rect()
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .horizontal()
            .cross_align(Alignment::Center)
            .padding(Gaps::new_symmetric(0.0, 4.0))
            .corner_radius(4.0)
            .background(match hovering() {
                true => palette().object_hover_bg,
                false => Color::TRANSPARENT,
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            // `spawn_forever` is not needed: nothing here takes this row down, and the
            // call is a thread of `reveal`'s own either way.
            .on_press(move |_| reveal::reveal(path.clone()))
            .child(tree_name(name, false))
    }
}

/// Panic on a thread of this call's own: what a worker dying looks like, which is the case
/// the box exists for at all -- the pane it was working for is left waiting, and nothing
/// on screen would otherwise say why.
///
/// Named, like every thread this app starts, because the record says which thread died.
fn panic_off_thread() {
    let started = std::thread::Builder::new()
        .name("the Debug page's panic".to_owned())
        .spawn(|| panic!("a panic asked for on the Debug page"));
    if let Err(error) = started {
        log::warn!("the panicking thread could not be started: {error}");
    }
}

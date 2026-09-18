//! The two things the language server is drawn as: the control in the top bar that
//! starts and stops it, and the band that asks before a first start.
//!
//! Both draw what `src/ui/language.rs` holds and press what it offers. Neither decides
//! anything of its own.

use super::*;

/// What the control is called. Not the program's name: the bar has room for three letters
/// beside two chevrons, and what the reader is being told is which of the app's parts this
/// is. The program's own name is in the tooltip and in the Project view.
const SERVER_NAME: &str = "LSP";

/// The control in the top bar: one press starts the language server, the next stops it.
///
/// Named, bordered and coloured rather than an icon alone. It is the only thing in the app
/// that starts a process the reader did not ask for by name, so what it is about is
/// written on it, and its state is the border and the colour rather than a shape a reader
/// has to have learned. **Nothing about it changes width** -- the two history buttons sit
/// beside it at the bar's right corner, and a label or an icon that grew would walk them
/// out from under the pointer -- so the state is said in the same three letters, the same
/// square of icon, and the tooltip.
///
/// The icon is a link and not a pair of braces: braces are code, which is what every other
/// icon in this app is already about -- a file of it, a function in it, a binary of it --
/// and they say nothing about what this one is for. What a language server is asked here is
/// where a name leads, and a link is that question rather than the machinery answering it.
/// It also holds its shape at the size the bar draws it, which a magnifier over a `</>`
/// does not. Beside three letters naming the kind of server, that is the whole caption.
///
/// Off it is text alone, with no border: a part of the app nobody has asked anything of
/// should not look like it is holding something. A border is what says a press would do
/// something, so it comes up under the pointer and stays while a server is there; running
/// puts `server_bg` under it, the one colour of its own in the app, since a process the
/// reader started is worth telling apart from a toggle that happens to be on; and
/// something going on -- starting, or a server reading the project -- turns the icon into
/// a loader, the only moving thing in the bar, which says an answer is not ready rather
/// than not there.
#[derive(Clone, PartialEq)]
pub(crate) struct ServerButton;

impl Component for ServerButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let language = use_consume::<Talking>().0;
        let proj = use_consume::<Proj>().0;
        let jobs = use_consume::<LspJobs>();

        let held = language.read();
        let open = proj.read();
        let directory = open.workspace();

        // With no directory there is nothing to run a server over.
        let live = directory.is_some();
        let tooltip = held.words(&open.server(), directory.as_deref());

        let square = icon_size();
        // Dim only where a press would do nothing. Off is a control the reader is meant
        // to find, not one that is unavailable, so it is written as plainly as the two
        // buttons beside it; what says it is off is the lack of a border and a colour.
        let colour = match (&held.state, live) {
            (_, false) => dimmed(palette().icon_fg, palette().pane_bg),
            (Lsp::Failed(_), _) => palette().invalid_fg,
            _ => palette().icon_fg,
        };
        // A border under the pointer, and while there is a server to press about; none at
        // all when it is off and nothing is over it. A server that is there wears the
        // icon's own colour faded into whatever the box encloses -- a line around
        // something working should be quieter than the thing inside it, and a failure is
        // the one state that gets the full colour, being the one worth looking at.
        let edge = match (&held.state, live && hovering()) {
            (Lsp::Failed(_), _) => palette().invalid_fg,
            (Lsp::Off, false) => Color::TRANSPARENT,
            (Lsp::Off, true) => palette().hairline,
            (Lsp::Running { .. }, _) => dimmed(colour, palette().server_bg),
            _ => dimmed(colour, palette().pane_bg),
        };
        // A running server wears a ground of its own, over the hover: the one state the
        // control says by its own colour rather than by the pointer being on it.
        let running = matches!(held.state, Lsp::Running { .. });

        TooltipContainer::new(Tooltip::new(tooltip)).child(
            bar_pill(hovering, live, Glow::No)
                .horizontal()
                .spacing(4.0)
                .maybe(running, |button| button.background(palette().server_bg))
                .border(Border::new().fill(edge).width(1.0))
                .maybe(live, |button| {
                    button.on_press({
                        let jobs = jobs.clone();
                        // The same call the window's chord makes, so the two cannot come
                        // to mean different things.
                        move |_| toggle_server(language, proj, &jobs)
                    })
                })
                // The same square either way, so nothing beside it moves.
                .child(
                    rect()
                        .width(Size::px(square))
                        .height(Size::px(square))
                        .center()
                        .child(match held.busy() {
                            true => CircularLoader::new().size(square).into_element(),
                            false => glyph_in(("link", lucide::link()), colour),
                        }),
                )
                .child(label().text(SERVER_NAME.to_owned()).color(colour)),
        )
    }
}

/// The question a start puts when the reader has not agreed to the project's directory:
/// what would be run, what running it means, and where.
///
/// Under the top bar rather than in the Project view's own section, though that section
/// is where the other Start button is: the control above is pressed from wherever the
/// reader happens to be, and a question drawn in a tab they are not looking at is a press
/// that did nothing. It is a band and not a window over the app, and it lays out as
/// nothing while there is nothing to ask.
///
/// It wears `prompt_bg` rather than the bar's `pane_bg`, so that a question standing in
/// front of the app is a surface of its own and not more of the bar it hangs under. Both
/// colours it writes are held legible on that one by the contrast tests.
///
/// The directory is written out, because it is what is being agreed to.
#[derive(Clone, PartialEq)]
pub(crate) struct TrustPrompt;

impl Component for TrustPrompt {
    fn render(&self) -> impl IntoElement {
        let language = use_consume::<Talking>().0;
        let proj = use_consume::<Proj>().0;
        let jobs = use_consume::<LspJobs>();

        let asked = language.read().asking.clone();
        let Some(asking) = asked else {
            return rect().into_element();
        };

        wide_row()
            .padding(Gaps::new_symmetric(6.0, 12.0))
            .background(palette().prompt_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::flex(1.0))
                    .spacing(2.0)
                    .child(
                        label()
                            .text(format!(
                                "Let {} read this directory?",
                                asking.serving.program
                            ))
                            .color(palette().text_fg),
                    )
                    .child(
                        label()
                            .text("It runs the project's own build scripts and macros.".to_owned())
                            .color(palette().address_fg),
                    )
                    .child(dim_line(asking.directory.to_string_lossy().into_owned())),
            )
            .child(
                Button::new()
                    .on_press({
                        let jobs = jobs.clone();
                        move |_| agree_to_start(language, proj, &jobs)
                    })
                    .child("Start it"),
            )
            .child(
                Button::new()
                    .on_press(move |_| decline_start(language))
                    .child("Not now"),
            )
            .into_element()
    }
}

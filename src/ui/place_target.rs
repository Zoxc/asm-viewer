//! The place a diagnostic names, drawn as a target: the one component the Scratchpad pane
//! and the Project view both draw theirs with.
//!
//! The two go to different places -- the pad's editor takes a cursor, a file of the
//! project's opens as a document -- so the press is handed in and each pane reaches for
//! the contexts it needs while *it* renders. What is left is the drawing, and that is the
//! same both times.

use super::*;

/// A place a diagnostic points at, drawn as a **target**: `address_fg` at rest, which is
/// what says "a place" everywhere else in this app, and the relocation link's own hover --
/// the wash under it and the pointer over it -- which is what says "this can be pressed".
///
/// **The press is handed in.** `use_consume` is a hook and may only be called while a
/// component renders, so the contexts a press needs are consumed by whichever pane is
/// drawing this and the handler is closed over what they gave. An [`EventHandler`] never
/// compares equal, so this re-renders whenever the pane does; one label and one hover flag
/// is what that costs.
///
/// **Only a place the pane can actually reach is drawn as one.** Where it cannot, the
/// caller draws a plain label instead: a target that did nothing when pressed is the worse
/// of the two answers, a hover being a promise.
#[derive(Clone, PartialEq)]
pub(crate) struct PlaceTarget {
    /// What it reads: the file, the line and the column.
    pub(crate) text: String,
    /// Where pressing it goes. The press has already been stopped from propagating.
    pub(crate) press: EventHandler<Event<PressEventData>>,
}

impl Component for PlaceTarget {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let press = self.press.clone();

        CursorArea::new().child(
            rect()
                .maybe(hovering(), |rect| link_chrome(rect, None))
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // Both panes draw these inside a `ScrollView` that drags to scroll,
                    // and a press that reached it would be the start of one.
                    e.stop_propagation();

                    press.call(e);
                })
                .child(
                    label()
                        .text(self.text.clone())
                        .max_lines(1)
                        .color(match hovering() {
                            true => palette().name_hover_fg,
                            false => palette().address_fg,
                        }),
                ),
        )
    }
}

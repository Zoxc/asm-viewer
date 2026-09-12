//! The window that says which stored files would not parse and where they were put.
//!
//! A rescue nobody hears about is the same as no rescue, which is why this exists at all:
//! `rescue.rs` takes the file out of the way of the next write, and this is the only place
//! the reader is told it happened.

use super::*;

/// The paths [`Rescued`] holds, until the reader closes it. One of the app's four
/// [`notice`] windows, and drawn as nothing at all while the list is empty.
#[derive(Clone, PartialEq)]
pub(crate) struct RescuedPopup;

impl Component for RescuedPopup {
    fn render(&self) -> impl IntoElement {
        let mut rescued = use_consume::<Rescued>().0;
        let paths = rescued.read().clone();
        // The one way out, spelled once: Escape, a press outside, and the button.
        let mut close = move |_: ()| rescued.set(Vec::new());

        notice(close).maybe(!paths.is_empty(), |popup| {
            popup
                .child(
                    notice_body()
                        .child(label().text(match paths.len() {
                            1 => "This file would not load. It was moved aside:".to_owned(),
                            n => format!("{n} files would not load. They were moved aside:"),
                        }))
                        .children(
                            paths
                                .iter()
                                .map(|path| notice_path(path.display().to_string()).into())
                                .collect::<Vec<Element>>(),
                        ),
                )
                .child(
                    PopupButtons::new().child(
                        Button::new()
                            .on_press(move |_| close(()))
                            .filled()
                            .child("Close"),
                    ),
                )
        })
    }
}

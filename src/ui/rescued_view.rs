//! The window that says which stored files would not parse and where they were put.
//!
//! A rescue nobody hears about is the same as no rescue, which is why this exists at all:
//! `rescue.rs` takes the file out of the way of the next write, and this is the only place
//! the reader is told it happened.

use super::*;

/// How wide the window is: a state directory's path with a file name at the end of it,
/// which is all it draws.
const WIDTH: f32 = 520.0;

/// The paths [`Rescued`] holds, until the reader closes it.
///
/// Not freya's `PopupTitle` or `PopupContent`: both set a font size of their own, which
/// would draw this in a size the reader never chose. `Popup` itself is what is wanted --
/// the overlay layer, the dimmed background, the press outside and the Escape key -- and
/// it shows exactly when it has children, so an empty list is a window that is not there.
#[derive(Clone, PartialEq)]
pub(crate) struct RescuedPopup;

impl Component for RescuedPopup {
    fn render(&self) -> impl IntoElement {
        let mut rescued = use_consume::<Rescued>().0;
        let paths = rescued.read().clone();
        let close = move |_| rescued.set(Vec::new());

        Popup::new()
            .width(Size::px(WIDTH))
            .on_close_request(move |_| rescued.set(Vec::new()))
            .maybe(!paths.is_empty(), |popup| {
                popup
                    .child(
                        rect()
                            .padding(8.0)
                            .spacing(8.0)
                            .font(&fonts().ui)
                            .color(palette().text_fg)
                            .child(label().text(match paths.len() {
                                1 => "This file would not load. It was moved aside:".to_owned(),
                                n => format!("{n} files would not load. They were moved aside:"),
                            }))
                            .children(
                                paths
                                    .iter()
                                    .map(|path| {
                                        // A paragraph and not a label: a path is as long as
                                        // it is, and one that is cut off is one the reader
                                        // cannot go and look at.
                                        paragraph()
                                            .assembly_font()
                                            .color(palette().address_fg)
                                            .span(path.display().to_string())
                                            .into()
                                    })
                                    .collect::<Vec<Element>>(),
                            ),
                    )
                    .child(
                        PopupButtons::new()
                            .child(Button::new().on_press(close).filled().child("Close")),
                    )
            })
    }
}

//! The box over a source name saying what the server called it.
//!
//! At the root, as the file finder's overlay and the context menu's viewer are: a box
//! opened from inside a pane has to outlive the row it is about -- the rows are recycled
//! by the scroll view, and a node that vanishes under the pointer is dropped from freya's
//! hovered set with no leave event, which would strand the flag saying the pointer is
//! inside the box.
//!
//! **It places itself in one layout.** torin works out a node's origin a second time once
//! it knows the node's own size, where the position is not stacked and a side is
//! `auto` (`measure.rs`), so a global position pinned by its `bottom` puts the box's
//! bottom edge on the row's top with nobody measuring anything. `Attached`, which freya's
//! own tooltip is built on, offsets by a height it has to be told, and so costs a render
//! at the wrong place, an `on_sized`, and a render at the right one.

use super::*;

/// Where the box goes, given the name it is about and the window it is in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct HoverPlace {
    /// Whether it sits over the name's row or under it.
    pub(crate) over: bool,
    /// Its left edge, and how wide it is.
    pub(crate) left: f32,
    pub(crate) width: f32,
    /// How tall it may be: what is left between the name's row and the window's edge, and
    /// never more than the box's own limit. What does not fit scrolls.
    pub(crate) room: f32,
}

/// Above the name where the box fits there, under it where it does not, at the name's own
/// left edge, and inside the window whatever the name is near. `tallest` is how tall the
/// box may be at all.
///
/// **Above by preference, and not merely where there is more room.** The box sits over the
/// row the reader is already looking at, and the lines it covers are the ones they have
/// read. A window is taller under a name than over it nearly everywhere, so "more room"
/// put the box below almost always -- over the lines about to be read, and jumping from
/// one side to the other as the pointer moved down the file. The room below decides only
/// where the box would not fit above.
pub(crate) fn hover_place(name: Area, window: Size2D, tallest: f32) -> HoverPlace {
    let above = name.min_y() - HOVER_MARGIN;
    let below = window.height - name.max_y() - HOVER_MARGIN;
    let over = above >= tallest || above >= below;
    let width = HOVER_WIDTH.min(window.width - 2.0 * HOVER_MARGIN).max(0.0);
    HoverPlace {
        over,
        // The right edge first, so a name near the right of the window slides the box
        // left; the left edge after it, so a window narrower than the box wins.
        left: name
            .min_x()
            .min(window.width - width - HOVER_MARGIN)
            .max(HOVER_MARGIN),
        width,
        // What is left on that side, and never more than the box's own limit: a name with
        // pages to say about it would otherwise take the window.
        room: if over { above } else { below }.max(0.0).min(tallest),
    }
}

/// The box, and nothing at all until the server has said something about a name the
/// pointer is on.
#[derive(Clone, PartialEq)]
pub(crate) struct HoverBox;

impl Component for HoverBox {
    fn render(&self) -> impl IntoElement {
        let mut hover = use_consume::<Hovering>().0;
        let marked = use_consume::<Marked>().0;
        // How tall the answer is when nothing is holding it in, which is what says
        // whether there is anything to scroll. Measured rather than asked for: a scroll
        // view is `fill` by default, and one inside a box as tall as what it holds is the
        // two asking each other how tall they are; sized from its content instead, torin
        // clamps what it reports holding and the view believes it has nothing to scroll.
        let mut measured = use_state(|| None::<f32>);

        let held = hover.read().clone();
        let Some((about, said)) = held.showing() else {
            return rect().into_element();
        };
        // Not while a menu is open, which is what freya's own tooltip does, and not while
        // a selection is being swept out, which is what the two pane bars do: a box under
        // the pointer during a gesture is a box in the way of it.
        if ContextMenu::is_open() || sweeping(marked) {
            return rect().into_element();
        }

        let place = hover_place(about.drawn, window_size(), hover_height());
        let position = match place.over {
            // The box's **bottom** edge, at the row's top: see the module's note on why
            // this is one layout and not three.
            true => Position::new_global()
                .left(place.left)
                .bottom(window_size().height - about.drawn.min_y()),
            false => Position::new_global()
                .left(place.left)
                .top(about.drawn.max_y()),
        };

        rect()
            // Clears the inherited clip stack, which is how a box about a row inside a
            // scrolled pane is drawn outside it.
            .layer(Layer::Overlay)
            .position(position)
            .width(Size::px(place.width))
            .background(palette().pane_bg)
            .border(
                Border::new()
                    .width(1.0)
                    .alignment(BorderAlignment::Outer)
                    .fill(palette().hairline),
            )
            .corner_radius(HOVER_RADIUS)
            // Nothing to see until the answer has been measured: the first pass is drawn
            // at the full height the box may have, which is what measures it, and a short
            // answer would otherwise be a box that flashed tall and shrank. `Attached`,
            // freya's own, hides its first pass the same way.
            .opacity(f32::from(u8::from(measured().is_some())))
            .shadow(
                Shadow::new()
                    .y(2.0)
                    .blur(HOVER_BLUR)
                    .color(palette().panel_shadow),
            )
            .padding(HOVER_PAD)
            .on_pointer_over(move |_| {
                let mut waiting = hover.peek().clone();
                if waiting.over_box(true) {
                    hover.set(waiting);
                }
            })
            .on_pointer_out(move |_| {
                let mut waiting = hover.peek().clone();
                if waiting.over_box(false) {
                    hover.set(waiting);
                }
            })
            .child(
                // An answer taller than the box scrolls inside it, which is what the
                // pointer being able to reach the box is for. So the wheel has to reach
                // this, and nothing here is `Interactive::No`: with the markdown's code
                // blocks drawn as plain text there is no focusable widget inside to steal
                // a press or a drag.
                // **Its height is its content's, capped**, and not the `fill` a scroll
                // view is by default: the box is as tall as what it holds, and a `fill`
                // inside a box sized from what it holds is the two asking each other how
                // tall they are -- which hangs, rather than settling on anything.
                ScrollView::new()
                    // As tall as the answer, up to the room there is: below that it is
                    // the box's own height, above it the answer scrolls inside it. The
                    // first pass is drawn at the full height, which is what measures the
                    // answer, and the second settles.
                    .height(Size::px(
                        measured().map_or(place.room, |height| height.min(place.room)),
                    ))
                    .child(
                        // The server writes its answer in markdown and this draws it as
                        // markdown: the path and the signature as code, the doc comment with
                        // its headings, lists, emphasis and rules. Its colours are the
                        // palette's, through the app's own theme (`interface_theme`), the
                        // crate offering no theme per element.
                        rect()
                            .width(Size::fill())
                            // The answer's own height, which is what it is laid out at
                            // inside a scroll view whatever the view's own height is.
                            .on_sized(move |e: Event<SizedEventData>| {
                                measured.set_if_modified(Some(e.area.height()));
                            })
                            .child(
                                MarkdownViewer::new(said.to_owned())
                                    .code_editor_font_family(fonts().mono.family()),
                            ),
                    ),
            )
            .into_element()
    }
}

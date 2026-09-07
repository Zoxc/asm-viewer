//! The Shortcuts page: every key and every mouse gesture the app answers to, under the
//! place each applies (`src/shortcuts.rs`), with a box that filters them.
//!
//! **A page and not a panel.** The list is read once to learn the app and then rarely, so
//! it belongs where Settings is rather than in a sidebar group a reader gives a column of
//! the window to.
//!
//! The list is a plain `ScrollView` and not a `VirtualScrollView`: it is a few dozen rows
//! written into the binary, the way the Settings and Debug pages are, so there is no
//! `item_size` for a row height to have to agree with and a row is free to be as tall as
//! its text needs.

use super::*;

/// The air between one gesture's row and the next. More than a list's rows carry, because
/// these are read one at a time rather than scanned as a column: the reader is looking for
/// one gesture, not comparing forty.
const ROW_SPACING: f32 = 6.0;

/// The page's body: the filter box, then a heading and its rows for each section the
/// filter left.
#[derive(Clone, PartialEq)]
pub(crate) struct ShortcutsTab;

impl Component for ShortcutsTab {
    fn render(&self) -> impl IntoElement {
        // At the root and not here: only the tab on screen is mounted, so a filter this
        // scope owned would be emptied by a glance at another tab (`Shortcuts`,
        // `src/ui/state.rs`).
        let filter = use_consume::<Shortcuts>().0;
        let current = filter.read().clone();
        // Compiled here and handed down, so the rows are filtered once per render rather
        // than once per section: a `Regex` is not `PartialEq` and cannot live in a state.
        let matcher = current.matcher();
        let listed = shortcuts::matching(&matcher);
        let nothing = listed.is_empty();

        rect()
            .expanded()
            .background(palette().pane_bg)
            .font(&fonts().ui)
            .color(palette().text_fg)
            .child(FilterBox { filter })
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .children(
                            listed
                                .into_iter()
                                .map(|listed| {
                                    rect()
                                        .key(listed.section.place)
                                        .width(Size::fill())
                                        .spacing(ROW_SPACING)
                                        .child(section_heading(listed.section.place, None))
                                        .children(
                                            listed
                                                .gestures
                                                .into_iter()
                                                .map(|gesture| {
                                                    GestureRow { gesture }.into_element()
                                                })
                                                .collect::<Vec<Element>>(),
                                        )
                                        .into_element()
                                })
                                .collect::<Vec<Element>>(),
                        )
                        // A filter that matches nothing has to read as one. An empty page
                        // under a box with text in it looks like a page that failed to
                        // draw, which is the same lie an empty section would tell.
                        .maybe_child(nothing.then(|| {
                            info_line("Nothing answers to that.".to_owned()).into_element()
                        })),
                ),
            )
    }
}

/// One row: what the gesture does on the left, taking the room, and the gesture itself in a
/// column of its own on the right, in the fixed-width font it reads as keys in.
///
/// A component rather than a helper for the tooltip's sake: what a row does can be longer
/// than the room, and [`Fitted`] is a hook.
#[derive(PartialEq)]
struct GestureRow {
    gesture: &'static shortcuts::Gesture,
}

impl Component for GestureRow {
    fn render(&self) -> impl IntoElement {
        let fitted = use_fitted();
        let does = self.gesture.does.to_owned();

        cut_tooltip(
            fitted.cut(),
            does.clone(),
            rect()
                .width(Size::fill())
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .spacing(8.0)
                .child(tree_name_fitted(fitted, does, false, &[]))
                .child(
                    // Clipped, which is `field_row`'s lesson: a label given a width paints
                    // past it, and there is nothing between this column and the text
                    // beside it.
                    rect()
                        .width(Size::px(gesture_width()))
                        .overflow(Overflow::Clip)
                        .child(
                            one_line(self.gesture.keys.to_owned())
                                .width(Size::fill())
                                .font(&fonts().mono)
                                .color(palette().address_fg),
                        ),
                ),
        )
    }
}

/// The box over the list, and the one thing on this page that answers to a key.
///
/// A component of its own so the `Input` is not remounted whenever a row is filtered out
/// under it, which would take the caret with it.
#[derive(PartialEq)]
struct FilterBox {
    filter: State<Filter>,
}

impl Component for FilterBox {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(bottom_hairline())
            .padding(Gaps::new_symmetric(4.0, 8.0))
            .child(
                Input::new(
                    filter
                        .into_writable()
                        .map(|filter| &filter.pattern, |filter| &mut filter.pattern),
                )
                .placeholder("Filter")
                .compact()
                .width(Size::fill())
                // The app's own chords, declined before the edit so they reach the root
                // rather than being typed in as an `f` or a `p`. Without this the page
                // that names Ctrl+P is the one place it does not work (`FilterBar`
                // declines the same three, for the same reason).
                .on_pre_key_down(Callback::new(
                    move |e: Event<KeyboardEventData>| {
                        if is_find_chord(&e.key, e.modifiers)
                            || is_search_chord(&e.key, e.modifiers)
                            || is_finder_chord(&e.key, e.modifiers)
                        {
                            return false;
                        }
                        match &e.key {
                            Key::Named(NamedKey::Enter)
                            | Key::Named(NamedKey::Escape)
                            | Key::Named(NamedKey::Shift) => true,
                            Key::Named(NamedKey::Tab) => false,
                            _ => {
                                e.stop_propagation();
                                e.prevent_default();
                                true
                            }
                        }
                    },
                )),
            )
    }
}

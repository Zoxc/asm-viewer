//! Small pieces of drawing shared by panes that share nothing else.
//!
//! Each holds no state and has no opinion about what it is drawn in: a hairline, a
//! placeholder body, a tooltip wrapper, a heading, a labelled field. They ended up
//! scattered because each was written where it was first wanted and then used again
//! somewhere unrelated -- and leaving one where it began would make that file every other
//! caller's dependency for the sake of half a dozen lines.
//!
//! Nothing here decides anything; a file of parts is all it is.

use super::*;

pub(crate) fn bottom_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.5,
        left: 0.0,
    })
}

pub(crate) fn right_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.0,
        right: 0.5,
        bottom: 0.0,
        left: 0.0,
    })
}

/// The body of a tab that has nothing to show. Takes an owned string as well as a
/// literal, because one of these messages names the file it could not find.
pub(crate) fn placeholder(text: impl Into<String>) -> Element {
    let text: String = text.into();
    rect()
        .expanded()
        .padding(5.0)
        .background(palette().pane_bg)
        .child(label().text(text))
        .into()
}

pub(crate) fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

/// A row's or a chip's own text, shown in full where it could only show part of it.
///
/// Every panel list and both tab strips use this rather than `TooltipContainer` directly,
/// so that the one thing they must agree on -- how long the pointer has to sit still, see
/// [`TOOLTIP_DELAY`] -- is decided once.
pub(crate) fn row_tooltip(text: String, row: impl IntoElement) -> TooltipContainer {
    TooltipContainer::new(Tooltip::new(text))
        .delay(TOOLTIP_DELAY)
        .child(row.into_element())
}

/// The short tag saying what kind of file a row is, in the column every row of the
/// objects tree keeps for it. Grey and small: it labels the row rather than naming it.
pub(crate) fn tag_label(tag: &str) -> impl IntoElement {
    label()
        .text(tag.to_owned())
        .width(Size::px(TAG_WIDTH))
        .font_size(TAG_FONT_SIZE)
        .color(palette().address_fg)
        .max_lines(1)
}

/// What a row is called, taking whatever width the columns beside it left.
///
/// Ellipsised rather than simply cut, which is what the other panel lists do: those cut
/// against the edge of the pane, where the cut is self-evident, while this one cuts
/// against the member count beside it and a name ending flush against a number reads as
/// though it ended there. The `…` is also what says the row's tooltip has more to show.
///
/// The label sits in a box of its own rather than being the `flex` child itself. A
/// `flex` child is measured from its content first, so a label placed there directly
/// takes the width of its whole name and pushes the count off the row.
pub(crate) fn tree_name(text: String, dim: bool) -> impl IntoElement {
    rect()
        .width(Size::flex(1.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(text)
                .width(Size::fill())
                .max_lines(1)
                // Unset rather than `text_fg` when it is not dimmed, so the row goes on
                // inheriting the interface colour from the root the way it always did.
                .maybe(dim, |name| name.color(palette().address_fg))
                .text_overflow(TextOverflow::Ellipsis),
        )
}

/// `text` cut down to [`CHIP_NAME_CHARS`], with an ellipsis where the rest was.
///
/// On a character boundary, so a multi-byte name cannot panic here, and only when there is
/// something to cut: a name that fits keeps its own last character rather than gaining a …
/// for nothing.
pub(crate) fn elide(text: &str) -> String {
    match text.char_indices().nth(CHIP_NAME_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
        None => text.to_owned(),
    }
}

/// What a source file is called in a list: the last component of its path, or the whole
/// of it when there is nothing else to call it.
pub(crate) fn file_name(file: &str) -> String {
    Path::new(file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_owned())
}

/// The heading over one section of the project view, with whatever the section's own
/// action is on the right of it.
///
/// A hairline under it rather than a weight or a colour of its own: the pane is a column
/// of short sections, and a rule is what says where one ends without adding a fifth text
/// size to a window that has four.
pub(crate) fn section_heading(text: &str, action: Option<Element>) -> impl IntoElement {
    rect()
        .width(Size::fill())
        // Padded rather than a fixed row height, unlike every other bar in the app: a
        // section's action is a `Button`, which is taller than a row, and a fixed height
        // would draw the rule through it.
        .padding(Gaps::new_symmetric(2.0, 0.0))
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .border(bottom_hairline())
        .child(
            label()
                .text(text.to_owned())
                .width(Size::flex(1.0))
                .font_weight(FontWeight::BOLD)
                .max_lines(1),
        )
        .maybe_child(action)
}

/// One labelled field: what it is on the left in a fixed column, what it says on the
/// right taking the rest.
///
/// The column is fixed for `SourceRow`'s reason -- the values line up under one another
/// whatever the labels turn out to be -- and it is a `flex` row so that a text box in the
/// value position takes the width that is left rather than the width of its contents.
pub(crate) fn field_row(name: &str, value: impl IntoElement) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(
            label()
                .text(name.to_owned())
                .width(Size::px(FIELD_LABEL_WIDTH))
                .color(palette().address_fg)
                .max_lines(1),
        )
        .child(value)
}

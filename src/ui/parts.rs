//! Small stateless pieces of drawing shared by panes that share nothing else.

use super::*;

pub(crate) fn bottom_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.5,
        left: 0.0,
    })
}

/// Which of a paired row's two edges the run of paired rows ends at: the row above, or
/// below, is not paired. A row alone is both.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct Edges {
    pub(crate) top: bool,
    pub(crate) bottom: bool,
}

impl Edges {
    /// The edges of row `row`, `paired` saying which rows are, asked of the neighbours.
    pub(crate) fn of(row: usize, paired: impl Fn(usize) -> bool) -> Edges {
        Edges {
            top: !row.checked_sub(1).is_some_and(&paired),
            bottom: !row.checked_add(1).is_some_and(&paired),
        }
    }

    pub(crate) fn any(self) -> bool {
        self.top || self.bottom
    }
}

/// The rule a run of paired rows wears along its top and its bottom, on the rows at
/// either end: a line inside the row, so it takes no height from it -- every row of a
/// listing being exactly `code_row_height()` -- and nothing down the sides.
pub(crate) fn pair_border(edges: Edges) -> Border {
    Border::new().fill(palette().pair_edge).width(BorderWidth {
        top: if edges.top { 1.0 } else { 0.0 },
        right: 0.0,
        bottom: if edges.bottom { 1.0 } else { 0.0 },
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

/// The body of a tab that has nothing to show.
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

/// The same in a colour of its own, for a line that is saying how something went.
pub(crate) fn info_line_in(text: String, color: Color) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text).color(color))
}

/// A row's or a chip's own text, shown in full where the row could only show part of it.
/// Used rather than `TooltipContainer` directly so that [`TOOLTIP_DELAY`] is decided once.
pub(crate) fn row_tooltip(text: String, row: impl IntoElement) -> TooltipContainer {
    TooltipContainer::new(Tooltip::new(text))
        .delay(TOOLTIP_DELAY)
        .child(row.into_element())
}

/// The short tag saying what kind of file a row is, in the column every row of the objects
/// tree keeps for it.
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
/// The label sits in a box of its own rather than being the `flex` child itself: a `flex`
/// child is measured from its content first, so a label placed there directly takes the
/// width of its whole name and pushes the count off the row.
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

/// `text` cut down to [`CHIP_NAME_CHARS`], with an ellipsis where the rest was. On a
/// character boundary, so a multi-byte name cannot panic here.
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
pub(crate) fn section_heading(text: &str, action: Option<Element>) -> impl IntoElement {
    rect()
        .width(Size::fill())
        // Padded rather than a fixed row height: a section's action is a `Button`, which
        // is taller than a row, and a fixed height would draw the rule through it.
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

/// One labelled field: what it is on the left in a fixed column, what it says on the right
/// taking the rest. A `flex` row, so a text box in the value position takes the width that
/// is left rather than the width of its contents.
///
/// **The name is clipped inside its column**, and that is not tidiness. A `label` given a
/// width draws its text at that width and paints past it regardless; there is nothing
/// between this column and the value beside it, so a name too long for the column drew
/// over a control the reader was meant to press. The column follows the font
/// ([`field_label_width`]) so that a reader who enlarges the interface font is not the one
/// who finds this out.
pub(crate) fn field_row(name: &str, value: impl IntoElement) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(
            rect()
                .width(Size::px(field_label_width()))
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .text(name.to_owned())
                        .width(Size::fill())
                        .color(palette().address_fg)
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis),
                ),
        )
        .child(value)
}

/// A block of a tool's own output, laid out the way it wrote it: one label per line, in
/// the fixed-width font, so rustc's carets sit under what they point at. A line too wide
/// for the pane **wraps** rather than being cut off at its right edge.
///
/// Wrapping does move a caret out from under the character it points at, which is why this
/// block used to cut instead. What settles it is which line pays: a line that fits is
/// untouched, so every block narrower than the pane is drawn exactly as it was, and the
/// only line that wraps is the one clipping would have thrown the end of away entirely.
/// `--> src/main.rs:9:17` is that line -- the half of a diagnostic that says *where* --
/// and a caret under the wrong column is a worse drawing of something the reader can
/// still read, where a cut is the answer not being there at all.
pub(crate) fn text_block(text: &str, color: Color) -> Element {
    rect()
        .width(Size::fill())
        .children(
            text.lines()
                .map(|line| {
                    label()
                        .text(line.to_owned())
                        .assembly_font()
                        .color(color)
                        .into()
                })
                .collect::<Vec<Element>>(),
        )
        .into_element()
}

/// One thing the compiler said: a line that can be scanned, and cargo's own rendering of
/// it under that. The header adds the **place**, taken from the span rather than from the
/// text.
///
/// `place` is drawn by whoever calls this, because what a place can be *pressed* to reach
/// differs between the two panes that draw diagnostics: the scratchpad's puts its editor's
/// cursor on the line, the project's opens the file. Both agree that a place they cannot
/// reach is a plain label — [`diagnostic_place`] is that label, and a target that did
/// nothing when pressed would be the worse of the two answers.
pub(crate) fn diagnostic_block(diagnostic: &Diagnostic, place: Option<Element>) -> Element {
    rect()
        .width(Size::fill())
        .padding(Gaps::new(2.0, 0.0, 6.0, 0.0))
        .child(
            rect()
                .width(Size::fill())
                // Tall enough for what is in it and never shorter than an ordinary row:
                // the message wraps, so the header is one row for almost every diagnostic
                // and as many as the sentence needs for the one that does not fit.
                .height(Size::auto())
                .min_height(Size::px(list_row_height()))
                .horizontal()
                // Start and not `Center`: what a wrapped message stands beside is the word
                // `error` and the place, which belong against its first line.
                .cross_align(Alignment::Start)
                .spacing(6.0)
                .content(Content::Flex)
                .child(
                    label()
                        .text(match diagnostic.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            Level::Note => "note",
                        })
                        // An error is the red every invalid thing wears, a warning the one
                        // warm hue in the palette, and a note recedes.
                        .color(match diagnostic.level {
                            Level::Error => palette().invalid_fg,
                            Level::Warning => palette().string_fg,
                            Level::Note => palette().address_fg,
                        })
                        .max_lines(1),
                )
                .maybe_child(place)
                // The sentence rustc wrote, wrapping rather than cut at the pane's edge.
                .child(
                    label()
                        .text(diagnostic.message.clone())
                        .width(Size::flex(1.0)),
                ),
        )
        .child(text_block(&diagnostic.rendered, palette().text_fg))
        .into_element()
}

/// How a diagnostic's place is spelled: the file, the line and the column. A registry path
/// is most of a line on its own and which crate it is in is the useful half, so a file
/// outside the directory being built is cut down to its name.
pub(crate) fn diagnostic_place(span: &cargo::Span, whole: bool) -> String {
    let file = match whole {
        true => span.file.clone(),
        false => file_name(&span.file),
    };
    format!("{file}:{}:{}", span.line, span.column)
}

/// The item that shows a file, or a folder, where the rest of the reader's tools are: on
/// a document's tab, and on a Files row. One item everywhere, since the path is all it is
/// about; which of the two it names is worked out where the call is made.
///
/// The call is a subprocess and is made on a thread of its own (`crate::reveal`), so
/// there is no task here for the press that closes the menu to drop.
pub(crate) fn reveal_item(path: PathBuf) -> MenuButton {
    MenuButton::new()
        .on_press(move |_| reveal::reveal(path.clone()))
        .child("Show in file manager")
}

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

/// **What a lit link looks like, in one place.** The wash and the rounded corner every
/// link in the app shares, and an `underline` -- a rule along the bottom in the lit
/// colour -- where one is asked for. Every link in a code row wears both: an operand of
/// an instruction, a name in the source, a label in the object's listing, each of which
/// has nothing but this and its colour to say it can be pressed. A place a diagnostic
/// names takes the wash alone, being a line of its own rather than a run inside one.
pub(crate) fn link_chrome(rect: Rect, underline: Option<Color>) -> Rect {
    let rect = rect.background(palette().link_hover_bg).corner_radius(6.0);
    match underline {
        Some(colour) => rect.border(Border::new().fill(colour).width(BorderWidth {
            top: 0.0,
            right: 0.0,
            bottom: 2.0,
            left: 0.0,
        })),
        None => rect,
    }
}

/// How far inside a code row's top and bottom edge that box is drawn, where the link is a
/// run of the row's own text and the row draws the box for it: a row is its font plus
/// twelve of leading (`code_row_height`), so this keeps the wash around the text instead
/// of around the row, and the rule under it about where an underline would be.
pub(crate) const LINK_BOX_INSET: f32 = 4.0;

/// The rule over a bar drawn under what it belongs to, as [`bottom_hairline`] is the rule
/// under one drawn over it.
pub(crate) fn top_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.5,
        right: 0.0,
        bottom: 0.0,
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

/// [`placeholder`]'s box with no message in it: a pane with nothing to show and nothing
/// to say about it -- `Showing::Nothing`, or a prop naming a tab the table no longer
/// holds. The colour is the caller's, the panes having two: `pane_bg` on the source side,
/// `asm_pane_bg` on the assembly side and in the split around them.
pub(crate) fn blank_pane(background: Color) -> Element {
    rect().expanded().background(background).into()
}

pub(crate) fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

/// The colour a line saying how something went is drawn in: the red every invalid thing
/// wears when it is bad news, and the receding grey when it is not. For a line laid out by
/// the pane around it; one on its own is [`verdict_line`].
pub(crate) fn verdict_fg(bad: bool) -> Color {
    match bad {
        true => palette().invalid_fg,
        false => palette().address_fg,
    }
}

/// One line saying how something went, in that colour: what the build panes, the language
/// server's section and the scratchpad all say their verdicts with, so a pane cannot
/// diverge from another in padding, in clipping or in what the colour means.
///
/// Clipped to one line. What is said here is a sentence, and a build that could not start
/// carries an error of any length behind it; a pane whose height jumps by four lines when
/// cargo is missing is worse than a line the reader has to widen the pane to read.
pub(crate) fn verdict_line(verdict: Verdict) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .padding(Gaps::new_symmetric(2.0, 6.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(verdict.text)
                .color(verdict_fg(verdict.bad))
                .max_lines(1),
        )
}

/// The frame every sidebar-style row is drawn in: the height a list's rows are, the
/// padding and the spacing their columns are laid on, and the three-way background --
/// picked out, under the pointer, or nothing. The caller appends its own press, its menu
/// and its children, and hands the result to [`row_tooltip`].
///
/// A row that is picked out wears the selection, `text_select_bg`, which is what a sweep
/// paints under the characters it took in a code pane: being picked out says the same
/// thing in a list as in the code. That is while the keyboard is in the list; a list it
/// is not in draws its pick in the neutral grey instead, so the blue is always what the
/// next key would act on (`ui/picks.rs`). Both beat the hover, which is why the match
/// below is on the selection first.
///
/// **The hover state stays the caller's.** There is no `.hover()` pseudo-state, so a row
/// that lights under the pointer holds a `use_state` of its own, and a hook may only run
/// while a component renders, which this is not. Reading it here is what subscribes the
/// row being rendered to it, exactly as asking for a colour is.
///
/// The height is [`list_row_height`] and nothing else: a row and the `VirtualScrollView`
/// over it must agree about `item_size`, or scrolling misaligns. [`dead_list_row`] is the
/// same frame with nothing to answer the pointer with.
pub(crate) fn list_row(mut hovering: State<bool>, chosen: Chosen) -> Rect {
    let background = match chosen {
        Chosen::Live => palette().text_select_bg,
        Chosen::Idle => palette().selected_bg,
        Chosen::No if hovering() => palette().row_hover_bg,
        Chosen::No => Color::TRANSPARENT,
    };
    row_frame(background)
        .on_pointer_over(move |_| hovering.set_if_modified(true))
        .on_pointer_out(move |_| hovering.set_if_modified(false))
}

/// A list row that answers the pointer with nothing: a bookmark whose place does not
/// resolve, which is drawn dimmed and goes nowhere, so it has no hover to light.
pub(crate) fn dead_list_row() -> Rect {
    row_frame(Color::TRANSPARENT)
}

/// What the two share: the frame, and the one padding and the one spacing every list row
/// lays its columns out on.
fn row_frame(background: Color) -> Rect {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        // A row's name is the `flex` child taking what the fixed columns leave, which
        // torin only works out under `Content::Flex`.
        .content(Content::Flex)
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .padding(Gaps::new_symmetric(0.0, 5.0))
        .spacing(5.0)
        .background(background)
        .overflow(Overflow::Clip)
}

/// The rest of a text the row had only room for part of, **mounted only where the text
/// was cut**: a tooltip repeating a name already whole on screen is noise the pointer
/// drags down a list.
pub(crate) fn cut_tooltip(cut: bool, text: String, row: impl IntoElement) -> Element {
    let row = row.into_element();
    match cut {
        false => row,
        true => TooltipContainer::new(Tooltip::new(text))
            .delay(CUT_TOOLTIP_DELAY)
            .child(row)
            .into_element(),
    }
}

/// A tooltip saying what the thing under it does not: a file's path where its name is
/// drawn, where a matched line is, what a button does. Shown whatever fitted, since what
/// it says is not on screen either way -- and it **waits**, freya's own half second,
/// being a second thought rather than the rest of what is being read.
pub(crate) fn extra_tooltip(text: String, row: impl IntoElement) -> Element {
    TooltipContainer::new(Tooltip::new(text))
        .child(row.into_element())
        .into_element()
}

/// A row drawn by `text` where the whole of what it names is `whole`: the rest of a cut
/// text where the two are the same, and something more where they differ. The History and
/// Bookmarks rows and the tab chips are one row for a symbol and the other for a file.
pub(crate) fn name_tooltip(cut: bool, text: &str, whole: String, row: impl IntoElement) -> Element {
    match whole == text {
        true => cut_tooltip(cut, whole, row),
        false => extra_tooltip(whole, row),
    }
}

/// The disclosure triangle a row that folds draws, in the [`chevron_width`] column every
/// row of its list keeps: the Lucide chevron, pointing down where the row is open and
/// right where it is shut. `None` is that column with nothing in it, which is what a row
/// that cannot fold draws.
///
/// An icon and not the `\u{25b8}`/`\u{25be}` characters it was, so the shape is the app's
/// rather than whatever interface font the desktop names: it is sized against the row and
/// not against the text, it is centred in the row instead of sitting on its baseline, and
/// it is Lucide, as every other small mark here already is.
///
/// **Whether it is open is said in accessibility's own `expanded` and nowhere else.** An
/// `SvgViewer` rasterises to an image, so which chevron it drew is not in the element tree
/// at all; the flag is what a screen reader is told and what `disclosures`
/// (`src/ui/tests.rs`) finds the triangles by.
pub(crate) fn disclosure(open: Option<bool>) -> Element {
    let side = chevron_size();

    rect()
        .width(Size::px(chevron_width()))
        .center()
        .map(open, |column, open| {
            column
                .a11y_builder(move |node| node.set_expanded(open))
                .child(
                    SvgViewer::new(match open {
                        true => ("chevron-down", lucide::chevron_down()),
                        false => ("chevron-right", lucide::chevron_right()),
                    })
                    .width(Size::px(side))
                    .height(Size::px(side))
                    .color(palette().icon_fg)
                    .show_loader(false),
                )
        })
        .into_element()
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

/// Whether the one line of text a row draws fitted the room it was given.
///
/// **freya cannot be asked this of a `label`.** It reports the box a text was laid out in
/// and never the width the text wanted, and a label asked for an ellipsis is laid out at
/// the width of its own box -- so what it measures is the ellipsised line, which is the
/// box again. A `paragraph` hands back the paragraph skia drew (`ParagraphHolder`), and
/// that answers it outright: `did_exceed_max_lines` is false for a line that fitted and
/// true for one that was cut, off the paragraph freya built anyway.
///
/// The answer lands in a state the row reads as it renders, since what it decides -- a
/// tooltip mounted or not -- is a render's decision. torin emits `Sized` every time it
/// measures the node and not only when the box changed, so a row handed new text at the
/// width it already had answers again.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Fitted {
    holder: State<ParagraphHolder>,
    cut: State<bool>,
}

/// A hook, so it is called in the row's own render and handed down from there.
pub(crate) fn use_fitted() -> Fitted {
    Fitted {
        holder: use_state(ParagraphHolder::default),
        cut: use_state(|| false),
    }
}

impl Fitted {
    /// Whether the text was cut. Reading it is what subscribes the row to it.
    pub(crate) fn cut(self) -> bool {
        (self.cut)()
    }

    /// The line freya draws, told to answer.
    pub(crate) fn measuring(self, line: Paragraph) -> Paragraph {
        let Fitted { holder, mut cut } = self;
        line.holder(holder.read().clone())
            .on_sized(move |_: Event<SizedEventData>| {
                // Bound to a `let` of its own before the write: the borrow of the holder
                // ends with the statement, and a read held across a `set` panics.
                let answer = holder
                    .peek()
                    .0
                    .borrow()
                    .as_ref()
                    .map(|laid| laid.paragraph.did_exceed_max_lines());
                if let Some(answer) = answer {
                    cut.set_if_modified(answer);
                }
            })
    }
}

/// One line of text, cut with an ellipsis where the room ran out.
pub(crate) fn one_line(text: String) -> Paragraph {
    paragraph()
        .max_lines(1)
        .text_overflow(TextOverflow::Ellipsis)
        .span(Span::new(text))
}

/// What a search or a filter matched in `text`, as the pairs a paragraph highlights by:
/// **UTF-16 units**, which is what skia indexes a paragraph in and what a column is
/// counted in everywhere else that meets it (`src/chars.rs`). Byte ranges in, since
/// everything outside the text engine is bytes.
///
/// A mark that is not on a character boundary, or runs off the end, is dropped rather than
/// panicking: these come from a regex over the same string, but a row draws a *cut* line
/// where a hit was found in the whole one.
pub(crate) fn marked_units(text: &str, marks: &[Range<usize>]) -> Vec<(usize, usize)> {
    marks
        .iter()
        .filter(|mark| {
            mark.end <= text.len()
                && text.is_char_boundary(mark.start)
                && text.is_char_boundary(mark.end)
        })
        .map(|mark| {
            (
                chars::units(&text[..mark.start]),
                chars::units(&text[..mark.end]),
            )
        })
        .collect()
}

/// One line of a file as a hit or a reference row draws it: cut with an ellipsis where the
/// room runs out, and washed where a search found something.
///
/// **No span at all where there is no text**, which is the row of a file that would not
/// read: an empty span is a piece of the row all the same, and a row with one in it is a
/// row that says it has text.
pub(crate) fn found_line(text: &str, marks: &[Range<usize>]) -> Paragraph {
    let line = paragraph()
        .width(Size::fill())
        .max_lines(1)
        .text_overflow(TextOverflow::Ellipsis);
    let line = match text.is_empty() {
        true => line,
        false => line.span(Span::new(text.to_owned())),
    };
    marked(line, text, marks)
}

/// A line with what matched in it marked: the wash behind those runs, and nothing else
/// about the text changed. The paragraph's own highlight, freya giving a span no
/// background of its own.
pub(crate) fn marked(line: Paragraph, text: &str, marks: &[Range<usize>]) -> Paragraph {
    match marks.is_empty() {
        true => line,
        false => line
            .highlights(marked_units(text, marks))
            .highlight_color(palette().match_bg),
    }
}

/// What a row is called, taking whatever width the columns beside it left.
///
/// The text sits in a box of its own rather than being the `flex` child itself: a `flex`
/// child is measured from its content first, so a line placed there directly takes the
/// width of its whole name and pushes the count off the row.
pub(crate) fn tree_name(text: String, dim: bool, marks: &[Range<usize>]) -> impl IntoElement {
    let line = marked(one_line(text.clone()), &text, marks);
    name_box(line, dim)
}

/// The same line, measured: `fitted` is told whether it was cut.
pub(crate) fn one_line_fitted(fitted: Fitted, text: String) -> Paragraph {
    fitted.measuring(one_line(text))
}

/// The same, measured, for a row whose tooltip is only shown where the name was cut.
pub(crate) fn tree_name_fitted(
    fitted: Fitted,
    text: String,
    dim: bool,
    marks: &[Range<usize>],
) -> impl IntoElement {
    let line = marked(one_line_fitted(fitted, text.clone()), &text, marks);
    name_box(line, dim)
}

fn name_box(line: Paragraph, dim: bool) -> impl IntoElement {
    rect()
        .width(Size::flex(1.0))
        .overflow(Overflow::Clip)
        .child(
            line.width(Size::fill())
                // Unset rather than `text_fg` when it is not dimmed, so the row goes on
                // inheriting the interface colour from the root the way it always did.
                .maybe(dim, |name| name.color(palette().address_fg)),
        )
}

/// Whether [`elide`] would cut `text`: what a chip asks instead of measuring, its text
/// being cut by the count and never by the room.
pub(crate) fn elided(text: &str) -> bool {
    text.chars().count() > CHIP_NAME_CHARS
}

/// `text` cut down to [`CHIP_NAME_CHARS`], with an ellipsis where the rest was. On a
/// character boundary, so a multi-byte name cannot panic here.
pub(crate) fn elide(text: &str) -> String {
    match text.char_indices().nth(CHIP_NAME_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
        None => text.to_owned(),
    }
}

/// What a text box says, or `None` when it says nothing.
pub(crate) fn given(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
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
            one_line(text.to_owned())
                .width(Size::flex(1.0))
                .font_weight(FontWeight::BOLD),
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
    field_row_in(name, palette().address_fg, value)
}

/// The same row with the name's colour handed in: what a field whose name says something
/// by its colour is built on, the settings page's being dim while the value is inherited.
pub(crate) fn field_row_in(name: &str, colour: Color, value: impl IntoElement) -> impl IntoElement {
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
                .child(one_line(name.to_owned()).width(Size::fill()).color(colour)),
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
        false => source::name_of(Path::new(&span.file)),
    };
    format!("{file}:{}:{}", span.line, span.column)
}

/// The dot a code row is marked with, at its left edge: a source line that produced code,
/// an instruction the debug info places on a source line.
///
/// The column is given up by every row, marked or not, so the numbers and addresses beside
/// it sit in the same place either way. The dot is rounded to whole device pixels like the
/// arrow gutter's strokes, one half a pixel across being a smear.
pub(crate) fn code_mark(marked: bool) -> Element {
    let dot = pixel_grid().span(0.0, MARK_SIZE).thick;
    rect()
        .width(Size::px(MARK_COLUMN))
        .height(Size::px(code_row_height()))
        .center()
        .child(
            rect()
                .width(Size::px(dot))
                .height(Size::px(dot))
                .corner_radius(dot / 2.0)
                .maybe(marked, |el| el.background(palette().compiled_fg)),
        )
        .into_element()
}

//! Small stateless pieces of drawing shared by panes that share nothing else, and the
//! macro every keyed row is written with.

use super::*;

/// Two of the three parts a keyed component needs, written once: the `KeyExt` impl over
/// its `key` field, so `.key(..)` has somewhere to go, and the `keyed` its `render_key`
/// answers with.
///
/// **The third is the row's own**, one line inside its `impl Component`:
///
/// ```ignore
/// fn render_key(&self) -> DiffKey {
///     self.keyed()
/// }
/// ```
///
/// Only `render_key` is read. Without it a row takes the `.key(..)` call, stores it, and
/// is diffed by position all the same, so each row's hover and state stay with the slot
/// rather than with what was drawn in it (`agents/UI.md`). Nothing in the types said the
/// three go together; now nothing calls the `keyed` this writes, and the `deny` makes that
/// a compile error naming the row.
///
/// A row with generic parameters names them first, bounds and all:
/// `keyed!([T: Place] PlaceRow<T>);`.
macro_rules! keyed {
    ($row:ident) => {
        keyed!([] $row);
    };
    ([$($generic:tt)*] $row:ty) => {
        impl<$($generic)*> KeyExt for $row {
            fn write_key(&mut self) -> &mut DiffKey {
                &mut self.key
            }
        }

        #[deny(dead_code)]
        impl<$($generic)*> $row {
            /// The key this was built with, or the type's own where the call site gave
            /// none.
            fn keyed(&self) -> DiffKey {
                self.key.clone().or(self.default_key())
            }
        }
    };
}

pub(crate) use keyed;

/// The rule between two surfaces, drawn by whichever of them owns the edge: the palette's
/// hairline along the one side the caller names, and nothing along the other three.
fn hairline(side: BorderWidth) -> Border {
    Border::new().fill(palette().hairline).width(side)
}

/// The rule under a bar drawn over what it belongs to.
pub(crate) fn bottom_hairline() -> Border {
    hairline(BorderWidth {
        bottom: 0.5,
        ..BorderWidth::default()
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

/// The rule over a bar drawn under what it belongs to, as [`bottom_hairline`] is the rule
/// under one drawn over it.
pub(crate) fn top_hairline() -> Border {
    hairline(BorderWidth {
        top: 0.5,
        ..BorderWidth::default()
    })
}

pub(crate) fn right_hairline() -> Border {
    hairline(BorderWidth {
        right: 0.5,
        ..BorderWidth::default()
    })
}

/// The body of a tab that has nothing to show.
pub(crate) fn placeholder(text: impl Into<String>) -> Element {
    placeholder_on(palette().pane_bg, text)
}

/// [`placeholder`] on the ground the caller says, as [`blank_pane`] is. The assembly
/// side has one of its own (`asm_pane_bg`), and a message drawn on `pane_bg` there is
/// the pane changing colour under the reader every time a line resolves to nothing or
/// takes long enough to say so.
pub(crate) fn placeholder_on(background: Color, text: impl Into<String>) -> Element {
    let text: String = text.into();
    rect()
        .expanded()
        .padding(5.0)
        .background(background)
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

/// The inset a code listing is drawn with, filling the box its pane gives it.
///
/// The inset is the listing's own and not the pane's, so the bar above runs the pane's
/// full width the way a header does. All four listings take it -- the instruction list,
/// the source list, an object's code on a tab and the same in the Scratchpad -- and the
/// four have to agree: a page of rows, a reveal and a sweep are each measured in the
/// height inside it.
pub(crate) fn listing_inset(listing: impl IntoElement) -> Rect {
    rect().expanded().padding(5.0).child(listing)
}

/// One row over a code listing saying what the reader is looking at is out of date, in
/// the header's own colours: a notice about the listing, drawn where the listing is
/// named. The Source pane draws [`STALE_SOURCE`] over a file whose bytes are not the ones
/// the binary was built from; the Scratchpad draws [`STALE_PROGRAM`] over a program the
/// reader has edited since it was built.
pub(crate) fn stale_banner(text: &'static str) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(palette().header_bg)
        .child(label().text(text).color(palette().text_fg))
        .into()
}

pub(crate) fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

/// `rows` in a column, or a line saying `empty` where there are none.
///
/// One shape for every section that draws a list which may be empty, so the empty state
/// cannot come to differ from one section to the next.
pub(crate) fn rows_or(rows: Vec<Element>, empty: &str) -> Element {
    match rows.is_empty() {
        true => info_line(empty.to_owned()).into_element(),
        false => rect().width(Size::fill()).children(rows).into_element(),
    }
}

/// One line of secondary text: a count beside a row, a path beside a name, a state
/// written in words.
///
/// **A role with a name, rather than the colour spelled out.** [`Palette`] documents
/// `address_fg` as *where a thing is* -- the instruction addresses and the source
/// line-number gutter -- and a dozen labels borrow it to mean dim. Whether secondary
/// text stays that colour is a decision the palette cannot make while the role has no
/// name; with one it is this line.
///
/// freya's builder and not an element, so a caller can still say how wide the line is or
/// how it is aligned.
pub(crate) fn dim_line(text: impl Into<String>) -> Label {
    label()
        .text(text.into())
        .color(palette().address_fg)
        .max_lines(1)
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

/// What a bar with a pattern box says under it when the pattern will not compile: the
/// regex's own complaint, in the colour every invalid thing wears and clipped to one line.
/// Drawn by both filter bars and by the find bar.
///
/// **A pattern that will not compile has to read as one**: matching nothing looks exactly
/// like a list with nothing in it, so the reason is written out.
///
/// The line sits in a `rect` because a `label` carries no padding and no clipping of its
/// own.
pub(crate) fn invalid_line(error: String) -> Element {
    rect()
        .width(Size::fill())
        .padding(Gaps::new(0.0, 6.0, 5.0, 6.0))
        .overflow(Overflow::Clip)
        .child(label().text(error).color(palette().invalid_fg).max_lines(1))
        .into_element()
}

/// The frame every sidebar-style row is drawn in: the height a list's rows are, the
/// padding and the spacing their columns are laid on, and the three-way background --
/// picked out, under the pointer, or nothing. The caller appends its own press, its menu
/// and its children, and hands the result to one of the tooltips -- [`cut_tooltip`],
/// [`extra_tooltip`] or [`name_tooltip`].
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

/// A list row that answers the pointer with nothing: the bookmark whose place does not
/// resolve, drawn dimmed and going nowhere, and the Project view's binary and override
/// rows, which state what is there. Nothing to press, so no hover to light.
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

/// What a bar button wears with the pointer somewhere else.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glow {
    /// Nothing: the bar's own ground, until the pointer arrives.
    No,
    /// The hover wash, held while a menu the button opened is up, so a button whose menu
    /// is on screen stays picked out.
    Open,
    /// A toggle that is on, which is a step darker than the hover and beats it.
    On,
}

/// The frame of a small button in a bar: a [`toggle_size`] square cut to
/// [`BAR_BUTTON_RADIUS`], its glyph centred, lit under the pointer. The caller adds the
/// press, the child and the tooltip.
///
/// `live` is whether pressing it would do anything. A dead button takes neither the
/// pointer handlers nor the wash, which is the whole of how a history chevron with nowhere
/// to go is drawn disabled; the caller dims its glyph.
///
/// **The hover state stays the caller's**, [`list_row`]'s reason: there is no `.hover()`
/// pseudo-state, so a button that lights holds a `use_state` of its own, and a hook may
/// only run while a component renders, which this is not. Reading it here is what
/// subscribes the button being rendered to it.
///
/// A `Rect` and not an `Element`, so a caller with a box of its own says so on the frame
/// itself: the tab list's button is as wide as the chips' close column and as tall as the
/// bar it is pinned to the end of, being a control in the tab strip rather than a square
/// dropped in one.
pub(crate) fn bar_button(hovering: State<bool>, live: bool, glow: Glow) -> Rect {
    bar_control(hovering, live, glow)
        .width(Size::px(toggle_size()))
        .height(Size::px(toggle_size()))
        .center()
}

/// The same button round a **word** rather than a glyph: as tall as the square and as wide
/// as what it holds, with [`BAR_PILL_PAD`] at each end. The project's name in the top bar
/// and the language server's control.
pub(crate) fn bar_pill(hovering: State<bool>, live: bool, glow: Glow) -> Rect {
    bar_control(hovering, live, glow)
        .height(Size::px(toggle_size()))
        .center()
        .padding(Gaps::new_symmetric(0.0, BAR_PILL_PAD))
}

/// What the two share: the corner, the wash, and the two pointer handlers -- attached only
/// where a press would do something, so a dead button does not light.
fn bar_control(mut hovering: State<bool>, live: bool, glow: Glow) -> Rect {
    let background = match glow {
        Glow::On => palette().toggle_on_bg,
        Glow::Open => palette().toggle_hover_bg,
        Glow::No if live && hovering() => palette().toggle_hover_bg,
        Glow::No => Color::TRANSPARENT,
    };
    rect()
        .corner_radius(BAR_BUTTON_RADIUS)
        .background(background)
        .maybe(live, |button| {
            button
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
        })
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
/// **Whether it is open is said in accessibility's own `expanded` and nowhere else.** A
/// glyph is drawn from a raster, so which chevron it drew is not in the element tree
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
                .child(glyph_sized(
                    match open {
                        true => ("chevron-down", lucide::chevron_down()),
                        false => ("chevron-right", lucide::chevron_right()),
                    },
                    side,
                    palette().icon_fg,
                ))
        })
        .into_element()
}

/// **The one small glyph, in one place.** A Lucide icon at [`icon_size`] in the palette's
/// `icon_fg`: a tab bar's button, a page's icon, a panel's header, a document's row, a
/// file in the tree.
///
/// `icon` is written `("name", lucide::name())` everywhere, the name beside the bytes,
/// because the name is what the raster cache is keyed by (`src/ui/glyph.rs`).
pub(crate) fn glyph(icon: (&'static str, Bytes)) -> Element {
    glyph_in(icon, palette().icon_fg)
}

/// The same glyph in a colour of the caller's own: the language server's button, which
/// says by the icon's colour which of its states it is in.
pub(crate) fn glyph_in(icon: (&'static str, Bytes), colour: Color) -> Element {
    glyph_sized(icon, icon_size(), colour)
}

/// An icon at a side of the caller's own: every icon in the app is one of these, drawn on
/// whole device pixels (`src/ui/glyph.rs`).
pub(crate) fn glyph_sized(icon: (&'static str, Bytes), side: f32, colour: Color) -> Element {
    Glyph { icon, side, colour }.into_element()
}

/// The short tag saying what kind of file a row is, in the column every row of the objects
/// tree keeps for it.
pub(crate) fn tag_label(tag: &str) -> impl IntoElement {
    dim_line(tag.to_owned())
        .width(Size::px(tag_width()))
        .font_size(tag_font_size())
}

/// The column a folding row counts what is under it in: the digits at [`tag_font_size`],
/// a [`COUNT_GUTTER`] before them, and nothing at all for a row with nothing to count --
/// a file that has produced no objects yet. Every row of a list that has such a column
/// keeps it, so the names line up down the list.
///
/// A column of its own and not a label at the end of the row: the count is measured whole
/// before the name is handed what the columns leave, so a sidebar dragged narrow
/// ellipsises the name and never eats the digits (`agents/Sidebar.md`).
pub(crate) fn count_column(count: Option<usize>) -> Element {
    rect()
        .padding(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))
        .map(count, |column, count| {
            column.child(dim_line(count.to_string()).font_size(tag_font_size()))
        })
        .into_element()
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
/// **UTF-16 units**, which is what skia indexes a paragraph in. Byte ranges in, since
/// everything outside the text engine is bytes (`src/chars.rs`).
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
            let units = chars::utf16_range(text, mark.clone());
            (units.start, units.end)
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
/// being cut by the count and never by the room. The walk stops at the cut rather than
/// counting the whole name (`chars::byte_of_char`).
pub(crate) fn elided(text: &str) -> bool {
    chars::byte_of_char(text, CHIP_NAME_CHARS) < text.len()
}

/// `text` cut down to [`CHIP_NAME_CHARS`], with an ellipsis where the rest was. On a
/// character boundary, so a multi-byte name cannot panic here.
pub(crate) fn elide(text: &str) -> String {
    let end = chars::byte_of_char(text, CHIP_NAME_CHARS);
    match end < text.len() {
        true => format!("{}\u{2026}", &text[..end]),
        false => text.to_owned(),
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
        // Padded rather than a fixed row height: a section's action is a button, which is
        // taller than a row, and a fixed height would draw the rule through it.
        .padding(Gaps::new_symmetric(4.0, 0.0))
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

/// A heading's action: a glyph and a word in the bar's own [`bar_pill`], flat until the
/// pointer is on it, rather than a raised `Button` that outweighs the heading it stands in.
/// The glyph is what says "a button" where the pill has no wash.
///
/// `live` is whether a press would do anything; a dead one is dimmed and does not light.
/// The press is handed in, so this re-renders whenever the section does, [`PlaceTarget`]'s
/// bargain.
#[derive(Clone, PartialEq)]
pub(crate) struct HeadingButton {
    /// The glyph, as [`glyph`] takes one, drawn in the word's colour so it dims with it.
    pub(crate) icon: (&'static str, Bytes),
    pub(crate) text: &'static str,
    pub(crate) live: bool,
    pub(crate) press: EventHandler<Event<PressEventData>>,
}

impl Component for HeadingButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let press = self.press.clone();
        let colour = match self.live {
            true => palette().text_fg,
            false => dimmed(palette().text_fg, palette().pane_bg),
        };

        bar_pill(hovering, self.live, Glow::No)
            .maybe(self.live, |button| {
                button.on_press(move |e: Event<PressEventData>| press.call(e))
            })
            .horizontal()
            .spacing(4.0)
            .child(glyph_in(self.icon.clone(), colour))
            .child(label().text(self.text).max_lines(1).color(colour))
    }
}

/// One section of a page: its heading with the section's own action on the right, and
/// room under it for the rows the caller adds, [`SECTION_GAP`] apart.
pub(crate) fn section(title: &str, action: Option<Element>) -> Rect {
    rect()
        .width(Size::fill())
        .spacing(SECTION_GAP)
        .child(section_heading(title, action))
}

/// The column a page's sections stand in: as wide as the page, with the page's own margins
/// round it and [`SECTION_SPACE`] between one section and the next.
///
/// Apart from [`page`] because not every page is one scroll: the Shortcuts page keeps its
/// filter box above the scroll, and the Scratchpad's column stands beside a split and is
/// not scrolled at all.
pub(crate) fn page_column() -> Rect {
    rect()
        .width(Size::fill())
        .padding(PAGE_PAD)
        .spacing(SECTION_SPACE)
}

/// A page: the pane's ground, whatever stands above the scroll, and the scroll the body is
/// in. Nothing to do with `page_row` (`src/ui/pages_menu.rs`), which is the table saying what a
/// [`Page`] is; this is how any page's body is drawn, the Scratchpad's pane included.
///
/// Nothing sets the font or the text colour here. The root sets both once and every page
/// inherits them (`app`, `src/ui.rs`); two of the five used to set them again, which is
/// how the copies came to differ.
pub(crate) fn page(bar: Option<Element>, body: Rect) -> Rect {
    rect()
        .expanded()
        .background(palette().pane_bg)
        .maybe_child(bar)
        .child(ScrollView::new().child(body))
}

/// A window that says something and offers a button or two: its width, and the way out of
/// it that Escape and a press outside take. The caller adds the body and the buttons.
///
/// Over `Popup` itself and not its `PopupTitle`/`PopupContent`: both of those set a font
/// size of their own, which would draw the window in a size the reader never chose. What
/// `Popup` is wanted for is the overlay layer, the dimmed background, the press outside and
/// the Escape key -- and that it shows exactly when it has children, so a caller with
/// nothing to ask adds none and the window is not there.
pub(crate) fn notice(on_close: impl Into<EventHandler<()>>) -> Popup {
    Popup::new()
        .width(Size::px(NOTICE_WIDTH))
        .on_close_request(on_close)
}

/// The body of one: the air round it, the gap between its lines, and the interface font,
/// which `Popup` does not set.
pub(crate) fn notice_body() -> Rect {
    rect()
        .padding(NOTICE_PAD)
        .spacing(NOTICE_PAD)
        .font(&fonts().ui)
        .color(palette().text_fg)
}

/// A line of one that is not the question itself: what was done, or what will go with it.
///
/// [`dim_line`] with the clip taken off. A notice is a fixed width and its lines are
/// sentences, so what does not fit wraps; a line cut at the box's edge would lose the half
/// that says what goes with the thing being deleted.
pub(crate) fn notice_line(text: String) -> Label {
    dim_line(text).max_lines(None)
}

/// A path in one: a paragraph and not a label, because a path is as long as it is and one
/// cut off is one the reader cannot go and look at.
pub(crate) fn notice_path(text: String) -> Paragraph {
    paragraph()
        .assembly_font()
        .color(palette().address_fg)
        .span(text)
}

/// A list under the line that says what it is: the heading, and the rows taking the rest
/// of the pane.
///
/// The frame both grouped panels draw their answer in. The top is any element and not a
/// heading's text, so the Objects panel's "Add binaries..." button over its tree is the
/// same drawing rather than a third copy of these three rects. A `Rect` and not an
/// `Element`, so a caller that wants a ground under the whole of it says so on the frame
/// itself.
pub(crate) fn headed(heading: Element, list: Element) -> Rect {
    rect()
        .expanded()
        .content(Content::Flex)
        .child(heading)
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(list),
        )
}

/// The cell a field's value is laid out in, where the value is more than one thing: a box
/// and the button that fills it, a label and the button that undoes it, a stepper's two
/// buttons round its number. As wide as the cell [`field_row`] gives it, so a box inside
/// takes the width that is left rather than the width of its own text.
pub(crate) fn value_row() -> Rect {
    flex_row(Size::flex(1.0))
}

/// The same across a whole pane rather than inside a field's cell: [`field_row_in`] itself,
/// the two prompt bands, a Debug page row, a gesture row. [`Size::fill`] and not
/// [`Size::flex`], there being no cell around it to take a share of.
pub(crate) fn wide_row() -> Rect {
    flex_row(Size::fill())
}

/// What the two share: one line of things a fixed width apart, laid out under
/// [`Content::Flex`] so whichever of them is the row's `flex` child gets what the others
/// leave.
fn flex_row(width: Size) -> Rect {
    rect()
        .width(width)
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(ROW_GAP)
}

/// A choice between a handful of named options, written into wherever `choose` puts it: the
/// theme on the Settings page and the cargo profile on the Project view.
///
/// Keyed by the option's own text, so a segment is diffed by which option it is. The
/// options are borrowed and the labels are `'static`, being written out at the call site;
/// what crosses into the handlers is the option itself, which is [`Copy`].
pub(crate) fn choice<C: Copy + PartialEq + 'static>(
    options: &[(C, &'static str)],
    current: C,
    choose: impl FnMut(C) + Clone + 'static,
) -> SegmentedButton {
    SegmentedButton::new().children(
        options
            .iter()
            .map(|&(option, text)| {
                let mut choose = choose.clone();
                ButtonSegment::new()
                    .key(text)
                    .selected(current == option)
                    .on_press(move |_| choose(option))
                    .child(text)
                    .into()
            })
            .collect::<Vec<Element>>(),
    )
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
    wide_row()
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
///
/// The lines take no colour of their own: the root sets `text_fg` and a label inherits it
/// (`ui.rs`).
pub(crate) fn text_block(text: &str) -> Element {
    rect()
        .width(Size::fill())
        .children(
            text.lines()
                .map(|line| label().text(line.to_owned()).assembly_font().into())
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
/// cursor on the line, the project's opens the file. Both hand it to `PlaceTarget`
/// (`src/ui/place_target.rs`), which draws a place neither can reach as the plain line it
/// would have been.
pub(crate) fn diagnostic_block(diagnostic: &Diagnostic, place: Option<Element>) -> Element {
    // An error is the red every invalid thing wears, a warning the one warm hue in the
    // palette, and a note recedes.
    let (word, colour) = match diagnostic.level {
        Level::Error => ("error", palette().invalid_fg),
        Level::Warning => ("warning", palette().string_fg),
        Level::Note => ("note", palette().address_fg),
    };
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
                .child(label().text(word).color(colour).max_lines(1))
                .maybe_child(place)
                // The sentence rustc wrote, wrapping rather than cut at the pane's edge.
                .child(
                    label()
                        .text(diagnostic.message.clone())
                        .width(Size::flex(1.0)),
                ),
        )
        .child(text_block(&diagnostic.rendered))
        .into_element()
}

/// How a diagnostic's place is spelled: the file, the line and the column. The file as cargo
/// named it, which for one under the directory being built is a short path relative to where
/// it ran.
pub(crate) fn diagnostic_place(span: &cargo::Span) -> String {
    place_of(&span.file, span)
}

/// The same place with the file cut down to its own name, which is what a file outside the
/// directory being built gets: a registry path is most of a line on its own, and which crate
/// it is in is the useful half.
pub(crate) fn diagnostic_place_by_name(span: &cargo::Span) -> String {
    place_of(&source::name_of(Path::new(&span.file)), span)
}

/// One place out of a file spelled either way.
fn place_of(file: &str, span: &cargo::Span) -> String {
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

//! One row of a code listing, as every kind of row in the three listings is drawn: the
//! width every row of a listing shares (`ui/width.rs`), the wash for the run and the pair,
//! and the two pointer handlers that pick rows and characters out. The row kinds hand in
//! what differs -- a gutter, the text, a menu -- and keep what is theirs on top.
//!
//! The text is one `paragraph()`, with a relocation link placed **inside** it as an inline
//! child: freya reserves a placeholder sized from the child and moves the child's layout
//! node to it, so the link keeps its own hover, cursor and press, and to the text engine it
//! is one unit of the row (`Piece::Inline`). The character selection is the app's own
//! (`src/chars.rs`); freya supplies the two primitives a paragraph has anyway, the skia
//! hit-test behind its [`ParagraphHolder`] and its highlight paint. No editor, no rope.
//!
//! The pointer's icon is the row's to set, in one place: an I-beam over the text and to
//! the right of it, the hand over a link inside it (which says so through `over_link`),
//! and the arrow over the gutter and on leaving the row. Set only when it changes, since
//! each set is a message to the platform, and kept in one cell for the whole thread: a
//! row's own memory of it would be wrong the moment the row beside it set something else.
//!
//! **Nothing inside a row may listen to `pointer_down`.** A bubbling event is measured
//! once, against the deepest listener, and every ancestor's handler is handed the same
//! data (`notes/upstream/freya.md`), so a child listening to the down would hand the row a
//! location relative to the child and the column would be wrong. The links listen to the
//! press, which is a different event, and to `over`/`out`.

use std::cell::Cell;

use super::*;

thread_local! {
    /// The icon last set from a row, so a move that changes nothing sends nothing.
    static ICON: Cell<CursorIcon> = const { Cell::new(CursorIcon::Default) };
}

/// The horizontal padding every code row takes, which an absolutely placed child of the
/// row is inside: a caret placed at a column is placed from the padding's inner edge.
const ROW_PAD: f32 = 3.0;

/// Set the pointer's icon, if it is not that already.
fn set_icon(icon: CursorIcon) {
    ICON.with(|last| {
        if last.get() != icon {
            last.set(icon);
            Cursor::set(icon);
        }
    });
}

/// What a row's text is: the pieces the clipboard sees, the spans the paragraph draws,
/// and the part of the pane's character selection this row draws.
pub(crate) struct Text {
    /// The row's text as it is drawn, which is what the columns count and the copy takes.
    pub(crate) line: Line,
    /// The spans before the inline element, and after it; all of them when there is none.
    pub(crate) head: Vec<Span<'static>>,
    pub(crate) inline: Option<Element>,
    pub(crate) tail: Vec<Span<'static>>,
    /// What this row draws of the character selection.
    pub(crate) chars: RowChars,
}

/// What one row draws of the pane's character selection, as the list tells it: its
/// columns inside the run, and the caret where the run's lead is on this row. Worked out
/// per row so a row's prop changes only when an end moves on it, and not as a sweep
/// passes over other rows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct RowChars {
    /// The columns from the first end on this row to the second on its own, unclamped at
    /// the end ([`CharSelection::of_row`] asked with no width); `None` outside the run.
    pub(crate) highlight: Option<(usize, usize)>,
    /// The column the caret is drawn at, where the run's lead is on this row. The text
    /// engine leaves it undrawn under a highlight, so it shows for a press and not for a
    /// sweep, as an editor's does.
    pub(crate) cursor: Option<usize>,
}

/// The wash row `row` wears for its pane's selection: the selection's own colour for a row
/// of a run picked out whole -- from the gutter, by Ctrl+A, from outside the panes -- which
/// is a run with no characters under it; and the faded one for the caret's row, which is
/// where a press on the text has left one that no sweep has moved. A sweep of characters
/// washes nothing: the highlight is the selection then.
pub(crate) fn wash_of(
    rows: Option<RowSelection>,
    chars: Option<CharSelection>,
    row: usize,
) -> Wash {
    match chars {
        None if rows.is_some_and(|run| run.contains(row)) => Wash::Selected,
        Some(chars) if chars.is_empty() && chars.lead().row == row => Wash::Cursor,
        _ => Wash::None,
    }
}

impl RowChars {
    pub(crate) fn of(chars: Option<CharSelection>, row: usize) -> Self {
        RowChars {
            highlight: chars.and_then(|chars| chars.of_row(row, usize::MAX)),
            cursor: chars
                .map(CharSelection::lead)
                .filter(|lead| lead.row == row)
                .map(|lead| lead.col),
        }
    }
}

/// What every row of a listing shares.
#[derive(Clone)]
pub(crate) struct Chrome {
    pub(crate) pane: Pane,
    /// The listing row, which the runs speak.
    pub(crate) row: usize,
    /// What the row is a row of, for the run it starts (see `Picked::file`).
    pub(crate) file: Option<Arc<str>>,
    pub(crate) paired: Option<Edges>,
    pub(crate) wash: Wash,
    pub(crate) widest: Widest,
    pub(crate) listing: u64,
    /// Whether the row reports its width to `widest`. A separator does not: its rule fills
    /// the row, so it would report the row plus its gutter and the widest would grow by a
    /// gutter's width every layout, without end.
    pub(crate) measured: bool,
}

/// The row: its chrome, what comes `before` the text -- a gutter, an address, a line
/// number -- the `text` where the row has any, and the `menu` the right button opens.
pub(crate) fn code_row(
    chrome: Chrome,
    before: Vec<Element>,
    text: Option<Text>,
    menu: Option<Rc<dyn Fn(Event<PressEventData>)>>,
) -> Rect {
    let marked = use_consume::<Marked>().0;
    let shift = use_consume::<Shift>().0;
    // The laid-out paragraph, for the pointer to be answered in columns. One per row, as
    // freya's own editor keeps one per line.
    let holder = use_state(ParagraphHolder::default);
    // Where the row and its paragraph were laid out, so a pointer location relative to
    // the row can be made relative to the text. Cells and not states: nothing renders
    // from them, and the difference between the two is scroll-invariant.
    let row_x = use_hook(|| Rc::new(Cell::new(0.0f32)));
    let text_x = use_hook(|| Rc::new(Cell::new(0.0f32)));
    // Whether the pointer is over the link inside the text, which the link's box says.
    let over_link = use_hook(|| Rc::new(Cell::new(false)));
    // Whether the paragraph has been laid out, which is when the holder can answer where
    // a column is: the caret is drawn from the render after that.
    let mut laid = use_state(|| false);
    let grid = pixel_grid();

    let Chrome {
        pane,
        row,
        file,
        paired,
        wash,
        widest,
        listing,
        measured,
    } = chrome;
    let has_text = text.is_some();

    // The column under `at`, a location relative to the row: `None` left of the text on
    // a press, which is the gutter and picks rows out alone; and on a sweep column 0,
    // since a pointer left of the text is where the line starts.
    let column = {
        let holder = holder.clone();
        let (row_x, text_x) = (row_x.clone(), text_x.clone());
        move |at: CursorPoint, press: bool| -> Option<usize> {
            if !has_text {
                return None;
            }
            let x = at.x as f32 - (text_x.get() - row_x.get());
            if x < 0.0 {
                return if press { None } else { Some(0) };
            }
            caret_col(&holder.read(), x, at.y as f32)
        }
    };

    let paragraph = text.map(|text| {
        let units = text.line.units();
        let highlight = text
            .chars
            .highlight
            .map(|(from, to)| (from.min(units), to.min(units)));
        let text_x = text_x.clone();
        // The caret, where the run's lead is on this row and no sweep has picked
        // characters out: a stroke of the row's own, on the device pixel grid, where the
        // engine's would sit on the glyph's fractional edge and two pixels wide. Drawn
        // once the paragraph has been laid out, from where the holder says the column is.
        let caret = text
            .chars
            .cursor
            .filter(|_| highlight.is_none() && laid())
            .and_then(|col| caret_x(&holder.read(), col.min(units)))
            .map(|x| {
                let stroke = grid.stroke(text_x.get() - row_x.get() - ROW_PAD + x, 1.0);
                rect()
                    .position(Position::new_absolute().left(stroke.near).top(0.0))
                    .width(Size::px(stroke.thick))
                    .height(Size::px(code_row_height()))
                    .background(palette().caret_fg)
            });
        // The link, in a box that says when the pointer is over it: the hand is the
        // link's and the I-beam the text's, and the row sets both (`set_icon`).
        let inline = text.inline.map(|inline| {
            let (entered, left) = (over_link.clone(), over_link.clone());
            rect()
                .on_pointer_over(move |_| {
                    entered.set(true);
                    set_icon(CursorIcon::Pointer);
                })
                .on_pointer_out(move |_| {
                    left.set(false);
                    set_icon(CursorIcon::Text);
                })
                .child(inline)
        });
        let paragraph = paragraph()
            .max_lines(1)
            // The row's whole height, so the highlight -- which the engine expands to
            // the paragraph's box -- runs from one row into the next with no gap.
            .height(Size::fill())
            .holder(holder.read().clone())
            .on_sized(move |e: Event<SizedEventData>| {
                text_x.set(e.area.min_x());
                laid.set_if_modified(true);
            })
            .highlights(highlight.map(|h| vec![h]))
            .highlight_color(palette().text_select_bg)
            .cursor_mode(CursorMode::Expanded)
            .vertical_align(VerticalAlign::Center)
            .spans_iter(text.head.into_iter())
            .maybe_child(inline)
            .spans_iter(text.tail.into_iter());
        (paragraph, caret)
    });
    let (paragraph, caret) = match paragraph {
        Some((paragraph, caret)) => (Some(paragraph), caret),
        None => (None, None),
    };

    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        // As wide as the pane or the listing's widest row, whichever is more, and what
        // it holds measured under it -- which is what lets the list scroll sideways to a
        // long row while the wash still runs the whole width. The width reported is the
        // content's, not the laid-out one: see `ui/width.rs`.
        .width(Widest::row_width(widest.floor(listing), listing))
        .on_sized({
            let row_x = row_x.clone();
            move |e: Event<SizedEventData>| {
                row_x.set(e.area.min_x());
                if measured {
                    widest.note(listing, e.inner_sizes.width);
                }
            }
        })
        .height(Size::px(code_row_height()))
        // Horizontally only: the gutter's lines run to the row's own top and bottom
        // edges, and padding there would break every line in the column once per row.
        .padding(Gaps::new_symmetric(0.0, ROW_PAD))
        .assembly_font()
        // Nothing of this row's own under the pointer: it is lit by the other pane's run,
        // where it is the same place, and by this pane's, where it is in it.
        .background(row_background(paired.is_some(), wash))
        .maybe(paired.is_some_and(Edges::any), |el| {
            el.border(pair_border(paired.unwrap_or_default()))
        })
        // The *down* and not the press: a drag is over by the time a press fires, so a
        // selection swept out with the button held has to begin as it goes down. The
        // right button's down is the menu, **in the same handler**: `on_secondary_down`
        // is `on_pointer_down` under another name and would replace this one
        // (`secondary`).
        .on_pointer_down({
            let column = column.clone();
            let holder = holder.clone();
            move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    let press = column(e.element_location(), true).map(|col| {
                        // freya counts the presses in one place: two on a word take the
                        // word, three the row's text, as the text engine divides them.
                        match EventsCombos::pressed(e.global_location()) {
                            PressEventType::Double => word_at(&holder.read(), col)
                                .map(|(from, to)| Press::Span(from, to))
                                .unwrap_or(Press::At(col)),
                            PressEventType::Triple | PressEventType::Quadruple => {
                                Press::Span(0, usize::MAX)
                            }
                            PressEventType::Single => Press::At(col),
                        }
                    });
                    mark_press(marked, *shift.peek(), pane, file.clone(), row, press);
                    return;
                }
                let Some(e) = secondary(e) else {
                    return;
                };
                if let Some(menu) = &menu {
                    menu(e);
                }
            }
        })
        // Sweeping a selection out to here, and to the column under the pointer. Every
        // move and not `pointer_over`, which fires once on entry: a sweep along a row
        // has to follow the pointer. And the icon: an I-beam over the text and right of
        // it, the hand over the link, the arrow over the gutter.
        .on_pointer_move({
            let (row_x, text_x) = (row_x.clone(), text_x.clone());
            move |e: Event<PointerEventData>| {
                let at = e.element_location();
                mark_drag(marked, pane, row, column(at, false));
                let on_text = has_text && at.x as f32 >= text_x.get() - row_x.get();
                set_icon(if over_link.get() {
                    CursorIcon::Pointer
                } else if on_text {
                    CursorIcon::Text
                } else {
                    CursorIcon::Default
                });
            }
        })
        .on_pointer_out(|_| set_icon(CursorIcon::Default))
        .children(before)
        .maybe_child(paragraph)
        .maybe_child(caret)
}

/// The padding that puts a listing's rows on the device pixel grid, from where the box
/// around them was laid out: whatever fraction the bars, tabs and fonts above a listing
/// add up to, its rows are washed and highlighted as whole pixels, so two rows' washes
/// meet on an edge instead of each fading into the other over the pixel they share.
/// Handed the box's area as its `on_sized` reports it; read as the box's top padding.
#[derive(Clone, Copy)]
pub(crate) struct Nudge(State<f32>);

pub(crate) fn use_nudge() -> Nudge {
    Nudge(use_state(|| 0.0f32))
}

impl Nudge {
    /// The box was laid out with its top at `top`. The grid is taken at the render and
    /// not here, so the handler asks nothing of the runtime.
    pub(crate) fn measured(self, grid: Grid, top: f32) {
        let mut nudge = self.0;
        nudge.set_if_modified(grid.nudge(top));
    }

    /// The padding to put on the box's top. A read: the box re-renders as it lands.
    pub(crate) fn padding(self) -> Gaps {
        Gaps::new(*self.0.read(), 0.0, 0.0, 0.0)
    }
}

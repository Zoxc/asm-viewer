//! One row of a code listing, as every kind of row in the three listings is drawn: the
//! width every row of a listing shares (`ui/width.rs`), the wash for the run and the pair,
//! and the two pointer handlers that pick rows and characters out. The row kinds hand in
//! what differs -- a gutter, the text, a menu -- and keep what is theirs on top.
//!
//! The text is one `paragraph()`, with a relocation link placed **inside** it as an inline
//! child: freya reserves a placeholder sized from the child and moves the child's layout
//! node to it, so the link keeps its own hover, cursor and press, and to the text engine it
//! is one unit of the row (`Piece::Inline`). The character selection is the app's own
//! (`src/chars.rs`); freya supplies one primitive a paragraph has anyway, the skia
//! hit-test behind its [`ParagraphHolder`], which answers both where a pointer is
//! (`caret_col`) and where a column is (`caret_x`). No editor, no rope -- and no engine
//! paint either: the highlight and the caret are rects of the row's own, placed by the
//! column's x and the row's height on the device pixel grid, where the engine's highlight
//! is the glyphs' tight box and leaves a seam between one row's and the next's.
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
    /// Whether the inline element is a link only while Ctrl is held -- a door into the
    /// object's code -- which is when the hand is shown over it; a link that is one
    /// always shows the hand always.
    pub(crate) door: bool,
    /// The columns of the runs of this row's own text that are links, in the order they
    /// are drawn. A door that is text and not an element: the row's columns stay the
    /// file's own, which is what lets a press on one say where it was in the terms
    /// everything else speaks (`src/chars.rs`).
    ///
    /// `head` is cut at their edges before it is drawn ([`cut_at`]), so lighting a link
    /// changes a span's style and never where the spans are cut -- a boundary that moved
    /// with the pointer would re-shape the row and widen the listing for good.
    pub(crate) links: Vec<Range<usize>>,
    /// What a press on one of them does, given its columns. Built per row, as the menu
    /// is.
    pub(crate) on_link: Option<Rc<dyn Fn(Range<usize>)>>,
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
    /// The column the caret is drawn at, where the run's lead is on this row -- over a
    /// selection too, at its lead, since that is where the next key moves from.
    pub(crate) cursor: Option<usize>,
}

/// The wash row `row` wears: the faded one for the caret's row, which is where the run's
/// lead is while nothing is selected -- a press on the text or in the gutter, a landing,
/// a key -- and nothing otherwise: a selection is drawn by the row under its text, and
/// washes no row.
pub(crate) fn wash_of(chars: Option<CharSelection>, row: usize) -> Wash {
    match chars {
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

/// The list a row is in, provided by each list to its rows: its scroll and its box, for
/// a row that draws the caret out of the pane's sight to bring it sideways into it -- the
/// box and not the row's own `visible_area`, which freya reports unclipped, the whole row
/// wide -- and the paragraphs its rows have lent it, for a sweep that has left the rows
/// to ask a row where a column is. The widest row is the sideways extent, asked under the
/// key the list hands [`use_sweep_beyond`] every render: a key held here is made once and
/// would go on naming the listing the list was mounted on.
#[derive(Clone)]
pub(crate) struct Listing {
    pub(crate) controller: ScrollController,
    pub(crate) bounds: Rc<Cell<Area>>,
    pub(crate) texts: Rc<RefCell<HashMap<usize, RowText>>>,
    pub(crate) widest: Widest,
}

/// A row's laid-out paragraph and where it starts, lent to the list by the row as it
/// renders: what answers a column for an x on a row the pointer is not over. Written
/// afresh by every render of the row, so a row the list has stopped building leaves a
/// stale entry that no reach asks about, the rows asked about being on screen.
#[derive(Clone)]
pub(crate) struct RowText {
    holder: ParagraphHolder,
    text_x: Rc<Cell<f32>>,
}

impl Listing {
    /// A fresh list, with nothing lent yet.
    pub(crate) fn new(controller: ScrollController, widest: Widest) -> Self {
        Listing {
            controller,
            bounds: Rc::new(Cell::new(Area::zero())),
            texts: Rc::new(RefCell::new(HashMap::new())),
            widest,
        }
    }

    /// The column at window x `x` on row `row`, off the paragraph the row lent: 0 for a
    /// row with no text and for an x left of its text, the end for one right of it.
    fn column_at(&self, row: usize, x: f32) -> usize {
        let texts = self.texts.borrow();
        let Some(text) = texts.get(&row) else {
            return 0;
        };
        let x = x - text.text_x.get();
        if x < 0.0 {
            return 0;
        }
        caret_col(&text.holder, x, 0.0).unwrap_or(0)
    }
}

/// How far inside the pane's edge a caret brought into sight is put.
const CARET_INSET: f32 = 8.0;

/// How wide the caret is drawn, in logical pixels: two, as most editors draw theirs, and
/// on the grid so it is whole device pixels either side of the column.
const CARET_WIDTH: f32 = 2.0;

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
///
/// The menu is handed the column the pointer was over, which is what a question about the
/// name under it needs and only this knows: `None` in the gutter, and on a row with no
/// text at all.
pub(crate) fn code_row(
    chrome: Chrome,
    before: Vec<Element>,
    text: Option<Text>,
    menu: Option<Rc<dyn Fn(Event<PressEventData>, Option<usize>)>>,
) -> Rect {
    let marked = use_consume::<Marked>().0;
    let shift = use_consume::<Shift>().0;
    let listing = use_consume::<Listing>();
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
    // Whether the link is one now: always, or only while Ctrl is held, for a door. Asked
    // at the pointer's move and not subscribed to, the icon being set from handlers.
    let ctrl = try_consume_context::<Ctrl>().map(|ctrl| ctrl.0);
    let alt = try_consume_context::<Alt>().map(|alt| alt.0);
    let door = text.as_ref().is_some_and(|text| text.door);
    let hand = move || !door || ctrl.is_some_and(|ctrl| *ctrl.peek());
    // The links in the row's own text, and what a press on one does. Taken out of `text`
    // before it is moved into the paragraph below, since the handlers need them.
    let links: Vec<Range<usize>> = text
        .as_ref()
        .map(|text| text.links.clone())
        .unwrap_or_default();
    let on_link = text.as_ref().and_then(|text| text.on_link.clone());
    // Which of them the pointer is over, or `None`. Written with `set_if_modified`, so a
    // row is drawn again when the pointer crosses a link's edge and not as it moves along
    // one.
    let mut over = use_state(|| None::<usize>);
    // Alt says a press on a link is not a door, so what is under the pointer is text.
    let held = move || alt.is_some_and(|alt| *alt.peek());
    let at_link = {
        let links = Rc::new(links.clone());
        move |column: Option<usize>| -> Option<usize> {
            let column = column?;
            links.iter().position(|link| link.contains(&column))
        }
    };
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
        listing: listing_key,
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

    // Lent to the list, for a sweep that has left the rows to ask this one where a
    // column is.
    if has_text {
        listing.texts.borrow_mut().insert(
            row,
            RowText {
                holder: holder.read().clone(),
                text_x: text_x.clone(),
            },
        );
    }

    let lit = over();
    let paragraph = text.map(|text| {
        let units = text.line.units();
        let highlight = text
            .chars
            .highlight
            .map(|(from, to)| (from.min(units), to.min(units)));
        let text_x = text_x.clone();
        // Where a column is, from the row's padded edge, once the paragraph has been laid
        // out and the holder can say.
        let column_x = {
            let holder = holder.clone();
            let (row_x, text_x) = (row_x.clone(), text_x.clone());
            move |col: usize| -> Option<f32> {
                laid().then_some(())?;
                let x = caret_x(&holder.read(), col.min(units))?;
                Some(text_x.get() - row_x.get() - ROW_PAD + x)
            }
        };
        // The highlight: a rect of the row's own from the first column's x to the last's,
        // the row's whole height, on the grid -- so one row's meets the next's on a pixel
        // edge. An empty row inside the run shows as a stub, or the run would read as
        // broken there.
        let selected = highlight.and_then(|(from, to)| {
            let (left, right) = (column_x(from)?, column_x(to)?);
            let right = if right > left {
                right
            } else if units == 0 {
                left + code_row_height() / 4.0
            } else {
                return None;
            };
            let span = grid.span(left, right);
            // Not interactive, and nor is the caret: a mark answers no press and no move.
            Some(
                rect()
                    .interactive(false)
                    .position(Position::new_absolute().left(span.near).top(0.0))
                    .width(Size::px(span.thick))
                    .height(Size::px(code_row_height()))
                    .background(palette().text_select_bg),
            )
        });
        // The caret, where the run's lead is on this row and no sweep has picked
        // characters out: a stroke of the row's own, on the device pixel grid, where the
        // engine's would sit on the glyph's fractional edge and two pixels wide.
        // Drawn over a selection too, at its lead: it is where the next key moves from.
        let caret = text.chars.cursor.and_then(column_x).map(|x| {
            // A caret past the pane's edge brings the list sideways to it: the
            // keyboard walks the caret along a row longer than the pane, and the
            // pane has to follow. From a task and not the render, since a scroll
            // is a write; the list answers with a layout, whose `on_sized` moves
            // `visible`, and a caret then inside asks for nothing more.
            let seen = listing.bounds.get();
            if seen.width() > 0.0 {
                let at = row_x.get() + ROW_PAD + x;
                let shove = if at < seen.min_x() {
                    Some(seen.min_x() - at + CARET_INSET)
                } else if at + 1.0 > seen.max_x() {
                    Some(seen.max_x() - at - 1.0 - CARET_INSET)
                } else {
                    None
                };
                if let Some(shove) = shove.filter(|shove| shove.abs() >= 1.0) {
                    let mut controller = listing.controller;
                    // Nothing to bring in from the left of the row's own start.
                    let shove = shove.min(-(row_x.get() - seen.min_x()).min(0.0));
                    spawn(async move {
                        let (x0, _) = <(i32, i32)>::from(controller);
                        let target = (x0 + shove.round() as i32).min(0);
                        if target != x0 {
                            controller.scroll_to_x(target);
                        }
                    });
                }
            }
            // From the column rightward, so a caret on column 0 starts where the
            // text does.
            let stroke = grid.span(x, x + CARET_WIDTH);
            rect()
                .interactive(false)
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
                    set_icon(if hand() {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Text
                    });
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
            .vertical_align(VerticalAlign::Center)
            .spans_iter(
                light(
                    cut_at(text.head, &links),
                    lit.and_then(|lit| links.get(lit)),
                )
                .into_iter(),
            )
            .maybe_child(inline)
            .spans_iter(text.tail.into_iter());
        (paragraph, selected, caret)
    });
    let (paragraph, selected, caret) = match paragraph {
        Some((paragraph, selected, caret)) => (Some(paragraph), selected, caret),
        None => (None, None, None),
    };

    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        // As wide as the pane or the listing's widest row, whichever is more, and what
        // it holds measured under it -- which is what lets the list scroll sideways to a
        // long row while the wash still runs the whole width. The width reported is the
        // content's, not the laid-out one: see `ui/width.rs`.
        .width(Widest::row_width(widest.floor(listing_key), listing_key))
        .on_sized({
            let row_x = row_x.clone();
            move |e: Event<SizedEventData>| {
                row_x.set(e.area.min_x());
                if measured {
                    widest.note(listing_key, e.inner_sizes.width);
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
            let at_link = at_link.clone();
            let links = links.clone();
            move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    let at = column(e.element_location(), true);
                    // freya counts the presses in one place, and **asking is counting**:
                    // a press it was not asked about is one the next reads as a double.
                    // So it is asked exactly once, whatever the press turns out to be.
                    let presses = EventsCombos::pressed(e.global_location());
                    // A link is followed on a single press with nothing held: two presses
                    // on a name are what take the word, and Alt says this one is not a
                    // door.
                    let link = at_link(at)
                        .filter(|_| presses == PressEventType::Single && !held())
                        .zip(on_link.clone());
                    if let Some((link, follow)) = link {
                        // And it picks no line out: the press is the question and not a
                        // place in the file.
                        follow(links[link].clone());
                        return;
                    }
                    let press = at.map(|col| {
                        // Two presses on a word take the word, three the row's text, as
                        // the text engine divides them.
                        match presses {
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
                // The column before the event is turned into a press: what the menu is
                // asked about is where the pointer was.
                let at = column(e.element_location(), true);
                let Some(e) = secondary(e) else {
                    return;
                };
                if let Some(menu) = &menu {
                    menu(e, at);
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
                let column = column(at, false);
                mark_drag(marked, pane, row, column);
                // Not while a selection is being swept out: a drag along a line would
                // otherwise underline every name it passed under.
                let hovered = match dragging(marked, pane) || held() {
                    true => None,
                    false => at_link(column),
                };
                over.set_if_modified(hovered);
                let on_text = has_text && at.x as f32 >= text_x.get() - row_x.get();
                set_icon(if (over_link.get() && hand()) || hovered.is_some() {
                    CursorIcon::Pointer
                } else if on_text {
                    CursorIcon::Text
                } else {
                    CursorIcon::Default
                });
            }
        })
        .on_pointer_out(move |_| {
            over.set_if_modified(None);
            set_icon(CursorIcon::Default);
        })
        .children(before)
        // Before the paragraph in the tree, so it is painted under the text -- and
        // **always there**, as is the caret's slot: freya matches siblings by position,
        // so a rect appearing before the paragraph on the press would move the paragraph
        // along one and remount it, link and all, between the down and the up, and the
        // press meant for the link would never fire.
        .maybe(has_text, |el| {
            el.child(selected.unwrap_or_else(nothing))
                .maybe_child(paragraph)
                .child(caret.unwrap_or_else(nothing))
        })
}

/// `head` cut so that every run in `links` is exactly one span of it, splitting a span
/// that holds more than one and leaving the rest alone.
///
/// Where a row's links came from its own colour runs this changes nothing: a name is one
/// capture and the columns were taken from it. They stop coming from there once a
/// language server says which names are links (`src/links.rs`), and its spans are its own,
/// so this is what keeps [`light`]'s one job -- change a span's style -- from silently
/// doing nothing to a link that straddles a boundary.
///
/// **The cut is the same on every render**, never only under the pointer. A span split in
/// two measures a shade wider than the same characters in one, skia shaping each
/// separately, and the widest row a listing has drawn only ever grows (`src/ui/width.rs`):
/// a cut that came and went with the pointer would widen the listing for good.
///
/// Columns are UTF-16 units, and a boundary inside a character is not one: a split there
/// would cut a `char` in half, so the span is left whole.
pub(crate) fn cut_at(head: Vec<Span<'static>>, links: &[Range<usize>]) -> Vec<Span<'static>> {
    if links.is_empty() {
        return head;
    }
    let mut cut = Vec::with_capacity(head.len());
    let mut column = 0;
    for span in head {
        let units = span.text.encode_utf16().count();
        let (from, to) = (column, column + units);
        column = to;
        // Where inside this span a link begins or ends, in the order they are drawn.
        let mut edges: Vec<usize> = links
            .iter()
            .flat_map(|link| [link.start, link.end])
            .filter(|edge| *edge > from && *edge < to)
            .map(|edge| edge - from)
            .collect();
        if edges.is_empty() {
            cut.push(span);
            continue;
        }
        edges.sort_unstable();
        edges.dedup();
        let mut at = 0;
        for edge in edges.into_iter().chain(std::iter::once(units)) {
            let Some(piece) = utf16_slice(&span.text, at..edge) else {
                continue;
            };
            at = edge;
            cut.push(Span {
                text: std::borrow::Cow::Owned(piece),
                text_style_data: span.text_style_data.clone(),
            });
        }
        // A cut that fell inside a character leaves nothing of the span, so it is kept
        // whole rather than lost.
        if at == 0 {
            cut.push(span);
        }
    }
    cut
}

/// `text` between two UTF-16 offsets, and `None` where either falls inside a character.
fn utf16_slice(text: &str, units: Range<usize>) -> Option<String> {
    let (mut from, mut to) = (None, None);
    let mut seen = 0;
    for (at, character) in text.char_indices() {
        if seen == units.start {
            from = Some(at);
        }
        if seen == units.end {
            to = Some(at);
        }
        seen += character.len_utf16();
    }
    if seen == units.start {
        from = Some(text.len());
    }
    if seen == units.end {
        to = Some(text.len());
    }
    let (from, to) = (from?, to?);
    (from < to).then(|| text[from..to].to_owned())
}

/// `head` with the run at `columns` drawn as a link under the pointer, and unchanged
/// where nothing is.
///
/// Every span the link covers is drawn as one, and no span is ever cut here: [`cut_at`]
/// has already made sure none straddles a link's edge, so each is wholly inside the run
/// or wholly outside it. A boundary that moved with the pointer would re-shape the row,
/// and the widest row a listing has drawn only ever grows.
///
/// More than one span where a link crosses a colour boundary, which a name the server
/// placed may do and a name taken from a colour run never could.
///
/// The colour is the underline's too: freya's spans carry a decoration and no colour for
/// it, and skia draws one in the text's own. Which is what is wanted -- one colour says
/// both -- and is what `name_hover_fg` already describes itself as.
fn light(head: Vec<Span<'static>>, columns: Option<&Range<usize>>) -> Vec<Span<'static>> {
    let Some(columns) = columns else {
        return head;
    };
    let mut column = 0;
    head.into_iter()
        .map(|span| {
            let units = span.text.encode_utf16().count();
            let at = column;
            column += units;
            match at >= columns.start && column <= columns.end && at < column {
                true => span
                    .color(palette().name_hover_fg)
                    .text_decoration(TextDecoration::Underline),
                false => span,
            }
        })
        .collect()
}

/// A mark's slot with no mark in it: nothing drawn, nothing hit, no size.
fn nothing() -> Rect {
    rect()
        .interactive(false)
        .position(Position::new_absolute().left(0.0).top(0.0))
        .width(Size::px(0.0))
        .height(Size::px(0.0))
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

    /// The padding as it is, for a handler.
    pub(crate) fn value(self) -> f32 {
        *self.0.peek()
    }
}

/// How often the view moves while a sweep is held past an edge: a row up or down, and
/// a row's height sideways, each time.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(40);

/// Whether `pane`'s run is being swept: the button down on it.
fn dragging(marked: State<Marks>, pane: Pane) -> bool {
    marked
        .peek()
        .of(pane)
        .as_ref()
        .is_some_and(|picked| picked.rows.dragging)
}

/// Where a sweep at `at`, a window location, reaches once it has left the rows of
/// `listing`: [`beyond`], with the rows' top worked out from the list's scroll and its
/// nudge, and the column off the paragraph the row lent. `None` while the pointer is over
/// a row, which answers for itself.
fn reach(listing: &Listing, nudge: Nudge, length: usize, at: CursorPoint) -> Option<Caret> {
    let area = listing.bounds.get();
    let (_, scrolled) = <(i32, i32)>::from(listing.controller);
    let rows_top = nudge.value() + scrolled as f32;
    let bounds = Bounds {
        left: area.min_x(),
        top: area.min_y(),
        right: area.max_x(),
        bottom: area.max_y(),
    };
    let reached = beyond(
        bounds,
        rows_top,
        code_row_height(),
        length,
        at.x as f32,
        at.y as f32,
    )?;
    Some(Caret {
        row: reached.row,
        col: listing.column_at(reached.row, reached.x),
    })
}

/// The handler that carries a sweep on once the pointer has left the rows: outside the
/// listing's box, the pane, or the window. The platform keeps reporting the pointer while
/// a button is held wherever it goes, freya forwards every move and sends its global move
/// to every listener without hit-testing (`notes/upstream/freya.md`), so this goes on the
/// listing's box as `on_global_pointer_move` and asks [`beyond`] where the sweep reaches:
/// nothing while the pointer is over a row, which answers for itself. `length` is the
/// listing's rows.
///
/// Held past an edge of the box, the sweep **scrolls the view**: a task moves it every
/// [`AUTOSCROLL_TICK`] towards the pointer -- a row up or down, a row's height sideways --
/// and reaches the run out to what came in, for as long as the button is down and the
/// pointer stays past an edge; the pointer's last place is kept in a cell the handler
/// writes and the task reads, since nothing arrives from a pointer that is not moving. A
/// hook, for the cells to outlive the handler a render makes afresh; one task at a time,
/// the flag says.
///
/// `length` and `key` are the listing's rows and its identity in [`Widest`], both this
/// render's: neither is held in the [`Listing`], since a list handed another listing -- a
/// link followed in place, a symbol previewed into the temporal tab, a companion file
/// switching -- is not mounted again, and a key made at the mount would name a listing
/// that is gone. `Widest` answers nothing for one it does not hold: a sideways extent of
/// zero, and an autoscroll that pins the pane to its left edge.
pub(crate) fn use_sweep_beyond(
    marked: State<Marks>,
    pane: Pane,
    listing: Listing,
    nudge: Nudge,
    length: usize,
    key: u64,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let last = use_hook(|| Rc::new(Cell::new(None::<CursorPoint>)));
    let running = use_hook(|| Rc::new(Cell::new(false)));

    move |e: Event<PointerEventData>| {
        let at = e.global_location();
        last.set(Some(at));
        if let Some(caret) = reach(&listing, nudge, length, at) {
            mark_drag(marked, pane, caret.row, Some(caret.col));
        }

        let area = listing.bounds.get();
        let past = at.y < area.min_y() as f64
            || at.y >= area.max_y() as f64
            || at.x < area.min_x() as f64
            || at.x >= area.max_x() as f64;
        if !past || running.get() || !dragging(marked, pane) {
            return;
        }
        running.set(true);
        let (last, running, listing) = (last.clone(), running.clone(), listing.clone());
        spawn(async move {
            loop {
                Timer::after(AUTOSCROLL_TICK).await;
                let Some(at) = last.get() else { break };
                if !dragging(marked, pane) {
                    break;
                }
                let area = listing.bounds.get();
                // Each offset counts down from zero, so towards the far side is less.
                let side = |before: bool, past: bool| {
                    if before {
                        1
                    } else if past {
                        -1
                    } else {
                        0
                    }
                };
                let down = side(at.y < area.min_y() as f64, at.y >= area.max_y() as f64);
                let across = side(at.x < area.min_x() as f64, at.x >= area.max_x() as f64);
                if down == 0 && across == 0 {
                    break;
                }
                let step = code_row_height() as i32;
                let (x, y) = <(i32, i32)>::from(listing.controller);
                let mut controller = listing.controller;
                if down != 0 {
                    let extent = (length as f32 * code_row_height() - area.height()).max(0.0);
                    let target = (y + down * step).clamp(-(extent as i32), 0);
                    if target != y {
                        controller.scroll_to_y(target);
                    }
                }
                if across != 0 {
                    let extent = (listing.widest.extent(key) - area.width()).max(0.0);
                    let target = (x + across * step).clamp(-(extent as i32), 0);
                    if target != x {
                        controller.scroll_to_x(target);
                    }
                }
                if let Some(caret) = reach(&listing, nudge, length, at) {
                    mark_drag(marked, pane, caret.row, Some(caret.col));
                }
            }
            running.set(false);
        });
    }
}

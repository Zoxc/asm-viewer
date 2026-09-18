//! One row of a code listing, as every kind of row in the three listings is drawn: the
//! width every row of a listing shares (`ui/width.rs`), the wash for the run and the pair,
//! and the two pointer handlers that pick rows and characters out. The row kinds hand in
//! what differs -- a gutter, the text, a menu -- and keep what is theirs on top.
//!
//! The text is one `paragraph()` of spans, links included: a link is a run of the row's
//! own text ([`TextLinks`]), so a sweep selects across it and copies it as it does any
//! other text. The character selection is the app's own
//! (`src/chars.rs`); freya supplies one primitive a paragraph has anyway, the skia
//! hit-test behind its [`ParagraphHolder`], wrapped at the head of this file as the probes
//! that answer where a pointer is (`caret_col`), where a column is (`caret_x`) and where a
//! word ends (`word_at`). No editor, no rope -- and no engine paint either: the highlight
//! and the caret are rects of the row's own, placed by the column's x and the row's height
//! on the device pixel grid, where the engine's highlight is the glyphs' tight box and
//! leaves a seam between one row's and the next's.
//!
//! **A link is drawn one way wherever it is** -- the wash, the rounded corner, the rule
//! under it and the lit colour of `link_chrome` -- and is drawn as one only while a press
//! on it would be a door. That answer is the link's own ([`TextLinks`]'s `is_link`), asked
//! once and used for the light, for the pointer's icon and by the press, so none of the
//! three can offer what the others will not do. The box is the row's, placed over the
//! link's columns ([`lit_box`]), a span having nothing to draw one with.
//!
//! The pointer's icon is the row's to set, in one place: an I-beam over the text and to
//! the right of it, the hand over a link inside it, and the arrow over the gutter and on
//! leaving the row. Set only
//! when it changes, since each set is a message to the platform, and kept in one cell for
//! the whole thread: a row's own memory of it would be wrong the moment the row beside it
//! set something else.
//!
//! **Nothing inside a row may listen to `pointer_down`.** A bubbling event is measured
//! once, against the deepest listener, and every ancestor's handler is handed the same
//! data (`notes/upstream/freya.md`), so a child listening to the down would hand the row a
//! location relative to the child and the column would be wrong.
//!
//! **A row kind may not put on a handler this already sets.** freya keeps an element's
//! handlers in a map by event name, so `.on_pointer_out(..)` chained onto what [`row`]
//! returns *replaces* the row's own and nothing says so -- the icon is then never put
//! back, and the pointer leaves the listing still wearing whatever the row last set. The
//! row sets `pointer_down`, `pointer_move`, `pointer_out` and `sized`; a kind that wants
//! one of those has to be given it here.
//!
//! [`row`] is the drawing and holds nothing itself. What a row keeps is [`RowCells`], which
//! answers both ways between a column and an x and is the one thing a handler clones; the
//! marks around the text are [`marks`], and the write a caret past the pane's edge makes is
//! named there ([`bring_caret_into_view`]) rather than left inside the drawing. Each
//! handler is built by a function of its own.

use std::cell::Cell;
use std::rc::Weak;

use freya::elements::paragraph::ParagraphHolderInner;
use freya::engine::prelude::{RectHeightStyle, RectWidthStyle};

use super::*;
// Named here and not in the prelude: `chords.rs` has a `Stroke` of its own, and a glob
// carrying this one into every `ui` module would put two of them in scope.
use crate::pixels::Stroke;

thread_local! {
    /// The icon last set from a row, so a move that changes nothing sends nothing.
    static ICON: Cell<CursorIcon> = const { Cell::new(CursorIcon::Default) };
}

/// The horizontal padding every code row takes, which an absolutely placed child of the
/// row is inside: a caret placed at a column is placed from the padding's inner edge.
const ROW_PAD: f32 = 3.0;

/// The icon a row last set, for a test to ask what the pointer over something is. The
/// cell is the thread's and a test runs the app on its own thread, so this is what that
/// test's own pointer left behind.
#[cfg(test)]
pub(crate) fn icon_now() -> CursorIcon {
    ICON.with(|last| last.get())
}

/// Set the pointer's icon, if it is not that already.
fn set_icon(icon: CursorIcon) {
    ICON.with(|last| {
        if last.get() != icon {
            last.set(icon);
            Cursor::set(icon);
        }
    });
}

/// The column of a laid-out paragraph under a point `x`, `y` in its own logical
/// coordinates, in the UTF-16 units the text engine counts in. A point left of the text is
/// column 0 and one right of it is the end. `None` before the paragraph has been laid out,
/// which is a holder freya's own code unwraps and which a press cannot reach, the row
/// having nothing to press on until it is drawn.
fn caret_col(holder: &ParagraphHolder, x: f32, y: f32) -> Option<usize> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let scale = inner.scale_factor as f32;
    let at = inner
        .paragraph
        .get_glyph_position_at_coordinate(((x * scale) as i32, (y * scale) as i32));
    Some(at.position.max(0) as usize)
}

/// Where column `col` of a laid-out paragraph is, in logical pixels from its left edge: the
/// left of the character there, or the right of the last one for the column past the end,
/// and 0 for an empty text. `None` before layout, as [`caret_col`] is.
fn caret_x(holder: &ParagraphHolder, col: usize) -> Option<f32> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let scale = inner.scale_factor as f32;
    let rects = |from: usize, to: usize| {
        inner
            .paragraph
            .get_rects_for_range(from..to, RectHeightStyle::Tight, RectWidthStyle::Tight)
    };
    let x = match rects(col, col + 1).first() {
        Some(text) => text.rect.left,
        None => match col
            .checked_sub(1)
            .and_then(|before| rects(before, col).first().map(|text| text.rect.right))
        {
            Some(right) => right,
            None => 0.0,
        },
    };
    Some(x / scale)
}

/// The character under a point `x`, `y` of a laid-out paragraph, as the column it starts
/// at: what a press or the pointer is *on*, where [`caret_col`] is the nearest boundary.
/// Over the half of a character nearer the boundary after it -- the right half in a
/// left-to-right run, the left half in a right-to-left one -- the nearest boundary is that
/// one. So the character is the one ending at the boundary if its box holds `x`, one unit
/// wide or two, and the one starting there if not. Past the end it is the end, which is
/// no character. `None` before layout.
fn char_col(holder: &ParagraphHolder, x: f32, y: f32) -> Option<usize> {
    let col = caret_col(holder, x, y)?;
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let x = x * inner.scale_factor as f32;
    let holds = |from: usize| {
        inner
            .paragraph
            .get_rects_for_range(from..col, RectHeightStyle::Tight, RectWidthStyle::Tight)
            .iter()
            .any(|text| text.rect.left <= x && x < text.rect.right)
    };
    let before = (1..=2)
        .filter_map(|width| col.checked_sub(width))
        .find(|&from| holds(from));
    Some(before.unwrap_or(col))
}

/// The word around column `col` of a laid-out paragraph, as the text engine divides
/// words; `None` before layout, as [`caret_col`] is.
fn word_at(holder: &ParagraphHolder, col: usize) -> Option<(usize, usize)> {
    let inner = holder.0.borrow();
    let inner = inner.as_ref()?;
    let range = inner
        .paragraph
        .get_word_boundary(col.min(u32::MAX as usize) as u32);
    Some((range.start, range.end))
}

/// The right button's half of a `pointer_down`, as the press a context menu opens from,
/// or `None` for any other button.
///
/// freya's `on_secondary_down` is `on_pointer_down` under another name, and an element
/// keeps one handler per event, so a row that picks itself out on the down and opens a
/// menu on the down has to do both in one handler -- the later of the two would replace
/// the earlier and the press would pick out nothing (`notes/upstream/freya.md`).
fn secondary(e: Event<PointerEventData>) -> Option<Event<PressEventData>> {
    e.try_map(|data| match data {
        PointerEventData::Mouse(mouse) if mouse.button == Some(MouseButton::Right) => {
            Some(PressEventData::Mouse(mouse))
        }
        _ => None,
    })
}

/// What a row's text is: the pieces the clipboard sees, the spans the paragraph draws,
/// the part of the pane's character selection this row draws, and which runs of it are
/// links.
pub(crate) struct Text {
    /// The row's text as it is drawn, which is what the columns count and the copy takes.
    pub(crate) line: Line,
    /// The spans the paragraph draws, which add up to `line`.
    pub(crate) spans: Vec<Span<'static>>,
    /// What this row draws of the character selection.
    pub(crate) chars: RowChars,
    /// What the pane's find bar is looking for, whose matches on this row are washed under
    /// the text; `None` where no bar is open.
    pub(crate) marking: Option<Marking>,
    /// Which runs of the row's text are links, and what its names are; `None` for a row
    /// with neither.
    pub(crate) links: Option<TextLinks>,
}

/// The runs of a row's own text that are links, by their columns, in the order they are
/// drawn. A link is text and not an element, so the row's columns stay the text's own:
/// a press on one says where it was in the terms everything else speaks (`src/chars.rs`),
/// and a sweep selects across it as it selects across any other text.
///
/// `is_link` says whether a press on one is a door **now**, and the light, the hand and
/// the press are all picked by it, so none of the three offers what the others will not
/// do. A name in the source is always one; a label in the object's listing only while
/// Ctrl is held, and a plain press on it is the row's own: the row picked out, and a
/// sweep begun.
///
/// [`Text::spans`] are cut at their edges before they are drawn ([`cut_at`]), so lighting
/// a link changes a span's style and never where the spans are cut -- a boundary that
/// moved with the pointer would re-shape the row and widen the listing for good.
pub(crate) struct TextLinks {
    pub(crate) columns: Vec<Range<usize>>,
    pub(crate) is_link: Rc<dyn Fn() -> bool>,
    /// What a press on one follows. Built per row, as the menu is.
    pub(crate) follow: Rc<dyn Fn(Range<usize>)>,
    /// The colour a lit link's text and the rule under it take.
    pub(crate) lit_fg: Color,
    /// The columns of **every** name the server placed on this row, links and the places
    /// where one is defined alike: what the pointer is answered about. A superset of
    /// `columns`, and not fed to [`cut_at`] -- hovering a name changes no span's style, so
    /// it cuts the row nowhere and cannot widen the listing.
    pub(crate) names: Vec<Range<usize>>,
    /// What the pointer moving onto one of those names, or off them all, says. Built per
    /// row, as the rest is: the row knows where a name is drawn, and the pane knows what
    /// place it is.
    pub(crate) on_hover: Option<Rc<dyn Fn(Under)>>,
}

/// What a row says about the name under the pointer.
pub(crate) enum Under {
    /// The pointer is on the name at these columns of the row's own text, drawn at this
    /// box in the window's own logical pixels.
    Name(Range<usize>, Area),
    /// It is on none of them.
    Off,
    /// The row has moved under it -- a scroll, a resize, a listing redrawn -- so nothing
    /// said about it is about anything on screen any more.
    Moved,
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
/// wide -- the paragraphs its rows have lent it, for a sweep that has left the rows to ask
/// a row where a column is, the widest row drawn, which every row is floored to and which
/// is the sideways extent, how many rows it has, which is how far it scrolls and how far a
/// sweep reaches, how tall it is, which is what a reveal and a page are measured against,
/// and the nudge that puts the rows on the device pixel grid.
///
/// **The box's own `on_sized` writes the box, the height and the nudge in one call**
/// ([`Listing::measured`]), so the one measurement has the one home and a caller picks
/// how to ask for it rather than where: [`Listing::viewport`] and [`Listing::padding`]
/// read, so the scope that asked is woken by the next measurement; [`Listing::height`]
/// and the box are peeked, for the handlers and the tasks that must not subscribe.
///
/// The listing that width is held under, and how many rows it has, are **cells**, written
/// by every render of the list ([`Listing::drawing`], [`Listing::counting`]) and read
/// where they are wanted. A list is not mounted again when its listing changes -- a link
/// followed in place, a symbol previewed into the temporal tab, a companion file
/// switching, the worker answering -- and this context is made once, so a key stored at
/// the mount would go on naming the listing the list started on, for which [`Widest`]
/// answers nothing. The same holds a step harder for the sweep's autoscroll, a task that
/// outlives the render that spawned it and reads both at every tick.
#[derive(Clone)]
pub(crate) struct Listing {
    pub(crate) controller: ScrollController,
    pub(crate) bounds: Rc<Cell<Area>>,
    pub(crate) texts: Rc<RefCell<HashMap<usize, RowText>>>,
    pub(crate) widest: Widest,
    key: Rc<Cell<u64>>,
    /// How many rows the list is drawing: see [`Listing::counting`].
    rows: Rc<Cell<usize>>,
    /// How far down the rows are pushed to sit on the device pixel grid: see
    /// [`Listing::padding`].
    nudge: State<f32>,
    /// How tall the list is: see [`Listing::viewport`]. The `VirtualScrollView` measures
    /// itself but keeps the answer, so the box around it is what is measured.
    viewport: State<f32>,
}

/// A row's laid-out paragraph and where it starts, lent to the list by the row as it
/// renders: what answers a column for an x on a row the pointer is not over. Written
/// afresh by every render of the row, and the paragraph held **weakly**: a strong hold
/// would keep a shaped paragraph for every row the reader has scrolled past. A row the
/// list has stopped building drops its paragraph, so its entry answers nothing and the
/// next render of the list drops it ([`Listing::drawing`]); no reach asks one meanwhile,
/// a sweep only asking about rows on screen.
#[derive(Clone)]
pub(crate) struct RowText {
    holder: Weak<RefCell<Option<ParagraphHolderInner>>>,
    text_x: Rc<Cell<f32>>,
}

#[cfg(test)]
impl Listing {
    /// One with no box behind it: nothing measured, nothing lent, and a scroll nobody
    /// moves. For a test that calls what a row's text is built from without drawing a
    /// list to build it in.
    pub(crate) fn detached() -> Listing {
        Listing::new(
            ScrollController::new(0, 0, Vec::new()),
            Widest::detached(),
            State::create(0.0),
            State::create(0.0),
        )
    }
}

impl Listing {
    /// A fresh list, with nothing lent yet and no listing drawn. The two states are
    /// handed in rather than made here, this being called once from inside a hook's
    /// closure.
    pub(crate) fn new(
        controller: ScrollController,
        widest: Widest,
        nudge: State<f32>,
        viewport: State<f32>,
    ) -> Self {
        Listing {
            controller,
            bounds: Rc::new(Cell::new(Area::zero())),
            texts: Rc::new(RefCell::new(HashMap::new())),
            widest,
            key: Rc::new(Cell::new(0)),
            rows: Rc::new(Cell::new(0)),
            nudge,
            viewport,
        }
    }

    /// The box was laid out as `area`, which is everything the list learns from being
    /// measured: where its top is, which is what the rows are pushed off, how tall it is,
    /// and the box a sweep is judged against. The grid is taken at the render and not
    /// here, so the handler asks nothing of the runtime.
    pub(crate) fn measured(&self, grid: Grid, area: Area) {
        let (mut nudge, mut viewport) = (self.nudge, self.viewport);
        viewport.set_if_modified(area.height());
        nudge.set_if_modified(grid.nudge(area.min_y()));
        self.bounds.set(area);
    }

    /// The padding that puts the rows on the device pixel grid, read as the box's top
    /// padding: whatever fraction the bars, tabs and fonts above a listing add up to, its
    /// rows are washed and highlighted as whole pixels, so two rows' washes meet on an
    /// edge instead of each fading into the other over the pixel they share. A read: the
    /// box re-renders as it lands.
    pub(crate) fn padding(&self) -> Gaps {
        Gaps::new(*self.nudge.read(), 0.0, 0.0, 0.0)
    }

    /// That padding as it is, for a handler, which subscribes nothing.
    fn nudge(&self) -> f32 {
        *self.nudge.peek()
    }

    /// How tall the list is, which is what a reveal, a page and the scroll's extent are
    /// measured against: the state itself, for the hooks that must **read** it. Nothing
    /// is known before the first layout, so a reveal in an unmeasured pane keeps what it
    /// owes ([`reveal_row`]) and only a read is woken when the measurement lands. A
    /// reveal made outside a render peeks this same state per call, the pane having been
    /// zero tall when the closure was built.
    pub(crate) fn viewport(&self) -> State<f32> {
        self.viewport
    }

    /// That height as it is, for a handler and for a task, which subscribe nothing.
    pub(crate) fn height(&self) -> f32 {
        *self.viewport.peek()
    }

    /// The listing the list is drawing, told to this by every render of the list: what
    /// its rows are floored to and what a sweep's sideways extent is asked under.
    ///
    /// Also where the lent paragraphs are swept, this being the one call every list makes
    /// on every render: an entry whose row has gone answers nothing, and nothing else
    /// removed one, so the map kept an entry per row ever built -- a `Weak` pinning an
    /// allocation and a cell of its own for every row scrolled past, for the life of the
    /// pane. A row unmounts after the render that stopped building it, so what is swept
    /// is a render behind and the map holds the rows on screen and the last render's.
    pub(crate) fn drawing(&self, listing: u64) {
        self.key.set(listing);
        self.texts
            .borrow_mut()
            .retain(|_, text| text.holder.strong_count() > 0);
    }

    /// That listing, as a row and a sweep ask for it.
    pub(crate) fn key(&self) -> u64 {
        self.key.get()
    }

    /// How many rows the list is drawing, told to this by every render of it as the
    /// listing is: what a sweep beyond the rows reaches over, and what the scroll can
    /// go as far as.
    pub(crate) fn counting(&self, rows: usize) {
        self.rows.set(rows);
    }

    /// That count, as a sweep asks for it.
    pub(crate) fn rows(&self) -> usize {
        self.rows.get()
    }

    /// Where the rows actually sit, in pixels down the listing: the controller's offset
    /// held to [`scroll_extent`], which is **not** always what the controller says.
    /// Turning an uncorrected one into a row put the sweep past the end of the listing
    /// with the pointer in the middle of the rows, and every row from the press to the
    /// last was picked out.
    pub(crate) fn scrolled(&self) -> f32 {
        let (_, scrolled) = <(i32, i32)>::from(self.controller);
        let extent = scroll_extent(self.rows(), code_row_height(), self.height());
        (scrolled as f32).clamp(-extent, 0.0)
    }

    /// Where row 0's top is, relative to the box's top: the nudge that puts the rows on
    /// the grid, plus the scroll, which counts down from zero.
    fn rows_top(&self) -> f32 {
        self.nudge() + self.scrolled()
    }

    /// The column at window x `x` on row `row`, off the paragraph the row lent: 0 for a
    /// row with no text, for an x left of its text, and for a row the list has stopped
    /// building, whose paragraph went with it; the end for an x right of the text.
    pub(crate) fn column_at(&self, row: usize, x: f32) -> usize {
        let texts = self.texts.borrow();
        let Some(text) = texts.get(&row) else {
            return 0;
        };
        let x = x - text.text_x.get();
        if x < 0.0 {
            return 0;
        }
        let Some(holder) = text.holder.upgrade() else {
            return 0;
        };
        caret_col(&ParagraphHolder(holder), x, 0.0).unwrap_or(0)
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
    /// Whether the row reports its width to the listing's [`Widest`]. A separator does
    /// not: its rule fills the row, so it would report the row plus its gutter and the
    /// widest would grow by a gutter's width every layout, without end.
    pub(crate) measured: bool,
}

/// The cells and states a row keeps for as long as it is mounted, and the two questions
/// they answer: which column a place on the row is, and where a column is drawn. A handler
/// clones one of these rather than four cells of its own.
#[derive(Clone)]
struct RowCells {
    /// The laid-out paragraph, for the pointer to be answered in columns. One per row, as
    /// freya's own editor keeps one per line.
    holder: State<ParagraphHolder>,
    /// Where the row was laid out, and where its paragraph was, so a pointer location
    /// relative to the row can be made relative to the text. Cells and not states:
    /// nothing renders from them, and the difference between the two is scroll-invariant.
    row_x: Rc<Cell<f32>>,
    text_x: Rc<Cell<f32>>,
    /// Where the row's top is, which is what the hover box is placed against. Its own
    /// cell rather than a corner of `row_x`'s: this one moves with every scroll, and the
    /// move is what says a box drawn against it is about a place that has gone.
    row_y: Rc<Cell<f32>>,
    /// Which of the row's names the pointer is on. A cell and not a state: hovering a
    /// name changes nothing this row draws, and only the box is redrawn for it.
    named: Rc<Cell<Option<usize>>>,
    /// Whether the paragraph has been laid out, which is when the holder can answer where
    /// a column is: the caret is drawn from the render after that.
    laid: State<bool>,
    /// Whether the row has text at all. A row without answers no column.
    has_text: bool,
}

/// A row's cells, made once as it mounts.
fn use_row_cells(has_text: bool) -> RowCells {
    RowCells {
        holder: use_state(ParagraphHolder::default),
        row_x: use_hook(|| Rc::new(Cell::new(0.0f32))),
        text_x: use_hook(|| Rc::new(Cell::new(0.0f32))),
        row_y: use_hook(|| Rc::new(Cell::new(f32::NAN))),
        named: use_hook(|| Rc::new(Cell::new(None))),
        laid: use_state(|| false),
        has_text,
    }
}

impl RowCells {
    /// The column `probe` finds at `at`, a location relative to the row, and `left` for
    /// one left of the text.
    fn column(
        &self,
        at: CursorPoint,
        left: Option<usize>,
        probe: fn(&ParagraphHolder, f32, f32) -> Option<usize>,
    ) -> Option<usize> {
        let x = self.x_into_text(at)?;
        match x < 0.0 {
            true => left,
            false => probe(&self.holder.read(), x, at.y as f32),
        }
    }

    /// The column a press lands on: `None` left of the text, which is the gutter and
    /// picks rows out alone. Either button.
    fn pressed_column(&self, at: CursorPoint) -> Option<usize> {
        self.column(at, None, caret_col)
    }

    /// The column the pointer reaches: 0 left of the text, where the line starts. A sweep
    /// off that edge carries the run there.
    fn swept_column(&self, at: CursorPoint) -> Option<usize> {
        self.column(at, Some(0), caret_col)
    }

    /// The character the pointer is on, as the column it starts at: what a link or a name
    /// is looked for under, where the two above are the boundary a caret goes to. Over a
    /// name's last character the boundary can be past the name, and the pointer is still
    /// on it. `None` left of the text, which is on no character.
    fn pointed_column(&self, at: CursorPoint) -> Option<usize> {
        self.column(at, None, char_col)
    }

    /// How far into the row's text `at` is, negative left of where the text begins. `None`
    /// for a row without text, which answers no column at all. The three questions above
    /// and the pointer's icon are this one arithmetic: the row-relative x less the paragraph's x
    /// within the row, both taken from `on_sized` and so scroll-invariant.
    fn x_into_text(&self, at: CursorPoint) -> Option<f32> {
        self.has_text
            .then(|| at.x as f32 - (self.text_x.get() - self.row_x.get()))
    }

    /// Where column `col` of a row `units` long is, from the row's padded edge, once the
    /// paragraph has been laid out and the holder can say.
    fn column_x(&self, col: usize, units: usize) -> Option<f32> {
        (*self.laid.read()).then_some(())?;
        let x = caret_x(&self.holder.read(), col.min(units))?;
        Some(self.text_x.get() - self.row_x.get() - ROW_PAD + x)
    }

    /// The device pixel span columns `from..to` of a row `units` long cover, once the
    /// paragraph is laid out: [`None`] before that, and for a span that covers nothing.
    ///
    /// The one place the rule for a box over a run of a row's own text is written -- the
    /// selection's, the lit link's and every find hit's -- so all three are on the grid the
    /// same way and all three answer nothing before layout.
    fn span(&self, grid: Grid, from: usize, to: usize, units: usize) -> Option<Stroke> {
        let (from, to) = (from.min(units), to.min(units));
        let (left, right) = (self.column_x(from, units)?, self.column_x(to, units)?);
        (right > left).then(|| grid.span(left, right))
    }

    /// Lend the row's paragraph to the list, for a sweep that has left the rows to ask
    /// this row where a column is. A row with no text lends nothing.
    fn lend(&self, listing: &Listing, row: usize) {
        if !self.has_text {
            return;
        }
        let lent = RowText {
            holder: Rc::downgrade(&self.holder.read().0),
            text_x: self.text_x.clone(),
        };
        listing.texts.borrow_mut().insert(row, lent);
    }
}

impl TextLinks {
    /// Which of the links column `column` is in, and `None` where it is in none.
    fn at(&self, column: Option<usize>) -> Option<usize> {
        let column = column?;
        self.columns.iter().position(|link| link.contains(&column))
    }
}

/// A row's links as its handlers share them, once the row's text has been moved into its
/// paragraph; `None` for a row with none.
type RowLinks = Option<Rc<TextLinks>>;

/// Whether a press on one of the row's links is a door *now*: the links' own answer. The
/// light and the hand are both picked by it, so neither can offer what a press will not do.
fn open(links: &RowLinks) -> bool {
    links.as_ref().is_some_and(|links| (links.is_link)())
}

/// What the right button opens on a row, handed the press and the column the pointer was
/// over. The column is what a question about the name under it needs and only the row
/// knows: `None` in the gutter, and on a row with no text at all.
///
/// Named once, so a menu builder cannot copy the type without the rule about its second
/// argument.
pub(crate) type RowMenu = Rc<dyn Fn(Event<PressEventData>, Option<usize>)>;

/// The row: its chrome, what comes `before` the text -- a gutter, an address, a line
/// number -- the `text` where the row has any, and the `menu` the right button opens.
pub(crate) fn code_row(
    chrome: Chrome,
    before: Vec<Element>,
    mut text: Option<Text>,
    menu: Option<RowMenu>,
) -> Rect {
    let marked = use_consume::<Marked>().0;
    let shift = use_consume::<Shift>().0;
    let listing = use_consume::<Listing>();
    let cells = use_row_cells(text.is_some());
    // Which of the links in the row's own text the pointer is over. Written with
    // `set_if_modified`, so a row is drawn again when the pointer crosses a link's edge
    // and not as it moves along one.
    let mut over = use_state(|| None::<usize>);
    let alt = use_consume::<Alt>().0;
    let grid = pixel_grid();

    // The row's links and its names, taken out of `text` before its spans are moved into
    // the paragraph below, since the handlers need them.
    let links: RowLinks = text
        .as_mut()
        .and_then(|text| text.links.take())
        .map(Rc::new);
    let on_hover = links.as_ref().and_then(|links| links.on_hover.clone());
    // Built only for a row that has names, which is a source row and no other.
    let tell = links
        .as_ref()
        .filter(|links| !links.names.is_empty())
        .map(|links| tell_hover(&cells, links.clone()));
    let tell_out = tell.clone();
    // The widest row of the listing the list is drawing now, and the listing itself: this
    // row's floor and what it reports its own width under, read once so the two agree.
    let (widest, listing_key) = (listing.widest, listing.key());
    cells.lend(&listing, chrome.row);

    // The run of the row's own text under the pointer, drawn as a link where a press on
    // it would be a door: the link's own rule, which the pointer's icon is picked by too,
    // and Alt, which says no to every link in every pane.
    //
    // Both are read only while the pointer is on a run, so a row nobody is pointing at is
    // on neither modifier's list -- and read whether or not the answer is yes, or the row
    // would never be drawn again when the modifier came up.
    let lit = over().filter(|_| open(&links) && !*alt.read());
    let columns = lit
        .and_then(|lit| links.as_ref()?.columns.get(lit))
        .cloned();
    let lit_fg = links
        .as_ref()
        .map_or(palette().name_hover_fg, |links| links.lit_fg);
    let drawn = text.map(|text| {
        let units = text.line.units();
        let (selected, caret) = marks(&cells, &listing, grid, text.chars, units);
        let wash = lit_box(&cells, grid, columns.as_ref(), units, lit_fg);
        let finds = text
            .marking
            .as_ref()
            .map(|marking| marking.hits(&text.line))
            .unwrap_or_default();
        let matched = found(&cells, grid, &finds, units);
        (
            wash,
            matched,
            selected,
            text_paragraph(&cells, text, &links, columns.as_ref(), lit_fg),
            caret,
        )
    });

    let el = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        // As wide as the pane or the listing's widest row, whichever is more, and what
        // it holds measured under it -- which is what lets the list scroll sideways to a
        // long row while the wash still runs the whole width. The width reported is the
        // content's, not the laid-out one: see `ui/width.rs`.
        .width(Widest::row_width(widest.floor(listing_key), listing_key))
        .on_sized(on_measured(
            &cells,
            on_hover,
            widest,
            listing_key,
            chrome.measured,
        ))
        .height(Size::px(code_row_height()))
        // Horizontally only: the gutter's lines run to the row's own top and bottom
        // edges, and padding there would break every line in the column once per row.
        .padding(Gaps::new_symmetric(0.0, ROW_PAD))
        .assembly_font()
        // Nothing of this row's own under the pointer: it is lit by the other pane's run,
        // where it is the same place, and by this pane's, where it is in it.
        .background(row_background(chrome.paired.is_some(), chrome.wash))
        .maybe(chrome.paired.is_some_and(Edges::any), |el| {
            el.border(pair_border(chrome.paired.unwrap_or_default()))
        })
        .on_pointer_down(on_down(&cells, &chrome, &links, menu, marked, shift, alt))
        .on_pointer_move(on_move(&cells, &chrome, &links, tell, over, marked, alt))
        .on_pointer_out(move |_| {
            over.set_if_modified(None);
            if let Some(tell) = &tell_out {
                tell(None);
            }
            set_icon(CursorIcon::Default);
        })
        .children(before);

    // The lit link's box, the find bar's matches and the selection before the paragraph
    // in the tree, so all three are painted under the text -- and **always there**, as is
    // the caret's slot: freya matches siblings by position, so a rect appearing before the
    // paragraph on the press would move the paragraph along one and remount it, between
    // the down and the up of every press.
    match drawn {
        Some((wash, matched, selected, paragraph, caret)) => el
            .child(wash)
            // Under the selection: the match the pane is on wears both, and the one it
            // is on has to read as the selected one.
            .child(matched)
            .child(selected)
            .child(paragraph)
            .child(caret),
        None => el,
    }
}

/// Say which name the pointer is on, where it is drawn, and say it only when the answer
/// has changed: a move along one name arrives many times over. A column goes in: the row
/// knows where its names are drawn, and the pane knows what place one is.
fn tell_hover(cells: &RowCells, links: Rc<TextLinks>) -> Rc<dyn Fn(Option<usize>)> {
    let cells = cells.clone();
    Rc::new(move |column: Option<usize>| {
        let on =
            column.and_then(|column| links.names.iter().position(|name| name.contains(&column)));
        // Off a name, only the crossing is worth saying: that there is nothing under the
        // pointer stays true however far it moves. **On** one, every move is said, a move
        // being what puts the wait for it back to the beginning (`src/ui/hovering.rs`).
        let crossed = cells.named.replace(on) != on;
        if !crossed && on.is_none() {
            return;
        }
        let Some(tell) = links.on_hover.as_ref() else {
            return;
        };
        let Some(columns) = on.and_then(|on| links.names.get(on)).cloned() else {
            return tell(Under::Off);
        };
        let holder = cells.holder.read();
        let edge = |column| caret_x(&holder, column).map(|x| cells.text_x.get() + x);
        // A row whose paragraph is not laid out yet answers no column, and a box placed
        // against nothing would be drawn in the window's corner.
        let (Some(left), Some(right)) = (edge(columns.start), edge(columns.end)) else {
            return tell(Under::Off);
        };
        tell(Under::Name(
            columns,
            Area::new(
                (left, cells.row_y.get()).into(),
                Size2D::new(right - left, code_row_height()),
            ),
        ));
    })
}

/// A box of the row's own over `span`, from `top` down `height`: what the selection, the
/// caret, the lit link and a find hit are each drawn as. Absolutely placed inside the row
/// and answering no pointer, a mark being a picture and not a control.
///
/// The colour is the caller's, the lit link taking [`link_chrome`]'s rather than one of its
/// own.
fn box_over(span: Stroke, top: f32, height: f32) -> Rect {
    rect()
        .interactive(false)
        .position(Position::new_absolute().left(span.near).top(top))
        .width(Size::px(span.thick))
        .height(Size::px(height))
}

/// The two marks a row draws around its text: the selection's, painted under it, and the
/// caret's, over it. Both are always drawn -- [`nothing`] where there is no mark -- since
/// freya matches siblings by position (see the children at the foot of [`row`]).
///
/// Neither is interactive: a mark answers no press and no move.
fn marks(
    cells: &RowCells,
    listing: &Listing,
    grid: Grid,
    chars: RowChars,
    units: usize,
) -> (Rect, Rect) {
    // The highlight: a rect of the row's own from the first column's x to the last's, the
    // row's whole height, on the grid -- so one row's meets the next's on a pixel edge. An
    // empty row inside the run shows as a stub, or the run would read as broken there.
    let selected = chars.highlight.and_then(|(from, to)| {
        // The stub is this mark's own rule and the only thing it does not share with the
        // other two: an empty row has one column, so `span` answers nothing for it.
        let span = match units == 0 {
            true => {
                let left = cells.column_x(0, units)?;
                grid.span(left, left + code_row_height() / 4.0)
            }
            false => cells.span(grid, from, to, units)?,
        };
        Some(box_over(span, 0.0, code_row_height()).background(palette().text_select_bg))
    });

    // The caret, where the run's lead is on this row and no sweep has picked characters
    // out: a stroke of the row's own, on the device pixel grid, where the engine's would
    // sit on the glyph's fractional edge and two pixels wide. Drawn over a selection too,
    // at its lead: it is where the next key moves from.
    let at = chars.cursor.and_then(|col| cells.column_x(col, units));
    if let Some(x) = at {
        bring_caret_into_view(listing, cells.row_x.get(), cells.row_x.get() + ROW_PAD + x);
    }
    let caret = at.map(|x| {
        // From the column rightward, so a caret on column 0 starts where the text does.
        let stroke = grid.span(x, x + CARET_WIDTH);
        box_over(stroke, 0.0, code_row_height()).background(palette().caret_fg)
    });

    (
        selected.unwrap_or_else(nothing),
        caret.unwrap_or_else(nothing),
    )
}

/// A caret at window x `at`, on a row whose own left edge is `row_left`, brought into the
/// pane's sight: the keyboard walks the caret along a row longer than the pane, and the
/// pane has to follow. From a task and not the render, since a scroll is a write; the list
/// answers with a layout, whose `on_sized` moves `visible`, and a caret then inside asks
/// for nothing more.
///
/// The one write drawing a row makes.
fn bring_caret_into_view(listing: &Listing, row_left: f32, at: f32) {
    let seen = listing.bounds.get();
    if seen.width() <= 0.0 {
        return;
    }
    let shove = if at < seen.min_x() {
        Some(seen.min_x() - at + CARET_INSET)
    } else if at + 1.0 > seen.max_x() {
        Some(seen.max_x() - at - 1.0 - CARET_INSET)
    } else {
        None
    };
    let Some(shove) = shove.filter(|shove| shove.abs() >= 1.0) else {
        return;
    };
    let mut controller = listing.controller;
    // Nothing to bring in from the left of the row's own start.
    let shove = shove.min(-(row_left - seen.min_x()).min(0.0));
    spawn(async move {
        let (x0, _) = <(i32, i32)>::from(controller);
        let target = (x0 + shove.round() as i32).min(0);
        if target != x0 {
            controller.scroll_to_x(target);
        }
    });
}

/// The box a lit run of the row's own text wears: [`link_chrome`], the one answer to what
/// a lit link looks like, placed by the run's columns and inside the row's height
/// ([`link_box_height`]), in the link's own lit colour. So a name in the source, a label in
/// the object's listing and an operand of an instruction are all lit one way.
///
/// A rect of the row's own, as the selection's is, because a span carries no box: freya's
/// text styles have a colour, a weight and a decoration and nothing to draw one with.
/// Nothing until the paragraph is laid out, which is when the row can say where a column
/// is.
fn lit_box(
    cells: &RowCells,
    grid: Grid,
    columns: Option<&Range<usize>>,
    units: usize,
    lit_fg: Color,
) -> Rect {
    let Some(columns) = columns else {
        return nothing();
    };
    let Some(span) = cells.span(grid, columns.start, columns.end, units) else {
        return nothing();
    };
    link_chrome(
        box_over(span, LINK_BOX_INSET, link_box_height()),
        Some(lit_fg),
    )
}

/// What the find bar matched on this row, washed under the text: one rect per match, in
/// [`find_bg`](Palette::find_bg), placed exactly as the selection is.
///
/// **One slot holding however many**, rather than a rect each among the row's own
/// children: freya matches siblings by position, and a count that changes with what is
/// typed would move the paragraph along and remount it (see the children at the foot of
/// [`row`]). The slot itself is always there, empty when nothing matched.
fn found(cells: &RowCells, grid: Grid, finds: &[Range<usize>], units: usize) -> Rect {
    let washes: Vec<Element> = finds
        .iter()
        .filter_map(|columns| cells.span(grid, columns.start, columns.end, units))
        .map(|span| {
            box_over(span, 0.0, code_row_height())
                .background(palette().find_bg)
                .into_element()
        })
        .collect();
    nothing().children(washes)
}

/// The row's text as one paragraph. `lit` is the columns of the run of the row's own text
/// that is drawn as a link, the box around it being the row's ([`lit_box`]).
fn text_paragraph(
    cells: &RowCells,
    text: Text,
    links: &RowLinks,
    lit: Option<&Range<usize>>,
    lit_fg: Color,
) -> Paragraph {
    let (text_x, mut laid) = (cells.text_x.clone(), cells.laid);
    let columns = links.as_ref().map_or(&[][..], |links| &links.columns[..]);
    paragraph()
        .max_lines(1)
        // The row's whole height, so the highlight -- which the engine expands to the
        // paragraph's box -- runs from one row into the next with no gap.
        .height(Size::fill())
        .holder(cells.holder.read().clone())
        .on_sized(move |e: Event<SizedEventData>| {
            text_x.set(e.area.min_x());
            laid.set_if_modified(true);
        })
        .vertical_align(VerticalAlign::Center)
        .spans_iter(light(cut_at(text.spans, columns), lit, lit_fg).into_iter())
}

/// The row's `on_sized`: where it was laid out, whether it has moved out from under a box
/// drawn against it, and its width reported to the listing's [`Widest`].
fn on_measured(
    cells: &RowCells,
    on_hover: Option<Rc<dyn Fn(Under)>>,
    widest: Widest,
    listing: u64,
    measured: bool,
) -> impl FnMut(Event<SizedEventData>) + 'static {
    let cells = cells.clone();
    move |e: Event<SizedEventData>| {
        cells.row_x.set(e.area.min_x());
        // A row that has moved -- a scroll, a resize, a listing redrawn -- takes any box
        // drawn against it with it. Watched here rather than at the wheel: a
        // `VirtualScrollView` stops the wheel event it acted on, so the pane never sees
        // the one that matters, and this covers the keyboard, the sweep's autoscroll and
        // a font change as well.
        if cells.row_y.replace(e.area.min_y()) != e.area.min_y() && cells.named.take().is_some() {
            if let Some(tell) = on_hover.as_ref() {
                tell(Under::Moved);
            }
        }
        if measured {
            widest.note(listing, e.inner_sizes.width);
        }
    }
}

/// **What a press on a row's text means**, once the column it landed on is known: one
/// press a caret there, two the word under it, three or more the row's whole text, as the
/// text engine divides them.
///
/// `word` is asked only where the answer turns on it, and answers for the row as it is
/// laid out now; a press on a row with no word boundary to give -- one not laid out yet
/// -- is the caret the single press would have been.
fn pressed(
    presses: PressEventType,
    col: usize,
    word: impl FnOnce(usize) -> Option<(usize, usize)>,
) -> Press {
    match presses {
        PressEventType::Double => word(col)
            .map(|(from, to)| Press::Span(from, to))
            .unwrap_or(Press::At(col)),
        PressEventType::Triple | PressEventType::Quadruple => Press::Span(0, usize::MAX),
        PressEventType::Single => Press::At(col),
    }
}

/// The row's `on_pointer_down`: a link followed, a run started, or the menu.
///
/// The *down* and not the press: a drag is over by the time a press fires, so a selection
/// swept out with the button held has to begin as it goes down. The right button's down is
/// the menu, **in the same handler**: `on_secondary_down` is `on_pointer_down` under
/// another name and would replace this one ([`secondary`]).
fn on_down(
    cells: &RowCells,
    chrome: &Chrome,
    links: &RowLinks,
    menu: Option<RowMenu>,
    marked: State<Marks>,
    shift: State<bool>,
    alt: State<bool>,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let (cells, links) = (cells.clone(), links.clone());
    let (pane, row, file) = (chrome.pane, chrome.row, chrome.file.clone());
    move |e: Event<PointerEventData>| {
        if e.button() == Some(MouseButton::Left) {
            let at = cells.pressed_column(e.element_location());
            let on = cells.pointed_column(e.element_location());
            // freya counts the presses in one place, and **asking is counting**: a press
            // it was not asked about is one the next reads as a double. So it is asked
            // exactly once, whatever the press turns out to be.
            let presses = EventsCombos::pressed(e.global_location());
            // A link is followed on a single press with nothing held, and only while it
            // is a door: two presses on a name are what take the word, Alt says this one
            // is not a door, and a label is one only under Ctrl -- without which the
            // press is the row's, as it is over any other text.
            let followed = links.as_ref().and_then(|links| {
                let link = links.columns.get(links.at(on)?)?;
                let door = presses == PressEventType::Single && !*alt.peek() && (links.is_link)();
                door.then(|| (links, link.clone()))
            });
            if let Some((links, link)) = followed {
                // And it picks no line out: the press is the question and not a place in
                // the file.
                (links.follow)(link);
                return;
            }
            let press =
                at.map(|col| pressed(presses, col, |col| word_at(&cells.holder.read(), col)));
            mark_press(marked, *shift.peek(), pane, file.clone(), row, press);
            return;
        }
        // The column before the event is turned into a press: what the menu is asked
        // about is the character the pointer was on.
        let at = cells.pointed_column(e.element_location());
        let Some(e) = secondary(e) else {
            return;
        };
        if let Some(menu) = &menu {
            menu(e, at);
        }
    }
}

/// The row's `on_pointer_move`: the sweep out to the column under the pointer, which name
/// is under it, and the pointer's icon -- an I-beam over the text and right of it, the
/// hand over a link, the arrow over the gutter.
///
/// Every move and not `pointer_over`, which fires once on entry: a sweep along a row has
/// to follow the pointer.
fn on_move(
    cells: &RowCells,
    chrome: &Chrome,
    links: &RowLinks,
    tell: Option<Rc<dyn Fn(Option<usize>)>>,
    mut over: State<Option<usize>>,
    marked: State<Marks>,
    alt: State<bool>,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let (cells, links) = (cells.clone(), links.clone());
    let (pane, row) = (chrome.pane, chrome.row);
    move |e: Event<PointerEventData>| {
        let at = e.element_location();
        mark_drag(marked, pane, row, cells.swept_column(at));
        let column = cells.pointed_column(at);
        // Neither the link under the pointer nor the name is answered while a selection
        // is being swept out: a drag along a line would otherwise light every name it
        // passed under. The name is said whether it is a link or not -- a name where one
        // is defined is not a link and is still something to ask the server about -- but
        // under the same guard, for the same reason.
        let sweeping = dragging(marked, pane);
        // Alt says a press on a link is not a door, so what is under the pointer is text.
        // The link under it is kept all the same: the light asks Alt for itself, so the
        // link lights again as Alt comes up, with no move to say so.
        let alt = *alt.peek();
        let hovered = (!sweeping).then(|| links.as_ref()?.at(column)).flatten();
        over.set_if_modified(hovered);
        if let Some(tell) = &tell {
            tell(if sweeping || alt { None } else { column });
        }
        let on_text = cells.x_into_text(at).is_some_and(|x| x >= 0.0);
        // The hand over a link, and only while a press on it would be a door: the link's
        // own rule, which is what lights it, so the two cannot disagree.
        set_icon(if hovered.is_some() && !alt && open(&links) {
            CursorIcon::Pointer
        } else if on_text {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        });
    }
}

/// `spans` cut so that every run in `links` is exactly one span of it, splitting a span
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
///
/// **`links` must be ascending and must not overlap.** The spans are walked left to right
/// and so are the edges, once for the whole row rather than once per span, so an edge
/// behind the one before it is passed over and the span it fell in is left whole -- the
/// same fallback a cut inside a character takes, and never a lost piece of text. The order
/// is `Links::of`'s, which sorts a file's names by line and column (`src/links.rs`) and
/// which `Named::linked` keeps (`src/ui/source_row.rs`); a label in an object's listing is
/// one run and the whole row (`src/ui/section_view.rs`), and an instruction has at most one
/// (`src/ui/assembly.rs`).
pub(crate) fn cut_at(spans: Vec<Span<'static>>, links: &[Range<usize>]) -> Vec<Span<'static>> {
    if links.is_empty() {
        return spans;
    }
    let mut cut = Vec::with_capacity(spans.len());
    let mut column = 0;
    // Every edge of every link, in the order they are drawn: one cursor carried across the
    // spans, so each link is read once for the row.
    let mut edges = links
        .iter()
        .flat_map(|link| [link.start, link.end])
        .peekable();
    for span in spans {
        let units = chars::units(&span.text);
        let (from, to) = (column, column + units);
        column = to;
        // What is behind this span is behind every span after it.
        while edges.next_if(|edge| *edge <= from).is_some() {}
        // Nothing begins or ends inside it, so there is nothing to cut. What is past it is
        // left where it is, for the span it does fall in.
        if edges.peek().is_none_or(|edge| *edge >= to) {
            cut.push(span);
            continue;
        }
        // Where inside this span a link begins or ends. Two links that touch state the one
        // edge twice, and the span is cut there once.
        let mut last = from;
        let inside = std::iter::from_fn(|| loop {
            let edge = edges.next_if(|edge| *edge < to)?;
            if edge != last {
                last = edge;
                return Some(edge - from);
            }
        });
        let mut at = 0;
        for edge in inside.chain(std::iter::once(units)) {
            let Some(piece) = chars::slice_of(&span.text, at..edge) else {
                continue;
            };
            at = edge;
            cut.push(Span {
                text: std::borrow::Cow::Owned(piece.to_owned()),
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

/// `spans` with the run at `columns` drawn as a link under the pointer, and unchanged
/// where nothing is.
///
/// The colour is all a span can say: the wash, the corner and the rule under it are the
/// row's ([`lit_box`]), a text style having nothing to draw a box with.
///
/// Every span the link covers is drawn as one, and no span is ever cut here: [`cut_at`]
/// has already made sure none straddles a link's edge, so each is wholly inside the run
/// or wholly outside it. A boundary that moved with the pointer would re-shape the row,
/// and the widest row a listing has drawn only ever grows.
///
/// More than one span where a link crosses a colour boundary, which a name the server
/// placed may do and a name taken from a colour run never could.
fn light(
    spans: Vec<Span<'static>>,
    columns: Option<&Range<usize>>,
    lit_fg: Color,
) -> Vec<Span<'static>> {
    let Some(columns) = columns else {
        return spans;
    };
    let mut column = 0;
    spans
        .into_iter()
        .map(|span| {
            let units = chars::units(&span.text);
            let at = column;
            column += units;
            match at >= columns.start && column <= columns.end && at < column {
                true => span.color(lit_fg),
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

/// How often the view moves while a sweep is held past an edge: a row up or down, and
/// a row's height sideways, each time.
const AUTOSCROLL_TICK: Duration = Duration::from_millis(40);

/// Whether `pane`'s run is being swept: the button down on it.
fn dragging(marked: State<Marks>, pane: Pane) -> bool {
    marked
        .peek()
        .of(pane)
        .as_ref()
        .is_some_and(|picked| picked.dragging)
}

/// Where a sweep at `at`, a window location, reaches once it has left the rows of
/// `listing`: [`beyond`], with the rows' top worked out from where the rows sit
/// ([`Listing::rows_top`]), and the column off the paragraph the row lent. `None` while
/// the pointer is over a row, which answers for itself.
fn reach(listing: &Listing, at: CursorPoint) -> Option<Caret> {
    let area = listing.bounds.get();
    let bounds = Bounds {
        left: area.min_x(),
        top: area.min_y(),
        right: area.max_x(),
        bottom: area.max_y(),
    };
    let reached = beyond(
        bounds,
        listing.rows_top(),
        code_row_height(),
        listing.rows(),
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
/// nothing while the pointer is over a row, which answers for itself.
///
/// Held past an edge of the box, the sweep **scrolls the view**: a task moves it every
/// [`AUTOSCROLL_TICK`] towards the pointer -- a row up or down, a row's height sideways --
/// and reaches the run out to what came in, for as long as the button is down and the
/// pointer stays past an edge; the pointer's last place is kept in a cell the handler
/// writes and the task reads, since nothing arrives from a pointer that is not moving. A
/// hook, for the cells to outlive the handler a render makes afresh; one task at a time,
/// the flag says.
///
/// **The rows and the key are the render's**, and neither is carried into the task: both
/// are the [`Listing`]'s own cells, written by every render of the list and read at the
/// tick. A task outlives the render that spawned it and the sweep that started it, so a
/// task holding the count it began with goes on scrolling a listing the pane has stopped
/// drawing, and one holding the key finds `Widest` answering nothing for a listing it no
/// longer holds -- a sideways extent of zero and a pane pinned to its left edge.
pub(crate) fn use_sweep_beyond(
    marked: State<Marks>,
    pane: Pane,
    listing: Listing,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let last = use_hook(|| Rc::new(Cell::new(None::<CursorPoint>)));
    let running = use_hook(|| Rc::new(Cell::new(false)));

    move |e: Event<PointerEventData>| {
        let at = e.global_location();
        last.set(Some(at));
        if let Some(caret) = reach(&listing, at) {
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
                let (x, _) = <(i32, i32)>::from(listing.controller);
                // Where the rows are and not where the controller says, so a tick steps
                // from what the reader can see (`Listing::scrolled`).
                let y = listing.scrolled() as i32;
                let mut controller = listing.controller;
                if down != 0 {
                    let extent = scroll_extent(listing.rows(), code_row_height(), area.height());
                    let target = (y + down * step).clamp(-(extent as i32), 0);
                    if target != y {
                        controller.scroll_to_y(target);
                    }
                }
                if across != 0 {
                    let extent = (listing.widest.extent(listing.key()) - area.width()).max(0.0);
                    let target = (x + across * step).clamp(-(extent as i32), 0);
                    if target != x {
                        controller.scroll_to_x(target);
                    }
                }
                if let Some(caret) = reach(&listing, at) {
                    mark_drag(marked, pane, caret.row, Some(caret.col));
                }
            }
            running.set(false);
        });
    }
}

#[cfg(test)]
mod tests;

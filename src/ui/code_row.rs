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
//! hit-test behind its [`ParagraphHolder`], wrapped at the head of this file as the probes
//! that answer where a pointer is (`caret_col`), where a column is (`caret_x`) and where a
//! word ends (`word_at`). No editor, no rope -- and no engine paint either: the highlight
//! and the caret are rects of the row's own, placed by the column's x and the row's height
//! on the device pixel grid, where the engine's highlight is the glyphs' tight box and
//! leaves a seam between one row's and the next's.
//!
//! **A link is drawn one way wherever it is** -- the wash, the rounded corner, the rule
//! under it and the lit colour of `link_chrome` -- and is drawn as one only while a press
//! on it would be a door. That answer is the link's own ([`Drawn`]'s `open`), asked once
//! and used for the light, for the pointer's icon and by the press, so none of the three
//! can offer what the others will not do. An element inside the text wears its own box; a
//! run of the row's text is washed by a box the row places over its columns
//! ([`lit_box`]), a span having nothing to draw one with.
//!
//! The pointer's icon is the row's to set, in one place: an I-beam over the text and to
//! the right of it, the hand over a link inside it -- which says it is under the pointer
//! through `over_link` -- and the arrow over the gutter and on leaving the row. Set only
//! when it changes, since each set is a message to the platform, and kept in one cell for
//! the whole thread: a row's own memory of it would be wrong the moment the row beside it
//! set something else.
//!
//! **Nothing inside a row may listen to `pointer_down`.** A bubbling event is measured
//! once, against the deepest listener, and every ancestor's handler is handed the same
//! data (`notes/upstream/freya.md`), so a child listening to the down would hand the row a
//! location relative to the child and the column would be wrong. The links listen to the
//! press, which is a different event, and to `over`/`out`.
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
/// the part of the pane's character selection this row draws, and what of it is a link.
///
/// `L` is what this row kind's links are, and the two kinds are two types: [`InlineLink`],
/// one element inside the paragraph, and [`TextLinks`], runs of the row's own text. A kind
/// whose link is there or not says `Option<..>`, one that never has any says [`NoLinks`].
/// So the half a row used to fill with nothing is gone, and with it the row holding one
/// kind's element beside the other's columns -- which no row ever wanted and every reader
/// of this had to rule out.
pub(crate) struct Text<L> {
    /// The row's text as it is drawn, which is what the columns count and the copy takes.
    pub(crate) line: Line,
    /// The spans before an inline link, and after it; all of them where there is none.
    pub(crate) head: Vec<Span<'static>>,
    pub(crate) tail: Vec<Span<'static>>,
    /// What this row draws of the character selection.
    pub(crate) chars: RowChars,
    /// The columns of every match the pane's find bar has on this row, washed under the
    /// text. Empty where no bar is open, and where nothing is typed in one.
    pub(crate) finds: Vec<Range<usize>>,
    /// The columns of **every** name the server placed on this row, links and the places
    /// where one is defined alike: what the pointer is answered about. A superset of
    /// `links`, and not fed to [`cut_at`] -- hovering a name changes no span's style, so
    /// it cuts the row nowhere and cannot widen the listing.
    pub(crate) names: Vec<Range<usize>>,
    /// What the pointer moving onto one of them, or off them all, says. Built per row,
    /// as the two above are: the row knows where a name is drawn, and the pane knows what
    /// place it is.
    pub(crate) on_hover: Option<Rc<dyn Fn(Under)>>,
    /// What of this row is a link.
    pub(crate) links: L,
}

/// What a row kind's links are, asked of it as the row is drawn. Every kind answers in
/// the one shape [`Drawn`], which is what keeps the drawing one function rather than a
/// copy of it per kind.
pub(crate) trait RowLinks: Sized {
    fn drawn(self) -> Drawn;
}

/// A row's links as the drawing reads them: whether a press on one is a door **now**, and
/// then **either** an element inside the paragraph **or** runs of the row's own text, with
/// what a press on one follows. The fields are this module's and the constructors are the
/// only way to one, so what the type parameter keeps apart stays apart here: nothing can
/// hand the drawing both, or half of either.
#[derive(Default)]
pub(crate) struct Drawn {
    open: Option<Rc<dyn Fn() -> bool>>,
    inline: Option<Element>,
    runs: Option<(Vec<Range<usize>>, Rc<dyn Fn(Range<usize>)>)>,
}

impl Drawn {
    /// One element inside the paragraph, and whether a press on it is a door now.
    fn element(element: Element, open: Rc<dyn Fn() -> bool>) -> Self {
        Drawn {
            open: Some(open),
            inline: Some(element),
            runs: None,
        }
    }

    /// Runs of the row's own text, whether a press on one is a door now, and what such a
    /// press follows.
    fn runs(
        columns: Vec<Range<usize>>,
        open: Rc<dyn Fn() -> bool>,
        follow: Rc<dyn Fn(Range<usize>)>,
    ) -> Self {
        Drawn {
            open: Some(open),
            inline: None,
            runs: Some((columns, follow)),
        }
    }
}

/// A row whose text is text: no links of either kind.
pub(crate) struct NoLinks;

impl RowLinks for NoLinks {
    fn drawn(self) -> Drawn {
        Drawn::default()
    }
}

/// A row kind whose link is there or not, which the assembly rows' is: an instruction
/// naming nothing has none.
impl<L: RowLinks> RowLinks for Option<L> {
    fn drawn(self) -> Drawn {
        self.map_or_else(Drawn::default, RowLinks::drawn)
    }
}

/// One element drawn inside the row's paragraph, between [`Text`]'s `head` and its
/// `tail`: a relocation target's name, a branch's displacement, the address an unnamed
/// call goes to. The element keeps its own hover, press and colour; what the row needs of
/// it is `is_link` -- whether a press on it is a door **now** -- which is what the
/// pointer's icon is picked by. That is the element's own rule and not a second copy of
/// it, so the hand and the light cannot disagree.
pub(crate) struct InlineLink {
    pub(crate) element: Element,
    pub(crate) is_link: Rc<dyn Fn() -> bool>,
}

impl RowLinks for InlineLink {
    fn drawn(self) -> Drawn {
        Drawn::element(self.element, self.is_link)
    }
}

/// The columns of the runs of a row's own text that are links, in the order they are
/// drawn. A door that is text and not an element: the row's columns stay the file's own,
/// which is what lets a press on one say where it was in the terms everything else speaks
/// (`src/chars.rs`).
///
/// `is_link` is the same question an [`InlineLink`] answers -- whether a press on one is a
/// door **now** -- and the light, the hand and the press are all picked by it, so none of
/// the three offers what the others will not do. A name in the source is always one; a
/// label in the object's listing only while Ctrl is held, and a plain press on it is the
/// row's own: the row picked out, and a sweep begun.
///
/// [`Text::head`] is cut at their edges before it is drawn ([`cut_at`]), so lighting a
/// link changes a span's style and never where the spans are cut -- a boundary that moved
/// with the pointer would re-shape the row and widen the listing for good.
pub(crate) struct TextLinks {
    pub(crate) columns: Vec<Range<usize>>,
    pub(crate) is_link: Rc<dyn Fn() -> bool>,
    /// What a press on one follows. Built per row, as the menu is.
    pub(crate) follow: Rc<dyn Fn(Range<usize>)>,
}

impl RowLinks for TextLinks {
    fn drawn(self) -> Drawn {
        Drawn::runs(self.columns, self.is_link, self.follow)
    }
}

impl<L: RowLinks> Text<L> {
    /// This text with its links asked what they are: where the row kind's own type ends
    /// and the one drawing begins.
    fn asked(self) -> Text<Drawn> {
        Text {
            line: self.line,
            head: self.head,
            tail: self.tail,
            chars: self.chars,
            finds: self.finds,
            names: self.names,
            on_hover: self.on_hover,
            links: self.links.drawn(),
        }
    }
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
/// sweep reaches, and the nudge that puts the rows on the device pixel grid.
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
}

/// A row's laid-out paragraph and where it starts, lent to the list by the row as it
/// renders: what answers a column for an x on a row the pointer is not over. Written
/// afresh by every render of the row, and the paragraph held **weakly**: the list is keyed
/// by row and never forgets one, so a strong hold would keep a shaped paragraph for every
/// row the reader has scrolled past. A row the list has stopped building leaves an entry
/// that answers nothing, and no reach asks it: a sweep only asks about rows on screen.
#[derive(Clone)]
pub(crate) struct RowText {
    holder: Weak<RefCell<Option<ParagraphHolderInner>>>,
    text_x: Rc<Cell<f32>>,
}

impl Listing {
    /// A fresh list, with nothing lent yet and no listing drawn. The nudge is handed in
    /// rather than made here, this being called once from inside a hook's closure.
    pub(crate) fn new(controller: ScrollController, widest: Widest, nudge: State<f32>) -> Self {
        Listing {
            controller,
            bounds: Rc::new(Cell::new(Area::zero())),
            texts: Rc::new(RefCell::new(HashMap::new())),
            widest,
            key: Rc::new(Cell::new(0)),
            rows: Rc::new(Cell::new(0)),
            nudge,
        }
    }

    /// The box was laid out with its top at `top`, which is what the rows are pushed off.
    /// The grid is taken at the render and not here, so the handler asks nothing of the
    /// runtime.
    pub(crate) fn measured(&self, grid: Grid, top: f32) {
        let mut nudge = self.nudge;
        nudge.set_if_modified(grid.nudge(top));
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

    /// The listing the list is drawing, told to this by every render of the list: what
    /// its rows are floored to and what a sweep's sideways extent is asked under.
    pub(crate) fn drawing(&self, listing: u64) {
        self.key.set(listing);
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
        let extent = scroll_extent(self.rows(), code_row_height(), self.bounds.get().height());
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
    /// Whether the pointer is over the link inside the text, which the link's box says.
    over_link: Rc<Cell<bool>>,
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
        over_link: use_hook(|| Rc::new(Cell::new(false))),
        named: use_hook(|| Rc::new(Cell::new(None))),
        laid: use_state(|| false),
        has_text,
    }
}

impl RowCells {
    /// The column under `at`, a location relative to the row: `None` left of the text on
    /// a press, which is the gutter and picks rows out alone; and on a sweep column 0,
    /// since a pointer left of the text is where the line starts.
    fn column(&self, at: CursorPoint, press: bool) -> Option<usize> {
        if !self.has_text {
            return None;
        }
        let x = at.x as f32 - (self.text_x.get() - self.row_x.get());
        if x < 0.0 {
            return if press { None } else { Some(0) };
        }
        caret_col(&self.holder.read(), x, at.y as f32)
    }

    /// Where column `col` of a row `units` long is, from the row's padded edge, once the
    /// paragraph has been laid out and the holder can say.
    fn column_x(&self, col: usize, units: usize) -> Option<f32> {
        (*self.laid.read()).then_some(())?;
        let x = caret_x(&self.holder.read(), col.min(units))?;
        Some(self.text_x.get() - self.row_x.get() - ROW_PAD + x)
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

/// What of a row is a link, once the row kind's own type is gone ([`Drawn`]). The element
/// is not here: it goes into the paragraph, and nothing after that wants it.
#[derive(Clone)]
struct Links {
    /// Whether a press on this row's link is a door *now*: the link's own answer, whether
    /// it is the element inside the text or a run of the text itself. The light and the
    /// hand are both picked by it, so neither can offer what a press will not do.
    open: Rc<dyn Fn() -> bool>,
    /// The columns of the runs of the row's own text that are links.
    columns: Rc<Vec<Range<usize>>>,
    /// What a press on one of those runs follows.
    follow: Option<Rc<dyn Fn(Range<usize>)>>,
}

impl Links {
    /// The row's links taken out of its text, before the spans are moved into the
    /// paragraph, and the element that goes inside it.
    fn taken(text: &mut Option<Text<Drawn>>) -> (Self, Option<Element>) {
        let Drawn { open, inline, runs } = text
            .as_mut()
            .map(|text| std::mem::take(&mut text.links))
            .unwrap_or_default();
        let (columns, follow) = match runs {
            Some((columns, follow)) => (columns, Some(follow)),
            None => (Vec::new(), None),
        };
        let links = Links {
            open: open.unwrap_or_else(|| Rc::new(|| false)),
            columns: Rc::new(columns),
            follow,
        };
        (links, inline)
    }

    /// Which of the runs column `column` is in, and `None` where it is in none.
    fn at(&self, column: Option<usize>) -> Option<usize> {
        let column = column?;
        self.columns.iter().position(|link| link.contains(&column))
    }
}

/// The row: its chrome, what comes `before` the text -- a gutter, an address, a line
/// number -- the `text` where the row has any, and the `menu` the right button opens.
///
/// The menu is handed the column the pointer was over, which is what a question about the
/// name under it needs and only this knows: `None` in the gutter, and on a row with no
/// text at all.
pub(crate) fn code_row<L: RowLinks>(
    chrome: Chrome,
    before: Vec<Element>,
    text: Option<Text<L>>,
    menu: Option<Rc<dyn Fn(Event<PressEventData>, Option<usize>)>>,
) -> Rect {
    // The row kind's links asked what they are, and then the drawing, which is one
    // function whatever the kind: a drawing generic in the kind would be a whole copy of
    // itself per kind, for the two questions the four lines below ask.
    row(chrome, before, text.map(Text::asked), menu)
}

/// The drawing, which every row kind's [`Text`] has come to the one shape for.
fn row(
    chrome: Chrome,
    before: Vec<Element>,
    mut text: Option<Text<Drawn>>,
    menu: Option<Rc<dyn Fn(Event<PressEventData>, Option<usize>)>>,
) -> Rect {
    let marked = use_consume::<Marked>().0;
    let shift = use_consume::<Shift>().0;
    let listing = use_consume::<Listing>();
    let cells = use_row_cells(text.is_some());
    // Which of the links in the row's own text the pointer is over. Written with
    // `set_if_modified`, so a row is drawn again when the pointer crosses a link's edge
    // and not as it moves along one.
    let mut over = use_state(|| None::<usize>);
    let alt = try_consume_context::<Alt>().map(|alt| alt.0);
    let grid = pixel_grid();

    // The row's links, taken out of `text` before its spans are moved into the paragraph
    // below, since the handlers need them.
    let (links, inline) = Links::taken(&mut text);
    // Every name on the row and what to say about the one under the pointer, taken out
    // of `text` for the same reason the links are.
    let names = text
        .as_ref()
        .map(|text| text.names.clone())
        .unwrap_or_default();
    let on_hover = text.as_ref().and_then(|text| text.on_hover.clone());
    let tell = tell_hover(&cells, names, on_hover.clone());
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
    let lit = over().filter(|_| {
        let blocked = alt.is_some_and(|alt| *alt.read());
        (links.open)() && !blocked
    });
    let columns = lit.and_then(|lit| links.columns.get(lit)).cloned();
    let drawn = text.map(|text| {
        let units = text.line.units();
        let (selected, caret) = marks(&cells, &listing, grid, text.chars, units);
        let wash = lit_box(&cells, grid, columns.as_ref(), units);
        let matched = found(&cells, grid, &text.finds, units);
        let inline = inline.map(|element| link_box(&cells, &links, element));
        (
            wash,
            matched,
            selected,
            text_paragraph(&cells, text, &links, columns.as_ref(), inline),
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
        .on_pointer_move(on_move(
            &cells,
            &chrome,
            &links,
            tell.clone(),
            over,
            marked,
            alt,
        ))
        .on_pointer_out(move |_| {
            over.set_if_modified(None);
            tell(None);
            set_icon(CursorIcon::Default);
        })
        .children(before);

    // The lit link's box, the find bar's matches and the selection before the paragraph
    // in the tree, so all three are painted under the text -- and **always there**, as is
    // the caret's slot: freya matches siblings by position, so a rect appearing before the
    // paragraph on the press would move the paragraph along one and remount it, link and
    // all, between the down and the up, and the press meant for the link would never fire.
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
fn tell_hover(
    cells: &RowCells,
    names: Vec<Range<usize>>,
    on_hover: Option<Rc<dyn Fn(Under)>>,
) -> Rc<dyn Fn(Option<usize>)> {
    let cells = cells.clone();
    Rc::new(move |column: Option<usize>| {
        let on = column.and_then(|column| names.iter().position(|name| name.contains(&column)));
        // Off a name, only the crossing is worth saying: that there is nothing under the
        // pointer stays true however far it moves. **On** one, every move is said, a move
        // being what puts the wait for it back to the beginning (`src/ui/hovering.rs`).
        let crossed = cells.named.replace(on) != on;
        if !crossed && on.is_none() {
            return;
        }
        let Some(tell) = on_hover.as_ref() else {
            return;
        };
        let Some(columns) = on.and_then(|on| names.get(on)).cloned() else {
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
    let selected = chars
        .highlight
        .map(|(from, to)| (from.min(units), to.min(units)))
        .and_then(|(from, to)| {
            let (left, right) = (cells.column_x(from, units)?, cells.column_x(to, units)?);
            let right = if right > left {
                right
            } else if units == 0 {
                left + code_row_height() / 4.0
            } else {
                return None;
            };
            let span = grid.span(left, right);
            Some(
                rect()
                    .interactive(false)
                    .position(Position::new_absolute().left(span.near).top(0.0))
                    .width(Size::px(span.thick))
                    .height(Size::px(code_row_height()))
                    .background(palette().text_select_bg),
            )
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
        rect()
            .interactive(false)
            .position(Position::new_absolute().left(stroke.near).top(0.0))
            .width(Size::px(stroke.thick))
            .height(Size::px(code_row_height()))
            .background(palette().caret_fg)
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
/// (`LINK_BOX_INSET`). So a name in the source and a label in the object's listing are lit
/// exactly as an operand of an instruction is, which wears the same chrome as an element.
///
/// A rect of the row's own, as the selection's is, because a span carries no box: freya's
/// text styles have a colour, a weight and a decoration and nothing to draw one with.
/// Nothing until the paragraph is laid out, which is when the row can say where a column
/// is.
fn lit_box(cells: &RowCells, grid: Grid, columns: Option<&Range<usize>>, units: usize) -> Rect {
    let Some(columns) = columns else {
        return nothing();
    };
    let (from, to) = (columns.start.min(units), columns.end.min(units));
    let (Some(left), Some(right)) = (cells.column_x(from, units), cells.column_x(to, units)) else {
        return nothing();
    };
    if right <= left {
        return nothing();
    }
    let span = grid.span(left, right);
    link_chrome(
        rect()
            .interactive(false)
            .position(Position::new_absolute().left(span.near).top(LINK_BOX_INSET))
            .width(Size::px(span.thick))
            .height(Size::px(code_row_height() - 2.0 * LINK_BOX_INSET)),
        Some(palette().name_hover_fg),
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
        .filter_map(|columns| {
            let (from, to) = (columns.start.min(units), columns.end.min(units));
            let (left, right) = (cells.column_x(from, units)?, cells.column_x(to, units)?);
            if right <= left {
                return None;
            }
            let span = grid.span(left, right);
            Some(
                rect()
                    .interactive(false)
                    .position(Position::new_absolute().left(span.near).top(0.0))
                    .width(Size::px(span.thick))
                    .height(Size::px(code_row_height()))
                    .background(palette().find_bg)
                    .into_element(),
            )
        })
        .collect();
    nothing().children(washes)
}

/// The link inside the text, in a box that says when the pointer is over it: the hand is
/// the link's and the I-beam the text's, and the row sets both ([`set_icon`]).
fn link_box(cells: &RowCells, links: &Links, element: Element) -> Rect {
    let (entered, left) = (cells.over_link.clone(), cells.over_link.clone());
    let open = links.open.clone();
    rect()
        .on_pointer_over(move |_| {
            entered.set(true);
            set_icon(if open() {
                CursorIcon::Pointer
            } else {
                CursorIcon::Text
            });
        })
        .on_pointer_out(move |_| {
            left.set(false);
            set_icon(CursorIcon::Text);
        })
        .child(element)
}

/// The row's text as one paragraph: the spans before the link, the link itself, and the
/// spans after it. `lit` is the columns of the run of the row's own text that is drawn as
/// a link, the box around it being the row's ([`lit_box`]).
fn text_paragraph(
    cells: &RowCells,
    text: Text<Drawn>,
    links: &Links,
    lit: Option<&Range<usize>>,
    inline: Option<Rect>,
) -> Paragraph {
    let (text_x, mut laid) = (cells.text_x.clone(), cells.laid);
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
        .spans_iter(light(cut_at(text.head, &links.columns), lit).into_iter())
        .maybe_child(inline)
        .spans_iter(text.tail.into_iter())
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

/// The row's `on_pointer_down`: a link followed, a run started, or the menu.
///
/// The *down* and not the press: a drag is over by the time a press fires, so a selection
/// swept out with the button held has to begin as it goes down. The right button's down is
/// the menu, **in the same handler**: `on_secondary_down` is `on_pointer_down` under
/// another name and would replace this one ([`secondary`]).
fn on_down(
    cells: &RowCells,
    chrome: &Chrome,
    links: &Links,
    menu: Option<Rc<dyn Fn(Event<PressEventData>, Option<usize>)>>,
    marked: State<Marks>,
    shift: State<bool>,
    alt: Option<State<bool>>,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let (cells, links) = (cells.clone(), links.clone());
    let (pane, row, file) = (chrome.pane, chrome.row, chrome.file.clone());
    move |e: Event<PointerEventData>| {
        if e.button() == Some(MouseButton::Left) {
            let at = cells.column(e.element_location(), true);
            // freya counts the presses in one place, and **asking is counting**: a press
            // it was not asked about is one the next reads as a double. So it is asked
            // exactly once, whatever the press turns out to be.
            let presses = EventsCombos::pressed(e.global_location());
            // A link is followed on a single press with nothing held, and only while it
            // is a door: two presses on a name are what take the word, Alt says this one
            // is not a door, and a label is one only under Ctrl -- without which the
            // press is the row's, as it is over any other text.
            let link = links
                .at(at)
                .filter(|_| presses == PressEventType::Single && !held(alt) && (links.open)())
                .zip(links.follow.clone());
            if let Some((link, follow)) = link {
                // And it picks no line out: the press is the question and not a place in
                // the file.
                follow(links.columns[link].clone());
                return;
            }
            let press = at.map(|col| {
                // Two presses on a word take the word, three the row's text, as the text
                // engine divides them.
                match presses {
                    PressEventType::Double => word_at(&cells.holder.read(), col)
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
        // The column before the event is turned into a press: what the menu is asked
        // about is where the pointer was.
        let at = cells.column(e.element_location(), true);
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
    links: &Links,
    tell: Rc<dyn Fn(Option<usize>)>,
    mut over: State<Option<usize>>,
    marked: State<Marks>,
    alt: Option<State<bool>>,
) -> impl FnMut(Event<PointerEventData>) + 'static {
    let (cells, links) = (cells.clone(), links.clone());
    let (pane, row) = (chrome.pane, chrome.row);
    move |e: Event<PointerEventData>| {
        let at = e.element_location();
        let column = cells.column(at, false);
        mark_drag(marked, pane, row, column);
        // Neither the link under the pointer nor the name is answered while a selection
        // is being swept out: a drag along a line would otherwise light every name it
        // passed under. The name is said whether it is a link or not -- a name where one
        // is defined is not a link and is still something to ask the server about -- but
        // under the same guard, for the same reason.
        let sweeping = dragging(marked, pane) || held(alt);
        let hovered = (!sweeping).then(|| links.at(column)).flatten();
        over.set_if_modified(hovered);
        tell(if sweeping { None } else { column });
        let on_text = cells.has_text && at.x as f32 >= cells.text_x.get() - cells.row_x.get();
        // The hand over a link, whichever kind it is, and only while a press on it would
        // be a door: the link's own rule, which is what lights it, so the two cannot
        // disagree.
        let on_link = cells.over_link.get() || hovered.is_some();
        set_icon(if on_link && (links.open)() {
            CursorIcon::Pointer
        } else if on_text {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        });
    }
}

/// Alt held, which says a press on a link is not a door, so what is under the pointer is
/// text.
fn held(alt: Option<State<bool>>) -> bool {
    alt.is_some_and(|alt| *alt.peek())
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
        let units = chars::units(&span.text);
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
fn light(head: Vec<Span<'static>>, columns: Option<&Range<usize>>) -> Vec<Span<'static>> {
    let Some(columns) = columns else {
        return head;
    };
    let mut column = 0;
    head.into_iter()
        .map(|span| {
            let units = chars::units(&span.text);
            let at = column;
            column += units;
            match at >= columns.start && column <= columns.end && at < column {
                true => span.color(palette().name_hover_fg),
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

//! The bar's own scrolling: where every chip has been measured to, how far the row of
//! them is slid, and every rule over the two.
//!
//! **The strip scrolls itself, without a `ScrollView`.** It is one row, and what it needs
//! is an offset: the wheel over it moves that offset sideways, opening a tab or going to
//! one brings its chip into view ([`use_reveal`]), and a drag held near either end
//! scrolls the strip under the pointer. freya's own scroll view answers none of the three
//! well -- a controller handed to it from outside only reaches it when something else
//! happens to re-render it, its wheel means the vertical axis, and it stops answering the
//! wheel at all while a drag is under way (`notes/upstream/freya.md`) -- and a row of
//! chips needs no scrollbar, no keyboard scrolling and no drag-to-scroll to make up for.
//!
//! Where the chips are is **measured** and not worked out, a chip being as wide as its
//! name. The measurements are peeked and never read, so a layout wakes nothing on its
//! own; what wakes the reveal is `shape`, which counts up when a chip changes width or
//! moves **along the row** -- a tab opened, closed or moved -- and not when the row slides
//! under the window, which is the strip being scrolled. Each chip is therefore measured
//! with the offset taken back off.

use super::*;

/// Every measurement the bar keeps, made in one place: the hook [`TabBar`] opens with.
pub(super) fn use_bar() -> Bar {
    let shape = use_state(|| 0u64);
    Bar {
        places: use_consume::<Chipped>().0,
        viewport: use_state(|| None),
        content: use_state(|| 0.0f32),
        // Read as well as written: the row of chips is drawn at this offset, so the bar
        // has to be woken when it changes.
        offset: use_state(|| 0.0f32),
        shape,
        // A read, which is what subscribes the bar to a new shape.
        laid_out: shape(),
    }
}

/// Where every chip is: its two sides along the row, one entry per open tab. The map the
/// panes keep their places in, keyed by a tab rather than by a place on one.
pub(crate) type Chips = Positions<Tab, (f32, f32)>;

/// Where the bar has measured its chips to, at the root rather than in [`TabBar`] itself.
///
/// The bar is mounted at most once, so this is the same one state either way -- and at
/// the root a test can read what the bar still holds a place for, there being no reading
/// a component's own state from outside.
#[derive(Clone, Copy)]
pub(crate) struct Chipped(pub(crate) State<Chips>);

/// What the bar has been measured as, how far along it is, and every rule over the two:
/// passed about as one thing because nothing that scrolls can do without all of it.
#[derive(Clone, Copy)]
pub(crate) struct Bar {
    /// Every chip's two sides, along the row: where each was laid out, less the offset,
    /// so that scrolling the strip moves none of them. Only the open tabs: nothing
    /// measures a chip that has gone, so its entry is dropped when its tab closes.
    pub(crate) places: State<Chips>,
    /// The two sides of the strip: what a chip has to be inside to be in view.
    pub(crate) viewport: State<Option<(f32, f32)>>,
    /// How wide the chips are altogether.
    pub(crate) content: State<f32>,
    /// How far the row of chips is slid to the left, which is never positive.
    pub(crate) offset: State<f32>,
    /// Counts up whenever the bar takes a new shape, which is what wakes the reveal.
    pub(crate) shape: State<u64>,
    /// What that count said as the bar was drawn. A new shape is counted from here rather
    /// than from the count itself, so a whole layout's chips settling at once is one new
    /// shape and not one apiece.
    pub(crate) laid_out: u64,
}

impl Bar {
    /// Where a chip sits **along the row**: where it was laid out, less how far the row is
    /// slid under the window.
    ///
    /// A chip that moved along the row or changed width is the bar taking a new shape -- a
    /// tab opened, closed or moved. The strip being scrolled moves every chip in the
    /// window and none along the row, and owes the reveal nothing. Under a pixel is the
    /// subtraction and not a move, each end being worked out from whatever offset the row
    /// was drawn at.
    pub(crate) fn chip_sized(self, tab: Tab, min_x: f32, max_x: f32) {
        let slid = *self.offset.peek();
        let at = (min_x - slid, max_x - slid);
        let held = self.places.peek().at(&tab);
        let shifted =
            held.is_none_or(|(min, max)| (min - at.0).abs() >= 1.0 || (max - at.1).abs() >= 1.0);
        if !shifted {
            return;
        }
        let mut places = self.places;
        places.write().remember(tab, at);
        self.reshaped();
    }

    /// Where the strip is cut off, which is what a chip is measured against. A strip of
    /// another size is a new shape: what was in view was in view of the old one. A wider
    /// strip also raises the floor, so it takes the same scroll of nothing as
    /// [`Bar::content_sized`]; otherwise widening the window leaves empty ground past the
    /// last chip.
    pub(crate) fn viewport_sized(self, min_x: f32, max_x: f32) {
        let at = Some((min_x, max_x));
        if *self.viewport.peek() == at {
            return;
        }
        let mut viewport = self.viewport;
        viewport.set(at);
        self.scroll_by(0.0);
        self.reshaped();
    }

    /// How wide the chips are altogether, which is what says how far the strip may be
    /// scrolled. A bar that has lost a chip may now fit the window: a scroll of nothing
    /// puts the offset back inside the new floor, which [`Bar::scroll_by`] clamps against
    /// only as it moves. Otherwise a closed tab leaves empty ground past the last chip
    /// until something scrolls.
    pub(crate) fn content_sized(self, width: f32) {
        if *self.content.peek() == width {
            return;
        }
        let mut content = self.content;
        content.set(width);
        self.scroll_by(0.0);
    }

    /// The wheel over the strip is the strip's own axis, whichever axis it arrives on: a
    /// bar has no second one, and a reader turning the wheel over it means "further
    /// along".
    pub(crate) fn wheel(self, delta_x: f32, delta_y: f32) {
        let by = match delta_y.abs() > delta_x.abs() {
            true => delta_y,
            false => delta_x,
        };
        self.scroll_by(by);
    }

    /// A drag held near either end of the strip scrolls it towards that end, `x` being
    /// where the pointer is in the window. It is the only way to reach the far end while
    /// carrying a tab, and a pointer held anywhere else moves nothing.
    pub(crate) fn drag_edge(self, dragging: bool, x: f32) {
        if !dragging {
            return;
        }
        let Some((left, right)) = *self.viewport.peek() else {
            return;
        };
        if x < left + DRAG_EDGE {
            self.scroll_by(DRAG_STEP);
        } else if x > right - DRAG_EDGE {
            self.scroll_by(-DRAG_STEP);
        }
    }

    /// Move the strip `delta` pixels, positive being towards its start, and never past
    /// either end: the first chip does not leave the left edge, and the last does not
    /// leave the right.
    pub(crate) fn scroll_by(self, delta: f32) {
        let Some((left, right)) = *self.viewport.peek() else {
            return;
        };
        let floor = -(*self.content.peek() - (right - left)).max(0.0);
        let want = (*self.offset.peek() + delta).clamp(floor, 0.0);
        let mut offset = self.offset;
        offset.set_if_modified(want);
    }

    /// The bar has taken a new shape, which is what the reveal wakes on.
    fn reshaped(self) {
        let mut shape = self.shape;
        shape.set(self.laid_out + 1);
    }
}

/// Bring the tab on screen into view when it changes, and when the bar takes a new shape
/// -- a tab opened, closed or moved -- which is what makes an opening reveal the tab it
/// opened, and what brings the tab being read back after a chip to its left has gone or a
/// chip has been dropped past it, either of which slides it out of sight.
///
/// **Not on every layout**, which would take the strip back off the reader the moment they
/// scrolled it to look at something else.
pub(super) fn use_reveal(strip: State<Strip>, bar: Bar) {
    let active = strip.read().active();
    use_side_effect_with_deps(
        &(active, bar.laid_out),
        move |(active, _): &(Option<Tab>, u64)| {
            let Some(active) = *active else {
                return;
            };
            let Some((min, max)) = bar.places.peek().at(&active) else {
                return;
            };
            let Some((left, right)) = *bar.viewport.peek() else {
                return;
            };
            // The places are along the row; where the chip is in the window is that, slid.
            let slid = *bar.offset.peek();
            let (min, max) = (min + slid, max + slid);
            if min < left {
                bar.scroll_by(left - min);
            } else if max > right {
                bar.scroll_by(right - max);
            }
        },
    );
}

/// How near either end of the strip a drag has to be held for it to scroll, and how far it
/// goes per move of the pointer.
pub(crate) const DRAG_EDGE: f32 = 24.0;
const DRAG_STEP: f32 = 12.0;

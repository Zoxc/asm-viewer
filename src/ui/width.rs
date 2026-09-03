//! The widest row a code listing has drawn, and the width every row of it takes from that.
//!
//! A `VirtualScrollView` scrolls sideways exactly as far as the widest row it has *built*:
//! its cross-axis extent is the measured width of its content, and its content is the
//! handful of rows on screen. A row as wide as its pane -- `width(Size::fill())`, which is
//! what every code row was -- leaves that extent the viewport's own and the wheel's
//! `delta_x` moving nothing, so a row that should be reachable sideways has to be
//! measured to its content instead. What that costs is the row's **wash**: the selection
//! and the pair are the row's own background, and a row as wide as its text washes only
//! as far as its text.
//!
//! [`Widest`] is the answer to both at once, taken from freya's own `CodeEditor`, which
//! gives every line the same width: a row is **as wide as the pane or as the widest row
//! the listing has drawn, whichever is more**, and what it holds is measured under it.
//! Every built row is then that wide, so the extent the view scrolls over **is** that
//! number -- stable whichever rows happen to be built, and never shrinking while a wide
//! row scrolls out of the built range -- and the wash of every row runs the whole of it.
//! A row reports what its content measured (`inner_sizes`, the children plus padding) and
//! never the width it was laid out at, or the pane's own width would be folded in and a
//! pane narrowed afterwards would keep a sideways scroll over nothing. What that costs is
//! **one layout**: a row wider than anything the listing has drawn is laid out at the
//! old width first, reports itself, and is drawn whole on the next.
//!
//! It is a width and not a minimum, which was the first shape and would have cost that
//! layout nothing. torin sizes an auto-width node from its minimum *plus* its children:
//! the minimum is where the accumulation starts rather than a floor on its result
//! (`notes/upstream/freya.md`), so a row `width(auto)` with a `min_width` of the pane
//! came out the pane's width and its content again.
//!
//! What was not taken from the editor is its estimate of the widest line -- the most
//! characters on a line times the width of one `W`: that needs Skia on the UI thread, is
//! wrong for a tab or a double-width glyph (a paragraph pinned to a width it cannot fit
//! is clipped), and an object's code cannot know its widest line before the worker has
//! decoded it. So the extent is the widest row *drawn so far*, as the scratchpad's run
//! output already is: a wide row further down is reachable once the reader has scrolled
//! to it, which is the order it is read in anyway.

use std::hash::{DefaultHasher, Hash, Hasher};

use super::*;

/// The widest row drawn, held against the listing it was drawn in: its key and the width.
///
/// One per list, handed to each row the way the list's scroll controller is. A row reads
/// it (`floor`) as it renders, so the state's writes re-render it; a row writes it (`note`)
/// as it is measured, and only ever wider.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Widest(State<(u64, f32)>);

pub(crate) fn use_widest() -> Widest {
    Widest(use_state(|| (0, 0.0)))
}

impl Widest {
    /// The key a listing is held under: the identity of what outlives its rows -- the
    /// disassembly, the highlighted file, the object -- under the fixed-width font's size,
    /// so a font made smaller does not keep the width the larger one measured. Read in
    /// the list's render, which is what subscribes the list to the font.
    pub(crate) fn key(addr: usize) -> u64 {
        let mut hasher = DefaultHasher::new();
        (addr, fonts().mono.size().to_bits()).hash(&mut hasher);
        hasher.finish()
    }

    /// The width every row of `listing` is at least: the widest drawn so far, or nothing
    /// for a listing no row has reported yet, which [`Widest::row_width`] turns into the
    /// pane's own width. A read, so the row asking is re-rendered when the answer grows.
    pub(crate) fn floor(&self, listing: u64) -> f32 {
        let (key, width) = *self.0.read();
        if key == listing {
            width
        } else {
            0.0
        }
    }

    /// The widest row of `listing` as a handler asks it, subscribing nothing: what the
    /// list can be scrolled sideways over.
    pub(crate) fn extent(&self, listing: u64) -> f32 {
        let (key, width) = *self.0.peek();
        if key == listing {
            width
        } else {
            0.0
        }
    }

    /// A row of `listing` measured `natural` wide: kept if it is the widest so far, or the
    /// first of a listing the state does not yet hold -- which is the reset, made without
    /// an effect: the rows of the listing before drop to the pane's width on the first
    /// render of the new one, and report again as they are laid out.
    pub(crate) fn note(&self, listing: u64, natural: f32) {
        // Bound before the write: a `peek` holds a guard, and writing under it panics.
        let (key, seen) = *self.0.peek();
        if key != listing || natural > seen {
            let mut state = self.0;
            state.set((listing, natural));
        }
    }

    /// The width a row takes: the pane's or `floor`, whichever is more. `Fn` and not
    /// `fill`, since only a closure can say "at least the parent's" -- and a width, not a
    /// minimum, for the reason in the module's doc. Compared by its data, so a row whose
    /// floor has not moved is not re-laid out.
    pub(crate) fn row_width(floor: f32, listing: u64) -> Size {
        Size::func_data(
            move |context| Some(context.available_parent.max(floor)),
            &(listing, floor.to_bits()),
        )
    }
}

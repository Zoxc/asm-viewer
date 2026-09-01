//! The numbers the interface is laid out on, and the fonts those numbers come from.
//!
//! One file, because the row heights *are* a function of the fonts: `list_row_height` and
//! `code_row_height` are `row_height_for` over `fonts().ui` and `fonts().mono`, so the
//! constants and the thread-local they read cannot be separated without one of them
//! reaching across a file boundary for the other. What is here is every measurement no
//! single component owns -- a lane's width, a tag's column, how long a tooltip waits --
//! and the accessor that makes asking for a font a subscription to it.

use super::*;

/// The leading a row adds to the font it is drawn in, and the floor under the answer.
///
/// Additive and not a multiple, because leading is what it is: twelve logical pixels of
/// air is legible above an 11px line and above a 30px one, where a ratio that reads well
/// at one of those is cramped or cavernous at the other. Twelve is also the number the app
/// already had -- it is exactly what `ROW_HEIGHT`'s 26 was over the 14px fixed-width
/// default. One leading and not one per kind of row: a row is a line of text with air
/// above and below it wherever it is drawn, and two numbers here would be two answers to
/// a question the fonts already differ on. The floor is
/// against a hand-edited `settings.toml`: `FontSetting::size` refuses a size that is not
/// positive, but 0.1 is positive, and a list whose `item_size` is a fraction of a pixel is
/// a division nothing recovers from.
const ROW_LEADING: f32 = 12.0;
const MIN_ROW_HEIGHT: f32 = 14.0;

/// A row's height from the size of the font written in it: the one derivation, so that
/// the two heights below can differ only in which font they ask about.
fn row_height_for(font_size: f32) -> f32 {
    (font_size + ROW_LEADING).round().max(MIN_ROW_HEIGHT)
}

/// Height of a row drawn in the **interface** font -- the objects tree, the symbol and
/// history lists, the tab strips and chips, the project and settings rows -- and the
/// `item_size` of the scroll views over them. **A view's `item_size` and the height its
/// rows actually draw at must be equal or scrolling misaligns**, which is why each of
/// these is one function and not a number every site repeats.
///
/// **It follows the fonts, which is 9c's decision and the one that was actually open.**
/// It was a `const`, and the settings page makes both sizes something a reader can change
/// -- so the alternative was a page that offers a 20pt assembly font and draws it clipped
/// inside a 26px row, with a sentence somewhere admitting it. That is a worse answer than
/// the work: every consumer of the number already goes through one of four places
/// (`item_size`, a row's own `height`, the gutter's stroke geometry, and
/// `row_at`/`row_offset`), and a function is what keeps them from being able to disagree.
/// What made it *safe* is that the two halves are read in the same render pass: the state
/// under `fonts()` is written before anything is re-rendered, so a scroll view and the
/// rows it builds cannot see different heights, and the per-tab positions 8b saves are
/// **rows** rather than pixel offsets precisely so that a change here does not move any of
/// them.
pub(crate) fn list_row_height() -> f32 {
    row_height_for(fonts().ui.size())
}

/// Height of a row drawn in the **fixed-width** font -- the instruction and source rows,
/// the editor's own lines, a run's output -- and the `item_size` of the views over those.
///
/// **Two heights and not one, because no row mixes the two fonts.** Every row in the code
/// panes sets `assembly_font()` on itself and on every span it draws; the sidebar's rows
/// set nothing and inherit the interface font from the root. So the larger of the two
/// sizes was never a constraint either kind of row was under -- it was one number serving
/// two lists, where raising the assembly font padded the sidebar and raising the interface
/// font padded the disassembly. The heights are independent because the fonts are, which
/// is what the settings page already implies by offering them separately.
///
/// This is also the height `row_at`/`row_offset` convert against, and they are the code
/// panes' alone: `use_kept_position` and `reveal_row` are called by `InstructionList` and
/// `SourceList` and by nothing else, the sidebar's lists keeping no per-tab position and
/// having nothing to reveal.
pub(crate) fn code_row_height() -> f32 {
    row_height_for(fonts().mono.size())
}

/// The height of the strip a filter bar's text box sits in. Taller than a row by the room
/// an `Input`'s border and its own inner margin need; it is a bar and not a row, and
/// nothing lines up with it.
///
/// The **list** height, because a filter bar sits over one of the three sidebar lists and
/// is drawn in the interface font like the rows under it -- there is no filter over a code
/// pane, and a bar following the assembly font would grow over a list it has nothing to do
/// with.
pub(crate) fn filter_height() -> f32 {
    list_row_height() + 6.0
}

/// The side of one of the three square toggle buttons: a row less the air around it, so
/// the `Aa` and `.*` written inside them follow the interface font like everything else.
/// `filter_height`'s height for `filter_height`'s reason -- they are two parts of one bar.
pub(crate) fn toggle_size() -> f32 {
    list_row_height() - 4.0
}

/// How much bigger than the interface font a tab bar's icon is drawn. A Lucide glyph
/// fills its whole box where a letter fills its x-height, so an icon at exactly the text
/// size reads as the larger of the two; a quarter up from it sits it on the same optical
/// line as the word beside it. It is a multiple and not a pixel count because the
/// interface font is the desktop's (`fonts()`), so an icon that did not follow it would
/// be a postage stamp beside a 20px title, or tower over a 9px one.
const ICON_SCALE: f32 = 1.25;

/// The side of a tab bar icon: the interface font, scaled, and capped so that it is never
/// what decides how tall the bar is -- a row has to keep a little air above and below
/// whatever the desktop's font size turns out to be.
pub(crate) fn icon_size() -> f32 {
    (fonts().ui.size() * ICON_SCALE)
        .round()
        .min(list_row_height() - 8.0)
}

/// The column a file row's disclosure triangle sits in, and the width every row of the
/// objects tree gives up to it so that the tags below one another line up whether or not
/// the row has a triangle.
pub(crate) const CHEVRON_WIDTH: f32 = 14.0;

/// How far an archive member is indented past the file it belongs to. Past the triangle
/// and into the tag column, so the nesting is legible in a 300px sidebar without the name
/// starting halfway across it.
pub(crate) const TREE_INDENT: f32 = 16.0;

/// The column a project field's name is written in, so the values beside them line up
/// whatever each is called -- `SourceRow`'s line-number gutter's reason.
pub(crate) const FIELD_LABEL_WIDTH: f32 = 72.0;

/// The column the short format tag is written in. Fixed, so the names to the right of it
/// start at the same x whatever the tag says -- the reason `SourceRow`'s line-number
/// gutter is a fixed width and not a minimum.
pub(crate) const TAG_WIDTH: f32 = 34.0;

/// The tag is written smaller than the row's own text: it is a label on the row and not
/// what the row is called.
pub(crate) const TAG_FONT_SIZE: f32 = 10.0;

/// How long a list row's tooltip waits before it appears.
///
/// `TooltipContainer` defaults to 500ms, which is right for a button whose tooltip
/// explains it and wrong for a row whose tooltip *is* its own text, cut off: a truncated
/// name is read by sweeping the pointer down the list, and half a second per row makes
/// that gesture useless. Zero rather than a small number because the component still
/// fades the tooltip in over 150ms, so "no delay" is already not a pop -- adding a wait
/// in front of that animation only makes the sweep lag behind the pointer.
///
/// The filter toggles deliberately keep the 500ms default: their tooltip explains what
/// `\b` means rather than finishing a word the row could not fit, and a pointer crossing
/// the bar on its way somewhere else should not light up three of them.
pub(crate) const TOOLTIP_DELAY: Duration = Duration::ZERO;

/// How long a symbol may be under analysis before the panes admit they are waiting.
///
/// The number is a judgement about attention rather than about the work: under a fifth of
/// a second a change reads as instantaneous, so a message that appeared inside it would
/// be a flicker between two listings and nothing else -- and on every one of the samples
/// in this repo except `viewer-sample` every symbol comes back inside it. Past it the
/// reader has noticed, and a pane that says nothing at all is a pane that looks broken.
///
/// It is only ever *started* by a selection change (`use_analysis`), never polled, so a
/// wait that ends before it costs one timer task and no render at all.
pub(crate) const SLOW_ANALYSIS: Duration = Duration::from_millis(180);

/// How many characters of a tab chip's name are drawn before the rest is elided. A Rust
/// symbol's demangled name runs to hundreds of them, and a strip is only useful while
/// every tab in it is still a tab.
///
/// A character count and not a width, which is what every other truncation in this file
/// is. Bounding the width would be the better answer and torin will not have it: a
/// `maximum_width` anywhere in the chip makes the chip shrinkable, and a horizontal
/// scroll view measures its children against the space *left* in it, so with more chips
/// than fit the ones past the edge are handed no width at all and drawn as a bare ×.
/// Seen in the headless renders at twelve open tabs, not reasoned about.
pub(crate) const CHIP_NAME_CHARS: usize = 40;

/// The width of one lane of the assembly view's branch gutter: how far apart two branch
/// lines running down the same rows are drawn.
pub(crate) const LANE_WIDTH: f32 = 7.0;

/// How thick a branch line is, horizontal run and arrowhead included. One logical pixel,
/// which is a hairline on a scaled display and the thinnest thing skia will draw solidly
/// on an unscaled one.
pub(crate) const BRANCH_STROKE: f32 = 1.0;

/// How far the horizontal run reaches past the innermost lane, which is where the
/// arrowhead sits.
pub(crate) const ARROW_WIDTH: f32 = 7.0;

/// The gap between an arrowhead's tip and the first digit of the address column, so that
/// the gutter reads as a column of its own rather than as decoration on the addresses.
pub(crate) const GUTTER_PAD: f32 = 3.0;

/// The length of each of the two strokes an arrowhead is made of, and how far each is
/// turned from the horizontal. Both of them pivot on the tip, so the pair is a `>`.
pub(crate) const ARROW_STROKE: f32 = 5.0;
pub(crate) const ARROW_ANGLE: f32 = 30.0;

/// How many rows above a row scrolled into view from the other pane are kept on screen.
/// A line landing against the top edge answers "what is this" without answering "where in
/// the function is it", which is half of why the two panes are side by side at all.
pub(crate) const CONTEXT_ROWS: f32 = 3.0;

thread_local! {
    /// The two fonts the window is currently drawn in.
    ///
    /// **`palette()`'s story exactly, and for the same reasons.** `fonts.rs` handed out a
    /// `&'static Fonts` from a `OnceLock`, and the doc on it spelled out why making *that*
    /// re-readable would have looked like a fix and not been one: nothing subscribes to a
    /// `&'static`, so a changed answer would have reached only whatever happened to
    /// re-render for some other reason -- half the window in the new font and half in the
    /// old. A `State` behind the accessor is the fix, because **asking for a font is what
    /// subscribes the caller to it**: a settings page that writes here repaints every
    /// scope that drew a glyph and no other, wherever in the tree it sits.
    ///
    /// Thread-local and global rather than a context, for the reason the appearance is:
    /// the two row heights, `icon_size` and `FontExt` are free functions and trait methods called
    /// from `if` arms, render callbacks and free functions, none of which may run a hook.
    /// A `State` is `!Send`, only the UI thread draws, and nothing off it may ask.
    ///
    /// An `Arc` because `Fonts` owns two `Vec`s of families and a read is a clone: at a
    /// row per `assembly_font()` that is one refcount rather than four short strings.
    /// Initialised from the stored settings so the first frame is already in the right
    /// font -- `use_settings` writes the same value back on mount, idempotently.
    static FONTS: State<Arc<Fonts>> =
        State::create_global(Arc::new(fonts::resolve(&Settings::load())));
}

/// The fonts to draw with, and a subscription to them for whoever asks.
pub(crate) fn fonts() -> Arc<Fonts> {
    FONTS.with(|fonts| Arc::clone(&fonts.read()))
}

/// Draw in these fonts from now on. Unlike `set_appearance` there is nothing to invalidate
/// alongside it -- `HIGHLIGHTED` caches spans with the palette's *colours* baked in, and a
/// span carries no font -- so this is the write and nothing else. It stays a function of
/// its own all the same, so that the one place fonts change is as findable as the one
/// place the theme does.
pub(crate) fn set_fonts(next: Fonts) {
    FONTS.with(|fonts| {
        let mut fonts = *fonts;
        fonts.set_if_modified(Arc::new(next));
    });
}

/// Applying one of the two fonts. freya takes font families one at a time, pushing
/// each onto the element's own list and appending the parent's behind it, so the
/// chain is set by calling `font_family` in order of preference.
pub(crate) trait FontExt: TextStyleExt + Sized {
    fn font(mut self, font: &Font) -> Self {
        for family in &font.families {
            self = self.font_family(family.clone());
        }
        self.font_size(font.size())
    }

    /// The desktop's interface font, set on the root and inherited by everything.
    fn interface_font(self) -> Self {
        self.font(&fonts().ui)
    }

    /// The desktop's fixed-width font, for the assembly rows.
    fn assembly_font(self) -> Self {
        self.font(&fonts().mono)
    }
}

impl<T: TextStyleExt + Sized> FontExt for T {}

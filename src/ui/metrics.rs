//! The numbers the interface is laid out on, and the fonts those numbers come from --
//! one file, because the row heights are a function of the fonts.

use super::*;

/// A row is the font it is drawn in plus 12 of leading, floored at 14. The floor is
/// against a hand-edited `settings.toml`: a size of 0.1 passes `FontSetting::size`, and
/// an `item_size` of a fraction of a pixel is unrecoverable.
fn row_height_for(font_size: f32) -> f32 {
    (font_size + 12.0).round().max(14.0)
}

/// Height of a row drawn in the **interface** font -- the objects tree, the symbol and
/// history lists, the tab strips and chips, the project and settings rows -- and the
/// `item_size` of the scroll views over them.
///
/// **A view's `item_size` and the height its rows actually draw at must be equal or
/// scrolling misaligns.** There are two heights, this and [`code_row_height`], so a view
/// and its rows have to agree about which as well as about the number. Both follow the
/// fonts; it is safe because the two halves are read in the same render pass.
pub(crate) fn list_row_height() -> f32 {
    row_height_for(fonts().ui.size())
}

/// Height of a row drawn in the **fixed-width** font -- the instruction and source rows,
/// the editor's own lines, a run's output -- and the `item_size` of the views over those.
/// Two heights and not one because no row mixes the two fonts. This is also the height
/// `row_at`/`row_offset` convert against.
pub(crate) fn code_row_height() -> f32 {
    row_height_for(fonts().mono.size())
}

/// The side of one of the three square toggle buttons in a filter bar: a row less the air
/// around it.
pub(crate) fn toggle_size() -> f32 {
    list_row_height() - 4.0
}

/// The side of a tab bar icon: the interface font, a quarter bigger, and capped so it is
/// never what decides how tall the bar is.
pub(crate) fn icon_size() -> f32 {
    (fonts().ui.size() * 1.25)
        .round()
        .min(list_row_height() - 8.0)
}

/// The column a file row's disclosure triangle sits in, kept by every row of the objects
/// tree so the tags line up whether or not a row has one.
pub(crate) const CHEVRON_WIDTH: f32 = 14.0;

/// How far an archive member is indented past the file it belongs to.
pub(crate) const TREE_INDENT: f32 = 16.0;

/// The column a project field's name is written in, so the values line up whatever each
/// is called.
pub(crate) const FIELD_LABEL_WIDTH: f32 = 72.0;

/// How wide the Scratchpad view's list of pads is.
///
/// A fixed width and not a `ResizableContainer`, which is what the two splits in this app
/// that a reader can drag are: a dock tab that is not the active one in its panel is
/// unmounted, and a `ResizablePanel` forgets its size on unmount — so a draggable width
/// here would need a number kept at the root, the way `SplitRatio` is, for something
/// nobody has asked to be able to drag.
pub(crate) const PAD_LIST_WIDTH: f32 = 150.0;

/// The column the short format tag is written in. Fixed, so the names to the right of it
/// start at the same x whatever the tag says.
pub(crate) const TAG_WIDTH: f32 = 34.0;

/// The tag is written smaller than the row's own text.
pub(crate) const TAG_FONT_SIZE: f32 = 10.0;

/// How long a list row's tooltip waits before it appears. Zero, against
/// `TooltipContainer`'s 500ms default: a truncated name is read by sweeping the pointer
/// down the list. The filter toggles and the toolbar's two history buttons keep the
/// default: neither says anything the eye has already read off the control.
pub(crate) const TOOLTIP_DELAY: Duration = Duration::ZERO;

/// How long a symbol may be under analysis before the panes admit they are waiting. Only
/// ever *started* by a selection change (`use_analysis`) and never polled.
pub(crate) const SLOW_ANALYSIS: Duration = Duration::from_millis(180);

/// How many characters of a tab chip's name are drawn before the rest is elided.
///
/// A character count and not a width, deliberately: a `maximum_width` anywhere in a chip
/// makes it shrinkable, and a horizontal scroll view measures children against the space
/// *left*, so chips past the edge get no width and draw as a bare ×.
pub(crate) const CHIP_NAME_CHARS: usize = 40;

/// How far apart two branch lines running down the same rows are drawn.
pub(crate) const LANE_WIDTH: f32 = 7.0;

/// How thick a branch line is, horizontal run and arrowhead included.
pub(crate) const BRANCH_STROKE: f32 = 1.0;

/// How far the horizontal run reaches past the innermost lane, where the arrowhead sits.
pub(crate) const ARROW_WIDTH: f32 = 7.0;

/// The gap between an arrowhead's tip and the first digit of the address column.
pub(crate) const GUTTER_PAD: f32 = 3.0;

/// The length of each of the two strokes an arrowhead is made of, and how far each is
/// turned from the horizontal. Both pivot on the tip, so the pair is a `>`.
pub(crate) const ARROW_STROKE: f32 = 5.0;
pub(crate) const ARROW_ANGLE: f32 = 30.0;

/// How many rows above a row scrolled into view from the other pane are kept on screen.
pub(crate) const CONTEXT_ROWS: f32 = 3.0;

thread_local! {
    /// The two fonts the window is currently drawn in.
    ///
    /// A `State`, because **asking for a font is what subscribes the caller to it**: a
    /// write here repaints every scope that drew a glyph and no other. Thread-local and
    /// global rather than a context, since the readers are free functions and trait
    /// methods that may not run a hook; `State` is `!Send` and only the UI thread draws,
    /// so nothing off it may ask.
    static FONTS: State<Arc<Fonts>> =
        State::create_global(Arc::new(fonts::resolve(&Settings::load())));
}

/// The fonts to draw with, and a subscription to them for whoever asks.
pub(crate) fn fonts() -> Arc<Fonts> {
    FONTS.with(|fonts| Arc::clone(&fonts.read()))
}

/// Draw in these fonts from now on -- the one writer. Unlike `set_appearance` there is
/// nothing to invalidate alongside it: a cached highlight span carries colours, no font.
pub(crate) fn set_fonts(next: Fonts) {
    FONTS.with(|fonts| {
        let mut fonts = *fonts;
        fonts.set_if_modified(Arc::new(next));
    });
}

/// Applying one of the two fonts. freya takes families one at a time, pushing each onto
/// the element's own list and appending the parent's behind it, so the chain is set by
/// calling `font_family` in order of preference.
pub(crate) trait FontExt: TextStyleExt + Sized {
    fn font(mut self, font: &Font) -> Self {
        for family in &font.families {
            self = self.font_family(family.clone());
        }
        self.font_size(font.size())
    }

    fn assembly_font(self) -> Self {
        self.font(&fonts().mono)
    }
}

impl<T: TextStyleExt + Sized> FontExt for T {}

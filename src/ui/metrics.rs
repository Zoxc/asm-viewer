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

/// The device pixel grid the window is being drawn on, and a subscription to it.
///
/// freya keeps the scale factor on `Platform`, a root context the renderer provides --
/// `freya-winit` writes the window's own, multiplied by the user zoom, and `freya-testing`
/// writes whatever `with_scale_factor` was given -- so it is reachable from inside any
/// component's render, and reading it here subscribes that component the way asking for a
/// colour or a font does. Every stroke thin enough for the grid to matter goes through
/// [`Grid`]; see `src/pixels.rs` for why the edges and not the centre are what is rounded.
pub(crate) fn pixel_grid() -> Grid {
    Grid::new(*Platform::get().scale_factor.read())
}

/// The side of one of the three square toggle buttons in a filter bar: a row less the air
/// around it.
pub(crate) fn toggle_size() -> f32 {
    list_row_height() - 4.0
}

/// The side of the square the × on a document's tab is centred in.
///
/// **This is what makes the close a target you hit rather than one you aim at**: the air
/// around the glyph, four pixels of it on every side, is what a press that misses the mark
/// still lands in. It is written as the glyph plus that air rather than as a share of the
/// row, so growing one grows the other; the row is only the cap, since a target taller than
/// the bar it sits in would decide how tall the bar is.
pub(crate) fn close_target() -> f32 {
    (close_glyph() + 8.0).min(list_row_height() - 2.0)
}

/// The size the × on a tab is drawn at: the interface font, a third bigger.
///
/// Bigger than the text beside it because it is a *mark* and not a letter -- at the
/// interface size the multiplication sign is a thin scratch that reads as dirt on the tab.
/// [`close_target`] is written in terms of this rather than the other way round, so the air
/// around the glyph is what stays fixed when either the font or the row height moves.
pub(crate) fn close_glyph() -> f32 {
    (fonts().ui.size() * 1.33).round()
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
/// that a reader can drag are: a tab that is not the one on screen is unmounted, and a
/// `ResizablePanel` forgets its size on unmount — so a draggable width
/// here would need a number kept at the root, the way `SplitRatio` is, for something
/// nobody has asked to be able to drag.
pub(crate) const PAD_LIST_WIDTH: f32 = 150.0;

/// The column a Search hit's line number is written in, so the lines under one file start
/// at the same x whether the number has two digits or five.
pub(crate) const LINE_NUMBER_WIDTH: f32 = 38.0;

/// The column the short format tag is written in. Fixed, so the names to the right of it
/// start at the same x whatever the tag says.
pub(crate) const TAG_WIDTH: f32 = 34.0;

/// The tag is written smaller than the row's own text.
pub(crate) const TAG_FONT_SIZE: f32 = 10.0;

/// The air an archive row's member count keeps to its left. Part of the count's own
/// column and not of the name's: the name is the row's flex child, so whatever the count
/// does not claim is what the name grows into, and without the gutter a sidebar dragged
/// narrow runs the ellipsis straight into the digits.
pub(crate) const COUNT_GUTTER: f32 = 6.0;

/// How long a list row's tooltip waits before it appears. Zero, against
/// `TooltipContainer`'s 500ms default: a truncated name is read by sweeping the pointer
/// down the list. The filter toggles and the toolbar's two history buttons keep the
/// default: neither says anything the eye has already read off the control.
pub(crate) const TOOLTIP_DELAY: Duration = Duration::ZERO;

/// How wide the file finder's panel is: enough for a deep path and its name beside it,
/// and narrow enough that the window behind it is still recognisable.
pub(crate) const FINDER_WIDTH: f32 = 600.0;

/// How far under the top of the window it sits. Not centred down the window: a reader
/// typing a path is looking at the box, and the box is where an editor's is.
pub(crate) const FINDER_TOP: f32 = 80.0;

/// The air between the finder's panel and the box in it.
pub(crate) const FINDER_PAD: f32 = 8.0;

/// The corner the finder's panel is cut to, and how far its shadow is blurred: with the
/// app undimmed behind it, the shadow is the whole of what says the panel is over it, so
/// it is a soft one and not a hairline.
pub(crate) const FINDER_RADIUS: f32 = 6.0;
pub(crate) const FINDER_BLUR: f32 = 20.0;

/// The most rows it draws at once; the rest are scrolled to. A list as tall as the window
/// would cover the app it is drawn over.
pub(crate) const FINDER_ROWS: usize = 12;

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

/// How thick the rule a separator row draws across its middle is. Its own constant and
/// not [`BRANCH_STROKE`]: the two are drawn to the same weight today, but a rule that
/// reads against the pane and a branch line that reads against the rule are two
/// judgements, and the palette already keeps them apart.
pub(crate) const BLOCK_RULE_STROKE: f32 = 1.0;

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

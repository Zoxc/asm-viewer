use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};

use async_io::Timer;
use freya::code_editor::{
    CodeEditor, CodeEditorData, EditorLanguage, EditorSyntaxTheme, EditorThemePartialExt, Rope,
    SyntaxBlocks, SyntaxHighlighter, TextNode,
};
use freya::icons::lucide;
use freya::prelude::*;
use rfd::AsyncFileDialog;

use analysis::{
    open_files_streaming, Assembly, Instruction, LineInfo, Object, Progress, SpanKind, Symbol,
    SymbolData,
};

use crate::filter::{Filter, Matcher};
use crate::fonts::{self, Font, Fonts};
use crate::history::History;
use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
use crate::project::{self, Details, Document, Project, ProjectId, Recent, Selection, Session};
use crate::rows::RowSelection;
use crate::scratchpad::{
    Build, Dependency, Diagnostic, Ended, Failure, Half, Level, Problem, RunEvent, RunOutput,
    Running, Scratchpad, Stream,
};
use crate::settings::{Appearance, FontSetting, Settings, Theme as ThemeChoice};
use crate::source::{self, SourceFile};
use crate::tabs::{Positions, Tabs};
use crate::tree::{format_tag, Expansion, LoadId, Loads, ObjectTree, TreeRow, ARCHIVE_TAG};

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
fn list_row_height() -> f32 {
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
fn code_row_height() -> f32 {
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
fn filter_height() -> f32 {
    list_row_height() + 6.0
}

/// The side of one of the three square toggle buttons: a row less the air around it, so
/// the `Aa` and `.*` written inside them follow the interface font like everything else.
/// `filter_height`'s height for `filter_height`'s reason -- they are two parts of one bar.
fn toggle_size() -> f32 {
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
fn icon_size() -> f32 {
    (fonts().ui.size() * ICON_SCALE)
        .round()
        .min(list_row_height() - 8.0)
}

/// The column a file row's disclosure triangle sits in, and the width every row of the
/// objects tree gives up to it so that the tags below one another line up whether or not
/// the row has a triangle.
const CHEVRON_WIDTH: f32 = 14.0;

/// How far an archive member is indented past the file it belongs to. Past the triangle
/// and into the tag column, so the nesting is legible in a 300px sidebar without the name
/// starting halfway across it.
const TREE_INDENT: f32 = 16.0;

/// The column a project field's name is written in, so the values beside them line up
/// whatever each is called -- `SourceRow`'s line-number gutter's reason.
const FIELD_LABEL_WIDTH: f32 = 72.0;

/// The column the short format tag is written in. Fixed, so the names to the right of it
/// start at the same x whatever the tag says -- the reason `SourceRow`'s line-number
/// gutter is a fixed width and not a minimum.
const TAG_WIDTH: f32 = 34.0;

/// The tag is written smaller than the row's own text: it is a label on the row and not
/// what the row is called.
const TAG_FONT_SIZE: f32 = 10.0;

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
const TOOLTIP_DELAY: Duration = Duration::ZERO;

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
const SLOW_ANALYSIS: Duration = Duration::from_millis(180);

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
const CHIP_NAME_CHARS: usize = 40;

/// The width of one lane of the assembly view's branch gutter: how far apart two branch
/// lines running down the same rows are drawn.
const LANE_WIDTH: f32 = 7.0;

/// How thick a branch line is, horizontal run and arrowhead included. One logical pixel,
/// which is a hairline on a scaled display and the thinnest thing skia will draw solidly
/// on an unscaled one.
const BRANCH_STROKE: f32 = 1.0;

/// How far the horizontal run reaches past the innermost lane, which is where the
/// arrowhead sits.
const ARROW_WIDTH: f32 = 7.0;

/// The gap between an arrowhead's tip and the first digit of the address column, so that
/// the gutter reads as a column of its own rather than as decoration on the addresses.
const GUTTER_PAD: f32 = 3.0;

/// The length of each of the two strokes an arrowhead is made of, and how far each is
/// turned from the horizontal. Both of them pivot on the tip, so the pair is a `>`.
const ARROW_STROKE: f32 = 5.0;
const ARROW_ANGLE: f32 = 30.0;

/// How many rows above a row scrolled into view from the other pane are kept on screen.
/// A line landing against the top edge answers "what is this" without answering "where in
/// the function is it", which is half of why the two panes are side by side at all.
const CONTEXT_ROWS: f32 = 3.0;

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// Every colour the app draws, in one place.
///
/// There are two instances, [`Palette::LIGHT`] and [`Palette::DARK`], and [`palette`] is
/// how anything reaches whichever is current. The indirection was more of the point than
/// the struct was: the dark mode `Goals.md` asks for under *UI* is the same design at
/// dark-mode lightness rather than a second one, so it is one more `const` beside the
/// first and no edit at all to the call sites that name a colour.
///
/// **The dark values are the light ones carried over, not chosen again.** Every
/// relationship in the light palette is a relationship in the dark one: the header sits
/// one step off the pane on both (below it in light, above it in dark, which is the same
/// step through the same background), the pin is the focus at more alpha, the branch
/// hover is the branch line brightened towards the address column's hue, and each of the
/// eight code colours keeps its hue and its place in the ordering. What could *not* be
/// carried over literally are the translucent washes: `blend` composites them over the
/// pane, so the same alpha over a dark ground is a fraction of the contrast it had over
/// white, and each of them was re-judged against what it comes out as rather than
/// inverted. The tests below are what hold both halves of that -- a contrast floor for
/// every foreground on the surface it is drawn on, and a visible-difference floor for
/// every wash over the row under it.
///
/// It is also not freya's own theming (`Theme` / `ColorsSheet` / `define_theme!`).
/// `ColorsSheet` has a fixed set of fields naming none of these roles, a `define_theme!`
/// per row component would be styling machinery for elements nothing outside this file
/// ever styles, and -- the part that settles it -- the half of the palette this step is
/// about cannot be read from the element tree at all: the source pane's colours are baked
/// into a `SyntaxBlocks` by a highlighter that runs when a file is *loaded*, so they have
/// to be plain values available outside any component. See [`Palette::syntax`]. freya's
/// own theme is still given the matching *sheet* (`interface_theme`), because the handful
/// of freya components in the window -- the filter boxes, the scrollbars, the tooltips --
/// read their colours from it and nowhere else.
struct Palette {
    // Surfaces and chrome, carried over from the original floem styling.
    /// A pane's own body, and the tab header above the active one, which is white so
    /// that it reads as the top edge of that body rather than as part of the tab bar.
    pane_bg: Color,
    /// The interface text: every label that does not ask for a colour of its own. Set
    /// once on the root and *inherited* -- freya resolves an unset `color` from the
    /// parent's (`freya-core`'s `TextStyleState::from_data`), so the whole chrome follows
    /// from one call there. It is black in the light palette, which is exactly what the
    /// default was, so the light theme is unchanged by this field existing.
    text_fg: Color,
    header_bg: Color,
    hairline: Color,
    selected_bg: Color,
    object_hover_bg: Color,
    symbol_pane_bg: Color,
    symbol_hover_bg: Color,
    asm_pane_bg: Color,
    /// The pointer's own hover, on an instruction row and on a source line alike: both
    /// panes show code, and one colour for "the pointer is here" reads across them as one
    /// gesture.
    code_row_hover_bg: Color,
    /// The cross-view highlight: this row is what the row the pointer is on maps to on the
    /// other side. Weaker than the hover, and translucent like it, so a row carrying both
    /// comes out as the hover *over* this rather than as one or the other -- see `blend`.
    line_focus_bg: Color,
    /// The same highlight, made to stay by a click. The one colour in two strengths rather
    /// than two colours, because it is the one relationship: a pin is the position the
    /// reader asked to keep, and the pointer wandering off to a second one must not make
    /// the two indistinguishable.
    line_pin_bg: Color,
    /// The wash over the half of a panel a dragged tab would land in.
    drop_preview_bg: Color,
    /// A Lucide glyph in a dock tab header. A step lighter than the title beside it,
    /// because the icon is what the eye finds the tab by and the word is what tells it
    /// apart: an icon as dark as its label competes with it at the same size. It is the
    /// palette's one chrome *foreground* -- every other colour in this group is a surface,
    /// the interface text itself being freya's theme colour and inherited.
    icon_fg: Color,
    /// A filter toggle that is on, and one the pointer is over. Two shades of the header's
    /// own grey rather than a colour of their own: a 22px square is small enough that "this
    /// one is pressed" has to be read from how dark it is, and against `header_bg` these are
    /// the two steps that are legible without looking like a third kind of thing.
    toggle_on_bg: Color,
    toggle_hover_bg: Color,
    /// The wash behind a relocation link the pointer is over, lightening whatever row
    /// background is under it.
    link_hover_bg: Color,
    /// A branch line in the assembly gutter, with its corner and its arrowhead. Quieter
    /// than anything it runs beside: the gutter is a diagram of the listing and must not
    /// compete with it for the eye.
    branch_fg: Color,
    /// The same line while a branch of the row under the pointer runs down it. One hue in
    /// two strengths, the way `line_focus_bg` and `line_pin_bg` are and for the same
    /// reason -- it is one relationship, drawn twice over -- and that hue is the address
    /// column's own, since a branch names a place in the listing exactly as an address
    /// does.
    branch_hover_fg: Color,
    /// The run of rows a reader has picked out to copy. Translucent like the two washes
    /// above and composited with them by `blend`, since a row can be selected, pointed at
    /// and pinned at once -- and a *grey* where those two are blue, because it answers a
    /// different question: the blues say what this row maps to on the other side, and this
    /// says what would land on the clipboard.
    row_select_bg: Color,

    // The code colours, shared by both panes. Which syntactic category takes which of
    // them is [`Palette::syntax`]; these names are the category rather than either pane's
    // own vocabulary, because they answer for both.
    /// Where a thing is: the instruction addresses, and the source line-number gutter.
    address_fg: Color,
    /// What is being done: mnemonics and prefixes, source keywords, operators and types.
    keyword_fg: Color,
    /// What it is being done to: registers, and source variables, parameters and fields.
    operand_fg: Color,
    /// A value written out: immediates, and source numbers, booleans and constants.
    literal_fg: Color,
    /// A string literal. Source-only, and one of the two colours the assembly side has no
    /// equivalent for at all.
    string_fg: Color,
    /// A comment. Source-only, and the other one.
    comment_fg: Color,
    /// The glue between the operands: brackets, commas, and on the assembly side the
    /// operand-size keywords (`qword ptr`) that are glue in exactly the same way.
    punctuation_fg: Color,
    /// A name that names one thing: a relocation target in the assembly, a function,
    /// method or module in the source. Also the source pane's plain text, which is what
    /// most of a line is.
    name_fg: Color,
    /// A relocation link under the pointer, and the underline drawn beneath it.
    name_hover_fg: Color,

    /// What a pattern that will not compile, and the reason it will not, are written in.
    invalid_fg: Color,
}

impl Palette {
    const LIGHT: Palette = Palette {
        pane_bg: Color::WHITE,
        text_fg: Color::BLACK,
        header_bg: Color::from_rgb(245, 245, 245), // WHITE_SMOKE
        hairline: Color::from_rgb(211, 211, 211),  // LIGHT_GRAY
        selected_bg: Color::from_rgb(211, 211, 211),
        object_hover_bg: Color::from_rgb(144, 238, 144), // LIGHT_GREEN
        symbol_pane_bg: Color::from_rgb(243, 243, 228),
        symbol_hover_bg: Color::from_rgb(226, 226, 205),
        asm_pane_bg: Color::from_rgb(248, 248, 248),
        code_row_hover_bg: Color::from_argb(160, 228, 237, 216),
        line_focus_bg: Color::from_argb(70, 120, 160, 220),
        line_pin_bg: Color::from_argb(120, 120, 160, 220),
        drop_preview_bg: Color::from_argb(60, 105, 89, 132),
        icon_fg: Color::from_rgb(90, 90, 90),
        toggle_on_bg: Color::from_rgb(196, 196, 196),
        toggle_hover_bg: Color::from_rgb(225, 225, 225),
        link_hover_bg: Color::from_af32rgb(0.6, 255, 255, 255),
        branch_fg: Color::from_rgb(176, 188, 202),
        branch_hover_fg: Color::from_rgb(90, 116, 148),
        row_select_bg: Color::from_argb(80, 96, 110, 128),

        address_fg: Color::from_rgb(118, 141, 169),
        keyword_fg: Color::from_rgb(116, 94, 147),
        operand_fg: Color::from_rgb(87, 103, 65),
        literal_fg: Color::from_rgb(80, 107, 135),
        // The two source-only ones, picked to sit at the same lightness and the same low
        // saturation as the five above rather than at a highlighter's usual brightness:
        // a terracotta, which is the one warm hue nothing else here uses, and a sage
        // grey-green well clear of the operand olive.
        string_fg: Color::from_rgb(148, 98, 74),
        comment_fg: Color::from_rgb(128, 148, 128),
        punctuation_fg: Color::from_rgb(102, 102, 102),
        name_fg: Color::from_rgb(50, 50, 50),
        name_hover_fg: Color::from_rgb(105, 89, 132),

        invalid_fg: Color::from_rgb(176, 0, 32),
    };

    /// The same palette at dark-mode lightness.
    ///
    /// Read it beside `LIGHT` rather than on its own: every value here is the one above it
    /// turned through the background, and where a light value is a step *down* from the
    /// surface it sits on, the dark one is the same step *up*. The ground is 30/30/32, a
    /// hair cooler than neutral the way white is not, and the pane relationships are the
    /// light ones reflected -- the header a step off the body, the assembly pane a step
    /// off the header, the symbol pane the one faintly warm surface it already was.
    ///
    /// The eight code colours keep their hue and their ordering: purple for what is being
    /// done, olive for what it is done to, blue for a value written out, terracotta for a
    /// string, sage for a comment, grey for the glue, near-white for a name and the slate
    /// blue for an address. They move to where a dark ground can carry them -- lightness
    /// up, saturation down a little, since a colour that is legible on white at 40%
    /// lightness is a glare at 40% on black -- and the *relative* order of their weights
    /// is unchanged, which is what keeps an instruction and a statement reading at the
    /// same density (5e).
    const DARK: Palette = Palette {
        pane_bg: Color::from_rgb(30, 30, 32),
        text_fg: Color::from_rgb(232, 232, 232),
        header_bg: Color::from_rgb(40, 40, 43),
        hairline: Color::from_rgb(62, 62, 66),
        selected_bg: Color::from_rgb(66, 66, 72),
        // The one hue in the chrome, and the one the light palette states loudest: a row
        // the pointer is over in the objects tree. Carried over means the same green at
        // the same distance from its surface, which on a dark ground is a deep one.
        object_hover_bg: Color::from_rgb(48, 92, 52),
        symbol_pane_bg: Color::from_rgb(38, 38, 33),
        symbol_hover_bg: Color::from_rgb(52, 52, 44),
        asm_pane_bg: Color::from_rgb(34, 34, 36),
        // The four translucent ones. Each is stated as what it should come out as over
        // the pane rather than as the light value with its channels flipped: `blend` puts
        // 30/30/32 under them, so an alpha that lightened white by a little darkens a dark
        // ground by nothing at all. The colour under the alpha is therefore *lighter* than
        // the ground here where it is darker than white there, and the alphas are up.
        code_row_hover_bg: Color::from_argb(110, 120, 160, 110),
        line_focus_bg: Color::from_argb(80, 130, 170, 230),
        line_pin_bg: Color::from_argb(140, 130, 170, 230),
        drop_preview_bg: Color::from_argb(90, 150, 130, 190),
        icon_fg: Color::from_rgb(160, 160, 160),
        toggle_on_bg: Color::from_rgb(88, 88, 92),
        toggle_hover_bg: Color::from_rgb(60, 60, 64),
        // A wash that *lightens* the row under it, exactly as in the light palette --
        // "raised" is lighter on both grounds -- but a quarter of the way to white rather
        // than three fifths of it, which over a row background of 30 is the same step of
        // about fifty levels that 0.6 over white was.
        link_hover_bg: Color::from_af32rgb(0.25, 255, 255, 255),
        branch_fg: Color::from_rgb(96, 108, 124),
        branch_hover_fg: Color::from_rgb(150, 178, 210),
        row_select_bg: Color::from_argb(90, 150, 165, 185),

        address_fg: Color::from_rgb(132, 156, 186),
        keyword_fg: Color::from_rgb(178, 150, 214),
        operand_fg: Color::from_rgb(158, 180, 120),
        literal_fg: Color::from_rgb(130, 175, 214),
        string_fg: Color::from_rgb(214, 150, 120),
        comment_fg: Color::from_rgb(128, 158, 128),
        punctuation_fg: Color::from_rgb(150, 150, 150),
        // A step *below* the interface text, as the light `name_fg` is a step above black:
        // the code panes are what the eye rests on and the chrome around them names
        // itself once.
        name_fg: Color::from_rgb(216, 216, 216),
        name_hover_fg: Color::from_rgb(190, 168, 224),

        invalid_fg: Color::from_rgb(240, 110, 120),
    };

    /// This palette in the shape `freya-code-editor`'s highlighter wants, so that the
    /// source pane is coloured from here rather than by `EditorSyntaxTheme::light()` --
    /// a GitHub-ish theme with no relationship to the colours the assembly pane beside it
    /// is drawn in.
    ///
    /// Thirty-four capture fields onto seven colours. The buckets are the assembly side's
    /// own meanings, which four of its five turn out to have a source equivalent for --
    /// `address_fg` is the fifth, and it is shared already, being the instruction addresses
    /// and the source line-number gutter rather than anything a capture names:
    ///
    /// - `keyword_fg`, the mnemonic's purple, takes keywords and operators, and types
    ///   with them: in C a type name *is* a keyword, and in Rust a builtin one reads like
    ///   one. Attributes and labels are language directives and go here too.
    /// - `operand_fg`, the register's olive, takes variables, parameters and fields,
    ///   because a register is the assembly's variable. This is the bucket that does the
    ///   most for the two panes reading as one: an instruction is mostly registers and a
    ///   statement is mostly identifiers, so both sides end up with olive at the same
    ///   density, in the same place in the line.
    /// - `literal_fg`, the immediate's blue, takes numbers, booleans and constants -- and
    ///   the escapes inside a string, which are a character written another way.
    /// - `name_fg`, the relocation target's near-black, takes functions, methods, macros,
    ///   modules and constructors. That colour already *is* a function's name on the
    ///   assembly side, so `call helper` and `helper(x)` name it identically. It is also
    ///   the colour of text no capture claims, which is most of a line.
    /// - `punctuation_fg`, the `Other` span's grey, takes brackets and delimiters.
    ///
    /// One trap in how this is read back: `syntax.rs`'s `resolve_capture_color` decides a
    /// capture is unmapped by comparing its colour to `text` and then walks up the dotted
    /// name looking for a segment whose colour differs, so a child field set to the same
    /// colour as `text` inherits its *parent's* colour instead of the text colour.
    /// Nothing below is caught by it -- every field equal to `name_fg`, which is `text`
    /// here, has a parent equal to it as well, so the walk ends where it started -- but
    /// giving, say, `punctuation_bracket` the text colour while `punctuation` keeps the
    /// grey would silently paint brackets grey. What decides that is which fields *share*
    /// a value and not what the values are, so it is a property a second palette can
    /// break by accident -- a dark `punctuation_fg` that happened to land on the dark
    /// `name_fg` would do it -- and `captures_do_not_walk_up` asserts it for both.
    fn syntax(&self) -> EditorSyntaxTheme {
        EditorSyntaxTheme {
            text: self.name_fg,
            // A run of leading indentation, which the highlighter colours rather than
            // leaving to the text: `SourceRow` draws it as plain spaces, so this is the
            // one field here that is never actually seen.
            whitespace: self.punctuation_fg,
            attribute: self.keyword_fg,
            boolean: self.literal_fg,
            comment: self.comment_fg,
            constant: self.literal_fg,
            constructor: self.name_fg,
            escape: self.literal_fg,
            function: self.name_fg,
            function_macro: self.name_fg,
            function_method: self.name_fg,
            keyword: self.keyword_fg,
            label: self.keyword_fg,
            module: self.name_fg,
            number: self.literal_fg,
            operator: self.keyword_fg,
            property: self.operand_fg,
            punctuation: self.punctuation_fg,
            punctuation_bracket: self.punctuation_fg,
            punctuation_delimiter: self.punctuation_fg,
            punctuation_special: self.keyword_fg,
            string: self.string_fg,
            string_escape: self.literal_fg,
            string_special: self.string_fg,
            tag: self.keyword_fg,
            text_literal: self.string_fg,
            text_reference: self.name_fg,
            text_title: self.keyword_fg,
            text_uri: self.string_fg,
            text_emphasis: self.name_fg,
            type_: self.keyword_fg,
            variable: self.operand_fg,
            variable_builtin: self.keyword_fg,
            variable_parameter: self.operand_fg,
        }
    }
}

thread_local! {
    /// Which of the two palettes the window is currently drawn in.
    ///
    /// A `State` and not a plain value, and this is the whole of how a theme switch repaints:
    /// `State::read` subscribes the reactive context that is running, so **asking for a
    /// colour is what subscribes a component to the theme**. A scope that draws nothing
    /// coloured is not woken; every scope that does is, exactly once, wherever in the tree it
    /// sits and whatever built it -- a row a `VirtualScrollView` recycles as much as a tab.
    ///
    /// The two answers that were weighed against it, and why they lose:
    ///
    /// - **A context read threaded through the call sites.** freya's own components do this
    ///   (`use_applied_theme!` is a `use_consume` of its `State<Theme>`), and it is the
    ///   idiomatic answer -- but a hook must run unconditionally in a component body, and
    ///   `palette()` is called from `row_background`, `kind_color`, `hairline_border` and a
    ///   dozen other free functions, from inside `if` arms and render callbacks, and from
    ///   `Highlighted::new`, which is not a component at all. So it would be a hook line in
    ///   each of the twenty-one components *plus* the free functions left reading a static
    ///   anyway, and a component whose line was forgotten would be a patch of the old theme
    ///   that nothing points at.
    /// - **Re-rendering the tree from the root.** This does not work: freya marks a child
    ///   scope dirty only when its props change (`freya-core`'s `runner.rs`), and every view
    ///   here is a unit `Component` whose props never do -- that is the same memoisation that
    ///   makes a selection change cost only the tabs that read it. Forcing it with a `key`
    ///   that changes with the theme would work by *remounting*, which resets the scope
    ///   storage below it: the three filters, the objects tree's folds, every hover and every
    ///   scroll controller. A theme switch is not a reason to lose the reader's search.
    ///
    /// What it costs, honestly: `palette()` was a `&'static` and is now a thread-local
    /// lookup, a subscribe and a generational-box read -- some tens of nanoseconds, against
    /// perhaps a thousand calls in a full render of both panes. The reference it hands back is
    /// still `'static`, because the two palettes are `const`s and only the *choice* is state.
    ///
    /// It is thread-local because a `State` is `!Send` (freya's storage is `Rc`-based) and
    /// created global because it must outlive every scope that reads it -- including, in the
    /// tests, a `TestingRunner`'s whole tree. Only the UI thread draws, so only the UI thread
    /// ever touches this one; a second thread would silently get a second copy of it, which is
    /// why nothing off the UI thread may ask for a colour.
    static APPEARANCE: State<Appearance> = State::create_global(Appearance::Light);
}

/// The colours to draw with, and a subscription to the theme for whoever asks.
fn palette() -> &'static Palette {
    match appearance() {
        Appearance::Light => &Palette::LIGHT,
        Appearance::Dark => &Palette::DARK,
    }
}

/// The same subscription, for the two things that want the choice itself rather than a
/// colour: freya's own theme sheet, and the effect that keeps it in step.
fn appearance() -> Appearance {
    APPEARANCE.with(|appearance| *appearance.read())
}

/// Draw in this appearance from now on. **The only way to change it**, deliberately: the
/// source pane's spans are cached with the palette's colours already resolved into them,
/// so a switch has to empty `HIGHLIGHTED` as well, and that clear lives here rather than
/// at the call site that happens to switch the theme today. `set_if_modified_and_then` is
/// what makes the pair one step -- the cache is emptied exactly when the value really
/// changed, so setting the appearance it is already in costs nothing and re-highlights
/// nothing.
fn set_appearance(next: Appearance) {
    APPEARANCE.with(|appearance| {
        let mut appearance = *appearance;
        appearance.set_if_modified_and_then(next, || highlighted().clear());
    });
}

/// Which appearance a stored choice comes to on a windowing system that prefers
/// `preferred`.
///
/// The two enums are one distinction twice over -- a *choice* has three answers and a
/// *preference* has two -- so this is a rule and not a lookup: a reader who named a theme
/// is answered by their own answer, and `Desktop` is the only one of the three that is a
/// question at all.
///
/// Pure, and handed the platform's answer rather than asking for it, exactly as
/// `fonts.rs`'s `resolve_font` is handed the desktop's: the rule is then testable with no
/// window anywhere, which is the whole reason the read and the rule are two functions.
fn resolve_appearance(choice: ThemeChoice, preferred: PreferredTheme) -> Appearance {
    match choice {
        ThemeChoice::Light => Appearance::Light,
        ThemeChoice::Dark => Appearance::Dark,
        ThemeChoice::Desktop => match preferred {
            PreferredTheme::Light => Appearance::Light,
            PreferredTheme::Dark => Appearance::Dark,
        },
    }
}

/// The whole of the wiring between the stored choice and what is drawn: read both inputs,
/// resolve them, and write the answer down through [`set_appearance`] -- the one function
/// that may change the appearance, and so the one that empties `HIGHLIGHTED`. There is
/// deliberately no second path: a switch that reached the palette without passing through
/// there would leave the source pane's spans in the colours of the theme before it.
///
/// **Not a `use_hook`, and that is the point.** `Platform::preferred_theme` is a `State`
/// freya keeps from the windowing system itself -- winit answers `Window::theme()` on
/// Windows, macOS, X11 and Wayland alike, and freya re-sets the state on a `ThemeChanged`
/// event -- so *reading* it here subscribes this scope to it, and a desktop that goes dark
/// while the app is running re-runs this and repaints. That is a real gain over what this
/// replaced: the old answer came from a subprocess (`kreadconfig`, `gsettings`,
/// `defaults`) asked once at startup, which could not follow the desktop it was asking
/// about and could not be asked at all from a window that had not been opened yet. A
/// `use_hook` here would put that limitation back, one line at a time.
///
/// The *choice* arrives as a value rather than being loaded here, and since 9c it is a
/// value that can change: `Prefs` holds it, the settings page writes it, and the root
/// reads it -- so the same two-hop path that carries a desktop switch carries a click on
/// the Dark button. That is also what lets a test hand this a choice without the machine's
/// own settings file having a vote in what the test asserts.
///
/// Written from the render body rather than from an effect, deliberately: an effect lands
/// a frame late, and a frame late on a dark desktop is a white window flashing at someone
/// who asked for neither. The write is idempotent (`set_if_modified_and_then`), so the
/// render this runs in and every render after it that resolves the same way cost nothing.
fn use_theme(choice: ThemeChoice) {
    let preferred = *Platform::get().preferred_theme.read();

    set_appearance(resolve_appearance(choice, preferred));
}

/// The whole of the wiring between the settings and what they are settings *of*: the
/// appearance, the fonts, and `settings.toml`.
///
/// Three things come out of one state, and they are deliberately not three mechanisms.
/// The theme resolves in the render body, because `use_theme` must (a frame late is a
/// white flash); the fonts and the write go in one effect, because both are consequences
/// of the settled value rather than of the keystroke, and `fonts::resolve` allocates.
///
/// **The baseline is why a run that never opens the page writes no file.** `Settings::save`
/// has no policy in front of it by design -- a settings change is already as rare as a
/// deliberate action -- but "the settings as they were loaded" is not a change, and saving
/// it would create `settings.toml` on every first launch, which is `project.rs`'s rule
/// about a directory made by the first write that has something to say. So what the file
/// says is kept beside the hook and compared, exactly as `Saves::written` is.
///
/// `set_fonts` runs unconditionally, baseline or not: it is idempotent
/// (`set_if_modified`), and the alternative -- trusting that the thread-local was
/// initialised from the same file this hook loaded -- is two readers of one file agreeing
/// by luck.
fn use_settings(prefs: State<EditedSettings>) {
    use_settings_with(prefs, |settings: &Settings| settings.save());
}

/// The same, with the write handed in -- `use_analysis`/`use_analysis_with`'s shape and
/// for the same reason: [`Settings::save`] writes to the machine's real settings file, so
/// a test that mounted this would be editing the settings of whoever ran it.
fn use_settings_with(prefs: State<EditedSettings>, mut save: impl FnMut(&Settings) + 'static) {
    // What the file currently says: the settings as they were loaded, and thereafter
    // whatever was last written. It has to *move*, not sit at the loaded value -- a reader
    // who changes a setting and changes it back would otherwise leave the file holding the
    // middle answer, which is `Saves::written`'s rule and the same bug it exists for. An
    // `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let written = use_hook(|| Rc::new(RefCell::new(prefs.peek().settings())));
    let settings = prefs.read().settings();

    use_theme(settings.theme);

    use_side_effect_with_deps(&settings, move |settings: &Settings| {
        set_fonts(fonts::resolve(settings));

        let mut written = written.borrow_mut();
        if *settings != *written {
            *written = settings.clone();
            save(settings);
        }
    });
}

/// The sheet freya's own components read their colours from.
///
/// The palette above is deliberately *not* moved into freya's theming -- `ColorsSheet`
/// names none of these roles, and half the palette is consumed outside the element tree
/// entirely. But the window does hold a few freya components (the filter boxes, the
/// scrollbars, the resizable handle, the tooltips, the context menu), they read their
/// colours from this and from nothing else, and a white text box on a dark pane is not a
/// theme switch. So the base sheet follows the appearance, and the one override this app
/// has always made -- the tooltip's font size, which its theme hardcodes and no element
/// can set -- is applied on top of whichever it is.
fn interface_theme(appearance: Appearance) -> Theme {
    let mut theme = match appearance {
        Appearance::Light => light_theme(),
        Appearance::Dark => dark_theme(),
    };

    if let Some(tooltip) = theme.get::<TooltipThemePreference>("tooltip").cloned() {
        theme.set(
            "tooltip",
            TooltipThemePreference {
                font_size: Preference::Specific(fonts().ui.size()),
                ..tooltip
            },
        );
    }

    theme
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

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
fn fonts() -> Arc<Fonts> {
    FONTS.with(|fonts| Arc::clone(&fonts.read()))
}

/// Draw in these fonts from now on. Unlike `set_appearance` there is nothing to invalidate
/// alongside it -- `HIGHLIGHTED` caches spans with the palette's *colours* baked in, and a
/// span carries no font -- so this is the write and nothing else. It stays a function of
/// its own all the same, so that the one place fonts change is as findable as the one
/// place the theme does.
fn set_fonts(next: Fonts) {
    FONTS.with(|fonts| {
        let mut fonts = *fonts;
        fonts.set_if_modified(Arc::new(next));
    });
}

/// Applying one of the two fonts. freya takes font families one at a time, pushing
/// each onto the element's own list and appending the parent's behind it, so the
/// chain is set by calling `font_family` in order of preference.
trait FontExt: TextStyleExt + Sized {
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

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
struct Objects(State<Vec<Arc<Object>>>);

/// The files being read into [`Objects`] right now, shared through context so the sidebar
/// can say so.
///
/// It is a state of its own and not a field of the objects list because it is about
/// exactly what that list has *not* got: a file appears here when it is asked for and
/// leaves when nothing more is coming out of it, whether or not it produced anything at
/// all. See [`Loads`] for the model and [`open_binaries`] for what fills it in.
#[derive(Clone, Copy)]
struct Loading(State<Loads>);

/// The active document, shared through context.
///
/// Since 6c this *is* the active tab: everything on screen in the content area is the one
/// entry of [`Open`] that this names. Nothing beside it says which tab is active, and
/// since Step 1 there is nothing beside it saying which *file* is shown either — that was
/// `Shown`, the Source pane's own answer to the same question for its own strip, and the
/// two strips are now one.
///
/// `None` is nothing open, which is exactly an empty [`Open`].
#[derive(Clone, Copy)]
struct Active(State<Option<Document>>);

/// The tabs open in the content area, shared through context.
///
/// The list only; the active one is [`Active`]. Every entry is a place in a binary — an
/// object or a function — or a source file, and there is no "nothing" entry: having
/// nothing open is an empty list, which is the placeholder state.
///
/// Objects are in here alongside functions on purpose. A tab is a place the reader has
/// open, the sidebar's object rows have always *been* a selection, and giving them a tab
/// is what keeps `Active` equal to the active tab without a second "selected but not
/// open" state beside it. The tab for an object is named after the object and the
/// Assembly and Source panes show the same placeholders they always did for one.
#[derive(Clone, Copy)]
struct Open(State<Tabs<Document>>);

/// Which row each open tab's **assembly** side was left on, shared through context.
///
/// Beside [`Open`] rather than inside it, and beside it rather than inside
/// [`InstructionList`], for one reason each. Inside `Tabs` it would be a field of what
/// the strip draws, so a scroll of the reader's would re-render every tab; inside the
/// pane it would live and die with the component, which is precisely the bug this fixes —
/// one scroll controller is reused for every symbol, so a tab switch used to leave the
/// new function at the offset the old one was at. Here it outlives both the component and
/// any one document, which is what a *tab's* position has to do.
///
/// Keyed by [`Document`] — the same identity [`Open`] keys by, so an entry means "this
/// tab" for exactly as long as that tab is in the list, and never accidentally means a
/// second symbol of the same name in another object. It is also why the persisted form
/// cannot reuse the key and identifies its tabs by path and name instead (`project.rs`).
#[derive(Clone, Copy)]
struct AsmAt(State<Positions<Document>>);

/// Which row each open tab's **source** side was left on. [`AsmAt`]'s other half, and
/// keyed by the same document rather than by the file the pane happens to be showing:
/// a tab has two sides and each remembers its own row, so two functions compiled from one
/// file no longer share a position they have no reason to share.
#[derive(Clone, Copy)]
struct SrcAt(State<Positions<Document>>);

/// Where the reader has been, shared through context. Named `Hist` because `History` is
/// the type it holds, the same way `Active` holds a `Document`.
#[derive(Clone, Copy)]
struct Hist(State<History>);

/// The project the app is in, as the project view holds it.
///
/// Two of its three fields are `String`s where [`Details`] has `Option`s, because this is
/// what is in two text boxes and a text box has no third state: an empty box *is* how a
/// reader says "I have not said". [`OpenProject::details`] is the conversion and is the
/// one place the two spellings meet, so nothing else in the app has to know that an
/// unnamed project is an absent key rather than an empty string.
///
/// This is a state and not a value read out of `project.rs` on demand for the reason
/// every other context here is one: something renders it, so a change to it has to
/// re-render that something. Making it a state is also what let `Saves::given` stop being
/// a value carried across the save calls and become an ordinary baseline — a rename is
/// now a state change like any other, seen by the same observer, and written at once
/// because `name` lives in `project.toml`.
#[derive(Clone, Default, PartialEq)]
struct OpenProject {
    /// The directory the project is stored in, which is its identity. `None` until a
    /// project exists on disk at all — a run in which nothing has been opened or named
    /// has allocated none, deliberately.
    id: Option<ProjectId>,
    name: String,
    directory: String,
}

impl OpenProject {
    /// The project as it was found on disk.
    fn opened(id: ProjectId, project: &Project) -> OpenProject {
        OpenProject {
            id: Some(id),
            name: project.name.clone().unwrap_or_default(),
            directory: project
                .directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// What of this reaches `project.toml`.
    ///
    /// A box holding nothing but spaces is a box holding nothing: the alternative is a
    /// project named `" "`, which is anonymous everywhere it is drawn and named
    /// everywhere it is compared. Trimmed rather than refused, so the reader is never
    /// told off for a trailing space.
    fn details(&self) -> Details {
        Details {
            name: given(&self.name).map(str::to_owned),
            directory: given(&self.directory).map(PathBuf::from),
        }
    }
}

/// What a text box says, or `None` when it says nothing.
fn given(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

/// The open project, shared through context.
#[derive(Clone, Copy)]
struct Proj(State<OpenProject>);

/// The user's settings as the settings page has them.
///
/// [`OpenProject`]'s shape, and for its reason: `Settings` spells a family the reader has
/// not chosen as an *absent* key, and a text box has no third state -- an empty box **is**
/// how a reader says "I have not said". So the family is a `String` here and an
/// `Option<String>` there, [`EditedSettings::settings`] is the one place the two spellings
/// meet, and it trims, so a box of spaces is a box of nothing rather than a font family
/// named `" "`.
///
/// The size does *not* get the same treatment, and that is the one place this differs.
/// It is edited by a stepper rather than by a text box (see [`SettingsTab`]), so there is
/// no half-typed state to hold and no third answer for text that is not a number: an
/// `Option<f32>` here is an `Option<f32>` there and the mapping is the identity. The
/// theme is likewise the stored enum itself -- three buttons, three answers.
#[derive(Clone, Debug, Default, PartialEq)]
struct EditedSettings {
    theme: ThemeChoice,
    interface: EditedFont,
    fixed: EditedFont,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct EditedFont {
    family: String,
    /// In points, like the file and like [`Font::points`], so that the number on screen,
    /// the number the desktop answered and the number written down are one number.
    size: Option<f32>,
}

impl EditedSettings {
    /// The settings as they were read off disk.
    fn of(settings: &Settings) -> EditedSettings {
        EditedSettings {
            theme: settings.theme,
            interface: EditedFont::of(&settings.interface),
            fixed: EditedFont::of(&settings.fixed),
        }
    }

    /// What of this reaches `settings.toml` -- and, through [`fonts::resolve`], what is on
    /// screen. Total, deliberately: there is no state of this struct that does not say
    /// something, so nothing between the page and the file can be pending or invalid.
    fn settings(&self) -> Settings {
        Settings {
            theme: self.theme,
            interface: self.interface.setting(),
            fixed: self.fixed.setting(),
        }
    }
}

impl EditedFont {
    fn of(setting: &FontSetting) -> EditedFont {
        EditedFont {
            family: setting.family().unwrap_or_default().to_owned(),
            size: setting.size(),
        }
    }

    fn setting(&self) -> FontSetting {
        FontSetting {
            family: given(&self.family).map(str::to_owned),
            size: self.size,
        }
    }
}

/// The settings, shared through context.
///
/// A root context and not state inside the settings view, for the reason `Proj` is one:
/// the page is a dockable tab that may not be mounted, while the theme and the fonts are
/// resolved at the root on every render. The page edits this; [`use_settings`] is what
/// notices.
#[derive(Clone, Copy)]
struct Prefs(State<EditedSettings>);

/// Every state a project owns.
///
/// One value because a project switch touches all of them at once — it closes everything
/// that belonged to the project being left and restores everything that belongs to the
/// one being entered — and because the two halves of that, [`clear_project`] and
/// [`restore_project`], would otherwise be eight-argument functions called from three
/// places. It is `Copy` and holds nothing but handles, so passing it is passing eight
/// pointers.
#[derive(Clone, Copy)]
struct ProjectStates {
    proj: State<OpenProject>,
    objects: State<Vec<Arc<Object>>>,
    /// The files on their way into `objects`. It belongs to the project for the reason
    /// the objects do: leaving one abandons what was being read for it, including the
    /// files that have produced nothing yet and so are not in `objects` to be closed one
    /// by one.
    loading: State<Loads>,
    open: State<Tabs<Document>>,
    asm_at: State<Positions<Document>>,
    src_at: State<Positions<Document>>,
    active: State<Option<Document>>,
    history: State<History>,
}

/// The eight states as a component sees them: through the contexts the root provides, so
/// a view that switches projects needs none of them handed down to it.
fn use_project_states() -> ProjectStates {
    ProjectStates {
        proj: use_consume::<Proj>().0,
        objects: use_consume::<Objects>().0,
        loading: use_consume::<Loading>().0,
        open: use_consume::<Open>().0,
        asm_at: use_consume::<AsmAt>().0,
        src_at: use_consume::<SrcAt>().0,
        active: use_consume::<Active>().0,
        history: use_consume::<Hist>().0,
    }
}

/// The flattened symbol list, shared through context so the Symbols tab does not
/// have to rebuild it and the root does not have to re-render to hand it over.
#[derive(Clone, Copy)]
struct Symbols(Memo<SymbolList>);

/// Every object's text symbols flattened into one list, rebuilt only when the object
/// list changes. Compared by pointer so passing it around stays O(1).
#[derive(Clone)]
struct SymbolList(Arc<Vec<Symbol>>);

impl PartialEq for SymbolList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Everything the analysis crate has to say about the selected symbol, shared through
/// context so every pane that maps between source and assembly reads the same answer --
/// and worked out on a thread of its own, so no pane waits for it.
///
/// See [`use_analysis`] for where the work runs, how an answer nobody wants any more is
/// dropped, and why the state has three fields rather than one.
#[derive(Clone, Copy)]
struct Analysis(State<Analyzed>);

/// What the two panes are drawing, and what is being worked out for them.
///
/// Three fields and not "the answer for the current selection", because the answer for
/// the current selection is exactly what there is not while it is being worked out, and
/// the panes have to draw *something* in the meantime.
#[derive(Clone, Default)]
struct Analyzed {
    /// The symbol the panes are drawing and everything they draw it from.
    ///
    /// It is the selected symbol whenever the worker has caught up, and the one selected
    /// *before* it while it has not: a listing is replaced by the next listing, never by
    /// a blank pane. That ordering is the whole of the "quiet" requirement -- a symbol
    /// that decodes in two milliseconds still costs a frame or two to come back over a
    /// channel, and clearing the pane first would be a flash of empty on every single
    /// click. `None` is the selection not being a symbol at all, which is answered on the
    /// spot and never waits for anything.
    shown: Option<Studied>,
    /// The symbol the worker is working on, or `None` when it is idle. It is what tells
    /// the panes apart the two ways `shown` can be `None`: nothing selected, and nothing
    /// *yet*.
    pending: Option<Symbol>,
    /// Whether `pending` has been outstanding for [`SLOW_ANALYSIS`], and the only thing
    /// that ever puts a message on screen. A wait worth naming is one the reader has
    /// already noticed; anything shorter is noise, and a spinner that appears for one
    /// frame per click is worse than the wait it is describing.
    slow: bool,
}

impl PartialEq for Analyzed {
    fn eq(&self, other: &Self) -> bool {
        self.shown == other.shown && self.pending == other.pending && self.slow == other.slow
    }
}

/// What a pane draws, which is one decision and not two panes' worth of `if`s.
enum Showing<'a> {
    /// This analysis, which is the only state that has one.
    Listing(&'a Studied),
    /// Nothing to draw and a word for why.
    Message(&'static str),
    /// Nothing to draw and nothing worth saying: a wait too short to name, with no
    /// previous listing to leave up. Only reachable before the first symbol of a session
    /// has been analysed, since after that there always is one.
    Nothing,
}

impl Analyzed {
    /// What the panes draw. One answer for both of them, so they cannot disagree about
    /// which of the "nothing here" states the app is in.
    ///
    /// The order of the arms is the design. A wait long enough to name wins over the
    /// listing still on screen, because leaving the previous function up for a second
    /// under the next function's tab is a lie the reader would read; anything shorter
    /// loses to it, because replacing a listing with a blank for one frame is a flash of
    /// white on every click.
    fn showing(&self) -> Showing<'_> {
        match (&self.shown, &self.pending, self.slow) {
            (_, Some(_), true) => Showing::Message("Analysing..."),
            (Some(shown), _, _) => Showing::Listing(shown),
            (None, Some(_), false) => Showing::Nothing,
            (None, None, _) => Showing::Message("No symbol selected"),
        }
    }
}

/// Everything worked out about one symbol, in one value because it is worked out in one
/// go.
///
/// The disassembly and the line info travel together deliberately: they are asked for at
/// the same moment, they are read by the same two panes, and `AsmData` needs both to say
/// which source position an instruction came from. Handing them over separately is what
/// the `Lines` memo used to do, and it cost every selection change a second render -- the
/// disassembly arriving in one and the line info in the next.
#[derive(Clone)]
struct Studied {
    /// Which symbol this is the analysis of. The panes key their viewing position, their
    /// rows and their chip on it, so it travels with the answer rather than being read
    /// back out of `Sel`, which by then may be somewhere else entirely.
    symbol: Symbol,
    /// [`None`] for a symbol with no bytes to decode at all; the pane says so.
    assembly: Option<Arc<Assembly>>,
    /// Where this symbol's branches are drawn in the gutter. Derived from `assembly` and
    /// from nothing else, and built here beside it -- a lane layout that arrived a beat
    /// after the disassembly it belongs to would be drawn over the wrong rows.
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for Studied {
    fn eq(&self, other: &Self) -> bool {
        let same_assembly = match (&self.assembly, &other.assembly) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };

        self.symbol == other.symbol
            && same_assembly
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Studied {
    /// The whole of the expensive work, in the order it costs: `assembly` decodes and
    /// formats every instruction of the symbol, `line_info` builds this object's DWARF
    /// context on the first call against it (267 MB of it for `viewer-sample`) and walks
    /// the line program of every unit covering the symbol on each one.
    ///
    /// Nothing in here touches any UI state, which is what lets it run on a plain
    /// `std::thread`: it is handed a [`Symbol`] and hands back a value. See
    /// [`use_analysis`].
    fn new(symbol: Symbol) -> Studied {
        let assembly = symbol.data.assembly(&symbol.object);
        // An `Assembly`-less symbol has no rows to draw a gutter over, and `Lanes` is
        // built from the edges rather than from the assembly, so this needs no branch of
        // its own beyond the one that gets the edges.
        let lanes = Arc::new(match &assembly {
            Some(assembly) => Lanes::new(&assembly.edges, assembly.instructions.len()),
            None => Lanes::new(&[], 0),
        });
        let lines = SymbolLines::new(&symbol);

        Studied {
            symbol,
            assembly,
            lanes,
            lines,
        }
    }
}

/// What DWARF says about the selected symbol's instructions, or `None` when it says
/// nothing, and which of the files it names the Source pane draws beside it.
///
/// Worked out once for all its readers rather than once per pane: `Object::line_info`
/// walks the line program of every unit covering the symbol again on each call, even
/// though the DWARF context itself is built only once.
///
/// The file is worked out *here*, beside the info it comes from, rather than by whoever
/// wants it. The answer arrives from a worker thread, so anything reading `Sel` and this
/// together sees them disagree for as long as the work takes -- and asking the previous
/// symbol's `LineInfo` where the new symbol starts answers with the previous symbol's
/// file, which would open a tab for a file that has nothing to do with what was clicked.
/// Inside one value the two cannot disagree.
#[derive(Clone)]
struct SymbolLines {
    info: Option<Arc<LineInfo>>,
    /// Which of the files the symbol touches the Source pane draws: the one its first
    /// instruction was compiled from, which is the function's own file rather than one of
    /// the headers it inlined further in. A symbol whose entry instructions belong to no
    /// row at all -- a compiler-generated prologue is enough for that -- falls back to the
    /// first file the rows name, and one whose rows name no file at all has none.
    file: Option<Arc<str>>,
}

impl PartialEq for SymbolLines {
    fn eq(&self, other: &Self) -> bool {
        let same_info = match (&self.info, &other.info) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };

        // The file is compared by its text, not by pointer, for the reason `LinePos` is:
        // a path is a value. Two `LineInfo`s naming one file hold two `Arc<str>`s of it.
        same_info && self.file == other.file
    }
}

impl SymbolLines {
    /// The line info for `symbol`, with the file the Source pane draws beside it.
    fn new(symbol: &Symbol) -> SymbolLines {
        let info = symbol.data.line_info(&symbol.object);
        let file = info.as_ref().and_then(|info| {
            info.row_at(symbol.data.address)
                .and_then(|row| row.file)
                .and_then(|file| info.files().get(file))
                .or_else(|| info.files().first())
                .cloned()
        });

        SymbolLines { info, file }
    }
}

/// A source position the two panes point at together.
///
/// The file is half the identity rather than decoration: a symbol's rows can name several
/// files -- an inlined header's line 42 is not line 42 of the file the source pane has
/// open -- so a line number alone would light up the wrong row. Compared by its text and
/// not by pointer, unlike every other `Arc` the UI passes around: this is a position and
/// not an object, and two `LineInfo`s naming one file hold two `Arc<str>`s of its path.
#[derive(Clone, PartialEq)]
struct LinePos {
    file: Arc<str>,
    line: u32,
}

/// Which row put the focus where it is.
///
/// Paired with the position in `LineFocus`, and it is the pair a row compares against
/// before giving the focus up again (`release_focus`): two instructions compiled from one
/// source line share a position but not an address, so the origin is what tells them
/// apart, and two source rows differ in the position already.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusOrigin {
    /// The assembly row for the instruction at this address.
    Instruction(u64),
    /// The source row for the focused line itself.
    Source,
}

/// The source position the pointer is pointing at, and which side it points from.
#[derive(Clone, PartialEq)]
struct LineFocus {
    at: LinePos,
    from: FocusOrigin,
}

/// The cross-view focus, shared through context: hovering an instruction puts the position
/// it was compiled from here, hovering a source line puts that line here, and both panes
/// light up whatever matches. `None` while the pointer is on neither.
#[derive(Clone, Copy)]
struct Focused(State<Option<LineFocus>>);

/// Give up the focus a row set when the pointer leaves it, unless another row has taken it
/// over since.
///
/// A row cannot simply clear the focus. `pointerout` on the row being left and
/// `pointerover` on the one being entered are sorted against each other by an
/// `EventName::cmp` (freya-core `events/name.rs`) that answers `Less` for both of them, so
/// which of the two runs first is not something to lean on. Clearing only what this row
/// itself put there is right in either order -- and comparing the whole focus, origin as
/// well as position, is what keeps two instructions of one source line apart: they set the
/// same position, so the row being left would otherwise blank the highlight the row being
/// entered had just set.
fn release_focus(mut focused: State<Option<LineFocus>>, mine: Option<&LineFocus>) {
    if mine.is_some() && focused.peek().as_ref() == mine {
        focused.set(None);
    }
}

/// One of the two panes that show code.
///
/// Not `Tab`, which names nine views of which seven have nothing to answer here: this is the
/// side of a mapping, and a mapping has exactly two.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Assembly,
    Source,
}

/// The source position a click fixed the two panes on.
///
/// A pin is the hover of 5b made to stay. Hovering is how the mapping is explored and it
/// has to end when the pointer moves on; clicking is how a reader says *this one*, and a
/// highlight that evaporated the moment the pointer left for the pane it had just scrolled
/// would be answering a question nobody asked. The two live side by side rather than one
/// replacing the other, so a pin never costs the hover and the hover can never quietly
/// undo a pin: both light their rows, the pin more strongly.
#[derive(Clone, PartialEq)]
struct Pin {
    at: LinePos,
    /// The pane that has yet to scroll `at` into view -- always the other one from the
    /// pane clicked -- and `None` once it has, or once it has decided there is nothing
    /// there to scroll to. Carried in the pin rather than in a state of its own because
    /// the request and the highlight are one gesture; keeping it separate from `at` is
    /// what makes clicking the same line twice two requests, so a pane the reader has
    /// scrolled away from by hand comes back.
    reveal: Option<Pane>,
}

/// The pinned position, shared through context. `None` until something is clicked, and
/// again whenever the selection changes (`use_clear_focus`).
#[derive(Clone, Copy)]
struct Pinned(State<Option<Pin>>);

/// Take the request `pane` is owed, if it is owed one.
///
/// The pin itself stays where it is -- it is what both panes light up, for as long as the
/// symbol is on screen -- and only the request to scroll is cleared, so that it is answered
/// once. Clearing it from inside the effect that reads it wakes that effect one more time,
/// which finds nothing and stops; the alternative, a counter that says "this is a different
/// click", would leave every pane having to remember which counter it last acted on.
fn take_reveal(mut pinned: State<Option<Pin>>, pane: Pane) -> Option<LinePos> {
    let at = {
        // `read` rather than `peek` on purpose: this is the subscription that wakes the
        // caller's effect on the next click, so it has to happen before any early return.
        let pin = pinned.read();
        match pin.as_ref() {
            Some(pin) if pin.reveal == Some(pane) => pin.at.clone(),
            _ => return None,
        }
    };

    if let Some(pin) = pinned.write().as_mut() {
        pin.reveal = None;
    }

    Some(at)
}

/// Bring the row at `index` into view, and leave the scroll alone when it already is.
///
/// A `VirtualScrollView` counts its offset *down* from zero -- `-offset / item_size` is the
/// first row it builds -- so a row's own offset is the negative of its distance from the
/// top, and whatever is set here is clamped against the content on the next layout
/// (`get_corrected_scroll_position`), which is why the arithmetic need not know how long
/// the list is.
///
/// Nothing moves while the row is already on screen and clear of the top edge. The gesture
/// this answers is reading down a function clicking one instruction after another: their
/// lines are in view on the other side already, and a pane that re-scrolled on every one of
/// them would be moving under the reader for no reason.
fn reveal_row(controller: &mut ScrollController, viewport: f32, index: usize) {
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -scrolled as f32;
    let height = code_row_height();
    let row = index as f32 * height;
    let margin = CONTEXT_ROWS * height;

    if row >= top + margin && row + height <= top + viewport {
        return;
    }

    controller.scroll_to_y(-((row - margin).max(0.0) as i32));
}

/// The row at the top of a code pane scrolled to `offset`, and the offset that puts `row`
/// there — the one place the two units meet.
///
/// [`code_row_height`] and not the list's: both callers are the two code panes
/// (`use_kept_position`, and `reveal_row` above), a sidebar list neither keeping a
/// per-tab position nor having a row to reveal.
///
/// A `VirtualScrollView`'s offset counts *down* from zero, so the arithmetic is a
/// negation and a divide by [`code_row_height`], which is those panes' `item_size`. Rounded
/// *down*, which is the half-row a position in rows gives up and the direction to give it
/// up in: the row at the top edge is the one the reader is looking at even when it is only
/// half on screen, and coming back to the one below it would lose the half they could see.
fn row_at(offset: i32) -> usize {
    ((-offset).max(0) as f32 / code_row_height()) as usize
}

fn row_offset(row: usize) -> i32 {
    -((row as f32 * code_row_height()) as i32)
}

/// Keep `controller` pointed at the row `tab` was last left at, and keep [`Positions`]
/// told where it is now.
///
/// Both panes' halves of "a viewing position per tab", from the one place: a pane holds
/// one scroll controller and shows one tab at a time, so switching tab means writing the
/// outgoing tab's row down and putting the incoming tab's row back. `length` is what the
/// pane is holding *now*, which is what makes the answer a row of this listing rather
/// than of the one it was saved from.
///
/// Two things make it work, and both are about *when* rather than what:
///
/// - **The effect is subscribed to the pane's own scroll**, because reading the
///   controller's position is a `State::read` inside it (`ScrollController`'s
///   `From<..> for (i32, i32)`, which is the only way to ask). So every scroll the reader
///   makes wakes this and is written down as it happens, rather than only on the way out
///   of the tab — which is what makes the position survive the window simply being closed,
///   and what makes it survive the pane unmounting (which the assembly pane does whenever
///   the selection is an object, taking its controller with it).
/// - **The tab the controller is *holding* is tracked here**, in a plain `Rc<RefCell<..>>`
///   rather than a `State`, and is not the same thing as the tab the app is showing. The
///   two differ for exactly one run of this effect — the one that has to move the view —
///   and every other write goes under the held tab, so a scroll that lands between a tab
///   switch and this effect cannot be written down against the tab it is not from. It is
///   not a `State` because nothing renders from it and writing one here would cost the
///   pane a second render on every switch. `open` is what keeps that from resurrecting a
///   tab that has just been closed: the run after a close is holding one, and the three
///   closing functions have already forgotten it.
///
/// **A [`Pin::reveal`] wins over a remembered position, and needs nothing to make it.**
/// The two never ask at the same moment: this moves the view only when the tab changes,
/// and a reveal is asked for by a click in the *other* pane, which changes no tab —
/// while a change of document, which does, drops the pin outright (`use_clear_focus`).
/// When a reveal does scroll, this effect wakes on the scroll it made and records it, so
/// the last thing the reader was shown is what the tab is remembered at. The memory
/// follows the reveal rather than fighting it.
fn use_kept_position<T: Clone + PartialEq + 'static>(
    mut positions: State<Positions<T>>,
    open: State<Tabs<T>>,
    mut controller: ScrollController,
    tab: &T,
    length: usize,
) {
    // Not `use_state`: see above. `use_hook` runs its initializer once per component, so
    // this is the pane's own memory of which tab its controller is scrolled for.
    let held = use_hook(|| Rc::new(RefCell::new(None::<T>)));

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would hold the first tab this pane ever showed for as long as it lived.
    use_side_effect_with_deps(&(tab.clone(), length), move |(tab, length): &(T, usize)| {
        // Reading the controller's position is what subscribes this effect to the pane's
        // scroll, so it has to happen before anything can return early.
        let (_, offset) = <(i32, i32)>::from(controller);
        let row = row_at(offset);

        // Cloned out of the borrow rather than held across the `borrow_mut` below, which
        // panics exactly the way a `State` guard held across a write does.
        let holding = held.borrow().clone();
        let switching = holding.as_ref() != Some(tab);
        let known = positions.peek().at(tab);
        let back_to = positions.peek().row(tab, *length);

        // Whose row the offset above is, and where this run has to move the view to.
        let (owner, moving) = match (&holding, known) {
            // Still showing the tab the controller is scrolled for -- a scroll, a resize,
            // a re-render. The offset is that tab's own and nothing moves.
            (Some(held), _) if held == tab => (Some(tab.clone()), None),
            // A switch, with a row for the tab arriving: the offset belongs to the one
            // being left, and the one arriving goes back to where it was.
            (Some(out), Some(_)) => (Some(out.clone()), Some(back_to)),
            // A switch onto a tab never seen: the top, and pointedly not wherever the tab
            // before it had got to, which is the whole bug this hook exists for.
            (Some(out), None) => (Some(out.clone()), Some(0)),
            // This pane's first run, on a tab it has a row for: a remount, or a session
            // just restored. Nothing to write down -- a fresh controller sits at the top,
            // which is not where this tab was -- and everything to put back.
            (None, Some(_)) => (None, Some(back_to)),
            // First run with nothing remembered: leave the view where it is. It is at the
            // top already, and this runs a beat *after* the pane's first render, so a
            // scroll to the top here would undo a wheel that got in before it.
            (None, None) => (Some(tab.clone()), None),
        };

        if let Some(owner) = owner {
            // Only for a tab that is still open, which is why the list is an argument
            // here at all: `close_tab` forgets a tab's position and then moves to a
            // neighbour, so the run that follows is holding a tab that has gone -- and
            // writing its row down would put it straight back, keyed by a `Document`
            // that holds a whole `Object`. That the last scroll before a close is lost
            // with it is the right answer twice over: there is no tab to bring it back
            // for, and the file it pointed into may be being let go of in the same
            // breath (`close_binary`).
            let still_open = open.peek().find(&owner).is_some();
            // And only when it has actually moved. `State::write` notifies whether or not
            // the value changes, and this runs on every scroll event, so writing back what
            // is already there would wake the save observer for a pointer sitting still.
            let at = positions.peek().at(&owner);
            if still_open && at != Some(row) {
                positions.write().remember(owner, row);
            }
        }
        if switching {
            *held.borrow_mut() = Some(tab.clone());
        }
        if let Some(row) = moving {
            // A no-op when the view is there already, and otherwise a write this effect
            // is subscribed to: it wakes once more, finds the tab it is holding is the
            // tab it is showing, and writes the row down.
            controller.scroll_to_y(row_offset(row));
        }
    });
}

/// The run of rows a reader has picked out to be copied, and which pane it is in.
///
/// One selection for the whole app rather than one per pane, and that is what the `pane`
/// is for. Ctrl+C has to have exactly one answer, and the pane it belongs to is not
/// something a reader can see: two runs lit at once in two panes, with the keyboard focus
/// -- which nothing draws -- deciding which of them lands on the clipboard, is a coin
/// flip dressed up as a feature. Picking a row in one pane therefore drops whatever the
/// other had, the way selecting in one text field drops the selection in the next.
#[derive(Clone, Copy, PartialEq)]
struct Marks {
    pane: Pane,
    rows: RowSelection,
}

/// The picked-out rows, shared through context: written by the row the pointer is on and
/// read by the list that draws it and copies it. `None` until something is picked, and
/// again whenever the listing under it is replaced.
#[derive(Clone, Copy)]
struct Marked(State<Option<Marks>>);

/// Whether Shift is held, which is what turns a click into "reach to here".
///
/// Its own state, and written from the root's *global* key handlers, because a pointer
/// event carries no modifiers at all: `MouseEventData` is a location and a button
/// (freya-core `events/data.rs`), so the only way to know what the keyboard was doing
/// when a row was clicked is to have been watching it. freya-edit does the same thing for
/// the same reason -- `TextDragging::shift`, fed by `EditableEvent::KeyDown` -- but from
/// the focused editor's own handlers; global ones here so that the first shift-click
/// after a pane is reached works, rather than only the ones after it has the focus.
#[derive(Clone, Copy)]
struct Shift(State<bool>);

/// The rows picked out in `pane`, and nothing when the selection is the other pane's.
///
/// Reads rather than peeks: this is what a list calls to work out what its rows draw, so
/// it is the subscription that repaints them as the run grows.
fn marked_rows(marked: State<Option<Marks>>, pane: Pane) -> Option<RowSelection> {
    (*marked.read())
        .filter(|marks| marks.pane == pane)
        .map(|marks| marks.rows)
}

/// Start a run at `row`, or -- with Shift held, in the pane the run is already in --
/// reach out to it from wherever that run started.
fn mark_press(mut marked: State<Option<Marks>>, shift: bool, pane: Pane, row: usize) {
    let rows = match *marked.peek() {
        Some(marks) if shift && marks.pane == pane => marks.rows.extended(row),
        _ => RowSelection::at(row),
    };

    marked.set_if_modified(Some(Marks { pane, rows }));
}

/// Sweep the run out to `row`, which does nothing at all unless the button is still down
/// on it -- the pointer crossing a row is the hover, and the hover is answered elsewhere.
fn mark_drag(mut marked: State<Option<Marks>>, pane: Pane, row: usize) {
    let Some(marks) = *marked.peek() else {
        return;
    };
    if marks.pane != pane {
        return;
    }

    marked.set_if_modified(Some(Marks {
        rows: marks.rows.dragged_to(row),
        ..marks
    }));
}

/// End the gesture, wherever in the window the button came up. The run stays: letting go
/// is the end of the drag and not the end of the selection.
///
/// The read is a `let` of its own and not the scrutinee of an `if let`, which is the shape
/// this was written in first and which panicked on every mouse-up: a `State`'s `peek`
/// hands back a guard borrowing the state, and the temporary holding an `if let`'s
/// scrutinee lives until the end of its *body*, so the write inside was a mutable borrow
/// taken while that one was still out (`writable_utils.rs:96`). `mark_drag`'s `let ...
/// else` and `mark_press`'s `match` end their temporaries with the statement, which is
/// why the same code was fine there and why nothing about it is visible at the call site.
/// `Marks` is `Copy`, so binding it first costs nothing at all.
fn mark_release(mut marked: State<Option<Marks>>) {
    let current = *marked.peek();

    if let Some(marks) = current {
        marked.set_if_modified(Some(Marks {
            rows: marks.rows.released(),
            ..marks
        }));
    }
}

/// Drop `pane`'s selection, and leave the other pane's alone.
///
/// Called when the listing itself is replaced -- another symbol, another file -- because
/// the run is a range of row *indices*, and rows 40 to 60 of the function the reader just
/// left are not a thing to keep highlighted in the one they arrived at.
fn unmark(mut marked: State<Option<Marks>>, pane: Pane) {
    if marked.peek().is_some_and(|marks| marks.pane == pane) {
        marked.set(None);
    }
}

/// What Ctrl+C, Ctrl+A and Escape do to a listing's selection.
///
/// One handler for both panes, differing in the pane it answers for and in how a row of
/// it reads as text. It goes on the pane's own focusable box rather than on a global key
/// handler, which would fire while a filter bar had the keyboard: two things writing the
/// clipboard from one Ctrl+C, with the global one sorting last (`EventName::cmp`) and so
/// winning, would take a copy out of the filter box and give back a page of disassembly.
fn on_listing_key(
    marked: State<Option<Marks>>,
    pane: Pane,
    rows: usize,
    line: impl Fn(usize) -> String + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    let mut marked = marked;

    move |e: Event<KeyboardEventData>| {
        let command = e.modifiers.contains(Modifiers::ctrl_or_meta());

        match &e.key {
            Key::Character(character) if command && character == "c" => {
                let picked = (*marked.peek()).filter(|marks| marks.pane == pane);
                if let Some(picked) = picked {
                    // Failing silently is the only answer there is: the clipboard is a
                    // root context freya-winit fills in from the window's display handle,
                    // so a platform that gave it none has none, and there is nowhere in a
                    // listing to say so.
                    Clipboard::set(picked.rows.text(&line)).ok();
                }
            }
            Key::Character(character) if command && character == "a" => {
                if let Some(rows) = RowSelection::all(rows) {
                    marked.set(Some(Marks { pane, rows }));
                }
            }
            Key::Named(NamedKey::Escape) => unmark(marked, pane),
            _ => {}
        }
    }
}

/// One instruction as one line of text, which is what the row draws and so what a copy of
/// the row has to be: the address column, then the formatted instruction with the
/// relocation target's name already substituted into its operand.
///
/// The arrow gutter is left out, being a picture of the branches rather than part of the
/// listing. The trailing name is the one case where the row shows something the format
/// spans do not hold -- a relocation the formatter offered no operand to substitute into
/// is drawn as a label after the whole instruction, and is copied in the same place.
fn asm_line(instruction: &Instruction) -> String {
    let mut text = format!("{:016X} ", instruction.address);
    text.extend(instruction.format.iter().map(|(span, _)| span.as_str()));

    if instruction.relocation_span.is_none() {
        if let Some(target) = &instruction.relocation {
            text.push(' ');
            text.push_str(target.display());
        }
    }

    text.truncate(text.trim_end().len());
    text
}

/// A loaded, highlighted source file, compared by pointer.
#[derive(Clone)]
struct SourceText(Arc<Highlighted>);

impl PartialEq for SourceText {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A disassembled symbol, where its branches are drawn and what says where its
/// instructions came from, compared by pointer.
#[derive(Clone)]
struct AsmData {
    assembly: Arc<Assembly>,
    object: Arc<Object>,
    /// The gutter layout for this symbol's branches. Derived from `assembly` and never
    /// from anything else, so the two are always in step -- but compared on its own all
    /// the same, since nothing in the type system says so.
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.object, &other.object)
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl AsmData {
    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.info.as_ref()?;
        let row = lines.row_at(self.assembly.instructions[index].address)?;
        Some(LinePos {
            file: lines.files().get(row.file?)?.clone(),
            line: row.line?,
        })
    }
}

/// What the source rows are built from: the file's text and highlighting, which file it
/// is -- a row hovered points the assembly pane at a position, and a line number is not
/// one on its own -- and which of its lines the assembly pane is pointing at, by the
/// pointer and by a click.
///
/// Both of those are line numbers rather than positions because the file has already been
/// matched: a position naming another of the symbol's files is not a row of this one, and
/// answering that once here beats answering it per visible row.
#[derive(Clone)]
struct SourceData {
    source: SourceText,
    file: Arc<str>,
    focus: Option<u32>,
    pin: Option<u32>,
    /// The run of rows picked out to be copied, or `None` when the selection is the
    /// assembly pane's or there is none.
    rows: Option<RowSelection>,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.focus == other.focus
            && self.pin == other.pin
            && self.rows == other.rows
    }
}

/// What the instruction rows are built from: the disassembly, the two positions the
/// source pane is pointing at, and the branches of the row the pointer is on. Kept apart
/// from `AsmData` so that a hover, which changes this and not that, cannot re-run anything
/// the disassembly drives.
#[derive(Clone, PartialEq)]
struct AsmRows {
    data: AsmData,
    focus: Option<LinePos>,
    pin: Option<LinePos>,
    /// The edges starting or ending at the hovered row, which every row the gutter draws
    /// them through has to know about. Worked out once here rather than per row, and
    /// empty while the pointer is on no row at all -- the overwhelmingly common case, in
    /// which the gutter is drawn in one colour and this costs nothing.
    touching: Vec<PlacedEdge>,
    /// The run of rows picked out to be copied, or `None` when the selection is the source
    /// pane's or there is none.
    rows: Option<RowSelection>,
}

/// What one row draws in the gutter: its own lanes, and how much of it belongs to a branch
/// of the row under the pointer.
#[derive(Clone, Copy, PartialEq)]
struct RowArrows {
    lanes: RowLanes,
    lit: Lit,
}

impl AsmRows {
    /// Whether the instruction at `index` is what the pointer is on in the source pane,
    /// and whether it is what a click pinned there. One source line is many instructions
    /// and every one of them lights up, so this asks each row's own position rather than
    /// looking for the first match.
    ///
    /// An instruction the debug info places nowhere is neither, which `Option`'s own `==`
    /// would get wrong in the case where nothing is focused either.
    fn lit(&self, index: usize) -> (bool, bool) {
        let Some(at) = self.data.position(index) else {
            return (false, false);
        };
        (
            self.focus.as_ref() == Some(&at),
            self.pin.as_ref() == Some(&at),
        )
    }

    /// Whether the row at `index` is one of the picked-out run.
    fn marked(&self, index: usize) -> bool {
        self.rows.is_some_and(|rows| rows.contains(index))
    }

    /// What the row at `index` draws in the gutter.
    fn arrows(&self, index: usize) -> RowArrows {
        RowArrows {
            lanes: self.data.lanes.row(index),
            lit: lanes::lit(&self.touching, index),
        }
    }
}

fn bottom_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.5,
        left: 0.0,
    })
}

fn right_hairline() -> Border {
    Border::new().fill(palette().hairline).width(BorderWidth {
        top: 0.0,
        right: 0.5,
        bottom: 0.0,
        left: 0.0,
    })
}

/// The body of a tab that has nothing to show. Takes an owned string as well as a
/// literal, because one of these messages names the file it could not find.
fn placeholder(text: impl Into<String>) -> Element {
    let text: String = text.into();
    rect()
        .expanded()
        .padding(5.0)
        .background(palette().pane_bg)
        .child(label().text(text))
        .into()
}

fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

/// `top` composited over `bottom`, both of them translucent.
///
/// An element has one background, so a row that is both hovered and pointed at from the
/// other pane would need a second rect inside it purely to carry the second colour.
/// Compositing the two here paints the pixels those two rects would have, since what lies
/// under both is the pane's own background either way.
fn blend(top: Color, bottom: Color) -> Color {
    let (top_alpha, bottom_alpha) = (top.a() as f32 / 255.0, bottom.a() as f32 / 255.0);
    let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
    if alpha == 0.0 {
        return Color::TRANSPARENT;
    }

    let channel = |top: u8, bottom: u8| {
        ((top as f32 * top_alpha + bottom as f32 * bottom_alpha * (1.0 - top_alpha)) / alpha)
            .round() as u8
    };

    Color::from_argb(
        (alpha * 255.0).round() as u8,
        channel(top.r(), bottom.r()),
        channel(top.g(), bottom.g()),
        channel(top.b(), bottom.b()),
    )
}

/// The background of a code row: the pointer's own hover, the run of rows picked out to be
/// copied, the cross-view highlight it got from the other pane, the stronger one a click
/// pinned there, or any of them over any of the others.
///
/// A row that is both pinned and pointed at is painted as pinned, the stronger of the two
/// saying everything the weaker would. A selection is not one of that pair and does not
/// replace either: the two say what this row maps to and this says what would be copied,
/// so the three stack.
fn row_background(hovering: bool, focused: bool, pinned: bool, selected: bool) -> Color {
    let cross = match (pinned, focused) {
        (true, _) => palette().line_pin_bg,
        (false, true) => palette().line_focus_bg,
        (false, false) => Color::TRANSPARENT,
    };

    // `blend` over a transparent bottom is the top colour unchanged, so a hovered row that
    // is neither selected nor lit comes out as the hover alone without a case of its own.
    let hovered = if hovering {
        blend(palette().code_row_hover_bg, cross)
    } else {
        cross
    };

    // The selection goes on top of the hover and not under it, the other way round from
    // every other pair here. It is the only one of the three that says what a keystroke
    // is about to act on, and a row swept over by the pointer -- which is every row of a
    // drag, one after another -- would otherwise show the hover and almost none of it.
    if selected {
        blend(palette().row_select_bg, hovered)
    } else {
        hovered
    }
}

fn kind_color(kind: SpanKind) -> Color {
    match kind {
        SpanKind::Mnemonic | SpanKind::Prefix => palette().keyword_fg,
        SpanKind::Register => palette().operand_fg,
        SpanKind::Number => palette().literal_fg,
        SpanKind::Address => palette().address_fg,
        SpanKind::Other => palette().punctuation_fg,
    }
}

// ---------------------------------------------------------------------------
// Source files
// ---------------------------------------------------------------------------

/// A source file ready to be drawn: its text as a rope, and the coloured spans
/// tree-sitter produced for each of its lines.
///
/// The highlighter comes from `freya-code-editor`, whose `CodeEditor` component this pane
/// deliberately does not use: it paints a line background only for the cursor's own row
/// and keeps its scroll state private, so it can neither highlight the set of lines an
/// instruction maps to nor be scrolled to one. Its `SyntaxHighlighter` is public on its
/// own and is exactly the shape these rows want. (The Scratchpad pane *does* use the
/// component -- see [`SourceEditor`] -- because neither objection survives the pane being
/// one the reader is typing in.)
struct Highlighted {
    rope: Rope,
    blocks: SyntaxBlocks,
    /// How many rows the pane draws, which is *not* `blocks.len()`: a rope counts a
    /// phantom empty line after a trailing newline and the highlighter pushes a block
    /// for it, and no editor shows that line.
    lines: usize,
}

impl Highlighted {
    /// Parse and colour a whole file, once. The highlighter is stateful across lines --
    /// that is what makes it a parser rather than a regex -- so this happens when the
    /// file is loaded and never while a row is being drawn.
    fn new(file: &SourceFile) -> Highlighted {
        let rope = Rope::from_str(file.text());
        let theme = palette().syntax();

        let mut highlighter = SyntaxHighlighter::new();
        // A language of `None` -- an extension no grammar here parses -- is not a
        // failure: the highlighter then hands back one plain span per line, in the
        // theme's text colour, and the pane renders exactly as it would without any of
        // this. A highlights query that will not compile lands in the same place.
        highlighter.set_language(language(file.path()).as_ref(), &theme);

        let mut blocks = SyntaxBlocks::default();
        highlighter.parse(&rope, &mut blocks, None, &theme);

        let lines = blocks
            .len()
            .saturating_sub(usize::from(file.text().ends_with('\n')));

        Highlighted {
            rope,
            blocks,
            lines,
        }
    }
}

/// The tree-sitter grammar to parse a file with, chosen by extension.
///
/// `freya-code-editor` ships no grammars on purpose, so these are the app's own
/// dependencies, pinned against the `tree-sitter` its highlighter is built on. `.h` goes
/// to C rather than C++ because that is what it is more often; a header the C grammar
/// misparses is coloured oddly, never dropped.
fn language(path: &Path) -> Option<EditorLanguage> {
    let (language, query) = match path.extension()?.to_str()? {
        "rs" => (
            tree_sitter_rust::LANGUAGE,
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "c" | "h" => (tree_sitter_c::LANGUAGE, tree_sitter_c::HIGHLIGHT_QUERY),
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" => {
            (tree_sitter_cpp::LANGUAGE, tree_sitter_cpp::HIGHLIGHT_QUERY)
        }
        _ => return None,
    };

    Some(EditorLanguage::new(language, query))
}

/// Every file highlighted so far.
///
/// A second cache behind `source`'s, and a `static` for the same reason: parsing a file
/// is the expensive half of showing it, the pane asks again on every render, and a
/// failure needs no entry here because `source::load` already remembers its own.
///
/// What is cached is not just the parse: `SyntaxBlocks` holds a `Color` per span, resolved
/// against `palette().syntax()` when the file was loaded, so an entry here is spans in the
/// palette that was current at the time. **A theme switch therefore has to empty this
/// map** -- the entries are not stale, they are the wrong theme, and nothing else in the
/// app would repaint them, a `SyntaxBlocks` being the one thing here a re-render does not
/// rebuild. That clear is [`set_appearance`], which is the only way the appearance can
/// change at all, so it cannot be routed around by a later call site. Re-highlighting
/// every open file is what a switch costs, which is why the parse belongs where it is
/// rather than in `source::load`: `source`'s cache of the *text* survives it.
static HIGHLIGHTED: LazyLock<Mutex<HashMap<PathBuf, Arc<Highlighted>>>> =
    LazyLock::new(Mutex::default);

fn highlighted() -> MutexGuard<'static, HashMap<PathBuf, Arc<Highlighted>>> {
    HIGHLIGHTED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// The file at `path`, read and highlighted, or `None` when it cannot be shown at all.
fn source_text(path: &Path) -> Option<SourceText> {
    if let Some(cached) = highlighted().get(path) {
        return Some(SourceText(cached.clone()));
    }

    // Read and parsed outside the lock, for the reason `source::load` does the same: this
    // is the slow step, and a racing caller's copy costs an allocation rather than a wait.
    // The `SourceFile` itself is not kept: the rope holds the text and the chip above the
    // pane holds the path, and `source`'s own cache is what keeps a second read from
    // touching the disk.
    let file = Arc::new(Highlighted::new(&*source::load(path)?));

    Some(SourceText(
        highlighted()
            .entry(path.to_path_buf())
            .or_insert(file)
            .clone(),
    ))
}

// ---------------------------------------------------------------------------
// Filtering
// ---------------------------------------------------------------------------

/// One of the three toggles beside a filter's text box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Toggle {
    Case,
    Word,
    Regex,
}

impl Toggle {
    /// The three of them in the order the bar draws them.
    const ALL: [Toggle; 3] = [Toggle::Case, Toggle::Word, Toggle::Regex];

    /// What the button is drawn as.
    ///
    /// Still text, and looked at twice. The first answer leaned on the dependency, which
    /// the tab bar's icons have since brought in, and on Lucide having nothing for a regex
    /// flag, which is simply wrong: the set carries `case-sensitive`, `whole-word` and
    /// `regex`, which are VS Code's three toggles glyph for glyph. Rendered at
    /// [`toggle_size`] beside these, they lose anyway. `case-sensitive` is an `Aa` drawn as
    /// strokes, so it says exactly what the two letters say and no more; `regex` at 17px
    /// is a splayed asterisk over a rounded box, muddier than the two characters it stands
    /// for; and `\b` and `.*` *are* the regex the toggle turns on, written out, which in a
    /// window whose filter bar compiles to a `regex::Regex` and whose reader is reading
    /// disassembly is the more precise label rather than the more cryptic one. `whole-word`
    /// is the one that is arguably better than its text, and one of three is not a set.
    /// The words are in the tooltip either way.
    fn glyph(self) -> &'static str {
        match self {
            Toggle::Case => "Aa",
            Toggle::Word => "\\b",
            Toggle::Regex => ".*",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Toggle::Case => "Match case",
            Toggle::Word => "Whole word",
            Toggle::Regex => "Regular expression",
        }
    }

    fn is_on(self, filter: &Filter) -> bool {
        match self {
            Toggle::Case => filter.case_sensitive,
            Toggle::Word => filter.whole_word,
            Toggle::Regex => filter.regex,
        }
    }

    fn flip(self, filter: &mut Filter) {
        match self {
            Toggle::Case => filter.case_sensitive = !filter.case_sensitive,
            Toggle::Word => filter.whole_word = !filter.whole_word,
            Toggle::Regex => filter.regex = !filter.regex,
        }
    }
}

/// One toggle button.
///
/// Whether it is on is a prop rather than something read here, so that typing a character
/// — which changes the one `Filter` all three of them share — re-renders the bar and none
/// of them.
#[derive(Clone, PartialEq)]
struct FilterToggle {
    filter: State<Filter>,
    toggle: Toggle,
    on: bool,
}

impl Component for FilterToggle {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut filter = self.filter;
        let toggle = self.toggle;

        let background = if self.on {
            palette().toggle_on_bg
        } else if hovering() {
            palette().toggle_hover_bg
        } else {
            Color::TRANSPARENT
        };

        TooltipContainer::new(Tooltip::new(toggle.tooltip())).child(
            rect()
                .width(Size::px(toggle_size()))
                .height(Size::px(toggle_size()))
                .center()
                .corner_radius(4.0)
                .background(background)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // The text box beside this one gives its keyboard focus up from
                    // `on_global_pointer_press`, which is how an `Input` notices a click
                    // that landed outside it. A toggle is not outside it in the way that
                    // matters: turning "whole word" on halfway through typing a name must
                    // not send the rest of the name nowhere. A press's cancellable events
                    // include the global press it derives, and non-capture globals are
                    // sorted to run last (freya-core `events/name.rs`), so preventing the
                    // default here reaches the input before it acts on it.
                    e.prevent_default();
                    toggle.flip(&mut filter.write());
                })
                .child(label().text(toggle.glyph()).max_lines(1)),
        )
    }
}

/// The filter over one of the sidebar lists: a text box, and the three toggles that say
/// how to read what is in it.
///
/// One component and three uses. The state it edits belongs to the tab that owns the list
/// rather than to the root — a filter is a view of a list and not part of the session — so
/// it arrives as a prop and never as a context, and nothing about it reaches `project.rs`.
#[derive(Clone, PartialEq)]
struct FilterBar {
    filter: State<Filter>,
}

impl Component for FilterBar {
    fn render(&self) -> impl IntoElement {
        let filter = self.filter;
        // Reading subscribes the bar to the filter, which is what puts a typed character
        // back on screen and lights a toggle that was just pressed.
        let current = filter.read().clone();
        // Compiled here as well as wherever the list is actually filtered. A `Regex` is
        // not something the two can share through a `State`: it is not `PartialEq`, and a
        // compiled program is not a value to compare anyway. Compiling one costs
        // microseconds against the milliseconds a pass over a list of names does.
        let error = current.matcher().error().map(str::to_owned);

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(filter_height()))
                    .horizontal()
                    // The toggles take their own widths and the box takes the rest, which
                    // torin only works out for a `flex` child of a `Content::Flex` parent.
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 5.0))
                    .spacing(2.0)
                    .child(
                        Input::new(
                            // The pattern is a field of the `Filter` rather than a state
                            // of its own, so that what was typed and how it is to be read
                            // are one value to compare and one thing to hand a memo.
                            // `Writable::map` is what lets the `Input` write into that
                            // field while still notifying everything watching the whole
                            // filter.
                            filter
                                .into_writable()
                                .map(|filter| &filter.pattern, |filter| &mut filter.pattern),
                        )
                        .placeholder("Filter")
                        .compact()
                        .width(Size::flex(1.0))
                        .maybe(error.is_some(), |input| {
                            input
                                .color(palette().invalid_fg)
                                .focus_border_fill(palette().invalid_fg)
                        }),
                    )
                    .children(Toggle::ALL.map(|toggle| {
                        FilterToggle {
                            filter,
                            toggle,
                            on: toggle.is_on(&current),
                        }
                        .into()
                    })),
            )
            // A pattern that will not compile has to read *as* one. Matching everything
            // would hide the half-typed `(` and matching nothing looks exactly like a
            // list with nothing in it, so the reason is written under the box it is in —
            // and the list below stays empty, which is now the truth rather than a
            // coincidence.
            .maybe_child(error.map(|error| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 6.0, 5.0, 6.0))
                    .overflow(Overflow::Clip)
                    .child(label().text(error).color(palette().invalid_fg).max_lines(1))
            }))
    }
}

/// A list under its own filter bar.
///
/// The bar goes above the list, which is where "filter bar under objects / symbols /
/// history" puts it: under the tab that names the list, the same place the assembly
/// goal's "bar under the Assembly tab" means. It takes its height off the top of the pane
/// rather than out of the list — the list is the `flex` child of a `Content::Flex` parent,
/// exactly as the source rows are under their path header — so a `VirtualScrollView`
/// inside it still starts at a row boundary whatever height the bar turns out to want,
/// which is not fixed: it grows by a line when the pattern will not compile.
fn filter_pane(filter: State<Filter>, background: Color, list: impl IntoElement) -> Element {
    rect()
        .expanded()
        .content(Content::Flex)
        .background(background)
        .child(FilterBar { filter })
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(list),
        )
        .into()
}

/// What a filter leaves of the symbol list: the list itself, and where in it the names
/// that matched it are.
///
/// Indices rather than a second `Vec<Symbol>`, because the list is 115k entries on
/// `viewer-sample` and a row wants to be told which entry it is rather than handed a copy
/// of it. `None` rather than every index in order, because no filter at all is the state
/// the list is in most of the time and that case then costs exactly what it cost before
/// there was a filter: no pass over the names and no allocation to say "all of them".
#[derive(Clone)]
struct Filtered {
    symbols: SymbolList,
    matches: Option<Arc<Vec<usize>>>,
}

impl PartialEq for Filtered {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && match (&self.matches, &other.matches) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Filtered {
    /// Filter on the name the row actually shows — the demangled one where there is one —
    /// because a filter the user cannot see the effect of on screen is not one.
    fn new(symbols: SymbolList, matcher: &Matcher) -> Self {
        let matches = match matcher {
            Matcher::Everything => None,
            matcher => Some(Arc::new(
                symbols
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| matcher.matches(symbol.data.display()))
                    .map(|(index, _)| index)
                    .collect(),
            )),
        };

        Filtered { symbols, matches }
    }

    /// How many rows there are, which is what the `VirtualScrollView` is given.
    fn len(&self) -> usize {
        self.matches
            .as_ref()
            .map_or(self.symbols.0.len(), |matches| matches.len())
    }

    /// Which symbol the row at `row` is.
    fn index(&self, row: usize) -> usize {
        self.matches.as_ref().map_or(row, |matches| matches[row])
    }
}

// ---------------------------------------------------------------------------
// Open tabs
// ---------------------------------------------------------------------------

/// Why a document is becoming the active one, which is the whole of what decides whether
/// the history records it.
///
/// **The push follows the cause and not the state**, which is the rule Step 1e settled:
/// the history is where the reader *went*, and moving between places they already have
/// open is not going anywhere. Until then a single effect observed the active document
/// and pushed on every change, which could not tell the two apart — a strip click and a
/// symbol-list click look identical from there — so the answer has to come from the call
/// site, where it is known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Visit {
    /// The reader went somewhere: a sidebar row, a relocation link, the Source pane's
    /// companion header, or a restored session landing the app on a document. Recorded,
    /// unless the history's cursor is on it already.
    Went,
    /// The reader moved between places already open, or something moved them: a tab in
    /// the strip, the neighbour a close lands on, a tab the restore is merely reopening,
    /// and [`navigate`], which moves the cursor itself. Recorded nowhere.
    Moved,
}

/// Make `target` the active document, opening a tab for it if it has none, and record the
/// visit when there was one.
///
/// The one path by which [`Active`] ever changes, which is what makes "the active
/// document is the active tab" an invariant rather than a convention: the sidebar's
/// object and symbol rows, an assembly relocation link, the Source pane's companion
/// header, the history panel and the back/forward buttons (both through [`navigate`]) and
/// the startup restore all come through here, so none of them has to know that tabs
/// exist. `None` opens nothing and is how the content area goes back to its placeholder;
/// it is never a visit, having nowhere to be a visit to.
///
/// **One function for both kinds of tab**, where until Step 1 there were two — `activate`
/// for the content area's functions and `open_file` for the Source pane's files, each
/// holding its own strip's invariant. The strips are one, so the rule is one, and opening
/// a file and opening a function differ in nothing but the value handed over.
///
/// **And one function for the history too**, where until Step 1e that was an effect
/// observing the active document from the root. The effect was the wrong shape rather
/// than merely in the wrong place: it saw *that* the document had changed and could not
/// see *why*, so a click on a tab in the strip was indistinguishable from a click on a
/// symbol in the list. `visit` is that missing half, and it is why the recording moved to
/// the one place every change already goes through rather than to each caller.
///
/// `History::would_push` is still asked, and it is what keeps [`navigate`] honest without
/// a "we are navigating" flag: back and forward land the cursor on the entry they moved
/// to, so a push would dedup away even if one were attempted.
///
/// Re-focusing a tab that is already open writes nothing: `State::write` notifies its
/// subscribers whether or not the value changes, so both the list and the active document
/// are asked before they are touched.
fn activate(
    mut open: State<Tabs<Document>>,
    mut active: State<Option<Document>>,
    mut history: State<History>,
    target: Option<Document>,
    visit: Visit,
) {
    // The copy that is *in the list* where there is one, so the identity a position is
    // keyed by does not change when the same file is reached again through a different
    // symbol's `LineInfo`: two of them naming one path hold two `Arc<str>`s of it.
    let existing = target
        .as_ref()
        .and_then(|target| open.peek().find(target).cloned());
    let target = match (existing, target) {
        (Some(open), _) => Some(open),
        (None, Some(target)) => {
            open.write().open(target.clone());
            Some(target)
        }
        (None, None) => None,
    };

    active.set_if_modified(target.clone());

    // `write()` notifies its subscribers before it hands the value over, whether or not
    // anything changes, so ask first: a push that would dedup away must not wake the
    // history panel. The guard from `peek` is gone before the write is reached.
    let Some(target) = target.filter(|_| visit == Visit::Went) else {
        return;
    };
    if history.peek().would_push(&target) {
        history.write().push(target);
    }
}

/// Close the tab showing `entry`, moving to a neighbouring one when it was the tab on
/// screen and to the placeholder when it was the last one open.
///
/// Landing on the neighbour is a [`Visit::Moved`] and records nothing: it is a place the
/// reader already had open, which is exactly what the strip is, and closing a tab is not
/// a way of visiting the one beside it.
///
/// Where the tab was left goes with it, **both sides of it**. A closed tab is not a tab,
/// so a position kept for one is both a lie — reopening it from the sidebar is a fresh
/// tab, which starts at the top — and a leak, since a [`Document::Assembly`] holds the
/// `Arc<Object>` it points into.
fn close_tab(
    mut open: State<Tabs<Document>>,
    active: State<Option<Document>>,
    history: State<History>,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    entry: &Document,
) {
    let was_showing = active.peek().as_ref() == Some(entry);
    let next = open.write().close(entry);
    asm_at.write().forget(entry);
    src_at.write().forget(entry);

    if was_showing {
        // Through `activate` like everything else, even though the neighbour is by
        // construction already open: this is a change of active document and there is one
        // way to make one. The write guards above are released before it is reached.
        activate(open, active, history, next, Visit::Moved);
    }
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The third of the functions that hold the app's invariants, beside [`activate`] and
/// [`close_tab`], and the only one that ever *removes* an object -- until 8c the app could
/// open a binary and never let go of one. The unit is the **file** and never the object:
/// an archive member is not something the reader opened, closing one member of 196 would
/// leave a file half-present with no row able to say so, and the saved `binaries` are a
/// list of paths, so half a file is not a thing the session could even record. One path
/// opened twice is therefore also one close: the objects list holds both copies,
/// `Object::path` cannot tell them apart, and neither could the file it would be written
/// to.
///
/// What each of the things pointing at those objects does with the news:
///
/// - The **assembly-driven tabs** whose document was in the file are closed, all of them
///   at once ([`Tabs::close_all`]), which is what closing the one tab the reader was on
///   would have done had its neighbours not gone with it. **Source-driven tabs survive**
///   ([`Document::in_file`] answers false for one): a file chip outlives the binary that
///   led the reader to it, because the text stands on its own and nothing records which
///   object opened it. That was the Source pane's separate strip being left alone; it is
///   now a rule of the one strip.
/// - The **active document** follows the tabs rather than degrading the way a restore's
///   does. Degrading has nothing to fall back *to* here: a file takes its objects and
///   their symbols together, so `resolve_or_degrade`'s symbol-to-object step would land on
///   an object that is going away in the same breath. What is left is the tab rule -- the
///   neighbouring tab, or nothing at all when the close emptied the strip -- and that is
///   also the only answer that keeps "the active document is the active tab" true, since
///   the placeholder with tabs still open would be a fourth state.
/// - The **history** drops its entries rather than degrading them ([`History::retaining`]),
///   which is the same walk and the same reasoning as a restore whose binaries have
///   changed: a list of places the reader cannot get back to is worse than a short list.
///   A visited source file is kept, by the same rule its tab is. It is *read* here too,
///   since the tab this lands on goes through `activate`.
/// - The **viewing positions** of the tabs that closed go with them, both sides of each
///   ([`Positions`]), which is not tidiness: every entry is keyed by a [`Document`], which
///   for an assembly-driven one holds the `Arc<Object>` it points into, so one left behind
///   would hold the file's bytes -- 331 MB of them, for `viewer-sample` -- for as long as
///   the app ran.
/// - **The file's load**, if it is still being read, is cancelled ([`Loads::cancel`]) —
///   which is not tidiness either: without it the objects still coming out of the worker
///   would arrive after the close and put the file back, one member at a time. The unit
///   there is the path for the same reason it is here, so one file opened twice closes
///   once and stops loading once.
/// - **The saved `binaries`** need nothing here at all. They are derived from the objects
///   by `project::binaries`, so removing them removes the path, and `project::record` sees
///   a *binaries* change and writes it to disk at once rather than marking it pending --
///   which is what `Goals.md` asks of a change the user made, and the first thing since
///   opening a file to take that path.
///
/// All the writes happen here, in one event handler, before anything can render: the
/// save observer therefore wakes once, with all of it settled, so the file that reaches
/// the disk never names a binary the app has already let go of.
fn close_binary(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    mut open: State<Tabs<Document>>,
    active: State<Option<Document>>,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    mut history: State<History>,
    path: &Path,
) {
    // Every guard below is taken out of its own statement, so none of them is still
    // alive when the next write -- or `activate` at the end -- is reached.
    let showing = active.peek().clone();
    let next = open
        .write()
        .close_all(showing.as_ref(), |tab| tab.in_file(path));

    // The same walk over the same rule, so the positions cannot outlive the tabs they
    // belong to.
    asm_at.write().forgetting(|tab| !tab.in_file(path));
    src_at.write().forgetting(|tab| !tab.in_file(path));

    let remaining = history.peek().retaining(|entry| !entry.in_file(path));
    history.set(remaining);

    objects.write().retain(|object| object.path != path);
    // Whatever is still being parsed out of this file is for a file the app has just let
    // go of. Dropping the entry is what makes the next batch of objects out of it be
    // dropped and the worker itself stop; see `take_load`.
    loading.write().cancel(path);

    if showing.is_some_and(|showing| showing.in_file(path)) {
        // Through `activate` like every other change of active document, even though the
        // tab it lands on is by construction already open — which is what makes it a
        // [`Visit::Moved`], exactly as closing one tab by hand is.
        activate(open, active, history, next, Visit::Moved);
    }
}

/// The menu a file row opens on a right-click: the one thing that can be done to a file
/// once it is open.
///
/// Built per press rather than once, because it closes over the path of the row it was
/// opened on -- freya's `ContextMenu` takes a whole `Menu` and places it at the pointer
/// (`freya-components/src/context_menu.rs`), so there is nothing to keep. The states come
/// in as an argument for the reason every row's do: this is called from an event handler,
/// where no hook may run.
fn close_menu(states: ProjectStates, path: PathBuf) -> Menu {
    let ProjectStates {
        objects,
        loading,
        open,
        asm_at,
        src_at,
        active,
        history,
        ..
    } = states;

    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                close_binary(
                    objects, loading, open, active, asm_at, src_at, history, &path,
                )
            })
            // "file" and not "object", because the row a reader right-clicks may be one
            // object of one file or the archive above 196 of them, and the same word has
            // to be true of both.
            .child("Close file"),
    )
}

// ---------------------------------------------------------------------------
// Opening binaries
// ---------------------------------------------------------------------------

/// Read and parse `paths`, putting each object into the list **as it is parsed**.
///
/// The opposite number of [`close_binary`], and the one path by which anything is ever
/// added to `objects`: the toolbar's Open, a session restore and a scratchpad's rebuild
/// all come through here, so they cannot differ about what opening a file means.
///
/// **A worker thread and a channel**, which is the shape `use_analysis` and the
/// scratchpad's worker already have and for the same reason: reading and parsing is
/// seconds of CPU on a large file and freya's executor is the UI thread. What is new is
/// that the answers come back one at a time (`analysis`'s `open_files_streaming`) rather
/// than as one `Vec` at the end — which is the whole of "explore while a binary is
/// processed". On `libanalysis-sample.rlib` that is 196 members arriving over the parse
/// instead of after it; on the 331 MB `viewer-sample`, which is one object, it is the row
/// in [`Loads`] appearing at once where the sidebar used to sit empty for the duration.
///
/// The channel is **unbounded and drained in batches**. Unbounded because backpressure
/// would be exactly wrong here — the worker is the thing that should run flat out, and the
/// objects it hands over are `Arc`s of bytes that already exist — and batched because a
/// write per member is a re-render per member, which for an archive whose members parse in
/// a millisecond is a hundred renders nobody sees. Draining what has already arrived
/// collapses each burst into one write.
async fn open_binaries(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    paths: Vec<PathBuf>,
) {
    // Registered before a byte is read, so the rows are on screen for the whole of the
    // wait rather than from whenever the first answer lands.
    let id = {
        let mut loading = loading;
        loading.write().begin(&paths)
    };

    let (sender, events) = async_channel::unbounded::<Progress>();
    std::thread::spawn(move || {
        open_files_streaming(paths, |progress| match sender.send_blocking(progress) {
            Ok(()) => ControlFlow::Continue(()),
            // The receiver has gone, which is `take_load` deciding that nothing more from
            // this load is wanted. Stopping here is what keeps a closed 331 MB file from
            // being parsed to the end into a value that will be dropped.
            Err(_) => ControlFlow::Break(()),
        });
    });

    take_load(objects, loading, id, events).await;
}

/// Take one load's answers until it has nothing left to say.
///
/// Split from [`open_binaries`] because it is the half with the rules in it, and because
/// a test can feed it by hand: what has to be asserted is what happens to an answer that
/// arrives *after* the reader has closed the file or left the project, which is a race
/// against a real worker and a fact against a channel the test writes into.
///
/// **An object nobody asked for any more is dropped rather than prevented.** That is
/// `use_analysis`'s rule in a second place, and it has to be: the worker is already
/// parsing when the file is closed, so the answer exists whatever the app does. It is
/// checked against [`Loads::holds`] — the load *and* the path, not the path alone, since a
/// file closed and reopened while the first parse ran is two loads and only the second
/// one's objects belong on screen.
///
/// Returning is what stops the worker: it drops the receiver, the next `send_blocking`
/// fails, and the walk breaks where it stands.
async fn take_load(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    id: LoadId,
    events: async_channel::Receiver<Progress>,
) {
    while let Ok(first) = events.recv().await {
        // Whatever else has arrived while the UI thread was elsewhere, taken in the same
        // pass so a burst of members costs one write.
        let mut batch = vec![first];
        while let Ok(more) = events.try_recv() {
            batch.push(more);
        }

        // Both lists are worked out under one read guard and the guard is gone before
        // anything writes -- the `peek`/`write` rule, and the reason this is not a single
        // loop that pushes and writes as it goes.
        let (parsed, finished) = {
            let held = loading.peek();
            let mut parsed: Vec<Arc<Object>> = Vec::new();
            let mut finished: Vec<PathBuf> = Vec::new();
            for progress in batch {
                match progress {
                    Progress::Parsed(object) if held.holds(id, &object.path) => parsed.push(object),
                    // An object for a file this load no longer holds: the reader closed
                    // it, or left the project, while it was being parsed.
                    Progress::Parsed(_) => {}
                    Progress::Finished(path) => finished.push(path),
                }
            }
            (parsed, finished)
        };

        if !parsed.is_empty() {
            objects.write().extend(parsed);
        }
        if !finished.is_empty() {
            let mut held = loading.write();
            for path in finished {
                held.finished(id, &path);
            }
        }

        // Nothing left that this load could still be asked about, either because it is
        // done or because everything it was reading has been closed. Returning drops the
        // receiver, which is the only thing that tells the worker.
        if !loading.peek().active(id) {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// A row's or a chip's own text, shown in full where it could only show part of it.
///
/// Every panel list and both tab strips use this rather than `TooltipContainer` directly,
/// so that the one thing they must agree on -- how long the pointer has to sit still, see
/// [`TOOLTIP_DELAY`] -- is decided once.
fn row_tooltip(text: String, row: impl IntoElement) -> TooltipContainer {
    TooltipContainer::new(Tooltip::new(text))
        .delay(TOOLTIP_DELAY)
        .child(row.into_element())
}

/// The short tag saying what kind of file a row is, in the column every row of the
/// objects tree keeps for it. Grey and small: it labels the row rather than naming it.
fn tag_label(tag: &str) -> impl IntoElement {
    label()
        .text(tag.to_owned())
        .width(Size::px(TAG_WIDTH))
        .font_size(TAG_FONT_SIZE)
        .color(palette().address_fg)
        .max_lines(1)
}

/// What a row is called, taking whatever width the columns beside it left.
///
/// Ellipsised rather than simply cut, which is what the other panel lists do: those cut
/// against the edge of the pane, where the cut is self-evident, while this one cuts
/// against the member count beside it and a name ending flush against a number reads as
/// though it ended there. The `…` is also what says the row's tooltip has more to show.
///
/// The label sits in a box of its own rather than being the `flex` child itself. A
/// `flex` child is measured from its content first, so a label placed there directly
/// takes the width of its whole name and pushes the count off the row.
fn tree_name(text: String, dim: bool) -> impl IntoElement {
    rect()
        .width(Size::flex(1.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(text)
                .width(Size::fill())
                .max_lines(1)
                // Unset rather than `text_fg` when it is not dimmed, so the row goes on
                // inheriting the interface colour from the root the way it always did.
                .maybe(dim, |name| name.color(palette().address_fg))
                .text_overflow(TextOverflow::Ellipsis),
        )
}

/// One opened file that contributed several objects — an archive — and the row its
/// members fold under. It has no `Object` behind it, an `.a`/`.lib` not being one, so it
/// selects nothing: pressing it folds it open or shut, which is all a file row is for
/// until Step 6c decides what an object *is* to the selection.
#[derive(Clone)]
struct ArchiveRow {
    name: String,
    path: PathBuf,
    members: usize,
    expansion: Expansion,
    /// Whether objects may still be arriving out of this file, which is the whole of the
    /// indicator: the tag column says so and the name is dimmed with it, rather than a
    /// spinner, because a sidebar row is one of hundreds and none of the others move.
    loading: bool,
    /// The group this row is, in the tab's set of the groups the reader has opened.
    /// [`None`] for a file that has contributed nothing yet: there is nothing behind it to
    /// fold, so there is nothing for the set to hold either.
    group: Option<usize>,
    expanded: State<HashSet<usize>>,
    key: DiffKey,
}

impl PartialEq for ArchiveRow {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.path == other.path
            && self.members == other.members
            && self.expansion == other.expansion
            && self.loading == other.loading
            && self.group == other.group
    }
}

impl KeyExt for ArchiveRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ArchiveRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut expanded = self.expanded;
        let group = self.group;
        let expansion = self.expansion;
        // The states closing a file has to answer for. Consumed here, in the render,
        // because the handler that uses them may not run a hook.
        let states = use_project_states();
        let path = self.path.clone();

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        // `Forced` draws no triangle, only the space one would have taken: while the
        // filter is holding the file open, folding it would hide the very rows the filter
        // put on screen, so there is nothing here to press. See `Expansion::Forced`. A row
        // with no group is the same answer for the other reason -- there is nothing behind
        // it yet -- and the space keeps its tag lined up with the rest.
        let chevron = match expansion {
            _ if self.group.is_none() => "",
            Expansion::Collapsed => "\u{25b8}",
            Expansion::Expanded => "\u{25be}",
            Expansion::Forced => "",
        };
        // Which format a file is is not known until it has been parsed, so one still being
        // read wears the one tag that is true of it: it is being read.
        let tag = if self.loading {
            "\u{2026}"
        } else {
            ARCHIVE_TAG
        };

        row_tooltip(
            self.path.display().to_string(),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                // The name is the `flex` child that takes what the three fixed columns
                // beside it leave, which torin only works out under `Content::Flex`.
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    // Nothing behind the row and nothing to fold: a file that has
                    // contributed no object yet.
                    let Some(group) = group else {
                        return;
                    };
                    if expansion == Expansion::Forced {
                        return;
                    }
                    let mut expanded = expanded.write();
                    if !expanded.remove(&group) {
                        expanded.insert(group);
                    }
                })
                // The archive is a file the reader opened, so it is one they can close,
                // even though it selects nothing and has no `Object` behind it.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                })
                .child(
                    label()
                        .text(chevron)
                        .width(Size::px(CHEVRON_WIDTH))
                        .color(palette().address_fg)
                        .max_lines(1),
                )
                .child(tag_label(tag))
                // Dimmed while it is being read, which is the second half of the
                // indicator: the tag says what is happening and the colour says that the
                // row is not yet the whole answer.
                .child(tree_name(self.name.clone(), self.loading))
                // How many objects came out of this file, which under a filter is how
                // many of them matched. It is the one thing about an archive that is not
                // visible while it is folded shut. A file that has produced nothing yet
                // shows no count rather than a zero: the count says what is behind the
                // row, and "nothing, so far" is what the rest of the row already says.
                .child(
                    label()
                        .text(if self.members == 0 {
                            String::new()
                        } else {
                            self.members.to_string()
                        })
                        .font_size(TAG_FONT_SIZE)
                        .color(palette().address_fg)
                        .max_lines(1),
                ),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One object: an archive member indented under its file, or a file that contributed
/// exactly one object and so is a row of its own.
#[derive(Clone)]
struct ObjectRow {
    object: Arc<Object>,
    selected: bool,
    /// Whether this object is one of several a file contributed. It decides the indent,
    /// and it decides what the tooltip says: a member's own name is the thing that gets
    /// cut off, while a lone object is named after its file and the useful extra is
    /// where that file is.
    member: bool,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && self.selected == other.selected
            && self.member == other.member
    }
}

impl KeyExt for ObjectRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ObjectRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let (open, active, history) = (states.open, states.active, states.history);
        let object = self.object.clone();
        let path = self.object.path.clone();

        let background = if self.selected {
            palette().selected_bg
        } else if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        let tooltip = if self.member {
            self.object.name.clone()
        } else {
            self.object.path.display().to_string()
        };

        row_tooltip(
            tooltip,
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    activate(
                        open,
                        active,
                        history,
                        Some(Document::Assembly(Selection::Object(object.clone()))),
                        Visit::Went,
                    );
                })
                // A lone object *is* the file it came out of, so it closes like one. A
                // member is not: it was never opened on its own, and the row that can
                // close the file it belongs to is the one above it. Right-clicking a
                // member therefore does nothing rather than quietly taking 195 rows the
                // reader was not pointing at with it.
                .maybe(!self.member, move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(&e, close_menu(states, path.clone()));
                    })
                })
                // The column a file row's triangle sits in, kept empty here so that the
                // tags of a file and of a lone object line up; a member is indented past
                // it instead.
                .child(rect().width(Size::px(if self.member {
                    CHEVRON_WIDTH + TREE_INDENT
                } else {
                    CHEVRON_WIDTH
                })))
                .child(tag_label(format_tag(self.object.format)))
                .child(tree_name(self.object.name.clone(), false)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
struct SymbolRow {
    symbols: SymbolList,
    index: usize,
    selected: bool,
    key: DiffKey,
}

impl PartialEq for SymbolRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
    }
}

impl KeyExt for SymbolRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SymbolRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let active = use_consume::<Active>().0;
        let open = use_consume::<Open>().0;
        let history = use_consume::<Hist>().0;
        let symbol = self.symbols.0[self.index].clone();
        let text = symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone();

        let background = if self.selected {
            palette().selected_bg
        } else if hovering() {
            palette().symbol_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            text.clone(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    activate(
                        open,
                        active,
                        history,
                        Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
                        Visit::Went,
                    );
                })
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a document is called where it is named in a list: the same demangled name the
/// symbol list shows for a function, the object's name for an object, and the file's own
/// last path component for a source file. The history rows and the tabs both draw this,
/// which is what makes a place read the same wherever it is named.
///
/// A file's *name* and not its path, because the strip is narrow and every one of these
/// paths shares most of its directory with the others. The whole of it is in the tooltip
/// ([`entry_tooltip`]), which is what the Source pane's header used to say.
fn entry_text(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Object(object)) => object.name.clone(),
        Document::Assembly(Selection::Symbol(symbol)) => symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone(),
        Document::Source(file) => file_name(file),
    }
}

/// What hovering a document's tab or row says. The whole path for a file, where the row
/// itself has only room for its name; everything else says what it draws, elided or not.
fn entry_tooltip(entry: &Document) -> String {
    match entry {
        Document::Source(file) => file.to_string(),
        entry => entry_text(entry),
    }
}

/// Which kind of tab this is, as the one glyph that tells the two apart.
///
/// The same two glyphs the dock's own Assembly and Source views wear (`Tab::icon`), and
/// deliberately so: the tab says which pane is in charge of it, so it should be named by
/// the pane it is about.
fn entry_icon(entry: &Document) -> Element {
    let (name, svg) = match entry {
        Document::Assembly(_) => ("binary", lucide::binary()),
        Document::Source(_) => ("file-code", lucide::file_code()),
    };

    let side = icon_size();
    SvgViewer::new((name, svg))
        .width(Size::px(side))
        .height(Size::px(side))
        .color(palette().icon_fg)
        .show_loader(false)
        .into_element()
}

/// The identity of what a document points at, for keying the row or tab that names it.
///
/// A tab keys by this alone, its place in the strip being stable. A history row pairs it
/// with the entry's index, because a row's identity is its place in the list: the entry at
/// an index changes when a push truncates the forward entries, and again when a push
/// bumps an existing entry to the newest position and shifts the ones behind it down. The
/// pointer alone would be identity enough now that no two entries are equal, but then a
/// bumped row would keep the hover state of the one that used to sit where it now does;
/// with the index in the key the moved rows are simply rebuilt, which for a list this
/// short costs nothing.
///
/// The variant is part of the key and not only the pointer, since a file is keyed by its
/// text: a hash of an address and a hash of a path could otherwise collide into one key
/// for two tabs of different kinds.
#[derive(Hash)]
enum EntryKey<'a> {
    Object(usize),
    Symbol(usize),
    Source(&'a str),
}

fn entry_key(entry: &Document) -> EntryKey<'_> {
    match entry {
        Document::Assembly(Selection::Object(object)) => {
            EntryKey::Object(Arc::as_ptr(object).addr())
        }
        Document::Assembly(Selection::Symbol(symbol)) => {
            EntryKey::Symbol(Arc::as_ptr(&symbol.data).addr())
        }
        Document::Source(file) => EntryKey::Source(file),
    }
}

/// One visited document in the history list. Clicking it moves the history cursor to
/// this entry rather than recording a new one, which is what `Nav::To` is for.
///
/// A visited *source file* is an entry like any function, which is the whole of what
/// Step 1e asked of this list: the history records documents, so it can list one, and the
/// row wears the same kind icon its tab does.
#[derive(Clone)]
struct HistoryRow {
    entry: Document,
    index: usize,
    /// Whether the cursor is on this entry, i.e. this is what is on screen.
    current: bool,
    key: DiffKey,
}

impl PartialEq for HistoryRow {
    fn eq(&self, other: &Self) -> bool {
        // `Document`'s own `PartialEq` is written in terms of `Arc::ptr_eq` for a place
        // in a binary and of text for a file.
        self.entry == other.entry && self.index == other.index && self.current == other.current
    }
}

impl KeyExt for HistoryRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for HistoryRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let active = use_consume::<Active>().0;
        let open = use_consume::<Open>().0;
        // Consuming does not subscribe -- only reading would, and this row never reads
        // the history; it only hands an index back to `navigate`.
        let history = use_consume::<Hist>().0;
        let index = self.index;
        let text = entry_text(&self.entry);

        let background = if self.current {
            palette().selected_bg
        } else if hovering() {
            palette().symbol_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            entry_tooltip(&self.entry),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| navigate(open, history, active, Nav::To(index)))
                .child(entry_icon(&self.entry))
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The clickable name of a relocation target, rendered in place of the meaningless
/// numeric operand.
#[derive(Clone)]
struct RelocationLabel {
    object: Arc<Object>,
    target: Arc<SymbolData>,
}

impl PartialEq for RelocationLabel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && Arc::ptr_eq(&self.target, &other.target)
    }
}

impl Component for RelocationLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let active = use_consume::<Active>().0;
        let open = use_consume::<Open>().0;
        let history = use_consume::<Hist>().0;
        let symbol = Symbol {
            object: self.object.clone(),
            data: self.target.clone(),
        };
        // The same name the disassembler substituted into the instruction text, so the
        // link reads as the operand it stands in for.
        let text = self.target.display().to_owned();

        CursorArea::new().child(
            rect()
                .maybe(hovering(), |rect| {
                    rect.background(palette().link_hover_bg)
                        .corner_radius(6.0)
                        .border(
                            Border::new()
                                .fill(palette().name_hover_fg)
                                .width(BorderWidth {
                                    top: 0.0,
                                    right: 0.0,
                                    bottom: 2.0,
                                    left: 0.0,
                                }),
                        )
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // A press bubbles, and the row under this label pins the line the
                    // instruction came from. Clicking the link means "go there", not "and
                    // also pin the line I am leaving", so the row never sees it.
                    e.stop_propagation();

                    activate(
                        open,
                        active,
                        history,
                        Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
                        Visit::Went,
                    );
                })
                .child(label().text(text).max_lines(1).color(if hovering() {
                    palette().name_hover_fg
                } else {
                    palette().name_fg
                })),
        )
    }
}

/// The branch gutter for one row: a vertical line for every lane running through it, the
/// horizontal run out to the listing where a branch starts or ends here, and an arrowhead
/// where one lands. `width` is the whole symbol's lane count and not this row's, so that
/// the addresses start at the same x on every row of the listing.
///
/// Rects, and not `freya-components`' `canvas()`, which was read before this was written
/// and is the wrong tool twice over. Its `RenderCallback` compares equal to every other
/// one, so a canvas whose *drawing* changed while its layout did not tells the diff
/// nothing -- and a row recycled by a `VirtualScrollView` is exactly that. And a line is a
/// rect: reaching for skia here would put raw drawing code in a file that has none.
///
/// The strokes are positioned absolutely, which is what lets the lanes sit at fixed
/// columns and the two halves of a corner meet in the middle of the row. It is also why
/// `InstructionRow` pads horizontally rather than on all four sides: a line has to reach
/// the row's own top and bottom edges, or the gutter would come out dashed with one gap
/// per row.
fn gutter(width: usize, arrows: RowArrows) -> impl IntoElement {
    let height = code_row_height();
    let middle = height / 2.0;
    // Where an arrowhead points, and where a horizontal run ends. Lane 0 is the innermost,
    // so the lanes are laid out leftwards from here.
    let tip = width as f32 * LANE_WIDTH + ARROW_WIDTH;
    let lane_x = move |lane: usize| (width - 1 - lane) as f32 * LANE_WIDTH + LANE_WIDTH / 2.0;

    // The horizontal run and the arrowhead are the two ends of one gesture -- the row the
    // pointer is on, and the row its branch goes to -- so both are lit exactly when a
    // branch of the hovered row has an end in this one.
    let lit = arrows.lit.corner;

    let stroke = move |left: f32, top: f32, wide: f32, tall: f32, lit: bool| {
        rect()
            .position(Position::new_absolute().left(left).top(top))
            .width(Size::px(wide))
            .height(Size::px(tall))
            .background(if lit {
                palette().branch_hover_fg
            } else {
                palette().branch_fg
            })
    };

    rect()
        .width(Size::px(tip + GUTTER_PAD))
        .height(Size::px(code_row_height()))
        .children((0..width).filter_map(move |lane| {
            let vertical = arrows.lanes.lanes[lane];
            let (top, tall) = match (vertical.top, vertical.bottom) {
                (true, true) => (0.0, height),
                (true, false) => (0.0, middle),
                (false, true) => (middle, height - middle),
                (false, false) => return None,
            };

            Some(
                stroke(
                    lane_x(lane) - BRANCH_STROKE / 2.0,
                    top,
                    BRANCH_STROKE,
                    tall,
                    arrows.lit.lanes[lane],
                )
                .into_element(),
            )
        }))
        .maybe_child(arrows.lanes.stub.map(|lane| {
            stroke(
                lane_x(lane),
                middle - BRANCH_STROKE / 2.0,
                tip - lane_x(lane),
                BRANCH_STROKE,
                lit,
            )
        }))
        // The two strokes of the arrowhead are one stroke turned about its right end,
        // which is the tip, once each way.
        .maybe(arrows.lanes.arrow, |el| {
            el.children([ARROW_ANGLE, -ARROW_ANGLE].map(|angle| {
                stroke(
                    tip - ARROW_STROKE,
                    middle - BRANCH_STROKE / 2.0,
                    ARROW_STROKE,
                    BRANCH_STROKE,
                    lit,
                )
                .rotate(angle)
                .transform_origin(TransformOrigin::right())
                .into_element()
            }))
        })
}

#[derive(Clone)]
struct InstructionRow {
    data: AsmData,
    index: usize,
    /// What this row draws in the gutter, worked out by the list for the same reason
    /// `focused` is: it is an answer about *other* rows -- the lanes lit in row 40 belong
    /// to a branch of row 12 -- and a row that read the hovered index itself would
    /// re-render every visible row on every pointer move whether or not its own picture
    /// changed.
    arrows: RowArrows,
    /// Where the pointer is, which this row writes and does not read. Kept out of the
    /// `PartialEq` below: it is the same handle for the whole life of the list.
    hover: State<Option<usize>>,
    /// Whether the source line the pointer is on is the one this instruction was compiled
    /// from. Worked out by the list rather than read from the focus here, so that a focus
    /// moving between two instructions of one line leaves every row untouched.
    focused: bool,
    /// Whether the source line a click pinned is that same line.
    pinned: bool,
    /// Whether this row is one of the run picked out to be copied. Worked out by the list
    /// for the reason `focused` is: the answer is a range, and a row that read it itself
    /// would re-render on every row the drag passes over rather than only when its own
    /// membership changes.
    selected: bool,
    key: DiffKey,
}

impl PartialEq for InstructionRow {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
            && self.index == other.index
            && self.focused == other.focused
            && self.pinned == other.pinned
            && self.selected == other.selected
            && self.arrows == other.arrows
    }
}

impl KeyExt for InstructionRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for InstructionRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut focused = use_consume::<Focused>().0;
        let mut pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let mut hover = self.hover;
        let index = self.index;
        let width = self.data.lanes.width();
        let instruction = &self.data.assembly.instructions[self.index];

        // Where this row points on the source side. Worked out once here rather than in
        // each of the three handlers, which all need the same answer.
        let at = self.data.position(self.index);
        let focus = at.clone().map(|at| LineFocus {
            at,
            from: FocusOrigin::Instruction(instruction.address),
        });
        let taken = focus.clone();

        let relocation = instruction
            .relocation
            .as_ref()
            .map(|target| RelocationLabel {
                object: self.data.object.clone(),
                target: target.clone(),
            });

        // The disassembler substitutes the relocation target's name for the placeholder
        // operand and says which span it landed in, so the row is three children rather
        // than one: the text before that span, the name as a clickable link, and the
        // text after it. That keeps the link in the operand's own position — inside the
        // brackets of a memory operand, where anything else leaves them empty, and after
        // the `rip+` of a rip-relative one, which is text on the link's left rather than
        // only on its right.
        //
        // A relocated instruction with no such span (the formatter offered no operand to
        // substitute into) has an empty tail, and the link is appended after the whole
        // instruction the way it always was.
        let (head, tail) = match instruction.relocation_span {
            Some(i) if relocation.is_some() && i < instruction.format.len() => {
                (&instruction.format[..i], &instruction.format[i + 1..])
            }
            _ => (&instruction.format[..], &[][..]),
        };

        // Whatever text runs up to the link ends in the formatter's padding to the
        // operand column, and Skia trims trailing whitespace when it measures a
        // paragraph — which would butt the name right up against the mnemonic. Make
        // that padding non-breaking to keep the column.
        let spans = |run: &[(String, SpanKind)], pad_end: bool| {
            let last = run.len().saturating_sub(1);
            run.iter()
                .enumerate()
                .map(|(i, (text, kind))| {
                    let text = if pad_end && i == last {
                        let kept = text.trim_end_matches(' ');
                        format!("{kept}{}", "\u{a0}".repeat(text.len() - kept.len()))
                    } else {
                        text.clone()
                    };

                    Span::new(text)
                        .color(kind_color(*kind))
                        .assembly_font()
                        .font_weight(if *kind == SpanKind::Mnemonic {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                })
                .collect::<Vec<_>>()
        };

        let head = paragraph()
            .max_lines(1)
            .spans_iter(spans(head, relocation.is_some()).into_iter());
        let tail = (!tail.is_empty()).then(|| {
            paragraph()
                .max_lines(1)
                .spans_iter(spans(tail, false).into_iter())
        });

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            // Horizontally only, where it used to be on all four sides: the gutter's lines
            // run to the row's own top and bottom edges, and three pixels of padding at
            // each of them would break every line in the column once per row. Nothing else
            // in the row moves, since its children are centred in it and none of them is
            // as tall as it is.
            .padding(Gaps::new_symmetric(0.0, 3.0))
            .assembly_font()
            .background(row_background(
                hovering(),
                self.focused,
                self.pinned,
                self.selected,
            ))
            // Where a run of rows starts, and why it is the *down* and not the press: a
            // drag is over by the time a press fires, so a selection swept out with the
            // button held has to begin the moment it goes down. It is left-button only,
            // like everything else a row answers to.
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Assembly, index);
                }
            })
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                // Two hovers, because they answer two questions. This one is local and is
                // this row's own background; the index is shared with the whole list,
                // because what the gutter does with it is about rows the pointer is
                // nowhere near.
                hover.set_if_modified(Some(index));
                focused.set_if_modified(taken.clone());
                // The third thing entering a row means, and the one that costs nothing
                // unless a button is down on the run: sweeping the selection out to here.
                // Added to the handler the cross-view focus already uses rather than to
                // one of its own -- a second `pointer_over` would answer the same event
                // twice.
                mark_drag(marked, Pane::Assembly, index);
            })
            .on_pointer_out(move |_| {
                hovering.set_if_modified(false);
                // Given up the way the cross-view focus is, and for the reason spelled out
                // on `release_focus`: `pointerout` on the row being left and `pointerover`
                // on the row being entered are not ordered against each other, so a row
                // may only take back what is still its own.
                if *hover.peek() == Some(index) {
                    hover.set(None);
                }
                release_focus(focused, focus.as_ref());
            })
            .on_press(move |_| {
                // An instruction the debug info places nowhere pins nothing rather than
                // clearing what is pinned: there is no position to point the source pane
                // at, and a click on a compiler-generated prologue byte is not a way of
                // losing the line the reader put there.
                if let Some(at) = at.clone() {
                    pinned.set(Some(Pin {
                        at,
                        reveal: Some(Pane::Source),
                    }));
                }
            })
            // Left of the addresses, and nothing at all for a symbol that branches
            // nowhere inside itself: an empty column would be a column, and most symbols
            // are that one.
            .maybe(width > 0, |el| el.child(gutter(width, self.arrows)))
            .child(
                label()
                    .text(format!("{:016X} ", instruction.address))
                    .min_width(Size::px(200.0))
                    .color(palette().address_fg)
                    .max_lines(1),
            )
            .child(head)
            .maybe_child(relocation)
            .maybe_child(tail)
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// One line of a source file: its number in a gutter, then its text.
///
/// `file` is carried to be pointed at rather than to be drawn: hovering the row tells the
/// assembly pane which position to light up, and a line number without the file it is a
/// line of is not one.
#[derive(Clone)]
struct SourceRow {
    source: SourceText,
    file: Arc<str>,
    index: usize,
    /// Whether the instruction the pointer is on was compiled from this line.
    focused: bool,
    /// Whether the instruction a click pinned was compiled from this line.
    pinned: bool,
    /// Whether this row is one of the run picked out to be copied, told to it by the list
    /// for the reason `InstructionRow`'s is.
    selected: bool,
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.index == other.index
            && self.focused == other.focused
            && self.pinned == other.pinned
            && self.selected == other.selected
    }
}

impl KeyExt for SourceRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SourceRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut focused = use_consume::<Focused>().0;
        let mut pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let index = self.index;
        let source = &self.source.0;

        // The position this row is, and so the one it points the assembly pane at: the
        // file the pane opened, at this row's own line -- its index plus one, for the same
        // reason the gutter below draws that number.
        let at = LinePos {
            file: self.file.clone(),
            line: self.index as u32 + 1,
        };
        let focus = Some(LineFocus {
            at: at.clone(),
            from: FocusOrigin::Source,
        });
        let taken = focus.clone();

        // In range because the list's length is this file's own `lines`, which is at most
        // `blocks.len()` -- and `SyntaxBlocks::get_line` unwraps rather than answering
        // `None`, so being in range is this row's responsibility.
        let spans = source
            .blocks
            .get_line(self.index)
            .iter()
            .map(|(color, node)| {
                let text = match node {
                    TextNode::Range(range) => source.rope.slice(range.clone()).to_string(),
                    // A run of leading indentation, which the highlighter hands over as a
                    // length rather than as text so an editor can draw it as dots. Here it
                    // is plain spaces, since this pane shows a file rather than edits one.
                    TextNode::LineOfChars { len, .. } => " ".repeat(*len),
                };
                Span::new(text).color(*color).assembly_font()
            })
            .collect::<Vec<_>>();

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            .padding(3.0)
            .assembly_font()
            .background(row_background(
                hovering(),
                self.focused,
                self.pinned,
                self.selected,
            ))
            // The same gesture as the assembly pane's, in the same order and for the same
            // reasons: the two panes show code and a reader picking lines out of one of
            // them must not have to learn the other.
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Source, index);
                }
            })
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                focused.set_if_modified(taken.clone());
                mark_drag(marked, Pane::Source, index);
            })
            .on_pointer_out(move |_| {
                hovering.set_if_modified(false);
                release_focus(focused, focus.as_ref());
            })
            // Every source row is a position, so unlike an instruction row this one always
            // has something to pin -- a line no instruction was compiled from included,
            // which the assembly pane answers by staying where it is.
            .on_press(move |_| {
                pinned.set(Some(Pin {
                    at: at.clone(),
                    reveal: Some(Pane::Assembly),
                }));
            })
            .child(
                label()
                    // Line numbers are 1-based, as DWARF's are, so the gutter reads the
                    // way an editor's does. Right-aligned in a column of its own so the
                    // text of every line starts at the same x whatever the number's
                    // width -- and the width is fixed rather than a minimum, because
                    // skia lays a paragraph out to the width it is given and aligns
                    // within *that*: a label free to be wider puts its number at the far
                    // right of the row, on top of the source text.
                    //
                    // The gap after the number is a non-breaking space for the reason
                    // `InstructionRow` uses one: skia trims trailing whitespace when it
                    // measures, which would butt the number against the text.
                    .text(format!("{}\u{a0}", self.index + 1))
                    .width(Size::px(60.0))
                    .text_align(TextAlign::Right)
                    .color(palette().address_fg)
                    .max_lines(1),
            )
            .child(paragraph().max_lines(1).spans_iter(spans.into_iter()))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

// ---------------------------------------------------------------------------
// The tab strip
// ---------------------------------------------------------------------------

/// One tab in the strip: the icon naming its kind, what it is called, an × that closes it,
/// and the pane's own white when it is the one on screen -- the same thing a dock tab
/// header does, so the two bars read as bars of the same kind.
///
/// A stateless helper rather than a component, the hover state belonging to the component
/// that called this, so no hook runs here.
fn chip(
    icon: Element,
    text: String,
    tooltip: String,
    active: bool,
    mut hovering: State<bool>,
    on_activate: impl FnMut(Event<PressEventData>) + 'static,
    mut on_close: impl FnMut(Event<PressEventData>) + 'static,
) -> impl IntoElement {
    // White for the active one, the way a dock tab header is: it reads as the top edge of
    // the pane below it rather than as part of the bar. The hover is the header's own grey
    // one step darker -- `selected_bg`, which is what a dock tab uses for a drop target,
    // would make a hovered chip darker than the active one and so more prominent than it.
    let background = if active {
        palette().pane_bg
    } else if hovering() {
        palette().toggle_hover_bg
    } else {
        Color::TRANSPARENT
    };

    row_tooltip(
        tooltip,
        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .height(Size::px(list_row_height()))
            .padding(Gaps::new_symmetric(0.0, 8.0))
            .spacing(6.0)
            .background(background)
            .border(right_hairline())
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(on_activate)
            .child(icon)
            .child(label().text(elide(&text)).max_lines(1))
            .child(
                rect()
                    // The press bubbles, and the chip under this one activates the tab.
                    // Closing a tab is not a way of first switching to it.
                    .on_press(move |e: Event<PressEventData>| {
                        e.stop_propagation();
                        on_close(e);
                    })
                    .child(
                        label()
                            .text("\u{00d7}")
                            .color(palette().address_fg)
                            .max_lines(1),
                    ),
            ),
    )
}

/// The bar a row of chips sits in. Shaped like `tab_bar`, which is the dock's own, since
/// both of them are a strip of tabs over a pane.
///
/// Horizontally scrollable, because unlike the dock's tabs these are opened by the dozen
/// and a chip that has fallen off the right-hand edge would be unreachable. The scrollbar
/// itself is off: it would eat a third of a one-row bar, and the wheel and a drag
/// both still move it.
fn chip_strip(chips: Vec<Element>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .background(palette().header_bg)
        .border(bottom_hairline())
        .child(
            ScrollView::new()
                .direction(Direction::Horizontal)
                .show_scrollbar(false)
                // The chips sit in a box of their own, whose width is `Inner`. The
                // scroll view's own content box is `fill`, and a child of one is measured
                // against the space *left* in it, so a strip with more chips than fit
                // would hand the ones past the edge no width at all and draw them as a
                // bare ×. Inside an `Inner` box every chip is measured from its own
                // content, the box comes out wider than the view, and that overflow is
                // exactly what there is to scroll.
                .child(
                    rect()
                        .horizontal()
                        .height(Size::fill())
                        .children(chips)
                        .into_element(),
                ),
        )
        .into_element()
}

/// One open document: a function, an object or a source file.
#[derive(Clone)]
struct TabChip {
    entry: Document,
    /// Whether this is the tab the content area is showing, i.e. whether it is [`Active`].
    active: bool,
    key: DiffKey,
}

impl PartialEq for TabChip {
    fn eq(&self, other: &Self) -> bool {
        // `Document`'s own `PartialEq`: `Arc::ptr_eq` for a place in a binary, text for a
        // file.
        self.entry == other.entry && self.active == other.active
    }
}

impl KeyExt for TabChip {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for TabChip {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let active = use_consume::<Active>().0;
        let open = use_consume::<Open>().0;
        let history = use_consume::<Hist>().0;
        let asm_at = use_consume::<AsmAt>().0;
        let src_at = use_consume::<SrcAt>().0;
        let (activated, closed) = (self.entry.clone(), self.entry.clone());

        chip(
            entry_icon(&self.entry),
            entry_text(&self.entry),
            entry_tooltip(&self.entry),
            self.active,
            hovering,
            // A tab in the strip is a place the reader already has open, so switching to
            // it is a move and records nothing. That is Step 1e's rule and the whole
            // reason `activate` is told why it is being called.
            move |_| activate(open, active, history, Some(activated.clone()), Visit::Moved),
            move |_| close_tab(open, active, history, asm_at, src_at, &closed),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The strip of open tabs over the content area.
///
/// **One strip and not two.** Until Step 1 there was a second one inside the Source pane,
/// over its own list of open files, and the two had two notions of what was open. A tab
/// decides what *both* panes show -- the assembly of a function beside the source it came
/// from, or a file beside the assembly for a line in it -- so there is one list of them,
/// and each chip's icon says which of its two sides is the one in charge.
///
/// Over the whole content area rather than inside either pane, which is where the plan's
/// sketch put it: a strip inside one of them would go wherever that pane was dragged,
/// taking the only way of switching documents into a 300px sidebar with it. In the default
/// layout the two are the same thing: the strip is the bar directly above the assembly.
///
/// Nothing at all when no tab is open, so an app with nothing loaded looks exactly as it
/// did before there were tabs.
#[derive(PartialEq)]
struct TabStrip;

impl Component for TabStrip {
    fn render(&self) -> impl IntoElement {
        let open = use_consume::<Open>().0;
        // Reading both subscribes the strip to them, so a tab opened or closed and a
        // change of which one is active each re-render this bar and nothing else.
        let active = use_consume::<Active>().0.read().clone();
        let entries = open.read().tabs().to_vec();

        if entries.is_empty() {
            return rect().into_element();
        }

        chip_strip(
            entries
                .iter()
                .map(|entry| {
                    TabChip {
                        entry: entry.clone(),
                        active: Some(entry) == active.as_ref(),
                        key: DiffKey::None,
                    }
                    .key(entry_key(entry))
                    .into()
                })
                .collect(),
        )
    }
}

/// `text` cut down to [`CHIP_NAME_CHARS`], with an ellipsis where the rest was.
///
/// On a character boundary, so a multi-byte name cannot panic here, and only when there is
/// something to cut: a name that fits keeps its own last character rather than gaining a …
/// for nothing.
fn elide(text: &str) -> String {
    match text.char_indices().nth(CHIP_NAME_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
        None => text.to_owned(),
    }
}

/// What a source file is called in a list: the last component of its path, or the whole
/// of it when there is nothing else to call it.
fn file_name(file: &str) -> String {
    Path::new(file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_owned())
}

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

/// The instruction rows themselves, a component of their own rather than part of
/// `AssemblyTab` because they follow two things the analysis must not follow.
///
/// The pointer focus and the picked-out run change on every pointer move across a row
/// boundary, and the tab above them changes only when a symbol is analysed. Nothing here
/// disassembles any more — `Studied` arrives decoded from the worker (`use_analysis`) —
/// but the split is still what keeps a hover from re-rendering the pane that would have
/// to *ask* for a disassembly, and it is what keeps `AssemblyTab` a plain dispatch over
/// the three things `Analyzed` can be saying.
///
/// The line info comes down as a prop, where it used to be read out of a `Lines` memo
/// here. That memo landed a beat after the disassembly it belonged to, so a pane taking
/// it as a prop rendered twice per selection change; the two now arrive in one value and
/// one write, which is the whole reason they are analysed together.
#[derive(Clone)]
struct InstructionList {
    assembly: Arc<Assembly>,
    /// The whole symbol and not just its object, because these rows answer to a *tab*
    /// as well as to a disassembly: `Document::Assembly(Selection::Symbol(symbol))` is the
    /// key its viewing position is kept under, and it is the one the strip and the session
    /// key by too.
    symbol: Symbol,
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && self.symbol == other.symbol
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Component for InstructionList {
    fn render(&self) -> impl IntoElement {
        // Only the position, not the origin the focus also carries: the rows are told
        // whether they match it, so a focus that moves from one instruction to another
        // compiled from the same line leaves this data equal and the whole list untouched.
        let focus = use_consume::<Focused>()
            .0
            .read()
            .as_ref()
            .map(|focus| focus.at.clone());
        let pinned = use_consume::<Pinned>().0;
        let pin = pinned.read().as_ref().map(|pin| pin.at.clone());
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Assembly);
        // The box the keyboard reaches this pane through. Focus is asked for by the
        // pointer going down anywhere inside it -- `pointer_down` bubbles, so the rows
        // need to know nothing about it -- and freya moves focus on nothing but such a
        // request (`AccessibilityIdExt::request_focus`), so a click in the listing is
        // what makes Ctrl+C mean this listing.
        let a11y = use_a11y();

        let mut controller = use_scroll_controller(ScrollConfig::default);
        // How tall the list is, which `reveal_row` needs to know whether the row it was
        // asked for is on screen already. `VirtualScrollView` measures itself but keeps
        // the answer, so the rect wrapping it -- the same box, since the view is
        // `Size::fill()` inside it -- is measured here instead.
        let mut viewport = use_state(|| 0.0f32);

        // Which row the pointer is on, which the rows write and the gutter reads. It lives
        // here and not in each row because it is the one thing about a row that the rows
        // *around* it need: hovering a `jne` lights its line all the way down to where it
        // lands, which is a row that knows nothing about the pointer.
        let hover = use_state(|| None::<usize>);

        let data = AsmData {
            assembly: self.assembly.clone(),
            object: self.symbol.object.clone(),
            lanes: self.lanes.clone(),
            lines: self.lines.clone(),
        };
        let length = data.assembly.instructions.len();
        // Where this tab was left, put back when it is switched to and written down as it
        // is scrolled. Beside the reveal effect below rather than inside it, because the
        // two answer to different things: a reveal is a click asking for a row, this is a
        // tab remembering one.
        use_kept_position(
            use_consume::<AsmAt>().0,
            use_consume::<Open>().0,
            controller,
            &Document::Assembly(Selection::Symbol(self.symbol.clone())),
            length,
        );
        let touching = hover()
            .map(|row| data.lanes.touching(row))
            .unwrap_or_default();

        let on_key_down = {
            let assembly = self.assembly.clone();
            on_listing_key(marked, Pane::Assembly, length, move |index| {
                assembly
                    .instructions
                    .get(index)
                    .map(asm_line)
                    .unwrap_or_default()
            })
        };

        // The deps are the disassembly and nothing the pointer touches, so this is armed
        // once per symbol; `use_side_effect`'s callback is built by a `use_hook` and would
        // otherwise still be holding the first symbol ever selected.
        use_side_effect_with_deps(&data, move |data: &AsmData| {
            let Some(at) = take_reveal(pinned, Pane::Assembly) else {
                return;
            };

            // The first instruction the line produced, and nothing at all when it produced
            // none here: a line the optimiser folded away, or one belonging to a function
            // that is not the one on screen. Scrolling somewhere arbitrary would be worse
            // than not scrolling, so the request is answered by having answered it.
            let Some(index) = (0..data.assembly.instructions.len())
                .find(|&index| data.position(index).as_ref() == Some(&at))
            else {
                return;
            };

            reveal_row(&mut controller, viewport(), index);
        });

        rect()
            .expanded()
            .a11y_id(a11y)
            .a11y_focusable(true)
            .on_pointer_down(move |_| a11y.request_focus())
            .on_key_down(on_key_down)
            .on_sized(move |e: Event<SizedEventData>| viewport.set_if_modified(e.area.height()))
            .child(
                VirtualScrollView::new_with_data_controlled(
                    AsmRows {
                        data,
                        focus,
                        pin,
                        touching,
                        rows,
                    },
                    move |i, rows: &AsmRows| {
                        let (focused, pinned) = rows.lit(i);
                        InstructionRow {
                            data: rows.data.clone(),
                            index: i,
                            focused,
                            pinned,
                            selected: rows.marked(i),
                            arrows: rows.arrows(i),
                            hover,
                            key: DiffKey::None,
                        }
                        .key(rows.data.assembly.instructions[i].address)
                        .into()
                    },
                    controller,
                )
                .length(length)
                .item_size(code_row_height()),
            )
    }
}

/// The source rows themselves, split out of `SourceTab` the way `InstructionList` is out
/// of `AssemblyTab` -- here not because the pane above is expensive to render, which it is
/// not, but because it has several early returns before it knows which file it is showing.
/// Hooks have to run on every render, and the scroll controller these rows are driven by
/// cannot be armed before the file it would scroll through is known.
#[derive(Clone)]
struct SourceList {
    source: SourceText,
    file: Arc<str>,
    /// The tab these rows belong to, which is what the viewing position is kept under and
    /// is **not** the same as the file being shown: this pane draws a source-driven tab's
    /// own file *and* an assembly-driven tab's companion, and two functions compiled from
    /// one file are two tabs with one file between them. Keying by the document is what
    /// stops them sharing a position they have no reason to share.
    document: Document,
}

impl PartialEq for SourceList {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.document == other.document
    }
}

impl Component for SourceList {
    fn render(&self) -> impl IntoElement {
        let focused = use_consume::<Focused>().0;
        let pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Source);
        let a11y = use_a11y();

        let mut controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);

        // Which line of *this* file each of the two cross-view positions names: a symbol's
        // rows can name several files and the pane has one of them open, so a position in
        // another of them is no line here at all.
        let line_here = |at: &LinePos| (at.file == self.file).then_some(at.line);
        let focus = focused
            .read()
            .as_ref()
            .and_then(|focus| line_here(&focus.at));
        let pin = pinned.read().as_ref().and_then(|pin| line_here(&pin.at));

        let length = self.source.0.lines;
        // The tab and not the file: see `SourceList::document`.
        use_kept_position(
            use_consume::<SrcAt>().0,
            use_consume::<Open>().0,
            controller,
            &self.document,
            length,
        );

        let on_key_down = {
            let source = self.source.clone();
            on_listing_key(marked, Pane::Source, length, move |index| {
                // The file's own text and not the row's spans: what the reader wants
                // pasted is the line as it is on disk, tabs and all, where the row draws
                // a run of leading whitespace as the plain spaces the highlighter hands
                // it over as. The newline is the join's business, not a line's.
                source
                    .0
                    .rope
                    .get_line(index)
                    .map(|line| {
                        let line = line.to_string();
                        line.trim_end_matches(|c| c == '\n' || c == '\r').to_owned()
                    })
                    .unwrap_or_default()
            })
        };

        use_side_effect_with_deps(self, move |list: &SourceList| {
            let Some(at) = take_reveal(pinned, Pane::Source) else {
                return;
            };

            // Nothing to scroll to when the instruction clicked came from a file this pane
            // is not showing, which is the same answer the highlight gives it: an inlined
            // header's line 42 is not line 42 of the file on screen. Nor when the line is
            // past the end of the file, which is source that has moved on since it was
            // compiled rather than debug info to be believed.
            if at.file != list.file {
                return;
            }
            let Some(index) = (at.line as usize)
                .checked_sub(1)
                .filter(|index| *index < list.source.0.lines)
            else {
                return;
            };

            reveal_row(&mut controller, viewport(), index);
        });

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .padding(5.0)
            .child(
                rect()
                    .expanded()
                    .a11y_id(a11y)
                    .a11y_focusable(true)
                    .on_pointer_down(move |_| a11y.request_focus())
                    .on_key_down(on_key_down)
                    .on_sized(move |e: Event<SizedEventData>| {
                        viewport.set_if_modified(e.area.height())
                    })
                    .child(
                        VirtualScrollView::new_with_data_controlled(
                            SourceData {
                                source: self.source.clone(),
                                file: self.file.clone(),
                                focus,
                                pin,
                                rows,
                            },
                            |i, data: &SourceData| {
                                let line = Some(i as u32 + 1);
                                SourceRow {
                                    source: data.source.clone(),
                                    file: data.file.clone(),
                                    index: i,
                                    focused: data.focus == line,
                                    pinned: data.pin == line,
                                    selected: data.rows.is_some_and(|rows| rows.contains(i)),
                                    key: DiffKey::None,
                                }
                                .key(i)
                                .into()
                            },
                            controller,
                        )
                        .length(length)
                        .item_size(code_row_height()),
                    ),
            )
    }
}

fn symbol_info(symbol: &Symbol) -> impl IntoElement {
    let data = &symbol.data;

    rect()
        .width(Size::fill())
        .child(info_line(format!("Symbol: `{}`", data.name)))
        .maybe_child(
            data.demangled
                .as_ref()
                .map(|demangled| info_line(format!("Demangled: `{}`", demangled))),
        )
        .maybe_child(
            data.section
                .as_ref()
                .map(|section| info_line(format!("Section: `{}`", section.name))),
        )
        .child(info_line(format!("Declared size: {} bytes", data.size)))
        // The declared size above is frequently 0 and is only ever displayed; what the
        // app actually reads is `extent`, so that is the number worth showing beside it.
        // `data_in` rather than `data`: the latter is the next-symbol estimate on its own,
        // which is not the range `assembly` decodes or `line_info` is asked about.
        .child(info_line(format!(
            "Extent: {} bytes",
            data.data_in(&symbol.object)
                .map(|bytes| bytes.len())
                .unwrap_or_default()
        )))
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

/// One of the nine dockable views. A tab is a persistent view rather than a slot the
/// active document drives, so each one renders itself off the state it is about and
/// subscribes to it on its own -- which also keeps a change of document from re-rendering
/// the whole tree.
///
/// **This, and not the content area's tab strip, is where a view that is not a document
/// belongs.** A tab in that strip is a [`Document`] -- a place in a binary, or a source
/// file -- which is what makes the Assembly and Source panes able to render "the active
/// tab", the history able to record it and the session able to write it down. A project,
/// the settings and a scratchpad's editor are none of those: there is one of each, they
/// resolve against no object and are no file on disk the panes could open, and neither
/// pane could draw one. So they are views here, where a singleton with its own state
/// already fits, rather than a third `Document` variant that every one of those five
/// places would need an answer for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tab {
    Objects,
    Symbols,
    Info,
    History,
    Assembly,
    Source,
    Project,
    Settings,
    Scratchpad,
}

impl Tab {
    /// The label shown in the tab bar.
    fn title(self) -> &'static str {
        match self {
            Tab::Objects => "Objects",
            Tab::Symbols => "Symbols",
            Tab::Info => "Info",
            Tab::History => "History",
            Tab::Assembly => "Assembly",
            Tab::Source => "Source",
            Tab::Project => "Project",
            Tab::Settings => "Settings",
            Tab::Scratchpad => "Scratchpad",
        }
    }

    /// The Lucide glyph drawn before the title, at the interface font's own size and in
    /// the palette's `icon_fg`.
    ///
    /// Each one names what the pane holds rather than what it looks like: `package` for
    /// **Objects**, an archive being literally a package of members and a linked image the
    /// same thing with one; `square-function` for **Symbols**, since only `SymbolKind::Text`
    /// symbols are kept and the list is therefore a list of functions; `info` and `history`
    /// for the two panes Lucide happens to have named after them; `binary` for **Assembly**,
    /// the one glyph in the set that says *machine code* where `code` and `terminal` say
    /// source and shell; and `file-code` for **Source**, a file rather than bare code
    /// because the pane is a strip of files and shows one of them. **Project** is
    /// `folder-open`, a project being a directory of the app's and pointing at one of the
    /// reader's, and open because it is the one the app is in rather than one of the
    /// several the pane also lists. **Settings** is `settings`, the cog every desktop has
    /// meant this by for thirty years -- the one place in this set where the obvious glyph
    /// is also the right one. **Scratchpad** is `notebook-pen`, which is what the pane
    /// literally is -- a pad with something to write on it with -- where `hammer` and
    /// `play` name the build rather than the thing being built and `flask-conical` calls
    /// it an experiment.
    ///
    /// The name is passed beside the bytes because `ImageSource` keys the raster cache on
    /// a hash of whatever it is given, and hashing nine short names per render is cheaper
    /// than hashing nine SVGs.
    fn icon(self) -> Element {
        let (name, svg) = match self {
            Tab::Objects => ("package", lucide::package()),
            Tab::Symbols => ("square-function", lucide::square_function()),
            Tab::Info => ("info", lucide::info()),
            Tab::History => ("history", lucide::history()),
            Tab::Assembly => ("binary", lucide::binary()),
            Tab::Source => ("file-code", lucide::file_code()),
            Tab::Project => ("folder-open", lucide::folder_open()),
            Tab::Settings => ("settings", lucide::settings()),
            Tab::Scratchpad => ("notebook-pen", lucide::notebook_pen()),
        };

        let side = icon_size();
        SvgViewer::new((name, svg))
            .width(Size::px(side))
            .height(Size::px(side))
            // The colour is given rather than inherited: `SvgViewer` rasterizes only once
            // it knows one, and with none set it waits for an `on_styled` to tell it the
            // inherited text colour, which is a frame late and a frame of nothing in a
            // 26px bar. Setting it also skips the loader, which is off in any case --
            // these are nine 24px glyphs rasterized synchronously out of the binary, and a
            // spinner in a tab header would be a lie about the work being done.
            .color(palette().icon_fg)
            .show_loader(false)
            .into_element()
    }

    fn view(self) -> Element {
        match self {
            Tab::Objects => ObjectsTab.into_element(),
            Tab::Symbols => SymbolsTab.into_element(),
            Tab::Info => InfoTab.into_element(),
            Tab::History => HistoryTab.into_element(),
            Tab::Assembly => AssemblyTab.into_element(),
            Tab::Source => SourceTab.into_element(),
            Tab::Project => ProjectTab.into_element(),
            Tab::Settings => SettingsTab.into_element(),
            Tab::Scratchpad => ScratchpadTab.into_element(),
        }
    }
}

#[derive(PartialEq)]
struct ObjectsTab;

impl Component for ObjectsTab {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;
        let filter = use_state(Filter::default);
        // Which files the reader has folded open. It belongs to the tab exactly the way
        // the filter does — a fold is a view of a list, not part of the session — so it
        // is a `use_state` here and nothing about it reaches `project.rs`. The set holds
        // group keys, which are `Arc` pointers (see `TreeRow::File`), so an entry left
        // behind by a file that has since been closed is harmless: nothing looks it up
        // again.
        let expanded = use_state(HashSet::<usize>::new);
        // A memo, not a walk per row: the `VirtualScrollView` has to be told how many
        // rows there are before it builds any of them, and the answer depends on the
        // filter *and* on which files are open. It is tens of names rather than the
        // symbol list's hundred thousand, but the length has to come from somewhere and
        // that somewhere is the flattened tree.
        // Reading `loading` here is what puts a file on screen the moment it is asked for
        // and takes the indicator off it when the last of its objects has landed: the memo
        // follows the list of files being read exactly as it follows the objects.
        let tree = use_memo(move || {
            ObjectTree::new(
                &objects.read(),
                &loading.read(),
                &filter.read().matcher(),
                &expanded.read(),
            )
        });
        let tree = tree.read().clone();
        // The selected object as the address its rows are keyed by, rather than as the
        // `Arc` itself: everything handed to a `VirtualScrollView` has to be `PartialEq`
        // and an `Object` is not, while pointer identity — which is the only identity the
        // UI uses anyway — compares as a number.
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Object(object))) => Some(Arc::as_ptr(object).addr()),
            _ => None,
        };
        let length = tree.len();

        filter_pane(
            filter,
            palette().pane_bg,
            VirtualScrollView::new_with_data(
                (tree, selected, expanded),
                |row,
                 (tree, selected, expanded): &(
                    ObjectTree,
                    Option<usize>,
                    State<HashSet<usize>>,
                )| {
                    match tree.row(row) {
                        TreeRow::File {
                            name,
                            path,
                            group,
                            members,
                            expansion,
                            loading,
                        } => ArchiveRow {
                            name: name.clone(),
                            path: path.clone(),
                            members: *members,
                            expansion: *expansion,
                            loading: *loading,
                            group: *group,
                            expanded: *expanded,
                            key: DiffKey::None,
                        }
                        // The path as well as the group, since a file with nothing behind
                        // it yet has no group and the path is the only identity it has.
                        // The two agree for every row that has both: one file is one row.
                        .key((*group, path))
                        .into(),
                        TreeRow::Object { object, member } => ObjectRow {
                            object: object.clone(),
                            selected: *selected == Some(Arc::as_ptr(object).addr()),
                            member: *member,
                            key: DiffKey::None,
                        }
                        .key(Arc::as_ptr(object).addr())
                        .into(),
                    }
                },
            )
            .length(length)
            .item_size(list_row_height()),
        )
    }
}

#[derive(PartialEq)]
struct SymbolsTab;

impl Component for SymbolsTab {
    fn render(&self) -> impl IntoElement {
        let symbols = use_consume::<Symbols>().0;
        let filter = use_state(Filter::default);
        // The one list where the filtering has to be a memo. It is 115k names on
        // `viewer-sample`, so the pass belongs to a change of the list or of the filter
        // rather than to a render — and the rows cannot each test themselves either, since
        // the `VirtualScrollView` has to be told its length before it builds any of them.
        let filtered =
            use_memo(move || Filtered::new(symbols.read().clone(), &filter.read().matcher()));
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Active>().0.read() {
            Some(Document::Assembly(Selection::Symbol(symbol))) => Some(symbol.clone()),
            _ => None,
        };
        let length = filtered.len();

        filter_pane(
            filter,
            palette().symbol_pane_bg,
            VirtualScrollView::new_with_data(
                (filtered, selected),
                |row, (filtered, selected): &(Filtered, Option<Symbol>)| {
                    // The row's place in the filtered list is not the symbol's place in
                    // the list it was filtered out of, and everything below — the key, the
                    // selection, `SymbolRow` itself — is about the symbol.
                    let index = filtered.index(row);
                    let symbol = &filtered.symbols.0[index];
                    SymbolRow {
                        symbols: filtered.symbols.clone(),
                        index,
                        selected: selected.as_ref() == Some(symbol),
                        key: DiffKey::None,
                    }
                    .key(Arc::as_ptr(&symbol.data).addr())
                    .into()
                },
            )
            .length(length)
            .item_size(list_row_height()),
        )
    }
}

#[derive(PartialEq)]
struct InfoTab;

impl Component for InfoTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Active>().0.read().clone();

        match &current {
            None => placeholder("Nothing selected"),
            Some(Document::Source(_)) => placeholder("No symbol selected"),
            Some(Document::Assembly(Selection::Object(object))) => rect()
                .expanded()
                .background(palette().pane_bg)
                .child(info_line(format!("Object: `{}`", object.name)))
                .child(info_line(format!("Format: {:?}", object.format)))
                .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
                .into(),
            Some(Document::Assembly(Selection::Symbol(symbol))) => rect()
                .expanded()
                .background(palette().pane_bg)
                .child(ScrollView::new().child(symbol_info(symbol).into_element()))
                .into(),
        }
    }
}

#[derive(PartialEq)]
struct HistoryTab;

impl Component for HistoryTab {
    fn render(&self) -> impl IntoElement {
        let history = use_consume::<Hist>().0;
        let filter = use_state(Filter::default);
        // A session's history is a handful of entries, so this is the objects list's
        // arrangement and not the symbol list's: filtered where the rows are built.
        let matcher = filter.read().matcher();

        // Reading subscribes this tab to the history, so a recorded entry or a moved
        // cursor re-renders the list and nothing else. `visited` is asked of the whole
        // history rather than of the rows, because an empty list means two different
        // things — nowhere has been yet, or nowhere that has been matches — and the two
        // are worth different words.
        let (rows, visited): (Vec<Element>, bool) = {
            let history = history.read();
            let cursor = history.cursor();
            let visited = history.recent().len() > 0;
            let rows = history
                .recent()
                .filter(|(_, entry)| matcher.matches(&entry_text(entry)))
                .map(|(index, entry)| {
                    HistoryRow {
                        entry: entry.clone(),
                        index,
                        current: cursor == Some(index),
                        key: DiffKey::None,
                    }
                    .key((index, entry_key(entry)))
                    .into()
                })
                .collect();

            (rows, visited)
        };

        // A plain `ScrollView` rather than a `VirtualScrollView`: a session's history is
        // a handful of entries, the rows are one label each, and this way the list is
        // built straight from the state it read instead of having to route the entries
        // through `new_with_data`. The same shape the objects list uses.
        filter_pane(
            filter,
            palette().symbol_pane_bg,
            match (visited, rows.is_empty()) {
                (false, _) => placeholder("Nothing visited yet"),
                (true, true) => placeholder("No matches"),
                (true, false) => ScrollView::new()
                    .child(rect().width(Size::fill()).children(rows).into_element())
                    .into_element(),
            },
        )
    }
}

/// Which file the Source pane is drawing, and whose side of the tab it is.
///
/// The one place either pane decides that, so the Source pane and the effect that drops
/// its picked-out rows cannot disagree about which listing is up. A **subject** is the
/// tab's own file, a **companion** is the file the drawn symbol was compiled from — and
/// which of the two it is comes from the active document's kind and from nothing else.
///
/// The companion comes out of the *analysis* and not out of `Active`, because the two
/// disagree for as long as the worker takes and it is the analysis that says which symbol
/// is actually drawn. `SymbolLines` carries the file beside the line info for exactly
/// this reason.
enum SourceSide {
    Subject(Arc<str>),
    Companion(Arc<str>),
}

impl SourceSide {
    fn file(&self) -> &Arc<str> {
        match self {
            SourceSide::Subject(file) | SourceSide::Companion(file) => file,
        }
    }
}

fn source_side(active: Option<&Document>, analysis: &Analyzed) -> Option<SourceSide> {
    match active? {
        Document::Source(file) => Some(SourceSide::Subject(file.clone())),
        Document::Assembly(_) => {
            let shown = analysis.shown.as_ref()?;
            shown.lines.file.clone().map(SourceSide::Companion)
        }
    }
}

/// The Assembly pane: a dispatch over the things [`Analyzed`] can be saying, and no work
/// of its own at all.
///
/// It reads the analysis and not the active document for everything it draws, which is
/// what keeps the listing and the rows that draw it in step: while the worker is catching
/// up the two disagree, and it is the analysis — the symbol whose disassembly is actually
/// in hand — that everything from the gutter to the kept scroll position is keyed by.
///
/// The one thing it does ask the active document is what *kind* of tab this is, because
/// on a source-driven one the assembly is the **companion** side and there is nothing to
/// put in it yet: which symbols a source line compiled into is Step 2's index, and picking
/// one of them is Step 1d. Until then this pane is empty for such a tab, rather than
/// carrying the analysis' "No symbol selected" over from a tab where that is the answer.
#[derive(PartialEq)]
struct AssemblyTab;

impl Component for AssemblyTab {
    fn render(&self) -> impl IntoElement {
        let active = use_consume::<Active>().0;
        let analysis = use_consume::<Analysis>().0;

        let source_driven = matches!(&*active.read(), Some(Document::Source(_)));
        if source_driven {
            return rect().expanded().background(palette().asm_pane_bg).into();
        }

        let analysis = analysis.read().clone();
        let studied = match analysis.showing() {
            Showing::Listing(studied) => studied,
            Showing::Message(text) => return placeholder(text),
            Showing::Nothing => return rect().expanded().background(palette().asm_pane_bg).into(),
        };
        let Some(assembly) = studied.assembly.clone() else {
            return rect()
                .padding(5.0)
                .child(label().text("Assembly unavailable"))
                .into();
        };
        // An architecture no backend claims is a *third* answer, and the one above is now
        // only "this symbol has no bytes". Naming it matters more than it looks: the
        // listing being empty is indistinguishable from a function that is empty, and
        // before the architecture reached the decoder this case was a confident page of
        // nonsense rather than nothing at all.
        if let Some(architecture) = assembly.undecodable {
            return placeholder(format!("No disassembler for {architecture}"));
        }

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .padding(5.0)
            .background(palette().asm_pane_bg)
            .child(InstructionList {
                assembly,
                symbol: studied.symbol.clone(),
                lanes: studied.lanes.clone(),
                lines: studied.lines.clone(),
            })
            .into()
    }
}

/// The bar over the Source pane naming the file it is showing as a **companion**, and
/// opening that file as a tab of its own when it is pressed.
///
/// It exists because the strip no longer does the job. A companion file is not a tab —
/// it is one side of the function's tab — so nothing else in the window says which file
/// the pane is drawing, and the whole path used to be a tooltip on a chip that is gone.
///
/// Pressing it is also the way a **source-driven tab is made**: the reader is looking at a
/// file and says "this file, on its own", and what they get is the same kind of thing the
/// symbol list gives them. Until the project explorer and the source search land
/// (`notes/Goals.md`, *Panels and tabs*) this is the only door into one, which is why it
/// is a press and not a label.
///
/// A subject file gets no header: it is the tab, and the strip already names it.
///
/// The two states come in as arguments and are not consumed here: this is called from
/// inside a `match`, and a hook may only run unconditionally in a component's body.
fn companion_header(
    open: State<Tabs<Document>>,
    active: State<Option<Document>>,
    history: State<History>,
    file: Arc<str>,
) -> Element {
    let document = Document::Source(file.clone());

    row_tooltip(
        file.to_string(),
        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .padding(Gaps::new_symmetric(0.0, 8.0))
            .spacing(6.0)
            .background(palette().header_bg)
            .border(bottom_hairline())
            .on_press(move |_| activate(open, active, history, Some(document.clone()), Visit::Went))
            .child(entry_icon(&Document::Source(file.clone())))
            .child(label().text(file_name(&file)).max_lines(1)),
    )
    .into_element()
}

/// The Source pane: the tab's source side, whichever of the two sides that is.
#[derive(PartialEq)]
struct SourceTab;

impl Component for SourceTab {
    fn render(&self) -> impl IntoElement {
        let active = use_consume::<Active>().0;
        let open = use_consume::<Open>().0;
        let history = use_consume::<Hist>().0;
        // Consumed unconditionally, hooks having to run on every render, and read here
        // because the companion file comes out of it -- and because reading it is what
        // subscribes this tab to it, so the pane fills in when a newly selected symbol's
        // line info is worked out, without the root re-rendering.
        let analysis = use_consume::<Analysis>().0.read().clone();
        let side = source_side(active.read().as_ref(), &analysis);

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, so the two
            // panes cannot disagree about whether anything is selected -- with one more
            // case of its own, since a symbol can be analysed and still name no file.
            return match analysis.showing() {
                Showing::Message(text) => placeholder(text),
                Showing::Nothing => rect().expanded().background(palette().pane_bg).into(),
                Showing::Listing(studied) if studied.lines.info.is_some() => {
                    placeholder("No source file for this symbol")
                }
                Showing::Listing(_) => placeholder("No line info"),
            };
        };

        let file = side.file().clone();
        let document = match &side {
            SourceSide::Subject(file) => Document::Source(file.clone()),
            // The *drawn* symbol's tab and not the active one, which is the same rule the
            // assembly side follows: while the worker is catching up the two disagree, and
            // a row written down against the tab that is arriving would be a row of the
            // listing that is leaving.
            SourceSide::Companion(_) => match analysis.shown.as_ref() {
                Some(studied) => Document::Assembly(Selection::Symbol(studied.symbol.clone())),
                None => return rect().expanded().background(palette().pane_bg).into(),
            },
        };

        rect()
            .expanded()
            // The header takes its own height and the list is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().pane_bg)
            .maybe_child(match &side {
                SourceSide::Companion(file) => {
                    Some(companion_header(open, active, history, file.clone()))
                }
                SourceSide::Subject(_) => None,
            })
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // Named in the message because the path is the only clue to *why*:
                    // source built on another machine, moved, or deleted since all look
                    // alike from here.
                    .child(match source_text(Path::new(&*file)) {
                        Some(source) => SourceList {
                            source,
                            file,
                            document,
                        }
                        .into_element(),
                        None => placeholder(format!("Source file not found: {file}")),
                    }),
            )
            .into()
    }
}

/// The heading over one section of the project view, with whatever the section's own
/// action is on the right of it.
///
/// A hairline under it rather than a weight or a colour of its own: the pane is a column
/// of short sections, and a rule is what says where one ends without adding a fifth text
/// size to a window that has four.
fn section_heading(text: &str, action: Option<Element>) -> impl IntoElement {
    rect()
        .width(Size::fill())
        // Padded rather than a fixed row height, unlike every other bar in the app: a
        // section's action is a `Button`, which is taller than a row, and a fixed height
        // would draw the rule through it.
        .padding(Gaps::new_symmetric(2.0, 0.0))
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .border(bottom_hairline())
        .child(
            label()
                .text(text.to_owned())
                .width(Size::flex(1.0))
                .font_weight(FontWeight::BOLD)
                .max_lines(1),
        )
        .maybe_child(action)
}

/// One labelled field: what it is on the left in a fixed column, what it says on the
/// right taking the rest.
///
/// The column is fixed for `SourceRow`'s reason -- the values line up under one another
/// whatever the labels turn out to be -- and it is a `flex` row so that a text box in the
/// value position takes the width that is left rather than the width of its contents.
fn field_row(name: &str, value: impl IntoElement) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(
            label()
                .text(name.to_owned())
                .width(Size::px(FIELD_LABEL_WIDTH))
                .color(palette().address_fg)
                .max_lines(1),
        )
        .child(value)
}

/// One binary the project has open, and how many objects came out of it.
///
/// Read off the loaded objects rather than off the saved `binaries`, because that is what
/// `project::binaries` derives the saved list *from*: what this row draws is therefore
/// what the next write will say, and a file closed from the Objects panel leaves this
/// list in the same instant it leaves that one.
fn binary_row(path: &Path, objects: usize) -> Element {
    let text = path.to_string_lossy().into_owned();
    row_tooltip(
        text.clone(),
        rect()
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(8.0)
            .content(Content::Flex)
            .child(tree_name(text, false))
            .child(
                label()
                    .text(match objects {
                        1 => "1 object".to_owned(),
                        many => format!("{many} objects"),
                    })
                    .color(palette().address_fg)
                    .max_lines(1),
            ),
    )
    .into_element()
}

/// One project in the recent list. Pressing it leaves the project on screen and opens
/// this one in its place.
#[derive(Clone, PartialEq)]
struct RecentRow {
    recent: Recent,
    key: DiffKey,
}

impl KeyExt for RecentRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for RecentRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let id = self.recent.id.clone();
        let recent = &self.recent;

        // The id where there is no name, in the colour a tag is drawn in: a project is
        // its directory, so the one thing it always has to be called is that directory's
        // name -- and drawing it as a name would claim the reader chose it.
        let (text, color) = match &recent.name {
            Some(name) => (name.clone(), palette().text_fg),
            None => (recent.id.as_str().to_owned(), palette().address_fg),
        };
        // What is known about it without opening it: where it points, and how much is in
        // it. Both come out of that project's own file.
        let about = match &recent.directory {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => match recent.binaries {
                0 => "empty".to_owned(),
                1 => "1 binary".to_owned(),
                many => format!("{many} binaries"),
            },
        };

        row_tooltip(
            recent.id.as_str().to_owned(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .padding(Gaps::new_symmetric(0.0, 4.0))
                .spacing(8.0)
                .content(Content::Flex)
                .background(match hovering() {
                    true => palette().object_hover_bg,
                    false => Color::TRANSPARENT,
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| switch_project(states, id.clone()))
                .child(
                    label()
                        .text(text)
                        .width(Size::flex(1.0))
                        .color(color)
                        .max_lines(1),
                )
                .child(label().text(about).color(palette().address_fg).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The Project pane: everything the app knows about the project it is in, the two things
/// about it the reader can say, and the other projects they can go to.
///
/// **One view and not two**, where `notes/Goals.md` asks for a project view and a
/// recent-projects view separately. They are one question -- which project am I in, and
/// what else is there -- and the recent list is how the reader *leaves* the project the
/// rest of the pane describes, so a tab of its own would be a tab that is empty in every
/// session where a project was reopened, which is all of them after the first. The goal's
/// "if none was open" case is answered by the pane itself: with no project the top half
/// says so and the list is the whole of what there is to do.
///
/// The recent list deliberately leaves out the project that is open. The pane above it is
/// already describing that one, in more detail and from live state rather than from a
/// file, so a row for it would be a second and staler copy of the name being typed three
/// lines higher up.
#[derive(PartialEq)]
struct ProjectTab;

impl Component for ProjectTab {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut proj = states.proj;
        let objects = states.objects;

        // Every row of the recent list is a small read of another project's own file, so
        // it is read when this view is mounted and again when the open project changes --
        // never per render, which a hover is. The effect also runs once on mount, which
        // costs one extra reading of a handful of short files and buys the alternative
        // not being a frame of "no recent projects" before the first one arrives.
        let mut recents = use_state(project::recent_projects);
        let open = proj.read().clone();
        use_side_effect_with_deps(&open.id, move |_: &Option<ProjectId>| {
            recents.set(project::recent_projects());
        });

        // What is open, grouped the way the saved list is: by path, in the order the
        // files were opened.
        let binaries: Vec<Element> = {
            let objects = objects.read();
            project::binaries(&objects)
                .into_iter()
                .map(|path| {
                    let count = objects.iter().filter(|object| object.path == path).count();
                    binary_row(&path, count)
                })
                .collect()
        };

        let others: Vec<Element> = recents
            .read()
            .iter()
            .filter(|recent| Some(&recent.id) != open.id.as_ref())
            .map(|recent| {
                RecentRow {
                    recent: recent.clone(),
                    key: DiffKey::None,
                }
                .key(recent.id.as_str().to_owned())
                .into()
            })
            .collect();

        let on_choose = move |_| {
            spawn(async move {
                let Some(handle) = AsyncFileDialog::new()
                    .set_title("Choose the project's directory...")
                    .pick_folder()
                    .await
                else {
                    return;
                };
                proj.write().directory = handle.path().to_string_lossy().into_owned();
            });
        };

        rect()
            .expanded()
            .background(palette().pane_bg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Project", None))
                        // The two editable fields. Each writes straight into `Proj`, so a
                        // keystroke is a state change the save observer sees like any
                        // other -- and `name` and `directory` live in `project.toml`,
                        // which is the file written at once, so a rename is on disk before
                        // the next click. That is `Goals.md`'s "user project changes save
                        // immediately" taken literally, and it costs a few hundred bytes
                        // written atomically per keystroke of something typed once.
                        .child(field_row(
                            "Name",
                            Input::new(
                                proj.into_writable()
                                    .map(|open| &open.name, |open| &mut open.name),
                            )
                            // An empty box is a project that has not been named, which is
                            // what makes it anonymous -- so the placeholder says that
                            // rather than inviting a name.
                            .placeholder("Unnamed")
                            .compact()
                            .width(Size::flex(1.0)),
                        ))
                        .child(field_row(
                            "Directory",
                            rect()
                                .width(Size::flex(1.0))
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .content(Content::Flex)
                                .spacing(6.0)
                                .child(
                                    Input::new(
                                        proj.into_writable().map(
                                            |open| &open.directory,
                                            |open| &mut open.directory,
                                        ),
                                    )
                                    .placeholder("None")
                                    .compact()
                                    .width(Size::flex(1.0)),
                                )
                                .child(Button::new().on_press(on_choose).child("Choose...")),
                        ))
                        // The directory the project is *stored* in, which is its identity
                        // and is never written inside either of the files in it. Shown
                        // because it is what the recent list names a project by and what
                        // a reader looking for these files on disk needs.
                        .child(field_row(
                            "Stored as",
                            label()
                                .text(match &open.id {
                                    Some(id) => id.as_str().to_owned(),
                                    // Not an error and not a missing project: a project
                                    // directory is made by the first write that has
                                    // something to put in it, so a run in which nothing
                                    // has been opened or named has none yet.
                                    None => "not saved yet".to_owned(),
                                })
                                .color(palette().address_fg)
                                .max_lines(1),
                        ))
                        .child(section_heading("Binaries", None))
                        .child(match binaries.is_empty() {
                            true => info_line("Nothing open".to_owned()).into_element(),
                            false => rect().width(Size::fill()).children(binaries).into_element(),
                        })
                        .child(section_heading(
                            "Recent projects",
                            Some(
                                Button::new()
                                    .on_press(move |_| new_project(states))
                                    .child("New project")
                                    .into_element(),
                            ),
                        ))
                        .child(match others.is_empty() {
                            true => info_line("No other projects".to_owned()).into_element(),
                            false => rect().width(Size::fill()).children(others).into_element(),
                        }),
                ),
            )
            .into_element()
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// The column a setting's status sits in, on the right of the value: wide enough for the
/// **Clear** button that appears there when the setting is the reader's own, so that the
/// value boxes above and below one another end at the same x whichever state each is in.
const SETTING_STATUS_WIDTH: f32 = 76.0;

/// How far one press of the size stepper moves a font, and the range it may be moved in.
///
/// Half a point, because that is the granularity the desktops themselves store (KDE writes
/// integers, Gnome's Pango descriptions and the Windows `LOGFONTW` conversion both produce
/// fractions) and because a whole point is a visible jump at nine of them. The bounds are
/// not a claim about taste: below five points the window's own chrome stops being legible
/// enough to change the setting back, and above thirty-two a row is taller than the
/// toolbar. A hand-edited `settings.toml` may still say anything, and is honoured -- these
/// bound the *stepper*, not the file.
const SIZE_STEP: f32 = 0.5;
const MIN_POINTS: f32 = 5.0;

const MAX_POINTS: f32 = 32.0;

/// The column the size is written in, between the two stepper buttons.
const SIZE_VALUE_WIDTH: f32 = 52.0;

/// A point size as the page writes it: `9`, `10.5`, and never `10.50` or `9.0`.
///
/// One decimal, because that is what the stepper's half-points need and what a desktop's
/// answer can carry (Gnome multiplies its size by `text-scaling-factor`, so 11 at 1.25 is
/// 13.75). Rounded for display only -- the value stored is the value stepped.
fn points_text(points: f32) -> String {
    let rounded = (points * 10.0).round() / 10.0;

    match rounded.fract() == 0.0 {
        true => format!("{rounded:.0}"),
        false => format!("{rounded:.1}"),
    }
}

/// One overridable setting: its name, what it says, and -- the whole point of this page --
/// whether what it says is the reader's answer or the one they are inheriting.
///
/// `notes/Goals.md` asks for "a default being unspecified with clear visual distinction",
/// and this is where that is cashed out. Three cues, deliberately more than one, because a
/// single quiet difference is one a reader has to be told about:
///
/// - **The name changes colour.** An overridden setting is written in `name_fg`, the
///   colour a function's name is drawn in; an inherited one in `address_fg`, the colour
///   everything that recedes is drawn in. That is the cue that reads down the column
///   without looking at any one row.
/// - **The value reads as text or as a placeholder.** An override is real text in the box;
///   an unspecified field shows what it is falling through to, in the box's placeholder
///   colour, so the reader is never asked to remember what the desktop said.
/// - **The Clear button is only there when there is something to clear.** It is also the
///   *only* way back to unspecified, which is why it is a button and not a keystroke: an
///   empty family box is unspecified, but a size has no empty state to type.
fn setting_row(
    name: &str,
    overridden: bool,
    value: impl IntoElement,
    clear: impl FnMut(Event<PressEventData>) + 'static,
) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height() + 8.0))
        .horizontal()
        .cross_align(Alignment::Center)
        .content(Content::Flex)
        .spacing(8.0)
        .child(
            label()
                .text(name.to_owned())
                .width(Size::px(FIELD_LABEL_WIDTH))
                // The same pair the value beside it uses: what the reader said is
                // ordinary interface text, what they are inheriting recedes into the
                // colour everything secondary in this app is written in.
                .color(match overridden {
                    true => palette().text_fg,
                    false => palette().address_fg,
                })
                .max_lines(1),
        )
        .child(value)
        .child(
            rect()
                .width(Size::px(SETTING_STATUS_WIDTH))
                .horizontal()
                .main_align(Alignment::End)
                .cross_align(Alignment::Center)
                .child(match overridden {
                    true => Button::new()
                        .compact()
                        .on_press(clear)
                        .child("Clear")
                        .into_element(),
                    // Not "unset" and not blank: the reader is being told where the value
                    // in the box beside this came from, which is the question the page
                    // exists to answer.
                    false => label()
                        .text("inherited")
                        .color(palette().address_fg)
                        .max_lines(1)
                        .into_element(),
                }),
        )
}

/// One of the two fonts, as three rows: the family, the size, and a line of the font
/// itself.
///
/// The preview earns its place on the fixed-width half and is kept on both for symmetry:
/// the interface font is already every label in the window, but the fixed-width one is
/// only visible when a symbol with code in it is open, and a reader changing it with the
/// Assembly pane on a placeholder would otherwise be typing family names at nothing. The
/// digits and the `l1I`/`O0` pairs are in it because they are what a monospaced face is
/// actually chosen for.
fn font_section(
    title: &str,
    edited: EditedFont,
    inherited: &Font,
    resolved: &Font,
    family: Writable<String>,
    size: impl FnMut(Option<f32>) + Clone + 'static,
) -> Element {
    let inherited_family = inherited
        .families
        .first()
        .map(|family| family.to_string())
        .unwrap_or_default();
    // What the stepper moves from: the reader's size where there is one, and otherwise the
    // one being inherited -- so the first press is one step away from what is on screen
    // rather than a jump to some number of this file's own choosing.
    let points = edited.size.unwrap_or(inherited.points);
    let step = |by: f32| {
        let mut size = size.clone();
        move |_: Event<PressEventData>| {
            let moved = (points + by).clamp(MIN_POINTS, MAX_POINTS);
            // Back onto the half-point grid, so that stepping away from a desktop's
            // 13.75 and back again lands on 13.75's neighbours rather than on a drift of
            // its own.
            size(Some((moved / SIZE_STEP).round() * SIZE_STEP));
        }
    };
    let mut clear_size = size.clone();

    rect()
        .width(Size::fill())
        .child(section_heading(title, None))
        .child(setting_row(
            "Family",
            given(&edited.family).is_some(),
            Input::new(family.clone())
                .placeholder(inherited_family)
                .compact()
                .width(Size::flex(1.0)),
            move |_| family.clone().set(String::new()),
        ))
        .child(setting_row(
            "Size",
            edited.size.is_some(),
            rect()
                .width(Size::flex(1.0))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .child(
                    Button::new()
                        .compact()
                        .on_press(step(-SIZE_STEP))
                        .child("-"),
                )
                .child(
                    label()
                        .text(format!("{} pt", points_text(points)))
                        // A fixed column, so that `+` does not move under the finger as
                        // the number beside it grows a digit or loses a decimal -- the
                        // reason `SourceRow`'s line-number gutter is a fixed width and not
                        // a minimum, and it matters more here, where the thing that would
                        // move is the button being pressed again.
                        .width(Size::px(SIZE_VALUE_WIDTH))
                        .text_align(TextAlign::Center)
                        .color(match edited.size {
                            Some(_) => palette().text_fg,
                            None => palette().address_fg,
                        })
                        .max_lines(1),
                )
                .child(Button::new().compact().on_press(step(SIZE_STEP)).child("+")),
            move |_| clear_size(None),
        ))
        .child(
            rect()
                .width(Size::fill())
                .padding(Gaps::new(2.0, 0.0, 8.0, FIELD_LABEL_WIDTH + 8.0))
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .text("Disassembly 0123 l1I O0 {}")
                        .font(resolved)
                        .color(palette().text_fg)
                        .max_lines(1),
                ),
        )
        .into()
}

/// The Settings pane: the theme, the two fonts, and which of those the reader has actually
/// chosen.
///
/// **A view and not a document**, which is the rule 8e settled and this inherits: the
/// content strip holds `Selection`s -- a place in a binary -- and there is one settings
/// page, resolving against no object, that neither code pane could draw. So it is a `Tab`,
/// the mechanism the app already has for "a pane with its own state the reader can put
/// where they like", and it is excluded from the saved session for free, a dock layout not
/// being persisted.
///
/// **What it writes and when.** Every control writes straight into `Prefs`, and
/// [`use_settings`] at the root is what turns that into a font, a theme and a file --
/// there is no Apply button and no autosave timer, `Settings::save` writing at once by
/// design. So a press here is on disk and on screen before the finger is off the button,
/// which is what makes the page its own preview: there is no "sample text" widget for the
/// interface font because the whole window is one.
#[derive(PartialEq)]
struct SettingsTab;

impl Component for SettingsTab {
    fn render(&self) -> impl IntoElement {
        let mut prefs = use_consume::<Prefs>().0;
        let edited = prefs.read().clone();
        // Both halves of what the page draws, from the same two functions the root
        // resolves with: what the reader would be getting with nothing set, and what they
        // are getting now. Cheap -- the desktop lookups behind them are cached for the
        // life of the process (`fonts::desktop_answer`).
        let inherited = fonts::inherited();
        let resolved = fonts::resolve(&edited.settings());

        // Only a question at all under `Desktop`, which is exactly what `resolve_appearance`
        // says: a reader who named a theme is answered by their own answer, so telling them
        // what the desktop prefers would be telling them about something that is not
        // happening. Reading it here also subscribes this pane, so the line follows a
        // desktop that changes its mind while the page is open.
        let following = (edited.theme == ThemeChoice::Desktop).then(|| {
            let preferred = *Platform::get().preferred_theme.read();

            info_line(format!(
                "Following the desktop, which prefers {}.",
                match preferred {
                    PreferredTheme::Light => "light",
                    PreferredTheme::Dark => "dark",
                }
            ))
            .into_element()
        });

        let themes = [
            (ThemeChoice::Light, "Light"),
            (ThemeChoice::Dark, "Dark"),
            (ThemeChoice::Desktop, "Desktop"),
        ];

        rect()
            .expanded()
            .background(palette().pane_bg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Appearance", None))
                        .child(field_row(
                            "Theme",
                            SegmentedButton::new().children(themes.map(|(choice, text)| {
                                ButtonSegment::new()
                                    .key(text)
                                    .selected(edited.theme == choice)
                                    .on_press(move |_| {
                                        prefs.write().theme = choice;
                                    })
                                    .child(text)
                                    .into()
                            })),
                        ))
                        .maybe_child(following)
                        .child(font_section(
                            "Interface font",
                            edited.interface.clone(),
                            &inherited.ui,
                            &resolved.ui,
                            prefs.into_writable().map(
                                |edited| &edited.interface.family,
                                |edited| &mut edited.interface.family,
                            ),
                            move |size| prefs.write().interface.size = size,
                        ))
                        .child(font_section(
                            "Fixed-width font",
                            edited.fixed.clone(),
                            &inherited.mono,
                            &resolved.mono,
                            prefs.into_writable().map(
                                |edited| &edited.fixed.family,
                                |edited| &mut edited.fixed.family,
                            ),
                            move |size| prefs.write().fixed.size = size,
                        ))
                        // Said here rather than left to be discovered, because it is the
                        // one consequence of a font change that is not a font: a row is
                        // its own font's size plus `ROW_LEADING`, and that is the
                        // `item_size` of the views over it, so a list gets taller with the
                        // font it is drawn in rather than clipping it. Two numbers and not
                        // one blended answer, because each half of the page above moves
                        // exactly one of them -- which is the whole of what a reader wants
                        // to know before stepping a size.
                        .child(info_line(format!(
                            "Rows follow the font they are drawn in: {} pixels in the \
                             lists, {} in the code panes.",
                            points_text(list_row_height()),
                            points_text(code_row_height())
                        ))),
                ),
            )
            .into_element()
    }
}

// ---------------------------------------------------------------------------
// Scratchpad
// ---------------------------------------------------------------------------

/// The scratchpad the app has open, and what its worker is doing about it.
///
/// A root context and not state inside the view, for the reason [`Prefs`] and [`Proj`]
/// are: the Scratchpad pane is a dockable tab, and a dock tab that is not the active one
/// in its panel is *unmounted*. A buffer the reader is typing into cannot live somewhere
/// that a click on the tab beside it throws away.
#[derive(Clone, Copy)]
struct Pad(State<PadState>);

/// The scratchpad's source, as `freya-code-editor` holds it: a rope, a cursor, an undo
/// history and the tree-sitter blocks the rows are drawn from.
///
/// Beside [`Pad`] rather than inside it, and it is the editor's copy that is the live
/// one: `Scratchpad::source` is a `String` the model writes out, while this is what the
/// keyboard edits, so one of the two has to follow the other and it is the model that
/// follows. `use_scratchpad_with`'s first effect is the whole of that mirroring.
///
/// Also a root context, for [`Pad`]'s reason and one more: the theme effect below has to
/// reach it whether or not the pane is on screen, since a `SyntaxBlocks` holds resolved
/// colours and nothing a re-render does would repaint them (see [`HIGHLIGHTED`]).
#[derive(Clone, Copy)]
struct PadText(State<CodeEditorData>);

/// The way to ask the scratchpad's worker for something, shared through context so that a
/// button in the pane can ask without the pane owning the thread.
///
/// Two senders and not one, because they carry traffic of two different shapes. `jobs` is
/// what the reader asked for, one message per press. `events` is what a *running program*
/// is saying, which is as many messages a second as it cares to write -- so it is
/// [`RUN_EVENTS`]-bounded where the other is unbounded, and that bound is the app's half
/// of the backpressure `scratchpad.rs` documents: a full channel blocks the thread reading
/// the pipe, which fills the pipe, which blocks the program.
#[derive(Clone)]
struct PadJobs {
    jobs: async_channel::Sender<PadJob>,
    events: async_channel::Sender<(u64, RunEvent)>,
}

/// How many of a running program's lines may sit between the pipe and the pane.
///
/// Big enough that an ordinary burst is never throttled and small enough that the queue is
/// not somewhere output can pile up unnoticed. It is a *bound* and not a buffer size: the
/// point is that there is a number here at all.
const RUN_EVENTS: usize = 512;

/// Everything the Scratchpad pane draws.
#[derive(Clone, Default)]
struct PadState {
    scratchpad: Scratchpad,
    /// Whether the worker has yet said what is on disk.
    ///
    /// `Saves::written`'s rule, in a second place and for the same reason: the app boots
    /// holding [`Scratchpad::default`] and the reader's own source arrives a thread
    /// later, so a save that ran before that answer landed would write the default source
    /// over a good scratchpad. Nothing is saved until this is true.
    opened: bool,
    /// Whether a build is running. It is what disables the Build button, which is the
    /// whole of "two builds cannot be started at once": one worker thread runs the jobs
    /// in order anyway, but a second job queued behind the first would build bytes the
    /// reader has since changed and answer for them afterwards.
    building: bool,
    /// What the last build of this run came back with, or `None` before there has been
    /// one. A build is not remembered across runs: it describes bytes on disk that the
    /// next `cargo build` will replace.
    built: Option<Build>,
    /// Why the package on disk is not what is on screen, or `None` when it is.
    ///
    /// [`Scratchpad::write`] refuses outright rather than generating a manifest that
    /// differs from the rows -- which is the model's rule and a good one -- so a bad row
    /// stops the *source* being written too, and the pane has to say so where the reader
    /// is looking. It is one sentence over the rows, which each say their own half.
    unsaved: Option<Failure>,
    /// Which run the arriving output belongs to, counted up by [`request_run`].
    ///
    /// **A number, where `use_analysis` was at pains not to have one** -- and the
    /// difference is worth stating, since the rule there is that superseding is a
    /// comparison and never a counter. It could compare because an answer carries the
    /// `Symbol` it is about and that symbol existed *before* the request. Here the thing an
    /// event is about is the process, and the process does not exist until the worker has
    /// forked -- by which time the first lines can already be on their way. There is
    /// nothing yet to compare against, so the run is numbered instead. It matters for a
    /// gesture that is one keypress long: stopping a program and starting another leaves
    /// the first one's last lines and its `Ended` still in flight, and untagged they would
    /// land in the new run's output and mark it finished.
    run: u64,
    run_state: RunState,
    /// What the running program has written. Behind an `Arc` because this struct is cloned
    /// on every render and on every answer the worker sends, and the deque under it holds
    /// thousands of lines: the clone is a refcount bump, and appending is one
    /// `Arc::make_mut` per *batch* of arrivals rather than one per line.
    output: Arc<RunOutput>,
}

/// Where the program the reader started has got to.
///
/// Four states and not a `bool`, because three of them draw differently and the fourth --
/// [`RunState::Starting`] -- is the one a `bool` would get wrong: a fork is fast but it is
/// not instant, and a Stop pressed in that window has to be remembered rather than
/// dropped. `Idle` is not "not running", it is *nothing has been run*, which is why the
/// output pane is absent rather than empty before the first press.
#[derive(Clone, Default)]
enum RunState {
    #[default]
    Idle,
    /// Asked for; the worker has not come back with a handle yet.
    Starting,
    Going(Running),
    Over(Ended),
}

impl PadState {
    /// What the compiler said about the last build. Warnings on a build that succeeded
    /// and errors on one that did not are the same list to a reader.
    fn diagnostics(&self) -> &[Diagnostic] {
        match &self.built {
            Some(Build::Built { diagnostics, .. }) => diagnostics,
            Some(Build::Rejected { diagnostics, .. }) => diagnostics,
            Some(Build::Unavailable(_)) | None => &[],
        }
    }

    /// cargo's own words, when they are about the dependency rows.
    ///
    /// **This is the whole of how a failed build points back at a row**, and it is a
    /// structural test rather than a search for a crate name in a sentence. A rejected
    /// build with no compiler diagnostics at all is cargo refusing *before* it compiled
    /// anything, and the only part of the generated package a reader can get wrong from
    /// this pane is `[dependencies]` -- so `no matching package named ... found`, which
    /// `analysis`' own note says is stated on stderr and nowhere else, is drawn under the
    /// rows it is about instead of in the diagnostics list. Once the compiler has spoken
    /// the same stderr is only `could not compile ... due to 1 previous error`, which
    /// says nothing the list below does not, so it is dropped.
    fn refusal(&self) -> Option<&str> {
        match &self.built {
            Some(Build::Rejected {
                diagnostics,
                message,
            }) if diagnostics.is_empty() && !message.is_empty() => Some(message),
            _ => None,
        }
    }

    /// The one line over the pane saying where the last build got to, and whether that
    /// line is bad news.
    fn status(&self) -> Option<(String, bool)> {
        if self.building {
            return Some(("Building...".to_owned(), false));
        }

        let count = |level: Level, one: &str, many: &str| {
            let count = self
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.level == level)
                .count();
            match count {
                0 => String::new(),
                1 => format!(": 1 {one}"),
                count => format!(": {count} {many}"),
            }
        };

        match self.built.as_ref()? {
            Build::Built { .. } => Some((
                format!("Built{}", count(Level::Warning, "warning", "warnings")),
                false,
            )),
            Build::Rejected { .. } => Some((
                format!("Not built{}", count(Level::Error, "error", "errors")),
                true,
            )),
            // Nothing was compiled, and the reason is a sentence written to be shown as
            // it stands -- a bad row, no cargo on the `PATH`, nowhere to keep a
            // scratchpad.
            Build::Unavailable(failure) => Some((failure.to_string(), true)),
        }
    }

    /// What the last build made, and so what there is to run.
    ///
    /// The path cargo *named*, carried through from the build rather than derived here --
    /// which is the same argument `scratchpad.rs` makes for asking cargo in the first
    /// place, and the reason the Run button is unavailable until something has been built:
    /// what runs is then, by construction, what the diagnostics on screen are about.
    fn executable(&self) -> Option<&Path> {
        match &self.built {
            Some(Build::Built { executable, .. }) => Some(executable),
            _ => None,
        }
    }

    /// Whether a program is on its way up or already going.
    fn is_running(&self) -> bool {
        matches!(self.run_state, RunState::Starting | RunState::Going(_))
    }

    /// The line over the output, saying where the run got to, and whether that is bad
    /// news. `None` before anything has been run, which is what leaves the pane out.
    fn run_status(&self) -> Option<(String, bool)> {
        let dropped = match self.output.dropped() {
            0 => String::new(),
            1 => " (1 earlier line dropped)".to_owned(),
            count => format!(" ({count} earlier lines dropped)"),
        };

        let (text, bad) = match &self.run_state {
            RunState::Idle => return None,
            RunState::Starting => ("Starting...".to_owned(), false),
            RunState::Going(_) => ("Running".to_owned(), false),
            RunState::Over(Ended::Exited(Some(0))) => ("Exited".to_owned(), false),
            RunState::Over(Ended::Exited(Some(code))) => (format!("Exited with {code}"), true),
            // A signal on Unix. Spelt as what is *known* rather than as a guess at which,
            // since the number is not portable and the app has no use for it.
            RunState::Over(Ended::Exited(None)) => ("Ended with no exit code".to_owned(), true),
            RunState::Over(Ended::Stopped) => ("Stopped".to_owned(), false),
            RunState::Over(Ended::Failed(error)) => (format!("Could not run it: {error}"), true),
        };

        Some((format!("{text}{dropped}"), bad))
    }
}

/// What the scratchpad's worker thread is asked for. Each carries the whole scratchpad
/// rather than a handle to one, so nothing the worker touches can change under it while
/// it is writing or building.
enum PadJob {
    Open(Scratchpad),
    Save(Scratchpad),
    Build(Scratchpad),
    /// Start what the last build made. The odd one out: it is not blocking work, and what
    /// it hands back is a handle rather than an answer. It goes to the worker all the same
    /// because it *forks*, and the thread that draws has no business doing that -- and
    /// because the scratchpad's directory, which becomes the program's working directory,
    /// is this thread's to hand out.
    Run {
        /// Which run this is, carried so that a handle arriving after the reader has moved
        /// on can be recognised and stopped rather than stored. See [`PadState::run`].
        run: u64,
        scratchpad: Scratchpad,
        executable: PathBuf,
        /// Where each line goes as it is written. A boxed callback rather than a channel,
        /// so `scratchpad.rs` never learns what the app carries its values in.
        emit: Box<dyn FnMut(RunEvent) + Send>,
    },
}

/// What it answers with.
enum PadAnswer {
    Opened(Scratchpad),
    /// Why the package could not be written, or `None` when it was.
    Saved(Option<Failure>),
    Built(Build),
    /// The handle to a started program, or why there is none. Everything the program then
    /// *says* arrives on the other channel, not here: this is the answer to "did it
    /// start", and the run itself has no answer, only an end.
    Started(u64, Result<Running, Failure>),
}

/// The work itself: the three blocking calls `scratchpad.rs` documents as never belonging
/// on a UI thread, and nothing else. Split out so [`use_scratchpad_with`] can be handed
/// something that answers without a disk or a compiler -- `use_analysis_with`'s shape and
/// for its reason.
fn pad_work(job: PadJob) -> PadAnswer {
    match job {
        PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad.opened()),
        PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.write().err()),
        PadJob::Build(scratchpad) => PadAnswer::Built(scratchpad.build()),
        PadJob::Run {
            run,
            scratchpad,
            executable,
            emit,
        } => PadAnswer::Started(run, scratchpad.run(&executable, emit)),
    }
}

/// The scratchpad's whole wiring: one worker thread, the editor's text mirrored into the
/// model, the model written out as it changes, and the theme carried into the editor's
/// own syntax blocks.
///
/// **One worker thread, and it is the only thing that ever touches the scratchpad's
/// directory.** Reading it back, writing the package and running `cargo build` are all
/// documented in `scratchpad.rs` as blocking, and a `cargo build` is seconds; putting
/// them on one thread rather than one each is not only about the UI thread staying free
/// but about the directory having a single writer, so a save cannot land in the middle of
/// the build that is reading what it writes.
///
/// **Saves supersede, builds never do.** A keystroke is a save and a reader types
/// faster than a package is written, so the loop drains its queue while what it is
/// holding is a save: only the newest says anything, and a build that has arrived behind
/// one writes the package itself on its way past. A build is what the reader *asked* for
/// and its answer is the point, so it is never dropped.
///
/// **A run does not sit on that thread**, which is the one thing 10d added to the shape.
/// `PadJob::Run` only starts the program and comes straight back; the program itself lives
/// on two threads of `scratchpad.rs`'s and reports on a second channel. It has to be that
/// way round for a reason the other three jobs do not have: a run has no bound on how long
/// it takes -- an accidental `loop {}` is the ordinary case in a buffer somebody is
/// experimenting in -- and a run queued like a build would freeze every save behind it, so
/// the reader could not even edit their way out of it. **Stopping does not go through the
/// worker either**, for the same reason turned around: a stop queued behind a build would
/// arrive after the thing it was meant to interrupt.
fn use_scratchpad(pad: State<PadState>, text: State<CodeEditorData>, states: ProjectStates) {
    use_scratchpad_with(pad, text, states, pad_work);
}

/// [`use_scratchpad`] with the work handed in, so a test can drive the wiring without
/// writing to the machine's own state directory or waiting on a compiler.
fn use_scratchpad_with(
    mut pad: State<PadState>,
    mut text: State<CodeEditorData>,
    states: ProjectStates,
    work: impl Fn(PadJob) -> PadAnswer + Send + 'static,
) -> PadJobs {
    // What the worker was last handed, which is what the disk therefore says. The
    // baseline `Saves::written` is, and it starts empty for the reason that one does: the
    // app boots holding [`Scratchpad::default`], and a baseline seeded from it would make
    // the reader's own scratchpad -- which arrives a thread later -- look like a change to
    // be written back. It is *seeded by the answer* instead, so a run in which nothing is
    // typed writes nothing at all and a scratchpad nobody has opened leaves no directory
    // behind, which is `project.rs`'s rule about a file made by the first write that has
    // something to say.
    //
    // An `Rc<RefCell>` rather than a `State`, since nothing renders from it.
    let sent = use_hook(|| Rc::new(RefCell::new(None::<Scratchpad>)));

    let requests = use_hook({
        let sent = sent.clone();
        move || {
            let (requests, jobs) = async_channel::unbounded::<PadJob>();
            let (answered, answers) = async_channel::unbounded::<PadAnswer>();
            // One channel for the app's lifetime rather than one per run, which is what
            // makes the run number on each event necessary and is also what makes it
            // enough: a stopped run's last lines have somewhere to go, and are recognised
            // and dropped when they get there.
            let (emitted, events) = async_channel::bounded::<(u64, RunEvent)>(RUN_EVENTS);

            std::thread::spawn(move || {
                while let Ok(job) = jobs.recv_blocking() {
                    let mut job = job;
                    // Superseded saves, dropped before they are started. Whatever is behind
                    // one is either a newer save or a build, and a build writes the package
                    // itself -- so nothing is lost by not writing this one.
                    while matches!(job, PadJob::Save(_)) {
                        match jobs.try_recv() {
                            Ok(newer) => job = newer,
                            Err(_) => break,
                        }
                    }

                    // A send that fails is the app shutting down and taking the receiver
                    // with it.
                    if answered.send_blocking(work(job)).is_err() {
                        return;
                    }
                }
            });

            spawn(async move {
                while let Ok(answer) = answers.recv().await {
                    match answer {
                        PadAnswer::Opened(scratchpad) => {
                            // The buffer is replaced rather than edited into place: this is
                            // the first thing that happens to it, so there is no cursor and
                            // no undo history to preserve, and `CodeEditorData` has no way to
                            // set its text that would keep either honest anyway.
                            //
                            // `palette()` is asked here on the UI thread -- freya's `spawn`
                            // runs its tasks there -- so this is the same thread-local every
                            // component reads, and reading it outside a reactive scope simply
                            // subscribes nothing.
                            let mut editor = CodeEditorData::new(
                                Rope::from_str(&scratchpad.source),
                                language(Path::new(SOURCE_FILE)),
                            );
                            editor.set_theme(palette().syntax());
                            // Without this the editor has no blocks at all and draws no
                            // lines: `CodeEditorData::new` configures the highlighter and
                            // never runs it.
                            editor.parse();
                            text.set(editor);

                            // The baseline, seeded by the answer rather than at mount: what
                            // is on disk is by definition what was last written, so a run in
                            // which nothing is typed asks for no save at all.
                            *sent.borrow_mut() = Some(scratchpad.clone());

                            let mut next = pad.peek().clone();
                            next.scratchpad = scratchpad;
                            next.opened = true;
                            pad.set(next);
                        }
                        PadAnswer::Saved(failure) => {
                            let mut next = pad.peek().clone();
                            next.unsaved = failure;
                            pad.set(next);
                        }
                        PadAnswer::Built(build) => {
                            let executable = match &build {
                                Build::Built { executable, .. } => Some(executable.clone()),
                                _ => None,
                            };

                            let mut next = pad.peek().clone();
                            next.building = false;
                            next.built = Some(build);
                            // A build writes the package on its way, so the reason the last
                            // save could not is answered by it too.
                            if !matches!(
                                next.built,
                                Some(Build::Unavailable(Failure::Dependencies(_)))
                            ) {
                                next.unsaved = None;
                            }
                            pad.set(next);

                            if let Some(executable) = executable {
                                reopen_binary(states, executable);
                            }
                        }
                        PadAnswer::Started(run, started) => {
                            let mut next = pad.peek().clone();
                            // A handle for a run the reader has already left -- they
                            // pressed Stop or Run again inside the fork. It is stopped
                            // here and nowhere else, because this is the first moment
                            // anything in the app is holding it: dropping it instead would
                            // leave a process running that nothing could ever name again.
                            let mine =
                                next.run == run && matches!(next.run_state, RunState::Starting);
                            match started {
                                Ok(running) if mine => next.run_state = RunState::Going(running),
                                Ok(running) => running.stop(),
                                Err(failure) if mine => {
                                    next.run_state =
                                        RunState::Over(Ended::Failed(failure.to_string()))
                                }
                                Err(_) => {}
                            }
                            pad.set(next);
                        }
                    }
                }
            });

            // What a running program is saying. A task of its own beside the answers,
            // since the two channels are answering different questions and a program that
            // never ends would otherwise be sharing a loop with every save.
            spawn(async move {
                while let Ok(first) = events.recv().await {
                    // Everything else already queued, taken in one go. A program printing
                    // in a tight loop would otherwise wake this task per line, and each
                    // wake is a state write and so a render: coalescing makes the cost one
                    // render per batch, which is the same "drain the queue" the analysis
                    // worker does for the same reason.
                    let mut batch = vec![first];
                    while let Ok(more) = events.try_recv() {
                        batch.push(more);
                    }

                    let mut next = pad.peek().clone();
                    let mut changed = false;
                    for (run, event) in batch {
                        // A run the reader has left. Its lines are not this run's output
                        // and its ending is not this run's ending.
                        if run != next.run {
                            continue;
                        }
                        changed = true;
                        match event {
                            RunEvent::Wrote(line) => Arc::make_mut(&mut next.output).push(line),
                            RunEvent::Ended(ended) => next.run_state = RunState::Over(ended),
                        }
                    }

                    if changed {
                        pad.set(next);
                    }
                }
            });

            PadJobs {
                jobs: requests,
                events: emitted,
            }
        }
    });

    // How the pane asks for a build. A context rather than an argument, because the
    // button that asks is inside a dockable view that is handed nothing, and returned as
    // well so that a test can ask without going through a button.
    let jobs = use_provide_context(|| requests.clone());

    // What is on disk, asked for once. `use_hook` runs on mount and never again, which is
    // what makes this the app's one reading of the scratchpad.
    use_hook({
        let requests = requests.clone();
        move || {
            let _ = requests
                .jobs
                .try_send(PadJob::Open(pad.peek().scratchpad.clone()));
        }
    });

    // The editor's text into the model. Reading the editor subscribes this to every edit;
    // a cursor move wakes it too, and the comparison is what makes that free.
    use_side_effect(move || {
        let typed = text.read().rope.to_string();

        let changed = pad.peek().scratchpad.source != typed;
        if changed {
            pad.write().scratchpad.source = typed;
        }
    });

    // The model onto the disk. Nothing is written while the two are the same, and the
    // baseline moves to what was last *sent*: a reader who changes a row and changes it
    // back has to write again, or the file would be left holding the middle answer.
    use_side_effect(move || {
        let state = pad.read().clone();
        if !state.opened {
            return;
        }

        let mut sent = sent.borrow_mut();
        if sent.as_ref() != Some(&state.scratchpad) {
            *sent = Some(state.scratchpad.clone());
            let _ = requests.jobs.try_send(PadJob::Save(state.scratchpad));
        }
    });

    // The theme, carried into the editor's own blocks. This is `HIGHLIGHTED`'s hazard in
    // a second place: a `SyntaxBlocks` holds colours already resolved out of the palette,
    // so the entries are not stale after a switch, they are the wrong theme -- and
    // `set_appearance`'s clear cannot reach inside a `CodeEditorData`. Re-setting the
    // theme rebuilds the highlighter's capture colours and `parse` re-colours every line.
    //
    // Reading the appearance here subscribes the root, which already reads it twice.
    use_side_effect_with_deps(&appearance(), move |_: &Appearance| {
        let mut editor = text.write();
        editor.set_theme(palette().syntax());
        editor.parse();
    });

    jobs
}

/// Ask for a build of what is on screen, unless one is already running.
///
/// The guard is here as well as on the button, so that "two builds cannot be started at
/// once" is a property of the request rather than of one control's disabled state.
fn request_build(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    if state.building {
        return;
    }

    // **A rebuild stops what the last one started.** Three reasons and each is sufficient:
    // cargo is about to write over the very file this process is running, which on some
    // systems is refused outright and on the rest silently makes the running program a
    // different program from the one on screen; `reopen_binary` is about to close the
    // objects that describe those bytes, so the listing the reader would go back to is
    // gone; and there is one Run button for one scratchpad, so a build that left a program
    // going would leave the reader with an output pane belonging to a build they can no
    // longer see. Editing stops nothing, deliberately -- a run is of an executable and not
    // of the buffer, and a keystroke that killed the reader's program would make it
    // impossible to take a note about what it printed.
    stop_run(pad);

    pad.write().building = true;
    let _ = jobs.jobs.try_send(PadJob::Build(state.scratchpad));
}

/// Run what the last build made.
///
/// Nothing happens without an executable, which is why the button is unavailable until a
/// build has succeeded: the alternative -- Run building first -- makes one press mean two
/// things, and puts a page of diagnostics on screen in answer to a request to run.
///
/// Whatever was running is stopped first. One scratchpad, one program: two generations of
/// output arriving into one list is a pane with no answer to "what is this", and the
/// second run's own first line would sit under the first run's last.
fn request_run(mut pad: State<PadState>, jobs: &PadJobs) {
    let state = pad.peek().clone();
    let Some(executable) = state.executable().map(Path::to_path_buf) else {
        return;
    };

    stop_run(pad);

    // The output starts empty and the run is numbered: everything still on its way from
    // the run before this one is now for a number nobody is listening to.
    let run = state.run + 1;
    let mut next = pad.peek().clone();
    next.run = run;
    next.run_state = RunState::Starting;
    next.output = Arc::new(RunOutput::default());
    pad.set(next);

    let events = jobs.events.clone();
    let _ = jobs.jobs.try_send(PadJob::Run {
        run,
        scratchpad: state.scratchpad,
        executable,
        // `send_blocking` and not `try_send`: a full channel has to *stop* the thread
        // reading the pipe, which is what puts the brakes on the program itself. Dropping
        // the line instead would be an output with silent holes in it.
        emit: Box::new(move |event| {
            let _ = events.send_blocking((run, event));
        }),
    });
}

/// Stop the program, for real.
///
/// The `Going` case is the whole of it: `Running::stop` kills the process, and the state
/// is *not* set to `Over` here -- the run's own `Ended` event is what says it, and it is
/// emitted only once the process has been reaped. So the pane says "Stopped" when the
/// program is actually gone rather than when the button was pressed.
///
/// `Starting` is the case a `bool` would have lost: the fork has been asked for and has
/// not come back, so there is nothing to kill yet. Leaving `Starting` behind is what makes
/// the handle unwanted when it arrives, and the answer handler stops it there.
fn stop_run(mut pad: State<PadState>) {
    let state = pad.peek().clone();
    match &state.run_state {
        RunState::Going(running) => running.stop(),
        RunState::Starting => {
            let mut next = state;
            next.run_state = RunState::Over(Ended::Stopped);
            pad.set(next);
        }
        RunState::Idle | RunState::Over(_) => {}
    }
}

/// Open the binary at `path` in place of whatever the app already had from it.
///
/// **Not a sixth function holding the tab invariants**: it is [`close_binary`] followed by
/// exactly what the toolbar's Open button does, in that order and in one handler.
///
/// Replacing rather than accumulating is the only answer available, and the reason is the
/// app's own identity rule: a binary is a **path**, `close_binary` closes by path, and
/// `project::binaries` derives the saved list from the objects by path -- so two
/// generations of one file cannot both be in the objects list without every one of those
/// answering for which is which. A rebuild writes the same path with different bytes, so
/// what was open is a listing of instructions that no longer exist.
///
/// What it costs the reader, honestly: `close_binary` takes the chips for that file's
/// functions, their viewing positions and the history entries into them, so a rebuild
/// leaves the content strip empty of the scratchpad and the reader clicks their function
/// again. Keeping them would mean re-resolving each tab by name against the new objects,
/// which is exactly what a session restore does for a rebuilt binary (`project.rs`'s
/// `Rebuilt`) and is that machinery pointed at a live state rather than at a file.
///
/// The close happens **first** and the parse after it, which is the one thing streaming
/// turned around: objects arrive one at a time, so there is no moment at which the whole
/// answer is in hand to be swapped in under a single handler. What that costs is a beat in
/// which the project has let go of the file -- `record` writes `project.toml` without it
/// and again with it once the first object lands -- and what it buys is that the two
/// generations of one path can never be in the objects list together, which is the rule
/// everything else here rests on. The row does not blink either way: `close_binary` takes
/// the objects and `open_binaries` puts the file straight back as one being read.
fn reopen_binary(states: ProjectStates, path: PathBuf) {
    // Unconditionally, and before the new objects go in: whether or not the new build
    // parses, the objects the app is holding describe bytes that are no longer there.
    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.active,
        states.asm_at,
        states.src_at,
        states.history,
        &path,
    );

    spawn(async move {
        open_binaries(states.objects, states.loading, vec![path]).await;
    });
}

/// The file a scratchpad's source is, as cargo and rustc spell it: what `language` is
/// asked about, and what a diagnostic's span names when it is about the reader's own
/// source rather than a crate they depend on.
const SOURCE_FILE: &str = "src/main.rs";

/// How much of a dependency row the crate name takes against the version beside it. A
/// name is a word and a requirement is a handful of characters, and both boxes have to
/// shrink together in a 300px sidebar.
const NAME_FLEX: f32 = 2.0;
const VERSION_FLEX: f32 = 1.0;

/// What the compiler's own word for a level is drawn in.
///
/// The palette has one red and one warm hue, and this is what they are for here: an error
/// is the red every invalid thing in the app is written in, a warning is the terracotta a
/// string literal is (the one warm colour in the set, and the only other thing that is
/// meant to catch the eye), and a note recedes into the colour everything secondary is
/// written in.
fn level_color(level: Level) -> Color {
    match level {
        Level::Error => palette().invalid_fg,
        Level::Warning => palette().string_fg,
        Level::Note => palette().address_fg,
    }
}

fn level_text(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Note => "note",
    }
}

/// A block of a tool's own output, laid out the way it wrote it: one label per line, in
/// the fixed-width font, so rustc's carets sit under what they point at.
///
/// One label per line rather than one holding the newlines, for the reason every list in
/// this app builds rows: a paragraph that wraps would put a caret under the wrong
/// character, and the whole point of a rendered diagnostic is the column it points at.
fn text_block(text: &str, color: Color) -> Element {
    rect()
        .width(Size::fill())
        .overflow(Overflow::Clip)
        .children(
            text.lines()
                .map(|line| {
                    label()
                        .text(line.to_owned())
                        .assembly_font()
                        .color(color)
                        .max_lines(1)
                        .into()
                })
                .collect::<Vec<Element>>(),
        )
        .into_element()
}

/// One thing the compiler said: a line that can be scanned, and cargo's own rendering of
/// it under that.
///
/// The header repeats the message the block below it opens with, which is deliberate and
/// is what every problems list does: the header is what a reader runs their eye down and
/// the block is what they stop to read. What the header adds is the **place**, taken from
/// the span rather than from the text -- `src/main.rs:3:5` for the reader's own source and
/// the file's name alone for a diagnostic out of a crate they depend on, which is a
/// distinction only the span can make.
fn diagnostic_block(diagnostic: &Diagnostic) -> Element {
    let place = diagnostic.span.as_ref().map(|span| {
        let file = match span.file == SOURCE_FILE {
            true => span.file.clone(),
            // A registry path is most of a line on its own, and which crate it is in is
            // the useful half of it.
            false => file_name(&span.file),
        };

        format!("{file}:{}:{}", span.line, span.column)
    });

    rect()
        .width(Size::fill())
        .padding(Gaps::new(2.0, 0.0, 6.0, 0.0))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .content(Content::Flex)
                .child(
                    label()
                        .text(level_text(diagnostic.level))
                        .color(level_color(diagnostic.level))
                        .max_lines(1),
                )
                .maybe_child(place.map(|place| {
                    label()
                        .text(place)
                        .color(palette().address_fg)
                        .max_lines(1)
                        .into_element()
                }))
                .child(
                    label()
                        .text(diagnostic.message.clone())
                        .width(Size::flex(1.0))
                        .max_lines(1),
                ),
        )
        .child(text_block(&diagnostic.rendered, palette().text_fg))
        .into_element()
}

/// One `[dependencies]` row: the crate, the version required of it, and the × that drops
/// it.
///
/// The problem is a prop rather than something worked out here, because it is a property
/// of the *list* -- `Problem::Repeated` is about two rows -- and `Scratchpad::problems`
/// answers for all of them at once so that every bad row can be marked rather than the
/// first one.
#[derive(Clone, PartialEq)]
struct DependencyRow {
    index: usize,
    dependency: Dependency,
    problem: Option<Problem>,
    key: DiffKey,
}

impl KeyExt for DependencyRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for DependencyRow {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let index = self.index;
        let problem = self.problem.clone();
        // Which box is wrong is the model's answer and not this pane's guess at one:
        // `Repeated` is about the name, and nothing in its wording says so.
        let half = problem.as_ref().map(Problem::half);

        // The two boxes write straight into the row they are drawn from, so a keystroke
        // is a state change the save effect sees like any other -- the project view's
        // name box, one level deeper. Indexing is safe because a row is mounted only for
        // an index the list has: the × below shortens the list, and the rows are rebuilt
        // from the shorter one before either box is read again.
        let name = pad.into_writable().map(
            move |pad: &PadState| &pad.scratchpad.dependencies[index].name,
            move |pad: &mut PadState| &mut pad.scratchpad.dependencies[index].name,
        );
        let version = pad.into_writable().map(
            move |pad: &PadState| &pad.scratchpad.dependencies[index].version,
            move |pad: &mut PadState| &mut pad.scratchpad.dependencies[index].version,
        );

        let marked = |input: Input, box_half: Half| {
            input.maybe(half == Some(box_half), |input: Input| {
                input
                    .color(palette().invalid_fg)
                    .focus_border_fill(palette().invalid_fg)
            })
        };

        rect()
            .width(Size::fill())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height() + 8.0))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .content(Content::Flex)
                    .spacing(6.0)
                    .child(marked(
                        Input::new(name)
                            .placeholder("crate")
                            .compact()
                            .width(Size::flex(NAME_FLEX)),
                        Half::Name,
                    ))
                    .child(marked(
                        Input::new(version)
                            .placeholder("version")
                            .compact()
                            .width(Size::flex(VERSION_FLEX)),
                        Half::Version,
                    ))
                    .child(
                        Button::new()
                            .compact()
                            .on_press(move |_| {
                                pad.write().scratchpad.dependencies.remove(index);
                            })
                            .child("\u{00d7}"),
                    ),
            )
            // Against the row it belongs to and never as one message at the top, which is
            // what `Scratchpad::problems` answering with every row's index is for.
            .maybe_child(problem.map(|problem| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 0.0, 4.0, 2.0))
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .text(problem.to_string())
                            .color(palette().invalid_fg)
                            .max_lines(1),
                    )
            }))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The scratchpad's source, in freya's own `CodeEditor`.
///
/// **5a rejected this component for the read-only source pane and 10c takes it, which is
/// not a reversal**: both of 5a's reasons were about painting and scrolling a listing
/// from outside, and neither survives the pane being one the reader is typing in.
/// `editor_line.rs` paints a line background for exactly one line, the cursor's own --
/// which is wrong for the source pane, where a set of lines maps to an instruction, and
/// exactly right here, where the only current line *is* the caret's. Its
/// `ScrollController` is built in a `use_hook` of its own and `CodeEditorData::scrolls` is
/// `pub(crate)` -- which stopped 5c from scrolling the pane to a line, and there is
/// nothing here that wants to. What is left is a real editor: a cursor, a selection, an
/// undo history, the clipboard, IME preedit, and an *incremental* tree-sitter re-parse per
/// keystroke through the same pipeline the source pane already borrows. Hand-rolling that
/// would be several hundred lines of text editing to end up with less.
///
/// Two things are still ours, and both are the reason the app looks like one app: the
/// colours come from the palette rather than from `EditorTheme::light()`, and the font is
/// the desktop's fixed-width one.
#[derive(PartialEq)]
struct SourceEditor;

impl Component for SourceEditor {
    fn render(&self) -> impl IntoElement {
        let text = use_consume::<PadText>().0;
        let a11y_id = use_hook(AccessibilityId::new_unique);

        let font = fonts();
        let size = font.mono.size();
        // The editor takes **one** family where everything else in the app takes a chain,
        // and freya appends the parent element's families behind an element's own -- so
        // the rest of the chain arrives by inheritance from the box around it, which is
        // what keeps a desktop naming a font that is not installed from silently landing
        // the listing in a proportional face.
        let family = font
            .mono
            .families
            .first()
            .map(|family| family.to_string())
            .unwrap_or_default();
        // The editor multiplies its font size by this and floors the answer, and what is
        // wanted is `code_row_height()` exactly -- so half a pixel of slack is what makes the
        // product land on it rather than one below it.
        let line_height = (code_row_height() + 0.5) / size;

        rect()
            .expanded()
            .background(palette().pane_bg)
            .assembly_font()
            .child(
                CodeEditor::new(text, a11y_id)
                    .font_size(size)
                    .font_family(family)
                    .line_height(line_height)
                    // The source pane draws indentation as plain spaces, and two panes of
                    // code in one window disagreeing about that would read as two editors.
                    .show_whitespace(false)
                    .background(palette().pane_bg)
                    .text(palette().name_fg)
                    .cursor(palette().text_fg)
                    // What would land on the clipboard, which is what `row_select_bg`
                    // already says in both code panes -- a character selection here where
                    // it is a run of rows there, and the same question either way.
                    .highlight(palette().row_select_bg)
                    // "You are here", which is `code_row_hover_bg`'s job in the other two
                    // panes. Reusing it rather than adding a ninth wash to the palette is
                    // safe because the editor paints no pointer hover at all, so the two
                    // meanings can never be on screen together in this pane.
                    .line_selected_background(palette().code_row_hover_bg)
                    .gutter_selected(palette().text_fg)
                    .gutter_unselected(palette().address_fg)
                    .whitespace(palette().punctuation_fg),
            )
    }
}

/// The lines a running program has written, as the row builder is handed them.
///
/// A wrapper for the identity: `PartialEq` here is `Arc::ptr_eq`, the app's rule
/// everywhere, and it is load-bearing rather than an optimisation -- deriving it would
/// compare thousands of strings on every render of a pane that is being appended to
/// several times a second.
#[derive(Clone)]
struct OutputRows(Arc<RunOutput>);

impl PartialEq for OutputRows {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One line, in the colour of the stream it came from.
///
/// **stdout and stderr are told apart by colour and by nothing else**, and the colour is
/// deliberately not the red every invalid thing in the app wears: a program writing to
/// stderr is not a program in error -- logs, progress and prompts all go there -- so it
/// takes `string_fg`, the palette's one warm hue and the colour a warning already is. Both
/// are palette fields, so both are answered in the dark theme by the same contrast floor
/// every other foreground is held to.
fn output_row(line: &crate::scratchpad::OutputLine) -> Element {
    let color = match line.stream {
        Stream::Out => palette().text_fg,
        Stream::Err => palette().string_fg,
    };

    rect()
        .width(Size::fill())
        .height(Size::px(code_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 12.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(line.text.to_string())
                .assembly_font()
                .color(color)
                .max_lines(1),
        )
        .into_element()
}

/// The Scratchpad pane: a source file the reader edits, the crates it asks for, a build,
/// and what the compiler said about it.
///
/// **A view and not a document**, which is the rule 8e settled and this inherits whole: a
/// chip in the content strip is a `Selection` -- a place in a binary -- and a scratchpad
/// is not one. What it *builds* is, and needs no rule at all: the executable goes through
/// `open_files` like any other binary and its functions are ordinary chips.
#[derive(PartialEq)]
struct ScratchpadTab;

impl Component for ScratchpadTab {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let state = pad.read().clone();

        // Every bad row at once, keyed by the row it belongs to. `Scratchpad::problems`
        // answers with all of them precisely so that a reader who has two rows wrong is
        // not shown them one at a time.
        let problems: HashMap<usize, Problem> = state.scratchpad.problems().into_iter().collect();
        let rows: Vec<Element> = state
            .scratchpad
            .dependencies
            .iter()
            .enumerate()
            .map(|(index, dependency)| {
                DependencyRow {
                    index,
                    dependency: dependency.clone(),
                    problem: problems.get(&index).cloned(),
                    key: DiffKey::None,
                }
                .key(index)
                .into()
            })
            .collect();

        let diagnostics: Vec<Element> = state.diagnostics().iter().map(diagnostic_block).collect();
        let refusal = state
            .refusal()
            .map(|message| text_block(message, palette().text_fg));
        let package = match state.scratchpad.directory() {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => "nowhere to keep a scratchpad".to_owned(),
        };

        // **One button, because there is one program.** While something is running the
        // only thing to want from it is to stop it, and a Run beside a Stop would be two
        // controls whose combined meaning has to be worked out. It is never both disabled
        // and hiding something: with nothing built there is nothing to run, and the status
        // line above says whether a build has happened.
        let running = state.is_running();
        let run_jobs = jobs.clone();
        let run = Button::new()
            .enabled(running || (state.executable().is_some() && !state.building))
            .on_press(move |_| match running {
                true => stop_run(pad),
                false => request_run(pad, &run_jobs),
            })
            .child(match running {
                true => "Stop",
                false => "Run",
            })
            .into_element();

        let output = state.run_status().map(|(text, bad)| {
            let lines = state.output.clone();
            let length = lines.len();

            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .background(palette().asm_pane_bg)
                .border(bottom_hairline())
                .child(
                    rect()
                        .width(Size::fill())
                        .height(Size::px(list_row_height()))
                        .horizontal()
                        .cross_align(Alignment::Center)
                        .padding(Gaps::new_symmetric(0.0, 12.0))
                        .spacing(8.0)
                        .content(Content::Flex)
                        .overflow(Overflow::Clip)
                        .child(label().text("Output").font_weight(FontWeight::BOLD))
                        .child(
                            label()
                                .text(text)
                                .width(Size::flex(1.0))
                                .color(match bad {
                                    true => palette().invalid_fg,
                                    false => palette().address_fg,
                                })
                                .max_lines(1),
                        ),
                )
                .child(
                    // The lines go through `new_with_data` and are not captured, which is
                    // the gotcha this list would otherwise walk straight into: the builder
                    // closure is never compared across renders, so a captured `Arc` would
                    // draw the first batch of output for ever.
                    VirtualScrollView::new_with_data(
                        OutputRows(lines),
                        |index, rows: &OutputRows| match rows.0.line(index) {
                            Some(line) => output_row(line),
                            // Only reachable if the list shortened between the length
                            // being read and the row being asked for, which the cap cannot
                            // do -- it drops from the front and keeps the count. An empty
                            // row rather than an index that panics all the same.
                            None => rect().height(Size::px(code_row_height())).into_element(),
                        },
                    )
                    .length(length)
                    .item_size(code_row_height()),
                )
                .into_element()
        });

        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new_symmetric(8.0, 12.0))
                    .spacing(6.0)
                    .child(section_heading(
                        "Scratchpad",
                        Some(
                            rect()
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .spacing(6.0)
                                .child(
                                    Button::new()
                                        // The whole of "two builds cannot be started at
                                        // once", on the control as well as in
                                        // `request_build`: a build takes seconds, and a
                                        // button that goes on looking pressable through
                                        // them is a button that gets pressed again.
                                        .enabled(!state.building)
                                        .on_press(move |_| request_build(pad, &jobs))
                                        .child(match state.building {
                                            true => "Building...",
                                            false => "Build",
                                        }),
                                )
                                .child(run)
                                .into_element(),
                        ),
                    ))
                    // The crate it generates, which is also what the executable it
                    // builds is called -- so the row that appears in the Objects list
                    // after a build is recognisable as this.
                    .child(field_row(
                        "Crate",
                        label()
                            .text(state.scratchpad.name().to_owned())
                            .width(Size::flex(1.0))
                            .max_lines(1),
                    ))
                    // Where it is on disk, which is the whole of what there is to know
                    // about a scratchpad the app did not have to invent a format for: the
                    // package cargo is handed *is* the storage. In a tooltip as well,
                    // because a state directory is longer than any pane this can be
                    // docked in -- which is what a tooltip is for everywhere else here.
                    .child(row_tooltip(
                        package.clone(),
                        field_row(
                            "Package",
                            label()
                                .text(package)
                                .width(Size::flex(1.0))
                                .color(palette().address_fg)
                                .max_lines(1),
                        ),
                    ))
                    .maybe_child(state.status().map(|(text, bad)| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(text)
                                    .color(match bad {
                                        true => palette().invalid_fg,
                                        false => palette().address_fg,
                                    })
                                    .max_lines(1),
                            )
                    }))
                    .child(section_heading(
                        "Dependencies",
                        Some(
                            Button::new()
                                .compact()
                                .on_press(move |_| {
                                    pad.write()
                                        .scratchpad
                                        .dependencies
                                        .push(Dependency::default());
                                })
                                .child("Add")
                                .into_element(),
                        ),
                    ))
                    .child(match rows.is_empty() {
                        true => info_line("No crates asked for".to_owned()).into_element(),
                        false => rect().width(Size::fill()).children(rows).into_element(),
                    })
                    // The package is what the reader is looking at, so the sentence saying
                    // it is not the package on disk goes with the rows that say why.
                    .maybe_child(state.unsaved.map(|failure| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(format!("Not saved: {failure}"))
                                    .color(palette().invalid_fg)
                                    .max_lines(1),
                            )
                    }))
                    // cargo's own words, when they are about these rows and are said
                    // nowhere else. See `PadState::refusal`.
                    .maybe_child(refusal),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(2.0))
                    .border(bottom_hairline())
                    .child(SourceEditor),
            )
            .maybe_child((!diagnostics.is_empty()).then(|| {
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .background(palette().asm_pane_bg)
                    .child(
                        ScrollView::new().child(
                            rect()
                                .width(Size::fill())
                                .padding(Gaps::new_symmetric(4.0, 12.0))
                                .children(diagnostics)
                                .into_element(),
                        ),
                    )
                    .into_element()
            }))
            // Under the diagnostics rather than over them: what the compiler said is about
            // the source directly above it, and what the program said is the newest thing
            // in the pane. Both are `flex(1)` against the editor's `flex(2)`, so a run in a
            // pane that is already showing warnings costs the editor a third of its height
            // and not all of it.
            .maybe_child(output)
    }
}

// ---------------------------------------------------------------------------
// Docking
// ---------------------------------------------------------------------------

/// Panel ids are only ever looked up inside the area that handed them out, so
/// each area numbers its own panels from zero.
type PanelId = u32;

/// One docking area: the tree of splits and tabbed panels filling one of the two
/// resizable panes. The nine tabs are shared between the two areas, so a drop
/// here has to take the tab out of `other` -- which is safe to write from
/// `on_drop` only because the two areas are separate `State`s, and freya's
/// docking holds a mutable borrow of just the one being dropped into.
///
/// Plain data apart from that handle, so the layout can be serialized later.
struct DockArea {
    tree: DockNode<Tab, PanelId>,
    next_panel_id: PanelId,
    other: Option<State<DockArea>>,
}

impl DockArea {
    /// An area split into one tabbed panel per group. Every split freya's docking
    /// builds gets an equal share, so the groups start at equal sizes and the
    /// handles between them are the only way to change that.
    fn split(direction: Direction, groups: Vec<Vec<Tab>>) -> Self {
        Self {
            next_panel_id: groups.len() as PanelId,
            tree: DockNode::Split {
                direction,
                children: groups
                    .into_iter()
                    .enumerate()
                    .map(|(panel_id, tabs)| {
                        DockNode::Panel(DockPanel::new(panel_id as PanelId, tabs))
                    })
                    .collect(),
            },
            other: None,
        }
    }

    /// The groups stacked top to bottom, which is what the sidebar looks like.
    fn column(groups: Vec<Vec<Tab>>) -> Self {
        Self::split(Direction::Vertical, groups)
    }

    /// The groups side by side, which is what the content area looks like.
    fn row(groups: Vec<Vec<Tab>>) -> Self {
        Self::split(Direction::Horizontal, groups)
    }

    fn take_panel_id(&mut self) -> PanelId {
        let panel_id = self.next_panel_id;
        self.next_panel_id += 1;
        panel_id
    }

    /// Whether `tab` is the one on top in whichever panel holds it.
    fn is_active(&self, tab: Tab) -> bool {
        let Some((panel_id, _)) = self.tree.find_tab(&tab) else {
            return false;
        };
        self.tree
            .panel(&panel_id)
            .and_then(|panel| panel.active_tab_id)
            == Some(tab)
    }

    /// Put `tab` into `panel_id` at `position`, or at the end when `None`, and
    /// take it out of every other panel of this area.
    fn place(&mut self, panel_id: PanelId, tab: Tab, position: Option<usize>) -> bool {
        let Some(panel) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        match position {
            Some(position) => panel.insert_tab(tab, position),
            None => panel.append_tab(tab),
        }
        self.tree.remove_tab_except(&tab, Some(&panel_id));
        true
    }

    /// Drop `tab`, which has just been dropped into the other area.
    fn evict(&mut self, tab: Tab) {
        if self.tree.remove_tab_except(&tab, None) {
            self.tidy();
        }
    }

    /// Fold away the panels a move emptied. An area that loses its last tab keeps
    /// one empty panel rather than going to `None`, so its pane stays on screen as
    /// a drop target and tabs can be dragged back into it.
    fn tidy(&mut self) {
        self.tree.close_empty_panels();
        if self.tree.is_empty() && !matches!(self.tree, DockNode::Panel(_)) {
            let panel_id = self.take_panel_id();
            self.tree = DockNode::Panel(DockPanel::new(panel_id, Vec::new()));
        }
    }
}

impl DockingModel for DockArea {
    type TabId = Tab;
    type PanelId = PanelId;
    type DropValue = Tab;

    fn root(&self) -> Option<&DockNode<Tab, PanelId>> {
        Some(&self.tree)
    }

    fn on_drop(&mut self, tab: Tab, target: DropTarget<PanelId>) -> bool {
        let dropped = match target {
            DropTarget::Tab { panel_id, position } => self.place(panel_id, tab, Some(position)),
            DropTarget::Center(panel_id) => self.place(panel_id, tab, None),
            DropTarget::Split { panel_id, side } => {
                let new_panel_id = self.next_panel_id;
                let new_panel = DockPanel::new(new_panel_id, vec![tab]);
                if self.tree.split_panel(&panel_id, side, &new_panel) {
                    self.next_panel_id += 1;
                    self.tree.remove_tab_except(&tab, Some(&new_panel_id));
                    true
                } else {
                    false
                }
            }
        };

        if dropped {
            self.tidy();
            // A drag carries only the tab, so the source area is not known -- but
            // there are only two, and dropping the tab where it already was is a
            // no-op for the other one.
            if let Some(mut other) = self.other {
                other.write().evict(tab);
            }
        }

        dropped
    }

    fn set_active(&mut self, panel_id: PanelId, tab: Tab) -> bool {
        let Some(panel) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        if !panel.tabs.contains(&tab) {
            return false;
        }
        panel.active_tab_id = Some(tab);
        true
    }
}

/// One tab header. The same shape the pane headers used to have, so a bar of them
/// reads like the old header strip.
fn tab_label(tab: Tab, background: Color) -> impl IntoElement {
    rect()
        .height(Size::px(list_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .spacing(6.0)
        .background(background)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(tab.icon())
        .child(label().text(tab.title()).max_lines(1))
}

fn tab_header(ctx: TabContext<Tab>, area: State<DockArea>) -> Element {
    let background = if ctx.is_drop_target {
        palette().selected_bg
    } else if area.read().is_active(ctx.tab_id) {
        palette().pane_bg
    } else {
        Color::TRANSPARENT
    };

    tab_label(ctx.tab_id, background).into_element()
}

/// The copy of the tab that follows the cursor while it is being dragged.
fn tab_drag(tab: Tab) -> Element {
    rect()
        .interactive(false)
        .border(right_hairline())
        .child(tab_label(tab, palette().selected_bg))
        .into_element()
}

fn tab_bar(ctx: TabBarContext<PanelId>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .horizontal()
        .background(palette().header_bg)
        .border(bottom_hairline())
        .children(ctx.tab_children)
        .into_element()
}

fn tab_content(tab: Option<Tab>) -> Element {
    match tab {
        Some(tab) => tab.view(),
        None => placeholder("Drag a tab here"),
    }
}

fn docking_area(area: State<DockArea>) -> impl IntoElement {
    DockingArea::new(
        area,
        |ctx: ContentContext<Tab, PanelId>| tab_content(ctx.tab_id),
        move |ctx: TabContext<Tab>| tab_header(ctx, area),
        tab_drag,
        tab_bar,
    )
    .preview_element(
        rect()
            .interactive(false)
            .expanded()
            .background(palette().drop_preview_bg),
    )
}

fn toolbar(objects: State<Vec<Arc<Object>>>, loading: State<Loads>) -> impl IntoElement {
    let on_open = move |_| {
        spawn(async move {
            let Some(handles) = AsyncFileDialog::new()
                .set_title("Open a binary file...")
                .pick_files()
                .await
            else {
                return;
            };

            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();

            // Off the UI thread, and one object at a time: the sidebar has a row per file
            // from here on and fills it in as the objects arrive. See `open_binaries`.
            open_binaries(objects, loading, paths).await;
        });
    };

    rect()
        .horizontal()
        .width(Size::fill())
        .border(bottom_hairline())
        .child(
            rect()
                .margin(4.0)
                .child(Button::new().on_press(on_open).child("Open")),
        )
}

/// Tell the save policy what the session looks like, whenever it changes.
///
/// `use_side_effect` re-runs its callback whenever a `State` that was `read()` inside
/// it changes (`freya-core/src/lifecycle/effect.rs`), so reading the state contexts
/// here makes this one observer the single choke point every mutation flows through:
/// `activate`, the toolbar's `objects.write()`, the history push inside `activate` and
/// the tab list know nothing about persistence, and neither will any future one. The subscriptions *are* the `read()` calls, which
/// is the whole of what makes adding a persisted field to `Session::from_state` also
/// add the state behind it to what wakes this.
///
/// Whether a change reaches the disk now or at the next `use_periodic_save` tick is
/// `project::record`'s decision, not this one's: opening a binary is written at once,
/// a document, a tab or a history entry is left pending. That policy is framework-free
/// and unit-tested in `project.rs`; all this hook owns is *when to look*.
///
/// One visit wakes this up to three times -- for `Active`, for the tab `activate` opened
/// and for the history entry it pushed -- which costs three derivations and three
/// comparisons and, since none of them is a binaries change, no write at all.
///
/// Scrolling a pane wakes it too, which is the one input here that a reader can produce
/// continuously. It costs no more than the three above, and it is bounded by the unit the
/// position is kept in: a viewing position is a *row*, so a scroll writes nothing until
/// the pane has moved a whole row, and `use_kept_position` compares before it
/// writes.
fn use_save_on_change(states: ProjectStates) {
    let ProjectStates {
        proj,
        objects,
        // What is still being read is deliberately not saved and deliberately does not
        // wake this: `binaries` is derived from the objects, so a file joins the saved
        // list when its first object lands and a file that never parses is never named,
        // which is exactly what it did before anything streamed.
        loading: _,
        open,
        asm_at,
        src_at,
        active,
        history,
    } = states;

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it. Each
        // guard lives to the end of the statement it is created in, which is the one
        // `record` call, and nothing here writes anything, so holding several at once is
        // the safe half of the `peek`/`write` gotcha rather than the dangerous one.
        let objects = objects.read();
        project::record(
            // The user-given half, which since 8e is a state like the rest rather than
            // something the save policy had to carry: the project view holds it, so it
            // arrives here the same way the binaries do and a rename is recorded by the
            // same observer that records everything else.
            proj.read().details(),
            project::binaries(&objects),
            Session::from_state(
                &objects,
                open.read().tabs(),
                &asm_at.read(),
                &src_at.read(),
                active.read().as_ref(),
                &history.read(),
            ),
        );
    });
}

/// Write out a pending change every `AUTOSAVE_INTERVAL`.
///
/// `use_hook` runs its initializer on mount and never again, so exactly one of these
/// loops exists; `spawn` is freya's own task spawner, and `async_io::Timer` is what
/// freya itself waits on inside spawned tasks (`freya-animation`'s hook and
/// `freya-sdk`'s timeout both do), so this adds no runtime -- async-io's reactor is
/// already in the process.
///
/// A tick that finds nothing pending does no IO at all, which is what makes the empty
/// baseline in `Saves` matter here: a tick during the startup parse, before anything
/// has been restored, has nothing to write and so cannot put an empty project over a
/// good file.
fn use_periodic_save() {
    use_hook(|| {
        spawn(async move {
            loop {
                Timer::after(project::AUTOSAVE_INTERVAL).await;
                project::flush();
            }
        });
    });
}

/// Forget the cross-view focus and the pin whenever the active document changes.
///
/// Both are positions inside the drawn symbol's line info, so they mean nothing once
/// that symbol is gone -- and the ordinary way the focus goes away, the pointer leaving the
/// row that set it, need never happen: clicking a relocation label navigates from an
/// assembly row the pointer is still sitting on, and the symbol it lands in was very often
/// compiled from the same file, so a line of that file would stay lit for a position in a
/// function no longer on screen until the pointer moved. A pin has no such ordinary way at
/// all -- staying is the whole of what makes it one -- so this is the only thing that ends
/// it short of another click.
///
/// Its own effect rather than a line inside the save observer: it has no business
/// subscribing to anything but the active document, and the two concerns stay separable.
fn use_clear_focus(
    active: State<Option<Document>>,
    focused: State<Option<LineFocus>>,
    pinned: State<Option<Pin>>,
) {
    use_side_effect(move || {
        // Reading subscribes the effect to the active document, which is the whole of
        // what it wants from it -- both are `None` again whatever the new one is.
        let _ = active.read();

        let (mut focused, mut pinned) = (focused, pinned);
        focused.set_if_modified(None);
        pinned.set_if_modified(None);
    });
}

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when the selection moves to another symbol, the source pane's when
/// another file is shown. Rows 40 to 60 of the function just left are not rows 40 to 60
/// of the one arrived at.
///
/// Here, at the root, and keyed on the two states that say *which listing* -- and
/// deliberately not on the listings themselves. The obvious version is a
/// `use_side_effect_with_deps` inside each list, and it is wrong twice over: `AsmData`
/// carries an `Arc<Lanes>` built fresh on every render (7b), so it compares unequal to
/// itself and the effect would fire on every render, wiping the run the press had just
/// started -- which is exactly what the headless check caught -- and a dep compared by
/// pointer can be fooled by a new allocation landing where the old one was.
///
/// Its own effect rather than a third line in `use_clear_focus`, because the two answer
/// to different things: a focus and a pin are positions in the selected symbol's line
/// info and go when *it* does, while the source pane's run is a range of lines in a file
/// that a change of symbol very often leaves open.
fn use_clear_marks(
    active: State<Option<Document>>,
    analysis: State<Analyzed>,
    marked: State<Option<Marks>>,
) {
    use_side_effect(move || {
        let _ = active.read();
        unmark(marked, Pane::Assembly);
    });
    // Which file the Source pane was drawing the last time this ran. An `Rc<RefCell>`
    // and not a `State` for `use_kept_position`'s reason: nothing renders from it, and a
    // state here would cost the root a second render every time the pane changed file.
    let showing = use_hook(|| Rc::new(RefCell::new(None::<Arc<str>>)));
    use_side_effect(move || {
        // The *file the Source pane is drawing*, which is what its rows index into, and
        // which is not the active document: an assembly-driven tab draws its companion,
        // so switching from one function to another compiled from the same file leaves
        // the same lines on screen and the run picked out in them still means something.
        // `source_side` is the one place either pane works that out, so this cannot
        // disagree with what is drawn.
        //
        // Compared against what it last was rather than answered to directly, because
        // reading the analysis subscribes this to all of it — a request going out and the
        // slow flag turning over are writes to it that change no listing, and dropping a
        // run of rows on one of those would take it away under the reader's hand.
        let file =
            source_side(active.read().as_ref(), &analysis.read()).map(|side| side.file().clone());
        // Cloned out of the borrow before the `borrow_mut`, which panics exactly the way
        // a `State` guard held across a write does.
        let was = showing.borrow().clone();
        if was == file {
            return;
        }
        *showing.borrow_mut() = file;

        unmark(marked, Pane::Source);
    });
}

/// Work the selected symbol out on a thread of its own, and hand the answer to the panes
/// through [`Analysis`].
///
/// **Where the work runs: one worker thread, for the app's lifetime.** Not a thread per
/// request and not a pool, because requests here *supersede* each other rather than
/// accumulating: a reader holding the down-arrow through a symbol list issues one per
/// row and wants exactly the last one's answer. A thread per request would put the whole
/// run of them through the most expensive call in the crate at once — the first
/// `line_info` against an object builds its entire DWARF context — with every answer but
/// one thrown away, and `DwarfCache` is a `OnceLock`, so the losers would not even be
/// racing usefully: they block on the winner. A pool has the same shape with a bound on
/// it. One worker instead, with the queue drained to its newest entry each time round, so
/// the requests the reader clicked past are dropped *before* they are started rather than
/// after. It also gives the answers an order — request order — which is what makes a stale
/// answer always an old one and never a new one.
///
/// This is deliberately not the multi-threading `notes/Goals.md` asks for under
/// "lightweight and multi threaded": that one is about parsing many objects at once, which
/// is [`open_binaries`]' worker and its own answer. This is one reader looking at one
/// function, where the useful number of threads is one and the point is only that it is
/// not the one drawing the window.
///
/// **How a superseded answer is dropped.** Every answer carries the [`Symbol`] it is
/// about, and it is kept only when that symbol is the one selected *now* — a comparison,
/// not a generation counter, because `Selection` compares by `Arc` pointer identity and so
/// already answers this exactly. A counter would be a second identity to keep in step with
/// the first, and would get the ordinary A → B → A case wrong: the answer for the first A
/// is a perfectly good answer for the third selection, and this shows it rather than
/// working it out again. A dropped answer is the normal case and not an error — it is what
/// clicking twice quickly *means* — so nothing logs, warns or retries.
///
/// **What the panes show meanwhile** is in [`Analyzed`]: the listing they already have,
/// until either the next one arrives or [`SLOW_ANALYSIS`] passes.
fn use_analysis(active: State<Option<Document>>, analysis: State<Analyzed>) {
    use_analysis_with(active, analysis, Studied::new);
}

/// The whole of [`use_analysis`], with the work itself as an argument so a test can hold
/// it still. Superseding is a race by construction — the answer that has to be dropped is
/// the one that arrives while the reader has already clicked on — and nothing can assert
/// it against a worker that answers as fast as it is asked.
fn use_analysis_with(
    active: State<Option<Document>>,
    mut analysis: State<Analyzed>,
    study: impl Fn(Symbol) -> Studied + Send + 'static,
) {
    // The worker and the task that listens to it, started once and never restarted. Both
    // channels are unbounded, which costs nothing here: the request side holds at most
    // what the reader has clicked since the worker last looked, and the answer side at
    // most one per request.
    let requests = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<Symbol>();
        let (answered, answers) = async_channel::unbounded::<Studied>();

        // A `std::thread` and not a spawned task, exactly as `open_files` is: this is
        // seconds of decoding and DWARF parsing, and freya's executor is the UI thread.
        std::thread::spawn(move || {
            while let Ok(symbol) = jobs.recv_blocking() {
                // Everything the reader clicked past while the last job ran, dropped
                // without being started. Only the newest is wanted, and finding that out
                // here rather than after the fact is the difference between a stale
                // answer costing a comparison and costing a second of decoding.
                let mut symbol = symbol;
                while let Ok(newer) = jobs.try_recv() {
                    symbol = newer;
                }

                // A send that fails is the app shutting down and taking the receiver
                // with it.
                if answered.send_blocking(study(symbol)).is_err() {
                    return;
                }
            }
        });

        spawn(async move {
            let mut analysis = analysis;
            while let Ok(studied) = answers.recv().await {
                // The superseding rule. Cloned out of the guard first, since everything
                // below it writes.
                let current = active.peek().clone();
                if !current
                    .as_ref()
                    .and_then(Document::symbol)
                    .is_some_and(|symbol| *symbol == studied.symbol)
                {
                    continue;
                }

                let mut next = analysis.peek().clone();
                if next.pending.as_ref() == Some(&studied.symbol) {
                    next.pending = None;
                    next.slow = false;
                }
                // Already on screen: the same symbol answered twice, which happens when
                // the reader clicks away and straight back before the worker has looked
                // at the queue. Keeping the listing that is up rather than replacing it
                // with an identical one saves re-rendering every row for nothing.
                if !next
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.symbol == studied.symbol)
                {
                    next.shown = Some(studied);
                }
                analysis.set_if_modified(next);
            }
        });

        requests
    });

    use_side_effect(move || {
        // Reading subscribes this to the active document, which is the only thing it
        // answers to; the state it writes is `peek`ed, so it cannot wake itself.
        let current = active.read().clone();

        let Some(symbol) = current.as_ref().and_then(Document::symbol).cloned() else {
            // Not a function: an object, a source file, or nothing open at all. There
            // is nothing to work out and so nothing to wait for, and the panes are told
            // at once — clearing is instant even though replacing is not. Anything still
            // in flight is for a place the reader has left and is dropped when it lands.
            analysis.set_if_modified(Analyzed::default());
            return;
        };

        let state = analysis.peek().clone();

        if state
            .shown
            .as_ref()
            .is_some_and(|shown| shown.symbol == symbol)
        {
            // Already drawn. Nothing to ask for — and nothing left to wait for either:
            // whatever the worker is still chewing on is for somewhere the reader has
            // since come back from, so the pane must not go on to say it is waiting for
            // it.
            if state.pending.is_some() {
                let mut next = state;
                next.pending = None;
                next.slow = false;
                analysis.set(next);
            }
            return;
        }
        if state.pending.as_ref() == Some(&symbol) {
            return;
        }

        let mut next = state;
        next.pending = Some(symbol.clone());
        next.slow = false;
        analysis.set(next);
        // Unbounded, so this cannot fail for any reason but the worker being gone.
        let _ = requests.try_send(symbol.clone());

        // The wait, started by the request and by nothing else. A timer per request
        // rather than something polled: a symbol that comes back inside `SLOW_ANALYSIS`
        // — which is nearly all of them — costs one task that wakes up, finds the request
        // it belongs to already answered, and writes nothing.
        spawn(async move {
            Timer::after(SLOW_ANALYSIS).await;
            let mut analysis = analysis;
            let still = analysis.peek().pending.as_ref() == Some(&symbol);
            if still {
                analysis.write().slow = true;
            }
        });
    });
}

/// A step through the navigation history.
///
/// Back and forward are what the mouse buttons ask for; `To` is the history panel
/// clicking an entry. All three are a cursor move over a `History` method, so that
/// everything which moves the cursor keeps going through `navigate`.
#[derive(Clone, Copy)]
enum Nav {
    Back,
    Forward,
    /// Straight to the entry at this index, the one `History::recent` handed the row.
    To(usize),
}

impl Nav {
    /// Whether there is an entry to step to.
    fn possible(self, history: &History) -> bool {
        match self {
            Self::Back => history.can_back(),
            Self::Forward => history.can_forward(),
            Self::To(index) => history.can_jump(index),
        }
    }

    /// Move the cursor and hand back the entry it landed on.
    fn step(self, history: &mut History) -> Option<Document> {
        match self {
            Self::Back => history.back(),
            Self::Forward => history.forward(),
            Self::To(index) => history.jump(index),
        }
    }
}

/// Move the selection one entry back or forward through the history.
///
/// The one place navigation happens, so the input handler below and the history panel
/// share the same two steps: move the cursor, then make the entry it landed on the active
/// tab. Nothing is pushed -- it is a [`Visit::Moved`], and `would_push` would be false for
/// it in any case, that entry being exactly what the cursor now sits on.
///
/// It goes through [`activate`] rather than setting the selection itself because the
/// history and the open tabs are two different lists: the history is everywhere the reader
/// has been and keeps entries long after their tab was closed, so going back to one has to
/// be able to open a tab for it again.
fn navigate(
    open: State<Tabs<Document>>,
    mut history: State<History>,
    active: State<Option<Document>>,
    nav: Nav,
) {
    // Ask before writing. `State::write` notifies its subscribers whether or not the
    // value it hands over changes, so back at the oldest entry -- or forward at the
    // newest -- must not reach for it at all: a no-op has to leave the history alone,
    // leave the document on screen alone, and wake nothing.
    if !nav.possible(&history.peek()) {
        return;
    }

    // The guard is released at the end of this statement, before the selection is set
    // and `activate` peeks the history back.
    let entry = nav.step(&mut history.write());
    if entry.is_some() {
        activate(open, active, history, entry, Visit::Moved);
    }
}

/// Reopen the last project -- its name, binaries, tabs and selection -- once, at startup.
///
/// *Which* project that is, and what a project even is, is `project::reopen`'s: the app
/// asks for the last one and is handed its id and its two halves, or nothing. Nothing
/// here chooses, which is what keeps the recent-projects view and this hook from being
/// two answers to the same question: that view goes through [`switch_project`], which
/// ends in the same [`restore_project`] this does.
///
/// `use_hook` runs its initializer on mount and never again, which is what makes this
/// happen exactly once.
fn use_restore_on_startup(states: ProjectStates) {
    use_hook(move || {
        let Some((id, project, session)) = project::reopen() else {
            return;
        };

        // Synchronously, and before anything else here: `project::reopen` has just seeded
        // the save policy's baseline from this same project, and the two have to agree by
        // the time the first effect runs or the save observer would see the name as a
        // change and write it straight back out -- with the binaries still empty, since
        // those are restored a worker thread later. Hooks run during the parent's render
        // and effects after it, which is what makes "before" a fact rather than a hope.
        let mut proj = states.proj;
        proj.set(OpenProject::opened(id, &project));

        restore_project(states, project, session);
    });
}

/// Put a project's binaries, tabs, active document and history on screen.
///
/// The whole of what a restore *is*, and shared by the two things that do one -- the app
/// starting and a switch to another project -- so that the second cannot drift from the
/// first. It is the toolbar's `on_open` pattern verbatim for the parsing itself:
/// CPU-bound `open_files` on a `std::thread`, the result back over an `async_channel`,
/// `spawn` being freya's own task spawner and callable both during render and from an
/// event handler. So a large binary parses with the window already up and interactive.
///
/// Every step degrades silently: no project or an unreadable one is `None`, a path that
/// no longer exists or no longer parses just contributes no `Object` (`open_files`
/// swallows its own failures), `Session::resolve` falls back from a vanished symbol to
/// its object and from a vanished object to nothing, and `Session::resolve_history` and
/// `Session::resolve_tabs` drop what no longer points anywhere -- the history keeping
/// its cursor on the right one. A source-driven tab resolves against nothing and so
/// always comes back, a deleted file included: it returns as a tab over the pane's own
/// "Source file not found", which is the true answer and a visible one.
///
/// **The strip is rebuilt through the functions that hold the app's invariants**, never
/// by writing the list directly, so a restored session is in a state the app could have
/// got into by hand: every tab through [`activate`], of either kind. Two orderings follow
/// from that and are the only genuinely new rules here:
///
/// - The **tabs before the active document**. `activate` opens what it cannot find, so
///   restoring the active one first would leave its tab at the end of the strip instead
///   of in the place the reader left it. The other direction is safe: it can have
///   degraded to its object while the strip still holds the symbol, and `activate` simply
///   opens a tab for it, which is also what the reader would see had they closed that tab
///   themselves.
/// - The **rows go into the two `Positions` maps before the tabs are opened**. Those maps
///   are the one thing the restore writes directly, and a pane puts its view back when it
///   notices the tab it is showing has changed, so a row arriving after the `activate`
///   arrives after the only moment anything looks at it.
///
/// Every write below happens in one go, before the frame can end: freya's effects are
/// woken by an async notify (`Effect::create`) rather than run at the write, so
/// `use_save_on_change` sees the settled result once and not each intermediate `Active`
/// the tab loop passes through.
fn restore_project(states: ProjectStates, project: Project, session: Session) {
    let ProjectStates {
        objects,
        loading,
        open,
        mut asm_at,
        mut src_at,
        active,
        history,
        ..
    } = states;

    if project.binaries.is_empty() {
        return;
    }

    spawn(async move {
        // The objects arrive as they are parsed and the sidebar fills in behind them, so
        // the reader can be clicking through the first archive member before the last one
        // exists. What waits for the whole load is the *session*: a tab, the active
        // document or a history entry is resolved against the objects by name, and
        // resolving one against a half-filled list would drop the tabs whose object had
        // not landed yet.
        open_binaries(objects, loading, project.binaries.clone()).await;

        let (objects, mut history) = (objects, history);
        // Nothing opened: leave the app empty *and* leave the file alone, so a
        // binary that is only temporarily missing is not forgotten.
        if objects.peek().is_empty() {
            return;
        }

        // Resolved against everything now loaded rather than just what this load
        // contributed, so this stays correct if the user managed to open something
        // first. All three are computed before any of them is set so the read guard is
        // long gone by the time anything is notified.
        let (restored_history, restored_tabs, restored_active) = {
            let loaded = objects.read();
            (
                session.resolve_history(&loaded),
                session.resolve_tabs(&loaded),
                session.resolve(&loaded),
            )
        };

        // The history first, so that the `Visit::Went` at the end of this has a cursor to
        // dedup against.
        history.set(restored_history);

        // Where each side of each tab was left goes in *before* the tab is opened; see
        // above. Then the strip, oldest tab first, and then the one that was active. Each
        // of these is an `Active` write that will be overwritten by the next, which is the
        // price of there being exactly one way to open a tab; the last one is the only one
        // anything observes.
        {
            let (mut asm, mut src) = (asm_at.write(), src_at.write());
            for (tab, asm_row, src_row) in &restored_tabs {
                asm.remember(tab.clone(), *asm_row);
                src.remember(tab.clone(), *src_row);
            }
        }
        for (tab, _, _) in restored_tabs {
            // Reopening a tab is not visiting it: the reader had it open, and the history
            // this restore has just set is the record of where they went.
            activate(open, active, history, Some(tab), Visit::Moved);
        }
        // The one exception, and it is what keeps the cursor and the app in step: the
        // document the app *lands on* is a place it went. `would_push` makes it free in
        // the ordinary case — the saved cursor entry is the saved active document, and
        // the two resolve through the same lookup to the same `Arc`s — and records it
        // exactly when they differ, which is when the cursor entry was dropped or the
        // active document degraded and the app really is somewhere new.
        activate(open, active, history, restored_active, Visit::Went);
    });
}

/// Empty the app of everything that belonged to the project being left.
///
/// **Through the functions that hold the invariants and nothing else**, which is the
/// same rule a restore goes through in the other direction: closing every binary takes
/// its objects, its assembly-driven tabs, their viewing positions, the history entries
/// into it and the active document with them ([`close_binary`]), and the source-driven
/// tabs it deliberately leaves standing are then closed one by one ([`close_tab`]).
/// Writing the list directly would be shorter and would be the one place in the app where
/// "the active document is the active tab" was held by hand.
///
/// The **history** is then emptied outright, which is the one thing here that no walk
/// reaches: `close_binary` drops only the entries into the file it closes and `close_tab`
/// drops none at all, so a visited source file — which belongs to no binary — would
/// otherwise survive into the project that comes next.
///
/// The source tabs go here where a closing *binary* deliberately leaves them alone: a
/// file tab outlives the binary that led the reader to it because the text stands on its
/// own, but it does not outlive the project, whose session is what recorded that it was
/// open.
fn clear_project(states: ProjectStates) {
    let ProjectStates {
        objects,
        mut loading,
        open,
        asm_at,
        src_at,
        active,
        history,
        ..
    } = states;

    // Every load at once, and before the closes rather than through them: a file that has
    // been asked for and has produced nothing yet is not in the objects list, so nothing
    // below would reach it, and its objects would arrive into the project that comes next.
    loading.write().clear();

    // Both reads are bound before anything writes, which is the `peek` guard rule and
    // also the plain iteration rule: `close_binary` writes the very list being walked.
    let binaries = project::binaries(&objects.peek());
    for path in binaries {
        close_binary(
            objects, loading, open, active, asm_at, src_at, history, &path,
        );
    }

    let remaining = open.peek().tabs().to_vec();
    for tab in &remaining {
        close_tab(open, active, history, asm_at, src_at, tab);
    }

    // And the history outright, which the two walks above deliberately do not do for it.
    // `close_binary` drops only the entries into the file it is closing, and `close_tab`
    // drops none at all -- a history entry outlives its tab, which is the whole point of
    // there being two lists. Neither reaches a visited *source file*, which belongs to no
    // binary; and the history belongs to the project, whose session is what recorded it.
    let mut history = history;
    history.set(History::default());
}

/// Leave the project on screen and open the one `id` names in its place.
///
/// Three steps, in an order that is the whole of why a switch is safe. `project::switch`
/// goes first: it flushes what the old project had pending while the save policy still
/// points at it, and re-points every baseline at the new one — empty, because the app is
/// about to be empty. Only then is the app emptied, so the save observer, which is woken
/// by a notify and runs after this handler rather than during it, sees one settled state
/// that matches the baseline exactly and writes nothing at all. The restore then arrives
/// as an ordinary change and is written into the new project the way any other is.
///
/// A project whose directory has gone since the list named it does nothing but leave the
/// reader where they are; the row goes on the next reading of the list.
fn switch_project(states: ProjectStates, id: ProjectId) {
    let Some((project, session)) = project::switch(&id) else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &project));
    restore_project(states, project, session);
}

/// Start a project nobody has named yet and go to it. [`switch_project`] with nothing to
/// restore, an empty project being empty.
fn new_project(states: ProjectStates) {
    let Some(id) = project::start_new() else {
        return;
    };

    clear_project(states);
    let mut proj = states.proj;
    proj.set(OpenProject::opened(id, &Project::default()));
}

pub fn app() -> impl IntoElement {
    // What the user has said: the theme choice and the two font overrides, read off disk
    // once and then edited by the settings page. Before everything, because the theme and
    // the fonts are resolved from it and both have to be right on the first frame.
    let prefs =
        use_provide_context(|| Prefs(State::create(EditedSettings::of(&Settings::load())))).0;
    // The theme, the fonts and the file, from that one state. See `use_settings`.
    use_settings(prefs);
    // freya's own components -- the filter boxes, the scrollbars, the resizable handle,
    // the tooltips -- take their colours from its `Theme` and not from the palette, so
    // the sheet has to follow the appearance too; `interface_theme` is also where the
    // tooltip's font size is set, which is the one thing freya's theme is used for that
    // has nothing to do with colours -- and the one place a font change has to be carried
    // into freya's theming rather than being picked up by a re-render, which is why the
    // interface size is a dep here beside the appearance.
    //
    // Two calls and not one: `use_init_theme` builds its value in a `use_hook`, so it
    // answers for the first render only, and the effect is what carries a later switch
    // into it. The effect's deps change on a theme or a font change and never per render.
    let mut interface = use_init_theme(|| interface_theme(appearance()));
    use_side_effect_with_deps(
        &(appearance(), fonts().ui.size()),
        move |(appearance, _): &(Appearance, f32)| {
            interface.set(interface_theme(*appearance));
        },
    );

    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    // The files on their way into it, which is what the Objects tree draws its
    // still-being-read rows from. Beside `objects` because it is the same list seen a
    // moment earlier.
    let loading = use_provide_context(|| Loading(State::create(Loads::default()))).0;
    let active = use_provide_context(|| Active(State::create(None))).0;
    // The places open in the content area, of which `active` is the one on screen. The
    // list is opened and closed only through `activate`/`close_tab`, which is what keeps
    // "the active document is the active tab" an invariant rather than a convention --
    // for the startup restore as much as for a click, since the list is part of the saved
    // session.
    let open = use_provide_context(|| Open(State::create(Tabs::default()))).0;
    // Where each side of each tab was left, which is a view of that list rather than a
    // second copy of it: an entry appears when a pane is scrolled and goes when the tab
    // it belongs to is closed, so the same functions hold this true as hold the list
    // itself.
    let asm_at = use_provide_context(|| AsmAt(State::create(Positions::default()))).0;
    let src_at = use_provide_context(|| SrcAt(State::create(Positions::default()))).0;
    let history = use_provide_context(|| Hist(State::create(History::default()))).0;
    // Where the pointer is pointing, which the assembly and source panes answer for each
    // other. A plain state like the ones above rather than something derived from them:
    // it is an input, written by whichever row the pointer is on.
    let focused = use_provide_context(|| Focused(State::create(None))).0;
    // Where a click fixed the two panes, which outlives the pointer moving on and is what
    // asks the other pane to scroll. Beside the focus rather than inside it because the
    // two answer different questions and a row can be either, neither or both.
    let pinned = use_provide_context(|| Pinned(State::create(None))).0;
    // The run of rows picked out to be copied, and whether the keyboard is holding Shift,
    // which is what turns the next click into "reach to here". Both are inputs like the
    // two above: one selection for the whole app, in whichever pane last took one.
    let marked = use_provide_context(|| Marked(State::create(None))).0;
    let mut shift = use_provide_context(|| Shift(State::create(false))).0;
    // Which project all of the above belongs to, and the two things the reader has said
    // about it. A state rather than something read out of `project.rs` when it is drawn,
    // because the project view both draws it and edits it -- which is also what let the
    // save policy stop carrying the name across its own calls.
    let proj = use_provide_context(|| Proj(State::create(OpenProject::default()))).0;
    // The eight of them together, since a project switch closes all of them and reopens
    // all of them.
    let states = ProjectStates {
        proj,
        objects,
        loading,
        open,
        asm_at,
        src_at,
        active,
        history,
    };
    use_save_on_change(states);
    use_clear_focus(active, focused, pinned);
    use_periodic_save();
    // After the save effect on purpose: the effect is in place, with the save policy's
    // empty baseline, before the restore can put anything into any of the states it
    // observes, so the restored session is seen by it as an ordinary change.
    use_restore_on_startup(states);

    // Rebuilt only when the object list changes, not on every selection change.
    let symbols = use_memo(move || {
        SymbolList(Arc::new(
            objects
                .read()
                .iter()
                .flat_map(|object| {
                    object.symbols_sorted.iter().cloned().map(|data| Symbol {
                        object: object.clone(),
                        data,
                    })
                })
                .collect::<Vec<_>>(),
        ))
    });
    use_provide_context(move || Symbols(symbols));

    // The selected symbol's disassembly and line info, worked out once on a worker thread
    // for every pane that wants them. Both used to run in `render` -- the disassembly in
    // the Assembly pane and the line info in a `use_memo` here -- which is worker-thread
    // work by the analysis crate's own note on it: the first line-info query against a big
    // binary builds the whole DWARF context (267 MB for `viewer-sample`) and stalled the
    // frame that asked for it.
    let analysis = use_provide_context(|| Analysis(State::create(Analyzed::default()))).0;
    use_analysis(active, analysis);
    // After the analysis, because the file the Source pane draws for an assembly-driven
    // tab is what the analysis says it is.
    use_clear_marks(active, analysis, marked);

    // The scratchpad: the source the reader edits, the crates it asks for, and the worker
    // that is the only thing which ever reads or writes its directory. Both states are
    // provided here rather than held by the view because a dock tab that is not the active
    // one in its panel is unmounted, and a buffer being typed into cannot live there.
    let pad = use_provide_context(|| Pad(State::create(PadState::default()))).0;
    let pad_text = use_provide_context(|| {
        PadText(State::create(CodeEditorData::new(
            Rope::from_str(&pad.peek().scratchpad.source),
            language(Path::new(SOURCE_FILE)),
        )))
    })
    .0;
    use_scratchpad(pad, pad_text, states);

    // One docking area per resizable pane: the left one a column of Objects, then
    // Symbols with Info tabbed beside it, then History at the bottom -- which is
    // where the goal asks for it, and where it is visible without a click. The
    // cost is that the three groups start at equal heights, so the symbol list is
    // shorter than it was; the handles between them, and dragging History onto the
    // middle panel, are both one gesture away. The right one is the split view the
    // goals ask to be the default: the source a symbol was compiled from beside its
    // assembly, at equal widths. All nine tabs share one `DockDrag<Tab>`, which
    // `use_drag` keeps at the root, so a tab can be dragged from either area into
    // the other; each area is told about the other so the one taking a tab can evict
    // it from the one losing it.
    let sidebar_dock = use_state(|| {
        DockArea::column(vec![
            vec![Tab::Objects],
            vec![Tab::Symbols, Tab::Info],
            vec![Tab::History],
        ])
    });
    let content_dock = use_state(|| {
        DockArea::row(vec![
            vec![Tab::Assembly, Tab::Project, Tab::Settings, Tab::Scratchpad],
            vec![Tab::Source],
        ])
    });
    use_hook(move || {
        let (mut sidebar_dock, mut content_dock) = (sidebar_dock, content_dock);
        sidebar_dock.write().other = Some(content_dock);
        content_dock.write().other = Some(sidebar_dock);
    });

    // The split is freya's own `ResizableContainer`: the sidebar panel keeps the
    // original fixed 300px (`PanelSize::px`, so the initial width is unchanged) and
    // the content panel is the single proportional one, which makes it take whatever
    // is left over -- the same thing the old `Size::flex(1.0)` did. Between them
    // freya inserts a `ResizableHandle`, a 4px draggable divider that replaces the
    // hairline border the sidebar used to draw. Docking cannot express a pixel
    // width, which is why this outer split is not itself a `DockingArea`.
    let split = ResizableContainer::new()
        .direction(Direction::Horizontal)
        .panel(
            ResizablePanel::new(PanelSize::px(300.0))
                .min_size(120.0)
                .child(docking_area(sidebar_dock)),
        )
        .panel(
            ResizablePanel::new(PanelSize::percent(100.0))
                .min_size(10.0)
                // The open-tab strip sits over the whole content area rather than inside
                // the Assembly pane: the active tab is what *both* panes show, and a bar
                // inside one of them would follow that pane wherever it was docked. The
                // docking area below it needs a parent that has been given the leftover
                // height, `DockingArea` rendering itself `.expanded()`.
                .child(
                    rect()
                        .expanded()
                        .content(Content::Flex)
                        .child(TabStrip)
                        .child(
                            rect()
                                .width(Size::fill())
                                .height(Size::flex(1.0))
                                .child(docking_area(content_dock)),
                        ),
                ),
        );

    rect()
        .expanded()
        .content(Content::Flex)
        .interface_font()
        // The interface text, set once here and inherited: freya resolves an element's
        // unset `color` from its parent's, so every label in the chrome that does not ask
        // for a colour of its own follows this one. In the light palette it is the black
        // that was already the default, so this changes nothing until the theme does.
        .color(palette().text_fg)
        .background(palette().pane_bg)
        // The mouse's own back and forward buttons drive the history. freya does
        // deliver them: winit turns X11 buttons 8 and 9, and Wayland's BTN_BACK/
        // BTN_SIDE and BTN_FORWARD/BTN_EXTRA, into `MouseButton::Back`/`Forward`,
        // freya-winit maps those one for one and puts them in the `PlatformEvent`,
        // and nothing between there and the handler filters on which button it is.
        // `on_global_pointer_down` rather than `on_pointer_down`: a global event is
        // emitted to its listeners with no hit test at all, so this fires wherever
        // in the window the cursor happens to be and no child can swallow it by
        // stopping propagation. The rows are unaffected -- `on_press` is left-button
        // only -- and so is `on_secondary_down`, which asks for the right button.
        .on_global_pointer_down(move |e: Event<PointerEventData>| match e.button() {
            Some(MouseButton::Back) => navigate(open, history, active, Nav::Back),
            Some(MouseButton::Forward) => navigate(open, history, active, Nav::Forward),
            _ => {}
        })
        // A row selection is swept out with the button down and ends wherever the button
        // comes up, which is very often not over the pane it started in -- so the end of
        // the gesture is watched for here, at the root, rather than by either list.
        .on_global_pointer_press(move |_| mark_release(marked))
        // Shift, watched globally for the reason on `Shift` itself: a pointer event
        // carries no modifiers, so the state of the key has to be known before the click
        // that asks about it. Global rather than on the focused pane so that the first
        // shift-click into a pane extends, instead of only the ones after it has the
        // keyboard; and it is a bool being set, so it costs a listening pane nothing.
        //
        // The key itself is tested as well as the modifier mask, and freya-edit's
        // `TextDragging` does the same: the press that turns Shift *on* is the one
        // platforms disagree about, some reporting the mask before the key it names and
        // some after. The mask is what keeps the two in step when a key event is missed
        // -- the window losing focus mid-gesture, say.
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            shift.set_if_modified(
                e.key == Key::Named(NamedKey::Shift) || e.modifiers.contains(Modifiers::SHIFT),
            );
        })
        .on_global_key_up(move |e: Event<KeyboardEventData>| {
            shift.set_if_modified(
                e.key != Key::Named(NamedKey::Shift) && e.modifiers.contains(Modifiers::SHIFT),
            );
        })
        // The context menu the objects tree opens on a file row. It is the *viewer* that
        // has to be here: it provides the root state `ContextMenu::open_from_event` looks
        // up -- opening a menu without one in an ancestor scope panics -- and it draws
        // the menu itself, at the pointer, on the overlay layer. At the root so the menu
        // inherits the interface font, as freya's own documentation asks, and it lays out
        // as nothing at all until a menu is open.
        .child(ContextMenuViewer::new())
        .child(toolbar(objects, loading))
        // `ResizableContainer` renders itself `.expanded()`, so it needs a parent
        // that has already been given the leftover height under the toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(split),
        )
}

/// The tests in this file that run the UI rather than the logic under it, and the
/// palette's, which have nowhere else to live.
///
/// Everything decided by cases lives in a framework-free module with its own tests
/// ([`crate::rows`] here), and this is deliberately not a second home for that. What is
/// here is what those modules cannot hold. The runner tests exist for the one class of
/// bug they are blind to by construction: a `State` borrow that is legal to the compiler
/// and panics at the moment a gesture ends. `mark_release` shipped holding a `peek` guard
/// across its own write, so *every* mouse-up on a run brought the window down, and no
/// amount of testing `RowSelection` would have said a word about it. A press, a sweep and
/// a release through freya's own headless runner is the smallest thing that would have.
///
/// The palette's tests are here because a `Color` is a freya type and the palette cannot
/// move out of this file. They assert the properties a second set of values can silently
/// break -- a foreground that has gone invisible against its own surface, a translucent
/// wash that says nothing over a dark ground, a capture colour that sends
/// `resolve_capture_color` walking up the dotted name -- rather than the values
/// themselves, which are a design and not an assertion.
#[cfg(test)]
mod tests {
    use super::*;
    use freya_testing::TestingRunner;

    /// Three rows wired exactly the way the two panes are, and no more of them than that:
    /// the press that starts a run, the `pointer_over` that sweeps it, and the release
    /// watched globally at the root, because the button very often comes up somewhere the
    /// run does not reach.
    fn harness() -> impl IntoElement {
        let marked = use_consume::<Marked>().0;

        let row = |index: usize| {
            rect()
                .width(Size::fill())
                .height(Size::px(20.0))
                .on_pointer_down(move |e: Event<PointerEventData>| {
                    if e.button() == Some(MouseButton::Left) {
                        mark_press(marked, false, Pane::Assembly, index);
                    }
                })
                .on_pointer_over(move |_| mark_drag(marked, Pane::Assembly, index))
                .into_element()
        };

        rect()
            .expanded()
            .on_global_pointer_press(move |_| mark_release(marked))
            .child(row(0))
            .child(row(1))
            .child(row(2))
    }

    /// The five states [`scrolling_harness`] is wired to, as context types of their own
    /// so that three `State<usize>`s cannot be confused for one another.
    #[derive(Clone, Copy)]
    struct KeptTab(State<String>);
    #[derive(Clone, Copy)]
    struct KeptAt(State<Positions<String>>);
    /// The tabs that are open, which is what a position is only kept for.
    #[derive(Clone, Copy)]
    struct KeptOpen(State<Tabs<String>>);
    #[derive(Clone, Copy)]
    struct KeptLength(State<usize>);
    /// The last row the pointer was over, which is how the test asks where the view
    /// actually is rather than believing what the map says about it.
    #[derive(Clone, Copy)]
    struct KeptTop(State<usize>);

    /// A scroll view wired the way both **code** panes are: one `ScrollController` reused
    /// across every tab the pane shows, `use_kept_position` between them, and
    /// [`code_row_height`] on both halves of the view -- which is what those panes are,
    /// and the only kind of list that keeps a position at all.
    fn scrolling_harness() -> impl IntoElement {
        let tab = use_consume::<KeptTab>().0;
        let at = use_consume::<KeptAt>().0;
        let open = use_consume::<KeptOpen>().0;
        let length = use_consume::<KeptLength>().0;
        let mut top = use_consume::<KeptTop>().0;

        let controller = use_scroll_controller(ScrollConfig::default);
        let showing = tab.read().clone();
        let rows = *length.read();
        use_kept_position(at, open, controller, &showing, rows);

        rect().expanded().child(
            VirtualScrollView::new_with_data_controlled(
                rows,
                move |index, _: &usize| {
                    rect()
                        .width(Size::fill())
                        .height(Size::px(code_row_height()))
                        .on_pointer_over(move |_| top.set(index))
                        .key(index)
                        .into()
                },
                controller,
            )
            .length(rows)
            .item_size(code_row_height()),
        )
    }

    /// A sidebar list's shape: the same view over [`list_row_height`], and no kept
    /// position, because the Objects and Symbols lists have none. It exists so that the
    /// agreement between an `item_size` and its rows is asserted for *both* heights rather
    /// than for one and assumed for the other.
    fn list_scrolling_harness() -> impl IntoElement {
        let mut top = use_consume::<KeptTop>().0;

        rect().expanded().child(
            VirtualScrollView::new_with_data(0usize, move |index, _: &usize| {
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height()))
                    .on_pointer_over(move |_| top.set(index))
                    .key(index)
                    .into()
            })
            .length(100usize)
            .item_size(list_row_height()),
        )
    }

    /// Switching tab puts the pane back where that tab was left, and a tab seen for the
    /// first time opens at the top rather than at the last one's offset.
    ///
    /// Headless because none of it is visible to any other kind of test: the position is
    /// read out of a `ScrollController` inside an effect that a scroll wakes, and what it
    /// is asserted against is which row a real `VirtualScrollView` put under the pointer.
    #[test]
    fn a_tab_comes_back_to_the_row_it_was_left_at() {
        let (mut test, (tab, at, open, _length, top)) = TestingRunner::new(
            scrolling_harness,
            (100., 100.).into(),
            |runner| {
                let mut tabs = Tabs::default();
                tabs.open("a".to_owned());
                tabs.open("b".to_owned());
                (
                    runner
                        .provide_root_context(|| KeptTab(State::create("a".to_owned())))
                        .0,
                    runner
                        .provide_root_context(|| KeptAt(State::create(Positions::default())))
                        .0,
                    runner
                        .provide_root_context(|| KeptOpen(State::create(tabs)))
                        .0,
                    runner
                        .provide_root_context(|| KeptLength(State::create(100)))
                        .0,
                    runner.provide_root_context(|| KeptTop(State::create(0))).0,
                )
            },
            1.,
        );
        let mut tab = tab;
        test.sync_and_update();

        // Where the top of the view is, asked the only way a pane can be asked: the
        // pointer is moved away first, or entering the same row twice is no event at all.
        let top_row = |test: &mut TestingRunner| {
            // Settled first: an effect is a spawned task, so the scroll it asks for lands
            // a poll after the state change that asked for it, and a view that moves under
            // a pointer already sitting still sends no `pointerover` to say so.
            for _ in 0..4 {
                test.sync_and_update();
            }
            test.move_cursor((50., 90.));
            test.sync_and_update();
            test.move_cursor((50., 5.));
            test.sync_and_update();
            *top.peek()
        };

        test.scroll((50., 50.), (0., -300.));
        test.sync_and_update();
        let left_at = top_row(&mut test);
        assert!(left_at > 0, "the wheel moved nothing");
        // The scroll was written down as it happened, which is what makes the position
        // survive the pane being left in any way at all -- including the window closing.
        assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

        // A tab this pane has never shown starts at the top, and pointedly not at the
        // offset the tab before it was at: that is the bug this hook exists for.
        tab.set("b".to_owned());
        test.sync_and_update();
        assert_eq!(top_row(&mut test), 0);
        // And the tab left behind is remembered, not overwritten by where the new one is.
        assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

        tab.set("a".to_owned());
        test.sync_and_update();
        assert_eq!(top_row(&mut test), left_at);

        // And closing the tab on screen does not put it back. `close_tab` forgets the
        // position and then moves to a neighbour, so the run that follows is holding a
        // tab that is gone -- which is a `Selection` holding a whole `Object` in the app.
        let (mut open, mut at) = (open, at);
        open.write().close(&"a".to_owned());
        at.write().forget(&"a".to_owned());
        tab.set("b".to_owned());
        for _ in 0..4 {
            test.sync_and_update();
        }
        assert_eq!(at.peek().at(&"a".to_owned()), None);
    }

    /// Nothing on screen: what a project switch does is to the states, and the states are
    /// what this asserts. A runner all the same, because a `State` needs a runtime and
    /// because the bug being looked for is a borrow held across a write, which is a
    /// runtime panic and not a compile error.
    fn project_harness() -> impl IntoElement {
        rect().expanded()
    }

    /// The eight contexts `app()` provides, in one `ProjectStates`, so a test can drive a
    /// switch exactly as the recent list's press does.
    ///
    /// A macro and not a function: the runner's type is `freya_core::integration::Runner`,
    /// which freya's prelude does not re-export, so naming it here would mean naming a
    /// crate the app does not depend on.
    macro_rules! project_states {
        () => {
            |runner: &mut _| project_states!(runner)
        };
        ($runner:expr) => {
            ProjectStates {
                proj: $runner
                    .provide_root_context(|| Proj(State::create(OpenProject::default())))
                    .0,
                objects: $runner
                    .provide_root_context(|| Objects(State::create(Vec::new())))
                    .0,
                loading: $runner
                    .provide_root_context(|| Loading(State::create(Loads::default())))
                    .0,
                open: $runner
                    .provide_root_context(|| Open(State::create(Tabs::default())))
                    .0,
                asm_at: $runner
                    .provide_root_context(|| AsmAt(State::create(Positions::default())))
                    .0,
                src_at: $runner
                    .provide_root_context(|| SrcAt(State::create(Positions::default())))
                    .0,
                active: $runner
                    .provide_root_context(|| Active(State::create(None)))
                    .0,
                history: $runner
                    .provide_root_context(|| Hist(State::create(History::default())))
                    .0,
            }
        };
    }

    /// Leaving a project leaves nothing of it behind: no object, no tab of either kind,
    /// no viewing position, no history entry and nothing active.
    ///
    /// Headless for the reason the swept run below is. `clear_project` goes through
    /// `close_binary` and `close_tab`, and each of those reads a state and then writes
    /// it -- which is legal to the compiler and panics at the moment it runs if the read
    /// is still borrowed. Asserting the emptiness is half of it; the other half is that
    /// the whole walk happens at all.
    ///
    /// The source-driven tab is the case a binary close deliberately leaves standing, so
    /// it is the one only this walk reaches.
    #[test]
    fn leaving_a_project_leaves_nothing_of_it_behind() {
        let symbols = fixture_symbols();
        let (first, second) = (symbols[0].clone(), symbols[1].clone());
        let object = first.object.clone();
        let source = Document::Source(Arc::from("/src/main.rs"));

        let (mut test, states) =
            TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
        test.sync_and_update();

        // The app as a session leaves it: a binary open, two of its functions in the
        // strip with a row remembered for one of them, a source file open beside them and
        // somewhere to go back to.
        let (mut objects, mut asm_at, mut src_at) = (states.objects, states.asm_at, states.src_at);
        objects.write().push(object.clone());
        let tab = |symbol: &Symbol| Document::Assembly(Selection::Symbol(symbol.clone()));
        let went = |target: Document| {
            activate(
                states.open,
                states.active,
                states.history,
                Some(target),
                Visit::Went,
            )
        };
        went(tab(&first));
        went(tab(&second));
        went(source.clone());
        asm_at.write().remember(tab(&first), 12);
        src_at.write().remember(source.clone(), 7);
        test.sync_and_update();

        assert_eq!(states.open.peek().tabs().len(), 3);
        // Three visits, the source file included: the history records documents, which is
        // what lets its panel list a file at all.
        assert_eq!(states.history.peek().entries().len(), 3);

        clear_project(states);
        test.sync_and_update();

        assert!(
            states.objects.peek().is_empty(),
            "an object was left behind"
        );
        assert!(
            states.open.peek().tabs().is_empty(),
            "a tab was left behind"
        );
        assert!(
            states.history.peek().entries().is_empty(),
            "a history entry was left behind"
        );
        // Not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points
        // into, so a position left here would hold the whole binary of the project just
        // left.
        assert_eq!(
            states.asm_at.peek().at(&tab(&first)),
            None,
            "a viewing position was left behind"
        );
        assert_eq!(
            states.src_at.peek().at(&source),
            None,
            "a source position was left behind"
        );
        assert!(
            states.active.peek().is_none(),
            "the app still points into the project just left"
        );
    }

    /// The history records where the reader *went* and not what is on screen.
    ///
    /// The rule Step 1e settled, and the reason `activate` is told why it is being called:
    /// opening a document is a visit, switching to a tab that is already open is not, and
    /// the neighbour a close lands on is not either. An effect observing the active
    /// document could not tell any of these apart, which is why the recording is no longer
    /// one.
    #[test]
    fn switching_to_an_open_tab_is_not_a_visit() {
        let symbols = fixture_symbols();
        let object = symbols[0].object.clone();
        let (first, second) = (
            Document::Assembly(Selection::Symbol(symbols[0].clone())),
            Document::Assembly(Selection::Symbol(symbols[1].clone())),
        );

        let (mut test, states) =
            TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
        test.sync_and_update();

        let mut objects = states.objects;
        objects.write().push(object);
        let go = |target: &Document, visit| {
            activate(
                states.open,
                states.active,
                states.history,
                Some(target.clone()),
                visit,
            )
        };

        go(&first, Visit::Went);
        go(&second, Visit::Went);
        test.sync_and_update();
        assert!(states.history.peek().entries() == [first.clone(), second.clone()]);

        // Back to the first through the strip: it is already open, so the reader has gone
        // nowhere and the cursor stays where it was.
        go(&first, Visit::Moved);
        test.sync_and_update();
        assert!(*states.active.peek() == Some(first.clone()));
        assert!(
            states.history.peek().entries() == [first.clone(), second.clone()],
            "a strip click was recorded as a visit"
        );
        assert_eq!(states.history.peek().cursor(), Some(1));

        // Going there deliberately *is* one, and bumps it to the newest position.
        go(&first, Visit::Went);
        test.sync_and_update();
        assert!(states.history.peek().entries() == [second, first.clone()]);

        // And closing the tab lands on the neighbour without recording it.
        close_tab(
            states.open,
            states.active,
            states.history,
            states.asm_at,
            states.src_at,
            &first,
        );
        test.sync_and_update();
        assert_eq!(states.open.peek().tabs().len(), 1);
        assert_eq!(
            states.history.peek().entries().len(),
            2,
            "closing a tab recorded the neighbour it landed on"
        );
    }

    /// Closing a binary takes its own tabs and leaves a source-driven one standing.
    ///
    /// The rule the one strip inherited from the two: a file tab outlives the binary that
    /// led the reader to it, because the text stands on its own and nothing records which
    /// object opened it. Worth a runner rather than a `Tabs` test, because what has to
    /// hold is that `close_binary` lands the *active* document somewhere sensible when the
    /// tab it was on goes and a tab of the other kind is what is left.
    #[test]
    fn closing_a_binary_keeps_the_source_tabs() {
        let symbols = fixture_symbols();
        let symbol = symbols[0].clone();
        let object = symbol.object.clone();
        let path = object.path.clone();
        let source = Document::Source(Arc::from("/src/main.rs"));
        let function = Document::Assembly(Selection::Symbol(symbol));

        let (mut test, states) =
            TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
        test.sync_and_update();

        let mut objects = states.objects;
        objects.write().push(object);
        let went = |target: Document| {
            activate(
                states.open,
                states.active,
                states.history,
                Some(target),
                Visit::Went,
            )
        };
        went(source.clone());
        went(function.clone());
        test.sync_and_update();
        assert_eq!(states.open.peek().tabs().len(), 2);

        close_binary(
            states.objects,
            states.loading,
            states.open,
            states.active,
            states.asm_at,
            states.src_at,
            states.history,
            &path,
        );
        test.sync_and_update();

        assert!(
            states.open.peek().tabs() == [source.clone()],
            "the file tab went with the binary"
        );
        assert!(
            *states.active.peek() == Some(source),
            "closing the binary did not land on the tab that survived it"
        );
    }

    /// The channel a load test feeds by hand, and the paths the harness registers as
    /// being read. Standing in for `open_binaries`' worker thread: what has to be
    /// asserted is what the app does with an answer that arrives after the reader has
    /// moved on, which against a real worker is a race and against a channel is a fact.
    /// The receiver is *taken* rather than cloned, because a clone left in the context
    /// map would keep the channel open for ever and the test could never see the one
    /// thing that stops a worker: `take_load` returning and dropping the last receiver.
    #[derive(Clone)]
    struct Feed(
        Arc<Mutex<Option<async_channel::Receiver<Progress>>>>,
        Arc<Vec<PathBuf>>,
    );

    /// The real `take_load` over the real Objects tree, with the worker replaced by
    /// [`Feed`]. The tree is mounted rather than left out so that every one of these
    /// tests also builds the rows for a file that is being read -- including the row with
    /// no group behind it, which is the one shape no other test reaches.
    fn load_harness() -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let loading = use_consume::<Loading>().0;
        let feed = use_consume::<Feed>().clone();

        use_hook(move || {
            let Feed(events, paths) = feed;
            let events = events
                .lock()
                .expect("the feed is not poisoned")
                .take()
                .expect("the harness is mounted once");
            // Bound out of its own statement, so the guard is gone before anything else
            // touches the state.
            let id = {
                let mut loading = loading;
                loading.write().begin(&paths)
            };
            spawn(async move { take_load(objects, loading, id, events).await });
        });

        rect().expanded().child(ObjectsTab)
    }

    /// `n` objects that all came out of one path, which is what an archive's members look
    /// like to everything above the analysis crate. Parsed `n` times rather than cloned,
    /// so they are `n` distinct `Arc`s exactly as real members are.
    fn fixture_objects(n: usize) -> (PathBuf, Vec<Arc<Object>>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/analysis/tests/fixtures/line_fixture.o");
        let objects = (0..n)
            .map(|_| {
                analysis::open_files(vec![path.clone()])
                    .first()
                    .expect("the fixture parses")
                    .clone()
            })
            .collect();
        (path, objects)
    }

    /// Mount [`load_harness`] over one path, and hand back the states and the sender the
    /// test plays the worker with.
    fn mount_load(
        path: &Path,
    ) -> (
        TestingRunner,
        ProjectStates,
        async_channel::Sender<Progress>,
    ) {
        let (sender, events) = async_channel::unbounded::<Progress>();
        let paths = Arc::new(vec![path.to_path_buf()]);
        let events = Arc::new(Mutex::new(Some(events)));
        let (test, states) = TestingRunner::new(
            load_harness,
            (300., 300.).into(),
            move |runner| {
                runner.provide_root_context(|| Feed(events.clone(), paths.clone()));
                project_states!(runner)
            },
            1.,
        );
        (test, states, sender)
    }

    /// How the Objects tree describes what is on screen, which is the one thing these
    /// tests are really about: a file that is being read has a row before it has an
    /// object, and stops saying so when the last of them has landed.
    fn reading(states: &ProjectStates) -> Vec<(String, usize, bool)> {
        let tree = ObjectTree::new(
            &states.objects.peek(),
            &states.loading.peek(),
            &Filter::default().matcher(),
            &HashSet::new(),
        );
        (0..tree.len())
            .filter_map(|row| match tree.row(row) {
                TreeRow::File {
                    name,
                    members,
                    loading,
                    ..
                } => Some((name.clone(), *members, *loading)),
                TreeRow::Object { .. } => None,
            })
            .collect()
    }

    /// The sub-step in one test: the objects of one file reach the sidebar one at a time,
    /// and the row for that file is there before the first of them is.
    #[test]
    fn objects_reach_the_sidebar_as_they_are_parsed() {
        let (path, objects) = fixture_objects(3);
        let (mut test, states, sender) = mount_load(&path);
        test.sync_and_update();

        // Before a single byte has been parsed. This is the state `Goals.md` asks for an
        // indicator for and which nothing could be in while the parse handed back one
        // `Vec` at the end.
        assert_eq!(reading(&states), [("line_fixture.o".to_owned(), 0, true)]);
        assert!(states.objects.peek().is_empty());

        for (arrived, object) in objects.iter().enumerate() {
            sender
                .send_blocking(Progress::Parsed(object.clone()))
                .expect("the app is still listening");
            pump(&mut test, || states.objects.peek().len() == arrived + 1);
            assert_eq!(
                reading(&states),
                [("line_fixture.o".to_owned(), arrived + 1, true)],
                "the file stopped saying it was being read before it was finished"
            );
            // The save side: the path joins the binaries with its first object, so a
            // session written half way through a parse names the file rather than a
            // truncated version of it. There is nothing else in `binaries` to truncate --
            // it is a list of paths.
            assert_eq!(project::binaries(&states.objects.peek()), [path.clone()]);
        }

        sender
            .send_blocking(Progress::Finished(path.clone()))
            .expect("the app is still listening");
        pump(&mut test, || !states.loading.peek().is_loading(&path));

        // Done, so the ordinary rules take over again: three objects out of one file is
        // an archive-shaped row, and nothing says it is still being read.
        assert_eq!(reading(&states), [("line_fixture.o".to_owned(), 3, false)]);
    }

    /// Closing a file half way through reading it takes the objects that have already
    /// arrived *and* the ones that have not.
    ///
    /// The second half is what needs a test: the worker is already parsing when the row
    /// is closed, so the answers exist whatever the app does, and without the check they
    /// would put the file back one member at a time.
    #[test]
    fn a_file_closed_while_it_is_read_takes_the_rest_of_its_objects_with_it() {
        let (path, objects) = fixture_objects(2);
        let (mut test, states, sender) = mount_load(&path);
        test.sync_and_update();

        sender
            .send_blocking(Progress::Parsed(objects[0].clone()))
            .expect("the app is still listening");
        pump(&mut test, || states.objects.peek().len() == 1);

        close_binary(
            states.objects,
            states.loading,
            states.open,
            states.active,
            states.asm_at,
            states.src_at,
            states.history,
            &path,
        );
        test.sync_and_update();
        assert!(states.objects.peek().is_empty());
        assert!(reading(&states).is_empty(), "a closed file is still a row");

        // The answer that was already on its way.
        sender
            .send_blocking(Progress::Parsed(objects[1].clone()))
            .expect("the worker has not been told yet");
        for _ in 0..8 {
            test.sync_and_update();
        }
        assert!(
            states.objects.peek().is_empty(),
            "an object arrived for a file that had been closed"
        );

        // And the worker is told, by the only thing that can tell it: the receiver is
        // gone, so its next send fails and the walk stops rather than parsing the rest of
        // a file nobody is waiting for.
        assert!(sender.send_blocking(Progress::Finished(path)).is_err());
    }

    /// Leaving a project while one of its files is being read. The load is cancelled by
    /// `clear_project` itself and not through `close_binary`, because a file that has
    /// produced nothing yet is not in the objects list for the per-path walk to reach.
    #[test]
    fn leaving_a_project_while_a_file_is_read_drops_what_was_still_coming() {
        let (path, objects) = fixture_objects(2);
        let (mut test, states, sender) = mount_load(&path);
        test.sync_and_update();

        clear_project(states);
        test.sync_and_update();
        assert!(states.loading.peek().paths().is_empty());
        assert!(reading(&states).is_empty());

        sender
            .send_blocking(Progress::Parsed(objects[0].clone()))
            .expect("the worker has not been told yet");
        for _ in 0..8 {
            test.sync_and_update();
        }
        assert!(
            states.objects.peek().is_empty(),
            "the project just left got an object out of the load it abandoned"
        );
        assert!(sender
            .send_blocking(Progress::Parsed(objects[1].clone()))
            .is_err());
    }

    /// Reading a file that is still being read, which is the whole point of the sub-step:
    /// an object that has arrived is an ordinary row, selecting it opens an ordinary tab,
    /// and the members still landing behind it change none of that.
    #[test]
    fn a_file_still_being_read_can_be_explored() {
        let (path, objects) = fixture_objects(3);
        let (mut test, states, sender) = mount_load(&path);
        test.sync_and_update();

        sender
            .send_blocking(Progress::Parsed(objects[0].clone()))
            .expect("the app is still listening");
        pump(&mut test, || states.objects.peek().len() == 1);

        // Through `activate`, which is the only way anything opens a tab -- a partially
        // read file is not a special case for it.
        let opened = Document::Assembly(Selection::Object(objects[0].clone()));
        activate(
            states.open,
            states.active,
            states.history,
            Some(opened.clone()),
            Visit::Went,
        );
        test.sync_and_update();

        for object in &objects[1..] {
            sender
                .send_blocking(Progress::Parsed(object.clone()))
                .expect("the app is still listening");
        }
        pump(&mut test, || states.objects.peek().len() == 3);
        sender
            .send_blocking(Progress::Finished(path.clone()))
            .expect("the app is still listening");
        pump(&mut test, || !states.loading.peek().is_loading(&path));

        assert!(
            *states.active.peek() == Some(opened),
            "the active document moved while the rest of the file was arriving"
        );
        assert_eq!(states.open.peek().tabs().len(), 1);
        assert_eq!(states.objects.peek().len(), 3);
    }

    /// What the two text boxes mean, which is the one place the project view's `String`s
    /// and `project.toml`'s absent keys meet. An empty box is not a project named the
    /// empty string: it is a project the reader has not named, which is what anonymous
    /// *is*, and a box holding spaces says exactly as much.
    #[test]
    fn an_empty_box_is_a_project_that_has_not_been_named() {
        assert_eq!(OpenProject::default().details(), Details::default());

        let blank = OpenProject {
            id: None,
            name: "   ".to_owned(),
            directory: String::new(),
        };
        assert_eq!(blank.details(), Details::default());

        let named = OpenProject {
            id: None,
            name: " kernel ".to_owned(),
            directory: "/src/kernel".to_owned(),
        };
        assert_eq!(
            named.details(),
            Details {
                name: Some("kernel".to_owned()),
                directory: Some(PathBuf::from("/src/kernel")),
            }
        );
    }

    /// And back the other way, which is what a restore and a switch both do: a project
    /// with no name comes back as an empty box rather than as the word "None".
    #[test]
    fn an_unnamed_project_comes_back_as_an_empty_box() {
        let id = ProjectId::new("project-1").expect("an id");
        let open = OpenProject::opened(id.clone(), &Project::default());
        assert_eq!(open.id, Some(id));
        assert!(open.name.is_empty() && open.directory.is_empty());
        // And a round trip through the two spellings changes nothing.
        assert_eq!(open.details(), Details::default());
    }

    /// The analysis worker's work, handed in through a context so a test can substitute
    /// one that stops when it is told to. `Arc<dyn Fn>` and not a generic, because a
    /// context value is one concrete type.
    #[derive(Clone)]
    struct Study(Arc<dyn Fn(Symbol) -> Studied + Send + Sync>);

    /// Every distinct symbol the panes were told to draw, in order. The assertion the
    /// superseding rule is really about is not what is on screen at the end but what was
    /// *never* on screen, and only a recording can say that.
    #[derive(Clone, Copy)]
    struct Seen(State<Vec<Symbol>>);

    /// The analysis wiring and nothing else: no panes, since what is under test is which
    /// answers reach them rather than what they draw.
    fn analysis_harness() -> impl IntoElement {
        let active = use_consume::<Active>().0;
        let analysis = use_consume::<Analysis>().0;
        let study = use_consume::<Study>().0;
        let mut seen = use_consume::<Seen>().0;

        use_analysis_with(active, analysis, move |symbol| study(symbol));

        use_side_effect(move || {
            let shown = analysis.read().shown.clone();
            let Some(shown) = shown else {
                return;
            };
            // `peek` on the state it writes, or the effect would wake itself for ever.
            let repeat = seen.peek().last().is_some_and(|last| *last == shown.symbol);
            if !repeat {
                seen.write().push(shown.symbol);
            }
        });

        rect().expanded()
    }

    /// Run the test runner until `ready` answers, and then a little further so that
    /// whatever the answer woke has run too.
    ///
    /// A worker thread and two channels sit between a state change and the state it ends
    /// in -- the analysis worker's and, since 10c, the scratchpad's -- so how many turns
    /// of the loop that takes is not something a test can know, only that it is finite. Failing loudly rather than asserting on what happened to
    /// have arrived, since "the answer never came" and "the answer was wrong" are
    /// different bugs.
    fn pump(test: &mut TestingRunner, ready: impl Fn() -> bool) {
        for _ in 0..200 {
            test.sync_and_update();
            if ready() {
                for _ in 0..4 {
                    test.sync_and_update();
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("the worker's answer never landed");
    }

    /// The committed gcc fixture the analysis crate is pinned against, parsed the way the
    /// app parses it. Small, real DWARF, three functions -- so a `Studied` built from one
    /// of its symbols has both halves and neither is empty.
    fn fixture_symbols() -> Vec<Symbol> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("crates/analysis/tests/fixtures/line_fixture.o");
        let objects = analysis::open_files(vec![path]);
        let object = objects.first().expect("the fixture parses").clone();

        object
            .symbols_sorted
            .iter()
            .cloned()
            .map(|data| Symbol {
                object: object.clone(),
                data,
            })
            .collect()
    }

    /// The central correctness question of Step 11c: an answer for a symbol the reader has
    /// already clicked past must never reach the panes.
    ///
    /// Staged rather than raced. The worker is a real thread running the real
    /// `use_analysis_with` machinery, but the work it does is a gate the test opens one
    /// job at a time, which is the only way to be *sure* the stale answer was produced,
    /// delivered and then dropped rather than merely being slow. That the test can set the
    /// selection twice while the worker sits blocked is itself the other half of the
    /// sub-step: the UI thread is not waiting for any of this.
    ///
    /// It also pins the hazard the per-tab viewing position brings: while a symbol is
    /// pending, `shown` is not it, so no pane is ever mounted for a tab whose listing does
    /// not exist yet -- which is what keeps `use_kept_position` from writing that tab down
    /// at row 0 before the reader has seen a single row of it.
    #[test]
    fn an_answer_for_a_symbol_no_longer_selected_is_dropped() {
        let symbols = fixture_symbols();
        let (first, second) = (symbols[0].clone(), symbols[1].clone());

        // The worker announces each job as it takes it and then waits to be let go.
        // `async_channel` on both sides and not `std::sync::mpsc`, whose `Receiver` is not
        // `Sync` and so cannot sit inside a shared `Fn`.
        let (started, starts) = async_channel::unbounded::<Symbol>();
        let (gate, gated) = async_channel::unbounded::<()>();
        let study = move |symbol: Symbol| {
            let _ = started.send_blocking(symbol.clone());
            let _ = gated.recv_blocking();
            Studied::new(symbol)
        };

        let (mut test, (selection, analysis, seen)) = TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            move |runner| {
                runner.provide_root_context(|| Study(Arc::new(study)));
                (
                    runner
                        .provide_root_context(|| Active(State::create(None)))
                        .0,
                    runner
                        .provide_root_context(|| Analysis(State::create(Analyzed::default())))
                        .0,
                    runner
                        .provide_root_context(|| Seen(State::create(Vec::new())))
                        .0,
                )
            },
            1.,
        );
        let mut selection = selection;
        let settle = |test: &mut TestingRunner| {
            for _ in 0..8 {
                test.sync_and_update();
            }
        };
        settle(&mut test);

        // The first click. The worker takes it and stops inside it.
        selection.set(Some(Document::Assembly(Selection::Symbol(first.clone()))));
        pump(&mut test, || !starts.is_empty());
        assert!(starts.recv_blocking().expect("the worker started") == first);
        assert!(
            analysis.peek().shown.is_none(),
            "the pane was handed a listing the worker has not produced"
        );

        // The second click, while the first is still being worked on. That the UI takes
        // it at all is the other half of what this sub-step is for.
        selection.set(Some(Document::Assembly(Selection::Symbol(second.clone()))));
        settle(&mut test);

        // Let the first one finish. Its answer is on the channel by the time the worker
        // announces the second job, so what follows is not a race with it.
        gate.send_blocking(()).expect("the gate");
        assert!(starts.recv_blocking().expect("the worker started") == second);
        settle(&mut test);

        assert!(
            analysis.peek().shown.is_none(),
            "an answer for a symbol the reader had left was put on screen"
        );
        assert!(analysis.peek().pending.as_ref() == Some(&second));

        // And the answer that is wanted lands.
        gate.send_blocking(()).expect("the gate");
        pump(&mut test, || analysis.peek().shown.is_some());

        let state = analysis.peek().clone();
        let shown = state.shown.expect("the second symbol was analysed");
        assert!(shown.symbol == second);
        assert!(state.pending.is_none());
        assert!(!state.slow);
        assert_eq!(
            seen.peek().len(),
            1,
            "a superseded listing reached the panes"
        );
        assert!(seen.peek()[0] == second);
    }

    /// The happy path, over the real work rather than a gate: a symbol selected comes back
    /// disassembled, with the line info and the file the Source pane draws beside it,
    /// and with the panes told about it exactly once.
    #[test]
    fn a_selected_symbol_comes_back_disassembled_and_mapped() {
        let symbol = fixture_symbols()
            .into_iter()
            .find(|symbol| symbol.data.name == "sum_to")
            .expect("the fixture holds sum_to");

        let (mut test, (selection, analysis, seen)) = TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            |runner| {
                runner.provide_root_context(|| Study(Arc::new(Studied::new)));
                (
                    runner
                        .provide_root_context(|| Active(State::create(None)))
                        .0,
                    runner
                        .provide_root_context(|| Analysis(State::create(Analyzed::default())))
                        .0,
                    runner
                        .provide_root_context(|| Seen(State::create(Vec::new())))
                        .0,
                )
            },
            1.,
        );
        let mut selection = selection;
        test.sync_and_update();

        selection.set(Some(Document::Assembly(Selection::Symbol(symbol.clone()))));
        pump(&mut test, || analysis.peek().shown.is_some());

        let state = analysis.peek().clone();
        let shown = state.shown.expect("the symbol was analysed");
        assert!(shown.symbol == symbol);
        assert!(state.pending.is_none());
        let assembly = shown.assembly.expect("sum_to holds code");
        assert!(!assembly.instructions.is_empty());
        let lines = shown.lines.info.expect("the fixture has DWARF");
        assert!(!lines.files().is_empty());
        assert!(shown
            .lines
            .file
            .as_deref()
            .is_some_and(|file| file.ends_with("line_fixture.c")));
        assert_eq!(seen.peek().len(), 1);

        // Selecting something that is not a symbol is answered on the spot: clearing does
        // not wait on the worker, only replacing does.
        selection.set(None);
        test.sync_and_update();
        assert!(analysis.peek().clone() == Analyzed::default());
    }

    /// What the panes are told to say, which is a rule about honesty rather than about
    /// pixels: a listing is replaced by the next listing and never by a blank, a wait is
    /// only named once it is long enough to have been noticed, and "no symbol selected" is
    /// said only when none is.
    #[test]
    fn nothing_is_said_until_the_wait_is_worth_saying() {
        let symbol = fixture_symbols().into_iter().next().expect("a symbol");
        let studied = Studied::new(symbol.clone());

        let idle = Analyzed::default();
        assert!(matches!(
            idle.showing(),
            Showing::Message("No symbol selected")
        ));

        // Nothing analysed yet and something on its way: an empty pane, not a message.
        let opening = Analyzed {
            pending: Some(symbol.clone()),
            ..Analyzed::default()
        };
        assert!(matches!(opening.showing(), Showing::Nothing));

        // The same wait, once it has gone on long enough to name.
        let slow = Analyzed {
            slow: true,
            ..opening.clone()
        };
        assert!(matches!(slow.showing(), Showing::Message("Analysing...")));

        // A listing in hand is drawn, and goes on being drawn while the next one is worked
        // out -- which is what keeps a click from flashing the pane empty.
        let showing = Analyzed {
            shown: Some(studied),
            ..idle
        };
        assert!(matches!(showing.showing(), Showing::Listing(_)));
        let replacing = Analyzed {
            pending: Some(symbol),
            ..showing
        };
        assert!(matches!(replacing.showing(), Showing::Listing(_)));
        // Until the wait is worth naming, and then the stale listing gives way to it.
        let dragging = Analyzed {
            slow: true,
            ..replacing
        };
        assert!(matches!(
            dragging.showing(),
            Showing::Message("Analysing...")
        ));
    }

    /// A component with no props at all, which is what every view in this file is: the
    /// six dock tabs, every row of every list. Its parent reads nothing coloured, so
    /// freya has no reason to re-render it -- the theme has to reach it on its own.
    #[derive(PartialEq)]
    struct ThemedRow;

    impl Component for ThemedRow {
        fn render(&self) -> impl IntoElement {
            rect().expanded().background(palette().pane_bg)
        }
    }

    fn theme_harness() -> impl IntoElement {
        rect().expanded().child(ThemedRow)
    }

    /// The same row, under the wiring that resolves the theme -- with the choice handed in
    /// rather than loaded, so that the settings file on the machine running the tests has
    /// no vote in what they assert.
    ///
    /// The root reads the appearance as well, which is not decoration: `app()` does the
    /// same (twice, for freya's own theme sheet), so the write `use_theme` makes during the
    /// render body wakes the very scope that made it. That settles only because the write
    /// is idempotent, and a test that hangs here is what would say it is not.
    fn desktop_theme_harness() -> impl IntoElement {
        use_theme(ThemeChoice::Desktop);
        let _ = appearance();

        rect().expanded().child(ThemedRow)
    }

    /// The first background anything paints, which is the row's: the harness's own rect
    /// has none, and a transparent background is what "none" is.
    fn painted(test: &TestingRunner) -> Fill {
        test.find(|_, element| {
            let background = element.style().background.clone();
            (background != Fill::Color(Color::TRANSPARENT)).then_some(background)
        })
        .expect("a painted row")
    }

    /// `HIGHLIGHTED` is process-wide while the appearance is per-thread, so the two tests
    /// that switch themes have to be the only one doing it at a time -- cargo runs them on
    /// threads of their own, and one clearing the cache the other has just filled would be
    /// a failure that comes and goes.
    static SWITCHING: Mutex<()> = Mutex::new(());

    /// The reactivity half of dark mode: a switch repaints a component that did not change
    /// and whose parent did not either.
    ///
    /// This is the assertion the design is for. Nothing about `ThemedRow` differs across
    /// the switch -- same type, same (absent) props, same parent element -- so freya will
    /// not re-render it for any reason except that it read the state that changed. Asking
    /// for a colour is that read.
    #[test]
    fn a_theme_switch_repaints_a_component_nothing_else_woke() {
        let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
        set_appearance(Appearance::Light);

        let (mut test, ()) = TestingRunner::new(theme_harness, (100., 100.).into(), |_| (), 1.);
        test.sync_and_update();

        assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));

        set_appearance(Appearance::Dark);
        test.sync_and_update();
        assert_eq!(painted(&test), Fill::Color(Palette::DARK.pane_bg));

        // And back, so the thread is left as it was found.
        set_appearance(Appearance::Light);
        test.sync_and_update();
        assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));
    }

    /// The other half: the source pane's spans are cached with the palette resolved into
    /// them, so a switch has to throw the cache away and parse again in the new colours.
    /// Nothing re-renders a `SyntaxBlocks`, which is why this cannot be left to the
    /// reactivity above.
    #[test]
    fn a_theme_switch_empties_the_highlighted_cache() {
        let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
        set_appearance(Appearance::Light);

        let directory =
            std::env::temp_dir().join(format!("assembly-viewer-theme-test-{}", std::process::id()));
        let path = directory.join("themed.rs");
        std::fs::create_dir_all(&directory).expect("creating the test directory");
        std::fs::write(&path, b"fn main() {}\n").expect("writing the source file");

        // A keyword, which is the one span whose colour is a palette entry rather than the
        // text colour -- and the reason this is a `.rs` file and not any file at all.
        let keyword = |path: &Path| {
            let text = source_text(path).expect("the file");
            let line = text.0.blocks.get_line(0);
            line.first().expect("a first span").0
        };

        assert_eq!(keyword(&path), Palette::LIGHT.keyword_fg);
        assert!(!highlighted().is_empty());

        set_appearance(Appearance::Dark);
        assert!(
            highlighted().is_empty(),
            "the switch left the old theme's spans behind"
        );
        assert_eq!(keyword(&path), Palette::DARK.keyword_fg);

        set_appearance(Appearance::Light);
        highlighted().clear();
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The rule the two enums exist to express: a named theme is its own answer, and
    /// `Desktop` is the only one of the three that asks the window anything.
    ///
    /// Pure, so the whole matrix is six lines and needs no window -- which is why
    /// `resolve_appearance` takes the platform's answer as an argument instead of reading
    /// it. What replaced the old subprocess is not testable at all on the machine running
    /// this; the rule in front of it is entirely.
    #[test]
    fn only_following_the_desktop_asks_the_desktop() {
        for preferred in [PreferredTheme::Light, PreferredTheme::Dark] {
            assert_eq!(
                resolve_appearance(ThemeChoice::Light, preferred),
                Appearance::Light
            );
            assert_eq!(
                resolve_appearance(ThemeChoice::Dark, preferred),
                Appearance::Dark
            );
        }

        assert_eq!(
            resolve_appearance(ThemeChoice::Desktop, PreferredTheme::Light),
            Appearance::Light
        );
        assert_eq!(
            resolve_appearance(ThemeChoice::Desktop, PreferredTheme::Dark),
            Appearance::Dark
        );
    }

    /// The half of dark mode that the subprocess could never have: the windowing system
    /// changing its mind about the theme, *after* the window is open, repaints it.
    ///
    /// freya keeps `Platform::preferred_theme` from winit's `Window::theme()` and re-sets
    /// it on the OS's `ThemeChanged` event, so setting it here is exactly what that event
    /// does -- and what this asserts is the path from there to `set_appearance` and out to
    /// a component that reads no props and was woken by nothing else.
    #[test]
    fn a_desktop_that_changes_its_mind_repaints_the_window() {
        let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
        // Left on the wrong one on purpose, so that the mount below has to be a real write
        // rather than a value that happened to already be there.
        set_appearance(Appearance::Dark);

        // `provide_root_context` runs its closure in the root scope, where freya-testing
        // has already put the `Platform` -- so this is how a test gets hold of the states
        // a renderer would otherwise be the only writer of.
        let (mut test, platform) = TestingRunner::new(
            desktop_theme_harness,
            (100., 100.).into(),
            |runner| runner.provide_root_context(Platform::get),
            1.,
        );
        test.sync_and_update();

        // freya-testing mounts on `PreferredTheme::Light`, and the choice is a question,
        // so the answer arrived on the first render: the appearance the thread was left in
        // is gone, and nothing had to be set by hand to do it.
        assert_eq!(appearance(), Appearance::Light);
        assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));

        // **Two passes, and the second is not padding.** The change reaches the window in
        // two hops -- the platform state wakes the scope holding `use_theme`, and the write
        // that scope makes wakes everything that drew a colour -- and a pass renders the
        // dirty scopes it *began* with, so the second hop lands in the pass after the
        // first. The renderer does the same thing on its own (a marked scope sends a
        // message that brings its loop straight back round and requests a redraw), so the
        // cost of resolving the theme in the render body rather than an effect is one
        // frame, spelled out here rather than hidden behind a loop that polls until it
        // likes the answer.
        let mut preferred = platform.preferred_theme;
        preferred.set(PreferredTheme::Dark);
        test.sync_and_update();
        assert_eq!(appearance(), Appearance::Dark);
        test.sync_and_update();

        assert_eq!(painted(&test), Fill::Color(Palette::DARK.pane_bg));

        // And back again, both to prove the wire runs in both directions and to leave the
        // thread as it was found.
        preferred.set(PreferredTheme::Light);
        test.sync_and_update();
        test.sync_and_update();
        assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));
    }

    /// sRGB relative luminance, and the contrast ratio between two colours, both as WCAG
    /// defines them. Written out rather than pulled in: it is eight lines, and a
    /// dependency for eight lines used by two tests is not a trade.
    fn luminance(color: Color) -> f32 {
        let channel = |value: u8| {
            let value = value as f32 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };

        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Every foreground is legible on the surface it is actually drawn on, in both
    /// palettes.
    ///
    /// The floor is 3.0 and not WCAG AA's 4.5 on purpose. Two of the light palette's own
    /// colours sit between 3 and 3.5 -- the address column and comments, both of which are
    /// *meant* to recede -- and this test is not here to redesign the light theme that has
    /// been on screen since 5e. It is here so that a value carried over to a dark ground
    /// cannot land on top of it: a foreground that came out at 1.5 would be a colour
    /// nobody can read, and that is what a second palette gets wrong.
    #[test]
    fn every_foreground_is_legible_on_its_own_surface() {
        for (theme, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
            // The code colours, on the pane each is drawn on: the assembly pane has no
            // comments and no strings, and the source pane is the plain one.
            let both = [
                ("address_fg", palette.address_fg),
                ("keyword_fg", palette.keyword_fg),
                ("operand_fg", palette.operand_fg),
                ("literal_fg", palette.literal_fg),
                ("punctuation_fg", palette.punctuation_fg),
                ("name_fg", palette.name_fg),
                ("name_hover_fg", palette.name_hover_fg),
            ];
            for (name, color) in both {
                for (surface, background) in [
                    ("asm_pane_bg", palette.asm_pane_bg),
                    ("pane_bg", palette.pane_bg),
                ] {
                    let ratio = contrast(color, background);
                    assert!(ratio >= 3.0, "{theme} {name} on {surface}: {ratio:.2}");
                }
            }

            for (name, color) in [
                ("string_fg", palette.string_fg),
                ("comment_fg", palette.comment_fg),
            ] {
                let ratio = contrast(color, palette.pane_bg);
                assert!(ratio >= 3.0, "{theme} {name} on pane_bg: {ratio:.2}");
            }

            // The chrome, on all three of the surfaces it is written over.
            for (name, color) in [
                ("text_fg", palette.text_fg),
                ("icon_fg", palette.icon_fg),
                ("invalid_fg", palette.invalid_fg),
            ] {
                for (surface, background) in [
                    ("pane_bg", palette.pane_bg),
                    ("header_bg", palette.header_bg),
                    ("symbol_pane_bg", palette.symbol_pane_bg),
                ] {
                    let ratio = contrast(color, background);
                    assert!(ratio >= 3.0, "{theme} {name} on {surface}: {ratio:.2}");
                }
            }

            // The branch gutter is a diagram and is drawn quiet deliberately -- 1.8 in the
            // light palette -- so its floor is only against a line that has disappeared
            // into the pane altogether, and the hovered one has to be the louder of the
            // two or hovering a row says nothing.
            let line = contrast(palette.branch_fg, palette.asm_pane_bg);
            let lit = contrast(palette.branch_hover_fg, palette.asm_pane_bg);
            assert!(line >= 1.5, "{theme} branch_fg: {line:.2}");
            assert!(lit > line, "{theme} branch_hover_fg: {lit:.2} vs {line:.2}");
        }
    }

    /// Every translucent wash still says something once it is composited.
    ///
    /// This is the half of a palette that cannot be carried over by turning its channels
    /// through the background: `blend` puts the pane under these, so the same alpha over a
    /// dark ground is a fraction of the step it was over white. Each is asserted as what
    /// it comes out as -- and the pin, which is the focus said louder, has to stay louder.
    #[test]
    fn every_wash_reads_against_the_pane_under_it() {
        // How far a wash moves the surface it is over, in the channel it moves most.
        let step = |wash: Color, ground: Color| {
            let over = blend(wash, ground);
            let channel = |top: u8, bottom: u8| (top as i32 - bottom as i32).unsigned_abs();
            channel(over.r(), ground.r())
                .max(channel(over.g(), ground.g()))
                .max(channel(over.b(), ground.b()))
        };

        for (theme, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
            for (name, wash, ground) in [
                (
                    "code_row_hover_bg",
                    palette.code_row_hover_bg,
                    palette.asm_pane_bg,
                ),
                ("line_focus_bg", palette.line_focus_bg, palette.asm_pane_bg),
                ("line_pin_bg", palette.line_pin_bg, palette.asm_pane_bg),
                ("row_select_bg", palette.row_select_bg, palette.asm_pane_bg),
                ("drop_preview_bg", palette.drop_preview_bg, palette.pane_bg),
            ] {
                let step = step(wash, ground);
                assert!(step >= 10, "{theme} {name}: {step} levels");
            }

            let focus = step(palette.line_focus_bg, palette.asm_pane_bg);
            let pin = step(palette.line_pin_bg, palette.asm_pane_bg);
            assert!(pin > focus, "{theme} pin {pin} vs focus {focus}");
        }
    }

    /// The `resolve_capture_color` trap, in both palettes.
    ///
    /// It decides a capture is unmapped by comparing its colour to `text` and then walks
    /// *up* the dotted name, so a child field holding the text colour while its parent
    /// holds another is silently painted in the parent's. Nothing in either mapping is
    /// caught by it -- but that is a fact about which fields share a value, so a second
    /// palette can break it by landing two colours on each other by accident.
    #[test]
    fn captures_do_not_walk_up() {
        for (name, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
            let theme = palette.syntax();
            let dotted = [
                ("function.macro", theme.function_macro, theme.function),
                ("function.method", theme.function_method, theme.function),
                (
                    "punctuation.bracket",
                    theme.punctuation_bracket,
                    theme.punctuation,
                ),
                (
                    "punctuation.delimiter",
                    theme.punctuation_delimiter,
                    theme.punctuation,
                ),
                (
                    "punctuation.special",
                    theme.punctuation_special,
                    theme.punctuation,
                ),
                ("string.escape", theme.string_escape, theme.string),
                ("string.special", theme.string_special, theme.string),
                // A `text.*` capture's parent is `text` itself, which `capture_color`
                // answers for with the text colour, so these can only ever agree.
                ("text.literal", theme.text_literal, theme.text),
                ("text.reference", theme.text_reference, theme.text),
                ("text.title", theme.text_title, theme.text),
                ("text.uri", theme.text_uri, theme.text),
                ("text.emphasis", theme.text_emphasis, theme.text),
                ("variable.builtin", theme.variable_builtin, theme.variable),
                (
                    "variable.parameter",
                    theme.variable_parameter,
                    theme.variable,
                ),
            ];

            for (capture, child, parent) in dotted {
                assert!(
                    child != theme.text || parent == theme.text,
                    "{name}: {capture} takes the text colour while its parent does not, \
                     so it would be painted in the parent's",
                );
            }
        }
    }

    /// A `Fonts` with nothing left to ask the desktop about, so a test asserting a size
    /// asserts a size and not whatever `kreadconfig` happens to answer on the machine
    /// running it -- `needs_desktop` declines to spawn anything when both halves are
    /// chosen, which is exactly the case here.
    fn fixed_fonts(ui: f32, mono: f32) -> Fonts {
        fonts::resolve(&Settings {
            theme: ThemeChoice::Desktop,
            interface: FontSetting {
                family: Some("Interface".to_owned()),
                size: Some(ui),
            },
            fixed: FontSetting {
                family: Some("Fixed".to_owned()),
                size: Some(mono),
            },
        })
    }

    /// The same pair as an [`EditedSettings`], which is what the page holds.
    fn fixed_edited(ui: f32, mono: f32) -> EditedSettings {
        EditedSettings {
            theme: ThemeChoice::Desktop,
            interface: EditedFont {
                family: "Interface".to_owned(),
                size: Some(ui),
            },
            fixed: EditedFont {
                family: "Fixed".to_owned(),
                size: Some(mono),
            },
        }
    }

    /// Two components with no props at all, one row at each of the two heights.
    /// `ThemedRow`'s twins, and for the same reason: nothing about either changes across a
    /// font change, so freya has no reason to re-render them except that they read the
    /// state. Their backgrounds differ so that `painted_height` can ask for one of them by
    /// name rather than by which came first.
    #[derive(PartialEq)]
    struct FontedRow;

    impl Component for FontedRow {
        fn render(&self) -> impl IntoElement {
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .background(palette().pane_bg)
        }
    }

    #[derive(PartialEq)]
    struct FontedCodeRow;

    impl Component for FontedCodeRow {
        fn render(&self) -> impl IntoElement {
            rect()
                .width(Size::fill())
                .height(Size::px(code_row_height()))
                .background(palette().asm_pane_bg)
        }
    }

    fn font_harness() -> impl IntoElement {
        rect().expanded().child(FontedRow).child(FontedCodeRow)
    }

    /// The height of the row painted in `fill`, as it was actually laid out -- not as it
    /// was asked for. That distinction is the test: a row height function returning a new
    /// number proves nothing on its own, since a component that was never re-rendered is
    /// still the old height on screen.
    fn painted_height(test: &TestingRunner, fill: Color) -> f32 {
        test.find(|node, element| {
            let background = element.style().background.clone();
            (background == Fill::Color(fill)).then(|| node.layout().area.height())
        })
        .expect("a painted row")
    }

    /// The reactivity half of 9c, and the direct analogue of the theme's: a font change
    /// repaints a component nothing else woke, *and* moves it, since the row heights are
    /// derived from the fonts rather than being constants beside them.
    ///
    /// It is also where the two heights are asserted to be **independent**, which is the
    /// whole of the split: no row mixes the fonts, so a size the reader steps must move
    /// the rows drawn in *that* font and no others. 9pt and 10.5pt are the app's own
    /// defaults -- 12 and 14 logical pixels, so 24 and 26 -- and each of the two changes
    /// below leaves the other row exactly where it was.
    #[test]
    fn a_font_change_repaints_and_resizes_a_component_nothing_else_woke() {
        set_fonts(fixed_fonts(9.0, 10.5));

        let (mut test, ()) = TestingRunner::new(font_harness, (200., 200.).into(), |_| (), 1.);
        test.sync_and_update();

        let list = palette().pane_bg;
        let code = palette().asm_pane_bg;

        assert_eq!((list_row_height(), code_row_height()), (24.0, 26.0));
        assert_eq!(painted_height(&test, list), 24.0);
        assert_eq!(painted_height(&test, code), 26.0);

        // 18pt is 24 logical pixels, so the code row is 36 -- and the list row is still
        // the 24 it was, the assembly font having nothing to say about it.
        set_fonts(fixed_fonts(9.0, 18.0));
        test.sync_and_update();
        assert_eq!((list_row_height(), code_row_height()), (24.0, 36.0));
        assert_eq!(painted_height(&test, list), 24.0);
        assert_eq!(painted_height(&test, code), 36.0);

        // And the other way: 21pt is 28 pixels, so the list row is 40 and the code row is
        // back to the 26 its own unchanged font asks for.
        set_fonts(fixed_fonts(21.0, 10.5));
        test.sync_and_update();
        assert_eq!((list_row_height(), code_row_height()), (40.0, 26.0));
        assert_eq!(painted_height(&test, list), 40.0);
        assert_eq!(painted_height(&test, code), 26.0);
    }

    /// The invariant that made `ROW_HEIGHT` a `const` in the first place: a
    /// `VirtualScrollView`'s `item_size` and the height its rows actually draw at must be
    /// the same number, or scrolling misaligns -- silently, and looking like a rendering
    /// glitch rather than a bug.
    ///
    /// **It is two claims since the height was split in two**, so it is asserted over both
    /// kinds of list: a code pane, whose rows and `item_size` are [`code_row_height`] and
    /// which is the only kind with a kept position, and a sidebar list at
    /// [`list_row_height`]. A view handed the *other* height would misalign exactly as one
    /// handed a stale one would, and only a view of each kind can catch that.
    ///
    /// Asserted through real scroll views, by asking which row is under a given y: at the
    /// top of the list row *k* covers `[k*h, (k+1)*h)`, so a pointer at 90 is row 3 at 26px
    /// and row 2 at 36px. If the two numbers came apart, the rows would drift by one per
    /// row down the pane and this would answer something else. Each half also steps the
    /// font it is *not* drawn in and asserts that nothing moved.
    #[test]
    fn a_scroll_view_and_its_rows_agree_at_every_font_size() {
        set_fonts(fixed_fonts(9.0, 10.5));

        // Away and back, or entering the same row twice is no event at all.
        fn row_under(test: &mut TestingRunner, top: State<usize>, y: f64) -> usize {
            test.move_cursor((50., 5.));
            test.sync_and_update();
            test.move_cursor((50., y));
            test.sync_and_update();
            *top.peek()
        }

        // A font change wakes the rows through the state they read, and the view they sit
        // in re-measures behind them; several passes because the scroll view answers the
        // new item size on the render after the one that moved its rows.
        fn settle(test: &mut TestingRunner) {
            for _ in 0..4 {
                test.sync_and_update();
            }
        }

        {
            let (mut test, top) = TestingRunner::new(
                scrolling_harness,
                (200., 200.).into(),
                |runner| {
                    let mut tabs = Tabs::default();
                    tabs.open("a".to_owned());
                    runner.provide_root_context(|| KeptTab(State::create("a".to_owned())));
                    runner.provide_root_context(|| KeptAt(State::create(Positions::default())));
                    runner.provide_root_context(|| KeptOpen(State::create(tabs)));
                    runner.provide_root_context(|| KeptLength(State::create(100)));
                    runner.provide_root_context(|| KeptTop(State::create(0))).0
                },
                1.,
            );
            test.sync_and_update();

            assert_eq!(code_row_height(), 26.0);
            assert_eq!(row_under(&mut test, top, 90.), 3);

            // The interface font is not this pane's font, so stepping it moves nothing.
            set_fonts(fixed_fonts(21.0, 10.5));
            settle(&mut test);
            assert_eq!(code_row_height(), 26.0);
            assert_eq!(row_under(&mut test, top, 90.), 3);

            set_fonts(fixed_fonts(9.0, 18.0));
            settle(&mut test);
            assert_eq!(code_row_height(), 36.0);
            assert_eq!(row_under(&mut test, top, 90.), 2);
        }

        set_fonts(fixed_fonts(9.0, 10.5));

        {
            let (mut test, top) = TestingRunner::new(
                list_scrolling_harness,
                (200., 200.).into(),
                |runner| runner.provide_root_context(|| KeptTop(State::create(0))).0,
                1.,
            );
            test.sync_and_update();

            // 24 rather than 26: a sidebar row is the interface font's 12 pixels plus the
            // leading, and 90 is three of them down.
            assert_eq!(list_row_height(), 24.0);
            assert_eq!(row_under(&mut test, top, 90.), 3);

            // And the fixed-width font is not this list's font.
            set_fonts(fixed_fonts(9.0, 18.0));
            settle(&mut test);
            assert_eq!(list_row_height(), 24.0);
            assert_eq!(row_under(&mut test, top, 90.), 3);

            set_fonts(fixed_fonts(21.0, 10.5));
            settle(&mut test);
            assert_eq!(list_row_height(), 40.0);
            assert_eq!(row_under(&mut test, top, 90.), 2);
        }
    }

    /// What the settings page's four boxes mean, which is the one place its `String`s and
    /// `settings.toml`'s absent keys meet -- `an_empty_box_is_a_project_that_has_not_been_named`
    /// for fonts. An empty family box is not a font family named the empty string: it is a
    /// reader who has not chosen one, which is what unspecified *is*.
    #[test]
    fn an_empty_box_is_a_font_nobody_chose() {
        assert_eq!(EditedSettings::default().settings(), Settings::default());

        let blank = EditedFont {
            family: "   ".to_owned(),
            size: None,
        };
        assert_eq!(blank.setting(), FontSetting::default());

        // And a round trip through the two spellings changes nothing, in either direction:
        // the page is handed what the file says and hands back the same thing.
        let stored = Settings {
            theme: ThemeChoice::Dark,
            interface: FontSetting {
                family: Some("Cantarell".to_owned()),
                size: Some(11.0),
            },
            fixed: FontSetting {
                family: None,
                size: Some(10.5),
            },
        };
        assert_eq!(EditedSettings::of(&stored).settings(), stored);

        // A family the file wrote with spaces around it comes back trimmed, once, and does
        // not then differ from itself on the way out.
        let padded = Settings {
            interface: FontSetting {
                family: Some(" Fira Code ".to_owned()),
                ..FontSetting::default()
            },
            ..Settings::default()
        };
        let edited = EditedSettings::of(&padded);
        assert_eq!(edited.interface.family, "Fira Code");
        assert_eq!(edited.settings(), edited.settings());
    }

    /// A point size as the page writes it. `9` and not `9.0`, because the size a desktop
    /// answers is usually a whole number and a trailing `.0` on every one of them reads as
    /// precision that is not there; `10.5` because half-points are what the stepper moves
    /// in and what Pango descriptions carry.
    #[test]
    fn a_point_size_is_written_as_short_as_it_is() {
        assert_eq!(points_text(9.0), "9");
        assert_eq!(points_text(10.5), "10.5");
        assert_eq!(points_text(26.0), "26");
        // Gnome's `text-scaling-factor` multiplies the point size, so a third decimal is
        // reachable without anybody typing one.
        assert_eq!(points_text(13.75), "13.8");
    }

    /// Everything the settings write, recorded rather than performed.
    #[derive(Clone, Copy)]
    struct Saved(State<Vec<Settings>>);

    fn settings_harness() -> impl IntoElement {
        let prefs = use_consume::<Prefs>().0;
        let mut saved = use_consume::<Saved>().0;

        use_settings_with(prefs, move |settings: &Settings| {
            saved.write().push(settings.clone())
        });

        // The **code** row, because what this test steps is the fixed-width size: it is
        // the one whose consequences reach a file, a theme and a row all at once, and a
        // row drawn in the other font would now sit still through the whole of it.
        rect().expanded().child(FontedCodeRow)
    }

    /// The wiring 9c is: one state, and the theme, the fonts and the file all following
    /// from it -- with the write handed in, because the real one edits the settings of
    /// whoever runs the tests.
    ///
    /// Three things are asserted that nothing else can say. That a run in which the page is
    /// never opened writes **nothing**, so a first launch leaves no `settings.toml` behind.
    /// That a change reaches all three consequences from the one write. And that changing a
    /// setting *back* writes again -- the baseline moves to what was last written, or the
    /// file would be left holding the middle answer of the three.
    #[test]
    fn the_settings_reach_the_theme_the_fonts_and_the_file() {
        let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
        // Both left on the wrong answer on purpose, so that arriving at the right one has
        // to be a real write rather than a value that happened to already be there.
        set_appearance(Appearance::Dark);
        set_fonts(fixed_fonts(21.0, 21.0));

        let (mut test, (prefs, saved)) = TestingRunner::new(
            settings_harness,
            (200., 200.).into(),
            |runner| {
                (
                    runner
                        .provide_root_context(|| Prefs(State::create(fixed_edited(9.0, 10.5))))
                        .0,
                    runner
                        .provide_root_context(|| Saved(State::create(Vec::new())))
                        .0,
                )
            },
            1.,
        );
        let mut prefs = prefs;
        for _ in 0..4 {
            test.sync_and_update();
        }

        // Mounting is not a change: the settings were read off disk, and writing them
        // straight back would create the file on a launch where the reader did nothing.
        assert!(
            saved.peek().is_empty(),
            "a run that changed nothing wrote the settings file"
        );
        // But the app is drawn in them: the choice is `Desktop` and freya-testing mounts on
        // `PreferredTheme::Light`, and the fonts are the ones the state holds.
        assert_eq!(appearance(), Appearance::Light);
        assert_eq!(fonts().mono.points, 10.5);
        assert_eq!(painted_height(&test, palette().asm_pane_bg), 26.0);

        // A theme chosen. Two passes, for `a_desktop_that_changes_its_mind_repaints_the_window`'s
        // reason: the write the root makes wakes the scopes that drew a colour in the pass
        // after the one it was made in.
        prefs.write().theme = ThemeChoice::Dark;
        test.sync_and_update();
        assert_eq!(appearance(), Appearance::Dark);
        for _ in 0..4 {
            test.sync_and_update();
        }
        assert_eq!(saved.peek().len(), 1);
        assert_eq!(saved.peek()[0].theme, ThemeChoice::Dark);

        // A size chosen: the fonts follow, and the rows with them.
        prefs.write().fixed.size = Some(18.0);
        for _ in 0..4 {
            test.sync_and_update();
        }
        assert_eq!(fonts().mono.points, 18.0);
        assert_eq!(painted_height(&test, palette().asm_pane_bg), 36.0);
        assert_eq!(saved.peek().len(), 2);
        assert_eq!(saved.peek()[1].fixed.size, Some(18.0));

        // Cleared again, which is the whole of "a way back to unspecified": the override is
        // gone from the file, and the write happens even though this is the value the run
        // started from -- the baseline is what was last *written*, not what was loaded.
        prefs.write().fixed.size = Some(10.5);
        for _ in 0..4 {
            test.sync_and_update();
        }
        assert_eq!(saved.peek().len(), 3);
        assert_eq!(saved.peek()[2].fixed.size, Some(10.5));
        assert_eq!(painted_height(&test, palette().asm_pane_bg), 26.0);

        // And the thread is left as it was found.
        set_appearance(Appearance::Light);
    }

    #[test]
    fn a_swept_run_survives_the_button_coming_up() {
        let (mut test, marked) = TestingRunner::new(
            harness,
            (100., 100.).into(),
            |runner| {
                runner.provide_root_context(|| Shift(State::create(false)));
                runner
                    .provide_root_context(|| Marked(State::create(None)))
                    .0
            },
            1.,
        );
        test.sync_and_update();

        test.press_cursor((10., 10.));
        test.move_cursor((10., 30.));
        test.sync_and_update();
        assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);

        // The line that panicked, and the assertion that it no longer does is the test
        // getting this far at all.
        test.release_cursor((10., 30.));
        assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);

        // And the gesture really is over: a row entered afterwards is the pointer passing
        // over it, which is the panes' hover and not a sweep.
        test.move_cursor((10., 50.));
        test.sync_and_update();
        assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);
    }

    /// The scratchpad worker's work, handed in through a context so a test can answer
    /// without the machine's own state directory and without waiting on a compiler.
    /// `Arc<dyn Fn>` and not a generic, for [`Study`]'s reason: a context value is one
    /// concrete type.
    #[derive(Clone)]
    struct Working(Arc<dyn Fn(PadJob) -> PadAnswer + Send + Sync>);

    /// The way to ask the worker for a build, as the wiring hands it back. A `State` so
    /// that the harness can put it somewhere the test body can reach, which is what lets
    /// a build be asked for the way the button asks rather than through coordinates.
    #[derive(Clone, Copy)]
    struct Asking(State<Option<PadJobs>>);

    /// What the worker was handed, in the order it was handed it. The `Save`s carry the
    /// source they would have written, because *what* was written is half of what the
    /// save policy is for.
    #[derive(Clone, Debug, PartialEq)]
    enum Asked {
        Open,
        Save(String),
        Build(String),
        Run,
    }

    /// The scratchpad wiring and nothing else: no pane, since what is under test is which
    /// jobs the worker is handed and what its answers do to the app.
    fn scratchpad_harness() -> impl IntoElement {
        scratchpad_wiring();

        rect().expanded()
    }

    /// The same wiring under the real pane, for the one thing only the pane can be asked:
    /// whether its rows survive one of them being taken away.
    fn scratchpad_view_harness() -> impl IntoElement {
        scratchpad_wiring();

        rect().expanded().child(ScratchpadTab)
    }

    fn scratchpad_wiring() {
        let pad = use_consume::<Pad>().0;
        let text = use_consume::<PadText>().0;
        let work = use_consume::<Working>().0;
        let mut asking = use_consume::<Asking>().0;
        let states = use_project_states();

        let jobs = use_scratchpad_with(pad, text, states, move |job| work(job));
        use_hook(move || asking.set(Some(jobs)));
    }

    /// The committed gcc fixture again, standing in for what a build produced: `open_files`
    /// asks nothing of a file but that it parse, so a relocatable object is an artifact as
    /// far as everything this test is about is concerned.
    fn fixture_artifact() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o")
    }

    /// Mount the wiring over a worker that records every job and answers from `answer`.
    ///
    /// A macro rather than a function for `project_states!`'s reason -- the runner's type
    /// is not one this crate can name -- and it hands back everything a test then drives:
    /// the app's states, the scratchpad's two, the way to ask for a build and the record
    /// of what was asked.
    macro_rules! mount_scratchpad {
        ($harness:expr, $answer:expr) => {{
            let (asked, asks) = async_channel::unbounded::<Asked>();
            let answer = $answer;
            let work = move |job: PadJob| {
                let recorded = match &job {
                    PadJob::Open(_) => Asked::Open,
                    PadJob::Save(scratchpad) => Asked::Save(scratchpad.source.clone()),
                    PadJob::Build(scratchpad) => Asked::Build(scratchpad.source.clone()),
                    PadJob::Run { .. } => Asked::Run,
                };
                let _ = asked.send_blocking(recorded);
                answer(job)
            };

            let (mut test, (states, pad, text, asking)) = TestingRunner::new(
                $harness,
                (400., 400.).into(),
                move |runner: &mut _| {
                    let states = project_states!(runner);
                    runner.provide_root_context(move || Working(Arc::new(work)));
                    let pad = runner
                        .provide_root_context(|| Pad(State::create(PadState::default())))
                        .0;
                    let text = runner
                        .provide_root_context(|| {
                            PadText(State::create(CodeEditorData::new(
                                Rope::from_str(""),
                                language(Path::new(SOURCE_FILE)),
                            )))
                        })
                        .0;
                    let asking = runner
                        .provide_root_context(|| Asking(State::create(None)))
                        .0;

                    (states, pad, text, asking)
                },
                1.,
            );
            test.sync_and_update();

            (test, states, pad, text, asking, asks)
        }};
    }

    /// The scratchpad on disk is what the app opens on, and **nothing is written until it
    /// has arrived**.
    ///
    /// That second half is the whole reason the save baseline is seeded by the answer and
    /// not at mount: the app boots holding `Scratchpad::default`, the reader's own source
    /// comes back a worker thread later, and a save in between would put the default
    /// source over a scratchpad someone had been keeping. It is also what keeps a run in
    /// which the pane was never opened from creating the directory at all.
    #[test]
    fn a_scratchpad_is_read_before_anything_is_written_over_it() {
        let mut saved = Scratchpad::default();
        saved.source = "fn kept() {}\n".to_owned();
        saved.dependencies = vec![Dependency {
            name: "anyhow".to_owned(),
            version: "1.0.86".to_owned(),
        }];

        let answering = saved.clone();
        let (mut test, _states, pad, text, _asking, asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(_) => PadAnswer::Opened(answering.clone()),
                PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
                PadJob::Build(_) => unreachable!("this test never builds"),
                PadJob::Run { .. } => unreachable!("this test never runs"),
            });

        pump(&mut test, || pad.peek().opened);

        assert_eq!(pad.peek().scratchpad, saved);
        // The editor is holding it too, which is the half a reader can see: the buffer is
        // the live copy and the model follows it, so a restore that reached only the model
        // would be a pane showing the default source over a scratchpad that is not it.
        assert_eq!(text.peek().rope.to_string(), saved.source);

        assert_eq!(asks.try_recv(), Ok(Asked::Open));
        assert!(
            asks.is_empty(),
            "the package was written before the app knew what was in it"
        );
    }

    /// An edit is written out, and a row that cannot be written says so against itself.
    ///
    /// Both halves go through the one policy: the model follows the editor, the effect
    /// notices it differs from what was last sent, and the worker answers. What comes back
    /// for a bad row is `Failure::Dependencies`, carrying the **index** of every row that
    /// is wrong -- which is what lets the pane mark them in place rather than printing one
    /// sentence at the top.
    #[test]
    fn an_edit_is_written_and_a_bad_row_says_which_row() {
        let (mut test, _states, pad, text, _asking, asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                // The real refusal, without a disk: `write` fails on exactly what
                // `manifest` fails on, the manifest being what it refuses to generate.
                PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
                PadJob::Build(_) => unreachable!("this test never builds"),
                PadJob::Run { .. } => unreachable!("this test never runs"),
            });

        pump(&mut test, || pad.peek().opened);
        assert_eq!(asks.try_recv(), Ok(Asked::Open));

        // Typing. The rope is what the keyboard edits and the model is what is written, so
        // this is the same path a keystroke takes.
        let mut text = text;
        text.write().rope.insert(0, "// typed\n");
        pump(&mut test, || !asks.is_empty());

        let typed = format!("// typed\n{}", crate::scratchpad::DEFAULT_SOURCE);
        assert_eq!(asks.try_recv(), Ok(Asked::Save(typed.clone())));
        assert_eq!(pad.peek().scratchpad.source, typed);
        assert!(pad.peek().unsaved.is_none());

        // A row that names no crate. It is the *second* row, so the index in the answer is
        // the assertion: a failure that only said "one dependency to fix" would leave the
        // pane guessing which.
        let mut pad = pad;
        {
            let mut state = pad.write();
            state.scratchpad.dependencies = vec![
                Dependency {
                    name: "anyhow".to_owned(),
                    version: "1.0.86".to_owned(),
                },
                Dependency::default(),
            ];
        }
        pump(&mut test, || pad.peek().unsaved.is_some());

        assert_eq!(
            pad.peek().unsaved,
            Some(Failure::Dependencies(vec![(1, Problem::NoName)]))
        );

        // And fixing it writes again, rather than leaving the disk holding the last good
        // version for ever.
        pad.write().scratchpad.dependencies[1] = Dependency {
            name: "rand".to_owned(),
            version: "0.8".to_owned(),
        };
        pump(&mut test, || pad.peek().unsaved.is_none());
    }

    /// A build is asked for once however often the reader presses, and what it made is
    /// opened **in place of** what the build before it made.
    ///
    /// Both halves are about the same thing being true twice. A build takes seconds, so
    /// the pending state has to be honest enough that a second press cannot start a second
    /// one; and a rebuild writes the same path with different bytes, so the objects the app
    /// is holding for that path describe instructions that are no longer there. Opening
    /// without closing would leave two generations of one file in a list where a binary is
    /// identified by its path.
    #[test]
    fn a_build_runs_once_and_replaces_what_the_last_one_opened() {
        let artifact = fixture_artifact();
        let built = artifact.clone();
        let (mut test, states, pad, _text, asking, asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                PadJob::Save(_) => PadAnswer::Saved(None),
                PadJob::Build(_) => PadAnswer::Built(Build::Built {
                    executable: built.clone(),
                    diagnostics: Vec::new(),
                }),
                PadJob::Run { .. } => unreachable!("this test never runs"),
            });

        pump(&mut test, || pad.peek().opened);
        assert_eq!(asks.try_recv(), Ok(Asked::Open));

        let jobs = asking.peek().clone().expect("the wiring handed one back");
        request_build(pad, &jobs);
        // The second press, while the first is still in flight. Nothing at all happens.
        request_build(pad, &jobs);
        assert!(pad.peek().building);

        pump(&mut test, || !states.objects.peek().is_empty());
        assert!(!pad.peek().building);
        assert!(matches!(pad.peek().built, Some(Build::Built { .. })));

        let opened = |states: &ProjectStates| {
            states
                .objects
                .peek()
                .iter()
                .filter(|object| object.path == artifact)
                .count()
        };
        let first = opened(&states);
        assert!(first > 0, "the artifact was never opened");
        assert_eq!(
            asks.try_recv(),
            Ok(Asked::Build(pad.peek().scratchpad.source.clone()))
        );
        assert!(
            asks.is_empty(),
            "the second press started a second build of the same scratchpad"
        );

        // And again. The path is the same one, so what the first build left has to go
        // rather than sit beside it.
        request_build(pad, &jobs);
        // Waited for on the *objects* and not on the build, because a rebuild is now a
        // close followed by a streaming reopen: the build is over the moment cargo has
        // answered, and the artifact's objects come back over the load after it.
        pump(&mut test, || !pad.peek().building && opened(&states) > 0);

        assert_eq!(
            opened(&states),
            first,
            "a rebuild left the objects of the build before it in the list"
        );
    }

    /// Taking a dependency row away does not take the pane with it.
    ///
    /// The hazard is the one the gotchas list is about, and it is invisible to every other
    /// kind of test here: each box in a row writes into `dependencies[index]` through a
    /// mapped `Writable`, so a row that outlived the list being shortened would index past
    /// the end at the moment it was next read -- a panic, not a compile error. Mounting the
    /// real pane and shortening the list under it is the only thing that would say so.
    #[test]
    fn removing_a_dependency_row_does_not_take_the_pane_with_it() {
        let (mut test, _states, pad, _text, _asking, _asks) =
            mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
                PadJob::Build(_) => unreachable!("this test never builds"),
                PadJob::Run { .. } => unreachable!("this test never runs"),
            });

        pump(&mut test, || pad.peek().opened);

        let mut pad = pad;
        pad.write().scratchpad.dependencies = vec![
            Dependency {
                name: "anyhow".to_owned(),
                version: "1.0.86".to_owned(),
            },
            Dependency {
                name: "rand".to_owned(),
                version: "0.8".to_owned(),
            },
        ];
        for _ in 0..4 {
            test.sync_and_update();
        }

        // The first row, which is what the × on it does -- so the row left behind is the
        // one that was drawn at index 1.
        pad.write().scratchpad.dependencies.remove(0);
        for _ in 0..4 {
            test.sync_and_update();
        }

        assert_eq!(pad.peek().scratchpad.dependencies.len(), 1);
        assert_eq!(pad.peek().scratchpad.dependencies[0].name(), "rand");
    }

    /// A directory of this test's own, named after the line that asked for it -- the shape
    /// `scratchpad.rs`'s own file tests use, so a failing test leaves something
    /// identifiable behind.
    fn run_directory(line: u32) -> PathBuf {
        std::env::temp_dir().join(format!(
            "assembly-viewer-run-test-{}-{line}",
            std::process::id()
        ))
    }

    /// Build a program that says something and then never exits, and say where it is.
    ///
    /// A real `cargo build`, for `scratchpad.rs`'s reason: it is hermetic (no dependencies
    /// means no registry, so it is one rustc invocation) and it is the only way to have an
    /// executable that behaves the way the hazard this sub-step is about behaves. Nothing
    /// short of a real process can say whether a stop actually killed anything.
    fn looping_program(directory: &Path) -> PathBuf {
        let mut scratchpad = Scratchpad::new("looper").expect("a name");
        scratchpad.source = "fn main() {\n\
             \x20   println!(\"from the program\");\n\
             \x20   loop { std::thread::sleep(std::time::Duration::from_millis(50)); }\n\
             }\n"
        .to_owned();

        let build = scratchpad.build_in(directory);
        let Build::Built { executable, .. } = &build else {
            panic!("a build, got {build:?}");
        };
        executable.clone()
    }

    /// What a build left behind, put where a build would have put it.
    ///
    /// Written into the state rather than answered through `PadJob::Build`, so the
    /// artifact does not go through `reopen_binary` on the way: what that does with a
    /// rebuilt binary is `a_build_runs_once_and_replaces_what_the_last_one_opened`'s
    /// question, and it would cost these tests a parse of a real executable for nothing.
    fn already_built(mut pad: State<PadState>, executable: PathBuf) {
        pad.write().built = Some(Build::Built {
            executable,
            diagnostics: Vec::new(),
        });
    }

    /// The two things 10d exists to make true, and only a real process can say either.
    ///
    /// A program that prints and then loops for ever has **said something**, and it is on
    /// screen while it is still going -- which is the whole difference between this and
    /// `build_in`'s collect-the-output-and-return-it shape, since this program has no exit
    /// for such a shape to answer at. And asking it to stop **really kills it**: the state
    /// reaches `Over(Stopped)` only when the run's own `Ended` event arrives, and that is
    /// emitted after the process has been reaped -- so this waits for the process to be
    /// gone rather than for the button to have been pressed.
    #[test]
    fn a_run_streams_while_it_is_going_and_a_stop_really_ends_it() {
        let directory = run_directory(line!());
        let executable = looping_program(&directory);
        let cwd = directory.clone();

        let (mut test, _states, pad, _text, asking, _asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                PadJob::Save(_) => PadAnswer::Saved(None),
                // Nothing about the run is faked: the real spawn, the real pipes and the
                // real kill, reached through the same job the button sends.
                PadJob::Run {
                    run,
                    executable,
                    emit,
                    ..
                } => PadAnswer::Started(run, crate::scratchpad::run_in(&executable, &cwd, emit)),
                PadJob::Build(_) => unreachable!("this test never builds"),
            });

        pump(&mut test, || pad.peek().opened);
        already_built(pad, executable);
        test.sync_and_update();

        let jobs = asking.peek().clone().expect("the wiring handed one back");
        request_run(pad, &jobs);

        pump(&mut test, || pad.peek().output.len() > 0);
        let state = pad.peek().clone();
        assert_eq!(
            state
                .output
                .line(0)
                .map(|line| (line.stream, line.text.to_string())),
            Some((Stream::Out, "from the program".to_owned()))
        );
        assert!(state.is_running(), "it ended by itself");

        stop_run(pad);
        pump(&mut test, || !pad.peek().is_running());
        let state = pad.peek().clone();
        assert!(
            matches!(state.run_state, RunState::Over(Ended::Stopped)),
            "{:?}",
            state.run_status()
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A rebuild stops the program the last one started.
    ///
    /// cargo is about to write over the executable this process *is*, and `reopen_binary`
    /// is about to close the objects describing those bytes -- so a program left going
    /// across a build would be output arriving into a pane belonging to a build the reader
    /// can no longer see. Asserted through `request_build` rather than through the button,
    /// because the guard belongs to the request for the reason the two-builds-at-once one
    /// does: it has to be a property of asking, not of one control's disabled state.
    #[test]
    fn a_rebuild_stops_the_program_the_last_one_started() {
        let directory = run_directory(line!());
        let executable = looping_program(&directory);
        let cwd = directory.clone();

        let (mut test, _states, pad, _text, asking, _asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                PadJob::Save(_) => PadAnswer::Saved(None),
                PadJob::Run {
                    run,
                    executable,
                    emit,
                    ..
                } => PadAnswer::Started(run, crate::scratchpad::run_in(&executable, &cwd, emit)),
                // What the build itself answers does not matter here: the run is stopped
                // on the way to sending the job, before cargo would have been asked
                // anything at all.
                PadJob::Build(_) => PadAnswer::Built(Build::Unavailable(Failure::NoArtifact)),
            });

        pump(&mut test, || pad.peek().opened);
        already_built(pad, executable);
        test.sync_and_update();

        let jobs = asking.peek().clone().expect("the wiring handed one back");
        request_run(pad, &jobs);
        pump(&mut test, || pad.peek().output.len() > 0);
        assert!(pad.peek().is_running());

        request_build(pad, &jobs);
        pump(&mut test, || !pad.peek().is_running());
        let state = pad.peek().clone();
        assert!(
            matches!(state.run_state, RunState::Over(Ended::Stopped)),
            "{:?}",
            state.run_status()
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A program that will not start is a sentence, not a pane that sits on "Starting..."
    /// for ever. No subprocess: what is under test is that the failure the worker answers
    /// with reaches the line the reader reads.
    #[test]
    fn a_run_that_cannot_start_says_why() {
        let (mut test, _states, pad, _text, asking, _asks) =
            mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
                PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
                PadJob::Save(_) => PadAnswer::Saved(None),
                PadJob::Run { run, .. } => PadAnswer::Started(
                    run,
                    Err(Failure::NoProgram("No such file or directory".to_owned())),
                ),
                PadJob::Build(_) => unreachable!("this test never builds"),
            });

        pump(&mut test, || pad.peek().opened);
        already_built(pad, fixture_artifact());
        test.sync_and_update();

        let jobs = asking.peek().clone().expect("the wiring handed one back");
        request_run(pad, &jobs);
        pump(&mut test, || !pad.peek().is_running());

        let (text, bad) = pad.peek().run_status().expect("a status");
        assert!(text.contains("No such file or directory"), "{text}");
        assert!(bad);
    }
}

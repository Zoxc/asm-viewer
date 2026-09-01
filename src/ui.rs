use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};

use async_io::Timer;
use freya::code_editor::{
    EditorLanguage, EditorSyntaxTheme, Rope, SyntaxBlocks, SyntaxHighlighter, TextNode,
};
use freya::icons::lucide;
use freya::prelude::*;
use rfd::AsyncFileDialog;

use analysis::{open_files, Assembly, Instruction, LineInfo, Object, SpanKind, Symbol, SymbolData};

use crate::filter::{Filter, Matcher};
use crate::fonts::{fonts, Font};
use crate::history::History;
use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
use crate::project::{self, Project, Selection};
use crate::rows::RowSelection;
use crate::source::{self, SourceFile};
use crate::tabs::{Positions, Tabs};
use crate::tree::{format_tag, Expansion, ObjectTree, TreeRow, ARCHIVE_TAG};

/// Height of every row in the object, symbol and instruction lists. This must stay
/// equal to the `item_size` given to each `VirtualScrollView`.
const ROW_HEIGHT: f32 = 26.0;

/// The height of the strip a filter bar's text box sits in. Taller than `ROW_HEIGHT` by
/// the room an `Input`'s border and its own inner margin need; it is a bar and not a row,
/// and nothing lines up with it.
const FILTER_HEIGHT: f32 = 32.0;

/// The side of one of the three square toggle buttons.
const TOGGLE_SIZE: f32 = 22.0;

/// How much bigger than the interface font a tab bar's icon is drawn. A Lucide glyph
/// fills its whole box where a letter fills its x-height, so an icon at exactly the text
/// size reads as the larger of the two; a quarter up from it sits it on the same optical
/// line as the word beside it. It is a multiple and not a pixel count because the
/// interface font is the desktop's (`fonts()`), so an icon that did not follow it would
/// be a postage stamp beside a 20px title, or tower over a 9px one.
const ICON_SCALE: f32 = 1.25;

/// The side of a tab bar icon: the interface font, scaled, and capped so that it is never
/// what decides how tall the bar is -- a `ROW_HEIGHT` strip has to keep a little air above
/// and below whatever the desktop's font size turns out to be.
fn icon_size() -> f32 {
    (fonts().ui.size * ICON_SCALE)
        .round()
        .min(ROW_HEIGHT - 8.0)
}

/// The column a file row's disclosure triangle sits in, and the width every row of the
/// objects tree gives up to it so that the tags below one another line up whether or not
/// the row has a triangle.
const CHEVRON_WIDTH: f32 = 14.0;

/// How far an archive member is indented past the file it belongs to. Past the triangle
/// and into the tag column, so the nesting is legible in a 300px sidebar without the name
/// starting halfway across it.
const TREE_INDENT: f32 = 16.0;

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
/// There is one instance, [`Palette::LIGHT`], and [`palette`] is how anything reaches it.
/// The indirection is more of the point than the struct is: a dark mode is asked for in
/// *this* palette rather than as a second design (`Goals.md`, *UI*), so it is one more
/// `const` beside this one and a `palette()` that picks between them, instead of an edit
/// to every call site that names a colour. What it deliberately is not yet is reactive --
/// see the note on `palette`.
///
/// It is also not freya's own theming (`Theme` / `ColorsSheet` / `define_theme!`).
/// `ColorsSheet` has a fixed set of fields naming none of these roles, a `define_theme!`
/// per row component would be styling machinery for elements nothing outside this file
/// ever styles, and -- the part that settles it -- the half of the palette this step is
/// about cannot be read from the element tree at all: the source pane's colours are baked
/// into a `SyntaxBlocks` by a highlighter that runs when a file is *loaded*, so they have
/// to be plain values available outside any component. See [`Palette::syntax`].
struct Palette {
    // Surfaces and chrome, carried over from the original floem styling.
    /// A pane's own body, and the tab header above the active one, which is white so
    /// that it reads as the top edge of that body rather than as part of the tab bar.
    pane_bg: Color,
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
    /// grey would silently paint brackets grey.
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

/// The colours to draw with.
///
/// A `&'static` rather than anything a component subscribes to, which is what keeps this
/// step to the palette itself. Step 9's dark mode is a second `const` and a `palette()`
/// that chooses, plus the two things a *switch* needs that this does not have: something
/// that re-renders the tree when the choice changes -- freya re-renders a scope when the
/// state it read changes, and nothing reads anything here -- and a clear of `HIGHLIGHTED`,
/// where the source colours are already baked into the cached spans.
fn palette() -> &'static Palette {
    &Palette::LIGHT
}

/// Applying one of the two fonts. freya takes font families one at a time, pushing
/// each onto the element's own list and appending the parent's behind it, so the
/// chain is set by calling `font_family` in order of preference.
trait FontExt: TextStyleExt + Sized {
    fn font(mut self, font: &'static Font) -> Self {
        for family in &font.families {
            self = self.font_family(family.clone());
        }
        self.font_size(font.size)
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

/// The current selection, shared through context.
///
/// Since 6c this *is* the active tab: everything that is on screen in the content area is
/// the one entry of [`Open`] that this names. Nothing beside it says which tab is active,
/// which is why `Selection` is still the single thing the history records and the session
/// saves — it did not become a second state, it grew a list around it.
#[derive(Clone, Copy)]
struct Sel(State<Selection>);

/// The tabs open in the content area, shared through context.
///
/// The list only; the active one is `Sel`. Every entry is an `Object` or a `Symbol` —
/// [`Selection::None`] is never a tab, it is what the app is in when the list is empty,
/// which is the placeholder state.
///
/// Objects are in here alongside functions on purpose. A tab is a place the reader has
/// open, the sidebar's object rows have always *been* a selection, and giving them a tab
/// is what keeps `Sel` equal to the active tab without a second "selected but not open"
/// state beside it. The chip for an object is named after the object and the Assembly and
/// Source panes show the same placeholders they always did for one.
#[derive(Clone, Copy)]
struct Open(State<Tabs<Selection>>);

/// The source files open in the Source pane, shared through context. The list only; which
/// of them is on screen is [`Shown`], for the reason [`Open`] keeps that out too.
#[derive(Clone, Copy)]
struct Files(State<Tabs<Arc<str>>>);

/// Which of the open source files the Source pane is showing. `Some` exactly when
/// [`Files`] is non-empty, which [`open_file`] and [`close_file`] are what keep true.
#[derive(Clone, Copy)]
struct Shown(State<Option<Arc<str>>>);

/// Which row each open content tab was left on, shared through context.
///
/// Beside [`Open`] rather than inside it, and beside it rather than inside
/// [`InstructionList`], for one reason each. Inside `Tabs` it would be a field of what
/// the strip draws, so a scroll of the reader's would re-render every chip; inside the
/// pane it would live and die with the component, which is precisely the bug this fixes —
/// one scroll controller is reused for every symbol, so a tab switch used to leave the
/// new function at the offset the old one was at. Here it outlives both the component and
/// any one selection, which is what a *tab's* position has to do.
///
/// Keyed by `Selection`, which is compared by `Arc` pointer identity — the same identity
/// [`Open`] keys by, so an entry means "this tab" for exactly as long as that tab is in
/// the list, and never accidentally means a second symbol of the same name in another
/// object. It is also why the persisted form cannot reuse the key and identifies its tabs
/// by path and name instead (`project.rs`).
#[derive(Clone, Copy)]
struct AsmAt(State<Positions<Selection>>);

/// Which line each open source file was left on, shared through context. [`AsmAt`] for
/// the Source pane, keyed by the file the pane shows rather than by the selection: the
/// pane's tabs are files, and two symbols compiled from one file are one tab.
#[derive(Clone, Copy)]
struct SrcAt(State<Positions<Arc<str>>>);

/// Where the selection has been, shared through context. Named `Hist` because
/// `History` is the type it holds, the same way `Sel` holds a `Selection`.
#[derive(Clone, Copy)]
struct Hist(State<History>);

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

/// The selected symbol's line info, shared through context so every pane that maps
/// between source and assembly reads the same rows.
#[derive(Clone, Copy)]
struct Lines(Memo<SymbolLines>);

/// What DWARF says about the selected symbol's instructions, or `None` when it says
/// nothing, and which of the files it names the Source pane should open on.
///
/// Worked out once for all its readers rather than once per pane: `Object::line_info`
/// walks the line program of every unit covering the symbol again on each call, even
/// though the DWARF context itself is built only once.
///
/// The file is worked out *here*, beside the info it comes from, rather than by whoever
/// wants it. A `Memo` recomputes in a spawned task, so anything reading `Sel` and this
/// together sees them disagree for one beat after a selection change — and asking the
/// previous symbol's `LineInfo` where the new symbol starts answers with the previous
/// symbol's file, which would open a tab for a file that has nothing to do with what was
/// clicked. Inside the memo the two cannot disagree.
#[derive(Clone)]
struct SymbolLines {
    info: Option<Arc<LineInfo>>,
    /// Which of the files the symbol touches the Source pane opens on: the one its first
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
    /// The line info for `selection`, with the file the Source pane should open on.
    fn new(selection: &Selection) -> SymbolLines {
        let Selection::Symbol(symbol) = selection else {
            return SymbolLines {
                info: None,
                file: None,
            };
        };

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
/// Not `Tab`, which names six views of which four have nothing to answer here: this is the
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
    let row = index as f32 * ROW_HEIGHT;
    let margin = CONTEXT_ROWS * ROW_HEIGHT;

    if row >= top + margin && row + ROW_HEIGHT <= top + viewport {
        return;
    }

    controller.scroll_to_y(-((row - margin).max(0.0) as i32));
}

/// The row at the top of a pane scrolled to `offset`, and the offset that puts `row`
/// there — the one place the two units meet.
///
/// A `VirtualScrollView`'s offset counts *down* from zero, so the arithmetic is a
/// negation and a divide by `ROW_HEIGHT`, which is every list's `item_size`. Rounded
/// *down*, which is the half-row a position in rows gives up and the direction to give it
/// up in: the row at the top edge is the one the reader is looking at even when it is only
/// half on screen, and coming back to the one below it would lose the half they could see.
fn row_at(offset: i32) -> usize {
    ((-offset).max(0) as f32 / ROW_HEIGHT) as usize
}

fn row_offset(row: usize) -> i32 {
    -((row as f32 * ROW_HEIGHT) as i32)
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
/// while a selection change, which does, drops the pin outright (`use_clear_focus`).
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
            // writing its row down would put it straight back, keyed by a `Selection`
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
/// The highlighter comes from `freya-code-editor`, which is a dependency for this and
/// not for its editor component: `CodeEditor` paints a line background only for the
/// cursor's own row and keeps its scroll state private, so it can neither highlight the
/// set of lines an instruction maps to (5b) nor be scrolled to one (5c). Its
/// `SyntaxHighlighter` is public on its own and is exactly the shape these rows want.
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
/// palette that was current at the time. That is free while there is one palette. When
/// Step 9 adds a second, switching it has to `clear()` this map -- the entries are not
/// stale, they are the wrong theme, and nothing else in the app would repaint them.
/// Re-highlighting every open file is what a switch costs, which is why the parse belongs
/// where it is rather than in `source::load`: `source`'s cache of the *text* survives it.
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
    /// `TOGGLE_SIZE` beside these, they lose anyway. `case-sensitive` is an `Aa` drawn as
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
                .width(Size::px(TOGGLE_SIZE))
                .height(Size::px(TOGGLE_SIZE))
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
                    .height(Size::px(FILTER_HEIGHT))
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
        .child(rect().width(Size::fill()).height(Size::flex(1.0)).child(list))
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

/// Make `target` what the content area is showing, opening a tab for it if it has none.
///
/// The one path by which `Sel` ever changes, which is what makes "the selection is the
/// active tab" an invariant rather than a convention: the sidebar's object and symbol
/// rows, an assembly relocation link, the history panel and the back/forward buttons
/// (both through [`navigate`]) and the startup restore all come through here, so none of
/// them has to know that tabs exist. [`Selection::None`] opens nothing and is how the
/// content area goes back to its placeholder.
///
/// Re-focusing a tab that is already open writes nothing: `State::write` notifies its
/// subscribers whether or not the value changes, so both the list and the selection are
/// asked before they are touched.
fn activate(mut open: State<Tabs<Selection>>, mut selection: State<Selection>, target: Selection) {
    // The guard from `peek` has to be gone before `write` is reached, so the answer is
    // taken out of it first rather than tested inline.
    let already_open = matches!(target, Selection::None) || open.peek().find(&target).is_some();
    if !already_open {
        open.write().open(target.clone());
    }

    selection.set_if_modified(target);
}

/// Close the tab showing `entry`, moving to a neighbouring one when it was the tab on
/// screen and to the placeholder when it was the last one open.
///
/// Landing on the neighbour is an ordinary selection change, so it is recorded in the
/// history like any other: the reader is now somewhere else, and the way back to it is
/// the way back to anywhere else.
///
/// Where the tab was left goes with it. A closed tab is not a tab, so a position kept for
/// one is both a lie — reopening it from the sidebar is a fresh tab, which starts at the
/// top — and a leak, since a [`Selection`] holds the `Arc<Object>` it points into.
fn close_tab(
    mut open: State<Tabs<Selection>>,
    selection: State<Selection>,
    mut at: State<Positions<Selection>>,
    entry: &Selection,
) {
    let was_showing = *selection.peek() == *entry;
    let next = open.write().close(entry);
    at.write().forget(entry);

    if was_showing {
        // Through `activate` like everything else, even though the neighbour is by
        // construction already open: this is a selection change and there is one way to
        // make one. The write guard above is released before it is reached.
        activate(open, selection, next.unwrap_or(Selection::None));
    }
}

/// Open a tab for `file` in the Source pane and put the pane on it.
///
/// The file the pane is put on is the copy already in the list where there is one, so the
/// `Arc` the rows are keyed by does not change identity when the same file is reached
/// again through a different symbol's `LineInfo`.
fn open_file(mut files: State<Tabs<Arc<str>>>, mut shown: State<Option<Arc<str>>>, file: Arc<str>) {
    let existing = files.peek().find(&file).cloned();
    let file = match existing {
        Some(file) => file,
        None => {
            files.write().open(file.clone());
            file
        }
    };

    shown.set_if_modified(Some(file));
}

/// Close the tab showing `file`, moving to a neighbouring one when it was the file on
/// screen. The Source pane's own half of [`close_tab`], and the mirror of it: nothing
/// here touches the selection, because which file is on screen is a view of the symbol
/// rather than a place the reader has been.
fn close_file(
    mut files: State<Tabs<Arc<str>>>,
    mut shown: State<Option<Arc<str>>>,
    mut at: State<Positions<Arc<str>>>,
    file: &Arc<str>,
) {
    let was_showing = shown.peek().as_ref() == Some(file);
    let next = files.write().close(file);
    at.write().forget(file);

    if was_showing {
        shown.set(next);
    }
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The fifth of the functions that hold the app's invariants, beside [`activate`],
/// [`close_tab`], [`open_file`] and [`close_file`], and the only one that ever *removes*
/// an object -- until now the app could open a binary and never let go of one. The unit
/// is the **file** and never the object: an archive member is not something the reader
/// opened, closing one member of 196 would leave a file half-present with no row able to
/// say so, and `Project::binaries` is a list of paths, so half a file is not a thing the
/// session could even record. One path opened twice is therefore also one close: the
/// objects list holds both copies, `Object::path` cannot tell them apart, and neither
/// could the file it would be written to.
///
/// What each of the five things pointing at those objects does with the news:
///
/// - The **tabs** whose selection was in the file are closed, all of them at once
///   ([`Tabs::close_all`]), which is what closing the one tab the reader was on would
///   have done had its neighbours not gone with it.
/// - The **selection** follows the tabs rather than degrading the way a restore's does.
///   Degrading has nothing to fall back *to* here: a file takes its objects and their
///   symbols together, so `resolve_or_degrade`'s symbol-to-object step would land on an
///   object that is going away in the same breath. What is left is the tab rule -- the
///   neighbouring tab, or [`Selection::None`] when the close emptied the strip -- and
///   that is also the only answer that keeps "the selection is the active tab" true,
///   since the placeholder with tabs still open would be a fourth state.
/// - The **history** drops its entries rather than degrading them ([`History::retaining`]),
///   which is the same walk and the same reasoning as a restore whose binaries have
///   changed: a list of places the reader cannot get back to is worse than a short list.
/// - The **viewing positions** of the tabs that closed go with them ([`Positions`]), which
///   is not tidiness: every entry is keyed by a [`Selection`], which holds the
///   `Arc<Object>` it points into, so one left behind would hold the file's bytes -- 331 MB
///   of them, for `viewer-sample` -- for as long as the app ran.
/// - **`Project::binaries`** needs nothing here at all. It is derived from the objects by
///   `Project::from_state`, so removing them removes the path, and `project::record` sees
///   a *binaries* change and writes it to disk at once rather than marking it pending --
///   which is what `Goals.md` asks of a change the user made, and the first thing since
///   opening a file to take that path.
///
/// All four writes happen here, in one event handler, before anything can render: the
/// save observer therefore wakes once, with all of it settled, so the file that reaches
/// the disk never names a binary the app has already let go of.
///
/// The Source pane's open files are deliberately left alone. A file chip is a path on
/// disk that some symbol's line info named, nothing records which object opened it, and
/// the text stands perfectly well on its own -- the same reason a selection with no line
/// info neither opens nor closes one.
fn close_binary(
    mut objects: State<Vec<Arc<Object>>>,
    mut open: State<Tabs<Selection>>,
    selection: State<Selection>,
    mut at: State<Positions<Selection>>,
    mut history: State<History>,
    path: &Path,
) {
    // Every guard below is taken out of its own statement, so none of them is still
    // alive when the next write -- or `activate` at the end -- is reached.
    let showing = selection.peek().clone();
    let next = open.write().close_all(&showing, |tab| tab.in_file(path));

    // The same walk over the same rule, so the positions cannot outlive the tabs they
    // belong to.
    at.write().forgetting(|tab| !tab.in_file(path));

    let remaining = history.peek().retaining(|entry| !entry.in_file(path));
    history.set(remaining);

    objects.write().retain(|object| object.path != path);

    if showing.in_file(path) {
        // Through `activate` like every other selection change, even though the tab it
        // lands on is by construction already open. Landing there is an ordinary move,
        // so `use_record_history` records it exactly as it records closing one tab.
        activate(open, selection, next.unwrap_or(Selection::None));
    }
}

/// The menu a file row opens on a right-click: the one thing that can be done to a file
/// once it is open.
///
/// Built per press rather than once, because it closes over the path of the row it was
/// opened on -- freya's `ContextMenu` takes a whole `Menu` and places it at the pointer
/// (`freya-components/src/context_menu.rs`), so there is nothing to keep. The four states
/// come in as arguments for the reason every row's do: this is called from an event
/// handler, where no hook may run.
fn close_menu(
    objects: State<Vec<Arc<Object>>>,
    open: State<Tabs<Selection>>,
    selection: State<Selection>,
    at: State<Positions<Selection>>,
    history: State<History>,
    path: PathBuf,
) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| close_binary(objects, open, selection, at, history, &path))
            // "file" and not "object", because the row a reader right-clicks may be one
            // object of one file or the archive above 196 of them, and the same word has
            // to be true of both.
            .child("Close file"),
    )
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
fn tree_name(text: String) -> impl IntoElement {
    rect()
        .width(Size::flex(1.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(text)
                .width(Size::fill())
                .max_lines(1)
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
    /// The group this row is, in the tab's set of the groups the reader has opened.
    group: usize,
    expanded: State<HashSet<usize>>,
    key: DiffKey,
}

impl PartialEq for ArchiveRow {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.path == other.path
            && self.members == other.members
            && self.expansion == other.expansion
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
        // The five states closing a file has to answer for. Consumed here, in the
        // render, because the handler that uses them may not run a hook.
        let objects = use_consume::<Objects>().0;
        let selection = use_consume::<Sel>().0;
        let open = use_consume::<Open>().0;
        let at = use_consume::<AsmAt>().0;
        let history = use_consume::<Hist>().0;
        let path = self.path.clone();

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        // `Forced` draws no triangle, only the space one would have taken: while the
        // filter is holding the file open, folding it would hide the very rows the filter
        // put on screen, so there is nothing here to press. See `Expansion::Forced`.
        let chevron = match expansion {
            Expansion::Collapsed => "\u{25b8}",
            Expansion::Expanded => "\u{25be}",
            Expansion::Forced => "",
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
                .height(Size::px(ROW_HEIGHT))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
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
                    ContextMenu::open_from_event(
                        &e,
                        close_menu(objects, open, selection, at, history, path.clone()),
                    );
                })
                .child(
                    label()
                        .text(chevron)
                        .width(Size::px(CHEVRON_WIDTH))
                        .color(palette().address_fg)
                        .max_lines(1),
                )
                .child(tag_label(ARCHIVE_TAG))
                .child(tree_name(self.name.clone()))
                // How many objects came out of this file, which under a filter is how
                // many of them matched. It is the one thing about an archive that is not
                // visible while it is folded shut.
                .child(
                    label()
                        .text(self.members.to_string())
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
        let objects = use_consume::<Objects>().0;
        let selection = use_consume::<Sel>().0;
        let open = use_consume::<Open>().0;
        let at = use_consume::<AsmAt>().0;
        let history = use_consume::<Hist>().0;
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
                .height(Size::px(ROW_HEIGHT))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    activate(open, selection, Selection::Object(object.clone()));
                })
                // A lone object *is* the file it came out of, so it closes like one. A
                // member is not: it was never opened on its own, and the row that can
                // close the file it belongs to is the one above it. Right-clicking a
                // member therefore does nothing rather than quietly taking 195 rows the
                // reader was not pointing at with it.
                .maybe(!self.member, move |row| {
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(
                            &e,
                            close_menu(objects, open, selection, at, history, path.clone()),
                        );
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
                .child(tree_name(self.object.name.clone())),
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
        let selection = use_consume::<Sel>().0;
        let open = use_consume::<Open>().0;
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
                .height(Size::px(ROW_HEIGHT))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    activate(open, selection, Selection::Symbol(symbol.clone()));
                })
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a selection is called where it is named in a list: the same demangled name the
/// symbol list shows, or the object's name for an object. The history rows and the tab
/// chips both draw this, which is what makes a place read the same wherever it is named.
/// `Selection::None` reaches neither list -- `History::push` refuses it and it is never a
/// tab -- so its arm is unreachable in practice.
fn entry_text(entry: &Selection) -> String {
    match entry {
        Selection::None => String::new(),
        Selection::Object(object) => object.name.clone(),
        Selection::Symbol(symbol) => symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone(),
    }
}

/// The pointer identity of what a selection points at, for keying the row or chip that
/// names it. A tab chip keys by this alone, its place in the strip being stable. Paired
/// with the entry's index because a row's identity is its place in the list: the entry at
/// an index changes when a push truncates the forward entries, and again when a push
/// bumps an existing entry to the newest position and shifts the ones behind it down. The
/// pointer alone would be identity enough now that no two entries are equal, but then a
/// bumped row would keep the hover state of the one that used to sit where it now does;
/// with the index in the key the moved rows are simply rebuilt, which for a list this
/// short costs nothing.
fn entry_addr(entry: &Selection) -> usize {
    match entry {
        Selection::None => 0,
        Selection::Object(object) => Arc::as_ptr(object).addr(),
        Selection::Symbol(symbol) => Arc::as_ptr(&symbol.data).addr(),
    }
}

/// One visited selection in the history list. Clicking it moves the history cursor to
/// this entry rather than recording a new one, which is what `Nav::To` is for.
#[derive(Clone)]
struct HistoryRow {
    entry: Selection,
    index: usize,
    /// Whether the cursor is on this entry, i.e. this is what is on screen.
    current: bool,
    key: DiffKey,
}

impl PartialEq for HistoryRow {
    fn eq(&self, other: &Self) -> bool {
        // `Selection`'s own `PartialEq` is written in terms of `Arc::ptr_eq`.
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
        let selection = use_consume::<Sel>().0;
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
            text.clone(),
            rect()
                .width(Size::fill())
                .height(Size::px(ROW_HEIGHT))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| navigate(open, history, selection, Nav::To(index)))
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
        let selection = use_consume::<Sel>().0;
        let open = use_consume::<Open>().0;
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

                    activate(open, selection, Selection::Symbol(symbol.clone()));
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
    let middle = ROW_HEIGHT / 2.0;
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
        .height(Size::px(ROW_HEIGHT))
        .children((0..width).filter_map(move |lane| {
            let vertical = arrows.lanes.lanes[lane];
            let (top, tall) = match (vertical.top, vertical.bottom) {
                (true, true) => (0.0, ROW_HEIGHT),
                (true, false) => (0.0, middle),
                (false, true) => (middle, ROW_HEIGHT - middle),
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
            .height(Size::px(ROW_HEIGHT))
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
            .height(Size::px(ROW_HEIGHT))
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
// Tab strips
// ---------------------------------------------------------------------------

/// One tab in a strip: what it is called, an × that closes it, and the pane's own white
/// when it is the one on screen -- the same thing a dock tab header does, so the two bars
/// read as bars of the same kind.
///
/// A stateless helper rather than a component: the two kinds of chip differ only in what
/// they name and in what their two presses do, and the hover state belongs to the
/// component that called this, so no hook runs here.
fn chip(
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
            .height(Size::px(ROW_HEIGHT))
            .padding(Gaps::new_symmetric(0.0, 8.0))
            .spacing(6.0)
            .background(background)
            .border(right_hairline())
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(on_activate)
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
/// itself is off: it would eat a third of a `ROW_HEIGHT` bar, and the wheel and a drag
/// both still move it.
fn chip_strip(chips: Vec<Element>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(ROW_HEIGHT))
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

/// One open function or object, in the content area's strip.
#[derive(Clone)]
struct TabChip {
    entry: Selection,
    /// Whether this is the tab the content area is showing, i.e. whether it is `Sel`.
    active: bool,
    key: DiffKey,
}

impl PartialEq for TabChip {
    fn eq(&self, other: &Self) -> bool {
        // `Selection`'s own `PartialEq` is written in terms of `Arc::ptr_eq`.
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
        let selection = use_consume::<Sel>().0;
        let open = use_consume::<Open>().0;
        let at = use_consume::<AsmAt>().0;
        let text = entry_text(&self.entry);
        let (activated, closed) = (self.entry.clone(), self.entry.clone());

        chip(
            text.clone(),
            text,
            self.active,
            hovering,
            move |_| activate(open, selection, activated.clone()),
            move |_| close_tab(open, selection, at, &closed),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The strip of open tabs over the content area.
///
/// Over the whole content area rather than inside the Assembly pane, which is where the
/// plan's sketch put it. The tab decides what *both* panes show -- the assembly of a
/// function and the source it was compiled from are two views of the one place -- and a
/// strip inside one of them would go wherever that pane was dragged, taking the only way
/// of switching functions into a 300px sidebar with it. In the default layout the two are
/// the same thing: the strip is the bar directly above the assembly.
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
        let active = use_consume::<Sel>().0.read().clone();
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
                        active: *entry == active,
                        key: DiffKey::None,
                    }
                    .key(entry_addr(entry))
                    .into()
                })
                .collect(),
        )
    }
}

/// One open source file, in the Source pane's strip.
#[derive(Clone)]
struct FileChip {
    file: Arc<str>,
    active: bool,
    key: DiffKey,
}

impl PartialEq for FileChip {
    fn eq(&self, other: &Self) -> bool {
        // By its text and not by pointer, for the reason `LinePos` compares that way: a
        // path is a value, and two `LineInfo`s naming one file hold two `Arc<str>`s of it.
        self.file == other.file && self.active == other.active
    }
}

impl KeyExt for FileChip {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for FileChip {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let files = use_consume::<Files>().0;
        let shown = use_consume::<Shown>().0;
        let at = use_consume::<SrcAt>().0;
        let (activated, closed) = (self.file.clone(), self.file.clone());

        chip(
            // The file's own name; the strip is narrow and every one of these paths shares
            // most of its directory with the others. The whole path is in the tooltip,
            // which is what the pane's header used to say.
            file_name(&self.file),
            self.file.to_string(),
            self.active,
            hovering,
            move |_| open_file(files, shown, activated.clone()),
            move |_| close_file(files, shown, at, &closed),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
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

/// What a source file is called in its chip: the last component of its path, or the whole
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

#[derive(Clone, PartialEq)]
struct AssemblyView {
    symbol: Symbol,
}

impl Component for AssemblyView {
    fn render(&self) -> impl IntoElement {
        let assembly = self.symbol.data.assembly(&self.symbol.object);

        let Some(assembly) = assembly else {
            return rect()
                .padding(5.0)
                .child(label().text("Assembly unavailable"));
        };

        // Worked out here rather than in a memo of its own, which is what "beside the
        // disassembly" means: it is one pass over the branches of the symbol that was
        // just decoded, in the one component that re-renders exactly when a symbol is
        // selected. A memo would recompute it in a spawned task and so land a beat after
        // the disassembly it belongs to, which is the second reason `InstructionList`
        // exists at all.
        let lanes = Arc::new(Lanes::new(&assembly.edges, assembly.instructions.len()));

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .padding(5.0)
            .background(palette().asm_pane_bg)
            .child(InstructionList {
                assembly,
                symbol: self.symbol.clone(),
                lanes,
            })
    }
}

/// The instruction rows themselves, a component of their own rather than part of
/// `AssemblyView` because they follow two things the disassembly must not follow.
///
/// `SymbolData::assembly` decodes and formats the whole symbol on every call, so whatever
/// reads the cross-view focus has to be something that does not disassemble, or every
/// pointer move across a row boundary would decode the function again. The line info is
/// read here for a second reason: freya's `Memo` recomputes in a spawned task, so the
/// `Lines` context updates a beat after the selection it follows, and a pane taking it as
/// a prop renders twice per selection change rather than once.
#[derive(Clone)]
struct InstructionList {
    assembly: Arc<Assembly>,
    /// The whole symbol and not just its object, because these rows answer to a *tab*
    /// as well as to a disassembly: `Selection::Symbol(symbol)` is the key its viewing
    /// position is kept under, and it is the one the strip and the session key by too.
    symbol: Symbol,
    lanes: Arc<Lanes>,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && self.symbol == other.symbol
            && Arc::ptr_eq(&self.lanes, &other.lanes)
    }
}

impl Component for InstructionList {
    fn render(&self) -> impl IntoElement {
        let lines = use_consume::<Lines>().0.read().clone();
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
            lines,
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
            &Selection::Symbol(self.symbol.clone()),
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
                .item_size(ROW_HEIGHT),
            )
    }
}

/// The source rows themselves, split out of `SourceTab` the way `InstructionList` is out
/// of `AssemblyView` -- here not because the pane above is expensive to render, which it is
/// not, but because it has three early returns before it knows which file it is showing.
/// Hooks have to run on every render, and the scroll controller these rows are driven by
/// cannot be armed before the file it would scroll through is known.
#[derive(Clone)]
struct SourceList {
    source: SourceText,
    file: Arc<str>,
}

impl PartialEq for SourceList {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && Arc::ptr_eq(&self.file, &other.file)
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
        // The Source pane's tab is the file it is showing, so that is what the position
        // is kept under: two symbols compiled from one file share the tab, and so share
        // where it was left.
        use_kept_position(
            use_consume::<SrcAt>().0,
            use_consume::<Files>().0,
            controller,
            &self.file,
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
                        .item_size(ROW_HEIGHT),
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

/// One of the six dockable views. A tab is a persistent view rather than a slot
/// the selection drives, so each one renders itself off the current `Selection`
/// and subscribes to the state it needs on its own -- which also keeps a
/// selection change from re-rendering the whole tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tab {
    Objects,
    Symbols,
    Info,
    History,
    Assembly,
    Source,
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
    /// because the pane is a strip of files and shows one of them.
    ///
    /// The name is passed beside the bytes because `ImageSource` keys the raster cache on
    /// a hash of whatever it is given, and hashing six short names per render is cheaper
    /// than hashing six SVGs.
    fn icon(self) -> Element {
        let (name, svg) = match self {
            Tab::Objects => ("package", lucide::package()),
            Tab::Symbols => ("square-function", lucide::square_function()),
            Tab::Info => ("info", lucide::info()),
            Tab::History => ("history", lucide::history()),
            Tab::Assembly => ("binary", lucide::binary()),
            Tab::Source => ("file-code", lucide::file_code()),
        };

        let side = icon_size();
        SvgViewer::new((name, svg))
            .width(Size::px(side))
            .height(Size::px(side))
            // The colour is given rather than inherited: `SvgViewer` rasterizes only once
            // it knows one, and with none set it waits for an `on_styled` to tell it the
            // inherited text colour, which is a frame late and a frame of nothing in a
            // 26px bar. Setting it also skips the loader, which is off in any case --
            // these are six 24px glyphs rasterized synchronously out of the binary, and a
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
        }
    }
}

#[derive(PartialEq)]
struct ObjectsTab;

impl Component for ObjectsTab {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
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
        let tree = use_memo(move || {
            ObjectTree::new(&objects.read(), &filter.read().matcher(), &expanded.read())
        });
        let tree = tree.read().clone();
        // The selected object as the address its rows are keyed by, rather than as the
        // `Arc` itself: everything handed to a `VirtualScrollView` has to be `PartialEq`
        // and an `Object` is not, while pointer identity — which is the only identity the
        // UI uses anyway — compares as a number.
        let selected = match &*use_consume::<Sel>().0.read() {
            Selection::Object(object) => Some(Arc::as_ptr(object).addr()),
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
                        } => ArchiveRow {
                            name: name.clone(),
                            path: path.clone(),
                            members: *members,
                            expansion: *expansion,
                            group: *group,
                            expanded: *expanded,
                            key: DiffKey::None,
                        }
                        .key(*group)
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
            .item_size(ROW_HEIGHT),
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
        let filtered = use_memo(move || {
            Filtered::new(symbols.read().clone(), &filter.read().matcher())
        });
        let filtered = filtered.read().clone();
        let selected = match &*use_consume::<Sel>().0.read() {
            Selection::Symbol(symbol) => Some(symbol.clone()),
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
            .item_size(ROW_HEIGHT),
        )
    }
}

#[derive(PartialEq)]
struct InfoTab;

impl Component for InfoTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Sel>().0.read().clone();

        match &current {
            Selection::None => placeholder("Nothing selected"),
            Selection::Object(object) => rect()
                .expanded()
                .background(palette().pane_bg)
                .child(info_line(format!("Object: `{}`", object.name)))
                .child(info_line(format!("Format: {:?}", object.format)))
                .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
                .into(),
            Selection::Symbol(symbol) => rect()
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
                    .key((index, entry_addr(entry)))
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

#[derive(PartialEq)]
struct AssemblyTab;

impl Component for AssemblyTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Sel>().0.read().clone();

        match &current {
            Selection::Symbol(symbol) => AssemblyView {
                symbol: symbol.clone(),
            }
            .into_element(),
            _ => placeholder("No symbol selected"),
        }
    }
}

#[derive(PartialEq)]
struct SourceTab;

impl Component for SourceTab {
    fn render(&self) -> impl IntoElement {
        let files = use_consume::<Files>().0;
        let shown = use_consume::<Shown>().0;
        // Consumed unconditionally, hooks having to run on every render, but only read in
        // the branch that needs them: which of the three reasons nothing is open comes
        // from the selection, since the strip has nothing to say about it. Reading the
        // memo there also subscribes this tab to it, so the pane fills in when the line
        // info for a newly selected symbol is worked out, without the root re-rendering.
        let selection = use_consume::<Sel>().0;
        let lines = use_consume::<Lines>().0;

        let open: Vec<Arc<str>> = files.read().tabs().to_vec();
        let file = shown.read().clone();

        let Some(file) = file else {
            let current = selection.read().clone();
            let lines = lines.read().clone();
            return match (&current, &lines.info) {
                (Selection::Symbol(_), Some(_)) => placeholder("No source file open"),
                (Selection::Symbol(_), None) => placeholder("No line info"),
                _ => placeholder("No symbol selected"),
            };
        };

        rect()
            .expanded()
            // The strip takes its own height and the list is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(chip_strip(
                open.iter()
                    .map(|open| {
                        FileChip {
                            file: open.clone(),
                            active: *open == file,
                            key: DiffKey::None,
                        }
                        .key(&**open)
                        .into()
                    })
                    .collect(),
            ))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // Named in the message because the path is the only clue to *why*:
                    // source built on another machine, moved, or deleted since all look
                    // alike from here.
                    .child(match source_text(Path::new(&*file)) {
                        Some(source) => SourceList { source, file }.into_element(),
                        None => placeholder(format!("Source file not found: {file}")),
                    }),
            )
            .into()
    }
}

// ---------------------------------------------------------------------------
// Docking
// ---------------------------------------------------------------------------

/// Panel ids are only ever looked up inside the area that handed them out, so
/// each area numbers its own panels from zero.
type PanelId = u32;

/// One docking area: the tree of splits and tabbed panels filling one of the two
/// resizable panes. The six tabs are shared between the two areas, so a drop
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
        .height(Size::px(ROW_HEIGHT))
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
        .height(Size::px(ROW_HEIGHT))
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

fn toolbar(objects: State<Vec<Arc<Object>>>) -> impl IntoElement {
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

            // Reading and parsing is CPU bound and can take seconds on a large
            // binary, so keep it off the UI thread and hand the result back.
            let (sender, receiver) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = sender.send_blocking(open_files(paths));
            });

            if let Ok(parsed) = receiver.recv().await {
                let mut objects = objects;
                objects.write().extend(parsed);
            }
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
/// the three `selection.set(..)` sites, the toolbar's `objects.write()`,
/// `use_record_history`'s push and the two tab lists know nothing about persistence,
/// and neither will any future one. The subscriptions *are* the `read()` calls, which
/// is the whole of what makes adding a persisted field to `Project::from_state` also
/// add the state behind it to what wakes this.
///
/// Whether a change reaches the disk now or at the next `use_periodic_save` tick is
/// `project::record`'s decision, not this one's: opening a binary is written at once,
/// a selection, a tab or a history entry is left pending. That policy is framework-free
/// and unit-tested in `project.rs`; all this hook owns is *when to look*.
///
/// A selection change wakes this twice -- once for `Sel`, and again when
/// `use_record_history` pushes the entry that follows from it -- which costs two
/// derivations and two comparisons and, since neither is a binaries change, no write
/// at all. A selection change onto a symbol that was not open wakes it a third time,
/// for the tab `activate` opened, and that one is free for the same reason.
///
/// Scrolling a pane wakes it too, which is the one input here that a reader can produce
/// continuously. It costs no more than the three above, and it is bounded by the unit the
/// position is kept in: a viewing position is a *row*, so a scroll writes nothing until
/// the pane has moved a whole `ROW_HEIGHT`, and `use_kept_position` compares before it
/// writes.
fn use_save_on_change(
    objects: State<Vec<Arc<Object>>>,
    open: State<Tabs<Selection>>,
    asm_at: State<Positions<Selection>>,
    selection: State<Selection>,
    history: State<History>,
    files: State<Tabs<Arc<str>>>,
    src_at: State<Positions<Arc<str>>>,
    shown: State<Option<Arc<str>>>,
) {
    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it. Each
        // guard lives to the end of the statement it is created in, which is the one
        // `record` call, and nothing here writes anything, so holding eight at once is
        // the safe half of the `peek`/`write` gotcha rather than the dangerous one.
        let shown = shown.read();
        project::record(Project::from_state(
            &objects.read(),
            open.read().tabs(),
            &asm_at.read(),
            &selection.read(),
            &history.read(),
            files.read().tabs(),
            &src_at.read(),
            shown.as_deref(),
        ));
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

/// Record every selection in the navigation history.
///
/// Deliberately its own effect rather than a second job inside `use_save_on_change`,
/// even though both observe `Sel`: this one has no business subscribing to `Objects`,
/// and if it did, opening a binary -- which changes the objects and nothing else --
/// would run the history code for a selection it has already recorded. Two effects
/// with one subscription each also keep the two concerns separable, since only one of
/// them touches the disk. The choke-point property is untouched: this is still the
/// single observer of `Sel`, so every `selection.set(..)` site, present and future,
/// lands here without knowing it.
///
/// The history holds `Selection`s, which are `Arc`s compared by pointer, so an entry
/// is a refcount bump and no copy of any object data.
///
/// Nothing here pushes on navigation: back/forward (the next slice) move the cursor and
/// then set `Sel` to the entry they landed on, this effect runs as it does for any other
/// change, and `would_push` is false because that entry is exactly what the cursor is
/// now on. Navigation therefore costs no entry, and no separate "we are navigating"
/// flag is needed to make that true.
fn use_record_history(selection: State<Selection>, history: State<History>) {
    use_side_effect(move || {
        // Reading subscribes the effect to the selection; `peek` on the history does
        // not, because the effect must not subscribe to the state it writes.
        let selection = selection.read().clone();

        // `write()` notifies its subscribers before it hands the value over, whether or
        // not anything changes, so ask first: a push that would dedup away must not
        // wake the history panel.
        if history.peek().would_push(&selection) {
            let mut history = history;
            history.write().push(selection);
        }
    });
}

/// Forget the cross-view focus and the pin whenever the selection changes.
///
/// Both are positions inside the selected symbol's line info, so they mean nothing once
/// that symbol is gone -- and the ordinary way the focus goes away, the pointer leaving the
/// row that set it, need never happen: clicking a relocation label navigates from an
/// assembly row the pointer is still sitting on, and the symbol it lands in was very often
/// compiled from the same file, so a line of that file would stay lit for a position in a
/// function no longer on screen until the pointer moved. A pin has no such ordinary way at
/// all -- staying is the whole of what makes it one -- so this is the only thing that ends
/// it short of another click.
///
/// Its own effect for the reason `use_record_history` is: it has no business subscribing
/// to anything but `Sel`, and the two concerns stay separable.
fn use_clear_focus(
    selection: State<Selection>,
    focused: State<Option<LineFocus>>,
    pinned: State<Option<Pin>>,
) {
    use_side_effect(move || {
        // Reading subscribes the effect to the selection, which is the whole of what it
        // wants from it -- both are `None` again whatever the new selection is.
        let _ = selection.read();

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
    selection: State<Selection>,
    shown: State<Option<Arc<str>>>,
    marked: State<Option<Marks>>,
) {
    use_side_effect(move || {
        let _ = selection.read();
        unmark(marked, Pane::Assembly);
    });
    use_side_effect(move || {
        let _ = shown.read();
        unmark(marked, Pane::Source);
    });
}

/// Open a tab for the file the active symbol was compiled from, and put the Source pane
/// on it.
///
/// This is what keeps the source side following the selection now that it has tabs of its
/// own: selecting a function always shows that function's source, whichever file the
/// reader had switched to by hand, and the file is added to the strip if it was not there
/// already. Selecting one whose file *is* already open costs nothing but the focus moving
/// to its chip.
///
/// Only the symbol's own file is opened, never the rest of `LineInfo::files`. A Rust
/// function inlines dozens of them, and a strip that grew by dozens per click would stop
/// being a strip; reaching an inlined header's source is a list of the files a symbol
/// touches, which is a thing to build when there is somewhere to put it.
///
/// A selection with no line info opens nothing and closes nothing -- the pane keeps
/// showing whatever was open, which is what tabs mean, and the assembly side already says
/// that nothing is mapped by lighting no rows.
///
/// It reads only [`Lines`], not the selection, and that is load-bearing: a `Memo`
/// recomputes in a spawned task, so an effect reading both would see the new symbol beside
/// the previous symbol's `LineInfo` for one beat and open a tab for a file belonging to
/// neither. `SymbolLines` carries the file for exactly this reason.
fn use_open_source_file(
    lines: Memo<SymbolLines>,
    files: State<Tabs<Arc<str>>>,
    shown: State<Option<Arc<str>>>,
) {
    use_side_effect(move || {
        // Reading subscribes the effect to the line info; the two states it writes are
        // never read here, so it cannot wake itself.
        let file = lines.read().file.clone();
        if let Some(file) = file {
            open_file(files, shown, file);
        }
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
    fn step(self, history: &mut History) -> Option<Selection> {
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
/// tab. Nothing is pushed -- `use_record_history` sees the selection change like any other
/// and `would_push` is false for it, because that entry is exactly what the cursor now
/// sits on.
///
/// It goes through [`activate`] rather than setting the selection itself because the
/// history and the open tabs are two different lists: the history is everywhere the reader
/// has been and keeps entries long after their tab was closed, so going back to one has to
/// be able to open a tab for it again.
fn navigate(
    open: State<Tabs<Selection>>,
    mut history: State<History>,
    selection: State<Selection>,
    nav: Nav,
) {
    // Ask before writing. `State::write` notifies its subscribers whether or not the
    // value it hands over changes, so back at the oldest entry -- or forward at the
    // newest -- must not reach for it at all: a no-op has to leave the history alone,
    // leave the selection on screen alone, and wake nothing.
    if !nav.possible(&history.peek()) {
        return;
    }

    // The guard is released at the end of this statement, before the selection is set
    // and `use_record_history` peeks the history back.
    let entry = nav.step(&mut history.write());
    if let Some(entry) = entry {
        activate(open, selection, entry);
    }
}

/// Reopen the previous session's binaries, tabs and selection, once, at startup.
///
/// `use_hook` runs its initializer on mount and never again, which is what makes this
/// happen exactly once; `spawn` is freya's own task spawner and is callable during
/// render (`use_future` is built out of the same two calls), so the reading and
/// parsing is off the UI thread from the first frame. Beyond that this is the
/// toolbar's `on_open` pattern verbatim -- CPU-bound `open_files` on a `std::thread`,
/// the result back over an `async_channel` -- so a large binary parses with the window
/// already up and interactive.
///
/// Every step degrades silently: no state file or a corrupt one is `None`, a path that
/// no longer exists or no longer parses just contributes no `Object` (`open_files`
/// swallows its own failures), `Project::resolve` falls back from a vanished symbol to
/// its object and from a vanished object to nothing, and `Project::resolve_history` and
/// `Project::resolve_tabs` drop what no longer points anywhere -- the history keeping
/// its cursor on the right one.
///
/// **Both strips are rebuilt through the functions that hold the app's invariants**,
/// never by writing either list directly, so a restored session is in a state the app
/// could have got into by hand: every content tab through [`activate`], every source
/// file through [`open_file`]. Two orderings follow from that and are the only
/// genuinely new rules here:
///
/// - The **tabs before the selection**. `activate` opens what it cannot find, so
///   restoring the selection first would leave its chip at the end of the strip instead
///   of in the place the reader left it. The other direction is safe: the selection can
///   have degraded to its object while the strip still holds the symbol, and `activate`
///   simply opens a tab for it, which is also what the reader would see had they closed
///   that tab themselves.
/// - The **shown source file last**, since `open_file` puts the pane on whatever it just
///   opened. It is asked for by name and answers with the copy already in the list, so
///   the second call moves the pane and adds nothing.
///
/// The one thing the restore cannot promise is that the pane *stays* on that file:
/// `use_open_source_file` follows the selection, so the moment `Lines` resolves for the
/// restored symbol the pane moves to that symbol's own file. That is the pane's rule and
/// not a lost restore -- clicking the same symbol in a running session does the same --
/// and the strip it moves within is the restored one either way.
///
/// Every write below happens in one go, before the frame can end: freya's effects are
/// woken by an async notify (`Effect::create`) rather than run at the write, so
/// `use_record_history` and `use_save_on_change` see the settled result once and not
/// each intermediate `Sel` the tab loop passes through.
fn use_restore_on_startup(
    open: State<Tabs<Selection>>,
    mut asm_at: State<Positions<Selection>>,
    objects: State<Vec<Arc<Object>>>,
    selection: State<Selection>,
    history: State<History>,
    files: State<Tabs<Arc<str>>>,
    mut src_at: State<Positions<Arc<str>>>,
    shown: State<Option<Arc<str>>>,
) {
    use_hook(move || {
        let Some(project) = Project::load() else {
            return;
        };
        if project.binaries.is_empty() {
            return;
        }

        spawn(async move {
            let (sender, receiver) = async_channel::bounded(1);
            let paths = project.binaries.clone();
            std::thread::spawn(move || {
                let _ = sender.send_blocking(open_files(paths));
            });

            let Ok(parsed) = receiver.recv().await else {
                return;
            };
            // Nothing opened: leave the app empty *and* leave the file alone, so a
            // binary that is only temporarily missing is not forgotten.
            if parsed.is_empty() {
                return;
            }

            let (mut objects, mut history) = (objects, history);
            objects.write().extend(parsed);

            // Resolved against everything now loaded rather than just `parsed`, so
            // this stays correct if the user managed to open something first. All
            // three are computed before any of them is set so the read guard is long
            // gone by the time anything is notified.
            let (restored_history, restored_tabs, restored_selection) = {
                let loaded = objects.read();
                (
                    project.resolve_history(&loaded),
                    project.resolve_tabs(&loaded),
                    project.resolve(&loaded),
                )
            };

            // The history first, so that when `use_record_history` observes the
            // selection there is already a cursor to dedup against. The saved cursor
            // entry is the saved selection -- that is what put it there -- and the two
            // resolve through the same lookup to the same `Arc`s, so `would_push` is
            // false and the restored session costs no duplicate entry. It is only when
            // the cursor entry was dropped, or the selection degraded, that the two
            // differ, and then a push is exactly right: the app is somewhere new.
            history.set(restored_history);

            // The strip, oldest chip first, and then the one that was active. Each of
            // these is a `Sel` write that will be overwritten by the next, which is the
            // price of there being exactly one way to open a content tab; the last one
            // is the only one anything observes.
            //
            // Where each tab was left goes in *before* the tab is opened, and this is the
            // one place either map is written from outside a pane. A pane restores its
            // position when it notices the tab it is showing has changed, so a row that
            // arrived after the `activate` would arrive after the only moment it is
            // looked at.
            {
                let mut at = asm_at.write();
                for (tab, row) in &restored_tabs {
                    at.remember(tab.clone(), *row);
                }
            }
            for (tab, _) in restored_tabs {
                activate(open, selection, tab);
            }
            activate(open, selection, restored_selection);

            // The Source pane's strip, which needs no resolving: a path that is gone is
            // still a tab, showing the pane's own "Source file not found".
            let restored_sources = project.resolve_sources();
            {
                let mut at = src_at.write();
                for (file, row) in &restored_sources {
                    at.remember(file.clone(), *row);
                }
            }
            for (file, _) in &restored_sources {
                open_file(files, shown, file.clone());
            }
            if let Some(file) = project.shown_source() {
                open_file(files, shown, Arc::from(file));
            }
        });
    });
}

pub fn app() -> impl IntoElement {
    // The tooltip is a freya component whose theme hardcodes a 14px font, and the
    // theme is the only way in: its `theme` override field is private. floem's
    // tooltip was a plain label that inherited the interface font, so hand the theme
    // the interface size; the family it does inherit from the row it is attached to.
    use_init_theme(|| {
        let mut theme = Theme::default();

        if let Some(tooltip) = theme.get::<TooltipThemePreference>("tooltip").cloned() {
            theme.set(
                "tooltip",
                TooltipThemePreference {
                    font_size: Preference::Specific(fonts().ui.size),
                    ..tooltip
                },
            );
        }

        theme
    });

    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    let selection = use_provide_context(|| Sel(State::create(Selection::None))).0;
    // The places open in the content area, of which `selection` is the active one, and the
    // source files open in the Source pane, of which `shown` is. Both lists are opened and
    // closed only through `activate`/`close_tab` and `open_file`/`close_file`, which is
    // what keeps "the selection is the active tab" and "a file is shown exactly when one
    // is open" invariants rather than conventions -- for the startup restore as much as
    // for a click, since both lists are now part of the saved session.
    let open = use_provide_context(|| Open(State::create(Tabs::default()))).0;
    let files = use_provide_context(|| Files(State::create(Tabs::default()))).0;
    let shown = use_provide_context(|| Shown(State::create(None))).0;
    // Where each of those tabs was left, which is a view of the two lists rather than a
    // second copy of them: an entry appears when a pane is scrolled and goes when the tab
    // it belongs to is closed, so the same five functions hold this true as hold the
    // lists themselves.
    let asm_at = use_provide_context(|| AsmAt(State::create(Positions::default()))).0;
    let src_at = use_provide_context(|| SrcAt(State::create(Positions::default()))).0;
    let history = use_provide_context(|| Hist(State::create(History::default()))).0;
    // Where the pointer is pointing, which the assembly and source panes answer for each
    // other. A plain state like the three above rather than something derived from them:
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
    use_save_on_change(
        objects, open, asm_at, selection, history, files, src_at, shown,
    );
    use_record_history(selection, history);
    use_clear_focus(selection, focused, pinned);
    use_clear_marks(selection, shown, marked);
    use_periodic_save();
    // After the save effect on purpose: the effect is in place, with the save policy's
    // empty baseline, before the restore can put anything into any of the states it
    // observes, so the restored session is seen by it as an ordinary change.
    use_restore_on_startup(
        open, asm_at, objects, selection, history, files, src_at, shown,
    );

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

    // The selected symbol's line info, worked out once for every pane that wants it.
    // `use_memo` re-runs on a change to anything read inside it, so this follows the
    // selection whether or not the Source tab is on screen -- which costs nothing in the
    // default layout, where it is. The query itself is the expensive part and runs here,
    // on the UI thread, against `line.rs`'s own note that it is worker-thread work: the
    // first one against a big binary builds the whole DWARF context (267 MB for
    // `viewer-sample`) and will visibly stall the frame. Moving it off is Step 11's item,
    // and it should move `assembly()` with it rather than one of the two alone.
    let lines = use_memo(move || {
        // Cloned out rather than held: the read guard would otherwise be alive for the
        // whole of a query that can take a second.
        let selection = selection.read().clone();
        SymbolLines::new(&selection)
    });
    use_provide_context(move || Lines(lines));
    // Reads that memo and nothing else, so it cannot see a symbol beside another symbol's
    // line info. Registered after the memo it follows, for the obvious reason.
    use_open_source_file(lines, files, shown);

    // One docking area per resizable pane: the left one a column of Objects, then
    // Symbols with Info tabbed beside it, then History at the bottom -- which is
    // where the goal asks for it, and where it is visible without a click. The
    // cost is that the three groups start at equal heights, so the symbol list is
    // shorter than it was; the handles between them, and dragging History onto the
    // middle panel, are both one gesture away. The right one is the split view the
    // goals ask to be the default: the source a symbol was compiled from beside its
    // assembly, at equal widths. All six tabs share one `DockDrag<Tab>`, which
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
    let content_dock = use_state(|| DockArea::row(vec![vec![Tab::Assembly], vec![Tab::Source]]));
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
            Some(MouseButton::Back) => navigate(open, history, selection, Nav::Back),
            Some(MouseButton::Forward) => navigate(open, history, selection, Nav::Forward),
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
        .child(toolbar(objects))
        // `ResizableContainer` renders itself `.expanded()`, so it needs a parent
        // that has already been given the leftover height under the toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(split),
        )
}

/// The one test in this file that runs the UI rather than the logic under it.
///
/// Everything decided by cases lives in a framework-free module with its own tests
/// ([`crate::rows`] here), and this is deliberately not a second home for that. It exists
/// for the one class of bug those tests are blind to by construction: a `State` borrow
/// that is legal to the compiler and panics at the moment a gesture ends. `mark_release`
/// shipped holding a `peek` guard across its own write, so *every* mouse-up on a run
/// brought the window down, and no amount of testing `RowSelection` would have said a
/// word about it. A press, a sweep and a release through freya's own headless runner is
/// the smallest thing that would have.
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

    /// A scroll view wired the way both panes are: one `ScrollController` reused across
    /// every tab the pane shows, and `use_kept_position` between them.
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
                        .height(Size::px(ROW_HEIGHT))
                        .on_pointer_over(move |_| top.set(index))
                        .key(index)
                        .into()
                },
                controller,
            )
            .length(rows)
            .item_size(ROW_HEIGHT),
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

    #[test]
    fn a_swept_run_survives_the_button_coming_up() {
        let (mut test, marked) = TestingRunner::new(
            harness,
            (100., 100.).into(),
            |runner| {
                runner.provide_root_context(|| Shift(State::create(false)));
                runner.provide_root_context(|| Marked(State::create(None))).0
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
}

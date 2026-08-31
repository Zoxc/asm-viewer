use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

use async_io::Timer;
use freya::code_editor::{
    EditorLanguage, EditorSyntaxTheme, Rope, SyntaxBlocks, SyntaxHighlighter, TextNode,
};
use freya::prelude::*;
use rfd::AsyncFileDialog;

use analysis::{open_files, Assembly, LineInfo, Object, SpanKind, Symbol, SymbolData};

use crate::filter::{Filter, Matcher};
use crate::fonts::{fonts, Font};
use crate::history::History;
use crate::project::{self, Project, Selection};
use crate::source::{self, SourceFile};

/// Height of every row in the object, symbol and instruction lists. This must stay
/// equal to the `item_size` given to each `VirtualScrollView`.
const ROW_HEIGHT: f32 = 26.0;

/// The height of the strip a filter bar's text box sits in. Taller than `ROW_HEIGHT` by
/// the room an `Input`'s border and its own inner margin need; it is a bar and not a row,
/// and nothing lines up with it.
const FILTER_HEIGHT: f32 = 32.0;

/// The side of one of the three square toggle buttons.
const TOGGLE_SIZE: f32 = 22.0;

/// How many rows above a row scrolled into view from the other pane are kept on screen.
/// A line landing against the top edge answers "what is this" without answering "where in
/// the function is it", which is half of why the two panes are side by side at all.
const CONTEXT_ROWS: f32 = 3.0;

// Palette, carried over from the original floem styling.
const HEADER_BG: Color = Color::from_rgb(245, 245, 245); // WHITE_SMOKE
const HAIRLINE: Color = Color::from_rgb(211, 211, 211); // LIGHT_GRAY
const SELECTED_BG: Color = Color::from_rgb(211, 211, 211);
const OBJECT_HOVER_BG: Color = Color::from_rgb(144, 238, 144); // LIGHT_GREEN
const SYMBOL_PANE_BG: Color = Color::from_rgb(243, 243, 228);
const SYMBOL_HOVER_BG: Color = Color::from_rgb(226, 226, 205);
const ASM_PANE_BG: Color = Color::from_rgb(248, 248, 248);
/// The pointer's own hover, on an instruction row and on a source line alike: both panes
/// show code, and one colour for "the pointer is here" reads across them as one gesture.
const CODE_ROW_HOVER_BG: Color = Color::from_argb(160, 228, 237, 216);
/// The cross-view highlight: this row is what the row the pointer is on maps to on the
/// other side. Weaker than the hover, and translucent like it, so a row carrying both
/// comes out as the hover *over* this rather than as one or the other -- see `blend`.
const LINE_FOCUS_BG: Color = Color::from_argb(70, 120, 160, 220);
/// The same highlight, made to stay by a click. The one colour in two strengths rather
/// than two colours, because it is the one relationship: a pin is the position the reader
/// asked to keep, and the pointer wandering off to a second one must not make the two
/// indistinguishable.
const LINE_PIN_BG: Color = Color::from_argb(120, 120, 160, 220);
const ADDRESS_FG: Color = Color::from_rgb(118, 141, 169);
const MNEMONIC_FG: Color = Color::from_rgb(116, 94, 147);
const REGISTER_FG: Color = Color::from_rgb(87, 103, 65);
const NUMBER_FG: Color = Color::from_rgb(80, 107, 135);
const OTHER_FG: Color = Color::from_rgb(102, 102, 102);
const RELOC_FG: Color = Color::from_rgb(50, 50, 50);
const RELOC_HOVER_FG: Color = Color::from_rgb(105, 89, 132);
/// The wash over the half of a panel a dragged tab would land in.
const DROP_PREVIEW_BG: Color = Color::from_argb(60, 105, 89, 132);
/// A filter toggle that is on, and one the pointer is over. Two shades of the header's
/// own grey rather than a colour of their own: a 22px square is small enough that "this
/// one is pressed" has to be read from how dark it is, and against `HEADER_BG` these are
/// the two steps that are legible without looking like a third kind of thing.
const TOGGLE_ON_BG: Color = Color::from_rgb(196, 196, 196);
const TOGGLE_HOVER_BG: Color = Color::from_rgb(225, 225, 225);
/// What a pattern that will not compile, and the reason it will not, are written in.
const INVALID_FG: Color = Color::from_rgb(176, 0, 32);

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
#[derive(Clone, Copy)]
struct Sel(State<Selection>);

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
/// nothing. Compared by pointer, like every other `Arc` the UI passes around.
///
/// Worked out once for all its readers rather than once per pane: `Object::line_info`
/// walks the line program of every unit covering the symbol again on each call, even
/// though the DWARF context itself is built only once.
#[derive(Clone)]
struct SymbolLines(Option<Arc<LineInfo>>);

impl PartialEq for SymbolLines {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
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

/// A loaded, highlighted source file, compared by pointer.
#[derive(Clone)]
struct SourceText(Arc<Highlighted>);

impl PartialEq for SourceText {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A disassembled symbol and what says where its instructions came from, compared by
/// pointer.
#[derive(Clone)]
struct AsmData {
    assembly: Arc<Assembly>,
    object: Arc<Object>,
    lines: SymbolLines,
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.object, &other.object)
            && self.lines == other.lines
    }
}

impl AsmData {
    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.0.as_ref()?;
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
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.focus == other.focus
            && self.pin == other.pin
    }
}

/// What the instruction rows are built from: the disassembly, and the two positions the
/// source pane is pointing at. Kept apart from `AsmData` so that a hover, which changes
/// this and not that, cannot re-run anything the disassembly drives.
#[derive(Clone, PartialEq)]
struct AsmRows {
    data: AsmData,
    focus: Option<LinePos>,
    pin: Option<LinePos>,
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
}

fn bottom_hairline() -> Border {
    Border::new().fill(HAIRLINE).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.5,
        left: 0.0,
    })
}

fn right_hairline() -> Border {
    Border::new().fill(HAIRLINE).width(BorderWidth {
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
        .background(Color::WHITE)
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

/// The background of a code row: the pointer's own hover, the cross-view highlight it got
/// from the other pane, the stronger one a click pinned there, or the hover over either.
///
/// A row that is both pinned and pointed at is painted as pinned, the stronger of the two
/// saying everything the weaker would.
fn row_background(hovering: bool, focused: bool, pinned: bool) -> Color {
    let cross = match (pinned, focused) {
        (true, _) => LINE_PIN_BG,
        (false, true) => LINE_FOCUS_BG,
        (false, false) => Color::TRANSPARENT,
    };

    // `blend` over a transparent bottom is the top colour unchanged, so an unlit hovered
    // row comes out as the hover alone without a case of its own.
    if hovering {
        blend(CODE_ROW_HOVER_BG, cross)
    } else {
        cross
    }
}

fn kind_color(kind: SpanKind) -> Color {
    match kind {
        SpanKind::Mnemonic | SpanKind::Prefix => MNEMONIC_FG,
        SpanKind::Register => REGISTER_FG,
        SpanKind::Number => NUMBER_FG,
        SpanKind::Address => ADDRESS_FG,
        SpanKind::Other => OTHER_FG,
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
    file: Arc<SourceFile>,
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
    fn new(file: Arc<SourceFile>) -> Highlighted {
        let rope = Rope::from_str(file.text());
        let theme = EditorSyntaxTheme::light();

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
            file,
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
    let file = Arc::new(Highlighted::new(source::load(path)?));

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
    /// No icon font: freya's Lucide set is behind a feature of its own and three toggles
    /// do not earn a dependency. Two of the three have a better answer than a picture
    /// anyway — `\b` and `.*` *are* the regex the toggle turns on, written out — and `Aa`
    /// is what every search box writes for case. The words are in the tooltip.
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
            TOGGLE_ON_BG
        } else if hovering() {
            TOGGLE_HOVER_BG
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
            .background(HEADER_BG)
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
                            input.color(INVALID_FG).focus_border_fill(INVALID_FG)
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
                    .child(label().text(error).color(INVALID_FG).max_lines(1))
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
// Rows
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ObjectRow {
    object: Arc<Object>,
    selected: bool,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && self.selected == other.selected
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
        let selection = use_consume::<Sel>().0;
        let object = self.object.clone();

        let background = if self.selected {
            SELECTED_BG
        } else if hovering() {
            OBJECT_HOVER_BG
        } else {
            Color::TRANSPARENT
        };

        rect()
            .width(Size::fill())
            .height(Size::px(ROW_HEIGHT))
            .padding(5.0)
            .background(background)
            .overflow(Overflow::Clip)
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(move |_| {
                let mut selection = selection;
                selection.set(Selection::Object(object.clone()));
            })
            .child(label().text(self.object.name.clone()).max_lines(1))
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
        let symbol = self.symbols.0[self.index].clone();
        let text = symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone();

        let background = if self.selected {
            SELECTED_BG
        } else if hovering() {
            SYMBOL_HOVER_BG
        } else {
            Color::TRANSPARENT
        };

        TooltipContainer::new(Tooltip::new(text.clone())).child(
            rect()
                .width(Size::fill())
                .height(Size::px(ROW_HEIGHT))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    let mut selection = selection;
                    selection.set(Selection::Symbol(symbol.clone()));
                })
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a history entry is called: the same demangled name the symbol list shows, or
/// the object's name for an object entry. `Selection::None` never reaches the history
/// -- `History::push` refuses it -- so its arm is unreachable in practice.
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

/// The pointer identity of what a history entry points at, for keying its row. Paired
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
        // Consuming does not subscribe -- only reading would, and this row never reads
        // the history; it only hands an index back to `navigate`.
        let history = use_consume::<Hist>().0;
        let index = self.index;
        let text = entry_text(&self.entry);

        let background = if self.current {
            SELECTED_BG
        } else if hovering() {
            SYMBOL_HOVER_BG
        } else {
            Color::TRANSPARENT
        };

        TooltipContainer::new(Tooltip::new(text.clone())).child(
            rect()
                .width(Size::fill())
                .height(Size::px(ROW_HEIGHT))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| navigate(history, selection, Nav::To(index)))
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
                    rect.background(Color::from_af32rgb(0.6, 255, 255, 255))
                        .corner_radius(6.0)
                        .border(Border::new().fill(RELOC_HOVER_FG).width(BorderWidth {
                            top: 0.0,
                            right: 0.0,
                            bottom: 2.0,
                            left: 0.0,
                        }))
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // A press bubbles, and the row under this label pins the line the
                    // instruction came from. Clicking the link means "go there", not "and
                    // also pin the line I am leaving", so the row never sees it.
                    e.stop_propagation();

                    let mut selection = selection;
                    selection.set(Selection::Symbol(symbol.clone()));
                })
                .child(label().text(text).max_lines(1).color(if hovering() {
                    RELOC_HOVER_FG
                } else {
                    RELOC_FG
                })),
        )
    }
}

#[derive(Clone)]
struct InstructionRow {
    data: AsmData,
    index: usize,
    /// Whether the source line the pointer is on is the one this instruction was compiled
    /// from. Worked out by the list rather than read from the focus here, so that a focus
    /// moving between two instructions of one line leaves every row untouched.
    focused: bool,
    /// Whether the source line a click pinned is that same line.
    pinned: bool,
    key: DiffKey,
}

impl PartialEq for InstructionRow {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
            && self.index == other.index
            && self.focused == other.focused
            && self.pinned == other.pinned
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
            .padding(3.0)
            .assembly_font()
            .background(row_background(hovering(), self.focused, self.pinned))
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                focused.set_if_modified(taken.clone());
            })
            .on_pointer_out(move |_| {
                hovering.set_if_modified(false);
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
            .child(
                label()
                    .text(format!("{:016X} ", instruction.address))
                    .min_width(Size::px(200.0))
                    .color(ADDRESS_FG)
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
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.index == other.index
            && self.focused == other.focused
            && self.pinned == other.pinned
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
            .background(row_background(hovering(), self.focused, self.pinned))
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                focused.set_if_modified(taken.clone());
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
                    .color(ADDRESS_FG)
                    .max_lines(1),
            )
            .child(paragraph().max_lines(1).spans_iter(spans.into_iter()))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
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

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .padding(5.0)
            .background(ASM_PANE_BG)
            .child(InstructionList {
                assembly,
                object: self.symbol.object.clone(),
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
    object: Arc<Object>,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly) && Arc::ptr_eq(&self.object, &other.object)
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

        let mut controller = use_scroll_controller(ScrollConfig::default);
        // How tall the list is, which `reveal_row` needs to know whether the row it was
        // asked for is on screen already. `VirtualScrollView` measures itself but keeps
        // the answer, so the rect wrapping it -- the same box, since the view is
        // `Size::fill()` inside it -- is measured here instead.
        let mut viewport = use_state(|| 0.0f32);

        let data = AsmData {
            assembly: self.assembly.clone(),
            object: self.object.clone(),
            lines,
        };
        let length = data.assembly.instructions.len();

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
            .on_sized(move |e: Event<SizedEventData>| viewport.set_if_modified(e.area.height()))
            .child(
                VirtualScrollView::new_with_data_controlled(
                    AsmRows { data, focus, pin },
                    |i, rows: &AsmRows| {
                        let (focused, pinned) = rows.lit(i);
                        InstructionRow {
                            data: rows.data.clone(),
                            index: i,
                            focused,
                            pinned,
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

/// The source the selected symbol was compiled from, as far as the debug info and this
/// machine's filesystem can say between them.
#[derive(Clone, PartialEq)]
struct SourceView {
    symbol: Symbol,
    lines: SymbolLines,
}

impl Component for SourceView {
    fn render(&self) -> impl IntoElement {
        let Some(lines) = &self.lines.0 else {
            return placeholder("No line info");
        };

        // Which of the files a symbol touches to open on: the one its first instruction
        // was compiled from, which is the function's own file rather than one of the
        // headers it inlined further in. A symbol whose entry instructions belong to no
        // row at all -- a compiler-generated prologue is enough for that -- falls back to
        // the first file the rows name.
        let file = lines
            .row_at(self.symbol.data.address)
            .and_then(|row| row.file)
            .and_then(|file| lines.files().get(file))
            .or_else(|| lines.files().first())
            .cloned();

        // There are always rows, since `LineInfo` is `None` rather than empty, but every
        // one of them may name no file -- which tells the reader as little as no line
        // info at all does, so it says the same thing.
        let Some(file) = file else {
            return placeholder("No line info");
        };

        // Named in the message because the path is the only clue to *why*: source built
        // on another machine, moved, or deleted since all look alike from here.
        let Some(source) = source_text(Path::new(&*file)) else {
            return placeholder(format!("Source file not found: {file}"));
        };

        // Which file is on screen is not otherwise visible anywhere: the tab is called
        // "Source" whatever it is showing, and a symbol's rows can name several files.
        let path = source.0.file.path().display().to_string();

        rect()
            .expanded()
            // The header takes its own height and the list is given the rest, which
            // torin only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(Color::WHITE)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(ROW_HEIGHT))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 8.0))
                    .background(HEADER_BG)
                    .border(bottom_hairline())
                    .overflow(Overflow::Clip)
                    .child(label().text(path).max_lines(1)),
            )
            .child(SourceList { source, file })
            .into()
    }
}

/// The source rows themselves, split out of `SourceView` the way `InstructionList` is out
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
                            },
                            |i, data: &SourceData| {
                                let line = Some(i as u32 + 1);
                                SourceRow {
                                    source: data.source.clone(),
                                    file: data.file.clone(),
                                    index: i,
                                    focused: data.focus == line,
                                    pinned: data.pin == line,
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
        .child(info_line(format!("Size: {} bytes", data.size)))
        .child(info_line(format!(
            "Data Length: `{:?}`",
            data.data().map(|d| d.len()).unwrap_or_default()
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
        let current = use_consume::<Sel>().0.read().clone();
        let filter = use_state(Filter::default);
        // Filtered where the rows are built rather than in a memo of its own: a file
        // contributes one object and an archive one per member, so this is tens of names
        // and not the symbol list's hundred thousand.
        let matcher = filter.read().matcher();

        let rows: Vec<Element> = objects
            .read()
            .iter()
            .filter(|object| matcher.matches(&object.name))
            .map(|object| {
                ObjectRow {
                    object: object.clone(),
                    selected: matches!(&current, Selection::Object(selected) if Arc::ptr_eq(selected, object)),
                    key: DiffKey::None,
                }
                .key(Arc::as_ptr(object).addr())
                .into()
            })
            .collect();

        // The list used to sit in a flex column and grow without bound; a tab has
        // a height of its own, so it scrolls instead of being clipped.
        filter_pane(
            filter,
            Color::WHITE,
            ScrollView::new().child(rect().width(Size::fill()).children(rows).into_element()),
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
            SYMBOL_PANE_BG,
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
                .background(Color::WHITE)
                .child(info_line(format!("Object: `{}`", object.name)))
                .child(info_line(format!("Format: {:?}", object.format)))
                .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
                .into(),
            Selection::Symbol(symbol) => rect()
                .expanded()
                .background(Color::WHITE)
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
            SYMBOL_PANE_BG,
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
        let current = use_consume::<Sel>().0.read().clone();
        // Reading the memo subscribes this tab to it, so the pane fills in when the line
        // info for a newly selected symbol is worked out, without the root re-rendering.
        let lines = use_consume::<Lines>().0.read().clone();

        match &current {
            Selection::Symbol(symbol) => SourceView {
                symbol: symbol.clone(),
                lines,
            }
            .into_element(),
            _ => placeholder("No symbol selected"),
        }
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
/// reads like the old `HEADER_BG` strip.
fn tab_label(tab: Tab, background: Color) -> impl IntoElement {
    rect()
        .height(Size::px(ROW_HEIGHT))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(background)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(label().text(tab.title()).max_lines(1))
}

fn tab_header(ctx: TabContext<Tab>, area: State<DockArea>) -> Element {
    let background = if ctx.is_drop_target {
        SELECTED_BG
    } else if area.read().is_active(ctx.tab_id) {
        Color::WHITE
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
        .child(tab_label(tab, SELECTED_BG))
        .into_element()
}

fn tab_bar(ctx: TabBarContext<PanelId>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(ROW_HEIGHT))
        .horizontal()
        .background(HEADER_BG)
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
            .background(DROP_PREVIEW_BG),
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
/// it changes (`freya-core/src/lifecycle/effect.rs`), so reading the three state
/// contexts here makes this one observer the single choke point every mutation flows
/// through: the three `selection.set(..)` sites, the toolbar's `objects.write()` and
/// `use_record_history`'s push know nothing about persistence, and neither will any
/// future one.
///
/// Whether a change reaches the disk now or at the next `use_periodic_save` tick is
/// `project::record`'s decision, not this one's: opening a binary is written at once,
/// a selection or a history entry is left pending. That policy is framework-free and
/// unit-tested in `project.rs`; all this hook owns is *when to look*.
///
/// A selection change wakes this twice -- once for `Sel`, and again when
/// `use_record_history` pushes the entry that follows from it -- which costs two
/// derivations and two comparisons and, since neither is a binaries change, no write
/// at all.
fn use_save_on_change(
    objects: State<Vec<Arc<Object>>>,
    selection: State<Selection>,
    history: State<History>,
) {
    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it.
        project::record(Project::from_state(
            &objects.read(),
            &selection.read(),
            &history.read(),
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
/// to come share the same two steps: move the cursor, then set the selection to the
/// entry it landed on. Nothing is pushed -- `use_record_history` sees the selection
/// change like any other and `would_push` is false for it, because that entry is
/// exactly what the cursor now sits on.
fn navigate(mut history: State<History>, mut selection: State<Selection>, nav: Nav) {
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
        selection.set(entry);
    }
}

/// Reopen the previous session's binaries and selection, once, at startup.
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
/// its object and from a vanished object to nothing, and `Project::resolve_history`
/// drops the entries that no longer point anywhere while keeping the cursor on the
/// right one.
fn use_restore_on_startup(
    objects: State<Vec<Arc<Object>>>,
    selection: State<Selection>,
    history: State<History>,
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

            let (mut objects, mut selection, mut history) = (objects, selection, history);
            objects.write().extend(parsed);

            // Resolved against everything now loaded rather than just `parsed`, so
            // this stays correct if the user managed to open something first. Both
            // are computed before either is set so the read guard is long gone by
            // the time anything is notified.
            let (restored_history, restored_selection) = {
                let loaded = objects.read();
                (project.resolve_history(&loaded), project.resolve(&loaded))
            };

            // The history first, so that when `use_record_history` observes the
            // selection there is already a cursor to dedup against. The saved cursor
            // entry is the saved selection -- that is what put it there -- and the two
            // resolve through the same lookup to the same `Arc`s, so `would_push` is
            // false and the restored session costs no duplicate entry. It is only when
            // the cursor entry was dropped, or the selection degraded, that the two
            // differ, and then a push is exactly right: the app is somewhere new.
            history.set(restored_history);
            selection.set(restored_selection);
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
    let history = use_provide_context(|| Hist(State::create(History::default()))).0;
    // Where the pointer is pointing, which the assembly and source panes answer for each
    // other. A plain state like the three above rather than something derived from them:
    // it is an input, written by whichever row the pointer is on.
    let focused = use_provide_context(|| Focused(State::create(None))).0;
    // Where a click fixed the two panes, which outlives the pointer moving on and is what
    // asks the other pane to scroll. Beside the focus rather than inside it because the
    // two answer different questions and a row can be either, neither or both.
    let pinned = use_provide_context(|| Pinned(State::create(None))).0;
    use_save_on_change(objects, selection, history);
    use_record_history(selection, history);
    use_clear_focus(selection, focused, pinned);
    use_periodic_save();
    // After the save effect on purpose: the effect is in place, with the save policy's
    // empty baseline, before the restore can put anything into any of the three states,
    // so the restored session is seen by it as an ordinary change.
    use_restore_on_startup(objects, selection, history);

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
        SymbolLines(match &selection {
            Selection::Symbol(symbol) => symbol.data.line_info(&symbol.object),
            _ => None,
        })
    });
    use_provide_context(move || Lines(lines));

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
                .child(docking_area(content_dock)),
        );

    rect()
        .expanded()
        .content(Content::Flex)
        .interface_font()
        .background(Color::WHITE)
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
            Some(MouseButton::Back) => navigate(history, selection, Nav::Back),
            Some(MouseButton::Forward) => navigate(history, selection, Nav::Forward),
            _ => {}
        })
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

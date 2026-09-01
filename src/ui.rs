//! The freya UI.
//!
//! The module is a directory of its own, and every one of its files is a **cut out of one
//! 8 700-line file** rather than a boundary designed from scratch: what each holds is
//! exactly what the section banners and the cross-references in `AGENTS.md` already said
//! belonged together, and nothing changed on the way across.
//!
//! Two mechanical decisions come with that and are worth stating once, here, rather than
//! being rediscovered in each file.
//!
//! **The imports below are the module's own prelude.** They are `pub(crate) use` and every
//! file under this one begins `use super::*;`, so each keeps the exact set of names it had
//! while it was a section of one file. The alternative -- a tailored import block per file
//! -- is tidier and buys nothing here: these files are the halves of one UI and they use
//! one another's types freely, so the tailored blocks would be eighteen copies of most of
//! this one.
//!
//! **Each file is re-exported straight back out.** A `mod x;` is followed by a
//! `pub(crate) use x::*;`, so a name means what it meant before the split wherever it is
//! written, `src/ui/tests.rs`'s `use super::*` included. The globs cannot collide, by
//! construction: every one of these names already shared a single namespace.
//!
//! **What is `pub(crate)` is what the compiler asked for and no more.** The blanket
//! alternative was a `pub(crate)` on all four hundred-odd items, fields and methods, which
//! is one line of script and a worse answer: with the visibility minimal, the annotations
//! *are* the list of what crosses a file boundary, and a struct whose fields are still
//! private is a struct nothing outside its file takes apart. It is `pub(crate)` and never
//! `pub` because dead-code analysis still runs on a `pub(crate)` item, so nothing here can
//! quietly stop being used and stay compiled.

pub(crate) use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::Duration,
};

pub(crate) use async_io::Timer;
pub(crate) use freya::code_editor::{
    CodeEditor, CodeEditorData, EditorLanguage, EditorSyntaxTheme, EditorThemePartialExt, Rope,
    SyntaxBlocks, SyntaxHighlighter, TextNode,
};
pub(crate) use freya::icons::lucide;
pub(crate) use freya::prelude::*;
pub(crate) use rfd::AsyncFileDialog;

pub(crate) use analysis::{
    open_files_streaming, Assembly, Instruction, LineInfo, Object, Progress, SpanKind, Symbol,
    SymbolData,
};

pub(crate) use crate::docs::{DocId, Docs};
pub(crate) use crate::filter::{Filter, Matcher};
pub(crate) use crate::fonts::{self, Font, Fonts};
pub(crate) use crate::history::History;
pub(crate) use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
pub(crate) use crate::project::{
    self, Details, Document, Project, ProjectId, Recent, Selection, Session,
};
pub(crate) use crate::rows::RowSelection;
pub(crate) use crate::scratchpad::{
    Build, Dependency, Diagnostic, Ended, Failure, Half, Level, Problem, RunEvent, RunOutput,
    Running, Scratchpad, Stream,
};
pub(crate) use crate::settings::{Appearance, FontSetting, Settings, Theme as ThemeChoice};
pub(crate) use crate::source::{self, SourceFile};
pub(crate) use crate::tabs::{self, Positions};
pub(crate) use crate::tree::{
    format_tag, Expansion, LoadId, Loads, ObjectTree, TreeRow, ARCHIVE_TAG,
};

mod metrics;
pub(crate) use metrics::*;
mod palette;
pub(crate) use palette::*;
mod state;
pub(crate) use state::*;

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
    is_open: impl Fn(&T) -> bool + 'static,
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
            // Only for a tab that is still open, which is why `is_open` is an argument
            // here at all: `close_tab` forgets a tab's position and then moves to a
            // neighbour, so the run that follows is holding a tab that has gone -- and
            // writing its row down would put it straight back, keyed by a `Document`
            // that holds a whole `Object`. That the last scroll before a close is lost
            // with it is the right answer twice over: there is no tab to bring it back
            // for, and the file it pointed into may be being let go of in the same
            // breath (`close_binary`).
            //
            // **It has to be asked of the states themselves, never of a `Memo` over
            // them.** A memo is recomputed by a task woken on a notify, so it can still
            // be reporting a just-closed tab as open during exactly the run this guard
            // exists for, and the resurrection would be back. The two real call sites
            // ask `Docs`, which the close has already written.
            let still_open = is_open(&owner);
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
fn activate(open: Open, mut history: State<History>, target: Option<Document>, visit: Visit) {
    let Open { mut dock, mut docs } = open;

    let Some(target) = target else {
        // Nothing to show. With views tabbed in the same panel there is usually still a
        // tab to be on, and falling to the first of them is what keeps a panel that has
        // tabs from having none of them active; an empty panel draws its own ground.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            let first = panel.tabs.first().copied();
            panel.active_tab_id = first;
        }
        return;
    };

    // The copy that is *in the table* where there is one, so the identity a position is
    // keyed by does not change when the same file is reached again through a different
    // symbol's `LineInfo`: two of them naming one path hold two `Arc<str>`s of it. The
    // read is bound and dropped before the write below, never held across it.
    let existing = docs.peek().id_of(&target);
    let id = match existing {
        Some(id) => id,
        None => docs.write().open(target.clone()),
    };
    let target = docs.peek().get(id).cloned();

    // Asked before it is written, which is what keeps re-focusing the tab that is
    // already on top from waking every pane that draws a document: `State::write`
    // notifies its subscribers whether or not the value it hands over changes. This is
    // `set_if_modified`'s job, done by hand because the value is a tree.
    let tab = Tab::Document(id);
    let settled = dock
        .peek()
        .document_panel()
        .is_some_and(|panel| panel.active_tab_id == Some(tab) && panel.tabs.contains(&tab));
    if !settled {
        dock.write().show_document(tab);
    }

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
    open: Open,
    history: State<History>,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    entry: &Document,
) {
    let Open { mut dock, mut docs } = open;
    let Some(id) = docs.peek().id_of(entry) else {
        return;
    };
    let tab = Tab::Document(id);

    // Worked out before anything is removed, which is what [`tabs::landing`] wants, and
    // in a scope of its own so no read guard is alive when the writes below start.
    let (was_showing, next) = {
        let dock = dock.peek();
        let Some(panel) = dock.document_panel() else {
            return;
        };
        (
            panel.active_tab_id == Some(tab),
            tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), |open| {
                *open == tab
            }),
        )
    };

    {
        // Removed by hand rather than through freya's `remove_tab_except`, which sets the
        // panel's active tab to `tabs.first()` when it takes the active one out. Landing
        // on the *neighbour* is a rule of this app, older than the list it is written
        // against, and letting the removal choose would quietly replace it.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel.tabs.retain(|open| *open != tab);
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    docs.write().close(id);
    asm_at.write().forget(entry);
    src_at.write().forget(entry);

    // A document landed on goes through `activate`, even though it is by construction
    // already open: it is a change of active document and there is one way to make one.
    // A *view* landed on is not a document at all, and the write above has already put
    // the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
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
    open: Open,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    mut history: State<History>,
    path: &Path,
) {
    let Open { mut dock, mut docs } = open;
    // Every guard below is taken out of its own statement or its own scope, so none of
    // them is still alive when the next write -- or `activate` at the end -- is reached.
    let showing = open.active();

    // Which tabs go, and what is left to be on, both worked out before anything is
    // removed. `closing` is asked of a *tab*: a view is never in a file, so the same walk
    // that closes a binary's functions leaves Project, Settings and the Scratchpad alone
    // for the same reason it leaves a source-driven tab alone.
    let (closing, next) = {
        let dock_ref = dock.peek();
        let docs_ref = docs.peek();
        let Some(panel) = dock_ref.document_panel() else {
            return;
        };
        let in_file = |tab: &Tab| match tab {
            Tab::Document(id) => docs_ref
                .get(*id)
                .is_some_and(|document| document.in_file(path)),
            Tab::View(_) => false,
        };
        let closing: Vec<Tab> = panel.tabs.iter().copied().filter(in_file).collect();
        let next = tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), in_file);
        (closing, next)
    };

    let was_showing = showing
        .as_ref()
        .is_some_and(|showing| showing.in_file(path));
    {
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel.tabs.retain(|tab| !closing.contains(tab));
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    {
        let mut docs = docs.write();
        for tab in &closing {
            if let Tab::Document(id) = tab {
                docs.close(*id);
            }
        }
    }

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

    // Through `activate` like every other change of active document, even though the tab
    // it lands on is by construction already open — which is what makes it a
    // [`Visit::Moved`], exactly as closing one tab by hand is. A view landed on is not a
    // document and the write above has already put the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
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
        history,
        ..
    } = states;

    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| close_binary(objects, loading, open, asm_at, src_at, history, &path))
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
        let (open, history) = (states.open, states.history);
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
        let open = use_open();
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
        let open = use_open();
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
                .on_press(move |_| navigate(open, history, Nav::To(index)))
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
        let open = use_open();
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

/// One document's tab header: the icon naming its kind, what it is called, an × that
/// closes it, and the pane's own white when it is the one on screen.
///
/// **Nothing here activates the tab.** freya wraps whatever a tab header returns in a
/// `DropZone` around a `rect().on_press(set_active)` around a `DragZone`, so pressing this
/// makes it the panel's active tab -- and since the active document is *derived* from
/// that, pressing it is what switches document. Which is also why the × must
/// `stop_propagation`: without it a close would first switch to the tab it is closing.
///
/// A stateless helper rather than a component, the hover state belonging to the component
/// that called this, so no hook runs here.
fn chip(
    icon: Element,
    text: String,
    tooltip: String,
    active: bool,
    mut hovering: State<bool>,
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
            .child(icon)
            .child(label().text(elide(&text)).max_lines(1))
            .child(
                rect()
                    // The press bubbles into freya's own wrapper, which activates the
                    // tab. Closing a tab is not a way of first switching to it.
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
/// The control that opens a list of every open document, pinned at the **right** of the
/// document panel's bar so it never scrolls away with the tabs it is there to reach.
///
/// The overflow answer. A tab past the right-hand edge of a scrolling bar is reachable
/// only by scrolling to it, and a reader who has opened thirty functions has no idea what
/// is out there -- so the bar carries one control that lists them all. **All of them, not
/// only the hidden ones**: which tabs are off-screen means measuring the bar's content
/// against its viewport, and a list that changes length as the reader drags the bar is
/// worse to use than a complete one. It is what every browser's tab list does.
///
/// **The popup is positioned here rather than through `ContextMenu`**, which is what every
/// other menu in the app uses. `ContextMenu` pins a menu's top-left corner to the pointer
/// and clamps to nothing, so opened from a button at the right-hand edge it would draw off
/// the side of the window. An absolute `right(0.)` inside this button's own box aligns the
/// popup's right edge with the button's and lets it open leftward, into the window.
///
/// The one thing that has to be copied from `ContextMenu` is its opening dance: `Menu`
/// closes itself on **any** global press, and the press that opens it is one, so the first
/// close request is swallowed.
#[derive(PartialEq)]
struct DocumentMenuButton;

impl Component for DocumentMenuButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;

        // Every tab in the panel and its active one, both read here so the menu is built
        // from one look at the dock. Views are in the list beside the documents: they are
        // tabs in the same bar and scroll off the same edge, so a list that left them out
        // would be a list of *some* of what is up there.
        let (tabs, active) = {
            let dock = open.dock.read();
            match dock.document_panel() {
                Some(panel) => (panel.tabs.clone(), panel.active_tab_id),
                None => (Vec::new(), None),
            }
        };
        if tabs.is_empty() {
            return rect().into_element();
        }

        let side = icon_size();
        let button = row_tooltip(
            "Open tabs".to_owned(),
            rect()
                .width(Size::px(DOCUMENT_MENU_WIDTH))
                .height(Size::px(list_row_height()))
                .main_align(Alignment::Center)
                .cross_align(Alignment::Center)
                .background(if showing() || hovering() {
                    palette().toggle_hover_bg
                } else {
                    Color::TRANSPARENT
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                // No guard against `Menu`'s own close-on-any-global-press, and none is
                // needed: the listeners for a global event are snapshotted when it is
                // *measured*, before any handler runs, and this opens on `on_press`, which
                // is derived from the same `MouseUp` that emits the global press. The menu
                // does not exist yet when that batch is built, so its close handler cannot
                // be in it. A popup opened from a `*_down` handler is the other case --
                // `ContextMenu`'s right-click menus are, which is why *they* carry the
                // swallow. Copying it here cost a click: the first press outside the menu
                // was eaten and dismissing it took two.
                .on_press(move |_| {
                    let was = showing();
                    showing.set(!was);
                })
                .child(
                    SvgViewer::new(("chevron-down", lucide::chevron_down()))
                        .width(Size::px(side))
                        .height(Size::px(side))
                        .color(palette().icon_fg)
                        .show_loader(false),
                ),
        );

        rect()
            .width(Size::px(DOCUMENT_MENU_WIDTH))
            .height(Size::px(list_row_height()))
            .child(button)
            .maybe_child(showing().then(|| {
                rect()
                    // Under the bar and aligned to its right-hand edge, so the list opens
                    // leftward into the window instead of off the side of it.
                    .position(Position::new_absolute().top(list_row_height()))
                    .child(
                        tabs_menu(open, history, &tabs, active, showing)
                            .on_close(move |_| showing.set(false))
                            // Keyed by how many rows it holds, so a list that grows while
                            // the menu is open remounts it. `MenuContainer` measures itself
                            // *once* and keeps the offset it worked out then, so a menu
                            // that widens after that keeps an offset for the width it used
                            // to be and hangs off the side of the window. Remounting is
                            // what makes it measure the size it actually is.
                            .key(tabs.len()),
                    )
                    .into_element()
            }))
            .into_element()
    }
}

/// The menu [`DocumentMenuButton`] opens: one row per tab in the document panel, in the
/// bar's own order, with the one on screen marked.
///
/// **Both kinds of tab**, since both scroll off the same edge — a view is reached from
/// here exactly as a document is, and each row wears the glyph its tab wears. The rows go
/// through `Tab::title`/`Tab::icon`, so neither kind needs a case here.
///
/// Built per press rather than kept, for `close_menu`'s reason: there is nothing to hold
/// on to between presses, and the list is a handful of rows.
fn tabs_menu(
    open: Open,
    history: State<History>,
    tabs: &[Tab],
    active: Option<Tab>,
    mut close: State<bool>,
) -> Menu {
    // Names and glyphs resolved in one pass, so the read guard on the table is gone
    // before any row's handler can run and write to it.
    let rows: Vec<(Tab, String, Element)> = {
        let docs = open.docs.read();
        tabs.iter()
            .map(|tab| (*tab, elide(&tab.title(&docs)), tab.icon(&docs)))
            .collect()
    };

    rows.into_iter()
        .fold(Menu::new(), |menu, (tab, title, icon)| {
            menu.child(
                // `MenuItem` and not `MenuButton`, which is what the file row's menu uses:
                // this one has a *current* row, and `selected` is freya's own way of drawing
                // it, so the marking follows the menu's theme instead of being a character
                // pushed in front of the name.
                MenuItem::new()
                    .selected(Some(tab) == active)
                    .on_press(move |_| {
                        match tab {
                            // A document already open is a place the reader has, so going to
                            // it is a move and records nothing -- the same rule pressing its
                            // tab obeys, and the reason `activate` is told why it is called.
                            Tab::Document(id) => {
                                let document = open.docs.peek().get(id).cloned();
                                activate(open, history, document, Visit::Moved);
                            }
                            // A view is not a document and never goes through `activate`:
                            // making it the panel's tab on top is the whole of showing it,
                            // and it is what pressing its header does too.
                            Tab::View(_) => {
                                let mut dock = open.dock;
                                let mut dock = dock.write();
                                if let Some(panel) = dock.document_panel_mut() {
                                    panel.active_tab_id = Some(tab);
                                }
                            }
                        }
                        close.set(false);
                    })
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(6.0)
                            .child(icon)
                            // `max_lines(1)`, or a name longer than the menu is wide wraps
                            // onto a second line and the row grows to hold it. The names are
                            // already cut to `CHIP_NAME_CHARS`, so one line is all any of
                            // them needs.
                            .child(label().text(title).max_lines(1)),
                    ),
            )
        })
}

fn chip_strip(mut chips: Vec<Element>, tab_count: usize) -> Element {
    // freya appends one more child than there are tabs: a `rect().expanded()` inside a
    // drop zone that drops past the last tab. `expanded()` is meaningless inside a
    // horizontal scroll view -- there is no leftover space to expand into -- so it is
    // given a width of its own and scrolls along with the tabs, staying the target it was
    // meant to be instead of collapsing to nothing.
    let filler = (chips.len() > tab_count).then(|| chips.split_off(tab_count));
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .horizontal()
        // The button takes its own width and the tabs are given the rest, which torin
        // only works out for a `flex` child of a `Content::Flex` parent.
        .content(Content::Flex)
        .background(palette().header_bg)
        .border(bottom_hairline())
        .child(
            ScrollView::new()
                .width(Size::flex(1.0))
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
                        .maybe_child(filler.map(|filler| {
                            rect()
                                .width(Size::px(DROP_PAST_LAST_TAB))
                                .height(Size::fill())
                                .children(filler)
                                .into_element()
                        }))
                        .into_element(),
                ),
        )
        .child(DocumentMenuButton)
        .into_element()
}

/// How wide the "drop past the last tab" target is in the document panel's bar. Enough to
/// aim at, narrow enough not to look like an empty tab.
const DROP_PAST_LAST_TAB: f32 = 24.0;

/// How wide [`DocumentMenuButton`] is. A square-ish target for one glyph.
const DOCUMENT_MENU_WIDTH: f32 = 26.0;

/// One open document's tab header, as the dock draws it.
///
/// A component and not a plain function because it has a hover state of its own, which is
/// what tells "about to close this tab" from "about to switch to it" -- the one piece of
/// feedback the dock's own view headers have never needed, there being no × on them.
#[derive(Clone)]
struct DocumentHeader {
    id: DocId,
    /// Whether this is the tab its panel is showing.
    active: bool,
    key: DiffKey,
}

impl PartialEq for DocumentHeader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.active == other.active
    }
}

impl KeyExt for DocumentHeader {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for DocumentHeader {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;
        let asm_at = use_consume::<AsmAt>().0;
        let src_at = use_consume::<SrcAt>().0;

        // A tab whose document has gone draws nothing rather than panicking in a render.
        // It should not be reachable -- a tab and its table entry are closed together.
        let Some(document) = open.docs.read().get(self.id).cloned() else {
            return rect().into_element();
        };
        let closed = document.clone();

        chip(
            entry_icon(&document),
            entry_text(&document),
            entry_tooltip(&document),
            self.active,
            hovering,
            move |_| close_tab(open, history, asm_at, src_at, &closed),
        )
        .into_element()
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
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<AsmAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
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
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<SrcAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
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

/// What can be a tab in the dock: one of the app's views, or one open document.
///
/// Two-kinded because the dock is now where *both* live. It is a handle and not the thing
/// itself in either arm, which is a type bound rather than a preference: freya's
/// `DockingModel::TabId` is `Copy + PartialEq + Hash + 'static`, and a [`Document`] holds
/// `Arc`s, compares by pointer identity and hashes by nothing at all. So a document is
/// carried as the [`DocId`] [`Docs`] knows it by.
///
/// The asymmetry between the two arms is deliberate and is enforced in
/// [`DockArea::on_drop`]: **a document may only ever be in the designated document panel,
/// while a view may be anywhere, that panel included.** One visible document is what lets
/// `Analysis`, `Marked`, `Focused` and `Pinned` each hold one answer for the window
/// instead of one per document; a view has no such constraint, so Project, Settings and
/// the Scratchpad stay tabbed beside the documents exactly as they were tabbed beside the
/// Assembly pane before.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tab {
    View(View),
    Document(DocId),
}

impl Tab {
    /// The label shown in the tab bar.
    ///
    /// A `String` and not a `&'static str` because a document is named after what it
    /// shows. `elide` is not applied here -- the header decides how much of a name it has
    /// room for.
    fn title(self, docs: &Docs) -> String {
        match self {
            Tab::View(view) => view.title().to_owned(),
            // A tab whose document has gone names nothing. It should not be reachable --
            // a tab and its `Docs` entry are closed together -- and drawing an empty
            // header is a better answer than panicking in a render.
            Tab::Document(id) => docs.get(id).map(entry_text).unwrap_or_default(),
        }
    }

    /// The Lucide glyph drawn before the title.
    ///
    /// A document wears the glyph its kind wears everywhere else, which is deliberately
    /// the same pair the Assembly and Source views wore: that is how the two kinds of tab
    /// are told apart, and it is the one thing that survived those two views being folded
    /// into a document's two sides.
    fn icon(self, docs: &Docs) -> Element {
        match self {
            Tab::View(view) => view.icon(),
            Tab::Document(id) => match docs.get(id) {
                Some(document) => entry_icon(document),
                None => rect().into_element(),
            },
        }
    }
}

/// One of the app's dockable views. A view is a persistent pane rather than a slot the
/// active document drives, so each one renders itself off the state it is about and
/// subscribes to it on its own -- which also keeps a change of document from re-rendering
/// the whole tree.
///
/// **This is where a pane that is not a document belongs.** A document is a place in a
/// binary or a source file, which is what makes the two code panes able to render it, the
/// history able to record it and the session able to write it down. A project, the
/// settings and a scratchpad's editor are none of those: there is one of each, they
/// resolve against no object and are no file on disk the panes could open, and neither
/// code pane could draw one. So they are views, where a singleton with its own state
/// already fits, rather than a third `Document` variant that every one of those five
/// places would need an answer for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum View {
    Objects,
    Symbols,
    Info,
    History,
    Project,
    Settings,
    Scratchpad,
}

impl View {
    /// The label shown in the tab bar.
    fn title(self) -> &'static str {
        match self {
            View::Objects => "Objects",
            View::Symbols => "Symbols",
            View::Info => "Info",
            View::History => "History",
            View::Project => "Project",
            View::Settings => "Settings",
            View::Scratchpad => "Scratchpad",
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
            View::Objects => ("package", lucide::package()),
            View::Symbols => ("square-function", lucide::square_function()),
            View::Info => ("info", lucide::info()),
            View::History => ("history", lucide::history()),
            View::Project => ("folder-open", lucide::folder_open()),
            View::Settings => ("settings", lucide::settings()),
            View::Scratchpad => ("notebook-pen", lucide::notebook_pen()),
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
            View::Objects => ObjectsTab.into_element(),
            View::Symbols => SymbolsTab.into_element(),
            View::Info => InfoTab.into_element(),
            View::History => HistoryTab.into_element(),
            View::Project => ProjectTab.into_element(),
            View::Settings => SettingsTab.into_element(),
            View::Scratchpad => ScratchpadTab.into_element(),
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
#[derive(Clone)]
struct AssemblyPane {
    document: Document,
}

impl PartialEq for AssemblyPane {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
    }
}

impl Component for AssemblyPane {
    fn render(&self) -> impl IntoElement {
        let analysis = use_consume::<Analysis>().0;

        let source_driven = matches!(self.document, Document::Source(_));
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
fn companion_header(open: Open, history: State<History>, file: Arc<str>) -> Element {
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
            .on_press(move |_| activate(open, history, Some(document.clone()), Visit::Went))
            .child(entry_icon(&Document::Source(file.clone())))
            .child(label().text(file_name(&file)).max_lines(1)),
    )
    .into_element()
}

/// The Source pane: the tab's source side, whichever of the two sides that is.
#[derive(Clone)]
struct SourcePane {
    document: Document,
}

impl PartialEq for SourcePane {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
    }
}

impl Component for SourcePane {
    fn render(&self) -> impl IntoElement {
        let open = use_open();
        let history = use_consume::<Hist>().0;
        // Consumed unconditionally, hooks having to run on every render, and read here
        // because the companion file comes out of it -- and because reading it is what
        // subscribes this tab to it, so the pane fills in when a newly selected symbol's
        // line info is worked out, without the root re-rendering.
        let analysis = use_consume::<Analysis>().0.read().clone();
        // The tab's own document and not `Active`: this pane is only ever mounted for the
        // tab it belongs to, and the document is in hand synchronously where `Active` is
        // a memo that catches up a beat later.
        let side = source_side(Some(&self.document), &analysis);

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
                SourceSide::Companion(file) => Some(companion_header(open, history, file.clone())),
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

/// The content area's panel that documents live in. The first one `DockArea::row` builds,
/// so it is where the reader's eye already is; see [`DockArea::documents`].
const DOCUMENT_PANEL: PanelId = 0;

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
    /// The panel documents live in, for the area that has one -- `Some` for the content
    /// area, `None` for the sidebar.
    ///
    /// **Not for the placeholder: for the opening.** A click in the symbol list opens a
    /// document, and that document has to land *somewhere*. A dock has many panels, the
    /// reader can drag things anywhere, and freya's `DockingModel` has no notion of "the
    /// panel documents belong to" -- so this names one. Everything else about it follows:
    /// it is exempt from [`DockArea::tidy`] so closing the last document cannot fold the
    /// content area away, it is the only panel [`DockArea::on_drop`] will let a document
    /// into, and it draws the app's own empty ground rather than "Drag a tab here".
    documents: Option<PanelId>,
    /// The side table, for the one question [`DockArea::on_drop`] has to ask about a
    /// document: whether it is still open. `Option` because it is wired up after the
    /// state exists, the way `other` is.
    docs: Option<State<Docs>>,
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
            documents: None,
            docs: None,
        }
    }

    /// Name `panel_id` the panel documents live in. See the field.
    fn with_documents(mut self, panel_id: PanelId) -> Self {
        self.documents = Some(panel_id);
        self
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

    /// The panel documents live in, for the area that has one.
    fn document_panel(&self) -> Option<&DockPanel<Tab, PanelId>> {
        self.tree.panel(&self.documents?)
    }

    /// The same panel, to write into. Every change to what is open goes through one of
    /// the three functions that hold the invariants, never through here directly.
    fn document_panel_mut(&mut self) -> Option<&mut DockPanel<Tab, PanelId>> {
        let documents = self.documents?;
        self.tree.panel_mut(&documents)
    }

    /// Put `tab` in the document panel if it is not there, and make it the tab on top.
    ///
    /// Documents are **appended after the views**, so Project, Settings and the Scratchpad
    /// keep the left of the bar and stay where the reader can always see them. The other
    /// order was tried -- documents first, where the content area's own strip used to be
    /// -- and a restored session's dozen tabs pushed all three views off the right-hand
    /// edge, which is a control that vanishes rather than a tab that scrolls. Documents
    /// are reachable from the symbol list and the history besides; the three views are
    /// reachable from nowhere else.
    fn show_document(&mut self, tab: Tab) {
        let Some(panel) = self.document_panel_mut() else {
            return;
        };
        if !panel.tabs.contains(&tab) {
            panel.tabs.push(tab);
        }
        panel.active_tab_id = Some(tab);
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

    /// Fold away the panels a move emptied, **except the document panel**. An area that
    /// loses its last tab keeps one empty panel rather than going to `None`, so its pane
    /// stays on screen as a drop target and tabs can be dragged back into it.
    ///
    /// This is freya's `close_empty_panels` written out rather than called, and the
    /// exemption is why. That sweep retains every non-empty child with no way to spare
    /// one, so the document panel would fold away the moment the last document was closed
    /// -- the one thing it exists not to do. It has to *replace* the call rather than
    /// follow it: a panel re-created after the sweep would come back somewhere else in
    /// the tree, having lost the place the reader put it.
    ///
    /// The two behaviours that are freya's and are kept: a split left with one child
    /// collapses into that child, and a lone panel at the root is never removed.
    fn tidy(&mut self) {
        Self::prune(&mut self.tree, self.documents);
        if self.tree.is_empty() && !matches!(self.tree, DockNode::Panel(_)) {
            let panel_id = self.take_panel_id();
            self.tree = DockNode::Panel(DockPanel::new(panel_id, Vec::new()));
        }
    }

    /// [`DockArea::tidy`]'s walk: drop every empty child that is not, and does not hold,
    /// the document panel, then collapse a split down to its only survivor.
    fn prune(node: &mut DockNode<Tab, PanelId>, documents: Option<PanelId>) {
        let DockNode::Split { children, .. } = node else {
            return;
        };
        children
            .iter_mut()
            .for_each(|child| Self::prune(child, documents));
        children.retain(|child| !child.is_empty() || Self::spares(child, documents));
        if children.len() == 1 {
            *node = children.remove(0);
        }
    }

    /// Whether `node` is, or contains, the panel documents live in.
    fn spares(node: &DockNode<Tab, PanelId>, documents: Option<PanelId>) -> bool {
        let Some(documents) = documents else {
            return false;
        };
        match node {
            DockNode::Panel(panel) => panel.panel_id == documents,
            DockNode::Split { children, .. } => children
                .iter()
                .any(|child| Self::spares(child, Some(documents))),
        }
    }

    /// Whether `tab` may land where a drop is aiming it.
    ///
    /// The asymmetry the two kinds of tab have: **a view may go anywhere, the document
    /// panel included; a document may only ever be in the document panel.** The first
    /// half is what keeps Project, Settings and the Scratchpad tabbed beside the
    /// documents, where they have always been. The second is what keeps exactly one
    /// document visible at a time, which is what lets `Analysis`, `Marked`, `Focused` and
    /// `Pinned` each hold one answer for the window rather than one per document.
    ///
    /// A refused drop answers `false`, which leaves the drag where it started rather than
    /// dropping the tab out of the app.
    fn accepts(&self, tab: Tab, target: &DropTarget<PanelId>) -> bool {
        let Tab::Document(id) = tab else {
            return true;
        };
        // A drag begun before its document was closed carries an id that stands for
        // nothing. Ids are never reused, so this can only ever be a dead one -- never
        // some other document that took its number -- and refusing it is the whole
        // payoff of that rule.
        if self.docs.is_some_and(|docs| docs.peek().get(id).is_none()) {
            return false;
        }
        match target {
            DropTarget::Tab { panel_id, .. } | DropTarget::Center(panel_id) => {
                self.documents == Some(*panel_id)
            }
            // A split always makes a *new* panel, which by construction is not the one
            // documents live in.
            DropTarget::Split { .. } => false,
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
        if !self.accepts(tab, &target) {
            return false;
        }

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
fn tab_label(tab: Tab, docs: State<Docs>, background: Color) -> impl IntoElement {
    let docs = docs.read();
    rect()
        .height(Size::px(list_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .spacing(6.0)
        .background(background)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(tab.icon(&docs))
        .child(label().text(elide(&tab.title(&docs))).max_lines(1))
}

fn tab_header(ctx: TabContext<Tab>, area: State<DockArea>, docs: State<Docs>) -> Element {
    let active = area.read().is_active(ctx.tab_id);

    match ctx.tab_id {
        // A document wears the chip the content area's own strip used to draw: the same
        // icon, the same elided name, the same ×. It is a component of its own because it
        // has a hover state; a view header has none, having nothing to close.
        Tab::Document(id) => DocumentHeader {
            id,
            active,
            key: DiffKey::None,
        }
        .into_element(),
        Tab::View(_) => {
            let background = if ctx.is_drop_target {
                palette().selected_bg
            } else if active {
                palette().pane_bg
            } else {
                Color::TRANSPARENT
            };
            tab_label(ctx.tab_id, docs, background).into_element()
        }
    }
}

/// The copy of the tab that follows the cursor while it is being dragged.
fn tab_drag(tab: Tab, docs: State<Docs>) -> Element {
    rect()
        .interactive(false)
        .border(right_hairline())
        .child(tab_label(tab, docs, palette().selected_bg))
        .into_element()
}

/// The bar a panel's tab headers sit in.
///
/// Two shapes, and the difference is how many tabs a panel can come to hold. A view panel
/// holds at most the seven views and always fits, so it is a plain row. The document panel
/// is opened into by the dozen, and a tab that has fallen off the right-hand edge would be
/// unreachable, so it gets the horizontally scrolling bar the content area's own strip
/// used to be -- which is where [`chip_strip`] came from and why it is still here.
fn tab_bar(ctx: TabBarContext<PanelId>, area: State<DockArea>) -> Element {
    if area.peek().documents == Some(ctx.panel_id) {
        return chip_strip(ctx.tab_children, ctx.tab_count);
    }

    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .horizontal()
        .background(palette().header_bg)
        .border(bottom_hairline())
        .children(ctx.tab_children)
        .into_element()
}

/// One document, drawn: its assembly beside the source it was compiled from.
///
/// **The two panes are inside the document rather than beside it**, which is the trade the
/// whole change is built on. It buys documents that the reader arranges the way they
/// already arrange the views, and it costs the Source pane being dockable on its own --
/// it can no longer be put below the assembly or dragged into the sidebar.
///
/// A `ResizableContainer` and not a nested `DockingArea`: a dock inside a dock tab is a
/// great deal of machinery for a two-way split, and nothing here wants the second one's
/// tabs, drops or drags.
///
/// Only the *active* tab's content is mounted, so this whole subtree -- both panes, both
/// scroll controllers -- is built afresh on every switch of document. That is what
/// `use_kept_position` is for, and it is why its "first run, on a tab it has a row for"
/// arm went from the rare case to the ordinary one.
#[derive(Clone, PartialEq)]
struct DocumentBody {
    id: DocId,
}

impl Component for DocumentBody {
    fn render(&self) -> impl IntoElement {
        let docs = use_consume::<OpenDocs>().0;
        let mut ratio = use_consume::<SplitRatio>().0;
        let splits = use_consume::<Splits>().0;

        // Where the reader last left the handle, written back as they drag it. Reading
        // the context is what subscribes this to the drag; `set_if_modified` is what
        // keeps the mount's own registration -- which writes the initial size back
        // unchanged -- from waking anything.
        use_side_effect(move || {
            let live = splits.read().panels.first().map(|panel| panel.size);
            if let Some(live) = live {
                ratio.set_if_modified(live);
            }
        });

        // `peek` and not `read`: `initial_size` is consulted once, in the panel's own
        // `use_hook` at mount, so subscribing this component to a number it can only act
        // on by being remounted would be a subscription to nothing -- and a loop with the
        // effect above.
        let assembly = ratio.peek().clamp(1.0, 99.0);

        // A tab whose document has gone draws nothing. Not reachable -- the tab and the
        // table entry are closed together -- but a render is no place to panic.
        let Some(document) = docs.read().get(self.id).cloned() else {
            return rect()
                .expanded()
                .background(palette().asm_pane_bg)
                .into_element();
        };

        ResizableContainer::new()
            .direction(Direction::Horizontal)
            .controller(splits)
            .panel(
                // `min_size` given rather than left to default: freya's default is a
                // quarter of the initial size, so it would move with the reader's own
                // drag instead of staying the floor.
                ResizablePanel::new(PanelSize::percent(assembly))
                    .min_size(10.0)
                    .child(AssemblyPane {
                        document: document.clone(),
                    }),
            )
            .panel(
                ResizablePanel::new(PanelSize::percent(100.0 - assembly))
                    .min_size(10.0)
                    .child(SourcePane { document }),
            )
            .into_element()
    }
}

/// What a panel draws: its active tab, or -- with no tabs at all -- an empty ground.
///
/// The empty ground differs by panel, which is why this is handed the whole context
/// rather than just the tab. "Drag a tab here" is right for a view panel, which is empty
/// only because the reader dragged everything out of it and can drag something back. It
/// is wrong for the document panel, which is empty because nothing is open -- so that one
/// draws what the app draws with nothing selected.
fn tab_content(ctx: ContentContext<Tab, PanelId>, area: State<DockArea>) -> Element {
    match ctx.tab_id {
        Some(Tab::View(view)) => view.view(),
        Some(Tab::Document(id)) => DocumentBody { id }.into_element(),
        // `peek` and not `read`: which panel holds documents is fixed when the area is
        // built, so subscribing to it would be a subscription to nothing.
        None if area.peek().documents == Some(ctx.panel_id) => placeholder("Nothing selected"),
        None => placeholder("Drag a tab here"),
    }
}

fn docking_area(area: State<DockArea>, docs: State<Docs>) -> impl IntoElement {
    DockingArea::new(
        area,
        move |ctx: ContentContext<Tab, PanelId>| tab_content(ctx, area),
        move |ctx: TabContext<Tab>| tab_header(ctx, area, docs),
        move |tab: Tab| tab_drag(tab, docs),
        move |ctx: TabBarContext<PanelId>| tab_bar(ctx, area),
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
            {
                // The dock and the table rather than `Active`: this has to write down
                // what is open *now*, and `Active` is a memo that catches up a beat
                // later. Reading the dock here is also what wakes this on a layout drag
                // -- `record` compares against its baselines and writes nothing, so that
                // is a wasted wake rather than a wasted write.
                let (dock, docs) = (open.dock.read(), open.docs.read());
                Session::from_state(
                    &objects,
                    &open_documents(&dock, &docs),
                    &asm_at.read(),
                    &src_at.read(),
                    active_document(&dock, &docs).as_ref(),
                    &history.read(),
                )
            },
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
    active: Memo<Option<Document>>,
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
    active: Memo<Option<Document>>,
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
/// What [`use_analysis_with`] needs of the active document: a **read**, which subscribes
/// the effect to it, and a **peek**, which does not. The distinction is load-bearing --
/// the effect must wake on a change of document and must not wake on its own writes -- so
/// it cannot collapse into one closure.
///
/// A trait so the hook can be driven by the [`Active`] memo in the app and by a plain
/// state in the tests, which are about the worker rather than about the tabs and have no
/// business building a dock to say which symbol is selected.
trait ReadsActive: Copy + 'static {
    fn read_active(self) -> Option<Document>;
    fn peek_active(self) -> Option<Document>;
}

impl ReadsActive for Memo<Option<Document>> {
    fn read_active(self) -> Option<Document> {
        self.read().clone()
    }

    fn peek_active(self) -> Option<Document> {
        self.peek().clone()
    }
}

impl ReadsActive for State<Option<Document>> {
    fn read_active(self) -> Option<Document> {
        self.read().clone()
    }

    fn peek_active(self) -> Option<Document> {
        self.peek().clone()
    }
}

fn use_analysis(active: Memo<Option<Document>>, analysis: State<Analyzed>) {
    use_analysis_with(active, analysis, Studied::new);
}

/// The whole of [`use_analysis`], with the work itself as an argument so a test can hold
/// it still. Superseding is a race by construction — the answer that has to be dropped is
/// the one that arrives while the reader has already clicked on — and nothing can assert
/// it against a worker that answers as fast as it is asked.
fn use_analysis_with(
    active: impl ReadsActive,
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
                let current = active.peek_active();
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
        let current = active.read_active();

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
fn navigate(open: Open, mut history: State<History>, nav: Nav) {
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
        activate(open, history, entry, Visit::Moved);
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
            activate(open, history, Some(tab), Visit::Moved);
        }
        // The one exception, and it is what keeps the cursor and the app in step: the
        // document the app *lands on* is a place it went. `would_push` makes it free in
        // the ordinary case — the saved cursor entry is the saved active document, and
        // the two resolve through the same lookup to the same `Arc`s — and records it
        // exactly when they differ, which is when the cursor entry was dropped or the
        // active document degraded and the app really is somewhere new.
        activate(open, history, restored_active, Visit::Went);
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
        close_binary(objects, loading, open, asm_at, src_at, history, &path);
    }

    let remaining = open.documents();
    for tab in &remaining {
        close_tab(open, history, asm_at, src_at, tab);
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
    // The id side table the dock's document tabs are handles into. Not the list of open
    // documents -- that is the document panel's own `tabs` vec -- only the mapping from
    // the handle a tab can hold to the document it stands for.
    let docs = use_provide_context(|| OpenDocs(State::create(Docs::default()))).0;
    let sidebar_dock = use_state(|| {
        DockArea::column(vec![
            vec![Tab::View(View::Objects)],
            vec![Tab::View(View::Symbols), Tab::View(View::Info)],
            vec![Tab::View(View::History)],
        ])
    });
    // Panel 0 of the content area is where documents live. Project, Settings and the
    // Scratchpad are tabbed in it beside them, which is where they have always been --
    // behind the Assembly pane, which is now a document's left-hand side rather than a
    // view of its own.
    let content_dock = use_state(|| {
        DockArea::row(vec![vec![
            Tab::View(View::Project),
            Tab::View(View::Settings),
            Tab::View(View::Scratchpad),
        ]])
        .with_documents(DOCUMENT_PANEL)
    });
    use_hook(move || {
        let (mut sidebar_dock, mut content_dock) = (sidebar_dock, content_dock);
        sidebar_dock.write().other = Some(content_dock);
        content_dock.write().other = Some(sidebar_dock);
        sidebar_dock.write().docs = Some(docs);
        content_dock.write().docs = Some(docs);
    });
    let content_dock = use_provide_context(move || ContentDock(content_dock)).0;
    let open = Open {
        dock: content_dock,
        docs,
    };
    // The active document, *derived* from the two above rather than kept beside them:
    // it is the document panel's active tab. See `Active` for why it is a memo and why
    // being a beat behind is right for everything that renders and wrong for the three
    // functions that hold the invariants.
    let active = use_provide_context(move || {
        Active(Memo::create(move || {
            active_document(&content_dock.read(), &docs.read())
        }))
    })
    .0;
    // Where each side of each tab was left, which is a view of what is open rather than a
    // second copy of it: an entry appears when a pane is scrolled and goes when the tab
    // it belongs to is closed, so the same functions hold this true as hold the tabs
    // themselves.
    // The assembly/source split, kept out here because a document's panes are unmounted
    // on every tab switch and take their sizes with them. See `SplitRatio`.
    use_provide_context(|| SplitRatio(State::create(DEFAULT_SPLIT)));
    use_provide_context(|| {
        Splits(State::create(ResizableContext {
            direction: Direction::Horizontal,
            ..Default::default()
        }))
    });
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
                .child(docking_area(sidebar_dock, docs)),
        )
        .panel(
            ResizablePanel::new(PanelSize::percent(100.0))
                .min_size(10.0)
                // No strip of its own any more: the open documents *are* tabs in the
                // dock's document panel, so the bar over them is that panel's own tab
                // bar. `DockingArea` renders itself `.expanded()`, so it needs a parent
                // that has been given the height.
                .child(docking_area(content_dock, docs)),
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
            Some(MouseButton::Back) => navigate(open, history, Nav::Back),
            Some(MouseButton::Forward) => navigate(open, history, Nav::Forward),
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

#[cfg(test)]
mod tests;

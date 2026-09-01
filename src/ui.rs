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

mod analyzed;
pub(crate) use analyzed::*;
mod assembly;
pub(crate) use assembly::*;
mod documents;
pub(crate) use documents::*;
mod filter_bar;
pub(crate) use filter_bar::*;
mod focus;
pub(crate) use focus::*;
mod highlight;
pub(crate) use highlight::*;
mod marks;
pub(crate) use marks::*;
mod metrics;
pub(crate) use metrics::*;
mod palette;
pub(crate) use palette::*;
mod parts;
pub(crate) use parts::*;
mod sidebar;
pub(crate) use sidebar::*;
mod source_view;
pub(crate) use source_view::*;
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

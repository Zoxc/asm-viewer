//! The freya UI.
//!
//! The imports below are this module's prelude: they are `pub(crate) use` and every file
//! under this one begins `use super::*;`. Each `mod x;` is followed by a
//! `pub(crate) use x::*;`, so a name means the same thing wherever it is written.
pub(crate) use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    ops::{ControlFlow, Range, RangeInclusive},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
    time::{Duration, Instant},
};

pub(crate) use async_io::Timer;
pub(crate) use freya::code_editor::{
    CodeEditor, CodeEditorData, EditorLanguage, EditorSyntaxTheme, EditorThemePartialExt, Rope,
    SyntaxBlocks, SyntaxHighlighter, TextNode,
};
pub(crate) use freya::icons::lucide;
pub(crate) use freya::prelude::*;
// The markdown the hover box draws its answer with. Its own crate rather than freya's
// prelude, freya not re-exporting it.
pub(crate) use freya_markdown::{MarkdownViewer, MarkdownViewerThemePreference};
// The editor's own text trait, which the prelude does not carry: where its cursor is and
// how to put it somewhere else.
pub(crate) use freya::text_edit::TextEditor;
pub(crate) use rfd::AsyncFileDialog;

pub(crate) use analysis::{
    open_files_streaming, Assembly, Bias, CodeListing, Instruction, LineInfo, Object, Operand,
    PlacedAddress, Progress, SectionAddress, Severity, SpanKind, Symbol, SymbolData,
};

pub(crate) use crate::bookmarks::{Bookmark, Bookmarks};
pub(crate) use crate::cargo::{self, Diagnostic, Level, Profile};
pub(crate) use crate::chars::{self, beyond, Bounds, Caret, CharSelection, Line, Motion};
pub(crate) use crate::compiled;
pub(crate) use crate::docs::{DocId, Docs, Entry};
pub(crate) use crate::document::{Address, Document, Kind, Pane};
pub(crate) use crate::files::{FileRow, FileRows, FileTree, Fold};
pub(crate) use crate::filter::{Filter, Filtered, Matcher};
pub(crate) use crate::fonts::{self, Font, Fonts};
pub(crate) use crate::functions::{self, Function};
pub(crate) use crate::history::{History, Place, Stop};
pub(crate) use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
pub(crate) use crate::languages;
pub(crate) use crate::links;
pub(crate) use crate::lsp::{self, Lookup};
pub(crate) use crate::naming::short_name;
pub(crate) use crate::pixels::Grid;
pub(crate) use crate::positions::{Driven, Positions, Spot, TopRow};
pub(crate) use crate::process::{self, Ended, OutputLine, RunEvent, RunOutput, Stream};
pub(crate) use crate::project::{
    self, Cargo, Details, LeftAt, Noticed, OnScreen, Project, Recent, RestoredEntry, RestoredTab,
    SavedDock, SavedDocument, SavedShown, SavedUi, SavingTab, Session,
};
pub(crate) use crate::references::{self, ReferenceRows};
pub(crate) use crate::reveal;
pub(crate) use crate::scratchpad::{
    is_source_file, own_source, run_in, Build, Failure, Half, PadId, PadListing, PadOrder, Problem,
    RowId, Scratchpad, SOURCE_FILE,
};
pub(crate) use crate::section;
pub(crate) use crate::settings::{FontSetting, Settings, Theme as ThemeChoice};
pub(crate) use crate::shared::{same_arc, ByPtr, Shared};
pub(crate) use crate::shortcuts;
pub(crate) use crate::source::{self, showable, SourceFile};
pub(crate) use crate::store::Store;
pub(crate) use crate::tabs::{Along, Page, Strip, Tab};
pub(crate) use crate::tree::{
    format_tag, Expansion, LoadId, Loads, ObjectTree, TreeRow, ARCHIVE_TAG,
};
pub(crate) use crate::verdict::{counted, Verdict};
pub(crate) use crate::visits::Visits;

mod analyzed;
pub(crate) use analyzed::*;
mod assembly;
pub(crate) use assembly::*;
mod bookmarks_view;
pub(crate) use bookmarks_view::*;
mod building;
pub(crate) use building::*;
mod chords;
pub(crate) use chords::*;
mod code_row;
pub(crate) use code_row::*;
mod coded;
pub(crate) use coded::*;
mod debug_view;
pub(crate) use debug_view::*;
mod dock;
// `Panel` by name as well as through the glob: freya's prelude has a `Panel` of its own,
// and an explicit import is what settles which one the app means.
pub(crate) use dock::Panel;
pub(crate) use dock::*;
mod documents;
pub(crate) use documents::*;
mod entries;
pub(crate) use entries::*;
mod files_view;
pub(crate) use files_view::*;
mod filter_bar;
pub(crate) use filter_bar::*;
mod find_bar;
pub(crate) use find_bar::*;
mod finder;
pub(crate) use finder::*;
mod focus;
pub(crate) use focus::*;
mod follow;
pub(crate) use follow::*;
mod glyph;
pub(crate) use glyph::*;
mod highlight;
pub(crate) use highlight::*;
mod hover_view;
pub(crate) use hover_view::*;
mod hovering;
pub(crate) use hovering::*;
mod hunt;
pub(crate) use hunt::*;
mod keyboard;
pub(crate) use keyboard::*;
mod keys;
pub(crate) use keys::*;
mod language;
pub(crate) use language::*;
mod language_view;
pub(crate) use language_view::*;
mod linking;
pub(crate) use linking::*;
mod list_box;
pub(crate) use list_box::*;
mod loading;
pub(crate) use loading::*;
mod locations;
pub(crate) use locations::*;
mod marks;
pub(crate) use marks::*;
mod menus;
pub(crate) use menus::*;
mod metrics;
pub(crate) use metrics::*;
mod no_project;
pub(crate) use no_project::*;
mod opened;
pub(crate) use opened::*;
mod pad;
pub(crate) use pad::*;
mod pad_view;
pub(crate) use pad_view::*;
mod pages_menu;
pub(crate) use pages_menu::*;
mod palette;
pub(crate) use palette::*;
mod parts;
pub(crate) use parts::*;
mod picks;
pub(crate) use picks::*;
mod place_row;
pub(crate) use place_row::*;
mod place_target;
pub(crate) use place_target::*;
mod project_view;
pub(crate) use project_view::*;
mod reading;
pub(crate) use reading::*;
mod rescued_view;
pub(crate) use rescued_view::*;
mod scrolling;
pub(crate) use scrolling::*;
mod search_view;
pub(crate) use search_view::*;
mod section_view;
pub(crate) use section_view::*;
mod session;
pub(crate) use session::*;
mod settings_view;
pub(crate) use settings_view::*;
mod shortcuts_view;
pub(crate) use shortcuts_view::*;
mod sidebar;
pub(crate) use sidebar::*;
mod source_bar;
pub(crate) use source_bar::*;
mod source_row;
pub(crate) use source_row::*;
mod source_view;
pub(crate) use source_view::*;
mod split;
pub(crate) use split::*;
mod state;
pub(crate) use state::*;
mod strip;
pub(crate) use strip::*;
mod studied;
pub(crate) use studied::*;
mod symbol_bar;
pub(crate) use symbol_bar::*;
mod width;
pub(crate) use width::*;
mod worker;
pub(crate) use worker::*;

/// One of the two history buttons at the left of the toolbar: the step it makes along
/// the trail of the tab on screen, drawn as the chevron pointing that way, with the entry
/// it would land on in its tooltip.
///
/// **A memo reads `Active` and the table rather than peeking them**, and that is the whole
/// of how the pair stays current: a switch of tab, a push onto any trail, a close that
/// drops entries, and every move of a cursor -- the one this button itself just made
/// included -- asks both again, and a button whose destination changed is drawn again. `Active` and not the strip: reading the strip would repaint the pair
/// whenever a tab moved along the bar, which is why `Active` is a memo at all.
///
/// A button with nothing in its direction is **dimmed rather than hidden**. Hiding it would
/// move the button beside it under the pointer, and a reader who has not been anywhere yet
/// would never learn the pair is there at all. Being disabled is the whole of the drawing:
/// no hover wash, no press handler, and the chevron in [`dimmed`], which is `icon_fg` faded
/// into the toolbar rather than a colour of its own that both palettes would have to keep
/// in step. The tooltip stays, naming the direction where it cannot name a destination.
#[derive(Clone, PartialEq)]
struct NavButton {
    /// Which way it steps.
    back: bool,
}

impl Component for NavButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let open = use_open();
        let active = use_consume::<Active>().0;

        let (nav, word, icon) = if self.back {
            (Nav::Back, "Back", ("chevron-left", lucide::chevron_left()))
        } else {
            (
                Nav::Forward,
                "Forward",
                ("chevron-right", lucide::chevron_right()),
            )
        };

        // The reads, in a memo whose value is what the button draws: `Docs` is written by
        // every push onto any trail, and the button is drawn again only when where it would
        // land changes. `nav` is fixed for the button's life, the pair always being built
        // in the same order.
        let destination = use_memo(move || {
            let docs = open.docs.read();
            active
                .read()
                .as_ref()
                .and_then(|(id, _)| docs.trail(*id))
                .and_then(|trail| nav.destination(trail))
                .map(stop_text)
        });
        let destination = destination.read().clone();
        let live = destination.is_some();
        let tooltip = match &destination {
            Some(name) => format!("{word} to {name}"),
            None => word.to_owned(),
        };

        // A button with nowhere to go keeps its tooltip and loses everything else: no
        // wash, no press, and the chevron dimmed. `bar_button` drops the first two; the
        // colour is this button's own, being the only disabled drawing in the app.
        let colour = match live {
            true => palette().icon_fg,
            false => dimmed(palette().icon_fg, palette().pane_bg),
        };

        TooltipContainer::new(Tooltip::new(tooltip)).child(
            bar_button(hovering, live, Glow::No)
                .maybe(live, |button| button.on_press(move |_| navigate(open, nav)))
                .child(glyph_in(icon, colour)),
        )
    }
}

fn toolbar() -> impl IntoElement {
    rect()
        .horizontal()
        .width(Size::fill())
        // `Content::Flex` so the gap below is measured last, out of what the two controls
        // left over, rather than claiming the bar and pushing them off its right edge.
        .content(Content::Flex)
        .cross_align(Alignment::Center)
        .border(bottom_hairline())
        .child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .margin(4.0)
                .spacing(4.0)
                // At the very left of the window: the ways in and out of a project, and
                // the way back to a page that has been closed.
                .child(PagesButton)
                // What those items are about, said where the reader can see it without
                // opening a menu.
                .child(ProjectChip),
        )
        // The bar's controls sit at its two ends, so the pair the reader reaches for
        // without looking stays under the same corner however many controls Open grows
        // neighbours.
        .child(rect().width(Size::flex(1.0)))
        .child(
            rect()
                .horizontal()
                .margin(4.0)
                .spacing(2.0)
                .child(ServerButton)
                .child(NavButton { back: true })
                .child(NavButton { back: false }),
        )
}

/// Every key the window answers to whatever holds the keyboard: the modifiers each
/// pointer gesture is read against, and every chord that is the window's own rather than
/// a list's, a box's or a pane's ([`Chord`]).
///
/// **One handler and not two.** An element keeps one handler per event name, so a second
/// `on_global_key_down` on the root would replace this one and take the modifier tracking
/// with it -- silently, with Ctrl-click and Shift-click going quiet. And a **global**
/// handler, since a plain key event is emitted only for the focused node that listens for
/// it: this one has to answer from wherever the keyboard is, including nowhere.
///
/// **It takes the bundles and not a state per binding.** Every chord below is a second
/// door onto something the app already has, so the states it wants are the states those
/// doors want, and a parameter per key would grow this list by one on every binding
/// added. What is passed by hand is what belongs to no bundle: where the keyboard can be
/// put, the finder, the language server with the worker it is spoken to through, and the
/// flags saying whether each place's following pane is up.
#[allow(clippy::too_many_arguments)]
pub(crate) fn root_key_down(
    keys: ModifierKeys,
    states: ProjectStates,
    keyboard: Keyboard,
    finder: State<Finder>,
    language: State<Language>,
    jobs: &LspJobs,
    follows: State<HashMap<Placing, bool>>,
    key: &Key,
    modifiers: Modifiers,
) {
    keys.down(key, modifiers);
    // **The one chord this key is, looked up once.** A key event is at most one of them
    // (`Chord::of`), so the window's keys are one `match` and not a chain of tests, and a
    // binding added here sits beside the others rather than after them. The `_` is every
    // chord a list, a box, a pane or the scratchpad answers where it is.
    let Some(chord) = Chord::of(key, modifiers) else {
        return;
    };
    let ProjectStates {
        proj,
        open,
        places,
        bookmarks,
        objects,
        arranged,
        ..
    } = states;
    let strip = open.strip;

    match chord {
        // The panels and the overlay that are reached from anywhere. Each panel chord
        // raises its panel and puts the keyboard in it (`reach_panel`); the three lists a
        // reader lives in, and Search. History, Bookmarks and Locations have none, being
        // a press away or opened by the question that fills them.
        Chord::Search => reach_panel(arranged.dock, keyboard, Panel::Search),
        Chord::Files => reach_panel(arranged.dock, keyboard, Panel::Files),
        Chord::Objects => reach_panel(arranged.dock, keyboard, Panel::Objects),
        Chord::Symbols => reach_panel(arranged.dock, keyboard, Panel::Symbols),
        Chord::Finder => {
            let root = proj.peek().workspace();
            open_finder(finder, root);
        }

        // The bar. Two spellings of the close, one door: the tab on screen goes whether
        // it is a page or a document (`close_showing`). The nine digits are one arm: they
        // differ in the number alone, and the number is the argument the answer takes.
        Chord::CloseTab | Chord::CloseTabF4 => close_showing(open, places),
        Chord::NextTab => step_tab(open, Along::Next),
        Chord::PreviousTab => step_tab(open, Along::Previous),
        Chord::NthTab(nth) => show_nth(open, nth as usize),

        // The trail of the tab on screen: the same call the mouse's side buttons and the
        // toolbar's two chevrons make, and nothing at all where the trail has no such
        // step.
        Chord::Back => navigate(open, Nav::Back),
        Chord::Forward => navigate(open, Nav::Forward),

        // The window's own doors. `show_page` for the two pages, which opens one beside
        // the tab on screen and raises one already open -- what the pages menu's row does.
        Chord::OpenProject => ask_for_a_project(states),
        Chord::Settings => show_page(open, Page::Settings),
        Chord::Shortcuts => show_page(open, Page::Shortcuts),
        Chord::Server => toggle_server(language, proj, jobs),

        // The reader's own list, added to or taken from: the tab menu's item asked of the
        // tab on screen rather than of the tab under the pointer. A page is no place, so
        // it has nothing to bookmark.
        Chord::Bookmark => {
            // Bound in a statement of its own: the toggle writes a state this read.
            let showing = open.active();
            if let Some(document) = showing {
                toggle_bookmark(bookmarks, objects, &document);
            }
        }

        // The pane that follows the one on screen, put away or brought back: the toggle on
        // the leading bar, pressed by key. A document tab writes the flag under its own
        // `Placing`; which pages have a second pane at all is the page table's
        // (`page_following`, `ui/pages_menu.rs`).
        Chord::OtherPane => {
            let showing = strip.peek().active();
            let of = match showing {
                Some(Tab::Document(id)) => Some(Placing::Tab(id)),
                Some(Tab::Page(page)) => page_following(page),
                None => None,
            };
            if let Some(of) = of {
                toggle_pane(of, open, follows);
            }
        }

        _ => {}
    }
}

/// Every root context handed back, so that `app()` and a headless test can each keep what
/// they need of it.
///
/// Flat, and holding handles two of its bundles hold too: what a caller wants of the root
/// is one state by the name it is called by, and `states.open` is the same `Open`
/// `doors.open` is.
#[derive(Clone, Copy)]
pub(crate) struct Roots {
    pub(crate) prefs: State<EditedSettings>,
    pub(crate) active: Memo<Option<Entry>>,
    pub(crate) keyboard: Keyboard,
    pub(crate) follows: State<HashMap<Placing, bool>>,
    pub(crate) states: ProjectStates,
    pub(crate) doors: Doors,
    /// The keyboard, the three modifiers a door reads among its five states.
    pub(crate) keys: ModifierKeys,
    pub(crate) finder: State<Finder>,
    pub(crate) analysis: State<Analyzed>,
    pub(crate) located: State<Located>,
    pub(crate) coded: State<Coded>,
    pub(crate) sectioned: Sectioned,
    pub(crate) sourced: State<Sourced>,
    pub(crate) showing: State<Option<Arc<Path>>>,
    pub(crate) finds: State<Finds>,
    pub(crate) pad: State<Pads>,
    pub(crate) pad_text: State<PadBuffers>,
    /// The one field that is no root context, and the only one a component cannot reach
    /// for itself ([`roots`]).
    pub(crate) opened: State<Opened>,
    pub(crate) language: State<Language>,
    pub(crate) follow: State<Follow>,
    pub(crate) linked: State<Linked>,
    pub(crate) hover: State<Hover>,
}

/// Put one value in the root scope's storage and hand it back: the shape every line of
/// [`roots`] wants, which the free function does not have.
fn provide<T: Clone + 'static>(value: T) -> T {
    provide_root_context(value.clone());
    value
}

/// Make a state, provide it under the context `wrap` names, and hand the state back:
/// what a line of [`roots`] that provides one state is, written once.
///
/// `wrap` is the context's own tuple struct, used as the function it is. Thirty lines
/// spelling the make, the wrap and the unwrap out are thirty places to wrap the wrong
/// state in the right newtype and read like the rest. The bundles and the memos keep
/// [`provide`]: they are not one state.
fn context<T: 'static, C: Clone + 'static>(wrap: fn(State<T>) -> C, value: T) -> State<T> {
    let state = State::create(value);
    provide_root_context(wrap(state));
    state
}

/// Every root context, made and provided in one call.
///
/// **The one list, for the app and for a headless test both.** `app()` calls this in a
/// `use_hook`, and a test's setup closure calls it through the runner (`test_roots`,
/// `src/ui/tests.rs`), so a context added here reaches the tests without a second list
/// being kept in step by hand.
///
/// A plain function and not a hook, which is what lets it serve both: the free
/// `provide_root_context` writes into the root scope's storage and takes no hook slot, so
/// all it wants of a caller is a current scope. A render has one, and so does the runner's
/// own `provide_root_context`, which is this same write wrapped in the root scope.
///
/// The two values a run decides for itself are handed in: where its files go, and what the
/// settings file said. Everything else starts at its default, [`Rescued`] included -- what
/// a load moved aside is written as the store says it, not a value here.
pub(crate) fn roots(store: Option<Store>, settings: &Settings) -> Roots {
    // The one store this run keeps its files in, handed down from here: no other module
    // looks the place up for itself.
    let store = context(Storage, store);
    let prefs = context(Prefs, EditedSettings::of(settings));
    let objects = context(Objects, Vec::new());
    let loading = context(Loading, Loads::default());
    // What is open, the strip and the id table together. Empty: what a restored session
    // puts in the bar is what the reader left, and a session that saved nothing opens on
    // the placeholder, the pages being one menu away.
    let open = provide(Open {
        strip: State::create(Strip::default()),
        docs: State::create(Docs::default()),
    });
    let (strip, docs) = (open.strip, open.docs);
    let active = provide(Active(Memo::create(move || {
        active_tab(&strip.read(), &docs.read())
    })))
    .0;
    // Every object's symbols as one list, the same way: a memo over the objects, so the
    // walk of a hundred thousand symbols is made once per load and not once per render.
    provide(Symbols(Memo::create(move || {
        objects
            .read()
            .iter()
            .flat_map(|object| {
                object.symbols_sorted.iter().cloned().map(|data| Symbol {
                    object: object.clone(),
                    data,
                })
            })
            .collect::<Vec<Symbol>>()
            .into()
    })));

    // Where the tab bar has measured its chips to. At the root and not in the bar: the
    // bar is mounted at most once, and a test reads the places off it from here.
    context(Chipped, Chips::default());

    // The three the window's arrangement is kept in: the sidebar's dock and the two
    // splits the reader can drag, each a number and the context it is read back out of
    // ([`Split`]). 50.0: what the leading side of a document starts at, before anything
    // is dragged. 380: what the widest group of the default arrangement needs to name
    // every panel in it, the four across the top being the widest. A group's bar neither
    // elides nor scrolls, so a narrower sidebar would open with the last name clipped.
    // The sidebar is the one of the three in pixels, its panel being a literal width.
    let dock = context(SidebarDock, DockArea::default());
    let document = Split::create(50.0, Unit::Percent, 1.0, 99.0);
    let sidebar = Split::create(380.0, Unit::Pixels, 120.0, 900.0);
    provide(DocumentSplit(document));
    provide(SidebarSplit(sidebar));
    let arranged = Arrangement {
        dock,
        sidebar: sidebar.size,
        split: document.size,
    };

    // Everything kept per place, which every closer forgets together.
    let places = provide(Places::create());
    context(Expanded, HashSet::new());
    // The row each list has picked out. At the root and not in the panels: a panel that is
    // not its dock tab's is unmounted, and a pick outlives the reader looking elsewhere.
    let picks = context(Picks, HashMap::new());
    let keyboard = provide(Keyboard::create());
    let follows = context(Follows, HashMap::new());
    // The Shortcuts page's box. At the root for the reason the type gives: the page is
    // unmounted whenever another tab is on screen.
    context(Shortcuts, Filter::default());
    let bookmarks = context(Bookmarked, Bookmarks::default());
    let marked = context(Marked, Marks::default());
    // Whether a sweep is under way, out of that state and not read off it: see [`Sweeping`].
    provide(Sweeping(Memo::create(move || sweeping_in(&marked.read()))));
    // What a door is given: the three states it shares with the rest of the app, and the
    // two halves of a landing, which it owns.
    let doors = provide(Doors {
        open,
        places,
        visits: State::create(Visits::default()),
        marked,
        land: State::create(None),
        plant: State::create(None),
        arrived: State::create(None),
    });
    // The keyboard: the five states the root's key handlers keep, three of them the
    // contexts every door reads.
    let keys = provide_modifiers();
    let proj = context(Proj, OpenProject::default());
    // Its file and its directory, out of it and not read off it: see [`ProjFile`].
    let file = provide(ProjFile(Memo::create(move || proj.read().file.clone()))).0;
    // The recent list, read again as the project changes: see [`Recents`].
    provide(Recents(Memo::create(move || {
        // Read to follow it, and for nothing in it.
        file.read();
        store
            .peek()
            .as_ref()
            .map(project::recent_projects)
            .unwrap_or_default()
            .into()
    })));
    provide(Workspace(Memo::create(move || proj.read().workspace())));
    // Whether a delete is being asked about. At the root, since the control that asks is
    // in the bar and the window that answers is over everything.
    context(Deleting, None);
    // And which project would not open, for the window that says so.
    let unopened = context(Unopened, None);
    // Where each file that would not parse was moved to. Empty here, and filled by a task
    // over the store as each file is moved, the startup's loads included.
    let rescued = context(Rescued, Vec::new());
    let stored = store.peek().clone();
    if let Some(stored) = stored {
        spawn(name_moved(stored, rescued));
    }
    let searched = context(Searching, Searched::default());
    let located = context(Locations, Located::default());
    // At the root, not in the overlay: the list of a project's files is kept between
    // opens, and the walk that fills it outlives the overlay being closed.
    let finder = context(Finding, Finder::default());
    // At the root, not in the Project tab: a tab that is not on screen is unmounted, and a
    // build that survives the reader looking away cannot live there.
    let build = context(Building, Builds::default());
    // The one place this list is written besides the struct itself: every reader of it
    // takes the bundle whole (`use_project_states`).
    let states = provide(ProjectStates {
        proj,
        store,
        unopened,
        objects,
        loading,
        open,
        places,
        visits: doors.visits,
        bookmarks,
        picks,
        searched,
        located,
        build,
        arranged,
        stay: State::create(Stay::default()),
    });

    let analysis = context(Analysis, Analyzed::default());
    let coded = context(Coding, Coded::default());
    // The reading of one object's code, whole: what is decoded, what is wanted next,
    // whose listing it is when no tab's, and the rows the view built of it.
    let sectioned = provide(Sectioned {
        reading: State::create(Reading::default()),
        window: State::create(None),
        beside: State::create(None),
        rows: State::create(None),
    });
    // The file the Source pane is showing, read off disk and parsed on a thread of its
    // own -- and every code pane's find bar, whose bars are kept per place. The file
    // itself is a state of its own, the pane writing it and three effects reading it.
    let sourced = context(Sourcing, Sourced::default());
    let showing = context(ShowingFile, None);
    let finds = provide(Looking(places.finds)).0;

    // At the root rather than in the tab: a tab off screen is unmounted, and neither a
    // buffer being typed into nor a program that was started can live there. The buffers
    // start empty and a pad gets its own when its source arrives.
    // 50.0: what the editor's side starts at, before anything is dragged.
    provide(PadSplit(Split::create(50.0, Unit::Percent, 1.0, 99.0)));
    let pad = context(Pad, Pads::default());
    let pad_text = context(PadText, PadBuffers::default());

    // Which files the server is told the reader has open, which is what makes it answer
    // about them at all -- and what a build has to say it rewrote, the server holding the
    // text it was given until it is told otherwise.
    //
    // The one state here that is no context: the three hooks that read it -- `use_opened`,
    // `use_linking` and `use_building` -- are the root's own and are handed it, so a
    // context would be one nothing ever consumes. A headless harness wires those same
    // hooks, and provides a context of its own over this state (`test_roots`).
    let opened = State::create(Opened::default());
    // At the root for the reason the rest are, and one more: a language server is a
    // process, and a process that outlives the view it was started from is one nothing can
    // stop. Beside it, where a followed name's answer lands; which of a source file's
    // names are links, which is the server's to say and not the pane's to guess; and what
    // the pointer is on, at the root because the box that draws it is, the rows it is
    // about being recycled under it.
    let language = context(Talking, Language::default());
    let follow = context(Following, Follow::default());
    let linked = context(Linking, Linked::default());
    let hover = context(Hovering, Hover::default());

    Roots {
        prefs,
        active,
        keyboard,
        follows,
        states,
        doors,
        keys,
        finder,
        analysis,
        located,
        coded,
        sectioned,
        sourced,
        showing,
        finds,
        pad,
        pad_text,
        opened,
        language,
        follow,
        linked,
        hover,
    }
}

/// The whole window. `opening` is the project named on the command line, where there was
/// one: it is opened in place of the project last open, and `main` has already answered for
/// a path that is not a project file at all.
pub struct Viewer {
    pub opening: Option<PathBuf>,
}

impl App for Viewer {
    fn render(&self) -> impl IntoElement {
        app(self.opening.as_deref())
    }
}

fn app(opening: Option<&Path>) -> impl IntoElement {
    // The store this run keeps its files in, the panic hook over it, and what the settings
    // file said, in that order and in one hook.
    //
    // The hook is here and not in `main` because freya installs a panic hook of its own
    // inside `launch`, so this is where ours can be the outer one (`crate::panics`). The
    // settings come after it and before everything else: the theme and the fonts are
    // resolved from them and both have to be right on the first frame.
    let (store, settings) = use_hook(|| {
        let store = Store::open();
        crate::panics::install(store.clone());
        let settings = store.as_ref().map(Settings::load).unwrap_or_default();
        (store, settings)
    });
    // Every root context, out of the one list a headless test is given too (`roots`).
    let roots = use_hook(|| roots(store, &settings));
    let Roots {
        prefs,
        active,
        keyboard,
        follows,
        states,
        doors,
        keys,
        finder,
        analysis,
        located,
        coded,
        sectioned,
        sourced,
        showing,
        finds,
        pad,
        pad_text,
        opened,
        language,
        follow,
        linked,
        hover,
    } = roots;
    let ProjectStates {
        proj,
        store,
        objects,
        open,
        places,
        searched,
        build,
        ..
    } = states;
    let marked = doors.marked;

    // The fonts the file names, written once and here: `FONTS` starts at the defaults, and
    // the effect in `use_settings_with` is a frame late. A `use_hook` runs in the root's
    // first render, before any child, so the first frame is already in them; every later
    // change is the effect's.
    use_hook(|| set_fonts(fonts::resolve(&settings)));
    // Owed at once, so the close hook has it, and written once the changes settle: the
    // font family box changes the settings once per keystroke.
    let settling = use_hook(Settle::default);
    use_settings_with(prefs, move |settings: &Settings| {
        if let Some(store) = store.peek().as_ref() {
            settings.owe(store);
            settling.after(SETTLE, crate::settings::flush);
        }
    });
    // freya's own components read their colours from its `Theme` rather than from the
    // palette, and the tooltip's font size can only be set there, so a font change has to
    // be carried in rather than picked up by a re-render. Two calls and not one:
    // `use_init_theme` builds its value in a `use_hook`, so it answers for the first
    // render only and the effect carries every later switch.
    let deps = (appearance(), fonts().ui.size());
    let mut interface = use_init_theme(|| interface_theme(deps.0, deps.1));
    use_side_effect_with_deps(&deps, move |(appearance, size): &(Appearance, f32)| {
        interface.set(interface_theme(*appearance, *size));
    });

    // The ask an opened row or a pressed chip leaves, spent on a pane of the tab on screen
    // -- the one a chip's tab last had the keyboard in, or the leading one -- and on the
    // caret that pane wants, once `use_land` has given that tab its runs.
    use_keyboard_asked(keyboard, doors);
    use_let_go_on_blur(keys);
    use_save_on_change(states);
    use_land(doors, active, sectioned, keyboard);
    use_periodic_save();
    // After the save effect on purpose: its empty baseline must be in place before the
    // restore writes anything, so the restored session is seen as an ordinary change.
    use_restore_on_startup(states, opening);

    use_reading_of(active, objects, sectioned);
    // The question and not the active document: a source-driven tab's assembly side
    // changes when a line in it is clicked, which changes no document.
    let asked = Asked {
        active,
        driven: places.driven,
    };
    let asks = use_analysis_with(
        asked,
        objects,
        sectioned,
        doors.visits,
        analysis,
        located,
        coded,
        showing,
        answer,
    );
    // The other three questions that worker answers, each asked beside the state it is
    // about and handed the way to ask: the window the section view wants next, the
    // Locations panel's query, and which lines of the file on screen have code.
    use_code_asks(sectioned, asks.clone());
    use_locate_asks(located, objects, asks.clone());
    use_mark_asks(coded, showing, objects, asks);
    // After the analysis: the file the Source pane draws is what the analysis says it is.
    use_clear_marks(active, asked, analysis, marked);

    // The reader: the file the Source pane is showing, read off disk and parsed on a
    // thread of its own. Not the analysis worker's queue, which a click can put seconds
    // of DWARF into (`agents/Worker.md`).
    use_source_reading(sourced, showing);
    // And read again once a binary lands, which may have been built from files that have
    // changed since they were read.
    use_rereading(sourced, objects);
    // The find bars' worker. Its own for the reason the source reader has one: a pattern
    // supersedes on every keystroke and must not queue behind the seconds of DWARF a
    // click costs (`agents/Worker.md`).
    use_find(finds);
    // The search's own worker, beside the analysis one and for its reasons: the walk reads
    // every file under the project directory, which is not the UI thread's to do.
    use_search_with(searched, |query, emit| crate::search::search(query, emit));
    // Here and not in the overlay: the walk fills a list that is kept between opens, and
    // has to go on after the overlay it was started from is closed.
    use_finder_with(finder, |root, emit| crate::walk::walk_files(root, emit));
    // The run's own store, handed to the worker at its spawn.
    let pad_store = store.peek().clone();
    use_scratchpad_with(pad, pad_text, sourced, pad_store.clone(), move |job| {
        pad_work(pad_store.as_ref(), job)
    });
    // After the scratchpad and before the server: a build says which files it rewrote,
    // and the server holds the text it was given until it is told otherwise.
    use_building(build, states, opened, sourced);

    let jobs = use_language(language, follow, located, linked, hover, proj, states.stay);
    // What a name followed in the source opens, which the answer above fills in.
    use_follow(follow, doors);
    use_opened(language, opened, open, showing, jobs.clone());
    use_linking(language, linked, showing, opened, jobs.clone());
    use_hovering(language, hover, jobs.clone());

    rect()
        .expanded()
        .content(Content::Flex)
        .font(&fonts().ui)
        // Set once and inherited: freya resolves an element's unset `color` from its
        // parent's, so the whole chrome follows this one call.
        .color(palette().text_fg)
        .background(palette().pane_bg)
        // Global rather than `on_pointer_down`: it is emitted with no hit test, so the
        // mouse's back/forward buttons work wherever the cursor is and no child can
        // swallow them by stopping propagation.
        .on_global_pointer_down(move |e: Event<PointerEventData>| {
            // The box goes wherever the press landed: the reader is doing something
            // else now, and the box is over what they pressed. Inside this handler and
            // not beside it: an element keeps one handler per event, and a second
            // `on_global_pointer_down` would silently replace this.
            hover_gone(hover);
            // And the keyboard goes wherever the press puts it: an ask still waiting for a
            // pane is not the reader's any more. A press that asks makes its ask after
            // this, a press coming after its down.
            unask_keyboard(keyboard);
            match e.button() {
                Some(MouseButton::Back) => navigate(open, Nav::Back),
                Some(MouseButton::Forward) => navigate(open, Nav::Forward),
                _ => {}
            }
        })
        // A sweep ends wherever the button comes up, very often not over the pane it
        // started in, so the end of the gesture is watched for here.
        // The **capture** phase and not the plain global press: that one is cancellable,
        // and freya's own scrollbar thumb cancels it (`prevent_default` in its press), so
        // a sweep let go of over the thumb never ended and the run followed the bare
        // pointer from then on (`notes/upstream/freya.md`).
        .on_capture_global_pointer_press(move |_| mark_release(marked))
        // A freya pointer event carries no modifiers, so Shift and Ctrl have to be known
        // before the click that asks about them: `ModifierKeys`.
        .on_global_key_down(move |e: Event<KeyboardEventData>| {
            hover_struck(hover, &e.key);
            root_key_down(
                keys,
                states,
                keyboard,
                finder,
                language,
                &jobs,
                follows,
                &e.key,
                e.modifiers,
            )
        })
        .on_global_key_up(move |e: Event<KeyboardEventData>| keys.up(&e.key, e.modifiers))
        // Provides the root state `ContextMenu::open_from_event` looks up: opening a menu
        // without one in an ancestor scope panics. It lays out as nothing until a menu
        // is open.
        .child(ContextMenuViewer::new())
        // Over everything, and drawn as nothing at all until a file has been moved aside.
        .child(RescuedPopup)
        // The same, until a delete is asked about.
        .child(DeleteProjectPopup)
        // And until a project the reader asked for would not open.
        .child(UnopenedPopup)
        // The same, until Ctrl+P. Over the window and not in a pane, so it is reached
        // from wherever the reader is.
        .child(FinderOverlay)
        // Over the panes and drawn as nothing until the server has said something about
        // the name under the pointer.
        .child(HoverBox)
        .child(toolbar())
        // Under the bar rather than in the view that has the other Start button: the
        // control above is pressed from wherever the reader is, and a question drawn
        // where they are not looking is a press that did nothing. Lays out as nothing
        // while there is nothing to ask.
        .child(TrustPrompt)
        // `WindowBody` renders a `ResizableContainer`, which renders itself `.expanded()`,
        // so it needs a parent that has already been given the leftover height under the
        // toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(WindowBody),
        )
}

#[cfg(test)]
mod tests;

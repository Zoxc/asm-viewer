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
    time::Duration,
};

pub(crate) use async_io::Timer;
pub(crate) use freya::code_editor::{
    CodeEditor, CodeEditorData, EditorLanguage, EditorSyntaxTheme, EditorThemePartialExt, Rope,
    SyntaxBlocks, SyntaxHighlighter, TextNode,
};
pub(crate) use freya::icons::lucide;
pub(crate) use freya::prelude::*;
// The editor's own text trait, which the prelude does not carry: where its cursor is and
// how to put it somewhere else.
pub(crate) use freya::text_edit::TextEditor;
pub(crate) use rfd::AsyncFileDialog;

pub(crate) use analysis::{
    open_files_streaming, Assembly, CodeListing, Instruction, LineInfo, Object, Progress, SpanKind,
    Symbol, SymbolData,
};

pub(crate) use crate::bookmarks::{Bookmark, Bookmarks};
pub(crate) use crate::cargo::{self, Diagnostic, Level, Profile};
pub(crate) use crate::chars::{beyond, Bounds, Caret, CharSelection, Line, Motion};
pub(crate) use crate::compiled;
pub(crate) use crate::docs::{DocId, Docs, Entry};
pub(crate) use crate::files::{shows_as_source, FileRow, FileRows, FileTree, Fold};
pub(crate) use crate::filter::{Filter, Matcher, Rank};
pub(crate) use crate::fonts::{self, Font, Fonts};
pub(crate) use crate::functions::{self, Function};
pub(crate) use crate::history::{History, Stop};
pub(crate) use crate::lanes::{self, Lanes, Lit, PlacedEdge, RowLanes};
pub(crate) use crate::lsp;
pub(crate) use crate::naming::short_name;
pub(crate) use crate::pixels::Grid;
pub(crate) use crate::project::{
    self, Cargo, Details, Document, Project, ProjectId, Recent, SavedDocument, Selection, Session,
};
pub(crate) use crate::rescue;
pub(crate) use crate::reveal;
pub(crate) use crate::rows::RowSelection;
pub(crate) use crate::scratchpad::{
    run_in, Build, Dependency, Ended, Failure, Half, PadId, PadListing, PadOrder, Problem,
    RunEvent, RunOutput, Running, Scratchpad, Stream,
};
pub(crate) use crate::section;
pub(crate) use crate::settings::{Appearance, FontSetting, Settings, Theme as ThemeChoice};
pub(crate) use crate::source::{self, SourceFile};
pub(crate) use crate::tabs::{self, Driven, Positions, Spot};
pub(crate) use crate::tree::{
    format_tag, Expansion, LoadId, Loads, ObjectTree, TreeRow, ARCHIVE_TAG,
};
pub(crate) use crate::uses::{self, UseRow, UseRows};
pub(crate) use crate::visits::Visits;

mod analyzed;
pub(crate) use analyzed::*;
mod assembly;
pub(crate) use assembly::*;
mod bookmarks_view;
pub(crate) use bookmarks_view::*;
mod building;
pub(crate) use building::*;
mod code_row;
pub(crate) use code_row::*;
mod dock;
pub(crate) use dock::*;
mod documents;
pub(crate) use documents::*;
mod files_view;
pub(crate) use files_view::*;
mod filter_bar;
pub(crate) use filter_bar::*;
mod focus;
pub(crate) use focus::*;
mod follow;
pub(crate) use follow::*;
mod highlight;
pub(crate) use highlight::*;
mod language;
pub(crate) use language::*;
mod locations;
pub(crate) use locations::*;
mod marks;
pub(crate) use marks::*;
mod metrics;
pub(crate) use metrics::*;
mod pad;
pub(crate) use pad::*;
mod pad_view;
pub(crate) use pad_view::*;
mod palette;
pub(crate) use palette::*;
mod parts;
pub(crate) use parts::*;
mod project_view;
pub(crate) use project_view::*;
mod reading;
pub(crate) use reading::*;
mod rescued_view;
pub(crate) use rescued_view::*;
mod search_view;
pub(crate) use search_view::*;
mod section_view;
pub(crate) use section_view::*;
mod settings_view;
pub(crate) use settings_view::*;
mod sidebar;
pub(crate) use sidebar::*;
mod source_view;
pub(crate) use source_view::*;
mod state;
pub(crate) use state::*;
mod symbol_bar;
pub(crate) use symbol_bar::*;
mod width;
pub(crate) use width::*;

/// One of the two history buttons at the left of the toolbar: the step it makes along
/// the trail of the tab on screen, drawn as the chevron pointing that way, with the entry
/// it would land on in its tooltip.
///
/// **It reads `Active` and the table rather than peeking them**, and that is the whole of
/// how the pair stays current: a switch of tab, a push onto any trail, a close that drops
/// entries, and every move of a cursor -- the one this button itself just made included
/// -- repaints both. `Active` and not the dock: reading the dock would repaint the pair on
/// every drag of a split, which is why `Active` is a memo at all.
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
        let mut hovering = use_state(|| false);
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

        // The reads, and with them the subscriptions. Bound to a `let` of their own and
        // dropped here: the press below writes the very state this looked at.
        let destination = {
            let docs = open.docs.read();
            active
                .read()
                .as_ref()
                .and_then(|(id, _)| docs.trail(*id))
                .and_then(|trail| nav.destination(trail))
                .map(stop_text)
        };
        let live = destination.is_some();
        let tooltip = match &destination {
            Some(name) => format!("{word} to {name}"),
            None => word.to_owned(),
        };

        let side = toggle_size();
        let glyph = icon_size();

        TooltipContainer::new(Tooltip::new(tooltip)).child(
            rect()
                .width(Size::px(side))
                .height(Size::px(side))
                .center()
                .corner_radius(4.0)
                .background(if live && hovering() {
                    palette().toggle_hover_bg
                } else {
                    Color::TRANSPARENT
                })
                .maybe(live, |button| {
                    button
                        .on_pointer_over(move |_| hovering.set_if_modified(true))
                        .on_pointer_out(move |_| hovering.set_if_modified(false))
                        .on_press(move |_| navigate(open, nav))
                })
                .child(
                    SvgViewer::new(icon)
                        .width(Size::px(glyph))
                        .height(Size::px(glyph))
                        .color(if live {
                            palette().icon_fg
                        } else {
                            dimmed(palette().icon_fg, palette().pane_bg)
                        })
                        .show_loader(false),
                ),
        )
    }
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

            open_binaries(objects, loading, paths).await;
        });
    };

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
                .margin(4.0)
                .child(Button::new().on_press(on_open).child("Open")),
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
/// pointer gesture is read against, and the chord that reaches the Search panel.
///
/// **One handler and not two.** An element keeps one handler per event name, so a second
/// `on_global_key_down` on the root would replace this one and take the modifier tracking
/// with it -- silently, with Ctrl-click and Shift-click going quiet. And a **global**
/// handler, since a plain key event is emitted only for the focused node that listens for
/// it: this one has to answer from wherever the keyboard is, including nowhere.
pub(crate) fn root_key_down(
    keys: ModifierKeys,
    searched: State<Searched>,
    dock: State<DockArea>,
    key: &Key,
    modifiers: Modifiers,
) {
    keys.down(key, modifiers);
    if is_search_chord(key, modifiers) {
        reach_search(searched, dock);
    }
}

pub fn app() -> impl IntoElement {
    // First of all, and here rather than in `main`: freya installs a panic hook of its
    // own inside `launch`, so this is where ours can be the outer one (`crate::panics`).
    use_hook(crate::panics::install);
    // Before everything else after that: the theme and the fonts are resolved from it and
    // both have to be right on the first frame.
    let prefs =
        use_provide_context(|| Prefs(State::create(EditedSettings::of(&Settings::load())))).0;
    use_settings_with(prefs, |settings: &Settings| settings.save());
    // freya's own components read their colours from its `Theme` rather than from the
    // palette, and the tooltip's font size can only be set there -- so a font change has
    // to be carried in rather than picked up by a re-render, hence the size in the deps.
    // Two calls and not one: `use_init_theme` builds its value in a `use_hook`, so it
    // answers for the first render only and the effect carries every later switch.
    let mut interface = use_init_theme(|| interface_theme(appearance()));
    use_side_effect_with_deps(
        &(appearance(), fonts().ui.size()),
        move |(appearance, _): &(Appearance, f32)| {
            interface.set(interface_theme(*appearance));
        },
    );

    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    let loading = use_provide_context(|| Loading(State::create(Loads::default()))).0;
    let docs = use_provide_context(|| OpenDocs(State::create(Docs::default()))).0;
    let sidebar_dock = use_state(|| {
        DockArea::column(vec![
            vec![
                Tab::View(View::Objects),
                Tab::View(View::Files),
                Tab::View(View::Search),
            ],
            vec![Tab::View(View::Symbols)],
            vec![
                Tab::View(View::History),
                Tab::View(View::Bookmarks),
                Tab::View(View::Locations),
            ],
        ])
    });
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
    let active = use_provide_context(move || {
        Active(Memo::create(move || {
            active_tab(&content_dock.read(), &docs.read())
        }))
    })
    .0;
    // 50.0: what the leading side starts at, before anything is dragged.
    use_provide_context(|| SplitRatio(State::create(50.0)));
    use_provide_context(|| {
        Splits(State::create(ResizableContext {
            direction: Direction::Horizontal,
            ..Default::default()
        }))
    });
    let asm_at = use_provide_context(|| AsmAt(State::create(Positions::default()))).0;
    let src_at = use_provide_context(|| SrcAt(State::create(Positions::default()))).0;
    let code_at = use_provide_context(|| CodeAt(State::create(Positions::default()))).0;
    let driven = use_provide_context(|| Drives(State::create(Driven::default()))).0;
    use_provide_context(|| Expanded(State::create(HashSet::new())));
    use_provide_context(|| Follows(State::create(HashMap::new())));
    let visits = use_provide_context(|| Visited(State::create(Visits::default()))).0;
    let bookmarks = use_provide_context(|| Bookmarked(State::create(Bookmarks::default()))).0;
    let landing = use_provide_context(|| Land(State::create(None))).0;
    let plant = use_provide_context(|| Plant(State::create(None))).0;
    let marked = use_provide_context(|| Marked(State::create(Marks::default()))).0;
    let marks_at = use_provide_context(|| MarksAt(State::create(Positions::default()))).0;
    let code_rows = use_provide_context(|| CodeRows(State::create(None))).0;
    let shift = use_provide_context(|| Shift(State::create(false))).0;
    let ctrl = use_provide_context(|| Ctrl(State::create(false))).0;
    let alt = use_provide_context(|| Alt(State::create(false))).0;
    let caps_is_ctrl = use_state(|| false);
    let control_held = use_state(|| false);
    let keys = ModifierKeys::new(shift, ctrl, alt, caps_is_ctrl, control_held);
    let proj = use_provide_context(|| Proj(State::create(OpenProject::default()))).0;
    let searched = use_provide_context(|| Searching(State::create(Searched::default()))).0;
    // At the root, not in the Project view: an inactive dock tab is unmounted, and a build
    // that survives the reader looking away cannot live there.
    let build = use_provide_context(|| Building(State::create(Builds::default()))).0;
    let states = ProjectStates {
        proj,
        objects,
        loading,
        open,
        asm_at,
        src_at,
        code_at,
        driven,
        marks_at,
        visits,
        bookmarks,
        searched,
        build,
    };
    use_save_on_change(states);
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    use_periodic_save();
    // After the save effect on purpose: its empty baseline must be in place before the
    // restore writes anything, so the restored session is seen as an ordinary change.
    use_restore_on_startup(states);
    // After the restore, which is the last of the loads a startup makes: `Settings::load`
    // above, the same again behind `fonts()`, and the project the line above reopened.
    // All three are synchronous, so one ask here catches everything they moved aside.
    use_provide_context(|| Rescued(State::create(rescue::moved())));

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

    let analysis = use_provide_context(|| Analysis(State::create(Analyzed::default()))).0;
    let located = use_provide_context(|| Locations(State::create(Located::default()))).0;
    let coded = use_provide_context(|| Coding(State::create(Coded::default()))).0;
    let reading = use_provide_context(|| Sections(State::create(Reading::default()))).0;
    let window = use_provide_context(|| Window(State::create(None))).0;
    use_reading_of(active, objects, reading, window);
    // The question and not the active document: a source-driven tab's assembly side
    // changes when a line in it is clicked, which changes no document.
    let asked = Asked { active, driven };
    use_analysis_with(
        asked, objects, visits, analysis, located, coded, reading, window, answer,
    );
    // After the analysis: the file the Source pane draws is what the analysis says it is.
    use_clear_marks(active, asked, analysis, marked);

    // The search's own worker, beside the analysis one and for its reasons: the walk reads
    // every file under the project directory, which is not the UI thread's to do.
    use_search_with(searched, |query, emit| crate::search::search(query, emit));

    // At the root rather than in the view: an inactive dock tab is unmounted, and neither
    // a buffer being typed into nor a program that was started can live there. The buffers
    // start empty and a pad gets its own when its source arrives.
    let pad = use_provide_context(|| Pad(State::create(Pads::default()))).0;
    let pad_text = use_provide_context(|| PadText(State::create(PadBuffers::default()))).0;
    use_scratchpad_with(pad, pad_text, states, pad_work);

    use_building_with(build, states, build_work);

    // At the root for the reason the other three are, and one more: a language server is a
    // process, and a process that outlives the view it was started from is one nothing can
    // stop.
    let language = use_provide_context(|| Talking(State::create(Language::default()))).0;
    let follow = use_provide_context(|| Following(State::create(Follow::default()))).0;
    use_language_with(language, follow, located, proj, language_work());
    // What a name followed in the source opens, which the answer above fills in.
    use_follow(follow, open, visits, marked, landing, plant, driven);

    // Docking cannot express a fixed pixel width, which is why the outer split is a
    // `ResizableContainer` and not a `DockingArea`: a fixed 300px sidebar beside the one
    // proportional panel, which therefore takes whatever is left.
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
                // `DockingArea` renders itself `.expanded()`, so it needs a parent that
                // has been given the height.
                .child(docking_area(content_dock, docs)),
        );

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
        .on_global_pointer_down(move |e: Event<PointerEventData>| match e.button() {
            Some(MouseButton::Back) => navigate(open, Nav::Back),
            Some(MouseButton::Forward) => navigate(open, Nav::Forward),
            _ => {}
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
            root_key_down(keys, searched, content_dock, &e.key, e.modifiers)
        })
        .on_global_key_up(move |e: Event<KeyboardEventData>| keys.up(&e.key, e.modifiers))
        // Provides the root state `ContextMenu::open_from_event` looks up: opening a menu
        // without one in an ancestor scope panics. It lays out as nothing until a menu
        // is open.
        .child(ContextMenuViewer::new())
        // Over everything, and drawn as nothing at all until a file has been moved aside.
        .child(RescuedPopup)
        .child(toolbar(objects, loading))
        // Under the bar rather than in the view that has the other Start button: the
        // control above is pressed from wherever the reader is, and a question drawn
        // where they are not looking is a press that did nothing. Lays out as nothing
        // while there is nothing to ask.
        .child(TrustPrompt)
        // `ResizableContainer` renders itself `.expanded()`, so it needs a parent that
        // has already been given the leftover height under the toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(split),
        )
}

#[cfg(test)]
mod tests;

//! The app's own tab bar and what it draws: the chips, the × on one, the list of every
//! open tab, and the body under it all.
//!
//! **The bar is the app's and not the dock's.** It cannot be folded away, split, or
//! dragged out of; what is open is a [`Strip`] the app holds, and a chip is a plain
//! element that activates its own tab. The sidebar keeps freya's docking
//! (`src/ui/dock.rs`), where a panel is furniture the reader may arrange.

use super::*;

/// What a chip is: an ordinary one in the bar, the tab on screen, or the copy that follows
/// the cursor while a tab is dragged. One value and not a column of flags, a chip being
/// exactly one of the three, and only the tab on screen having a keyboard to be inside it.
///
/// Whether a drop would land here is none of them: that rule is on another edge and is
/// worn with any of the three, the tab on screen being the one a reader most often drags.
#[derive(Clone, Copy)]
enum Mark {
    /// A chip like any other in the bar.
    Plain,
    /// The tab on screen, with whether the keyboard is inside it.
    Active { typing: bool },
    /// The copy that follows the cursor: the ground a drop lands on, no rule, and nothing
    /// that answers a pointer.
    Dragging,
}

/// One tab's chip: the icon naming its kind, what it is called, the × that closes it, and
/// the pane's own white when it is the one on screen.
///
/// **The press activates the tab**, this being the app's own bar: there is no wrapper
/// above it that does so, the way freya's docking has one. The × therefore has to stop
/// the press from reaching here, or a close would first switch to the tab it is closing.
///
/// **The tab on screen wears a rule along its top**, and the colour says where the keyboard
/// is: the gutter marks' own purple while it is inside the tab, and a dim grey while it is
/// anywhere else -- a sidebar list, a filter box. The mark is drawn on the chip that is
/// showing and on no other, so the bar says which tab is being read and whether it is
/// being typed into, without a second wash to tell from the first. `landing` is the other
/// rule, down the leading edge of the chip a dragged tab would land on.
///
/// A temporal tab -- the preview a sidebar row opens in, which the next row reuses -- is
/// told from one that stays by its name being **italic**, and by nothing else: it is the
/// same tab in every other way, and the slant is the one cue that says "provisional"
/// without taking room from the name.
///
/// A stateless helper rather than a component, so no hook runs here: the hover is the
/// caller's `use_state`, handed over to be read and written, and [`dragged`] passes `None`
/// for it, having nothing to hover. The × is a control of its own for the same reason and
/// arrives as a [`Tab`], which is all the identity a close needs. The caller adds the
/// press, the menu and the tooltip; the frame, the padding and the spacing are here, once,
/// for the bar and the drag alike.
fn chip(
    icon: Element,
    text: &str,
    mark: Mark,
    landing: bool,
    temporal: bool,
    hovering: Option<State<bool>>,
    close: Option<Tab>,
) -> Rect {
    let hovered = hovering.is_some_and(|hovering| hovering());
    // The active chip takes the pane's own background, so it reads as the top edge of the
    // pane below it. The hover stays lighter than that, or it would be more prominent
    // than the active tab.
    let background = match mark {
        Mark::Active { .. } => palette().pane_bg,
        Mark::Dragging => palette().selected_bg,
        Mark::Plain if hovered => palette().toggle_hover_bg,
        Mark::Plain => Color::TRANSPARENT,
    };
    // And a tab that is not the one on screen writes its name a step back, so the bar says
    // which tab is being read in the text as well as in the ground under it. A step and not
    // a fade: these are names the reader reads their way along.
    let name = match mark {
        Mark::Active { .. } | Mark::Dragging => palette().text_fg,
        Mark::Plain => faded(
            palette().text_fg,
            match hovered {
                true => palette().toggle_hover_bg,
                false => palette().header_bg,
            },
        ),
    };

    // Where a tab being dragged would land: the leading edge of the chip under the
    // pointer, in the same purple the tab on screen is marked with.
    let edge = landing.then(|| {
        Border::new()
            .fill(palette().compiled_fg)
            .width(BorderWidth {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: TAB_MARKER,
            })
    });

    // Painted and not laid out, so the mark takes no room from the name: a border is drawn
    // inside the box it is on.
    let marker = match mark {
        Mark::Active { typing } => Some(
            Border::new()
                .fill(match typing {
                    true => palette().compiled_fg,
                    false => dimmed(palette().icon_fg, palette().pane_bg),
                })
                .width(BorderWidth {
                    top: TAB_MARKER,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                }),
        ),
        Mark::Plain | Mark::Dragging => None,
    };

    // A chip is cut by the count and never by the room it has: `elide` is what shortened
    // it, and the bar scrolls rather than squeezing a chip (`metrics.rs`).
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .height(Size::px(tab_row_height()))
        // Air to the left of the icon and next to none to the right: what sits at that
        // end is the ×, which is a target of its own and carries its own.
        .padding(Gaps::new(0.0, 2.0, 0.0, 8.0))
        .spacing(6.0)
        .background(background)
        .border(right_hairline())
        .border(marker)
        .border(edge)
        .map(hovering, |chip, mut hovering| {
            chip.on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
        })
        .child(icon)
        .child(
            label()
                .text(elide(text))
                .color(name)
                .max_lines(1)
                .maybe(temporal, |chip| chip.font_slant(FontSlant::Italic)),
        )
        .maybe_child(close.map(|tab| TabClose { tab }.into_element()))
}

/// The × on a document's tab: **a target with padding around the glyph rather than a
/// bigger glyph**, and a wash of its own under the pointer.
///
/// A component and not another line of [`chip`] because the hover has to be *this*
/// control's, and freya has no `.hover()` pseudo-state: it is a `use_state` with
/// `on_pointer_over`/`on_pointer_out` around it, and a hook cannot run in a helper. The
/// tab under it stays lit at the same time -- the two are told apart by the wash being
/// deeper, not by the tab going out -- and the glyph comes up from `address_fg` to the
/// interface text, so what is about to happen is said twice.
///
/// It closes the tab itself rather than taking a handler: a `Component` is `PartialEq`, a
/// closure is not, and the [`Tab`] is all the identity a close needs.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct TabClose {
    pub(crate) tab: Tab,
}

impl Component for TabClose {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        let places = use_places();
        let tab = self.tab;

        rect()
            .width(Size::px(close_target()))
            .height(Size::px(close_target()))
            // Two pixels between the wash and whatever the × is drawn at the end of:
            // the chip's own right padding is two, which is not enough room for a square
            // that lights up. The control's own and not the chip's, so the × carries them
            // into the tab list's rows as well.
            .margin(Gaps::new(0.0, 2.0, 0.0, 0.0))
            .center()
            .corner_radius(4.0)
            .background(if hovering() {
                palette().close_hover_bg
            } else {
                Color::TRANSPARENT
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            // Without the `stop_propagation` the press reaches the chip under it and the
            // close first switches to the tab it is closing.
            .on_press(move |e: Event<PressEventData>| {
                e.stop_propagation();
                match tab {
                    Tab::Document(id) => close_tab(open, places, id),
                    Tab::Page(page) => close_page(open, page),
                }
            })
            .child(
                label()
                    .text("\u{00d7}")
                    .font_size(close_glyph())
                    .color(if hovering() {
                        palette().text_fg
                    } else {
                        palette().address_fg
                    })
                    .max_lines(1),
            )
    }
}

/// The control that opens a list of every open tab, pinned at the **right** of the bar so
/// it never scrolls away with the tabs it is there to reach. It lists all of them and not
/// only the hidden ones: which are off-screen would mean measuring the bar against its
/// viewport, and a list whose length changed as the bar was dragged would be worse to use.
///
/// The popup is positioned here rather than through `ContextMenu`, which pins a menu's
/// top-left corner to the pointer and clamps to nothing -- opened from a button at the
/// right-hand edge it would draw off the side of the window.
#[derive(PartialEq)]
pub(crate) struct TabListButton;

impl Component for TabListButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        let open = use_open();

        // Every tab and the one on screen, read together so the menu is built from one
        // look at the strip.
        let (tabs, active) = {
            let strip = open.strip.read();
            (strip.tabs().to_vec(), strip.active())
        };
        if tabs.is_empty() {
            return rect().into_element();
        }

        let button = extra_tooltip(
            "Open tabs".to_owned(),
            rect()
                .width(Size::px(TAB_LIST_WIDTH))
                .height(Size::px(tab_row_height()))
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
                // needed: global listeners are snapshotted when the event is measured,
                // before any handler runs, so the menu this press opens is not in that
                // batch. A popup opened from a `*_down` handler is the case that does need
                // the swallow; copying it here ate the first press outside the menu.
                .on_press(move |_| {
                    let was = showing();
                    showing.set(!was);
                })
                .child(glyph(("chevron-down", lucide::chevron_down()))),
        );

        rect()
            .width(Size::px(TAB_LIST_WIDTH))
            .height(Size::px(tab_row_height()))
            .child(button)
            .maybe_child(showing().then(|| {
                rect()
                    // Under the bar and aligned to its right-hand edge, so the list opens
                    // leftward into the window instead of off the side of it.
                    .position(Position::new_absolute().top(tab_row_height()))
                    .child(
                        tabs_menu(open, &tabs, active, showing)
                            .on_close(move |_| showing.set(false))
                            // Keyed by row count so a list that grows while the menu is
                            // open remounts it: `MenuContainer` measures itself once and
                            // keeps that offset, so a menu that widens afterwards hangs
                            // off the side of the window.
                            .key(tabs.len()),
                    )
                    .into_element()
            }))
            .into_element()
    }
}

/// The menu [`TabListButton`] opens: one row per open tab, in the bar's own order, with
/// the one on screen marked. Built per press, like `close_menu`.
fn tabs_menu(open: Open, tabs: &[Tab], active: Option<Tab>, mut close: State<bool>) -> Menu {
    // Names and glyphs resolved in one pass, so the read guard on the table is gone before
    // any row's handler can run and write to it.
    let rows: Vec<(Tab, String, Element)> = {
        let docs = open.docs.read();
        tabs.iter()
            .map(|tab| (*tab, elide(&tab_title(*tab, &docs)), tab_icon(*tab, &docs)))
            .collect()
    };

    rows.into_iter()
        .fold(Menu::new(), |menu, (tab, title, icon)| {
            menu.child(
                // `MenuItem` and not `MenuButton`: this menu has a *current* row, and
                // `selected` is freya's own way of drawing one.
                MenuItem::new()
                    .selected(Some(tab) == active)
                    .on_press(move |_| {
                        // A tab already open is a place the reader has, so going to it is
                        // a move and records nothing.
                        raise_tab(open, tab);
                        close.set(false);
                    })
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .width(Size::fill())
                            // Wide enough that the × is out at the row's own end rather
                            // than against the name, and every row's sits under the one
                            // above it: a menu is otherwise as wide as its longest name,
                            // which for a strip of short ones is barely wider than the ×.
                            .min_width(Size::px(TAB_LIST_ROW_WIDTH))
                            // The name is given what the × and the icon leave.
                            .content(Content::Flex)
                            .spacing(6.0)
                            .child(icon)
                            // `max_lines(1)`, or a name longer than the menu is wide wraps
                            // and the row grows to hold it.
                            .child(label().text(title).max_lines(1).width(Size::flex(1.0)))
                            .child(TabClose { tab }),
                    ),
            )
        })
}

/// What a tab is called in a list. Not elided here -- the chip decides how much of a name
/// it has room for.
fn tab_title(tab: Tab, docs: &Docs) -> String {
    match tab {
        Tab::Page(page) => page.title().to_owned(),
        Tab::Document(id) => docs.get(id).map(entry_text).unwrap_or_default(),
    }
}

/// The Lucide glyph drawn before that title.
fn tab_icon(tab: Tab, docs: &Docs) -> Element {
    match tab {
        Tab::Page(page) => page_icon(page),
        Tab::Document(id) => match docs.get(id) {
            Some(document) => entry_icon(document),
            None => rect().into_element(),
        },
    }
}

/// The glyph a page's tab is drawn with.
fn page_icon(page: Page) -> Element {
    match page {
        Page::Project => glyph(("folder-open", lucide::folder_open())),
        Page::Settings => glyph(("settings", lucide::settings())),
        Page::Shortcuts => glyph(("keyboard", lucide::keyboard())),
        Page::Scratchpad => glyph(("notebook-pen", lucide::notebook_pen())),
        Page::Debug => glyph(("bug", lucide::bug())),
    }
}

/// What a page's tab draws under the bar -- and what a window with no project draws in
/// place of its screen, there being no bar there to put a tab in.
pub(crate) fn page_body(page: Page) -> Element {
    match page {
        Page::Project => ProjectTab.into_element(),
        Page::Settings => SettingsTab.into_element(),
        Page::Shortcuts => ShortcutsTab.into_element(),
        Page::Scratchpad => ScratchpadTab.into_element(),
        Page::Debug => DebugTab.into_element(),
    }
}

/// The menu at the **top left of the window**: the ways in and out of a project, and under
/// them Project, Settings and the Scratchpad -- the whole of the way back to a page that has
/// been closed.
///
/// It lists all three pages and marks the ones that are open, rather than listing only the
/// closed ones: a menu whose rows come and go as tabs are closed is a menu a reader has to
/// read every time, where a list that is always the same three is one they learn. Picking
/// an open one shows it, which is what the reader meant by picking it.
///
/// **An item that would do nothing is left out** rather than drawn dim, freya's `MenuItem`
/// having no disabled state and the app answering that the way it does on a tab's menu:
/// Save as is not there with no project open, nor for one the app is keeping, which has Save
/// in the bar instead; Close project and Project are not there with no project at all.
///
/// The popup is positioned by hand, as [`TabListButton`]'s is: `ContextMenu` pins a menu
/// to the pointer and clamps to nothing.
#[derive(PartialEq)]
pub(crate) struct PagesButton;

impl Component for PagesButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        // Whether the menu that is up was opened with Alt held. Kept from the press rather
        // than read per render: the reader lets the key go to reach for the row, and a row
        // that vanished under their hand would be worse than not offering it at all.
        let mut asked = use_state(|| false);
        let alt = use_consume::<Alt>().0;
        let states = use_project_states();
        let rescued = use_consume::<Rescued>().0;
        let unopened = use_consume::<Unopened>().0;
        // Read and not peeked: the marks are drawn from it, so the menu has to follow a
        // page opening or closing while it is up.
        let strip = states.open.strip.read();
        // The Debug page is the ways to make the app misbehave on purpose, so it is not in
        // a menu the reader opened to get to their project. Alt held as the menu is opened
        // is what asks for it -- no rebuild, no variable, and nothing on screen for a
        // reader who has not asked.
        let pages: Vec<Page> = Page::ALL
            .into_iter()
            .filter(|page| *page != Page::Debug || asked())
            .collect();
        let is_open: Vec<bool> = pages
            .iter()
            .copied()
            .map(|page| strip.contains(Tab::Page(page)))
            .collect();
        drop(strip);
        // Read when the menu is opened and not per render: each row is a small read of
        // another project's own file.
        let recents = match showing() {
            true => recents_of(states.store),
            false => Vec::new(),
        };

        let side = toggle_size();
        let button = extra_tooltip(
            "Projects, Settings and the Scratchpad".to_owned(),
            rect()
                .width(Size::px(side))
                .height(Size::px(side))
                .center()
                .corner_radius(4.0)
                .background(if showing() || hovering() {
                    palette().toggle_hover_bg
                } else {
                    Color::TRANSPARENT
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    // Read here and not in the render: this press is the moment the
                    // reader is asking about, and a freya pointer event carries no
                    // modifiers of its own, which is what `Alt` is kept for.
                    let held = *alt.peek();
                    let was = showing();
                    asked.set(!was && held);
                    showing.set(!was);
                })
                .child(glyph(("menu", lucide::menu()))),
        );

        rect()
            .width(Size::px(side))
            .height(Size::px(side))
            .child(button)
            .maybe_child(showing().then(|| {
                rect()
                    // Under the button and hanging from its left edge, which is the
                    // window's: the menu opens rightward into it.
                    .position(Position::new_absolute().top(side))
                    .child(
                        main_menu(
                            states, rescued, unopened, &recents, &pages, &is_open, showing,
                        )
                        .on_close(move |_| showing.set(false)),
                    )
                    .into_element()
            }))
            .into_element()
    }
}

/// One row of that menu: a word, and what pressing it does. A helper and not a component,
/// the hover being `MenuItem`'s own.
pub(crate) fn menu_row(
    text: &str,
    mut close: State<bool>,
    mut act: impl FnMut() + 'static,
) -> MenuButton {
    let text = text.to_owned();
    MenuButton::new()
        .on_press(move |_| {
            act();
            close.set(false);
        })
        .child(label().text(text).max_lines(1))
}

/// What freya lays a `MenuItem` out at, so a row of the app's own beside them lines up.
/// Neither is reachable from the theme, so both are written here and pinned by a test.
const MENU_ROW_WIDTH: f32 = 105.0;
const MENU_ROW_PADDING: (f32, f32) = (6.0, 12.0);

/// The mark on a row that opens a submenu: freya's `SubMenu` draws none, so such a row is
/// otherwise the twin of one that acts. The glyph the Files tree folds with, so the app
/// points one way everywhere.
const SUBMENU_ARROW: &str = "\u{25b8}";

/// One of those rows: the name, and the arrow after it.
///
/// **After the name and not out at the row's own end**, which is where a desktop menu puts
/// it. A row here is a `MenuItem` -- `fill_minimum` inside a container that fits its
/// content -- so a child asking to fill takes the *window* and drags the menu out to it,
/// and nothing in the row can learn how wide the widest row made the menu
/// (`notes/upstream/freya.md`). The gap is what keeps the mark from reading as part of the
/// word.
///
/// `colour` is the dim row's, which is drawn in place of the live one and has to look like
/// it; a live row inherits the menu's own and is handed `None`. The arrow is a step back
/// from the name either way, being a mark about the row rather than part of what it says.
fn submenu_label(text: &str, colour: Option<Color>) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.0)
        .child(
            label()
                .text(text.to_owned())
                .max_lines(1)
                .map(colour, |name, colour| name.color(colour)),
        )
        .child(
            label()
                .text(SUBMENU_ARROW.to_owned())
                .max_lines(1)
                .color(colour.unwrap_or_else(|| palette().address_fg)),
        )
        .into_element()
}

/// A line between two groups of the menu. freya has no separator, and a `Menu` takes any
/// child, so it is a rect a pixel high in the colour the panes are divided by.
fn menu_rule() -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(1.0))
        .margin(Gaps::new_symmetric(4.0, 0.0))
        .background(palette().hairline)
        .into_element()
}

/// The menu [`PagesButton`] opens. Built per press, like the bar's own, which is what lets
/// it leave out the items that would do nothing.
fn main_menu(
    states: ProjectStates,
    rescued: State<Vec<PathBuf>>,
    unopened: State<Option<project::Failure>>,
    recents: &[Recent],
    // The pages this run has, asked for by the caller: `is_open` is one per page in this
    // order, and the two cannot be allowed to disagree.
    pages: &[Page],
    is_open: &[bool],
    close: State<bool>,
) -> Menu {
    let open = states.proj.peek().file.clone();
    let unsaved = open.as_deref().is_some_and(project::unsaved);

    let mut menu = Menu::new()
        .child(menu_row("Open a project...", close, move || {
            ask_for_a_project(states, rescued, unopened)
        }))
        .child(recents_submenu(states, rescued, unopened, recents, close))
        .child(menu_row(
            "Open a directory as a project...",
            close,
            move || ask_for_a_directory(states),
        ))
        .child(menu_row("Open a file as a project...", close, move || {
            ask_for_a_binary(states)
        }));

    if open.is_some() && !unsaved {
        menu = menu.child(menu_row("Save as...", close, move || {
            ask_where_to_save(states, project::Put::Copy)
        }));
    }
    if open.is_some() {
        menu = menu.child(menu_row("Close project", close, move || {
            close_project(states)
        }));
    }

    menu.child(menu_rule()).children(
        pages
            .iter()
            .copied()
            .zip(is_open.iter().copied())
            // The Project page is a reading of a project, so with none there is nothing
            // for it to draw.
            .filter(|(page, _)| *page != Page::Project || open.is_some())
            .map(|(page, open_already)| {
                let mut strip = states.open.strip;
                let mut close = close;
                MenuItem::new()
                    .selected(open_already)
                    .on_press(move |_| {
                        // `show` and not `push`: a page opens beside the tab on screen,
                        // the way anything else the reader opens does, and one already
                        // open is only raised. With no project it is still a tab: the bar
                        // comes back for it, and closing it takes the bar away again.
                        strip.write().show(Tab::Page(page));
                        close.set(false);
                    })
                    .child(
                        rect()
                            .horizontal()
                            .cross_align(Alignment::Center)
                            .spacing(6.0)
                            .child(page_icon(page))
                            .child(label().text(page.title()).max_lines(1)),
                    )
                    .into_element()
            })
            .collect::<Vec<Element>>(),
    )
}

/// The projects there have been, under one row of the menu. **Keyed by how many there
/// are**: `MenuContainer` measures itself once and keeps that offset, so a list that grew
/// after it was laid out would hang off the side of the window
/// (`notes/upstream/freya.md`).
pub(crate) fn recents_submenu(
    states: ProjectStates,
    rescued: State<Vec<PathBuf>>,
    unopened: State<Option<project::Failure>>,
    recents: &[Recent],
    close: State<bool>,
) -> Element {
    // Nothing to open: the row stays, drawn dim, rather than going away. It is the one
    // item here that is about the reader's *own* past, and a reader looking for a project
    // they had open should be told the list is empty rather than left to wonder where the
    // item went -- which is what leaving it out would say. A bare `rect` and not a
    // `MenuItem`: freya has no disabled item, and a dead row that still lights under the
    // pointer would be saying it can be pressed.
    if recents.is_empty() {
        return rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .min_width(Size::px(MENU_ROW_WIDTH))
            .padding(MENU_ROW_PADDING)
            .child(submenu_label(
                "Open recent",
                Some(dimmed(palette().text_fg, palette().pane_bg)),
            ))
            .into_element();
    }

    let rows: Vec<Element> = recents
        .iter()
        .map(|recent| {
            let path = recent.path.clone();
            menu_row(&project::label(&recent.path), close, move || {
                switch_project(states, rescued, unopened, path.clone())
            })
            .key(recent.path.to_string_lossy().into_owned())
            .into_element()
        })
        .collect();

    SubMenu::new()
        .label(submenu_label("Open recent", None))
        .children(rows)
        .key(recents.len())
        .into_element()
}

/// How wide [`TabListButton`] is.
pub(crate) const TAB_LIST_WIDTH: f32 = 26.0;

/// How wide a row of the list it opens is, at the least. A floor and not a width: a name
/// longer than this still has the room it needs, the menu growing to its longest row.
const TAB_LIST_ROW_WIDTH: f32 = 220.0;

/// One tab's chip, with the hover state a chip cannot hold for itself.
#[derive(Clone)]
pub(crate) struct TabHeader {
    pub(crate) tab: Tab,
    /// Whether this is the tab on screen.
    pub(crate) active: bool,
    /// Whether a tab being dragged would land here.
    pub(crate) landing: bool,
    pub(crate) key: DiffKey,
}

impl PartialEq for TabHeader {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab && self.active == other.active && self.landing == other.landing
    }
}

impl KeyExt for TabHeader {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for TabHeader {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        // Consumed here, in the render, for the menu: its handler may not run a hook.
        let states = use_project_states();
        let open = states.open;
        let keyboard = use_consume::<Keyboard>().0;
        let tab = self.tab;
        // Asked only of the chip that is showing, which is the only one that draws the
        // mark: asking is a subscription to the focus moving, and every chip taking one
        // would re-render the whole bar whenever it did.
        let mark = match self.active {
            true => Mark::Active {
                typing: keyboard_in_tab(keyboard),
            },
            false => Mark::Plain,
        };

        // What the chip is called and whether it is the temporal one, out of one read: the
        // chip follows the trail's current entry, so navigating in place renames it. A
        // page's name is its own, and leaving the table unread keeps a page's chip out of
        // every re-render a document causes.
        let (icon, text, tooltip, temporal) = match tab {
            Tab::Page(page) => (
                page_icon(page),
                page.title().to_owned(),
                page.title().to_owned(),
                false,
            ),
            Tab::Document(id) => {
                let docs = open.docs.read();
                // What it draws and what hovering it says out of one name: on a
                // symbol's tab the first is the short spelling of the second
                // (`entry_labels`).
                let (text, tooltip) = docs.get(id).map(entry_labels).unwrap_or_default();
                (
                    tab_icon(tab, &docs),
                    text,
                    tooltip,
                    docs.temporal() == Some(id),
                )
            }
        };

        name_tooltip(
            elided(&text),
            &text,
            tooltip,
            chip(
                icon,
                &text,
                mark,
                self.landing,
                temporal,
                Some(hovering),
                Some(tab),
            )
            // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
            // without it panics. A right-click is not a press, so this leaves the tab it
            // was opened on where it is rather than activating it first.
            .on_secondary_down(move |e: Event<PressEventData>| {
                // Read at the press rather than at the render: whether this tab has
                // company is not something the chip draws, so subscribing to the strip
                // for it would re-render every tab whenever any one of them opened. The
                // only tab open still gets its menu, the bookmark item being about the
                // tab itself; what it does without is the one row that would do nothing.
                // The document the rows are about is peeked here for the same reason: the
                // chip draws a name, not the entry behind it.
                let others = open.strip.peek().tabs().iter().any(|other| *other != tab);
                let subject = match tab {
                    Tab::Document(id) => open.docs.peek().get(id).cloned(),
                    Tab::Page(_) => None,
                };
                ContextMenu::open_from_event(&e, tab_menu(states, tab, others, subject));
            })
            .on_press(move |e: Event<PressEventData>| {
                raise_tab(open, tab);
                // The reader is going to read in it, so the keyboard goes there too: what
                // it lands on is the pane the tab is driven from (`use_keyboard_asked`).
                ask_for_keyboard(keyboard);
                // A double press on a temporal tab's chip makes it a tab that stays.
                // freya counts the presses (500 ms, 5 px), and nothing else on the chip
                // asks it, so the count is this handler's own.
                let Tab::Document(id) = tab else {
                    return;
                };
                let PressEventData::Mouse(mouse) = e.data() else {
                    return;
                };
                if !EventsCombos::pressed(mouse.global_location).is_double() {
                    return;
                }
                // Peeked in a statement of its own, so the guard is gone before the write.
                let temporal = open.docs.peek().temporal() == Some(id);
                if temporal {
                    let mut docs = open.docs;
                    docs.write().promote(id);
                }
            }),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// How thick the rule over the tab on screen is.
pub(crate) const TAB_MARKER: f32 = 2.0;

/// How wide the empty ground past the last chip is.
const PAST_LAST_TAB: f32 = 24.0;

/// The bar: a horizontally scrolling row of chips, since these are opened by the dozen,
/// with [`TabListButton`] pinned beside it. The scrollbar is off -- it would eat a third
/// of a one-row bar, and the wheel and a drag still move it.
///
/// **A chip can be dragged along the bar to move it**, which is the one thing here freya
/// is asked for: each chip is a `DropZone` around a `DragZone`, the pattern its own docking
/// uses, and a drop on a chip puts the dragged tab where that chip is. The zone past the
/// last chip is the one that appends. A drop anywhere else changes nothing: `DragZone`
/// clears the payload on the release wherever it lands, and nothing but these zones acts on
/// one.
///
/// **The strip scrolls itself, without a `ScrollView`.** It is one row, and what it needs is
/// an offset: the wheel over it moves that offset sideways, opening a tab or going to one
/// brings its chip into view ([`use_reveal`]), and a drag held near either end scrolls the
/// strip under the pointer. freya's own scroll view answers none of the three well -- a
/// controller handed to it from outside only reaches it when something else happens to
/// re-render it, its wheel means the vertical axis, and it stops answering the wheel at all
/// while a drag is under way (`notes/upstream/freya.md`) -- and a row of chips needs no
/// scrollbar, no keyboard scrolling and no drag-to-scroll to make up for.
///
/// Where the chips are is **measured** and not worked out, a chip being as wide as its name.
/// The measurements are peeked and never read, so a layout wakes nothing on its own; what
/// wakes the reveal is `shape`, which counts up when a chip changes width or moves **along
/// the row** -- a tab opened, closed or moved -- and not when the row slides under the
/// window, which is the strip being scrolled. Each chip is therefore measured with the
/// offset taken back off.
#[derive(PartialEq)]
pub(crate) struct TabBar;

impl Component for TabBar {
    fn render(&self) -> impl IntoElement {
        let strip = use_open().strip;
        // Where a drop would land, and whether anything is being dragged at all: the
        // second is what makes the first mean something, a zone the pointer left last time
        // never having been told the drag ended (`DragZone` clears the payload itself).
        let landing = use_state(|| None);
        let drag = use_drag::<Tab>();
        let over = drag.read().is_some().then(|| landing()).flatten();

        let bar = use_bar();
        use_reveal(strip, bar);

        let (tabs, active) = {
            let strip = strip.read();
            (strip.tabs().to_vec(), strip.active())
        };
        // A chip that has gone is never measured again, so its place would sit here for
        // the rest of the session. It is dropped once its tab is no longer open, which
        // costs the reveal nothing: the tab on screen is one of these.
        use_side_effect_with_deps(&tabs, move |tabs: &Vec<Tab>| {
            let mut places = bar.places;
            let closed = places.peek().keys().any(|tab| !tabs.contains(tab));
            if closed {
                places.write().forgetting(|tab| tabs.contains(tab));
            }
        });

        // The table read once, here, for the copies that follow the cursor: a hook may not
        // run in the loop that builds the chips.
        let docs = use_open().docs;
        let chips: Vec<Element> = tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let tab = *tab;
                // Keyed by the tab, so a tab that moves takes its hover, its tooltip and
                // its open menu with it instead of leaving them on whatever took its place.
                let header = TabHeader {
                    tab,
                    active: Some(tab) == active,
                    landing: over == Some(index),
                    key: DiffKey::None,
                }
                .key(tab);
                drop_zone(
                    strip,
                    drag,
                    landing,
                    index,
                    rect()
                        .on_sized(move |e: Event<SizedEventData>| {
                            bar.chip_sized(tab, e.area.min_x(), e.area.max_x())
                        })
                        .child(
                            DragZone::new(tab, header.into_element())
                                .drag_element(dragged(tab, &docs.read())),
                        )
                        .into_element(),
                )
            })
            .collect();

        rect()
            .width(Size::fill())
            .height(Size::px(tab_row_height()))
            .horizontal()
            // The button takes its own width and the tabs are given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().header_bg)
            .border(bottom_hairline())
            // On the global move because the pointer is over a chip, not over the strip's
            // own box, for the whole of the gesture.
            .on_global_pointer_move(move |e: Event<PointerEventData>| {
                bar.drag_edge(drag.peek().is_some(), e.global_location().x as f32)
            })
            .child(
                rect()
                    .width(Size::flex(1.0))
                    .height(Size::fill())
                    // What is past the strip's own edge is not drawn, this being what makes
                    // the offset below a scroll rather than a row hanging out of the window.
                    .overflow(Overflow::Clip)
                    .on_sized(move |e: Event<SizedEventData>| {
                        bar.viewport_sized(e.area.min_x(), e.area.max_x())
                    })
                    .on_wheel(move |e: Event<WheelEventData>| {
                        bar.wheel(e.delta_x as f32, e.delta_y as f32)
                    })
                    .child(
                        rect()
                            .horizontal()
                            .height(Size::fill())
                            .on_sized(move |e: Event<SizedEventData>| {
                                bar.content_sized(e.area.width())
                            })
                            // The scroll itself: the row slides under the box above.
                            .offset_x(*bar.offset.read())
                            .children(chips)
                            .child(
                                // The ground past the last chip, and the drop that appends.
                                drop_zone(
                                    strip,
                                    drag,
                                    landing,
                                    tabs.len(),
                                    rect()
                                        .width(Size::px(PAST_LAST_TAB))
                                        .height(Size::fill())
                                        .into_element(),
                                ),
                            )
                            .into_element(),
                    ),
            )
            .child(TabListButton)
    }
}

/// Every measurement the bar keeps, made in one place: the hook [`TabBar`] opens with.
fn use_bar() -> Bar {
    let places = use_state(Chips::default);
    // The test that asks what the bar still holds a place for hands it the list to keep
    // them in, there being no reading a component's own state from outside.
    #[cfg(test)]
    let places = try_consume_context::<Measured>().map_or(places, |measured| measured.0);
    let shape = use_state(|| 0u64);
    Bar {
        places,
        viewport: use_state(|| None),
        content: use_state(|| 0.0f32),
        // Read as well as written: the row of chips is drawn at this offset, so the bar
        // has to be woken when it changes.
        offset: use_state(|| 0.0f32),
        shape,
        // A read, which is what subscribes the bar to a new shape.
        laid_out: shape(),
    }
}

/// Where every chip is: its two sides along the row, one entry per open tab. The map the
/// panes keep their places in, keyed by a tab rather than by a place on one.
pub(super) type Chips = Positions<Tab, (f32, f32)>;

/// The list a test hands the bar to measure its chips into, so that it can read them.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct Measured(pub(crate) State<Chips>);

/// What the bar has been measured as, how far along it is, and every rule over the two:
/// passed about as one thing because nothing that scrolls can do without all of it.
#[derive(Clone, Copy)]
pub(super) struct Bar {
    /// Every chip's two sides, along the row: where each was laid out, less the offset,
    /// so that scrolling the strip moves none of them. Only the open tabs: nothing
    /// measures a chip that has gone, so its entry is dropped when its tab closes.
    pub(super) places: State<Chips>,
    /// The two sides of the strip: what a chip has to be inside to be in view.
    pub(super) viewport: State<Option<(f32, f32)>>,
    /// How wide the chips are altogether.
    pub(super) content: State<f32>,
    /// How far the row of chips is slid to the left, which is never positive.
    pub(super) offset: State<f32>,
    /// Counts up whenever the bar takes a new shape, which is what wakes the reveal.
    pub(super) shape: State<u64>,
    /// What that count said as the bar was drawn. A new shape is counted from here rather
    /// than from the count itself, so a whole layout's chips settling at once is one new
    /// shape and not one apiece.
    pub(super) laid_out: u64,
}

impl Bar {
    /// Where a chip sits **along the row**: where it was laid out, less how far the row is
    /// slid under the window.
    ///
    /// A chip that moved along the row or changed width is the bar taking a new shape -- a
    /// tab opened, closed or moved. The strip being scrolled moves every chip in the
    /// window and none along the row, and owes the reveal nothing. Under a pixel is the
    /// subtraction and not a move, each end being worked out from whatever offset the row
    /// was drawn at.
    pub(super) fn chip_sized(self, tab: Tab, min_x: f32, max_x: f32) {
        let slid = *self.offset.peek();
        let at = (min_x - slid, max_x - slid);
        let held = self.places.peek().at(&tab);
        let shifted =
            held.is_none_or(|(min, max)| (min - at.0).abs() >= 1.0 || (max - at.1).abs() >= 1.0);
        if !shifted {
            return;
        }
        let mut places = self.places;
        places.write().remember(tab, at);
        self.reshaped();
    }

    /// Where the strip is cut off, which is what a chip is measured against. A strip of
    /// another size is a new shape: what was in view was in view of the old one.
    pub(super) fn viewport_sized(self, min_x: f32, max_x: f32) {
        let at = Some((min_x, max_x));
        if *self.viewport.peek() == at {
            return;
        }
        let mut viewport = self.viewport;
        viewport.set(at);
        self.reshaped();
    }

    /// How wide the chips are altogether, which is what says how far the strip may be
    /// scrolled. A bar that has lost a chip may now fit the window: a scroll of nothing
    /// puts the offset back inside the new floor, which [`Bar::scroll_by`] clamps against
    /// only as it moves. Otherwise a closed tab leaves empty ground past the last chip
    /// until something scrolls.
    pub(super) fn content_sized(self, width: f32) {
        if *self.content.peek() == width {
            return;
        }
        let mut content = self.content;
        content.set(width);
        self.scroll_by(0.0);
    }

    /// The wheel over the strip is the strip's own axis, whichever axis it arrives on: a
    /// bar has no second one, and a reader turning the wheel over it means "further
    /// along".
    pub(super) fn wheel(self, delta_x: f32, delta_y: f32) {
        let by = match delta_y.abs() > delta_x.abs() {
            true => delta_y,
            false => delta_x,
        };
        self.scroll_by(by);
    }

    /// A drag held near either end of the strip scrolls it towards that end, `x` being
    /// where the pointer is in the window. It is the only way to reach the far end while
    /// carrying a tab, and a pointer held anywhere else moves nothing.
    pub(super) fn drag_edge(self, dragging: bool, x: f32) {
        if !dragging {
            return;
        }
        let Some((left, right)) = *self.viewport.peek() else {
            return;
        };
        if x < left + DRAG_EDGE {
            self.scroll_by(DRAG_STEP);
        } else if x > right - DRAG_EDGE {
            self.scroll_by(-DRAG_STEP);
        }
    }

    /// Move the strip `delta` pixels, positive being towards its start, and never past
    /// either end: the first chip does not leave the left edge, and the last does not
    /// leave the right.
    pub(super) fn scroll_by(self, delta: f32) {
        let Some((left, right)) = *self.viewport.peek() else {
            return;
        };
        let floor = -(*self.content.peek() - (right - left)).max(0.0);
        let want = (*self.offset.peek() + delta).clamp(floor, 0.0);
        let mut offset = self.offset;
        offset.set_if_modified(want);
    }

    /// The bar has taken a new shape, which is what the reveal wakes on.
    fn reshaped(self) {
        let mut shape = self.shape;
        shape.set(self.laid_out + 1);
    }
}

/// Bring the tab on screen into view when it changes, and when the bar takes a new shape
/// -- a tab opened, closed or moved -- which is what makes an opening reveal the tab it
/// opened, and what brings the tab being read back after a chip to its left has gone or a
/// chip has been dropped past it, either of which slides it out of sight.
///
/// **Not on every layout**, which would take the strip back off the reader the moment they
/// scrolled it to look at something else.
fn use_reveal(strip: State<Strip>, bar: Bar) {
    let active = strip.read().active();
    use_side_effect_with_deps(
        &(active, bar.laid_out),
        move |(active, _): &(Option<Tab>, u64)| {
            let Some(active) = *active else {
                return;
            };
            let Some((min, max)) = bar.places.peek().at(&active) else {
                return;
            };
            let Some((left, right)) = *bar.viewport.peek() else {
                return;
            };
            // The places are along the row; where the chip is in the window is that, slid.
            let slid = *bar.offset.peek();
            let (min, max) = (min + slid, max + slid);
            if min < left {
                bar.scroll_by(left - min);
            } else if max > right {
                bar.scroll_by(right - max);
            }
        },
    );
}

/// How near either end of the strip a drag has to be held for it to scroll, and how far it
/// goes per move of the pointer.
pub(super) const DRAG_EDGE: f32 = 24.0;
const DRAG_STEP: f32 = 12.0;

/// One place a dragged tab may be dropped: `position` in the bar, which is where the chip
/// there is now. The mark is drawn by the chip and the drop is answered here, so a chip
/// that is dragged away takes its own zone with it.
///
/// **Where the mark goes follows the pointer** (`on_pointer_move`) and not the zone it
/// entered: an enter fires once, on the crossing, and the crossing that matters here is
/// measured in the same breath as the render that starts the drag -- a zone entered before
/// the payload existed declines it, and nothing fires again until the pointer leaves and
/// comes back. A move, asked while a drag is under way, cannot miss it.
fn drop_zone(
    strip: State<Strip>,
    drag: State<Option<Tab>>,
    landing: State<Option<usize>>,
    position: usize,
    children: Element,
) -> Element {
    let mut strip = strip;
    let mut landing = landing;
    rect()
        .on_pointer_move(move |_| {
            if drag.peek().is_some() {
                landing.set_if_modified(Some(position));
            }
        })
        .child(DropZone::new(children, move |tab: Tab| {
            strip.write().move_to(tab, position);
        }))
        .into_element()
}

/// The copy of a chip that follows the cursor while it is being dragged: the chip itself,
/// on the ground a drop lands on, with nothing that answers a pointer. As `dock.rs` draws
/// a panel's, so the padding and the spacing cannot drift from the bar's.
fn dragged(tab: Tab, docs: &Docs) -> Element {
    rect()
        .interactive(false)
        .overflow(Overflow::Clip)
        .child(chip(
            tab_icon(tab, docs),
            &tab_title(tab, docs),
            Mark::Dragging,
            false,
            false,
            None,
            None,
        ))
        .into_element()
}

/// The content area: the bar, and under it the tab on screen -- a document's two panes, a
/// page, or the ground there is when nothing is open.
#[derive(PartialEq)]
pub(crate) struct ContentArea;

impl Component for ContentArea {
    fn render(&self) -> impl IntoElement {
        let strip = use_open().strip;
        // Only what is on screen, which is what this draws: the bar reads the rest of the
        // strip for itself, so a tab opening or moving does not rebuild the body.
        let active = strip.read().active();

        let body = match active {
            Some(Tab::Document(id)) => DocumentBody { id }.into_element(),
            Some(Tab::Page(page)) => page_body(page),
            None => placeholder("Nothing selected"),
        };

        rect()
            .expanded()
            .content(Content::Flex)
            .child(TabBar)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(body),
            )
    }
}

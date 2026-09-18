//! The window's main menu: the ways in and out of a project, the projects there have
//! been under one row of it, and the pages under them -- and the table saying what each
//! page is ([`PageRow`]).
//!
//! **The button is the toolbar's, not the tab bar's** (`src/ui.rs`): what it opens is
//! about the project and the pages, where the bar beside it is about the tabs.

use super::*;

/// One page's row of the UI's table: what it is drawn with, and the three rules the rest
/// of the app has about it.
///
/// A row per page rather than a match per column, which is [`Panel::row`]'s shape and its
/// reason: a page is one place and not five. [`Page::title`] and [`Page::stored`] stay in
/// `src/tabs.rs`, being framework-free -- and `stored` is a file format besides.
/// [`page_row`] matches on every page, so a page added to the enum has no row until one is
/// written for it.
struct PageRow {
    /// The Lucide glyph drawn before the title. A function rather than an element, so it
    /// is built in the scope that draws it: `glyph` asks for a colour, and asking is what
    /// subscribes a scope to the palette.
    icon: fn() -> Element,
    /// The window's own key for the page, where it has one: what the pages menu draws
    /// beside the row. The keys are the root's ([`root_key_down`]), so they work wherever
    /// the menu is opened from.
    key: Option<&'static str>,
    /// What the page's tab draws under the bar, built where it is drawn for the icon's
    /// reason.
    body: fn() -> Element,
    /// Whether the page is a reading of a project, so that with none open there is
    /// nothing for it to draw and the pages menu leaves the row out.
    needs_project: bool,
    /// Whether the pages menu offers the page only when Alt was held as it was opened.
    ///
    /// The Debug page is the ways to make the app misbehave on purpose, so it is not in a
    /// menu the reader opened to get to their project. Alt is what asks for it -- no
    /// rebuild, no variable, and nothing on screen for a reader who has not asked. Only
    /// the menu is gated: a session that names the page puts it back.
    hidden_unless_alt: bool,
    /// The place the page's second pane is filed under, for the chord that puts that pane
    /// away and brings it back ([`root_key_down`]); `None` for a page that has one pane.
    following: Option<Placing>,
}

/// This page's [`PageRow`].
fn page_row(page: Page) -> PageRow {
    match page {
        Page::Project => PageRow {
            icon: || glyph(("folder-open", lucide::folder_open())),
            key: None,
            body: || ProjectTab.into_element(),
            needs_project: true,
            hidden_unless_alt: false,
            following: None,
        },
        Page::Settings => PageRow {
            icon: || glyph(("settings", lucide::settings())),
            key: Some(shortcuts::key!(Settings)),
            body: || SettingsTab.into_element(),
            needs_project: false,
            hidden_unless_alt: false,
            following: None,
        },
        Page::Shortcuts => PageRow {
            icon: || glyph(("keyboard", lucide::keyboard())),
            key: Some(shortcuts::key!(Shortcuts)),
            body: || ShortcutsTab.into_element(),
            needs_project: false,
            hidden_unless_alt: false,
            following: None,
        },
        Page::Scratchpad => PageRow {
            icon: || glyph(("notebook-pen", lucide::notebook_pen())),
            key: None,
            body: || ScratchpadTab.into_element(),
            needs_project: false,
            hidden_unless_alt: false,
            following: Some(Placing::Pad),
        },
        Page::Debug => PageRow {
            icon: || glyph(("bug", lucide::bug())),
            key: None,
            body: || DebugTab.into_element(),
            needs_project: false,
            hidden_unless_alt: true,
            following: None,
        },
    }
}

/// The glyph a page's tab is drawn with ([`PageRow::icon`]).
pub(crate) fn page_icon(page: Page) -> Element {
    (page_row(page).icon)()
}

/// What a page's tab draws under the bar -- and what a window with no project draws in
/// place of its screen, there being no bar there to put a tab in ([`PageRow::body`]).
pub(crate) fn page_body(page: Page) -> Element {
    (page_row(page).body)()
}

/// The pane that follows this page's own, for the chord that puts it away
/// ([`PageRow::following`]).
pub(crate) fn page_following(page: Page) -> Option<Placing> {
    page_row(page).following
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
/// Its menu hangs from the button's left edge, which at the left of the bar is the
/// window's: the menu opens rightward into it.
#[derive(PartialEq)]
pub(crate) struct PagesButton;

impl Component for PagesButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        // Whether the menu that is up was opened with Alt held. Kept from the press rather
        // than read per render: the reader lets the key go to reach for the row, and a row
        // that vanished under their hand would be worse than not offering it at all.
        let mut asked = use_state(|| false);
        let alt = use_consume::<Alt>().0;
        let states = use_project_states();
        let recents = use_consume::<Recents>().0;
        // The pages this run offers, each with whether it is open already: one list of
        // pairs, so the mark cannot come to be about a different page than the row it is
        // drawn on. `hidden_unless_alt` is the Debug page's rule and is the row's
        // ([`PageRow`]).
        //
        // Built only while the menu is up, as the recents are read below: the marks are all
        // the strip is wanted for here, and nothing draws them until then. Read and not
        // peeked, so they follow a page opening or closing under an open menu; read every
        // render, it would subscribe the button to every tab opened, closed, moved or
        // raised.
        let pages: Vec<(Page, bool)> = match showing() {
            true => {
                let strip = states.open.strip.read();
                Page::ALL
                    .into_iter()
                    .filter(|page| !page_row(*page).hidden_unless_alt || asked())
                    .map(|page| (page, strip.contains(Tab::Page(page))))
                    .collect()
            }
            false => Vec::new(),
        };
        // Read only while the menu is up, for the strip's reason. The list itself was read
        // off disk as the project changed ([`Recents`]).
        let recents = match showing() {
            true => recents.read().clone(),
            false => Shared::default(),
        };

        let side = toggle_size();
        dropdown(
            (side, side),
            "Projects, Settings and the Scratchpad",
            glyph(("menu", lucide::menu())),
            hovering,
            showing,
            move |_| {
                // Read here and not in the render: this press is the moment the reader is
                // asking about, and a freya pointer event carries no modifiers of its own,
                // which is what `Alt` is kept for.
                let held = *alt.peek();
                let was = showing();
                asked.set(!was && held);
                showing.set(!was);
            },
            move || {
                main_menu(states, &recents, &pages, showing)
                    .on_close(move |_| showing.set(false))
                    .into_element()
            },
        )
    }
}

/// The menu [`PagesButton`] opens. Built per press, like the tab bar's (`strip.rs`),
/// which is what lets it leave out the items that would do nothing.
fn main_menu(
    states: ProjectStates,
    recents: &[Recent],
    // The pages this run has, each with whether it is open already, asked for by the
    // caller ([`PagesButton`]).
    pages: &[(Page, bool)],
    close: State<bool>,
) -> Menu {
    let open = states.proj.peek().file.clone();
    let store = states.store.peek().clone();
    let unsaved = open
        .as_deref()
        .zip(store.as_ref())
        .is_some_and(|(file, store)| project::unsaved(store, file));

    Menu::new()
        .child(menu_row(
            "Open a project...",
            Some(shortcuts::key!(OpenProject)),
            close,
            move || ask_for_a_project(states),
        ))
        .child(recents_submenu(states, recents, close))
        .child(menu_row(
            "Open a directory as a project...",
            None,
            close,
            move || ask_for_a_directory(states),
        ))
        .child(menu_row(
            "Open a file as a project...",
            None,
            close,
            move || ask_for_a_binary(states),
        ))
        .maybe_child((open.is_some() && !unsaved).then(|| {
            menu_row("Save as...", None, close, move || {
                ask_where_to_save(states, project::Put::Copy)
            })
        }))
        .maybe_child(
            open.is_some()
                .then(|| menu_row("Close project", None, close, move || close_project(states))),
        )
        .child(menu_rule())
        .children(
            pages
                .iter()
                .copied()
                // A page that is a reading of a project has nothing to draw with none open
                // ([`PageRow::needs_project`]).
                .filter(|(page, _)| !page_row(*page).needs_project || open.is_some())
                .map(|(page, open_already)| {
                    let opened = states.open;
                    let mut close = close;
                    let row = page_row(page);
                    MenuItem::new()
                        .selected(open_already)
                        .on_press(move |_| {
                            // The page door and not a write of the strip: a page opens beside
                            // the tab on screen, the way anything else the reader opens does,
                            // and one already showing is left alone (`show_page`).
                            show_page(opened, page);
                            close.set(false);
                        })
                        .child(
                            rect()
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .spacing(6.0)
                                .child((row.icon)())
                                .child(menu_label(page.title(), row.key)),
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
            menu_row(&recent.label, None, close, move || {
                switch_project(states, path.clone())
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

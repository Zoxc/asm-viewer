//! The app's own tab bar and what it draws: the chips, the × on one, the list of every
//! open tab, and the body under it all.
//!
//! **The bar is the app's and not the dock's.** It cannot be folded away, split, or
//! dragged out of; what is open is a [`Strip`] the app holds, and a chip is a plain
//! element that activates its own tab. The sidebar keeps freya's docking
//! (`src/ui/dock.rs`), where a panel is furniture the reader may arrange.

use super::*;

/// One tab's chip: the icon naming its kind, what it is called, the × that closes it, a
/// right-click menu, and the pane's own white when it is the one on screen.
///
/// **The press activates the tab**, this being the app's own bar: there is no wrapper
/// above it that does so, the way freya's docking has one. The × therefore has to stop
/// the press from reaching here, or a close would first switch to the tab it is closing.
///
/// **The tab on screen wears a rule along its top**, and the colour says where the keyboard
/// is: the gutter marks' own purple while it is inside the tab, and a dim grey while it is
/// anywhere else -- a sidebar list, a filter box. The mark is drawn on the chip that is
/// showing and on no other, so the bar says which tab is being read and whether it is
/// being typed into, without a second wash to tell from the first.
///
/// A temporal tab -- the preview a sidebar row opens in, which the next row reuses -- is
/// told from one that stays by its name being **italic**, and by nothing else: it is the
/// same tab in every other way, and the slant is the one cue that says "provisional"
/// without taking room from the name.
///
/// A stateless helper rather than a component, the hover state belonging to the caller, so
/// no hook runs here -- which is exactly why the × arrives as an element already built: it
/// carries a hover of its own and a hook has to run somewhere.
#[allow(clippy::too_many_arguments)]
fn chip(
    icon: Element,
    text: String,
    tooltip: String,
    active: bool,
    typing: bool,
    landing: bool,
    temporal: bool,
    mut hovering: State<bool>,
    close: Option<Element>,
    mut on_press: impl FnMut(Event<PressEventData>) + 'static,
    mut on_menu: impl FnMut(Event<PressEventData>) + 'static,
) -> impl IntoElement {
    // The active chip takes the pane's own background, so it reads as the top edge of the
    // pane below it. The hover stays lighter than that, or it would be more prominent
    // than the active tab.
    let background = if active {
        palette().pane_bg
    } else if hovering() {
        palette().toggle_hover_bg
    } else {
        Color::TRANSPARENT
    };

    // Where a tab being dragged would land: the leading edge of the chip under the
    // pointer, in the same purple the tab on screen is marked with.
    let mark = landing.then(|| {
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
    let marker = active.then(|| {
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
            })
    });

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
            .border(marker)
            .border(mark)
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
            // without it panics. A right-click is not a press, so this leaves the tab it
            // was opened on where it is rather than activating it first.
            .on_secondary_down(move |e: Event<PressEventData>| on_menu(e))
            .on_press(move |e: Event<PressEventData>| on_press(e))
            .child(icon)
            .child(
                label()
                    .text(elide(&text))
                    .max_lines(1)
                    .maybe(temporal, |name| name.font_slant(FontSlant::Italic)),
            )
            .maybe_child(close),
    )
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
        let asm_at = use_consume::<AsmAt>().0;
        let src_at = use_consume::<SrcAt>().0;
        let code_at = use_consume::<CodeAt>().0;
        let driven = use_consume::<Drives>().0;
        let marks_at = use_consume::<MarksAt>().0;
        let tab = self.tab;

        rect()
            .width(Size::px(close_target()))
            .height(Size::px(close_target()))
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
                    Tab::Document(id) => {
                        close_tab(open, asm_at, src_at, code_at, driven, marks_at, id)
                    }
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

        let button = row_tooltip(
            "Open tabs".to_owned(),
            rect()
                .width(Size::px(TAB_LIST_WIDTH))
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
                // needed: global listeners are snapshotted when the event is measured,
                // before any handler runs, so the menu this press opens is not in that
                // batch. A popup opened from a `*_down` handler is the case that does need
                // the swallow; copying it here ate the first press outside the menu.
                .on_press(move |_| {
                    let was = showing();
                    showing.set(!was);
                })
                .child(bar_icon(("chevron-down", lucide::chevron_down()))),
        );

        rect()
            .width(Size::px(TAB_LIST_WIDTH))
            .height(Size::px(list_row_height()))
            .child(button)
            .maybe_child(showing().then(|| {
                rect()
                    // Under the bar and aligned to its right-hand edge, so the list opens
                    // leftward into the window instead of off the side of it.
                    .position(Position::new_absolute().top(list_row_height()))
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
                            .spacing(6.0)
                            .child(icon)
                            // `max_lines(1)`, or a name longer than the menu is wide wraps
                            // and the row grows to hold it.
                            .child(label().text(title).max_lines(1)),
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
        Page::Project => bar_icon(("folder-open", lucide::folder_open())),
        Page::Settings => bar_icon(("settings", lucide::settings())),
        Page::Scratchpad => bar_icon(("notebook-pen", lucide::notebook_pen())),
    }
}

/// What a page's tab draws under the bar.
fn page_body(page: Page) -> Element {
    match page {
        Page::Project => ProjectTab.into_element(),
        Page::Settings => SettingsTab.into_element(),
        Page::Scratchpad => ScratchpadTab.into_element(),
    }
}

/// The menu at the **top left of the window** that opens Project, Settings and the
/// Scratchpad, and the whole of the way back to one that has been closed.
///
/// It lists all three and marks the ones that are open, rather than listing only the
/// closed ones: a menu whose rows come and go as tabs are closed is a menu a reader has to
/// read every time, where a list that is always the same three is one they learn. Picking
/// an open one shows it, which is what the reader meant by picking it.
///
/// The popup is positioned by hand, as [`TabListButton`]'s is: `ContextMenu` pins a menu
/// to the pointer and clamps to nothing.
#[derive(PartialEq)]
pub(crate) struct PagesButton;

impl Component for PagesButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        let open = use_open();
        // Read and not peeked: the marks are drawn from it, so the menu has to follow a
        // page opening or closing while it is up.
        let strip = open.strip.read();
        let is_open: Vec<bool> = Page::ALL
            .into_iter()
            .map(|page| strip.contains(Tab::Page(page)))
            .collect();
        drop(strip);

        let side = toggle_size();
        let button = row_tooltip(
            "Project, Settings and the Scratchpad".to_owned(),
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
                    let was = showing();
                    showing.set(!was);
                })
                .child(bar_icon(("menu", lucide::menu()))),
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
                        pages_menu(open, &is_open, showing).on_close(move |_| showing.set(false)),
                    )
                    .into_element()
            }))
            .into_element()
    }
}

/// The menu [`PagesButton`] opens: one row per page, the open ones marked. Built per
/// press, like the bar's own.
fn pages_menu(open: Open, is_open: &[bool], mut close: State<bool>) -> Menu {
    Page::ALL.into_iter().zip(is_open.iter().copied()).fold(
        Menu::new(),
        |menu, (page, open_already)| {
            menu.child(
                MenuItem::new()
                    .selected(open_already)
                    .on_press(move |_| {
                        // `show` and not `push`: a page opens beside the tab on screen,
                        // the way anything else the reader opens does, and one already
                        // open is only raised.
                        let mut strip = open.strip;
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
                    ),
            )
        },
    )
}

/// How wide [`TabListButton`] is.
pub(crate) const TAB_LIST_WIDTH: f32 = 26.0;

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
        let boxes = use_consume::<Keyboard>().0;
        let tab = self.tab;
        // Asked only of the chip that is showing, which is the only one that draws the
        // mark: asking is a subscription to the focus moving, and every chip taking one
        // would re-render the whole bar whenever it did.
        let typing = self.active && keyboard_in_tab(boxes);

        let Tab::Document(id) = tab else {
            let Tab::Page(page) = tab else {
                return rect().into_element();
            };
            return chip(
                page_icon(page),
                page.title().to_owned(),
                page.title().to_owned(),
                self.active,
                typing,
                self.landing,
                false,
                hovering,
                Some(TabClose { tab }.into_element()),
                move |_| raise_tab(open, tab),
                move |e: Event<PressEventData>| {
                    let others = open.strip.peek().tabs().iter().any(|other| *other != tab);
                    ContextMenu::open_from_event(&e, tab_menu(states, tab, others, None));
                },
            )
            .into_element();
        };

        // What the tab shows and whether it is the temporal one, out of one read: the
        // chip follows the trail's current entry, so navigating in place renames it.
        // Not reachable -- a tab and its trail are closed together -- but a render is no
        // place to panic.
        let (document, temporal) = {
            let docs = open.docs.read();
            (docs.get(id).cloned(), docs.temporal() == Some(id))
        };
        let Some(document) = document else {
            return rect().into_element();
        };

        let subject = document.clone();
        chip(
            entry_icon(&document),
            entry_text(&document),
            entry_tooltip(&document),
            self.active,
            typing,
            self.landing,
            temporal,
            hovering,
            Some(TabClose { tab }.into_element()),
            move |e: Event<PressEventData>| {
                raise_tab(open, tab);
                // A double press on a temporal tab's chip makes it a tab that stays.
                // freya counts the presses (500 ms, 5 px), and nothing else on the chip
                // asks it, so the count is this handler's own.
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
            },
            move |e: Event<PressEventData>| {
                // Read at the press rather than at the render: whether this tab has
                // company is not something the chip draws, so subscribing to the strip
                // for it would re-render every tab whenever any one of them opened. The
                // only tab open still gets its menu, the bookmark item being about the
                // tab itself; what it does without is the one row that would do nothing.
                let others = open.strip.peek().tabs().iter().any(|other| *other != tab);
                ContextMenu::open_from_event(
                    &e,
                    tab_menu(states, tab, others, Some(subject.clone())),
                );
            },
        )
        .into_element()
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
#[derive(PartialEq)]
pub(crate) struct TabBar;

impl Component for TabBar {
    fn render(&self) -> impl IntoElement {
        let strip = use_consume::<OpenTabs>().0;
        // Where a drop would land, and whether anything is being dragged at all: the
        // second is what makes the first mean something, a zone the pointer left last time
        // never having been told the drag ended (`DragZone` clears the payload itself).
        let landing = use_state(|| None);
        // Whether anything is being dragged is what makes the landing mean something: a
        // mark left where the pointer last was would otherwise outlive the drag, no zone
        // being told that a drag was abandoned (`DragZone` clears the payload itself).
        let drag = use_drag::<Tab>();
        let over = drag.read().is_some().then(|| landing()).flatten();
        let (tabs, active) = {
            let strip = strip.read();
            (strip.tabs().to_vec(), strip.active())
        };
        // The table read once, here, for the copies that follow the cursor: a hook may not
        // run in the loop that builds the chips.
        let docs = use_consume::<OpenDocs>().0;
        bar(strip, drag, landing, &docs.read(), &tabs, active, over)
    }
}

#[allow(clippy::too_many_arguments)]
fn bar(
    strip: State<Strip>,
    drag: State<Option<Tab>>,
    landing: State<Option<usize>>,
    docs: &Docs,
    tabs: &[Tab],
    active: Option<Tab>,
    over: Option<usize>,
) -> Element {
    let chips: Vec<Element> = tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            let tab = *tab;
            // Keyed by the tab, so a tab that moves takes its hover, its tooltip and its
            // open menu with it instead of leaving them on whatever took its place.
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
                DragZone::new(tab, header.into_element())
                    .drag_element(dragged(tab, docs))
                    .into_element(),
            )
        })
        .collect();

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
                // The chips sit in a box of their own, whose width is `Inner`. A child of
                // the scroll view's own `fill` content box is measured against the space
                // *left* in it, so chips past the edge would get no width and draw as a
                // bare ×. Inside an `Inner` box each is measured from its own content and
                // the overflow is what there is to scroll. (A tab's name is elided by
                // character count for the same reason: a `maximum_width` anywhere in one
                // makes it shrinkable again.)
                .child(
                    rect()
                        .horizontal()
                        .height(Size::fill())
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
        .into_element()
}

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

/// The copy of a chip that follows the cursor while it is being dragged.
fn dragged(tab: Tab, docs: &Docs) -> Element {
    rect()
        .interactive(false)
        .height(Size::px(list_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .spacing(6.0)
        .background(palette().selected_bg)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(tab_icon(tab, docs))
        .child(label().text(elide(&tab_title(tab, docs))).max_lines(1))
        .into_element()
}

/// The content area: the bar, and under it the tab on screen -- a document's two panes, a
/// page, or the ground there is when nothing is open.
#[derive(PartialEq)]
pub(crate) struct ContentArea;

impl Component for ContentArea {
    fn render(&self) -> impl IntoElement {
        let strip = use_consume::<OpenTabs>().0;
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

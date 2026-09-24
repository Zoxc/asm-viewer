//! The app's own tab bar: the chips, the × on one, the list of every open tab, and the
//! body under it all. How far the bar is slid and where its chips have been measured to
//! is `scroll.rs`.
//!
//! **The bar is the app's and not the dock's.** It cannot be folded away, split, or
//! dragged out of; what is open is a [`Strip`] the app holds, and a chip is a plain
//! element that activates its own tab. The sidebar keeps freya's docking
//! (`src/ui/dock.rs`), where a panel is furniture the reader may arrange.

use super::*;
use crate::counter;

mod scroll;
pub(crate) use scroll::*;

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
/// caller's `use_state`, handed over to be read and written, and [`DraggedChip`] passes `None`
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

    // A chip is cut by the count and never by the room it has: `chars::elide` is what shortened
    // it, and the bar scrolls rather than squeezing a chip (`metrics.rs`).
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .height(Size::px(tab_row_height()))
        // Air to the left of the icon and next to none to the right: what sits at that
        // end is the ×, which is a target of its own and carries its own.
        .padding(Gaps::new(0.0, 2.0, 0.0, 8.0))
        .spacing(GLYPH_GAP)
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
                .text(chars::elide(text, CHIP_NAME_CHARS))
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
            .corner_radius(BAR_BUTTON_RADIUS)
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
                close(open, places, tab);
            })
            // An icon and not the `×` character: a character is centred by its line box,
            // and where the mark falls inside that is the font's, so it sat low by a
            // different amount in every font.
            .child(glyph_sized(
                ("x", lucide::x()),
                close_icon(),
                if hovering() {
                    palette().text_fg
                } else {
                    palette().address_fg
                },
            ))
    }
}

/// The control that opens a list of every open tab, pinned at the **right** of the bar so
/// it never scrolls away with the tabs it is there to reach. It lists all of them and not
/// only the hidden ones: which are off-screen would mean measuring the bar against its
/// viewport, and a list whose length changed as the bar was dragged would be worse to use.
///
/// Its menu ends up aligned to the button's right-hand edge, so the list opens leftward
/// into the window instead of off the side of it.
#[derive(PartialEq)]
pub(crate) struct TabListButton;

impl Component for TabListButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        let open = use_open();
        let strip = open.strip;

        // A memo over the one thing the button draws from the strip, whether there are any
        // tabs, and not a read of it: the strip is written by every tab opened, closed,
        // moved or raised.
        let any = use_memo(move || !strip.read().tabs().is_empty());
        // The menu is unmounted with the bar's last tab, and nothing that closes it runs:
        // that close was a key answered at the root. Left set, the flag would open the
        // menu again with the next tab.
        use_side_effect(move || {
            if !any() {
                showing.set_if_modified(false);
            }
        });
        if !any() {
            return rect().into_element();
        }

        dropdown(
            (TAB_LIST_WIDTH, tab_row_height()),
            "Open tabs",
            glyph(("chevron-down", lucide::chevron_down())),
            hovering,
            showing,
            move |_| {
                let was = showing();
                showing.set(!was);
            },
            // Called only while the menu is up, which is when the strip is read: read and
            // not peeked, so the rows follow a tab opening or closing under an open menu.
            move || {
                let (tabs, active) = {
                    let strip = strip.read();
                    (strip.tabs().to_vec(), strip.active())
                };
                tabs_menu(open, &tabs, active, showing)
                    .on_close(move |_| showing.set(false))
                    // Keyed by row count so a list that grows while the menu is open
                    // remounts it: `MenuContainer` measures itself once and keeps that
                    // offset, so a menu that widens afterwards hangs off the side of the
                    // window.
                    .key(tabs.len())
                    .into_element()
            },
        )
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
            .map(|tab| {
                let (icon, names) = tab_drawn(*tab, shown(*tab, &docs));
                (*tab, chars::elide(&names.text, CHIP_NAME_CHARS), icon)
            })
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
                            .spacing(GLYPH_GAP)
                            .child(icon)
                            // `max_lines(1)`, or a name longer than the menu is wide wraps
                            // and the row grows to hold it.
                            .child(label().text(title).max_lines(1).width(Size::flex(1.0)))
                            .child(TabClose { tab }),
                    ),
            )
        })
}

/// What a tab is called and drawn as: a page's own title and glyph, or the [`Names`] and
/// kind of the document it shows, which is `None` for a page or a closed tab. Not elided
/// here -- the chip decides how much of a name it has room for.
fn tab_drawn(tab: Tab, document: Option<&Document>) -> (Element, Names) {
    match (tab, document) {
        (Tab::Page(page), _) => (page_icon(page), Names::page(page.title())),
        (Tab::Document(_), Some(document)) => (entry_icon(document), Names::of(document)),
        (Tab::Document(_), None) => (rect().into_element(), Names::default()),
    }
}

/// The document `tab` shows now, if it is a document's tab still open.
fn shown(tab: Tab, docs: &Docs) -> Option<&Document> {
    match tab {
        Tab::Document(id) => docs.get(id),
        Tab::Page(_) => None,
    }
}

/// How wide [`TabListButton`] is.
pub(crate) const TAB_LIST_WIDTH: f32 = 26.0;

/// How wide a row of the list it opens is, at the least. A floor and not a width: a name
/// longer than this still has the room it needs, the menu growing to its longest row.
const TAB_LIST_ROW_WIDTH: f32 = 220.0;

/// One tab's chip, with the hover state a chip cannot hold for itself.
#[derive(Clone, PartialEq)]
pub(crate) struct TabHeader {
    pub(crate) tab: Tab,
    /// Whether this is the tab on screen.
    pub(crate) active: bool,
    /// Whether a tab being dragged would land here.
    pub(crate) landing: bool,
    pub(crate) key: DiffKey,
}

keyed!(TabHeader);

counter!(
    /// Test-only: how many chips this thread has drawn. The bar holds one per open tab,
    /// and a chip drawn again draws what it drew before.
    pub(crate) fn chips_drawn() = CHIPS_DRAWN
);

impl Component for TabHeader {
    fn render(&self) -> impl IntoElement {
        #[cfg(test)]
        CHIPS_DRAWN.set(CHIPS_DRAWN.get() + 1);

        let hovering = use_state(|| false);
        // Consumed here, in the render, for the menu: its handler may not run a hook.
        let states = use_project_states();
        let open = states.open;
        let keyboard = use_consume::<Keyboard>();
        let tab = self.tab;
        // Copied out for the menu: whether this chip is the tab on screen is what says
        // which of the window's keys the menu may claim.
        let active = self.active;
        // Asked only of the chip that is showing, which is the only one that draws the
        // mark: asking is a subscription to the focus moving, and every chip taking one
        // would re-render the whole bar whenever it did.
        let mark = match active {
            true => Mark::Active {
                typing: keyboard_in_tab(keyboard),
            },
            false => Mark::Plain,
        };

        // The document the chip shows and whether it is the temporal one: the chip follows
        // the trail's current entry, so navigating in place renames it. **A memo over this
        // chip's own entry and not a read of the table**, which is written by every push
        // onto any tab's trail: read here, every chip in the bar was drawn again for each.
        // Safe to capture `tab` because the bar keys a chip by it. A page's chip reads
        // nothing, its name being its own.
        let docs = open.docs;
        let entry = use_memo(move || match tab {
            Tab::Document(id) => {
                let docs = docs.read();
                (docs.get(id).cloned(), docs.temporal() == Some(id))
            }
            Tab::Page(_) => (None, false),
        });
        let (document, temporal) = entry.read().clone();
        // What it draws and what hovering it says out of one name: on a symbol's tab the
        // first is the short spelling of the second.
        let (icon, names) = tab_drawn(tab, document.as_ref());
        let Names { text, tooltip, .. } = names;

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
                let others = open.strip.peek().has_others(tab);
                let subject = match tab {
                    Tab::Document(id) => open.docs.peek().get(id).cloned(),
                    Tab::Page(_) => None,
                };
                ContextMenu::open_from_event(&e, tab_menu(states, tab, others, active, subject));
            })
            .on_press(move |e: Event<PressEventData>| {
                raise_tab(open, tab);
                // The reader is going to read in it, so the keyboard goes there too: back
                // into the pane it was last in there, and otherwise the pane the tab is
                // driven from (`use_keyboard_asked`).
                return_keyboard(keyboard);
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
                write_if(open.docs, |docs| docs.promote(id));
            }),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
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
/// **The strip scrolls itself, without a `ScrollView`**, and every measurement that takes
/// is [`Bar`]'s (`scroll.rs`). What is here is where those measurements are made: the
/// handler on each chip, the one on the strip, the one on the row, the wheel, and the
/// pointer held at an edge while a tab is dragged.
#[derive(PartialEq)]
pub(crate) struct TabBar;

impl Component for TabBar {
    fn render(&self) -> impl IntoElement {
        let open = use_open();
        let strip = open.strip;
        // Where a drop would land, and whether anything is being dragged at all: the
        // second is what makes the first mean something, a drag that ends off the bar
        // telling no zone (`DragZone` clears the payload itself).
        let landing = use_state(|| None);
        let drag = use_drag::<Tab>();
        // A call is a read of the state, which `&*landing` would hide.
        #[allow(clippy::redundant_closure)]
        let over = drag.read().is_some().then(|| landing()).flatten();

        let bar = use_bar();
        use_reveal(strip, bar);
        // Not hit while a sweep is under way: a sweep dragged up out of a pane crosses the
        // chips, and freya's tooltip arms on the hover alone. Here and not on each chip, so
        // a sweep starting and ending draws no chip again.
        let sweeping = use_sweeping();

        let (tabs, active) = {
            let strip = strip.read();
            (strip.tabs().to_vec(), strip.active())
        };
        // A chip that has gone is never measured again, so its place would sit here for
        // the rest of the session. It is dropped once its tab is no longer open, which
        // costs the reveal nothing: the tab on screen is one of these.
        use_side_effect_with_deps(&tabs, move |tabs: &Vec<Tab>| {
            write_if(bar.places, |places| {
                places.forgetting(|tab| tabs.contains(tab))
            });
        });

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
                                .drag_element(DraggedChip { tab }.into_element()),
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
            .interactive(!sweeping)
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
        // Off the zones a release moves nothing, so nothing is marked. Only this zone's own
        // mark is taken off, in case the next zone's move came first.
        .on_pointer_out(move |_| {
            if *landing.peek() == Some(position) {
                landing.set(None);
            }
        })
        .child(DropZone::new(children, move |tab: Tab| {
            landing.set_if_modified(None);
            strip.write().move_to(tab, position);
        }))
        .into_element()
}

/// The copy of a chip that follows the cursor while it is being dragged: the chip itself,
/// on the ground a drop lands on, with nothing that answers a pointer. As `dock.rs` draws
/// a panel's, so the padding and the spacing cannot drift from the bar's.
///
/// **A component, so the table is read in its own render**: `DragZone` mounts it only
/// while a drag is under way, so the bar reads nothing of [`Docs`] and a wheel tick over
/// it names no tab.
#[derive(Clone, Copy, PartialEq)]
struct DraggedChip {
    tab: Tab,
}

impl Component for DraggedChip {
    fn render(&self) -> impl IntoElement {
        let docs = use_open().docs;
        let (icon, names) = tab_drawn(self.tab, shown(self.tab, &docs.read()));
        rect()
            .interactive(false)
            .overflow(Overflow::Clip)
            .child(chip(
                icon,
                &names.text,
                Mark::Dragging,
                false,
                false,
                None,
                None,
            ))
    }
}

/// The content area: the bar, and under it the tab on screen -- a document's two panes, a
/// page, or the ground there is when nothing is open.
#[derive(PartialEq)]
pub(crate) struct ContentArea;

impl Component for ContentArea {
    fn render(&self) -> impl IntoElement {
        let strip = use_open().strip;
        // A memo over the tab on screen, which is all this draws, and not a read of the
        // strip: the bar reads the rest for itself, so a tab opening or moving beside the
        // one on screen does not draw this again.
        let active = use_memo(move || strip.read().active());

        let body = match active() {
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

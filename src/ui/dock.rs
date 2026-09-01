//! The dock and the tab strip over it: what a tab is, the headers and bars freya asks for,
//! and the docking model the two areas are.
//!
//! One file because since 6c they are one mechanism -- open documents *are* dock tabs, so
//! the strip is the document panel's own tab bar and [`chip`] is what a [`tab_header`]
//! wraps.
//!
//! A [`Tab`] is two-kinded because freya's `DockingModel::TabId` is `Copy + PartialEq +
//! Hash` and a [`Document`] is none of the three. One panel is designated for documents
//! and [`DockArea::on_drop`] refuses one anywhere else, where a view may go anywhere;
//! [`DockArea::tidy`] is freya's `close_empty_panels` written out rather than called,
//! because that sweep exempts nothing. And nothing in a header activates its tab -- freya's
//! own wrapper does that, which is why the × must stop propagation.

use super::*;

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
pub(crate) struct DocumentMenuButton;

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
pub(crate) const DOCUMENT_MENU_WIDTH: f32 = 26.0;

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
pub(crate) enum Tab {
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
pub(crate) enum View {
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

/// Panel ids are only ever looked up inside the area that handed them out, so
/// each area numbers its own panels from zero.
pub(crate) type PanelId = u32;

/// The content area's panel that documents live in. The first one `DockArea::row` builds,
/// so it is where the reader's eye already is; see [`DockArea::documents`].
pub(crate) const DOCUMENT_PANEL: PanelId = 0;

/// One docking area: the tree of splits and tabbed panels filling one of the two
/// resizable panes. The nine tabs are shared between the two areas, so a drop
/// here has to take the tab out of `other` -- which is safe to write from
/// `on_drop` only because the two areas are separate `State`s, and freya's
/// docking holds a mutable borrow of just the one being dropped into.
///
/// Plain data apart from that handle, so the layout can be serialized later.
pub(crate) struct DockArea {
    pub(crate) tree: DockNode<Tab, PanelId>,
    next_panel_id: PanelId,
    pub(crate) other: Option<State<DockArea>>,
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
    pub(crate) docs: Option<State<Docs>>,
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
    pub(crate) fn with_documents(mut self, panel_id: PanelId) -> Self {
        self.documents = Some(panel_id);
        self
    }

    /// The groups stacked top to bottom, which is what the sidebar looks like.
    pub(crate) fn column(groups: Vec<Vec<Tab>>) -> Self {
        Self::split(Direction::Vertical, groups)
    }

    /// The groups side by side, which is what the content area looks like.
    pub(crate) fn row(groups: Vec<Vec<Tab>>) -> Self {
        Self::split(Direction::Horizontal, groups)
    }

    fn take_panel_id(&mut self) -> PanelId {
        let panel_id = self.next_panel_id;
        self.next_panel_id += 1;
        panel_id
    }

    /// The panel documents live in, for the area that has one.
    pub(crate) fn document_panel(&self) -> Option<&DockPanel<Tab, PanelId>> {
        self.tree.panel(&self.documents?)
    }

    /// The same panel, to write into. Every change to what is open goes through one of
    /// the three functions that hold the invariants, never through here directly.
    pub(crate) fn document_panel_mut(&mut self) -> Option<&mut DockPanel<Tab, PanelId>> {
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
    pub(crate) fn show_document(&mut self, tab: Tab) {
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
    pub(crate) fn tidy(&mut self) {
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
    pub(crate) fn accepts(&self, tab: Tab, target: &DropTarget<PanelId>) -> bool {
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

pub(crate) fn docking_area(area: State<DockArea>, docs: State<Docs>) -> impl IntoElement {
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

//! The dock and the tab strip over it: what a tab is, the headers and bars freya asks for,
//! and the docking model the two areas are. Open documents *are* dock tabs, so the strip
//! is the document panel's own tab bar and [`chip`] is what a [`tab_header`] wraps.
//!
//! A [`Tab`] is two-kinded because freya's `DockingModel::TabId` is `Copy + PartialEq +
//! Hash` and a [`Document`] is none of the three. One panel is designated for documents
//! and [`DockArea::on_drop`] refuses one anywhere else, where a view may go anywhere;
//! [`DockArea::tidy`] is freya's `close_empty_panels` written out rather than called,
//! because that sweep exempts nothing.

use super::*;

/// One document's tab header: the icon naming its kind, what it is called, an × that
/// closes it, and the pane's own white when it is the one on screen.
///
/// **Nothing here activates the tab.** freya wraps a tab header in a `DropZone` around a
/// `rect().on_press(set_active)` around a `DragZone`, so pressing this makes it the
/// panel's active tab -- which is also why the × must `stop_propagation`: without it a
/// close would first switch to the tab it is closing.
///
/// A stateless helper rather than a component, the hover state belonging to the caller, so
/// no hook runs here.
fn chip(
    icon: Element,
    text: String,
    tooltip: String,
    active: bool,
    mut hovering: State<bool>,
    mut on_close: impl FnMut(Event<PressEventData>) + 'static,
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
                    // Without this the press bubbles into freya's own wrapper and the
                    // close first switches to the tab it is closing.
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

/// The control that opens a list of every tab in the document panel, pinned at the
/// **right** of the bar so it never scrolls away with the tabs it is there to reach. It
/// lists all of them and not only the hidden ones: which are off-screen would mean
/// measuring the bar against its viewport, and a list whose length changed as the bar was
/// dragged would be worse to use.
///
/// The popup is positioned here rather than through `ContextMenu`, which pins a menu's
/// top-left corner to the pointer and clamps to nothing -- opened from a button at the
/// right-hand edge it would draw off the side of the window.
#[derive(PartialEq)]
pub(crate) struct DocumentMenuButton;

impl Component for DocumentMenuButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut showing = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;

        // Every tab in the panel and its active one, read together so the menu is built
        // from one look at the dock. Views are listed beside the documents: they scroll
        // off the same edge.
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
                // needed: global listeners are snapshotted when the event is measured,
                // before any handler runs, so the menu this press opens is not in that
                // batch. A popup opened from a `*_down` handler is the case that does need
                // the swallow; copying it here ate the first press outside the menu.
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

/// The menu [`DocumentMenuButton`] opens: one row per tab in the document panel, in the
/// bar's own order, with the one on screen marked. Built per press, like `close_menu`.
fn tabs_menu(
    open: Open,
    history: State<History>,
    tabs: &[Tab],
    active: Option<Tab>,
    mut close: State<bool>,
) -> Menu {
    // Names and glyphs resolved in one pass, so the read guard on the table is gone before
    // any row's handler can run and write to it.
    let rows: Vec<(Tab, String, Element)> = {
        let docs = open.docs.read();
        tabs.iter()
            .map(|tab| (*tab, elide(&tab.title(&docs)), tab.icon(&docs)))
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
                        match tab {
                            // A document already open is a place the reader has, so going
                            // to it is a move and records nothing.
                            Tab::Document(id) => {
                                let document = open.docs.peek().get(id).cloned();
                                activate(open, history, document, Visit::Moved);
                            }
                            // A view is not a document and never goes through `activate`:
                            // making it the tab on top is the whole of showing it.
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
                            // and the row grows to hold it.
                            .child(label().text(title).max_lines(1)),
                    ),
            )
        })
}

/// The document panel's tab bar: a horizontally scrolling row of chips, since these are
/// opened by the dozen, with [`DocumentMenuButton`] pinned beside it. The scrollbar is off
/// -- it would eat a third of a one-row bar, and the wheel and a drag still move it.
fn chip_strip(mut chips: Vec<Element>, tab_count: usize) -> Element {
    // freya appends one more child than there are tabs: a `rect().expanded()` inside a
    // drop zone that drops past the last tab. `expanded()` is meaningless inside a
    // horizontal scroll view, so it is given a width of its own instead of collapsing to
    // nothing.
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

/// How wide the "drop past the last tab" target is in the document panel's bar.
const DROP_PAST_LAST_TAB: f32 = 24.0;

/// How wide [`DocumentMenuButton`] is.
pub(crate) const DOCUMENT_MENU_WIDTH: f32 = 26.0;

/// One open document's tab header. A component rather than a plain function because it has
/// a hover state of its own, which a view header has no need of.
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

        // Not reachable -- a tab and its table entry are closed together -- but a render
        // is no place to panic.
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
/// A document is carried as the [`DocId`] [`Docs`] knows it by, because freya's
/// `DockingModel::TabId` is `Copy + PartialEq + Hash + 'static` and a [`Document`] is none
/// of the three.
///
/// The two arms are asymmetric, which [`DockArea::on_drop`] enforces: **a document may
/// only ever be in the designated document panel, while a view may be anywhere, that panel
/// included.** One visible document is what lets `Analysis`, `Marked`, `Focused` and
/// `Pinned` each hold one answer for the window.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Tab {
    View(View),
    Document(DocId),
}

impl Tab {
    /// The label shown in the tab bar. Not elided here -- the header decides how much of a
    /// name it has room for.
    fn title(self, docs: &Docs) -> String {
        match self {
            Tab::View(view) => view.title().to_owned(),
            Tab::Document(id) => docs.get(id).map(entry_text).unwrap_or_default(),
        }
    }

    /// The Lucide glyph drawn before the title.
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
/// active document drives, so each renders itself off the state it is about and subscribes
/// to it on its own -- which keeps a change of document from re-rendering the whole tree.
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
    /// The name is passed beside the bytes because `ImageSource` keys the raster cache on
    /// a hash of whatever it is given, and hashing a short name beats hashing an SVG.
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
            // Given rather than inherited: `SvgViewer` rasterizes only once it knows a
            // colour, and with none set it waits for an `on_styled` -- a frame late, and a
            // frame of nothing in a 26px bar.
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

/// Panel ids are only ever looked up inside the area that handed them out, so each area
/// numbers its own panels from zero.
pub(crate) type PanelId = u32;

/// The content area's panel that documents live in; see [`DockArea::documents`].
pub(crate) const DOCUMENT_PANEL: PanelId = 0;

/// One docking area: the tree of splits and tabbed panels filling one of the two resizable
/// panes. Tabs are shared between the two areas, so a drop here has to take the tab out of
/// `other` -- safe to write from `on_drop` only because the two areas are separate
/// `State`s and freya's docking borrows just the one being dropped into.
pub(crate) struct DockArea {
    pub(crate) tree: DockNode<Tab, PanelId>,
    next_panel_id: PanelId,
    pub(crate) other: Option<State<DockArea>>,
    /// The panel documents live in, for the area that has one -- `Some` for the content
    /// area, `None` for the sidebar.
    ///
    /// A document opened from the symbol list has to land *somewhere*, and freya's
    /// `DockingModel` has no notion of "the panel documents belong to", so this names one.
    /// The rest follows: it is exempt from [`DockArea::tidy`] so closing the last document
    /// cannot fold the content area away, it is the only panel [`DockArea::on_drop`] will
    /// let a document into, and it draws the app's own empty ground.
    documents: Option<PanelId>,
    /// The side table, for the one question [`DockArea::on_drop`] has to ask about a
    /// document: whether it is still open. `Option` because it is wired up after the state
    /// exists, the way `other` is.
    pub(crate) docs: Option<State<Docs>>,
}

impl DockArea {
    /// An area split into one tabbed panel per group, all at equal sizes.
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
    /// Documents are appended *after* the views, so Project, Settings and the Scratchpad
    /// keep the left of the bar: a restored session's dozen tabs would otherwise push all
    /// three off the right-hand edge, and they are reachable from nowhere else.
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

    /// Put `tab` into `panel_id` at `position`, or at the end when `None`, and take it out
    /// of every other panel of this area.
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
    /// stays on screen as a drop target.
    ///
    /// This is freya's `close_empty_panels` written out rather than called, because that
    /// sweep retains every non-empty child with no way to spare one and would fold the
    /// document panel away the moment the last document was closed. It has to *replace*
    /// the call rather than follow it: a panel re-created after the sweep would come back
    /// somewhere else in the tree. freya's two behaviours that are kept: a split left with
    /// one child collapses into it, and a lone panel at the root is never removed.
    pub(crate) fn tidy(&mut self) {
        Self::prune(&mut self.tree, self.documents);
        if self.tree.is_empty() && !matches!(self.tree, DockNode::Panel(_)) {
            self.tree = DockNode::Panel(DockPanel::new(self.next_panel_id, Vec::new()));
            self.next_panel_id += 1;
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

    /// Whether `tab` may land where a drop is aiming it: **a view may go anywhere, the
    /// document panel included; a document may only ever be in the document panel.** A
    /// refused drop answers `false`, which leaves the drag where it started.
    pub(crate) fn accepts(&self, tab: Tab, target: &DropTarget<PanelId>) -> bool {
        let Tab::Document(id) = tab else {
            return true;
        };
        // A drag begun before its document was closed carries an id that stands for
        // nothing. Ids are never reused, so this can only be a dead one and never some
        // other document that took its number.
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
            // A drag carries only the tab, so the source area is not known -- but there
            // are only two, and evicting a tab the other never had is a no-op.
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

/// A view's tab header, and the copy of any tab that follows the cursor while it is
/// dragged.
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

/// The bar a panel's tab headers sit in: a plain row for a view panel, whose seven views
/// always fit, and [`chip_strip`] for the document panel, which is opened into by the
/// dozen.
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

/// One document, drawn: its assembly beside the source it was compiled from, in a
/// `ResizableContainer` rather than a nested `DockingArea`.
///
/// Only the *active* tab's content is mounted, so this whole subtree -- both panes, both
/// scroll controllers -- is built afresh on every switch of document, which is what
/// `use_kept_position` is for.
#[derive(Clone, PartialEq)]
struct DocumentBody {
    id: DocId,
}

impl Component for DocumentBody {
    fn render(&self) -> impl IntoElement {
        let docs = use_consume::<OpenDocs>().0;
        let mut ratio = use_consume::<SplitRatio>().0;
        let splits = use_consume::<Splits>().0;

        // Where the reader last left the handle, written back as they drag it. Reading the
        // context is what subscribes this to the drag; `set_if_modified` keeps the mount's
        // own registration from waking anything.
        use_side_effect(move || {
            let live = splits.read().panels.first().map(|panel| panel.size);
            if let Some(live) = live {
                ratio.set_if_modified(live);
            }
        });

        // `peek` and not `read`: `initial_size` is consulted once, in the panel's own
        // `use_hook` at mount, so subscribing here would be a subscription to nothing --
        // and a loop with the effect above.
        let assembly = ratio.peek().clamp(1.0, 99.0);

        // Not reachable -- the tab and the table entry are closed together -- but a render
        // is no place to panic.
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

/// What a panel draws: its active tab, or -- with no tabs at all -- an empty ground. The
/// ground differs by panel, hence the whole context: a view panel is empty because the
/// reader dragged everything out of it, the document panel because nothing is open.
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

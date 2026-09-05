//! The sidebar's dock: what a panel is, the headers and bars freya asks for, and the
//! docking model the sidebar is.
//!
//! A [`Panel`] is the whole of what can be a tab here, which is why the model can be so
//! small: a document lives in the app's own strip (`src/ui/strip.rs`) and there is no way
//! to name one in this area at all.

use super::*;

/// One of the sidebar's panels. A panel is a **persistent pane** rather than a slot the
/// selection drives, so each renders itself off the state it is about and subscribes to it
/// on its own -- which keeps a change of document from re-rendering the whole tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Panel {
    Objects,
    Files,
    Search,
    Symbols,
    History,
    Bookmarks,
    Locations,
}

impl Panel {
    /// Every panel, in the order the default sidebar stacks them.
    const ALL: [Panel; 7] = [
        Panel::Objects,
        Panel::Files,
        Panel::Search,
        Panel::Symbols,
        Panel::History,
        Panel::Bookmarks,
        Panel::Locations,
    ];

    /// What a session names it. A name of its own rather than the title, for [`Page`]'s
    /// reason: a title is what the reader sees and may be reworded, where a stored name
    /// changing would empty every saved sidebar.
    fn stored(self) -> &'static str {
        match self {
            Panel::Objects => "objects",
            Panel::Files => "files",
            Panel::Search => "search",
            Panel::Symbols => "symbols",
            Panel::History => "history",
            Panel::Bookmarks => "bookmarks",
            Panel::Locations => "locations",
        }
    }

    /// The panel a session named, or `None` for a name this build does not have.
    fn from_stored(stored: &str) -> Option<Panel> {
        Panel::ALL
            .into_iter()
            .find(|panel| panel.stored() == stored)
    }

    fn title(self) -> &'static str {
        match self {
            Panel::Objects => "Objects",
            Panel::Files => "Files",
            Panel::Search => "Search",
            Panel::Symbols => "Symbols",
            Panel::History => "History",
            Panel::Bookmarks => "Bookmarks",
            Panel::Locations => "Locations",
        }
    }

    /// The Lucide glyph drawn before the title, at the interface font's own size and in
    /// the palette's `icon_fg`.
    ///
    /// The name is passed beside the bytes because `ImageSource` keys the raster cache on
    /// a hash of whatever it is given, and hashing a short name beats hashing an SVG.
    fn icon(self) -> Element {
        match self {
            Panel::Objects => bar_icon(("package", lucide::package())),
            Panel::Files => bar_icon(("folder-tree", lucide::folder_tree())),
            Panel::Search => bar_icon(("search", lucide::search())),
            Panel::Symbols => bar_icon(("square-function", lucide::square_function())),
            Panel::History => bar_icon(("history", lucide::history())),
            Panel::Bookmarks => bar_icon(("bookmark", lucide::bookmark())),
            Panel::Locations => bar_icon(("map-pin", lucide::map_pin())),
        }
    }

    fn body(self) -> Element {
        match self {
            Panel::Objects => ObjectsPanel.into_element(),
            Panel::Files => FilesPanel.into_element(),
            Panel::Search => SearchPanel.into_element(),
            Panel::Symbols => SymbolsPanel.into_element(),
            Panel::History => HistoryPanel.into_element(),
            Panel::Bookmarks => BookmarksPanel.into_element(),
            Panel::Locations => LocationsPanel.into_element(),
        }
    }
}

/// A tab bar's glyph, drawn at [`icon_size`] in the palette's `icon_fg`.
///
/// The colour is **given rather than inherited**: `SvgViewer` rasterizes only once it
/// knows one, and with none set it waits for an `on_styled` -- a frame late, and a frame
/// of nothing in a 26px bar.
pub(crate) fn bar_icon(icon: impl Into<ImageSource>) -> Element {
    let side = icon_size();
    SvgViewer::new(icon)
        .width(Size::px(side))
        .height(Size::px(side))
        .color(palette().icon_fg)
        .show_loader(false)
        .into_element()
}

/// Bring `panel` to the front of whichever group holds it: what a panel that answers a
/// question asked somewhere else does before it answers.
pub(crate) fn raise_panel(mut dock: State<DockArea>, panel: Panel) {
    dock.write().show_panel(panel);
}

/// Panel ids are only ever looked up inside the area that handed them out, so the sidebar
/// numbers its groups from zero.
pub(crate) type PanelId = u32;

/// The sidebar's docking area: the tree of splits and tabbed groups filling it. A panel
/// may be dragged into another group or split off into a group of its own, and it never
/// leaves the sidebar.
pub(crate) struct DockArea {
    pub(crate) tree: DockNode<Panel, PanelId>,
    next_panel_id: PanelId,
}

impl DockArea {
    /// The groups stacked top to bottom, which is what the sidebar looks like.
    pub(crate) fn column(groups: Vec<Vec<Panel>>) -> Self {
        Self {
            next_panel_id: groups.len() as PanelId,
            tree: DockNode::Split {
                direction: Direction::Vertical,
                children: groups
                    .into_iter()
                    .enumerate()
                    .map(|(panel_id, panels)| {
                        DockNode::Panel(DockPanel::new(panel_id as PanelId, panels))
                    })
                    .collect(),
            },
        }
    }

    /// The area a session described, or `None` where it described none this build can use.
    ///
    /// **Every panel this build has, exactly once.** What comes out of the file is a
    /// reader's arrangement and not a promise: a name this build does not have is dropped,
    /// one it does have twice is kept where it first appeared, and one the file never
    /// mentions -- a panel added since it was written -- is put in the first group, so the
    /// sidebar cannot come back with a panel missing and no way to reach it. An empty group
    /// is dropped, and a file describing nothing usable answers `None` and leaves the
    /// default alone.
    pub(crate) fn restored(saved: &SavedDock) -> Option<DockArea> {
        let mut seen = HashSet::new();
        let mut next = 0;
        let tree = Self::node_of(saved, &mut seen, &mut next)?;
        let mut area = DockArea {
            tree,
            next_panel_id: next,
        };
        for panel in Panel::ALL.into_iter().filter(|panel| !seen.contains(panel)) {
            area.add_to_first(panel);
        }
        Some(area)
    }

    /// One node of that walk. `seen` is what has been placed already, so a panel named
    /// twice lands once; `next` hands out the group ids, which have to be the area's own.
    fn node_of(
        saved: &SavedDock,
        seen: &mut HashSet<Panel>,
        next: &mut PanelId,
    ) -> Option<DockNode<Panel, PanelId>> {
        match saved {
            SavedDock::Split {
                horizontal,
                children,
            } => {
                let children: Vec<_> = children
                    .iter()
                    .filter_map(|child| Self::node_of(child, seen, next))
                    .collect();
                match children.len() {
                    0 => None,
                    // A split of one is the child itself: a group left alone by its
                    // siblings being dropped is not a split any more.
                    1 => children.into_iter().next(),
                    _ => Some(DockNode::Split {
                        direction: match horizontal {
                            true => Direction::Horizontal,
                            false => Direction::Vertical,
                        },
                        children,
                    }),
                }
            }
            SavedDock::Group { panels, showing } => {
                let panels: Vec<Panel> = panels
                    .iter()
                    .filter_map(|name| Panel::from_stored(name))
                    .filter(|panel| seen.insert(*panel))
                    .collect();
                if panels.is_empty() {
                    return None;
                }
                let id = *next;
                *next += 1;
                let mut group = DockPanel::new(id, panels);
                // The one that was showing, where it is still in this group; the first
                // otherwise, which is what `DockPanel::new` already chose.
                if let Some(panel) = showing.as_deref().and_then(Panel::from_stored) {
                    if group.tabs.contains(&panel) {
                        group.active_tab_id = Some(panel);
                    }
                }
                Some(DockNode::Panel(group))
            }
        }
    }

    /// Put `panel` in the first group there is: where a panel this build has and the file
    /// did not name ends up.
    fn add_to_first(&mut self, panel: Panel) {
        let mut node = &mut self.tree;
        loop {
            match node {
                DockNode::Split { children, .. } => match children.first_mut() {
                    Some(first) => node = first,
                    None => return,
                },
                DockNode::Panel(group) => {
                    group.tabs.push(panel);
                    return;
                }
            }
        }
    }

    /// How this area would be written down: the shape, each group's panels in their order,
    /// and which of them is showing.
    pub(crate) fn saved(&self) -> SavedDock {
        Self::saved_node(&self.tree)
    }

    fn saved_node(node: &DockNode<Panel, PanelId>) -> SavedDock {
        match node {
            DockNode::Split {
                direction,
                children,
            } => SavedDock::Split {
                horizontal: *direction == Direction::Horizontal,
                children: children.iter().map(Self::saved_node).collect(),
            },
            DockNode::Panel(group) => SavedDock::Group {
                panels: group
                    .tabs
                    .iter()
                    .map(|panel| panel.stored().to_owned())
                    .collect(),
                showing: group.active_tab_id.map(|panel| panel.stored().to_owned()),
            },
        }
    }

    /// Bring `panel` to the top of whichever group holds it, answering whether one does.
    pub(crate) fn show_panel(&mut self, panel: Panel) -> bool {
        let Some((panel_id, _)) = self.tree.find_tab(&panel) else {
            return false;
        };
        self.set_active(panel_id, panel)
    }

    /// Whether `panel` is the one on top in whichever group holds it.
    fn is_active(&self, panel: Panel) -> bool {
        let Some((panel_id, _)) = self.tree.find_tab(&panel) else {
            return false;
        };
        self.tree
            .panel(&panel_id)
            .and_then(|group| group.active_tab_id)
            == Some(panel)
    }

    /// Put `panel` into `panel_id` at `position`, or at the end when `None`, and take it
    /// out of every other group.
    fn place(&mut self, panel_id: PanelId, panel: Panel, position: Option<usize>) -> bool {
        let Some(group) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        match position {
            Some(position) => group.insert_tab(panel, position),
            None => group.append_tab(panel),
        }
        self.tree.remove_tab_except(&panel, Some(&panel_id));
        true
    }

    /// Fold away the groups a move emptied. An area that loses its last panel keeps one
    /// empty group rather than going to `None`, so the sidebar stays on screen as a drop
    /// target.
    ///
    /// This is freya's `close_empty_panels` written out rather than called, because that
    /// sweep leaves a tree with no panel at all where this leaves one. freya's two
    /// behaviours that are kept: a split left with one child collapses into it, and a lone
    /// panel at the root is never removed.
    pub(crate) fn tidy(&mut self) {
        Self::prune(&mut self.tree);
        if self.tree.is_empty() && !matches!(self.tree, DockNode::Panel(_)) {
            self.tree = DockNode::Panel(DockPanel::new(self.next_panel_id, Vec::new()));
            self.next_panel_id += 1;
        }
    }

    /// [`DockArea::tidy`]'s walk: drop every empty child, then collapse a split down to
    /// its only survivor.
    fn prune(node: &mut DockNode<Panel, PanelId>) {
        let DockNode::Split { children, .. } = node else {
            return;
        };
        children.iter_mut().for_each(Self::prune);
        children.retain(|child| !child.is_empty());
        if children.len() == 1 {
            *node = children.remove(0);
        }
    }
}

impl DockingModel for DockArea {
    type TabId = Panel;
    type PanelId = PanelId;
    type DropValue = Panel;

    fn root(&self) -> Option<&DockNode<Panel, PanelId>> {
        Some(&self.tree)
    }

    fn on_drop(&mut self, panel: Panel, target: DropTarget<PanelId>) -> bool {
        let dropped = match target {
            DropTarget::Tab { panel_id, position } => self.place(panel_id, panel, Some(position)),
            DropTarget::Center(panel_id) => self.place(panel_id, panel, None),
            DropTarget::Split { panel_id, side } => {
                let new_panel_id = self.next_panel_id;
                let new_panel = DockPanel::new(new_panel_id, vec![panel]);
                if self.tree.split_panel(&panel_id, side, &new_panel) {
                    self.next_panel_id += 1;
                    self.tree.remove_tab_except(&panel, Some(&new_panel_id));
                    true
                } else {
                    false
                }
            }
        };

        if dropped {
            self.tidy();
        }

        dropped
    }

    fn set_active(&mut self, panel_id: PanelId, panel: Panel) -> bool {
        let Some(group) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        if !group.tabs.contains(&panel) {
            return false;
        }
        group.active_tab_id = Some(panel);
        true
    }
}

/// A panel's tab header, and the copy of one that follows the cursor while it is dragged.
fn panel_label(panel: Panel, background: Color) -> impl IntoElement {
    rect()
        .height(Size::px(list_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .spacing(6.0)
        .background(background)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(panel.icon())
        .child(label().text(elide(panel.title())).max_lines(1))
}

fn panel_header(ctx: TabContext<Panel>, area: State<DockArea>) -> Element {
    let background = if ctx.is_drop_target {
        palette().selected_bg
    } else if area.read().is_active(ctx.tab_id) {
        palette().pane_bg
    } else {
        Color::TRANSPARENT
    };
    panel_label(ctx.tab_id, background).into_element()
}

/// The copy of the panel that follows the cursor while it is being dragged.
fn panel_drag(panel: Panel) -> Element {
    rect()
        .interactive(false)
        .border(right_hairline())
        .child(panel_label(panel, palette().selected_bg))
        .into_element()
}

/// The bar a group's headers sit in: a plain row, the seven panels always fitting.
fn panel_bar(ctx: TabBarContext<PanelId>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(list_row_height()))
        .horizontal()
        .background(palette().header_bg)
        .border(bottom_hairline())
        .children(ctx.tab_children)
        .into_element()
}

/// What a group draws: the panel on top, or the empty ground of a group the reader has
/// dragged everything out of.
fn panel_content(ctx: ContentContext<Panel, PanelId>) -> Element {
    match ctx.tab_id {
        Some(panel) => panel.body(),
        None => placeholder("Drag a panel here"),
    }
}

pub(crate) fn docking_area(area: State<DockArea>) -> impl IntoElement {
    DockingArea::new(
        area,
        |ctx: ContentContext<Panel, PanelId>| panel_content(ctx),
        move |ctx: TabContext<Panel>| panel_header(ctx, area),
        |panel: Panel| panel_drag(panel),
        |ctx: TabBarContext<PanelId>| panel_bar(ctx),
    )
    .preview_element(
        rect()
            .interactive(false)
            .expanded()
            .background(palette().drop_preview_bg),
    )
}

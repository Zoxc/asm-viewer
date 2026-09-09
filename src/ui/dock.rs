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

/// One panel's row of the table: the name a session stores it under, the title, and the
/// two things drawn for it. A row per panel rather than a match per column, so a panel is
/// one place and not four. [`Panel::row`] matches on every panel, so a panel added to the
/// enum has no row until one is written for it.
struct Row {
    /// What a session names it. A name of its own rather than the title, for [`Page`]'s
    /// reason: a title is what the reader sees and may be reworded, where a stored name
    /// changing would empty every saved sidebar.
    stored: &'static str,
    /// What the reader sees on the panel's tab header.
    title: &'static str,
    /// The Lucide glyph drawn before that title ([`glyph`]). A function rather than an
    /// element, so it is built in the scope that draws it: `glyph` asks for a colour, and
    /// asking is what subscribes a scope to the palette.
    icon: fn() -> Element,
    /// The panel itself, built where it is drawn for the same reason.
    body: fn() -> Element,
}

impl Panel {
    /// Every panel there is, grouped as the default sidebar stacks them: the groups top to
    /// bottom, each group's panels in their tab order. The one list a new panel goes in --
    /// a fresh sidebar is built from it ([`DockArea::default`]) and a saved one is filled
    /// out against it ([`DockArea::restored`]) -- so neither can come back without it.
    const GROUPS: [&'static [Panel]; 2] = [
        &[
            Panel::Objects,
            Panel::Files,
            Panel::Search,
            Panel::Locations,
        ],
        &[Panel::Symbols, Panel::History, Panel::Bookmarks],
    ];

    /// Every panel, in the order those groups name them.
    pub(super) fn all() -> impl Iterator<Item = Panel> {
        Panel::GROUPS.into_iter().flatten().copied()
    }

    /// This panel's [`Row`].
    fn row(self) -> Row {
        match self {
            Panel::Objects => Row {
                stored: "objects",
                title: "Objects",
                icon: || glyph(("package", lucide::package())),
                body: || ObjectsPanel.into_element(),
            },
            Panel::Files => Row {
                stored: "files",
                title: "Files",
                icon: || glyph(("folder-tree", lucide::folder_tree())),
                body: || FilesPanel.into_element(),
            },
            Panel::Search => Row {
                stored: "search",
                title: "Search",
                icon: || glyph(("search", lucide::search())),
                body: || SearchPanel.into_element(),
            },
            Panel::Symbols => Row {
                stored: "symbols",
                title: "Symbols",
                icon: || glyph(("square-function", lucide::square_function())),
                body: || SymbolsPanel.into_element(),
            },
            Panel::History => Row {
                stored: "history",
                title: "History",
                icon: || glyph(("history", lucide::history())),
                body: || HistoryPanel.into_element(),
            },
            Panel::Bookmarks => Row {
                stored: "bookmarks",
                title: "Bookmarks",
                icon: || glyph(("bookmark", lucide::bookmark())),
                body: || BookmarksPanel.into_element(),
            },
            Panel::Locations => Row {
                stored: "locations",
                title: "Locations",
                icon: || glyph(("map-pin", lucide::map_pin())),
                body: || LocationsPanel.into_element(),
            },
        }
    }

    /// What a session names it ([`Row::stored`]).
    fn stored(self) -> &'static str {
        self.row().stored
    }

    /// The panel a session named, or `None` for a name this build does not have.
    fn from_stored(stored: &str) -> Option<Panel> {
        Panel::all().find(|panel| panel.stored() == stored)
    }

    fn title(self) -> &'static str {
        self.row().title
    }

    /// The glyph drawn before the title ([`Row::icon`]).
    fn icon(self) -> Element {
        (self.row().icon)()
    }

    /// Whether the panel draws a filter box over its list, which is **where a chord that
    /// reaches it puts the keyboard**: in the box where there is one, so what is typed
    /// next narrows the list, and on the rows where there is not -- the Files tree, which
    /// has nothing to filter by. Either way the arrows, Enter and Escape are answered,
    /// the box handing on the keys the list under it owns (`ui/filter_bar.rs`).
    ///
    /// Not a column of [`Row`]: what there is to say is that one panel is the exception,
    /// which seven booleans would say worse.
    pub(super) fn filters(self) -> bool {
        !matches!(self, Panel::Files)
    }

    /// What the panel draws ([`Row::body`]).
    fn body(self) -> Element {
        (self.row().body)()
    }
}

/// Bring `panel` to the front of whichever group holds it: what a panel that answers a
/// question asked somewhere else does before it answers.
pub(crate) fn raise_panel(mut dock: State<DockArea>, panel: Panel) {
    dock.write().show_panel(panel);
}

/// Bring `panel` to the front **and put the keyboard in it**: what each of the four
/// panel chords does, from wherever the keyboard is. Both halves, and not a choice
/// between them: a panel behind another in its group is not there to be typed in until it
/// is raised, and one already on top is raised by a write that changes nothing.
///
/// The focus is asked for and not taken here, because only the panel on top in a group is
/// mounted: the box to put the keyboard in does not exist until the raise above has been
/// drawn. [`use_keyboard_asked`] spends the ask once the panel has registered one
/// (`ui/keyboard.rs`).
pub(crate) fn reach_panel(dock: State<DockArea>, keyboard: State<Keys>, panel: Panel) {
    raise_panel(dock, panel);
    ask_for_panel(keyboard, panel);
}

/// Panel ids are only ever looked up inside the area that handed them out, so the sidebar
/// numbers its groups from zero.
pub(crate) type PanelId = u32;

/// The sidebar's docking area: the tree of splits and tabbed groups filling it. A panel
/// may be dragged into another group or split off into a group of its own, and it never
/// leaves the sidebar.
pub(crate) struct DockArea {
    tree: DockNode<Panel, PanelId>,
    next_panel_id: PanelId,
}

impl Default for DockArea {
    /// The sidebar a reader with no saved arrangement of their own gets: [`Panel::GROUPS`]
    /// as it stands.
    fn default() -> Self {
        Self::column(Panel::GROUPS.iter().map(|group| group.to_vec()).collect())
    }
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
        for panel in Panel::all().filter(|panel| !seen.contains(panel)) {
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

    /// Each group's panels, in the order the groups are laid out: what
    /// [`DockArea::column`] is given, out of an area a reader has since rearranged.
    ///
    /// Test-only: nothing the app draws wants the groups as a list, and this is how a
    /// test asks about the shape without reaching into the tree.
    #[cfg(test)]
    pub(crate) fn groups(&self) -> Vec<Vec<Panel>> {
        fn walk(node: &DockNode<Panel, PanelId>, into: &mut Vec<Vec<Panel>>) {
            match node {
                DockNode::Panel(group) => into.push(group.tabs.clone()),
                DockNode::Split { children, .. } => {
                    children.iter().for_each(|child| walk(child, into))
                }
            }
        }
        let mut groups = Vec::new();
        walk(&self.tree, &mut groups);
        groups
    }

    /// Whether `panel` is the one on top in whichever group holds it: what a tab header
    /// draws itself by, and what a test asks rather than walking the tree.
    pub(crate) fn is_active(&self, panel: Panel) -> bool {
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
    fn tidy(&mut self) {
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

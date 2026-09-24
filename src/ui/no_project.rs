//! The window with no project open: what is drawn under the top bar either way, and this
//! screen in place of the tabs, the sidebar and the panes.
//!
//! The three ways in are the menu's own ([`ask_for_a_project`] and the two beside it), so
//! the screen offers nothing the bar does not; it is where they are found by a reader who
//! has just arrived and has no reason to open a menu yet.

use super::*;

/// Everything under the top bar: the sidebar and the panes, or -- with no project -- the
/// screen below, or a page shown in its place.
///
/// A component of its own and not a `match` inside `app()` for two reasons. `Proj` is
/// written by every keystroke in the Project view's boxes, and reading it at the root would
/// re-render the whole window for each; here the read is a **memo** over the one thing this
/// branch is about, whether there is a project at all. And `app()` is mounted by no test,
/// so a branch inside it is a branch nothing can ask about.
#[derive(PartialEq)]
pub(crate) struct WindowBody;

impl Component for WindowBody {
    fn render(&self) -> impl IntoElement {
        let file = use_consume::<ProjFile>().0;
        let sidebar_dock = use_consume::<SidebarDock>().0;
        let split = use_consume::<SidebarSplit>().0;
        let strip = use_open().strip;
        let stay = use_project_states().stay;
        let opened = use_memo(move || file.read().is_some());
        // A memo over the one thing this branch asks of the strip, not a read of it: the
        // bar is written by every tab opened, moved or closed, and this has to re-render
        // for one of those only when it takes the last tab away or brings the first back.
        let any_tabs = use_memo(move || !strip.read().tabs().is_empty());
        // Above the early return, as a hook has to be: the width is followed here rather
        // than beside the container, which is only built when a project is open.
        split.use_follow();

        if !opened() {
            // Settings and the Scratchpad are nobody's project's, so they open with none --
            // as ordinary tabs, which brings the bar back for them and takes it away again
            // when the last one is closed. There is no sidebar either way: that *is* a
            // project's.
            return match any_tabs() {
                true => ContentArea.into_element(),
                false => NoProject.into_element(),
            };
        }

        // The sidebar beside the one proportional panel, which therefore takes whatever is
        // left. Docking cannot express a literal width, which is why this split is a
        // `ResizableContainer` and not another `DockingArea`.
        //
        // Under a rect keyed by the stay, so every project mounts the panels afresh at the
        // widths it restored: a panel reads its size only when it mounts, and a switch from
        // one project to another never renders with none between.
        let container = ResizableContainer::new()
            .direction(Direction::Horizontal)
            .controller(split.context)
            .panel(
                ResizablePanel::new(split.panel_size())
                    .min_size(120.0)
                    .child(docking_area(sidebar_dock)),
            )
            .panel(
                ResizablePanel::new(split.rest())
                    .min_size(10.0)
                    .child(ContentArea),
            );
        rect()
            .expanded()
            .key(*stay.read())
            .child(container)
            .into_element()
    }
}

/// The screen: the ways into a project, and the projects there have been.
#[derive(PartialEq)]
pub(crate) struct NoProject;

impl Component for NoProject {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let recents = use_consume::<Recents>().0.read().clone();

        let rows: Vec<Element> = recents
            .iter()
            .map(|recent| {
                RecentRow {
                    recent: recent.clone(),
                    key: DiffKey::None,
                }
                .key(recent.path.to_string_lossy().into_owned())
                .into()
            })
            .collect();

        page(
            None,
            page_column()
                .child(
                    section("Open a project", None).child(
                        rect()
                            .horizontal()
                            .spacing(SECTION_GAP)
                            .child(
                                Button::new()
                                    .on_press(move |_| ask_for_a_project(states))
                                    .child("Project file..."),
                            )
                            .child(
                                Button::new()
                                    .on_press(move |_| ask_for_a_directory(states))
                                    .child("Directory..."),
                            )
                            .child(
                                Button::new()
                                    .on_press(move |_| ask_for_a_binary(states))
                                    .child("Binary..."),
                            ),
                    ),
                )
                .child(section("Recent projects", None).child(rows_or(rows, "None yet"))),
        )
        .into_element()
    }
}

//! The window with no project open: the top bar, and this in place of the tabs, the
//! sidebar and the panes.
//!
//! The three ways in are the menu's own ([`ask_for_a_project`] and the two beside it), so
//! the screen offers nothing the bar does not; it is where they are found by a reader who
//! has just arrived and has no reason to open a menu yet.

use super::*;

/// How wide the window asking about a delete is. The scratchpad's own is the same number
/// and stays its own: the two windows ask different questions and neither should move
/// because the other did.
const ASKING_WIDTH: f32 = 520.0;

/// What one of the project's buttons in the bar does.
///
/// An enum and not a handler, for [`TabClose`]'s reason: a `Component` is `PartialEq` and a
/// closure is not, so a button holding one would re-render on every render of the bar.
#[derive(Clone, Copy, PartialEq)]
enum Doing {
    /// Let the project go, leaving the app with none. It is left where it is.
    Close,
    /// Put it in a file, which is what an unsaved project has instead of a close.
    Save,
    /// Take it away. Asks first.
    Delete,
}

/// One of them, drawn the way the bar's other controls are.
#[derive(Clone, Copy, PartialEq)]
struct ChipButton {
    doing: Doing,
}

impl Component for ChipButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let states = use_project_states();
        let mut deleting = use_consume::<Deleting>().0;
        let doing = self.doing;

        let (tooltip, icon) = match doing {
            Doing::Close => ("Close the project", ("x", lucide::x())),
            Doing::Save => ("Save the project to a file", ("save", lucide::save())),
            Doing::Delete => ("Delete the project", ("trash-2", lucide::trash_2())),
        };

        let side = toggle_size();
        let glyph = icon_size();
        row_tooltip(
            tooltip.to_owned(),
            rect()
                .width(Size::px(side))
                .height(Size::px(side))
                .center()
                .corner_radius(4.0)
                .background(match hovering() {
                    true => palette().toggle_hover_bg,
                    false => Color::TRANSPARENT,
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| match doing {
                    Doing::Close => close_project(states),
                    Doing::Save => ask_where_to_save(states, project::Put::Move),
                    Doing::Delete => {
                        let name = states.proj.peek().file.as_deref().map(project::label);
                        deleting.set(name);
                    }
                })
                .child(
                    SvgViewer::new(icon)
                        .width(Size::px(glyph))
                        .height(Size::px(glyph))
                        .color(palette().icon_fg)
                        .show_loader(false),
                ),
        )
    }
}

/// The open project, in the top bar beside the menu: what it is called, and what can be
/// done with it.
///
/// **A close button, or Save and Delete.** A project the reader gave a place needs only to
/// be let go of; one the app is keeping has nowhere to be let go *to*, so the two things it
/// can have done to it are named outright rather than hidden behind a x that would mean one
/// of them. Delete asks first ([`DeleteProjectPopup`]); Save does not, having nothing to
/// undo, and it is a move rather than a copy -- there is no second project afterwards.
///
/// Pressing the name shows the Project view, which is where the rest of it is. Hovering it
/// says where the project is kept, which is the one thing the name leaves out.
#[derive(PartialEq)]
pub(crate) struct ProjectChip;

impl Component for ProjectChip {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let proj = use_consume::<Proj>().0;
        let mut strip = use_open().strip;
        // Read and not peeked: the bar follows the project being saved, closed or opened.
        let file = proj.read().file.clone();
        let Some(file) = file else {
            return rect().into_element();
        };
        let unsaved = project::unsaved(&file);

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(2.0)
            .child(row_tooltip(
                file.to_string_lossy().into_owned(),
                rect()
                    .height(Size::px(toggle_size()))
                    .center()
                    .padding(Gaps::new_symmetric(0.0, 6.0))
                    .corner_radius(4.0)
                    .background(match hovering() {
                        true => palette().toggle_hover_bg,
                        false => Color::TRANSPARENT,
                    })
                    .on_pointer_over(move |_| hovering.set_if_modified(true))
                    .on_pointer_out(move |_| hovering.set_if_modified(false))
                    .on_press(move |_| {
                        strip.write().show(Tab::Page(Page::Project));
                    })
                    .child(label().text(elide(&project::label(&file))).max_lines(1)),
            ))
            .maybe(!unsaved, |chip| {
                chip.child(ChipButton {
                    doing: Doing::Close,
                })
            })
            .maybe(unsaved, |chip| {
                chip.child(ChipButton { doing: Doing::Save })
                    .child(ChipButton {
                        doing: Doing::Delete,
                    })
            })
            .into_element()
    }
}

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
        let proj = use_consume::<Proj>().0;
        let sidebar_dock = use_consume::<SidebarDock>().0;
        let width = use_consume::<SidebarWidth>().0;
        let splits = use_consume::<SidebarSplits>().0;
        let strip = use_open().strip;
        let opened = use_memo(move || proj.read().file.is_some());
        // A memo over the one thing this branch asks of the strip, not a read of it: the
        // bar is written by every tab opened, moved or closed, and this has to re-render
        // for one of those only when it takes the last tab away or brings the first back.
        let any_tabs = use_memo(move || !strip.read().tabs().is_empty());
        // Registered here rather than beside the container, so that the drag is followed
        // for exactly as long as there is a sidebar to drag.
        use_sidebar_width(splits, width);

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
        // `peek` and not `read`, as the document's own split does: `initial_size` is
        // consulted once in the panel's `use_hook`, so a read here would subscribe to
        // nothing and loop with the effect that follows the drag (`use_sidebar_width`).
        ResizableContainer::new()
            .direction(Direction::Horizontal)
            .controller(splits)
            .panel(
                ResizablePanel::new(PanelSize::px(width.peek().clamp(120.0, 900.0)))
                    .min_size(120.0)
                    .child(docking_area(sidebar_dock)),
            )
            .panel(
                ResizablePanel::new(PanelSize::percent(100.0))
                    .min_size(10.0)
                    .child(ContentArea),
            )
            .into_element()
    }
}

/// A project the reader asked for that would not open, until they have been told.
///
/// A project file is never moved aside -- it may be their own file, beside their code -- so
/// one that will not parse is left exactly where it is and nothing is written over it. That
/// makes telling them the whole of what happens, and this is what carries it as far as the
/// window below.
#[derive(Clone, Copy)]
pub(crate) struct Unopened(pub(crate) State<Option<PathBuf>>);

/// The window that says so. Drawn as nothing at all until there is something to say, the
/// way `RescuedPopup` is.
#[derive(PartialEq)]
pub(crate) struct UnopenedPopup {
    pub(crate) naming: Option<PathBuf>,
}

impl Component for UnopenedPopup {
    fn render(&self) -> impl IntoElement {
        let mut unopened = use_consume::<Unopened>().0;

        Popup::new()
            .width(Size::px(ASKING_WIDTH))
            .on_close_request(move |_| unopened.set(None))
            .map(self.naming.clone(), |popup, path| {
                popup
                    .child(
                        rect()
                            .padding(8.0)
                            .spacing(8.0)
                            .font(&fonts().ui)
                            .color(palette().text_fg)
                            .child(label().text("That project would not open".to_owned()))
                            .child(
                                label()
                                    .text(
                                        "It is not there, or it is not a project file the \
                                         app can read. It has been left exactly as it is."
                                            .to_owned(),
                                    )
                                    .color(palette().address_fg),
                            )
                            // A path is as long as it is, and one that is cut off is one
                            // the reader cannot go and look at.
                            .child(
                                paragraph()
                                    .assembly_font()
                                    .color(palette().address_fg)
                                    .span(path.to_string_lossy().into_owned()),
                            ),
                    )
                    .child(
                        PopupButtons::new().child(
                            Button::new()
                                .filled()
                                .on_press(move |_| unopened.set(None))
                                .child("Close"),
                        ),
                    )
            })
    }
}

/// Whether the reader is being asked to confirm deleting the open project, and what it is
/// called while they answer.
///
/// The label and not the path: it is what the question names, and reading it once when the
/// question is asked is what keeps the popup from having to be told the project again.
#[derive(Clone, Copy)]
pub(crate) struct Deleting(pub(crate) State<Option<String>>);

/// The window the app asks before deleting a project.
///
/// Nothing is deleted until it is answered: the control in the bar sets [`Deleting`] and
/// this is what acts. `on_close_request` is the "no" for free -- Escape and a press outside
/// -- and an empty `Popup` draws nothing at all, so with nothing to ask this lays out as
/// nothing.
#[derive(PartialEq)]
pub(crate) struct DeleteProjectPopup {
    pub(crate) asking: Option<String>,
}

impl Component for DeleteProjectPopup {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let mut deleting = use_consume::<Deleting>().0;

        Popup::new()
            .width(Size::px(ASKING_WIDTH))
            .on_close_request(move |_| deleting.set(None))
            .map(self.asking.clone(), |popup, name| {
                popup
                    .child(
                        rect()
                            .padding(8.0)
                            .spacing(8.0)
                            .font(&fonts().ui)
                            .color(palette().text_fg)
                            .child(label().text(format!("Delete {name}?")))
                            .child(
                                label()
                                    .text(
                                        "It is saved nowhere else: its binaries and its \
                                         bookmarks go with it."
                                            .to_owned(),
                                    )
                                    .color(palette().address_fg),
                            ),
                    )
                    .child(
                        PopupButtons::new()
                            .child(
                                Button::new()
                                    .on_press(move |_| deleting.set(None))
                                    .child("Cancel"),
                            )
                            .child(
                                Button::new()
                                    .filled()
                                    .on_press(move |_| {
                                        deleting.set(None);
                                        delete_project(states);
                                    })
                                    .child("Delete"),
                            ),
                    )
            })
    }
}

/// Follow the sidebar's handle: what the reader drags it to becomes [`SidebarWidth`].
///
/// `splits.read()` is what subscribes this to the drag, and `set_if_modified` keeps the
/// panel's own registration at mount from waking anything. The document's split has the
/// same pair for the same reasons (`src/ui/split.rs`).
fn use_sidebar_width(splits: State<ResizableContext>, mut width: State<f32>) {
    use_side_effect(move || {
        let live = splits.read().panels.first().map(|panel| panel.size);
        if let Some(live) = live {
            width.set_if_modified(live);
        }
    });
}

/// The screen: the ways into a project, and the projects there have been.
#[derive(PartialEq)]
pub(crate) struct NoProject;

impl Component for NoProject {
    fn render(&self) -> impl IntoElement {
        let states = use_project_states();
        let rescued = use_consume::<Rescued>().0;
        let unopened = use_consume::<Unopened>().0;
        // Read on mount and never again: nothing on this screen changes the list, and the
        // one thing that would -- opening a project -- takes the screen away with it.
        let recents = use_hook(project::recent_projects);

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

        rect()
            .expanded()
            .background(palette().pane_bg)
            .child(
                ScrollView::new().child(
                    rect()
                        .width(Size::fill())
                        .padding(Gaps::new_symmetric(8.0, 12.0))
                        .spacing(6.0)
                        .child(section_heading("Open a project", None))
                        .child(
                            rect()
                                .horizontal()
                                .spacing(6.0)
                                .child(
                                    Button::new()
                                        .on_press(move |_| {
                                            ask_for_a_project(states, rescued, unopened)
                                        })
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
                        )
                        .child(section_heading("Recent projects", None))
                        .child(match rows.is_empty() {
                            true => info_line("None yet".to_owned()).into_element(),
                            false => rect().width(Size::fill()).children(rows).into_element(),
                        }),
                ),
            )
            .into_element()
    }
}

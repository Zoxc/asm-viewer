//! One document drawn: the two panes, which side leads, and the control that puts the
//! following one away.
//!
//! The pane the tab is driven from leads and the other follows, which is a fact about the
//! *document* and not about the panels it is drawn in; the two are the same components
//! either way, so nothing but their order changes.

use super::*;

/// Whether the pane a place is not driven from is up: what the reader last said about
/// this place, and where they have said nothing, what it opens with.
///
/// `document` is what a tab is showing, and [`None`] for the Scratchpad's pane, which is
/// no tab: its editor is the side it is driven from and the listing beside it is up until
/// the reader says otherwise.
///
/// **Only a source-driven tab opens with one pane**, and only on a file in no compiled
/// language: a `Cargo.toml` or a `.json` is read and never disassembled, so the pane
/// beside it would be an empty half of the window with a handle to drag it wider. The
/// question is `source::compiled`, off the same extension list the grammars come from,
/// and an extension it does not know is answered no -- an assembly side is offered for
/// the languages the app can say become machine code, and a file it cannot place opens
/// as source until the reader asks for one.
pub(crate) fn following(
    of: Placing,
    document: Option<&Document>,
    said: &HashMap<Placing, bool>,
) -> bool {
    match (said.get(&of), document) {
        (Some(&said), _) => said,
        (None, Some(document)) => {
            document.driven_from() != Pane::Source || source::compiled(document.file())
        }
        (None, None) => true,
    }
}

/// What a split's number means, which is the one thing the three differ in: the sidebar
/// is a literal width and the other two a share of the container.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Unit {
    Percent,
    Pixels,
}

/// One resizable split: the number the app holds for it, the `ResizableContext` its panels
/// register into, what that number means, and the bounds it is drawn within.
///
/// **Two states and not one, because the container will not remember the size.** A
/// `ResizablePanel` registers at its `initial_size` in a `use_hook` and takes its entry
/// out again in a `use_drop`, so a container that is unmounted -- the tab off screen, the
/// window's body rebuilt as a project arrives -- comes back at the initial sizes under new
/// panel ids. What survives is the number here, fed in as `initial_size` ([`panel_size`])
/// and written back as the handle is dragged ([`follow`]).
///
/// One value and not a pair of contexts per split: the three are the document's
/// ([`DocumentSplit`]), the sidebar's ([`SidebarSplit`]) and the Scratchpad's
/// ([`PadSplit`]), each made by one [`Split::create`] call, and each drawn by the same
/// three lines rather than by three copies of the clamp.
///
/// [`panel_size`]: Split::panel_size
/// [`follow`]: Split::follow
#[derive(Clone, Copy)]
pub(crate) struct Split {
    /// The number the app holds across the container's unmount, in [`Split::unit`].
    pub(crate) size: State<f32>,
    /// What the panels register into, so a drag on the handle can be read back out.
    pub(crate) context: State<ResizableContext>,
    unit: Unit,
    floor: f32,
    ceiling: f32,
}

impl Split {
    /// A split at `initial`, dragged within `floor..=ceiling`: the two states of one,
    /// made together.
    pub(crate) fn create(initial: f32, unit: Unit, floor: f32, ceiling: f32) -> Split {
        Split {
            size: State::create(initial),
            context: State::create(ResizableContext {
                direction: Direction::Horizontal,
                ..Default::default()
            }),
            unit,
            floor,
            ceiling,
        }
    }

    /// Follow the handle: what the reader drags it to becomes the number this holds,
    /// which is what carries the size across the container's own unmount.
    ///
    /// Reading the context is what subscribes the caller to the drag, and
    /// `set_if_modified` keeps the panels' registration at mount from waking anything.
    ///
    /// **A hook**, so every caller calls it while rendering and calls it unconditionally,
    /// above whatever early return it has: the document's split here, the Scratchpad's
    /// (`src/ui/pad_view.rs`) and the sidebar's (`src/ui/no_project.rs`).
    pub(crate) fn follow(self) {
        let (context, mut size) = (self.context, self.size);
        use_side_effect(move || {
            let live = context.read().panels.first().map(|panel| panel.size);
            if let Some(live) = live {
                size.set_if_modified(live);
            }
        });
    }

    /// The leading panel's `initial_size`: the number held, clamped to this split's
    /// bounds and in its unit. The bounds are the split's own and not the caller's, so the
    /// two panels of one split cannot be clamped differently.
    ///
    /// **A `peek` and never a `read`**: `initial_size` is consulted once, in the panel's
    /// own `use_hook` at mount, so subscribing to it would be a subscription to nothing --
    /// and a loop with [`Split::follow`].
    pub(crate) fn panel_size(self) -> PanelSize {
        let size = self.size.peek().clamp(self.floor, self.ceiling);
        match self.unit {
            Unit::Percent => PanelSize::percent(size),
            Unit::Pixels => PanelSize::px(size),
        }
    }

    /// The following panel's: what the leading one leaves. A percentage either way -- the
    /// rest of a share, or all of what a literal width leaves over.
    pub(crate) fn rest(self) -> PanelSize {
        match self.unit {
            Unit::Percent => {
                PanelSize::percent(100.0 - self.size.peek().clamp(self.floor, self.ceiling))
            }
            Unit::Pixels => PanelSize::percent(100.0),
        }
    }
}

/// How wide the **leading** side of a document is, as a percentage -- the side the tab is
/// driven from, which [`DocumentBody`] draws on the left in both kinds of tab. Kept by
/// place and not by pane, so switching from an assembly-driven tab to a source-driven one
/// leaves the handle where the reader put it instead of throwing the two widths across the
/// split.
#[derive(Clone, Copy)]
pub(crate) struct DocumentSplit(pub(crate) Split);

/// Put the pane that follows away, or bring it back: the one write that gesture is,
/// wherever it is made. The control on the bar ([`PaneToggle`]) and the window's key
/// (`Chord::OtherPane`) both call this, so the two cannot come to mean different things.
///
/// One insert under the [`Placing`] the gesture was made at -- a tab's id, or the
/// Scratchpad's pane -- and what it flips is what [`following`] says is up **now**, read
/// here rather than handed in: a gesture answers for the place as it stands rather than
/// for the render it was drawn in.
///
/// A tab whose document has left the table is nothing to flip: a menu or a key answered
/// after the tab closed.
pub(crate) fn toggle_pane(of: Placing, open: Open, mut said: State<HashMap<Placing, bool>>) {
    // Bound before the write: a read guard held across one panics.
    let document = match of {
        Placing::Tab(tab) => match open.docs.peek().get(tab).cloned() {
            Some(document) => Some(document),
            None => return,
        },
        Placing::Pad => None,
    };
    let up = following(of, document.as_ref(), &said.peek());
    said.write().insert(of, !up);
}

/// The control on the leading pane's bar that puts the pane the tab is not driven from
/// away, and brings it back.
///
/// **One control and not two**, wherever a following pane can be put away: the icon, the
/// tooltip, the hover box and the rule about where it sits are written once, and what
/// differs is only the [`Placing`] the flag is filed under.
///
/// **On the leading bar alone.** It names the following pane, which is always the
/// right-hand half of the split, so the control sits on the half that is always up and
/// the half it hides never carries one of its own: two of them, one closing the bar it is
/// drawn on, put the same button on screen twice for the sake of a press that takes its
/// own door away.
///
/// It takes the tab and reads the document out of [`Open`] rather than being handed
/// one: what it writes is filed under the tab anyway, and a [`Document`] prop would hold
/// an `Arc<Object>` in a control that every open tab draws.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct PaneToggle {
    pub(crate) of: Placing,
}

impl Component for PaneToggle {
    fn render(&self) -> impl IntoElement {
        let open = use_open();
        let docs = open.docs;
        let said = use_consume::<Follows>().0;
        let mut hovering = use_state(|| false);
        // Not hit while a sweep is under way, as the names beside it are not: the pointer
        // dragging a selection up past the bar would otherwise arm this tooltip.
        let sweeping = try_consume_context::<Marked>().is_some_and(|marked| sweeping(marked.0));

        // Which pane it is that follows, for the tooltip to say what the press does, and
        // whether it is up.
        let (name, up) = match self.of {
            Placing::Tab(tab) => {
                // Nothing to toggle behind an unfiled id: a harness that mounts a pane on
                // no tab.
                let Some(document) = docs.read().get(tab).cloned() else {
                    return rect().into_element();
                };
                // The side that follows is the one the tab is *not* driven from.
                let name = match document.driven_from() {
                    Pane::Source => "assembly",
                    Pane::Assembly => "source",
                };
                (name, following(self.of, Some(&document), &said.read()))
            }
            // The pad's editor is the side it is driven from, so the side that follows is
            // always the assembly.
            Placing::Pad => ("assembly", following(self.of, None, &said.read())),
        };
        let (icon, tip) = match up {
            true => (
                ("panel-right-close", lucide::panel_right_close()),
                format!("Hide the {name} pane"),
            ),
            false => (
                ("panel-right-open", lucide::panel_right_open()),
                format!("Show the {name} pane"),
            ),
        };
        let side = toggle_size();
        let of = self.of;

        // A box of the bar's own row height around the square, so the control sits beside
        // the first name in a bar that has grown a section rather than down the middle of
        // one.
        rect()
            .height(Size::px(list_row_height()))
            .main_align(Alignment::Center)
            .interactive(!sweeping)
            .child(extra_tooltip(
                tip,
                CursorArea::new().child(
                    rect()
                        .width(Size::px(side))
                        .height(Size::px(side))
                        .center()
                        .corner_radius(4.0)
                        .maybe(hovering(), |button| {
                            button.background(palette().toggle_hover_bg)
                        })
                        .on_pointer_over(move |_| hovering.set_if_modified(true))
                        .on_pointer_out(move |_| hovering.set_if_modified(false))
                        // The press writes nothing itself: which flag it is and the rule
                        // for flipping it are `toggle_pane`'s, which the key calls too.
                        .on_press(move |_| toggle_pane(of, open, said))
                        .child(glyph(icon)),
                ),
            ))
            .into_element()
    }
}

/// One document, drawn: the side it is driven from beside the side that follows, in a
/// `ResizableContainer` rather than a nested `DockingArea`.
///
/// **The driven side leads**, which is to say it is the left-hand pane: an assembly-driven
/// tab draws its listing there and a source-driven tab its own file, because in both the
/// leading pane is the one the reader came here to read and the trailing one is what it
/// resolves to. The two panes are the same components either way -- neither knows which
/// side of the split it was given -- so nothing but their order changes.
///
/// **The following pane is the one that can be put away**, by the toggle on either pane's
/// bar ([`PaneToggle`]), and a source-driven tab on a file in no compiled language opens
/// with it away already ([`following`]). What is left is the leading pane alone, with no
/// container and no handle: the app's one split width is untouched, so it comes back as
/// the reader left it on the next tab that has two panes.
///
/// Only the *active* tab's content is mounted, but a switch of tab is **not** a remount:
/// nothing here is keyed, so freya re-renders these same scopes with the new tab's props
/// and every hook they hold lives on. Only a switch to a tab of another *kind* builds the
/// subtree afresh, the element types at these paths having changed. So a hook that must
/// follow the tab reads what it is handed rather than capturing it (`use_window`,
/// `src/ui/section_view.rs`), and where a pane was left is kept outside it, which is what
/// `use_kept_position` is for. Navigating in place is not a switch of tab: this reads the
/// table, so a push onto the trail re-renders it and the panes are handed the new
/// document as a prop, keeping their controllers -- and the same hook files the row of
/// the place left under that place's own entry before putting the arriving one back. A
/// step between two places in *one* object's code is not even a switch of document, and
/// the same hook answers it for the same reason: what a position is kept under is the
/// place and not the document (`Entry`).
#[derive(Clone, PartialEq)]
pub(crate) struct DocumentBody {
    pub(crate) id: DocId,
}

impl Component for DocumentBody {
    fn render(&self) -> impl IntoElement {
        let docs = use_open().docs;
        let split = use_consume::<DocumentSplit>().0;
        let said = use_consume::<Follows>().0;

        // Where the reader last left the handle, written back as they drag it. Above the
        // early return below, as a hook has to be.
        split.follow();

        // Not reachable -- the tab and the table entry are closed together -- but a render
        // is no place to panic.
        let Some(document) = docs.read().get(self.id).cloned() else {
            return blank_pane(palette().asm_pane_bg);
        };

        let tab = self.id;
        // Bound before the panes are built, which take the document: reading it here is
        // also what subscribes this tab to its own toggle.
        let showing = following(Placing::Tab(tab), Some(&document), &said.read());

        // Which pane leads is the *document's* question and not the panels': the sizes
        // stay with the two places, the reader's side and the side that follows it, so
        // switching between the two kinds of tab leaves the handle where it was rather
        // than jumping it across the split.
        let driven = document.driven_from();
        let source = SourcePane {
            tab,
            document: document.clone(),
        }
        .into_element();
        let assembly = AssemblyPane { tab, document }.into_element();
        let (leads, follows) = match driven {
            Pane::Source => (source, assembly),
            Pane::Assembly => (assembly, source),
        };

        // The pane that follows, where this tab has one: put away by hand, or by the file
        // having no assembly side to show.
        if !showing {
            return leads;
        }

        ResizableContainer::new()
            .direction(Direction::Horizontal)
            .controller(split.context)
            .panel(
                // `min_size` given rather than left to default: freya's default is a
                // quarter of the initial size, so it would move with the reader's own
                // drag instead of staying the floor.
                ResizablePanel::new(split.panel_size())
                    .min_size(10.0)
                    .child(leads),
            )
            .panel(
                ResizablePanel::new(split.rest())
                    .min_size(10.0)
                    .child(follows),
            )
            .into_element()
    }
}

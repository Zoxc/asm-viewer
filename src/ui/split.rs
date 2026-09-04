//! One document drawn: the two panes, which side leads, and the control that puts the
//! following one away.
//!
//! The pane the tab is driven from leads and the other follows, which is a fact about the
//! *document* and not about the panels it is drawn in; the two are the same components
//! either way, so nothing but their order changes.

use super::*;

/// Whether the pane a tab is not driven from is up: what the reader last said about this
/// tab, and where they have said nothing, what its document opens with.
///
/// **Only a source-driven tab opens with one pane**, and only on a file in no compiled
/// language: a `Cargo.toml` or a `.json` is read and never disassembled, so the pane
/// beside it would be an empty half of the window with a handle to drag it wider. The
/// question is `source::compiled`, off the same extension list the grammars come from,
/// and an extension it does not know is answered no -- an assembly side is offered for
/// the languages the app can say become machine code, and a file it cannot place opens
/// as source until the reader asks for one.
pub(crate) fn following(tab: DocId, document: &Document, said: &HashMap<DocId, bool>) -> bool {
    match said.get(&tab) {
        Some(&said) => said,
        None => match document {
            Document::Source(file) => source::compiled(Path::new(&**file)),
            Document::Assembly(_) | Document::Code(_) => true,
        },
    }
}

/// The control on the leading pane's bar that puts the pane the tab is not driven from
/// away, and brings it back.
///
/// **On the leading bar alone.** It names the following pane, which is always the
/// right-hand half of the split, so the control sits on the half that is always up and
/// the half it hides never carries one of its own: two of them, one closing the bar it is
/// drawn on, put the same button on screen twice for the sake of a press that takes its
/// own door away.
///
/// It takes the tab and reads the document out of [`OpenDocs`] rather than being handed
/// one: what it writes is filed under the tab anyway, and a [`Document`] prop would hold
/// an `Arc<Object>` in a control that every open tab draws.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct PaneToggle {
    pub(crate) tab: DocId,
}

impl Component for PaneToggle {
    fn render(&self) -> impl IntoElement {
        let docs = use_consume::<OpenDocs>().0;
        let mut said = use_consume::<Follows>().0;
        let mut hovering = use_state(|| false);
        let tab = self.tab;
        // Not hit while a sweep is under way, as the names beside it are not: the pointer
        // dragging a selection up past the bar would otherwise arm this tooltip.
        let sweeping = try_consume_context::<Marked>().is_some_and(|marked| sweeping(marked.0));

        // Nothing to toggle behind a stray id: a harness that mounts a pane on no tab.
        let Some(document) = docs.read().get(tab).cloned() else {
            return rect().into_element();
        };
        // Which pane it is that follows, for the tooltip to say what the press does.
        let name = match &document {
            Document::Source(_) => "assembly",
            Document::Assembly(_) | Document::Code(_) => "source",
        };
        let up = following(tab, &document, &said.read());
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
        let (side, glyph) = (toggle_size(), icon_size());

        // A box of the bar's own row height around the square, so the control sits beside
        // the first name in a bar that has grown a section rather than down the middle of
        // one.
        rect()
            .height(Size::px(list_row_height()))
            .main_align(Alignment::Center)
            .interactive(!sweeping)
            .child(row_tooltip(
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
                        .on_press(move |_| {
                            said.write().insert(tab, !up);
                        })
                        .child(
                            SvgViewer::new(icon)
                                .width(Size::px(glyph))
                                .height(Size::px(glyph))
                                // Given rather than inherited, as the tab bar's icons are:
                                // `SvgViewer` rasterizes only once it knows a colour.
                                .color(palette().icon_fg)
                                .show_loader(false),
                        ),
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
/// Only the *active* tab's content is mounted, so this whole subtree -- both panes, both
/// scroll controllers -- is built afresh on every switch of tab, which is what
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
        let docs = use_consume::<OpenDocs>().0;
        let mut ratio = use_consume::<SplitRatio>().0;
        let splits = use_consume::<Splits>().0;
        let said = use_consume::<Follows>().0;

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
        let leading = ratio.peek().clamp(1.0, 99.0);

        // Not reachable -- the tab and the table entry are closed together -- but a render
        // is no place to panic.
        let Some(document) = docs.read().get(self.id).cloned() else {
            return rect()
                .expanded()
                .background(palette().asm_pane_bg)
                .into_element();
        };

        let tab = self.id;
        // Bound before the panes are built, which take the document: reading it here is
        // also what subscribes this tab to its own toggle.
        let showing = following(tab, &document, &said.read());

        // Which pane leads is the *document's* question and not the panels': the sizes
        // stay with the two places, the reader's side and the side that follows it, so
        // switching between the two kinds of tab leaves the handle where it was rather
        // than jumping it across the split.
        let (leads, follows) = match &document {
            Document::Source(_) => (
                SourcePane {
                    tab,
                    document: document.clone(),
                }
                .into_element(),
                AssemblyPane { tab, document }.into_element(),
            ),
            Document::Assembly(_) | Document::Code(_) => (
                AssemblyPane {
                    tab,
                    document: document.clone(),
                }
                .into_element(),
                SourcePane { tab, document }.into_element(),
            ),
        };

        // The pane that follows, where this tab has one: put away by hand, or by the file
        // having no assembly side to show.
        if !showing {
            return leads;
        }

        ResizableContainer::new()
            .direction(Direction::Horizontal)
            .controller(splits)
            .panel(
                // `min_size` given rather than left to default: freya's default is a
                // quarter of the initial size, so it would move with the reader's own
                // drag instead of staying the floor.
                ResizablePanel::new(PanelSize::percent(leading))
                    .min_size(10.0)
                    .child(leads),
            )
            .panel(
                ResizablePanel::new(PanelSize::percent(100.0 - leading))
                    .min_size(10.0)
                    .child(follows),
            )
            .into_element()
    }
}

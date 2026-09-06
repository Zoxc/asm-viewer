//! The box a code listing is drawn in, which is one box for all three of them.
//!
//! `InstructionList`, `SourceList` and `SectionList` differ in what their rows are and
//! where the rows come from. What holds the rows does not: the focusable box the keyboard
//! reaches the pane through, the `on_sized` the viewport and the nudge come out of, the
//! sweep that carries a run on past the edge, and the `VirtualScrollView` itself, one
//! `code_row_height()` a row. Every line of it is load-bearing -- the order that handler
//! writes in, the padding that puts the rows on the pixel grid, the focus a press asks
//! for -- so it is written once here and each list hands in its pane, its rows and its
//! builder.
//!
//! [`use_list_box`] is the hooks the box is made of and [`ListBox::render`] the box.
//! **Both are hooks**, so a list calls each once and on every render; what is the list's
//! own -- the position it puts back, the caret a door planted, the window it asks the
//! worker for -- stays in the list, between the two.

use super::*;

/// One listing's box, as the list draws its rows against and closes over.
#[derive(Clone)]
pub(crate) struct ListBox {
    /// Which pane the list is: what its run and a sweep past its edge are marked under.
    pane: Pane,
    /// The focusable box the keyboard reaches the pane through.
    a11y: AccessibilityId,
    /// The list's scroll, which its own position hooks move.
    pub(crate) controller: ScrollController,
    /// How tall the list is, which `reveal_row` needs to know whether the row it was
    /// asked for is on screen already. `VirtualScrollView` measures itself but keeps the
    /// answer, so the box around it is what is measured.
    pub(crate) viewport: State<f32>,
    /// The list as its rows and a sweep past its edge know it: its scroll, its box, the
    /// paragraphs the rows lend it, its widest row and its nudge.
    listing: Listing,
    /// The device pixel grid, read at the render so the `on_sized` handler asks nothing
    /// of the runtime.
    grid: Grid,
}

/// The hooks a code listing opens with, for the listing keyed `listing` drawn in `pane`.
/// A hook itself: called on every render of the list, before any of the list's own.
///
/// The key is the list's to work out, being the identity of what outlives its rows
/// ([`Widest::key`]), and is told to the [`Listing`] here rather than kept: a list is not
/// mounted again when its listing changes, and the context is made once.
pub(crate) fn use_list_box(pane: Pane, listing: u64) -> ListBox {
    // A `pointer_down` anywhere inside the box bubbles to it and asks for focus, which is
    // what makes Ctrl+C mean this listing.
    let a11y = use_a11y();
    use_tab_keyboard(a11y);
    let controller = use_scroll_controller(ScrollConfig::default);
    let viewport = use_state(|| 0.0f32);
    // The widest row drawn, under the listing's identity: what every row is at least as
    // wide as, so the list scrolls sideways over a stable extent.
    let widest = use_widest();
    // Made here and not in the context's closure, which runs once: a hook has to run on
    // every render.
    let nudge = use_state(|| 0.0f32);
    let grid = pixel_grid();
    let held = use_provide_context(|| Listing::new(controller, widest, nudge));
    held.drawing(listing);
    ListBox {
        pane,
        a11y,
        controller,
        viewport,
        listing: held,
        grid,
    }
}

impl ListBox {
    /// The box, around `length` rows built from `data` by `build`. A hook, the sweep's
    /// cells being one, so it is called once per render.
    ///
    /// The data goes to the `VirtualScrollView` and never into the builder: the builder
    /// closure is not compared across renders, so anything the rows depend on that is
    /// captured is never seen again.
    pub(crate) fn render<D: PartialEq + 'static>(
        &self,
        marked: State<Marks>,
        length: usize,
        on_key_down: impl FnMut(Event<KeyboardEventData>) + 'static,
        data: D,
        build: impl Fn(usize, &D) -> Element + 'static,
    ) -> Rect {
        let (a11y, grid) = (self.a11y, self.grid);
        let mut viewport = self.viewport;
        let listing = self.listing.clone();
        let bounds = listing.bounds.clone();

        rect()
            .expanded()
            .a11y_id(a11y)
            .a11y_focusable(true)
            .on_pointer_down(move |_| a11y.request_focus())
            .on_key_down(on_key_down)
            .on_sized({
                let listing = listing.clone();
                move |e: Event<SizedEventData>| {
                    viewport.set_if_modified(e.area.height());
                    listing.measured(grid, e.area.min_y());
                    bounds.set(e.area);
                }
            })
            .on_global_pointer_move(use_sweep_beyond(marked, self.pane, listing.clone(), length))
            // On the grid: see `Listing::padding`.
            .padding(listing.padding())
            .child(
                VirtualScrollView::new_with_data_controlled(data, build, self.controller)
                    .length(length)
                    .item_size(code_row_height()),
            )
    }
}

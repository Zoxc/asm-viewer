//! The two panes pointing at each other, and each tab's memory of where it was left.
//!
//! `Focused` is where the *pointer* is and `Pinned` is where a *click* fixed them: two
//! states, because a pin a hover could overwrite is a pin a hover silently undoes.

use super::*;

/// A source position the two panes point at together. The file is half the identity: an
/// inlined header's line 42 is not line 42 of the open file.
///
/// **Compared by its text and not by pointer**, unlike every other `Arc` the UI passes
/// around: two `LineInfo`s naming one file hold two `Arc<str>`s of its path.
#[derive(Clone, PartialEq)]
pub(crate) struct LinePos {
    pub(crate) file: Arc<str>,
    pub(crate) line: u32,
}

/// Which row put the focus where it is. It is the pair, position and origin, that
/// `release_focus` compares: two instructions compiled from one source line share a
/// position but not an address.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusOrigin {
    /// The assembly row for the instruction at this address.
    Instruction(u64),
    /// The source row for the focused line itself.
    Source,
}

#[derive(Clone, PartialEq)]
pub(crate) struct LineFocus {
    pub(crate) at: LinePos,
    pub(crate) from: FocusOrigin,
}

/// The cross-view focus, shared through context. `None` while the pointer is on neither
/// pane.
#[derive(Clone, Copy)]
pub(crate) struct Focused(pub(crate) State<Option<LineFocus>>);

/// Give up the focus a row set when the pointer leaves it, unless another row has taken it
/// over since.
///
/// **A row cannot clear the focus unconditionally**: `EventName::cmp` (freya-core
/// `events/name.rs`) leaves the order of the leaving row's `pointerout` and the entering
/// row's `pointerover` undefined, so this clears only what this row itself put there --
/// origin included, which is what keeps two instructions of one source line apart.
pub(crate) fn release_focus(mut focused: State<Option<LineFocus>>, mine: Option<&LineFocus>) {
    if mine.is_some() && focused.peek().as_ref() == mine {
        focused.set(None);
    }
}

/// One of the two panes that show code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Assembly,
    Source,
}

/// The source position a click fixed the two panes on.
#[derive(Clone, PartialEq)]
pub(crate) struct Pin {
    pub(crate) at: LinePos,
    /// The pane that has yet to scroll `at` into view -- always the other one from the
    /// pane clicked -- and `None` once it has. Separate from `at` so that clicking the
    /// same line twice is two requests.
    pub(crate) reveal: Option<Pane>,
}

/// The pinned position, shared through context. `None` until something is clicked, and
/// again whenever the selection changes (`use_clear_focus`).
#[derive(Clone, Copy)]
pub(crate) struct Pinned(pub(crate) State<Option<Pin>>);

/// The position `pane` still owes a scroll to, if it is owed one.
///
/// **A look and not a take.** The click that pins is, in a source-driven tab, the click
/// that asks for the listing, so the run this wakes is still holding the *previous* one,
/// in which no row matches. Consuming the request there would spend it on a listing that
/// cannot answer it and the one that can would arrive to nothing owed. So the field is
/// left meaning what it says -- the pane owes the scroll until it has made it -- and
/// [`reveal_made`] is what clears it. A request nothing ever matches stays owed until the
/// next click replaces it or [`use_clear_focus`] drops it with the tab.
pub(crate) fn owed_reveal(pinned: State<Option<Pin>>, pane: Pane) -> Option<LinePos> {
    // `read` and not `peek`: this is the subscription that wakes the caller's effect on
    // the next click, so it has to happen before any early return.
    let pin = pinned.read();
    match pin.as_ref() {
        Some(pin) if pin.reveal == Some(pane) => Some(pin.at.clone()),
        _ => None,
    }
}

/// Say that `pane` has made the scroll it was owed. The pin itself stays, only `reveal`
/// is cleared, so it is answered exactly once and a repeat click is a second request.
pub(crate) fn reveal_made(mut pinned: State<Option<Pin>>, pane: Pane) {
    let owed = matches!(pinned.peek().as_ref(), Some(pin) if pin.reveal == Some(pane));
    if !owed {
        return;
    }

    if let Some(pin) = pinned.write().as_mut() {
        pin.reveal = None;
    }
}

/// Bring the row at `index` into view, and leave the scroll alone when it already is.
///
/// A `VirtualScrollView` counts its offset *down* from zero and clamps whatever is set
/// here against the content on the next layout, so the arithmetic need not know how long
/// the list is.
pub(crate) fn reveal_row(controller: &mut ScrollController, viewport: f32, index: usize) {
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -scrolled as f32;
    let height = code_row_height();
    let row = index as f32 * height;
    let margin = CONTEXT_ROWS * height;

    if row >= top + margin && row + height <= top + viewport {
        return;
    }

    controller.scroll_to_y(-((row - margin).max(0.0) as i32));
}

/// Keep `controller` pointed at the row `tab` was last left at, and keep [`Positions`]
/// told where it is now. `length` is what the pane holds *now*, which is what makes the
/// answer a row of this listing rather than of the one it was saved from.
///
/// Two things make it work, and both are about *when*:
///
/// - **The effect is subscribed to the pane's own scroll**, because reading the
///   controller's position is a `State::read` inside it. So every scroll is written down
///   as it happens rather than on the way out of the tab, which is what makes a position
///   survive the window being closed and the pane unmounting.
/// - **The tab the controller is *holding*** is tracked here and is not the tab the app is
///   showing: they differ for exactly the one run that has to move the view, and every
///   write goes under the held tab.
pub(crate) fn use_kept_position<T: Clone + PartialEq + 'static>(
    mut positions: State<Positions<T>>,
    is_open: impl Fn(&T) -> bool + 'static,
    mut controller: ScrollController,
    tab: &T,
    length: usize,
) {
    // Which tab the controller is scrolled for. An `Rc<RefCell>` and not a `State`:
    // nothing renders from it, and a state would cost the pane a render per switch.
    let held = use_hook(|| Rc::new(RefCell::new(None::<T>)));

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would hold the first tab this pane ever showed.
    use_side_effect_with_deps(&(tab.clone(), length), move |(tab, length): &(T, usize)| {
        // Subscribes this effect to the pane's scroll, so it comes before any return.
        let (_, offset) = <(i32, i32)>::from(controller);
        // The row at the top of the pane. `code_row_height` and not the list's, this
        // being a code pane; rounded down, so a row half on screen is the row the reader
        // is looking at.
        let row = ((-offset).max(0) as f32 / code_row_height()) as usize;

        // Cloned out of the borrow rather than held across the `borrow_mut` below.
        let holding = held.borrow().clone();
        let switching = holding.as_ref() != Some(tab);
        let known = positions.peek().at(tab);
        let back_to = positions.peek().row(tab, *length);

        // Whose row the offset above is, and where this run has to move the view to.
        let (owner, moving) = match (&holding, known) {
            // Still showing the tab the controller is scrolled for: nothing moves.
            (Some(held), _) if held == tab => (Some(tab.clone()), None),
            // A switch: the offset belongs to the tab being left, and the one arriving
            // goes back to where it was, or to the top if it has never been seen.
            (Some(out), Some(_)) => (Some(out.clone()), Some(back_to)),
            (Some(out), None) => (Some(out.clone()), Some(0)),
            // This pane's first run, on a tab it has a row for: a remount or a restored
            // session. Nothing to write down, everything to put back.
            (None, Some(_)) => (None, Some(back_to)),
            // First run with nothing remembered: leave the view where it is, since this
            // runs a beat after the first render and would undo a wheel that got in.
            (None, None) => (Some(tab.clone()), None),
        };

        if let Some(owner) = owner {
            // Only for a tab that is still open: the run after a close is still holding
            // it, and writing its row down would put it straight back. **Asked of the
            // states themselves, never of a `Memo` over them**, which can still be
            // reporting a just-closed tab as open during exactly this run.
            let still_open = is_open(&owner);
            // And only when it has moved: `State::write` notifies whether or not the
            // value changes, and this runs on every scroll event.
            let at = positions.peek().at(&owner);
            if still_open && at != Some(row) {
                positions.write().remember(owner, row);
            }
        }
        if switching {
            *held.borrow_mut() = Some(tab.clone());
        }
        if let Some(row) = moving {
            // A write this effect is subscribed to: it wakes once more, finds the tab it
            // is holding is the tab it is showing, and writes the row down.
            controller.scroll_to_y(-((row as f32 * code_row_height()) as i32));
        }
    });
}

/// Forget the cross-view focus and the pin whenever the active document changes: both are
/// positions inside the drawn symbol's line info. Navigating from a relocation label
/// leaves the pointer sitting on a row, so the focus need never be released the ordinary
/// way, and a pin has no ordinary way at all.
pub(crate) fn use_clear_focus(
    active: Memo<Option<Document>>,
    focused: State<Option<LineFocus>>,
    pinned: State<Option<Pin>>,
) {
    use_side_effect(move || {
        // Subscribes the effect to the active document, which is all it wants from it.
        let _ = active.read();

        let (mut focused, mut pinned) = (focused, pinned);
        focused.set_if_modified(None);
        pinned.set_if_modified(None);
    });
}

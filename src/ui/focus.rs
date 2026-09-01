//! The two panes pointing at each other, and each tab's memory of where it was left.
//!
//! `Focused` is where the *pointer* is and `Pinned` is where a *click* fixed them, which
//! is why they are two states and not one: a pin a hover could overwrite is a pin a hover
//! silently undoes. A pin also carries the scroll one pane asks of the other, taken once.
//!
//! `use_kept_position` is the same machinery seen from the other end. Both are a pane's
//! `ScrollController` being told about a row it did not itself pick -- one because the
//! other pane asked, one because the tab arriving was left there -- so the rule about
//! which of the two wins is a sentence about one file rather than about two.

use super::*;

/// A source position the two panes point at together.
///
/// The file is half the identity rather than decoration: a symbol's rows can name several
/// files -- an inlined header's line 42 is not line 42 of the file the source pane has
/// open -- so a line number alone would light up the wrong row. Compared by its text and
/// not by pointer, unlike every other `Arc` the UI passes around: this is a position and
/// not an object, and two `LineInfo`s naming one file hold two `Arc<str>`s of its path.
#[derive(Clone, PartialEq)]
pub(crate) struct LinePos {
    pub(crate) file: Arc<str>,
    pub(crate) line: u32,
}

/// Which row put the focus where it is.
///
/// Paired with the position in `LineFocus`, and it is the pair a row compares against
/// before giving the focus up again (`release_focus`): two instructions compiled from one
/// source line share a position but not an address, so the origin is what tells them
/// apart, and two source rows differ in the position already.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusOrigin {
    /// The assembly row for the instruction at this address.
    Instruction(u64),
    /// The source row for the focused line itself.
    Source,
}

/// The source position the pointer is pointing at, and which side it points from.
#[derive(Clone, PartialEq)]
pub(crate) struct LineFocus {
    pub(crate) at: LinePos,
    pub(crate) from: FocusOrigin,
}

/// The cross-view focus, shared through context: hovering an instruction puts the position
/// it was compiled from here, hovering a source line puts that line here, and both panes
/// light up whatever matches. `None` while the pointer is on neither.
#[derive(Clone, Copy)]
pub(crate) struct Focused(pub(crate) State<Option<LineFocus>>);

/// Give up the focus a row set when the pointer leaves it, unless another row has taken it
/// over since.
///
/// A row cannot simply clear the focus. `pointerout` on the row being left and
/// `pointerover` on the one being entered are sorted against each other by an
/// `EventName::cmp` (freya-core `events/name.rs`) that answers `Less` for both of them, so
/// which of the two runs first is not something to lean on. Clearing only what this row
/// itself put there is right in either order -- and comparing the whole focus, origin as
/// well as position, is what keeps two instructions of one source line apart: they set the
/// same position, so the row being left would otherwise blank the highlight the row being
/// entered had just set.
pub(crate) fn release_focus(mut focused: State<Option<LineFocus>>, mine: Option<&LineFocus>) {
    if mine.is_some() && focused.peek().as_ref() == mine {
        focused.set(None);
    }
}

/// One of the two panes that show code.
///
/// Not `Tab`, which names nine views of which seven have nothing to answer here: this is the
/// side of a mapping, and a mapping has exactly two.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Assembly,
    Source,
}

/// The source position a click fixed the two panes on.
///
/// A pin is the hover of 5b made to stay. Hovering is how the mapping is explored and it
/// has to end when the pointer moves on; clicking is how a reader says *this one*, and a
/// highlight that evaporated the moment the pointer left for the pane it had just scrolled
/// would be answering a question nobody asked. The two live side by side rather than one
/// replacing the other, so a pin never costs the hover and the hover can never quietly
/// undo a pin: both light their rows, the pin more strongly.
#[derive(Clone, PartialEq)]
pub(crate) struct Pin {
    pub(crate) at: LinePos,
    /// The pane that has yet to scroll `at` into view -- always the other one from the
    /// pane clicked -- and `None` once it has, or once it has decided there is nothing
    /// there to scroll to. Carried in the pin rather than in a state of its own because
    /// the request and the highlight are one gesture; keeping it separate from `at` is
    /// what makes clicking the same line twice two requests, so a pane the reader has
    /// scrolled away from by hand comes back.
    pub(crate) reveal: Option<Pane>,
}

/// The pinned position, shared through context. `None` until something is clicked, and
/// again whenever the selection changes (`use_clear_focus`).
#[derive(Clone, Copy)]
pub(crate) struct Pinned(pub(crate) State<Option<Pin>>);

/// Take the request `pane` is owed, if it is owed one.
///
/// The pin itself stays where it is -- it is what both panes light up, for as long as the
/// symbol is on screen -- and only the request to scroll is cleared, so that it is answered
/// once. Clearing it from inside the effect that reads it wakes that effect one more time,
/// which finds nothing and stops; the alternative, a counter that says "this is a different
/// click", would leave every pane having to remember which counter it last acted on.
pub(crate) fn take_reveal(mut pinned: State<Option<Pin>>, pane: Pane) -> Option<LinePos> {
    let at = {
        // `read` rather than `peek` on purpose: this is the subscription that wakes the
        // caller's effect on the next click, so it has to happen before any early return.
        let pin = pinned.read();
        match pin.as_ref() {
            Some(pin) if pin.reveal == Some(pane) => pin.at.clone(),
            _ => return None,
        }
    };

    if let Some(pin) = pinned.write().as_mut() {
        pin.reveal = None;
    }

    Some(at)
}

/// Bring the row at `index` into view, and leave the scroll alone when it already is.
///
/// A `VirtualScrollView` counts its offset *down* from zero -- `-offset / item_size` is the
/// first row it builds -- so a row's own offset is the negative of its distance from the
/// top, and whatever is set here is clamped against the content on the next layout
/// (`get_corrected_scroll_position`), which is why the arithmetic need not know how long
/// the list is.
///
/// Nothing moves while the row is already on screen and clear of the top edge. The gesture
/// this answers is reading down a function clicking one instruction after another: their
/// lines are in view on the other side already, and a pane that re-scrolled on every one of
/// them would be moving under the reader for no reason.
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

/// The row at the top of a code pane scrolled to `offset`, and the offset that puts `row`
/// there — the one place the two units meet.
///
/// [`code_row_height`] and not the list's: both callers are the two code panes
/// (`use_kept_position`, and `reveal_row` above), a sidebar list neither keeping a
/// per-tab position nor having a row to reveal.
///
/// A `VirtualScrollView`'s offset counts *down* from zero, so the arithmetic is a
/// negation and a divide by [`code_row_height`], which is those panes' `item_size`. Rounded
/// *down*, which is the half-row a position in rows gives up and the direction to give it
/// up in: the row at the top edge is the one the reader is looking at even when it is only
/// half on screen, and coming back to the one below it would lose the half they could see.
fn row_at(offset: i32) -> usize {
    ((-offset).max(0) as f32 / code_row_height()) as usize
}

fn row_offset(row: usize) -> i32 {
    -((row as f32 * code_row_height()) as i32)
}

/// Keep `controller` pointed at the row `tab` was last left at, and keep [`Positions`]
/// told where it is now.
///
/// Both panes' halves of "a viewing position per tab", from the one place: a pane holds
/// one scroll controller and shows one tab at a time, so switching tab means writing the
/// outgoing tab's row down and putting the incoming tab's row back. `length` is what the
/// pane is holding *now*, which is what makes the answer a row of this listing rather
/// than of the one it was saved from.
///
/// Two things make it work, and both are about *when* rather than what:
///
/// - **The effect is subscribed to the pane's own scroll**, because reading the
///   controller's position is a `State::read` inside it (`ScrollController`'s
///   `From<..> for (i32, i32)`, which is the only way to ask). So every scroll the reader
///   makes wakes this and is written down as it happens, rather than only on the way out
///   of the tab — which is what makes the position survive the window simply being closed,
///   and what makes it survive the pane unmounting (which the assembly pane does whenever
///   the selection is an object, taking its controller with it).
/// - **The tab the controller is *holding* is tracked here**, in a plain `Rc<RefCell<..>>`
///   rather than a `State`, and is not the same thing as the tab the app is showing. The
///   two differ for exactly one run of this effect — the one that has to move the view —
///   and every other write goes under the held tab, so a scroll that lands between a tab
///   switch and this effect cannot be written down against the tab it is not from. It is
///   not a `State` because nothing renders from it and writing one here would cost the
///   pane a second render on every switch. `open` is what keeps that from resurrecting a
///   tab that has just been closed: the run after a close is holding one, and the three
///   closing functions have already forgotten it.
///
/// **A [`Pin::reveal`] wins over a remembered position, and needs nothing to make it.**
/// The two never ask at the same moment: this moves the view only when the tab changes,
/// and a reveal is asked for by a click in the *other* pane, which changes no tab —
/// while a change of document, which does, drops the pin outright (`use_clear_focus`).
/// When a reveal does scroll, this effect wakes on the scroll it made and records it, so
/// the last thing the reader was shown is what the tab is remembered at. The memory
/// follows the reveal rather than fighting it.
pub(crate) fn use_kept_position<T: Clone + PartialEq + 'static>(
    mut positions: State<Positions<T>>,
    is_open: impl Fn(&T) -> bool + 'static,
    mut controller: ScrollController,
    tab: &T,
    length: usize,
) {
    // Not `use_state`: see above. `use_hook` runs its initializer once per component, so
    // this is the pane's own memory of which tab its controller is scrolled for.
    let held = use_hook(|| Rc::new(RefCell::new(None::<T>)));

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would hold the first tab this pane ever showed for as long as it lived.
    use_side_effect_with_deps(&(tab.clone(), length), move |(tab, length): &(T, usize)| {
        // Reading the controller's position is what subscribes this effect to the pane's
        // scroll, so it has to happen before anything can return early.
        let (_, offset) = <(i32, i32)>::from(controller);
        let row = row_at(offset);

        // Cloned out of the borrow rather than held across the `borrow_mut` below, which
        // panics exactly the way a `State` guard held across a write does.
        let holding = held.borrow().clone();
        let switching = holding.as_ref() != Some(tab);
        let known = positions.peek().at(tab);
        let back_to = positions.peek().row(tab, *length);

        // Whose row the offset above is, and where this run has to move the view to.
        let (owner, moving) = match (&holding, known) {
            // Still showing the tab the controller is scrolled for -- a scroll, a resize,
            // a re-render. The offset is that tab's own and nothing moves.
            (Some(held), _) if held == tab => (Some(tab.clone()), None),
            // A switch, with a row for the tab arriving: the offset belongs to the one
            // being left, and the one arriving goes back to where it was.
            (Some(out), Some(_)) => (Some(out.clone()), Some(back_to)),
            // A switch onto a tab never seen: the top, and pointedly not wherever the tab
            // before it had got to, which is the whole bug this hook exists for.
            (Some(out), None) => (Some(out.clone()), Some(0)),
            // This pane's first run, on a tab it has a row for: a remount, or a session
            // just restored. Nothing to write down -- a fresh controller sits at the top,
            // which is not where this tab was -- and everything to put back.
            (None, Some(_)) => (None, Some(back_to)),
            // First run with nothing remembered: leave the view where it is. It is at the
            // top already, and this runs a beat *after* the pane's first render, so a
            // scroll to the top here would undo a wheel that got in before it.
            (None, None) => (Some(tab.clone()), None),
        };

        if let Some(owner) = owner {
            // Only for a tab that is still open, which is why `is_open` is an argument
            // here at all: `close_tab` forgets a tab's position and then moves to a
            // neighbour, so the run that follows is holding a tab that has gone -- and
            // writing its row down would put it straight back, keyed by a `Document`
            // that holds a whole `Object`. That the last scroll before a close is lost
            // with it is the right answer twice over: there is no tab to bring it back
            // for, and the file it pointed into may be being let go of in the same
            // breath (`close_binary`).
            //
            // **It has to be asked of the states themselves, never of a `Memo` over
            // them.** A memo is recomputed by a task woken on a notify, so it can still
            // be reporting a just-closed tab as open during exactly the run this guard
            // exists for, and the resurrection would be back. The two real call sites
            // ask `Docs`, which the close has already written.
            let still_open = is_open(&owner);
            // And only when it has actually moved. `State::write` notifies whether or not
            // the value changes, and this runs on every scroll event, so writing back what
            // is already there would wake the save observer for a pointer sitting still.
            let at = positions.peek().at(&owner);
            if still_open && at != Some(row) {
                positions.write().remember(owner, row);
            }
        }
        if switching {
            *held.borrow_mut() = Some(tab.clone());
        }
        if let Some(row) = moving {
            // A no-op when the view is there already, and otherwise a write this effect
            // is subscribed to: it wakes once more, finds the tab it is holding is the
            // tab it is showing, and writes the row down.
            controller.scroll_to_y(row_offset(row));
        }
    });
}

/// Forget the cross-view focus and the pin whenever the active document changes.
///
/// Both are positions inside the drawn symbol's line info, so they mean nothing once
/// that symbol is gone -- and the ordinary way the focus goes away, the pointer leaving the
/// row that set it, need never happen: clicking a relocation label navigates from an
/// assembly row the pointer is still sitting on, and the symbol it lands in was very often
/// compiled from the same file, so a line of that file would stay lit for a position in a
/// function no longer on screen until the pointer moved. A pin has no such ordinary way at
/// all -- staying is the whole of what makes it one -- so this is the only thing that ends
/// it short of another click.
///
/// Its own effect rather than a line inside the save observer: it has no business
/// subscribing to anything but the active document, and the two concerns stay separable.
pub(crate) fn use_clear_focus(
    active: Memo<Option<Document>>,
    focused: State<Option<LineFocus>>,
    pinned: State<Option<Pin>>,
) {
    use_side_effect(move || {
        // Reading subscribes the effect to the active document, which is the whole of
        // what it wants from it -- both are `None` again whatever the new one is.
        let _ = active.read();

        let (mut focused, mut pinned) = (focused, pinned);
        focused.set_if_modified(None);
        pinned.set_if_modified(None);
    });
}

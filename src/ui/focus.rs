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
    /// The panes that have yet to scroll `at` into view -- the other one from the pane
    /// clicked, or both for a click made in neither -- and none once they have. Separate
    /// from `at` so that clicking the same line twice is two requests.
    pub(crate) reveal: Owed,
    /// Whether this pin was made by a [`Landing`] -- a click from outside both panes --
    /// which names the **file** the Source pane shows as well as the line, where a click
    /// inside them cannot change which file is up. See `source_side`.
    pub(crate) landed: bool,
}

/// Which of the two panes still owe a scroll to a pin. A pair of flags and not an
/// `Option<Pane>`: a click in one pane asks the other, but a row in the Locations panel
/// is a click in neither and asks both.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Owed {
    pub(crate) assembly: bool,
    pub(crate) source: bool,
}

impl Owed {
    pub(crate) const BOTH: Owed = Owed {
        assembly: true,
        source: true,
    };

    /// A scroll owed by `pane` alone.
    pub(crate) fn by(pane: Pane) -> Owed {
        match pane {
            Pane::Assembly => Owed {
                assembly: true,
                source: false,
            },
            Pane::Source => Owed {
                assembly: false,
                source: true,
            },
        }
    }

    fn owes(self, pane: Pane) -> bool {
        match pane {
            Pane::Assembly => self.assembly,
            Pane::Source => self.source,
        }
    }

    fn paid(&mut self, pane: Pane) {
        match pane {
            Pane::Assembly => self.assembly = false,
            Pane::Source => self.source = false,
        }
    }
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
        Some(pin) if pin.reveal.owes(pane) => Some(pin.at.clone()),
        _ => None,
    }
}

/// Say that `pane` has made the scroll it was owed. The pin itself stays, only `pane`'s
/// half of `reveal` is cleared, so it is answered exactly once and a repeat click is a
/// second request.
pub(crate) fn reveal_made(mut pinned: State<Option<Pin>>, pane: Pane) {
    let owed = matches!(pinned.peek().as_ref(), Some(pin) if pin.reveal.owes(pane));
    if !owed {
        return;
    }

    if let Some(pin) = pinned.write().as_mut() {
        pin.reveal.paid(pane);
    }
}

/// A line to pin the moment `tab` becomes the active document.
///
/// What a click from outside the two panes -- a row in the Locations panel -- needs and
/// a click inside them does not: opening the document is an `activate`, and the change of
/// document that makes is exactly what [`use_clear_focus`] answers by dropping the pin,
/// so a pin set in the same handler would be gone a beat later. Left here instead, for
/// that effect to turn into the pin when the document it names arrives.
#[derive(Clone, PartialEq)]
pub(crate) struct Landing {
    pub(crate) tab: Document,
    pub(crate) at: LinePos,
}

/// The landing asked for, shared through context. `None` almost always: it is set in the
/// handler that opens a document and spent by the change of document that follows.
#[derive(Clone, Copy)]
pub(crate) struct Land(pub(crate) State<Option<Landing>>);

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
/// `opening` is where a tab **nothing is remembered for** lands -- the Source pane's
/// symbol's own lines, and `0`, the top, for a pane or a symbol with nothing better to
/// say. A row remembered for the tab always wins over it: it is the first open this
/// answers and not every one.
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
///
/// And the pane's reveal is made **here**, by `reveal`, rather than by an effect of its
/// own: it is handed the controller and answers whether it scrolled, and a scroll it made
/// is where the arriving tab goes instead of back to its row. The two are owed at once
/// when a row in the Locations panel opens a symbol on a line, and two effects' scrolls
/// land in whichever order the runtime wakes them -- with the reveal first, it had
/// marked itself made by the time the kept row was put over it. One effect has one
/// order. `reveal` reads the pin, which is what wakes this on a click inside a tab.
pub(crate) fn use_kept_position<T: Clone + PartialEq + 'static>(
    mut positions: State<Positions<T>>,
    is_open: impl Fn(&T) -> bool + 'static,
    mut reveal: impl FnMut(&mut ScrollController) -> bool + 'static,
    mut controller: ScrollController,
    tab: &T,
    length: usize,
    opening: usize,
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
        // Clamped the way a remembered row is, and for the same reason: a symbol's line
        // is a hint out of debug info and the file under it may have been cut short since.
        let opening = opening.min(length.saturating_sub(1));

        // Whose row the offset above is, and where this run has to move the view to.
        let (owner, moving) = match (&holding, known) {
            // Still showing the tab the controller is scrolled for: nothing moves.
            (Some(held), _) if held == tab => (Some(tab.clone()), None),
            // A switch: the offset belongs to the tab being left, and the one arriving
            // goes back to where it was, or to where a tab seen for the first time opens.
            (Some(out), Some(_)) => (Some(out.clone()), Some(back_to)),
            (Some(out), None) => (Some(out.clone()), Some(opening)),
            // This pane's first run, on a tab it has a row for: a remount or a restored
            // session. Nothing to write down, everything to put back.
            (None, Some(_)) => (None, Some(back_to)),
            // First run with nothing remembered -- which, both panes being mounted afresh
            // for every document, is the ordinary first open of a tab. It moves only for
            // a pane that has somewhere to open at: a `0` is left alone rather than
            // scrolled to, since this runs a beat after the first render and setting the
            // offset it already has would undo a wheel that got in.
            (None, None) => (Some(tab.clone()), (opening != 0).then_some(opening)),
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
        // The reveal first, and the kept row only when it made none: either scroll is a
        // write this effect is subscribed to, so it wakes once more, finds the tab it is
        // holding is the tab it is showing, and writes the row down.
        if reveal(&mut controller) {
            return;
        }
        if let Some(row) = moving {
            controller.scroll_to_y(-((row as f32 * code_row_height()) as i32));
        }
    });
}

/// Forget the cross-view focus and the pin whenever the active document changes: both are
/// positions inside the drawn symbol's line info. Navigating from a relocation label
/// leaves the pointer sitting on a row, so the focus need never be released the ordinary
/// way, and a pin has no ordinary way at all.
///
/// The one exception is a [`Landing`] naming the document that has just arrived, which
/// becomes the pin instead, owed by both panes. A landing is spent by whichever document
/// arrives, the one it named or another: it is for the next arrival only, and one left
/// lying would pin a line in a document opened for some other reason later.
pub(crate) fn use_clear_focus(
    active: Memo<Option<Document>>,
    focused: State<Option<LineFocus>>,
    pinned: State<Option<Pin>>,
    landing: State<Option<Landing>>,
) {
    use_side_effect(move || {
        // Subscribes the effect to the active document, which is all it wants from it;
        // the landing is peeked, so setting one wakes nothing until the document does.
        let active = active.read().clone();

        let (mut focused, mut pinned, mut landing) = (focused, pinned, landing);
        focused.set_if_modified(None);

        let asked = landing.peek().clone();
        if asked.is_some() {
            landing.set(None);
        }
        let landed = asked
            .filter(|landing| Some(&landing.tab) == active.as_ref())
            .map(|landing| Pin {
                at: landing.at,
                reveal: Owed::BOTH,
                landed: true,
            });
        pinned.set_if_modified(landed);
    });
}

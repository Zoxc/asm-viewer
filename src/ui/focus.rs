//! A place in a file, the two panes named as a pair, the landing a click from outside
//! them makes, and each tab's memory of where it was left.
//!
//! What the two panes say to each other is in `marks.rs`: each pane's picked-out run is
//! what the other pane lights the pair of, and owes a scroll to.

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

/// One of the two panes that show code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane {
    Assembly,
    Source,
}

/// A place to pick out the moment `tab` becomes the active document: a line, an
/// instruction, or both.
///
/// What a click from outside the two panes -- a row in the Locations panel, a door out
/// of one listing into another -- needs and a click inside them does not: opening the
/// document is an `open_document`, and the change of document that makes is exactly what
/// `use_land` answers by giving the arriving place its own runs, so a run picked out in
/// the same handler would be gone a beat later. Left here instead, for that effect to
/// turn into the source pane's run when the document it names arrives -- over whatever
/// the place had kept -- and to hand on as a [`Planting`] for the assembly pane, whose
/// rows come later than the document does.
#[derive(Clone, PartialEq)]
pub(crate) struct Landing {
    pub(crate) tab: Document,
    /// The line to pick out in the source pane, where the door knew one: a Locations row
    /// names a line, and an instruction's door the line it was compiled from where it has
    /// one. `None` for the door an unnamed call's target opens, which knows an address
    /// and no line.
    pub(crate) at: Option<LinePos>,
    /// The instruction to put the assembly pane's caret on, where the door was one, as an
    /// address in the space the tab's listing draws: **placed** (`AsmData::placed`) for
    /// an object's code, the symbol's own for a symbol's tab.
    pub(crate) address: Option<u64>,
    /// The characters to select on `at`'s line, in the UTF-16 units a pane counts columns
    /// in: a search hit picks out what it matched, and a definition an empty run at the
    /// name's own column, which is a caret there and nothing selected. `None` for the
    /// doors that pick out the row and leave the caret at its start. Means nothing
    /// without `at`.
    pub(crate) columns: Option<Range<usize>>,
}

/// The landing asked for, shared through context. `None` almost always: it is set in the
/// handler that opens a document and spent by the change of document that follows.
#[derive(Clone, Copy)]
pub(crate) struct Land(pub(crate) State<Option<Landing>>);

/// An instruction the assembly pane's caret is to be put on once the listing of `tab` is
/// drawn: the half of a [`Landing`] the change of document cannot answer, since the rows
/// arrive after the document does -- a symbol's from the worker, an object's code's as
/// the skeleton and then as the stretch decodes. Left by `use_land` as it plants the
/// other half, or by `land` for a tab already on top, and spent by the listing drawing
/// the document it names: `use_kept_place` for an object's code, which puts the caret on
/// the row at or below the address and keeps the address with it so a decode re-places it
/// on the instruction itself; `InstructionList`'s planting effect for a symbol's. Spent
/// by `use_land` on every change of document besides, so one left lying -- a listing that
/// never arrived -- plants nothing in a listing opened for some other reason later.
#[derive(Clone, PartialEq)]
pub(crate) struct Planting {
    pub(crate) tab: Document,
    pub(crate) address: u64,
}

/// The caret still to be planted, shared through context, `None` almost always.
#[derive(Clone, Copy)]
pub(crate) struct Plant(pub(crate) State<Option<Planting>>);

/// Bring the row at `index` into view, and leave the scroll alone when it already is.
///
/// A `VirtualScrollView` counts its offset *down* from zero and clamps whatever is set
/// here against the content on the next layout, so the arithmetic need not know how long
/// the list is.
///
/// **Already there is measured against the offset this would write**, and not against the
/// context rows on their own. A row in the first `CONTEXT_ROWS` of a listing cannot have
/// them all above it, so asking for them was asking for an offset above the top of the
/// list: the scroll went to 0, the next call measured it against the same impossible
/// margin, and found it wanting again. That is a write per call for ever, and the caller
/// that reads the scroll to make it is woken by it (`use_kept_position`).
pub(crate) fn reveal_row(controller: &mut ScrollController, viewport: f32, index: usize) {
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -scrolled as f32;
    let height = code_row_height();
    let row = index as f32 * height;
    let margin = CONTEXT_ROWS * height;
    let wanted = (row - margin).max(0.0);

    if top <= wanted && row + height <= top + viewport {
        return;
    }

    controller.scroll_to_y(-(wanted as i32));
}

/// Bring the row the keyboard is on into view, and only when it is not: no context rows,
/// unlike [`reveal_row`], since a key repeat that scrolled the view while the row was
/// still on screen would walk the rows away from under the reader; a row above the view
/// comes to its top, one below to its bottom, as an editor's does.
///
/// The row height is the caller's, this being the one rule the finder's list follows as
/// well as a code pane's, and the two are measured in different fonts.
pub(crate) fn reveal_caret(
    controller: &mut ScrollController,
    viewport: f32,
    height: f32,
    index: usize,
) {
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -scrolled as f32;
    let row = index as f32 * height;

    if row < top {
        controller.scroll_to_y(-(row.max(0.0) as i32));
    } else if row + height > top + viewport {
        controller.scroll_to_y(-((row + height - viewport).max(0.0) as i32));
    }
}

/// What a pane's [`use_kept_position`] asks of it every render: the reveal it owes, the
/// landing it can take, and the row it opens at. Held so the effect reads the latest and
/// not the first.
struct Asked {
    reveal: Box<dyn FnMut(&mut ScrollController) -> bool>,
    coming: Box<dyn FnMut(&Landing, &mut ScrollController) -> bool>,
    opening: usize,
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
/// order. `reveal` reads the marks, which is what wakes this on a click inside a tab.
///
/// **A tab arriving with a landing on its way goes to the row the landing names as it
/// draws it, and holds the move it would otherwise make until the landing is spent.**
/// `use_land` turns a landing into a run two passes after the switch reaches here -- it
/// runs off `Active`, which is a memo -- so a pane left to its own devices drew the
/// arriving document at the outgoing place's offset until then, and a pane that made its
/// move first drew it at the top of the file. `coming` is asked to take the landing: it
/// answers for a row of what this pane is drawing, with the same `reveal_row` the run
/// makes later, so the run finds the row already on screen and moves nothing. A landing
/// it does not take -- a door that knew only an address, or one meant for the other pane
/// -- leaves the move held rather than made, since that pass may still plant this pane a
/// run. Nothing is stranded by a landing that never lands: one is only ever left by a
/// move that changes the place, and that arrival is what spends it.
///
/// `reveal` and `opening` are the **latest render's**, kept in a cell for the effect to
/// take. A tab handed another document is not mounted again -- a link followed in place,
/// a search hit shown in the temporal tab -- so a reveal held from the mount would go on
/// measuring the row it owes against the file the pane drew then, refuse it, and leave
/// the pane to fall back to the top.
///
/// `listing` is the key of what the pane is drawing (`Widest::key`), a dep and nothing
/// else. The effect runs when a dep differs or a state it read is written, never because
/// the pane rendered; without the key, an answer arriving at the same tab with the same
/// row count -- two accessors, two monomorphisations of one generic -- leaves a reveal
/// owed until the next click or wheel, when the point of leaving it owed is that the
/// listing which can answer it finds it.
pub(crate) fn use_kept_position<T: Clone + PartialEq + 'static>(
    mut positions: State<Positions<T>>,
    is_open: impl Fn(&T) -> bool + 'static,
    reveal: impl FnMut(&mut ScrollController) -> bool + 'static,
    coming: impl FnMut(&Landing, &mut ScrollController) -> bool + 'static,
    mut controller: ScrollController,
    tab: &T,
    length: usize,
    listing: u64,
    opening: usize,
) {
    // Which tab the controller is scrolled for. An `Rc<RefCell>` and not a `State`:
    // nothing renders from it, and a state would cost the pane a render per switch.
    let held = use_hook(|| Rc::new(RefCell::new(None::<T>)));

    // The reveal and the opening row as this render made them. The effect below is handed
    // fresh deps, but its callback is built once in a `use_hook`, so a value passed to it
    // by hand would stay the first render's. The one the hook makes is never read: every
    // render writes over it before the effect can run.
    let latest = use_hook(|| {
        Rc::new(RefCell::new(Asked {
            reveal: Box::new(|_| false),
            coming: Box::new(|_, _| false),
            opening: 0,
        }))
    });
    *latest.borrow_mut() = Asked {
        reveal: Box::new(reveal),
        coming: Box::new(coming),
        opening,
    };

    // The move this hook owes the view and has not made. An `Rc<RefCell>` for the same
    // reason as the tab above.
    let owing = use_hook(|| Rc::new(RefCell::new(None::<usize>)));
    // A landing on its way, whichever document it names. Asked through
    // `try_consume_context`, a pane mounted without the landing machinery having none on
    // its way.
    let landing = try_consume_context::<Land>().map(|land| land.0);

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would hold the first tab this pane ever showed.
    use_side_effect_with_deps(
        &(tab.clone(), length, listing),
        move |(tab, length, _): &(T, usize, u64)| {
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
            let opening = latest.borrow().opening.min(length.saturating_sub(1));

            // Whose row the offset above is, and where this run has to move the view to.
            let (owner, moving) = match (&holding, known) {
                // Still showing the tab the controller is scrolled for: nothing moves.
                (Some(held), _) if held == tab => (Some(tab.clone()), None),
                // A switch: the offset belongs to the tab being left, and the one arriving
                // goes back to where it was, or to where a tab seen for the first time opens.
                // A `0` moves here, where the first run below leaves one alone: the offset on
                // screen is the tab being left, and the arriving one must not inherit it.
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
            // Read and not peeked: this subscribes the effect to the landing, which is what
            // wakes it on the pass the landing is spent.
            let coming = landing.and_then(|asked| asked.read().clone());
            if let Some(row) = moving {
                *owing.borrow_mut() = Some(row);
            }

            // The reveal first, and the kept row only when it made none: either scroll is a
            // write this effect is subscribed to, so it wakes once more, finds the tab it is
            // holding is the tab it is showing, and writes the row down.
            let mut asked = latest.borrow_mut();
            if (asked.reveal)(&mut controller) {
                *owing.borrow_mut() = None;
                return;
            }
            // Then the landing that has not been spent yet, which the pane takes when it
            // names a row of what it is drawing. The row is where this pane is going, so it
            // goes there as it draws the document and not two passes later, when `use_land`
            // has turned the same row into a run.
            if let Some(asking) = &coming {
                if (asked.coming)(asking, &mut controller) {
                    *owing.borrow_mut() = None;
                    return;
                }
                // Not this pane's row: the move is held rather than made, since the pass that
                // spends the landing may still plant this pane a run -- a door that knew only
                // an address leaves the other pane one -- and going to the opening row first
                // would show the top of the listing on the way.
                return;
            }
            drop(asked);
            if let Some(row) = owing.borrow_mut().take() {
                controller.scroll_to_y(-((row as f32 * code_row_height()) as i32));
            }
        },
    );
}

/// Every box the keyboard can be in inside the tab on screen, and whether something has
/// asked for it to go there.
///
/// The boxes are a **registration** and not a flag written when one takes the focus: focus
/// is *lost* without an event -- something else asks for it -- so what is asked of the
/// platform has to be asked at the moment the answer is drawn.
#[derive(Default)]
pub(crate) struct Keys {
    boxes: Vec<AccessibilityId>,
    /// Set by a press on a chip and spent by [`use_keyboard_asked`], which is where a box
    /// exists to be focused: the tab pressed may only have mounted its panes in the render
    /// the press caused.
    wanted: bool,
}

impl Keys {
    /// The box a tab takes the keyboard into: the first registered, which is the pane the
    /// tab is driven from -- `DocumentBody` mounts the leading side first.
    fn first(&self) -> Option<AccessibilityId> {
        self.boxes.first().copied()
    }
}

/// Register `a11y` as one of the boxes the keyboard can be in inside the tab on screen,
/// for as long as this scope is mounted. See [`Keys`].
pub(crate) fn use_tab_keyboard(a11y: AccessibilityId) {
    let mut keyboard = use_consume::<Keyboard>().0;
    use_hook(move || keyboard.write().boxes.push(a11y));
    use_drop(move || {
        keyboard.write().boxes.retain(|open| *open != a11y);
    });
}

/// Whether the keyboard is inside the tab on screen. Asking is what subscribes the caller
/// to the focus moving, `AccessibilityId::is_focused` reading the platform's own state.
pub(crate) fn keyboard_in_tab(keyboard: State<Keys>) -> bool {
    keyboard.read().boxes.iter().any(|a11y| a11y.is_focused())
}

/// Ask for the keyboard to go into the tab on screen: what pressing a chip does, so that
/// reading follows the tab the reader just chose.
pub(crate) fn ask_for_keyboard(mut keyboard: State<Keys>) {
    keyboard.write().wanted = true;
}

/// Spend that ask, once the tab it was made for has mounted what it has. In an effect and
/// not in the press, because the press is what mounts the panes: the box to focus does not
/// exist until the render it caused has run.
pub(crate) fn use_keyboard_asked(mut keyboard: State<Keys>) {
    use_side_effect(move || {
        let asked = keyboard.read().wanted;
        if !asked {
            return;
        }
        // Bound in a statement of its own, so no read guard is alive at the write below.
        let box_to_focus = keyboard.peek().first();
        keyboard.write().wanted = false;
        if let Some(a11y) = box_to_focus {
            a11y.request_focus();
        }
    });
}

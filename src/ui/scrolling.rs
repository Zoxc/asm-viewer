//! Keeping each pane scrolled where its place was left, and bringing a row into view.
//!
//! [`use_kept_position`] is a code pane's: it writes down the row the pane is scrolled to
//! as the reader scrolls, and puts that row back as the tab comes round again. The two
//! reveals are wider. [`reveal_row`] brings a row into view with the rows kept above it;
//! [`reveal_caret`] moves only for a row off screen, which is the rule the finder's list
//! and a filter bar's follow as well as a code pane's, each in its own row height.
//!
//! The runs a place arrives with are `focus.rs`'s: `use_land` gives a place its runs as
//! this file gives it its scroll.

use super::*;

/// Bring the row at `index` of a listing of `length` rows into view, and leave the scroll
/// alone when it already is.
///
/// A `VirtualScrollView` counts its offset *down* from zero, and the controller can be
/// holding one past the end that the view corrects only as it draws ([`scroll_extent`]).
/// Hence `length`: the offset read back is held to it, and so is the one written, which
/// `row - margin` puts past the end for any row in the last screenful.
///
/// Answers whether the row could be positioned at all: [`false`] only for a pane that has
/// not been measured, where the caller must keep whatever it owes. A caller keeping one
/// has to have **read** its viewport rather than peeked it, or nothing wakes it when the
/// measurement arrives and what it kept is never paid.
///
/// **Already there is measured against the offset this would write**, and not against the
/// context rows on their own. A row in the first `CONTEXT_ROWS` of a listing cannot have
/// them all above it, so asking for them was asking for an offset above the top of the
/// list: the scroll went to 0, the next call measured it against the same impossible
/// margin, and found it wanting again. That is a write per call for ever, and the caller
/// that reads the scroll to make it is woken by it (`use_kept_position`).
pub(crate) fn reveal_row(
    controller: &mut ScrollController,
    viewport: f32,
    length: usize,
    index: usize,
) -> bool {
    // **Nothing is known before the pane has been laid out.** A viewport of zero is not a
    // pane with no room, it is a pane not measured yet -- its first pass, which is the one
    // a door arrives on -- and the clamp below would read it as a pane too short to hold
    // the row and its margin, putting the row flush against the top: the one answer the
    // margin exists to avoid. So nothing is done and `false` says so, for the caller to
    // keep what it owes rather than spend it on a guess. The measurement wakes the next
    // pass, which pays it properly.
    if viewport <= 0.0 {
        return false;
    }
    let height = code_row_height();
    // How far the listing goes, which both the offset read here and the one written
    // below are held to (`scroll_extent`).
    let extent = scroll_extent(length, height, viewport);
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -(scrolled as f32).clamp(-extent, 0.0);
    let row = index as f32 * height;
    let margin = CONTEXT_ROWS * height;
    // The context rows are what the caller wants, never what it asks for: a caller hands
    // over the row it means and the margin is applied here, once, so that no two callers
    // disagree about how much of the listing above a row is part of showing it.
    //
    // **Never so far that the row itself leaves the view.** The margin is what is wanted
    // above the row and the row is what was asked for, so a pane too short to hold both
    // gives up the margin and not the row: scrolled to `row - margin` regardless, a pane
    // two rows tall showed the two rows *before* the instruction a door had just opened
    // it on. It is also what makes the offset written here satisfy the test above on the
    // next call, in every viewport -- which is what keeps a caller that is woken by its
    // own scroll from asking again for ever (`notes/upstream/freya.md`); the slack below
    // is the other half of that.
    //
    // **And never past the end of the listing**, which `row - margin` is for any row in
    // the last screenful. `lowest` is already inside the extent, so the clamp only ever
    // takes the margin back.
    let lowest = row + height - viewport;
    let wanted = (row - margin).max(lowest).min(row).clamp(0.0, extent);

    // The pixel of slack is the rounding, and is what keeps the test satisfiable at the
    // end of a listing: an offset is a whole number of pixels where a listing of rows is
    // not, so a view clamped hard against its end stands a fraction of a pixel short of
    // showing its last row entire. Asked for strictly, that row is asked for again on
    // every call, for ever where anything repeats the ask.
    if top <= wanted && row + height <= top + viewport + 1.0 {
        return true;
    }

    controller.scroll_to_y(-(wanted as i32));
    true
}

/// Bring the row the keyboard is on into view, and only when it is not: no context rows,
/// unlike [`reveal_row`], since a key repeat that scrolled the view while the row was
/// still on screen would walk the rows away from under the reader; a row above the view
/// comes to its top, one below to its bottom, as an editor's does.
///
/// The row height is the caller's, this being the one rule the finder's list follows as
/// well as a code pane's, and the two are measured in different fonts; `length` is the
/// list's rows, which is what the offset read back is held to, for the reason
/// [`reveal_row`] gives.
pub(crate) fn reveal_caret(
    controller: &mut ScrollController,
    viewport: f32,
    height: f32,
    length: usize,
    index: usize,
) {
    let (_, scrolled) = <(i32, i32)>::from(*controller);
    let top = -(scrolled as f32).clamp(-scroll_extent(length, height, viewport), 0.0);
    let row = index as f32 * height;

    if row < top {
        controller.scroll_to_y(-(row.max(0.0) as i32));
    } else if row + height > top + viewport {
        controller.scroll_to_y(-((row + height - viewport).max(0.0) as i32));
    }
}

/// [`reveal_caret`] with what a code listing already knows bound in: the reveal its
/// keyboard and its find bar are both handed, built once by [`use_listing_keys`].
///
/// **`Copy`**, so one build serves both: every capture is one -- a scroll controller, a
/// state and a length -- and the second owner costs a copy rather than a second call with
/// the arguments written out again.
///
/// **What it reads, it reads when the reveal is made and not when it is built.**
/// `viewport` is kept as the state and peeked per call, since a pane measured after this
/// was built would otherwise be revealed against a viewport of zero; the row height is
/// asked for per call, being a function of the fonts. `length` alone is the render's own,
/// which is what a reveal is about: the rows the render that made it drew.
///
/// The height is a code row's, so this is the code panes' rule and not the rule itself.
/// The finder's list follows the same one in the interface font and calls
/// [`reveal_caret`] for itself.
pub(crate) fn caret_reveal(
    mut controller: ScrollController,
    viewport: State<f32>,
    length: usize,
) -> impl FnMut(usize) + Copy + 'static {
    move |row| {
        reveal_caret(
            &mut controller,
            *viewport.peek(),
            code_row_height(),
            length,
            row,
        )
    }
}

/// What a pane's [`use_kept_position`] asks of it every render: the row it owes a reveal
/// to, the row a landing names in it, and the row it opens at. Held so the effect reads
/// the latest and not the first.
///
/// Both answers are rows and not scrolls: the hook holds the controller, the viewport and
/// the length, so the scroll is made there, once, and a pane says only which row it
/// means. Generic over the two closures rather than boxing them, this being written
/// afresh on every render.
struct Latest<R, C> {
    reveal: R,
    coming: C,
    opening: Option<usize>,
}

/// What a run of [`use_kept_position`] owes the view.
///
/// The two are not the same ask and must not be served alike. A **place** is put back
/// exactly: it is where the reader left the tab, and a margin added to it would be added
/// again on every switch, walking the tab up the listing. An **open** is a request --
/// *this is the row the tab is about* -- and the rows kept above it are part of showing
/// it, which is this hook's to add and never the caller's to subtract before asking.
///
/// Both put a row at the top of the pane, which is what a first open wants and a reveal
/// would not give: [`reveal_row`] leaves a row that is on screen already where it is, so
/// a symbol ten lines into a file would open at the top of the file rather than on itself.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Move {
    Place(usize),
    Open(usize),
}

/// **Whose row the pane's current offset is, and where this run has to move the view
/// to**: the decision [`use_kept_position`] makes once it knows what the controller is
/// holding, apart from the effect that acts on it.
///
/// `holding` is the tab the controller is scrolled for, [`None`] before this pane has run
/// at all; `tab` is the tab it is showing now. `known` says whether a row is remembered
/// for `tab` and `back_to` is that row clamped to the listing's length. `opening` is the
/// row a tab nothing is remembered for opens at, already clamped, and [`None`] for a pane
/// with nothing better to say than the top.
///
/// The owner is the tab the offset belongs to, which the caller writes the row down
/// under. It is the tab being **left** for the one run that switches: the offset on
/// screen is still that tab's.
fn kept_move(
    holding: Option<&Entry>,
    tab: &Entry,
    known: bool,
    back_to: usize,
    opening: Option<usize>,
) -> (Option<Entry>, Option<Move>) {
    match (holding, known) {
        // Still showing the tab the controller is scrolled for: nothing moves.
        (Some(held), _) if held == tab => (Some(tab.clone()), None),
        // A switch: the offset belongs to the tab being left, and the one arriving goes
        // back to where it was, or to where a tab seen for the first time opens. A `0`
        // moves here, where the first-run arm below leaves one alone: the offset on screen
        // is the tab being left, and the arriving one must not inherit it.
        (Some(out), true) => (Some(out.clone()), Some(Move::Place(back_to))),
        (Some(out), false) => (
            Some(out.clone()),
            // The top where the arriving tab has no row of its own to open at: a place
            // and not a reveal, since what must not survive the switch is the *outgoing*
            // tab's offset, and a reveal of the top would leave a small one where it can
            // already see row 0.
            Some(opening.map_or(Move::Place(0), Move::Open)),
        ),
        // This pane's first run, on a tab it has a row for: a remount or a restored
        // session. Nothing to write down, everything to put back.
        (None, true) => (None, Some(Move::Place(back_to))),
        // First run with nothing remembered -- which, both panes being mounted afresh for
        // every document, is the ordinary first open of a tab. It moves only for a pane
        // that has somewhere to open at: a `0` is left alone rather than scrolled to,
        // since this runs a beat after the first render and setting the offset it already
        // has would undo a wheel that got in.
        (None, false) => (Some(tab.clone()), opening.map(Move::Open)),
    }
}

/// Keep `controller` pointed at the row `tab` was last left at, and keep [`Positions`]
/// told where it is now. `length` is what the pane holds *now*, which is what makes the
/// answer a row of this listing rather than of the one it was saved from.
///
/// `opening` is the row a tab **nothing is remembered for** opens at -- the Source pane's
/// symbol's own line -- and [`None`] for a pane or a symbol with nothing better to say
/// than the top. It is the row itself and never a row backed off towards the top: the
/// rows kept above it are part of showing it, and applying them is this hook's, through
/// [`reveal_row`]. A row remembered for the tab always wins over it: it is the first open
/// this answers and not every one.
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
/// And the pane's reveal is made **here**, rather than by an effect of its own: `reveal`
/// answers the row this pane owes a scroll to, the scroll is made here, and a scroll made
/// is where the arriving tab goes instead of back to its row. The two are owed at once
/// when a row in the Locations panel opens a symbol on a line, and two effects' scrolls
/// land in whichever order the runtime wakes them -- with the reveal first, it had marked
/// itself made by the time the kept row was put over it. One effect has one order.
/// `reveal` reads the marks, which is what wakes this on a click inside a tab, so it is
/// asked before anything here can return.
///
/// A reveal the pane could be scrolled to is then said to be made, for `pane`, in the
/// marks the [`Marked`] context holds -- the state both panes pick their runs out in.
/// Said here and not by the caller, so a reveal is marked made exactly when the scroll
/// was: marked before it, one owed to a pane that cannot scroll yet is answered and
/// nothing is left to correct it ([`reveal_row`]).
///
/// **A tab arriving with a landing on its way goes to the row the landing names as it
/// draws it, and holds the move it would otherwise make until the landing is spent.**
/// `use_land` turns a landing into a run two passes after the switch reaches here -- it
/// runs off `Active`, which is a memo -- so a pane left to its own devices drew the
/// arriving document at the outgoing place's offset until then, and a pane that made its
/// move first drew it at the top of the file. `coming` is asked for the row the landing
/// names in what this pane is drawing, revealed with the same `reveal_row` the run makes
/// later, so the run finds the row already on screen and moves nothing. Only a row the
/// pane could be scrolled to counts as taken: a landing is gone to once, so one taken by
/// a pane with no measurement yet would be remembered as answered and never made good. A
/// landing it does not take -- a door that knew only an address, or one meant for the
/// other pane -- leaves the move held rather than made, since that pass may still plant
/// this pane a run. Nothing is stranded by a landing that never lands: one is only ever
/// left by a move that changes the place, and that arrival is what spends it.
///
/// `reveal` and `opening` are the **latest render's**, kept in a cell for the effect to
/// take. A tab handed another document is not mounted again -- a link followed in place,
/// a search hit shown in the temporal tab -- so a reveal held from the mount would go on
/// measuring the row it owes against the file the pane drew then, refuse it, and leave
/// the pane to fall back to the top.
///
/// `viewport` is how tall the pane is, which is all the row arithmetic here needs of it:
/// how far the pane can be scrolled, so neither the row taken from the offset nor the
/// offset written for a row is a place the view could not be at (`reveal_row`). It is
/// **read**, so the first measurement wakes this: a restore made before it landed is the
/// one write here that could not be held to an extent, and the run the measurement wakes
/// is what puts the pane back inside the listing.
///
/// `listing` is the key of what the pane is drawing (`Widest::key`), a dep and nothing
/// else. The effect runs when a dep differs or a state it read is written, never because
/// the pane rendered; without the key, an answer arriving at the same tab with the same
/// row count -- two accessors, two monomorphisations of one generic -- leaves a reveal
/// owed until the next click or wheel, when the point of leaving it owed is that the
/// listing which can answer it finds it.
///
/// `docs` says whether the entry a row is about to be written down for is still on an
/// open tab's trail: the run after a close is still holding the tab, and writing its row
/// down would put it straight back. **Asked of [`Docs`] itself, never of a [`Memo`] over
/// it**, which can still be reporting a just-closed tab as open during exactly that run.
/// The state and not a closure over it, so that rule is stated once.
pub(crate) fn use_kept_position(
    mut positions: State<Positions<Entry>>,
    docs: State<Docs>,
    pane: Pane,
    reveal: impl FnMut() -> Option<usize> + 'static,
    coming: impl FnMut(&Landing) -> Option<usize> + 'static,
    mut controller: ScrollController,
    viewport: State<f32>,
    tab: &Entry,
    length: usize,
    listing: u64,
    opening: Option<usize>,
) {
    // Which tab the controller is scrolled for. An `Rc<RefCell>` and not a `State`:
    // nothing renders from it, and a state would cost the pane a render per switch.
    let held = use_hook(|| Rc::new(RefCell::new(None::<Entry>)));

    // The two answers and the opening row as this render made them. The effect below is
    // handed fresh deps, but its callback is built once in a `use_hook`, so a value passed
    // to it by hand would stay the first render's. `None` only before the first render has
    // written it, which no run of the effect comes before.
    let latest = use_hook(|| Rc::new(RefCell::new(None)));
    *latest.borrow_mut() = Some(Latest {
        reveal,
        coming,
        opening,
    });

    // The move this hook owes the view and has not made. An `Rc<RefCell>` for the same
    // reason as the tab above.
    let owing = use_hook(|| Rc::new(RefCell::new(None::<Move>)));
    let answered = use_hook(|| Rc::new(RefCell::new(None::<Landing>)));
    // A landing on its way, whichever document it names. Asked through
    // `try_consume_context`, a pane mounted without the landing machinery having none on
    // its way.
    let landing = try_consume_context::<Doors>().map(|doors| doors.land);
    // Where a reveal made is said to be made, asked for the same way: a list that keeps a
    // position without the panes' marks is owed no reveal to answer.
    let marked = try_consume_context::<Marked>().map(|marked| marked.0);
    // The landing this pane has already gone to, held exactly as long as that landing is
    // on its way. **The pane does not spend the landing** -- `use_land` does, a pass or
    // more later -- so without this the reveal below is made again on every wake, and the
    // scroll a reveal makes is a wake (`notes/upstream/freya.md`): a write per pass, for
    // ever where the reveal cannot satisfy itself, which is any viewport too short to
    // hold the row and its context rows. Where it can, the loop is invisible until the
    // reader scrolls, and is then a pane that will not stay where they put it.

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would hold the first tab this pane ever showed.
    use_side_effect_with_deps(
        &(tab.clone(), length, listing),
        move |(tab, length, _): &(Entry, usize, u64)| {
            // Subscribes this effect to the pane's scroll, so it comes before any return.
            let (_, offset) = <(i32, i32)>::from(controller);
            let height = code_row_height();
            // How far the pane can be scrolled, which every offset here is held to: a
            // row taken from an uncorrected one is a row the reader is not looking at
            // (`scroll_extent`). **Read and not peeked**, so the first measurement wakes
            // this: until it lands there is no extent to speak of -- that of an
            // unmeasured pane is the whole listing -- and the move this run makes is the
            // one write here that could not be held to one.
            let seen = *viewport.read();
            let extent = scroll_extent(*length, height, seen);
            // Which puts the pane back inside the listing where something has left it
            // past the end -- the restore below made before the pane was measured,
            // freya's own End key, a listing that has grown shorter. The view has drawn
            // the corrected offset all along, so this moves nothing on screen: it makes
            // the number the controller holds the one the rows are at. Written once, the
            // run it wakes finding the offset inside and nothing to do.
            let inside = (offset as f32).clamp(-extent, 0.0) as i32;
            if seen > 0.0 && inside != offset {
                controller.scroll_to_y(inside);
            }
            // The row at the top of the pane. `code_row_height` and not the list's, this
            // being a code pane; rounded down, so a row half on screen is the row the reader
            // is looking at.
            let row = ((-inside).max(0) as f32 / height) as usize;

            // Cloned out of the borrow rather than held across the `borrow_mut` below.
            let holding = held.borrow().clone();
            let switching = holding.as_ref() != Some(tab);
            let known = positions.peek().at(tab).is_some();
            let back_to = positions.peek().row(tab, *length);
            // Clamped the way a remembered row is, and for the same reason: a symbol's line
            // is a hint out of debug info and the file under it may have been cut short since.
            let opening = latest
                .borrow()
                .as_ref()
                .and_then(|asked| asked.opening)
                .map(|row| row.min(length.saturating_sub(1)));

            // Whose row the offset above is, and where this run has to move the view to.
            let (owner, moving) = kept_move(holding.as_ref(), tab, known, back_to, opening);

            if let Some(owner) = owner {
                // Only for a place still on an open tab's trail, for the reason the doc
                // comment gives.
                let (id, stop) = &owner;
                let still_open = docs.peek().contains(*id, stop);
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
            // Forgotten with the landing it is about, so the same door pressed twice is
            // answered twice.
            if coming.is_none() {
                *answered.borrow_mut() = None;
            }
            if let Some(row) = moving {
                *owing.borrow_mut() = Some(row);
            }

            // The reveal first, and the kept row only when it made none: either scroll is a
            // write this effect is subscribed to, so it wakes once more, finds the tab it is
            // holding is the tab it is showing, and writes the row down. The row is the
            // pane's to say and the scroll this run's to make, over the viewport read
            // above and the length these rows are of.
            let mut asked = latest.borrow_mut();
            let owed = asked.as_mut().and_then(|asked| (asked.reveal)());
            if let Some(row) = owed {
                if reveal_row(&mut controller, seen, *length, row) {
                    // Only now: a reveal the pane could not be scrolled to is left owed,
                    // for the run the measurement wakes to pay.
                    if let Some(marked) = marked {
                        reveal_made(marked, pane);
                    }
                    *owing.borrow_mut() = None;
                    return;
                }
            }
            // Then the landing that has not been spent yet, which the pane takes when it
            // names a row of what it is drawing. The row is where this pane is going, so it
            // goes there as it draws the document and not two passes later, when `use_land`
            // has turned the same row into a run.
            if let Some(asking) = &coming {
                // Bound to a `let` of its own: the borrow must be over before the write.
                let gone = answered.borrow().as_ref() == Some(asking);
                let taken = !gone
                    && asked
                        .as_mut()
                        .and_then(|asked| (asked.coming)(asking))
                        .is_some_and(|row| reveal_row(&mut controller, seen, *length, row));
                if taken {
                    *answered.borrow_mut() = Some(asking.clone());
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
            // The margin is taken here and not by the caller, which had to know how much
            // of the listing above a row is part of showing it -- and could not say a row
            // inside the margin at all, that coming out as 0 and reading as nothing to do.
            let top = match owing.borrow_mut().take() {
                Some(Move::Place(row)) => Some(row),
                Some(Move::Open(row)) => Some(row.saturating_sub(CONTEXT_ROWS as usize)),
                None => None,
            };
            if let Some(row) = top {
                controller.scroll_to_y(-((row as f32 * height).min(extent) as i32));
            }
        },
    );
}

#[cfg(test)]
mod tests;

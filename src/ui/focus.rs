//! A place in a file, the two panes named as a pair, the landing a click from outside
//! them makes, what each tab keeps of where it was left and of its runs, and the effects
//! that spend a landing.
//!
//! What the two panes say to each other is in `marks.rs`: each pane's picked-out run is
//! what the other pane lights the pair of, and owes a scroll to. Arriving somewhere is
//! this file's: `use_kept_position` puts each pane's scroll back and `use_land` its run,
//! and `use_clear_marks` drops a run whose listing has been replaced.

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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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

/// What a place keeps of its two runs while its tab shows something else, or another tab
/// is on top: the runs as they were, by rows -- a symbol's listing and a file have the
/// same rows every time they are drawn -- and, for an object's code, whose rows are
/// counted afresh every time the tab is shown, the place each row of the assembly run
/// stood for in the rows it was picked out of, stamped with the reading generation those
/// rows were counted at. `use_land` writes the runs as a place is left and puts them back
/// as it arrives; the section view writes the places as its run changes and carries the
/// run through them when its rows are new ([`Kept::carry`]).
#[derive(Clone, PartialEq, Default)]
pub(crate) struct Kept {
    pub(crate) marks: Marks,
    /// For each row the assembly run holds -- its two ends -- the place it stood for.
    /// Empty except in an object's code.
    pub(crate) spots: Vec<(usize, Spot)>,
    /// The reading generation `spots` were taken against, under which the run's rows
    /// are still its rows.
    pub(crate) generation: Option<u64>,
}

impl Kept {
    /// The place of each end of `picked`, through `spot_at`; an end with no place is
    /// left out, and the carry drops a run either end of which is missing.
    pub(crate) fn spots_of(
        picked: Option<&Picked>,
        spot_at: impl Fn(usize) -> Option<Spot>,
    ) -> Vec<(usize, Spot)> {
        let Some(picked) = picked else {
            return Vec::new();
        };
        let (first, last) = picked.chars.ends();
        let mut rows = vec![first.row, last.row];
        rows.dedup();
        rows.into_iter()
            .filter_map(|row| Some((row, spot_at(row)?)))
            .collect()
    }

    /// The place kept for `row`, if one was.
    pub(crate) fn spot_of(&self, row: usize) -> Option<Spot> {
        self.spots
            .iter()
            .find(|(kept, _)| *kept == row)
            .map(|(_, spot)| *spot)
    }

    /// The assembly run carried to rows counted afresh: each end of it put through the
    /// place kept for it and `row_of`, which answers the row that place has now. `None`
    /// for no run, and for a run either end of which has no place or no row any more.
    pub(crate) fn carry(&self, row_of: impl Fn(Spot) -> Option<usize>) -> Option<Picked> {
        let picked = self.marks.assembly.as_ref()?;
        carried(picked, |row| row_of(self.spot_of(row)?))
    }
}

/// What a door out of one place into another is given, in one `Copy` bundle: where things
/// are open, the record of visits, the runs the two panes have picked out, and the two
/// halves of a landing left for the arrival.
///
/// Not an incidental grouping. [`documents::land`] is the one path a door takes, and every
/// door passes it these five -- so a sixth thing a landing needs is a field here and
/// nothing at a call site. Provided once by `app()` and taken in one [`use_doors`], which
/// is why a door's handler is the landing it is about and not six lines of preamble.
///
/// A bundle does not own its handles: `marked` is the state [`Marked`] hands the panes,
/// and `open` the [`Open`] [`ProjectStates`] carries.
#[derive(Clone, Copy)]
pub(crate) struct Doors {
    pub(crate) open: Open,
    pub(crate) visits: State<Visits>,
    pub(crate) marked: State<Marks>,
    /// The landing asked for. `None` almost always: it is set in the handler that opens a
    /// document and spent by the change of document that follows.
    pub(crate) land: State<Option<Landing>>,
    /// The caret still to be planted, `None` almost always.
    pub(crate) plant: State<Option<Planting>>,
}

/// What a door needs, as a component sees it.
pub(crate) fn use_doors() -> Doors {
    use_consume::<Doors>()
}

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

/// What a pane's [`use_kept_position`] asks of it every render: the reveal it owes, the
/// landing it can take, and the row it opens at. Held so the effect reads the latest and
/// not the first.
struct Latest {
    reveal: Box<dyn FnMut(&mut ScrollController) -> bool>,
    coming: Box<dyn FnMut(&Landing, &mut ScrollController) -> bool>,
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
#[derive(Clone, Copy)]
enum Move {
    Place(usize),
    Open(usize),
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
    reveal: impl FnMut(&mut ScrollController) -> bool + 'static,
    coming: impl FnMut(&Landing, &mut ScrollController) -> bool + 'static,
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

    // The reveal and the opening row as this render made them. The effect below is handed
    // fresh deps, but its callback is built once in a `use_hook`, so a value passed to it
    // by hand would stay the first render's. The one the hook makes is never read: every
    // render writes over it before the effect can run.
    let latest = use_hook(|| {
        Rc::new(RefCell::new(Latest {
            reveal: Box::new(|_| false),
            coming: Box::new(|_, _| false),
            opening: None,
        }))
    });
    *latest.borrow_mut() = Latest {
        reveal: Box::new(reveal),
        coming: Box::new(coming),
        opening,
    };

    // The move this hook owes the view and has not made. An `Rc<RefCell>` for the same
    // reason as the tab above.
    let owing = use_hook(|| Rc::new(RefCell::new(None::<Move>)));
    let answered = use_hook(|| Rc::new(RefCell::new(None::<Landing>)));
    // A landing on its way, whichever document it names. Asked through
    // `try_consume_context`, a pane mounted without the landing machinery having none on
    // its way.
    let landing = try_consume_context::<Doors>().map(|doors| doors.land);
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
            let known = positions.peek().at(tab);
            let back_to = positions.peek().row(tab, *length);
            // Clamped the way a remembered row is, and for the same reason: a symbol's line
            // is a hint out of debug info and the file under it may have been cut short since.
            let opening = latest
                .borrow()
                .opening
                .map(|row| row.min(length.saturating_sub(1)));

            // Whose row the offset above is, and where this run has to move the view to.
            let (owner, moving) = match (&holding, known) {
                // Still showing the tab the controller is scrolled for: nothing moves.
                (Some(held), _) if held == tab => (Some(tab.clone()), None),
                // A switch: the offset belongs to the tab being left, and the one arriving
                // goes back to where it was, or to where a tab seen for the first time opens.
                // A `0` moves here, where the first run below leaves one alone: the offset on
                // screen is the tab being left, and the arriving one must not inherit it.
                (Some(out), Some(_)) => (Some(out.clone()), Some(Move::Place(back_to))),
                (Some(out), None) => (
                    Some(out.clone()),
                    // The top where the arriving tab has no row of its own to open at:
                    // a place and not a reveal, since what must not survive the switch
                    // is the *outgoing* tab's offset, and a reveal of the top would
                    // leave a small one where it can already see row 0.
                    Some(opening.map_or(Move::Place(0), Move::Open)),
                ),
                // This pane's first run, on a tab it has a row for: a remount or a restored
                // session. Nothing to write down, everything to put back.
                (None, Some(_)) => (None, Some(Move::Place(back_to))),
                // First run with nothing remembered -- which, both panes being mounted afresh
                // for every document, is the ordinary first open of a tab. It moves only for
                // a pane that has somewhere to open at: a `0` is left alone rather than
                // scrolled to, since this runs a beat after the first render and setting the
                // offset it already has would undo a wheel that got in.
                (None, None) => (Some(tab.clone()), opening.map(Move::Open)),
            };

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
                // Bound to a `let` of its own: the borrow must be over before the write.
                let gone = answered.borrow().as_ref() == Some(asking);
                if !gone && (asked.coming)(asking, &mut controller) {
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

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when another question is asked, the source pane's when the pane moves
/// off the run's file. An object's code being counted afresh under its run is **not** a
/// replacement: the run is carried to the rows it now has ([`carry_assembly`], from the
/// section view's own rebuild), since the place it marks has an address and the rows do
/// not.
///
/// At the root and keyed on the states that say *which listing*, **never on the listings
/// themselves**: `AsmData` carries an `Arc<Lanes>` rebuilt every render, so an effect
/// inside each list would fire on every render and wipe the run the press just started.
///
/// **Neither run is dropped here on a change of the active entry.** A switch of place is
/// [`use_land`]'s: it saves the runs of the place being left and puts back the arriving
/// place's own, and it does so in an effect woken by the same change as these two, in
/// no order anyone can rely on -- a drop made here for the switch could land after the
/// restore and take the restored run with it. So each effect keeps the entry it last ran
/// for and, when the entry has changed, records the new one and does nothing else; what
/// it drops is a listing replaced *within* one place.
pub(crate) fn use_clear_marks(
    active: Memo<Option<Entry>>,
    asked: Asked,
    analysis: State<Analyzed>,
    marked: State<Marks>,
) {
    // The **question** and not the active document: a source-driven tab's listing is
    // replaced when a line in it is clicked, which changes no document, and a run picked
    // out of the last line's function would survive into the next one's as raw row
    // indices. The entry and the question this last ran for, in an `Rc<RefCell>` and not
    // a `State`: nothing renders from them.
    let asked_for = use_hook(|| Rc::new(RefCell::new(None::<(Option<Entry>, Option<Ask>)>)));
    use_side_effect(move || {
        let ask = asked.read_ask();
        let entry = active.peek().clone();
        // Cloned out of the borrow before the `borrow_mut`.
        let was = asked_for.borrow().clone();
        *asked_for.borrow_mut() = Some((entry.clone(), ask.clone()));
        let Some((was_entry, was_ask)) = was else {
            return;
        };
        // The same place asking another question: the listing under the run is going.
        if was_entry == entry && was_ask != ask {
            unmark(marked, Pane::Assembly);
        }
    });
    // Which entry, and which file the Source pane was drawing, the last time this ran.
    let showing = use_hook(|| Rc::new(RefCell::new((None::<Entry>, None::<Arc<str>>))));
    use_side_effect(move || {
        // The *file the Source pane is drawing*, which is not the active document: two
        // functions from one file leave the same lines on screen. Compared against what
        // it last was rather than answered to directly, since reading the analysis
        // subscribes this to writes -- a request, the slow flag -- that change no listing.
        let active = active.read().clone();
        let document = active.as_ref().map(|(_, stop)| &stop.document);
        let file =
            source_side(document, &analysis.read(), &marked.read()).map(|side| side.file().clone());
        // Cloned out of the borrow before the `borrow_mut`.
        let (was_entry, was) = showing.borrow().clone();
        let switched = was_entry != active;
        *showing.borrow_mut() = (active, file.clone());
        if switched || was == file {
            return;
        }

        // Dropped only when the pane moves **off the run's file**, and not whenever the
        // file changes: a run a landing plants is in the file the pane is about to show,
        // and the switch it causes -- from the listing being left to the one arriving --
        // must not be what drops it. A run in a file the pane never reaches stays,
        // undrawn, until the next question replaces it.
        let picked = marked
            .peek()
            .source
            .as_ref()
            .and_then(|picked| picked.file.clone());
        if picked.is_some() && was == picked {
            unmark(marked, Pane::Source);
        }
    });
}

/// One run of [`use_land`] as its stages share it: the switch it is answering, and what
/// the stages before have found.
struct Step {
    /// The place arriving, whose runs are being put back.
    active: Option<Entry>,
    /// The place being left, whose runs are kept under it.
    leaving: Option<Entry>,
    /// The source pane's run as this woke, before anything the run writes. A door onto
    /// the document already on top marks its line itself and leaves no landing, so what
    /// it picked out is already there when the new place wakes this (`documents::land`).
    standing: Option<Picked>,
    /// The landing this arrival is to spend, where a door left one for it.
    landed: Option<Landing>,
    /// The runs the arriving place kept, looked up only where no landing won.
    kept: Option<Kept>,
}

/// Give each place its own runs: whenever the active entry changes, keep the runs of the
/// place being left under its entry ([`Places::marks_at`]) and put the arriving place's own back
/// in both panes, the way its scroll rows come back -- the caret and the selection the
/// reader left in each. A place that has never been shown has nothing kept, and gets
/// what an arrival always got: a [`Landing`] naming this document, picked out in the
/// source pane with both panes owed the scroll; or, for a source-driven tab, the line it
/// is driven from, with none owed, so coming back to a tab whose assembly side is a
/// listing of one line shows which line and why.
///
/// Three rules settle what wins. **A landing wins over what was kept**, in both panes
/// ([`take_kept`]). **A kept run wins over the driven line**, being the more specific
/// ([`source_run`]). **A restored run owes no scroll** ([`keep_leaving`]).
///
/// **One effect, because the order is the whole of it**: detect the switch, keep the runs
/// of the place being left ([`keep_leaving`]), decide whose landing this is and hand its
/// instruction on ([`take_landing`]), look up what the arriving place kept
/// ([`take_kept`]), and write the two runs ([`source_run`], [`assembly_run`]). Each stage
/// is a function over the [`Step`] they share -- what one stage tells the next is a field
/// of it -- and the rule a stage keeps is written on the stage.
pub(crate) fn use_land(
    doors: Doors,
    places: Places,
    active: Memo<Option<Entry>>,
    code_rows: State<Option<Arc<Built>>>,
) {
    let Doors {
        open,
        mut marked,
        land: landing,
        plant,
        ..
    } = doors;
    let (driven, marks_at) = (places.driven, places.marks_at);
    // The entry the runs on screen belong to. An `Rc<RefCell>` and not a `State`:
    // nothing renders from it.
    let showing = use_hook(|| Rc::new(RefCell::new(None::<Entry>)));

    use_side_effect(move || {
        // Subscribes the effect to the active document, which is all it wants from it;
        // the landing is peeked, so setting one wakes nothing until the document does.
        let active = active.read().clone();

        // Cloned out of the borrow before the `borrow_mut`.
        let leaving = showing.borrow().clone();
        if leaving == active {
            return;
        }
        *showing.borrow_mut() = active.clone();

        let mut step = Step {
            active,
            leaving,
            standing: marked.peek().source.clone(),
            landed: None,
            kept: None,
        };

        keep_leaving(&step, open, marked, marks_at);
        take_landing(&mut step, open, landing, plant);
        take_kept(&mut step, marks_at);

        let marks = Marks {
            assembly: assembly_run(&step, code_rows),
            source: source_run(&step, driven),
        };
        marked.set_if_modified(marks);
    });
}

/// Keep the runs of the place being left under its entry ([`Places::marks_at`]).
///
/// **Settled** ([`Marks::settled`]): no gesture under way and **no scroll owed**, since
/// the kept scroll rows are what put each side back when the place is shown again and a
/// reveal beside them would fight them.
///
/// Kept here, on the way out, and not on every change of [`Marks`]: a sweep writes on
/// every pointer move, and the entry those writes belong to is a memo a beat behind them.
/// So the entry being left is the one held in the hook, as `use_kept_position` holds its
/// tab -- and only while it is still on its trail, since the run after a close is still
/// holding the place that has gone and would put its binary straight back.
///
/// Written only where the runs changed: `State::write` notifies whether or not the value
/// does. A place left with nothing picked out and nothing kept gets no entry at all: it
/// comes back as a place never seen does, and a restored session walks through every tab
/// it reopens.
fn keep_leaving(
    step: &Step,
    open: Open,
    marked: State<Marks>,
    mut marks_at: State<Positions<Entry, Kept>>,
) {
    let left = step
        .leaving
        .as_ref()
        .filter(|(tab, stop)| open.docs.peek().contains(*tab, stop));
    let Some(entry) = left else {
        return;
    };
    let was = marks_at.peek().at(entry);
    let kept = Kept {
        marks: marked.peek().settled(),
        ..was.clone().unwrap_or_default()
    };
    let unseen = was.is_none() && kept == Kept::default();
    if !unseen && was.as_ref() != Some(&kept) {
        marks_at.write().remember(entry.clone(), kept);
    }
}

/// Whose landing this is: a door leaves one for the arrival of the document it names,
/// which is not always the arrival this run is answering. The landing that names this one
/// becomes `step.landed`; every other is spent, one left lying being a line picked out in
/// a document opened for some other reason later.
///
/// The exception is a landing still **waiting** for its own arrival, which is what the
/// live tables say and not this run's arrival: two arrivals can fall between two runs of
/// the effect -- a door pressed while the one before it is still settling -- and a landing
/// left for the second, spent here on the first, would leave the door that made it
/// planting nothing at all.
///
/// **A landing's instruction is planted later than its line.** The line is a row of a
/// file, which has the same rows every time; the instruction is a row of a listing that
/// arrives after the document -- a symbol's from the worker, an object's code's as the
/// skeleton comes and again as the stretch decodes. So the address is handed on as a
/// [`Planting`] naming the document, for the listing that draws it to spend
/// (`use_kept_place`, `InstructionList`). A planting is written on every arrival, `None`
/// included, so a listing that never came leaves no caret for the next one that does.
fn take_landing(
    step: &mut Step,
    open: Open,
    mut landing: State<Option<Landing>>,
    mut plant: State<Option<Planting>>,
) {
    let asked = landing.peek().clone();
    let names_this = asked.as_ref().is_some_and(|asked| {
        Some(&asked.tab) == step.active.as_ref().map(|(_, stop)| &stop.document)
    });
    let waiting = asked.as_ref().is_some_and(|asked| {
        !names_this
            && active_document(&open.strip.peek(), &open.docs.peek()).as_ref() == Some(&asked.tab)
    });
    if asked.is_some() && !waiting {
        landing.set(None);
    }
    step.landed = asked.filter(|_| names_this);
    let planting = step.landed.as_ref().and_then(|landing| {
        Some(Planting {
            tab: landing.tab.clone(),
            address: landing.address?,
        })
    });
    plant.set_if_modified(planting);
}

/// What the arriving place kept, for both panes to be put back from.
///
/// **A landing wins over what was kept**: a click from outside named a line, and the run
/// it makes is the only run, or the assembly pane would light its old run beside the pair
/// of the new. So nothing is looked up behind one, and at most one of the two fields is
/// ever set.
fn take_kept(step: &mut Step, marks_at: State<Positions<Entry, Kept>>) {
    if step.landed.is_some() {
        return;
    }
    let Some(entry) = step.active.as_ref() else {
        return;
    };
    step.kept = marks_at.peek().at(entry);
}

/// The run the source pane arrives with: the landing's line, then the kept run, then the
/// line a source-driven tab is driven from.
///
/// The landing's line is picked out with both panes owed the scroll: the reader asked to
/// be taken there. **A kept run wins over the driven line**, being the more specific, and
/// the driven line is planted where the kept source run is none -- the ask is the run. It
/// is the fallback for a landing that named no line as well, the door an unnamed call's
/// target opens knowing an address and no line.
fn source_run(step: &Step, driven: State<Driven>) -> Option<Picked> {
    let asked = match (&step.landed, &step.kept) {
        (Some(landing), _) => landing
            .at
            .clone()
            .map(|at| line_pick(at.file, at.line, landing.columns.clone(), Owed::BOTH)),
        (None, Some(kept)) => kept.marks.source.clone(),
        (None, None) => None,
    };
    if asked.is_some() {
        return asked;
    }
    let Some(
        entry @ (
            _,
            Stop {
                document: Document::Source(file),
                ..
            },
        ),
    ) = &step.active
    else {
        return None;
    };
    let line = driven.peek().line(entry).or(entry.1.line())?;
    // A run already on that very row was put there by a door with more to say than the
    // line -- a column, or a run of the row -- and a line is the whole of what this knows.
    // Keeping it is what leaves the caret on the name a followed call was defined under,
    // the door onto the file already on top having marked it before this place woke the
    // effect.
    Some(
        step.standing
            .clone()
            .filter(|picked| picked.is_line(file, line))
            .unwrap_or_else(|| line_pick(file.clone(), line, None, Owed::default())),
    )
}

/// The run the assembly pane arrives with: the kept one, and none where a landing won --
/// the landing's instruction is planted later than its line, so the kept assembly run is
/// left out as the kept source run is ([`take_landing`]).
///
/// **An object's code is the one listing whose rows are not its rows next time**: the
/// reading is reset when the tab is left, and comes back as guesses. Its kept run is
/// carried through the places the section view kept for it ([`Kept::carry`]) -- here, when
/// the rows on screen are already that object's at another generation (a second tab on the
/// same code), and otherwise by the section view itself when it first builds rows again,
/// which is after this has run; until then the pane's run is none, never a run of rows
/// that are gone.
fn assembly_run(step: &Step, code_rows: State<Option<Arc<Built>>>) -> Option<Picked> {
    let kept = step.kept.as_ref()?;
    let Some((
        _,
        Stop {
            document: Document::Code(object),
            ..
        },
    )) = &step.active
    else {
        return kept.marks.assembly.clone();
    };
    let built = code_rows.peek().clone()?;
    if !built.reading.is_about(object) {
        return None;
    }
    if kept.generation == Some(built.reading.generation) {
        kept.marks.assembly.clone()
    } else {
        kept.carry(|spot| row_of(&built, spot))
    }
}

/// Every box the keyboard can be in inside the tab on screen, and whether something has
/// asked for it to go there.
///
/// The boxes are a **registration** and not a flag written when one takes the focus: focus
/// is *lost* without an event -- something else asks for it -- so what is asked of the
/// platform has to be asked at the moment the answer is drawn.
#[derive(Default)]
pub(crate) struct Keys {
    /// Every box the keyboard can be in inside the tab on screen, and which pane each is:
    /// [`None`] for the scratchpad's editor, which is a page and not a pane.
    boxes: Vec<(Option<Pane>, AccessibilityId)>,
    /// Asked for by a press on a chip or on a row that opened a tab, and spent by
    /// [`use_keyboard_asked`] once there is a box to spend it on -- which may be several
    /// renders later, a pane with nothing to draw yet registering none.
    wanted: bool,
}

impl Keys {
    /// The box a tab takes the keyboard into: the first registered, which is the pane the
    /// tab is driven from -- `DocumentBody` mounts the leading side first.
    /// The box an ask should be spent on: the **leading pane's**, where the tab on screen
    /// has one drawn, and otherwise whatever box there is.
    ///
    /// By the pane and not by the order they registered in: a pane keeps its box for as
    /// long as it is mounted and the temporal tab's panes outlive the documents they draw,
    /// so the first box registered is the pane whose side led whichever tab opened first --
    /// which is how a reader who opened a file and then a symbol had the keyboard put in
    /// the file beside the listing they had just asked for.
    fn wanted_box(&self, leads: Option<Pane>) -> Option<(Option<Pane>, AccessibilityId)> {
        leads
            .and_then(|pane| self.boxes.iter().find(|(of, _)| *of == Some(pane)))
            .or_else(|| self.boxes.first())
            .copied()
    }
}

/// Register `a11y` as one of the boxes the keyboard can be in inside the tab on screen,
/// for as long as this scope is mounted, as the box of `pane` -- [`None`] for a box that
/// is not a pane's. See [`Keys`].
pub(crate) fn use_tab_keyboard(pane: Option<Pane>, a11y: AccessibilityId) {
    let mut keyboard = use_consume::<Keyboard>().0;
    use_hook(move || keyboard.write().boxes.push((pane, a11y)));
    use_drop(move || {
        keyboard.write().boxes.retain(|(_, open)| *open != a11y);
    });
}

/// The focusable box `pane` registered, where it has one mounted: what puts the keyboard
/// back in the code after a find bar over it is closed.
pub(crate) fn pane_box(keyboard: State<Keys>, pane: Pane) -> Option<AccessibilityId> {
    keyboard
        .peek()
        .boxes
        .iter()
        .find(|(of, _)| *of == Some(pane))
        .map(|(_, a11y)| *a11y)
}

/// Whether the keyboard is inside the tab on screen. Asking is what subscribes the caller
/// to the focus moving, `AccessibilityId::is_focused` reading the platform's own state.
pub(crate) fn keyboard_in_tab(keyboard: State<Keys>) -> bool {
    keyboard
        .read()
        .boxes
        .iter()
        .any(|(_, a11y)| a11y.is_focused())
}

/// Ask for the keyboard to go into the tab on screen: what pressing a chip does, so that
/// reading follows the tab the reader just chose.
pub(crate) fn ask_for_keyboard(mut keyboard: State<Keys>) {
    keyboard.write().wanted = true;
}

/// Spend that ask, once the tab it was made for has mounted what it has. In an effect and
/// not in the press, because the press is what mounts the panes: the box to focus does not
/// exist until the render it caused has run.
pub(crate) fn use_keyboard_asked(mut keyboard: State<Keys>, open: Open, marked: State<Marks>) {
    use_side_effect(move || {
        // Which side leads the tab on screen, which is the pane the ask is for: the one
        // the reader asked to see, and the one `DocumentBody` draws first.
        let leads = {
            let (strip, docs) = (open.strip.read(), open.docs.read());
            active_document(&strip, &docs).map(|document| leading(&document))
        };
        // **An ask is kept until there is somewhere to spend it.** A tab opened from a
        // list has nothing to focus in the pass that opened it: its assembly side draws a
        // sentence until the worker answers, and a pane with nothing to show registers no
        // box at all -- so an ask spent on `None` was every ask a row ever made. Both
        // fields are *read*, which is what subscribes this to the boxes as well as to the
        // ask, so the pane arriving is what wakes it. The guard is gone before the write.
        let waiting = {
            let keys = keyboard.read();
            keys.wanted.then(|| keys.wanted_box(leads)).flatten()
        };
        let Some((pane, a11y)) = waiting else {
            return;
        };
        keyboard.write().wanted = false;
        a11y.request_focus();
        // **A pane handed the keyboard has a caret put in it**, where it has no run of its
        // own. Nothing was clicked in it -- a row of a list opened this tab, or a chip was
        // pressed -- and a listing with no run draws no caret, so the arrows, Home, End
        // and Ctrl+C would have nothing to act on and the pane would read as though the
        // keyboard were somewhere else. Only an ask does this, and never a press: a press
        // in a pane says where the caret goes, including the press under the last row that
        // deliberately picks nothing out.
        if let Some(pane) = pane {
            if marked.peek().of(pane).is_none() {
                mark_top(marked, pane);
            }
        }
    });
}

/// Forget an ask nobody has spent: what a press that puts the keyboard somewhere itself
/// says. An ask now waits for a box, so one made for a tab that never drew a pane would sit
/// there and be spent by whatever pane arrived next -- taking the keyboard out of the list
/// the reader had put it in meanwhile.
pub(crate) fn unask_keyboard(mut keyboard: State<Keys>) {
    if keyboard.peek().wanted {
        keyboard.write().wanted = false;
    }
}

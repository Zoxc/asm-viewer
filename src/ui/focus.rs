//! A place in a file, the landing a click from outside the two panes makes, what each
//! tab keeps of where it was left and of its runs, and the effects that spend a landing.
//!
//! What the two panes say to each other is in `marks.rs`: each pane's picked-out run is
//! what the other pane lights the pair of, and owes a scroll to. Arriving somewhere is
//! this file's: `use_land` gives the arriving place its runs and `use_clear_marks` drops
//! a run whose listing has been replaced. Where a pane is scrolled to is `scrolling.rs`,
//! whose `use_kept_position` puts a place's scroll back as `use_land` puts its runs.

use super::*;

/// A source position the two panes point at together. The file is half the identity: an
/// inlined header's line 42 is not line 42 of the open file.
///
/// **Compared by its path and not by pointer**, unlike every other `Arc` the UI passes
/// around: two `LineInfo`s naming one file make two `Arc<Path>`s of it.
#[derive(Clone, PartialEq)]
pub(crate) struct LinePos {
    pub(crate) file: Arc<Path>,
    pub(crate) line: u32,
}

impl LinePos {
    /// How a line is named wherever one is said out loud: the file's own name and the
    /// line, the full path being a tooltip's.
    pub(crate) fn spell(&self) -> String {
        format!("{}:{}", source::name_of(&self.file), self.line)
    }

    /// The position row `row` of `file` is.
    pub(crate) fn of_row(file: Arc<Path>, row: usize) -> LinePos {
        LinePos {
            file,
            line: LinePos::line_of(row),
        }
    }

    /// The row this line is, and [`None`] for line 0.
    pub(crate) fn row(&self) -> Option<usize> {
        LinePos::row_of(self.line)
    }

    /// The 1-based line row `row` is: **the one way up**, rows being counted from zero
    /// and lines from one.
    ///
    /// Saturating, and no row the app draws reaches it: a source file is read only up to
    /// [`source::MAX_SIZE`], so it has nothing like [`u32::MAX`] lines. Total so that no
    /// caller has to invent a line for a row that has none.
    pub(crate) fn line_of(row: usize) -> u32 {
        u32::try_from(row).unwrap_or(u32::MAX).saturating_add(1)
    }

    /// The row line `line` is, and [`None`] for line 0: **the one way down**.
    ///
    /// **Line 0 is no line.** Debug info writes it for instructions belonging to no
    /// source line, and a stored place or a compiler's own message can state it too, so
    /// it arrives from a file and is not the app's to invent a row for. Read as row 0 it
    /// would pick out the first line of the file, which is somewhere the reader was
    /// never sent.
    pub(crate) fn row_of(line: u32) -> Option<usize> {
        (line as usize).checked_sub(1)
    }
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
    /// and no line. The characters to select along it come with it ([`Landed`]).
    pub(crate) at: Option<Landed>,
    /// The instruction to put the assembly pane's caret on, where the door was one, in
    /// whichever space the tab is in ([`Address`]): the listing that spends it asks for
    /// the half it can use, so a caret meant for one listing is never planted in the
    /// other.
    pub(crate) address: Option<Address>,
}

/// The line a landing names, and the characters on it where the door knew them.
///
/// The two travel together because a column is counted along a line: a door that named
/// columns and no line would be naming characters of nowhere.
#[derive(Clone, PartialEq)]
pub(crate) struct Landed {
    pub(crate) pos: LinePos,
    /// The characters to select on `pos`'s line, as byte columns: a search hit picks out
    /// what it matched, and a definition an empty run at the name's own column, which is
    /// a caret there and nothing selected. `None` for the doors that pick out the row and
    /// leave the caret at its start.
    pub(crate) columns: Option<Range<usize>>,
}

impl Landed {
    /// The line alone, for the doors that pick out the row and say nothing about where
    /// along it the caret goes.
    pub(crate) fn line(pos: LinePos) -> Landed {
        Landed { pos, columns: None }
    }
}

/// An instruction the assembly pane's caret is to be put on once the listing of the
/// place `at` is drawn: the half of a [`Landing`] the change of document cannot answer,
/// since the rows arrive after the document does -- a symbol's from the worker, an
/// object's code's as the skeleton and then as the stretch decodes. Left by `use_land` as
/// it plants the other half, or by `land` for a tab already on top, and spent by the
/// listing drawing the place it names through [`take_planting`]: `use_kept_place` for an
/// object's code, which puts the caret on the row at or below the address and keeps the
/// address with it so a decode re-places it on the instruction itself;
/// `InstructionList`'s planting effect for a symbol's. Spent by `use_land` on every
/// change of place besides, so one left lying -- a listing that never arrived -- plants
/// nothing in a listing opened for some other reason later.
///
/// **Keyed by the place and not the document.** Two stops in one document are two
/// places drawn by one listing. Keyed by the document, a planting not yet spent when
/// Back was pressed was taken by the place Back went to.
#[derive(Clone, PartialEq)]
pub(crate) struct Planting {
    pub(crate) at: Stop,
    pub(crate) address: Address,
}

/// The address a planting left for the place `at`, taken: [`None`] where there is none, or
/// where it names another place, whose listing is left to spend it.
///
/// **Read and not peeked**, so a door opened over the tab already on top wakes the
/// caller; the read is a statement of its own, since a read guard held across the write
/// would panic. **Spent before the caller looks for a row**, so an address the listing
/// cannot place is dropped rather than left owed to a listing drawn later.
pub(crate) fn take_planting(mut plant: State<Option<Planting>>, at: &Stop) -> Option<Address> {
    let planting = plant.read().clone();
    let planting = planting.filter(|planting| planting.at == *at)?;
    plant.set(None);
    Some(planting.address)
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
/// are open, everything kept per place, the record of visits, the runs the two panes have
/// picked out, and the two halves of a landing left for the arrival.
///
/// Not an incidental grouping. [`documents::land`] is the one path a door takes, and every
/// door passes it these six -- so a seventh thing a landing needs is a field here and
/// nothing at a call site. Provided once by `app()` and taken in one [`use_doors`], which
/// is why a door's handler is the landing it is about and not six lines of preamble.
///
/// A bundle does not own its handles: `marked` is the state [`Marked`] hands the panes, and
/// `open` and `places` the ones [`ProjectStates`] carries. A closer takes [`Places`] on its
/// own, having no door to go through -- what a close forgets is [`Places::forgetting`]'s and
/// is unchanged by the handles being reachable here too.
#[derive(Clone, Copy)]
pub(crate) struct Doors {
    pub(crate) open: Open,
    /// Everything kept per place. The two doors into a *place* write one down after they
    /// land -- the line a tab's assembly side follows, the address a code tab was left at
    /// -- and [`use_land`] puts back what the arriving place kept.
    pub(crate) places: Places,
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

/// **Whether the Source pane has moved off the file its run is in**, which is when that
/// run is dropped: the decision [`use_clear_marks`]'s second effect makes, apart from the
/// effect that acts on it.
///
/// `was` and `active` are the entry this last ran for and the one it is running for now,
/// `was_file` and `file` the file the pane was drawing and the one it draws now, and
/// `picked` the file the run is in.
///
/// A switch of place is `false` and drops nothing, for the reason [`use_clear_marks`]
/// gives; so is the pane going on drawing the file it was.
///
/// What is left is dropped only when the pane moves off the **run's** file, and not
/// whenever the file changes: a run a landing plants is in the file the pane is about to
/// show, and the switch that causes must not be what drops it. A run in a file the pane
/// never reaches stays, undrawn, until the next question replaces it.
fn moved_off(
    was: Option<&Entry>,
    active: Option<&Entry>,
    was_file: Option<&Arc<Path>>,
    file: Option<&Arc<Path>>,
    picked: Option<&Arc<Path>>,
) -> bool {
    if was != active || was_file == file {
        return false;
    }
    picked.is_some() && was_file == picked
}

/// Drop a pane's picked-out rows when the listing they index into is replaced: the
/// assembly pane's when another question is asked, the source pane's when the pane moves
/// off the run's file. An object's code being counted afresh under its run is **not** a
/// replacement: the run is carried to the rows it now has ([`carry_assembly`], from the
/// section view's own rebuild), since the place it marks has an address and the rows do
/// not.
///
/// At the root and keyed on the states that say *which listing*, **never on the listings
/// themselves**. A list's listing changes on a switch of place as much as on a
/// replacement, and an effect inside the list could not tell the two apart; the code
/// listing's rows change again with every stretch decoded under the run.
///
/// **Neither run is dropped here on a change of the active entry.** A switch of place is
/// [`use_land`]'s: it saves the runs of the place being left and puts back the arriving
/// place's own, and it does so in an effect woken by the same change as these two, in
/// no order anyone can rely on -- a drop made here for the switch could land after the
/// restore and take the restored run with it. So each effect judges itself by the entry
/// the last run saw ([`use_on_change`]) and, where that has changed, does nothing; what
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
    // indices. What the last run saw is [`use_on_change`]'s, which is the whole of what
    // either of these two effects kept.
    use_on_change(
        move || (active.peek().clone(), asked.read_ask()),
        move |was, (entry, ask)| {
            let Some((was_entry, was_ask)) = was else {
                return;
            };
            // The same place asking another question: the listing under the run is going.
            if was_entry == entry && was_ask != ask {
                unmark(marked, Pane::Assembly);
            }
        },
    );
    use_on_change(
        move || {
            // The *file the Source pane is drawing*, which is not the active document: two
            // functions from one file leave the same lines on screen. Compared against what
            // it last was rather than answered to directly, since reading the analysis
            // subscribes this to writes -- a request, the word that it is taking a while --
            // that change no listing.
            let active = active.read().clone();
            let document = active.as_ref().map(|(_, stop)| &stop.document);
            // No code rows: only the file is wanted, and it is the line of a code tab's
            // companion, not its file, that is read out of them.
            let file = source_side(document, &analysis.read(), &marked.read(), None)
                .map(|side| side.file().clone());
            (active, file)
        },
        move |was, (active, file)| {
            let Some((was_entry, was_file)) = was else {
                return;
            };
            // The file the run is in. Bound to a `let` of its own: the guard must be over
            // before the `unmark` below.
            let picked = marked
                .peek()
                .source
                .as_ref()
                .and_then(|picked| picked.file.clone());
            let off = moved_off(
                was_entry.as_ref(),
                active.as_ref(),
                was_file.as_ref(),
                file.as_ref(),
                picked.as_ref(),
            );
            if off {
                unmark(marked, Pane::Source);
            }
        },
    );
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
/// **A place shown for the first time with nothing to land on takes the keyboard**, with a
/// caret on the first line of the side it is driven from ([`fresh_run`]): a file, a symbol
/// or an object's code just opened is what the reader is about to read.
///
/// **One effect, because the order is the whole of it**: detect the switch, keep the runs
/// of the place being left ([`keep_leaving`]), decide whose landing this is and hand its
/// instruction on ([`take_landing`]), look up what the arriving place kept
/// ([`take_kept`]), and write the two runs ([`source_run`], [`assembly_run`]). Each stage
/// is a function over the [`Step`] they share -- what one stage tells the next is a field
/// of it -- and the rule a stage keeps is written on the stage.
pub(crate) fn use_land(
    doors: Doors,
    active: Memo<Option<Entry>>,
    sectioned: Sectioned,
    keyboard: Keyboard,
) {
    let Doors {
        open,
        places,
        mut marked,
        land: landing,
        plant,
        ..
    } = doors;
    let (driven, marks_at) = (places.driven, places.marks_at);

    // The entry the runs on screen belong to is what the last run saw, which is
    // [`use_on_change`]'s to keep. Reading the active document is all this wants of it;
    // the landing is peeked, so setting one wakes nothing until the document does.
    use_on_change(
        move || active.read().clone(),
        move |was, active| {
            // The mount is a change like any other here: a session restored onto a tab
            // arrives at that place and is landed on it.
            let leaving = was.cloned().flatten();
            let active = active.clone();
            if leaving == active {
                return;
            }

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

            let mut marks = Marks {
                assembly: assembly_run(&step, sectioned),
                source: source_run(&step, driven),
            };
            if fresh_run(&step, &mut marks) {
                ask_for_keyboard(keyboard);
            }
            marked.set_if_modified(marks);
        },
    );
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
/// [`Planting`] naming the place, for the listing that draws it to spend
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
    let waiting = asked
        .as_ref()
        .is_some_and(|asked| !names_this && open.active().as_ref() == Some(&asked.tab));
    if asked.is_some() && !waiting {
        landing.set(None);
    }
    step.landed = asked.filter(|_| names_this);
    let planting = step.landed.as_ref().and_then(|landing| {
        Some(Planting {
            at: step.active.as_ref()?.1.clone(),
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
            .and_then(|at| line_pick(at.pos.file, at.pos.line, at.columns, Owed::BOTH)),
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
    step.standing
        .clone()
        .filter(|picked| picked.is_line(file, line))
        .or_else(|| line_pick(file.clone(), line, None, Owed::default()))
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
fn assembly_run(step: &Step, sectioned: Sectioned) -> Option<Picked> {
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
    // Peeked, an effect having no business waking on a window decoding, and asked about
    // this object: rows of another are the last listing's.
    let built = sectioned.peek_rows_of(object)?;
    if kept.generation == Some(built.reading.generation) {
        kept.marks.assembly.clone()
    } else {
        kept.carry(|spot| row_of(&built, spot))
    }
}

/// Put a caret on the first line of the driven side of a place shown for the first time
/// with no landing, where that side has no run already. Whether the place was such a one,
/// which is when the keyboard goes to it too.
///
/// Written into the runs this arrival writes, and not left to the ask for the keyboard:
/// that ask is spent in an effect of its own, and a caret it put there before this ran
/// would be written over by the arriving place's empty runs.
fn fresh_run(step: &Step, marks: &mut Marks) -> bool {
    let Some((_, stop)) = &step.active else {
        return false;
    };
    if step.landed.is_some() || step.kept.is_some() {
        return false;
    }
    let run = match stop.document.driven_from() {
        Pane::Assembly => &mut marks.assembly,
        Pane::Source => &mut marks.source,
    };
    run.get_or_insert_with(top_pick);
    true
}

#[cfg(test)]
mod tests;

//! Opening a document, closing a tab, and moving between them.
//!
//! Neither invariant is held here. A tab and its trail are made together and closed
//! together by [`Open`]'s own methods, which every opening and every close below writes
//! through, and the tab on screen being one of the open ones is [`Strip`]'s. What is here
//! is the rest: which door a click is, and what a close has to let go of.
//! [`open_document`], [`raise`],
//! [`raise_tab`], [`navigate`], [`close_tab`], [`close_others`] and [`close_binary`] are
//! what open or close a **document** tab or change what one shows, and every path that
//! opens a document -- [`land`] included -- goes through [`open_document`]. A page is the
//! one tab outside that: it draws state held at the root, so it has no trail, and
//! [`close_page`] takes its chip out of the bar and nothing else.
//!
//! The window's tab keys are answered here too, and each of them is one of those doors
//! and not a second way round it: [`step_tab`] and [`show_nth`] work out which tab the
//! bar names and hand it to [`raise_tab`], and [`close_showing`] sends the tab on screen
//! to whichever of [`close_tab`] and [`close_page`] it belongs to.

use super::*;

/// How a document is reached: where it opens, which the click that opened it says and
/// nothing about the state can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reach {
    /// From inside the tab on screen -- a relocation link, the companion header over the
    /// Source pane. Pushed onto that tab's trail in place of what it showed, so the place
    /// left is one Back away. Promotes a temporal tab: the reader is reading in it. With
    /// no document tab on screen there is nothing to replace, and this is [`Reach::NewTab`].
    InPlace,
    /// In a tab of its own that stays, beside the tab on screen: Ctrl+click on anything,
    /// or a menu item. A tab already showing the place is raised instead, and promoted
    /// when it was the temporal one -- what was asked for is a tab of this place that
    /// stays, and it has one.
    NewTab,
    /// From outside the panes -- a row in a sidebar list. Into the one temporal tab,
    /// pushed onto its trail so that Back inside it walks the rows clicked, or a new
    /// temporal tab beside the tab on screen while there is none. A tab already showing
    /// the place is raised instead, the temporal one included, which promotes nothing.
    Preview,
}

impl Reach {
    /// How a press on a link **inside** a pane opens what it names: in place, pushed onto
    /// the tab's trail so the place left is one Back away, the way a browser follows a
    /// link, or, with Ctrl held, in a tab of its own beside it.
    ///
    /// On the enum and not with any one pane, because it is the rule for every link in a
    /// code row -- an operand naming a symbol, the bare address of a call, a name in the
    /// source -- and none of them owns it. Peeked, this being asked in a press handler.
    pub(crate) fn inside(ctrl: State<bool>) -> Reach {
        if *ctrl.peek() {
            Reach::NewTab
        } else {
            Reach::InPlace
        }
    }

    /// How a click from outside the panes opens its place: a preview in the temporal tab,
    /// or, with Ctrl held, a tab of its own that stays.
    ///
    /// Here for the same reason: it is the rule for every row that opens something -- the
    /// three sidebar lists, the Files view, the Bookmarks, the Search and Locations panels
    /// -- and none of them owns it. Between the two, Ctrl says one thing everywhere: a tab
    /// of its own. Peeked, this being asked in a press handler.
    pub(crate) fn outside(ctrl: State<bool>) -> Reach {
        if *ctrl.peek() {
            Reach::NewTab
        } else {
            Reach::Preview
        }
    }
}

/// Open the file at `path` as a source tab, the way `reach` says. Whether anything opened.
///
/// The one door for a path taken off a listing of the filesystem -- a Files row, a finder
/// row -- where there is no line to land on. Two rules live here and are written nowhere
/// else. **A file the source pane would refuse opens nothing at all**
/// (`source::showable`, the reader's own first step: a regular file within the bound the
/// source cache reads, and not a symlink to one), so a press cannot make a tab that only
/// says why it is empty. And the document is named by `path`'s own spelling, **never
/// canonicalised**, since a [`Document::Source`] and a [`LinePos`] are compared as text:
/// reduced here, a line the debug info names would be picked out in nothing
/// (`src/project.rs`).
///
/// A path that names a *place* -- a hit, a reference, a definition -- goes through
/// [`open_source_place`] instead, which lands on the line and drives the assembly side
/// from it.
pub(crate) fn open_source_file(states: ProjectStates, path: &Path, reach: Reach) -> bool {
    if !showable(path) {
        return false;
    }
    let file = Document::Source(Arc::from(&*path.to_string_lossy()));
    open_document(states.open, states.visits, file, reach).is_some()
}

/// Open `target` the way `reach` says, make the tab it lands in the active one, and
/// record the visit. The one path by which a document is ever opened.
///
/// The tab the document landed in. Every read of the states is bound before any write to
/// them.
pub(crate) fn open_document(
    open: Open,
    visits: State<Visits>,
    target: Document,
    reach: Reach,
) -> Option<DocId> {
    open_stop(open, visits, Stop::whole(target), reach)
}

/// The same for a place inside a document: what a door into an object's code at an
/// address opens, so the trail holds the place and not just the listing (`land`).
///
/// A stop and not a document is what goes on the trail; everything else here -- the
/// visit, which tab is preferred, the temporal tab -- is about the document, since a
/// move inside one is not a new place to have been.
pub(crate) fn open_stop(
    open: Open,
    mut visits: State<Visits>,
    stop: Stop,
    reach: Reach,
) -> Option<DocId> {
    let mut docs = open.docs;
    let target = stop.document.clone();

    // Recorded whatever else happens, and only when it changes the record: `State::write`
    // notifies whether or not the value changes, and re-opening the place at the top must
    // not wake the History panel.
    if visits.peek().would_touch(&target) {
        visits.write().record(target.clone());
    }

    // The tab on screen and the tab showing the target, the tab on screen preferred when
    // it is one of several: two tabs can show one place.
    let active = open.active_tab();
    let showing = match &active {
        Some((id, current)) if *current == target => Some(*id),
        _ => docs.peek().showing(&target),
    };
    let temporal = docs.peek().temporal();

    // A tab already showing the place is raised instead of anything opening, under every
    // reach but the one that opens *in* the tab on screen, which has its own rules below.
    // The tab has the document; what it may not have is the place. Raising promotes the
    // temporal tab under the two reaches that ask for a tab that stays -- `NewTab`, and
    // `InPlace` with no tab on screen, which is `NewTab` -- and never under a preview.
    let in_place = reach == Reach::InPlace && active.is_some();
    if let Some(id) = showing.filter(|_| !in_place) {
        moved_to(open, id, &stop);
        if reach != Reach::Preview && temporal == Some(id) {
            docs.write().promote(id);
        }
        raise(open, id);
        return Some(id);
    }

    // Nothing left to raise: what each reach opens, or the trail the place goes on.
    match reach {
        Reach::InPlace if active.is_some() => {
            let (id, current) = active?;
            // Already showing the document: only a place inside it is a move, and only
            // a different one. Otherwise nothing is pushed, and a write would wake every
            // header.
            let moved = match current != target {
                true => docs.write().push(id, stop),
                false => moved_to(open, id, &stop),
            };
            // A link followed inside the temporal tab is the reader reading in it,
            // whether it left the document or moved within one.
            if moved {
                docs.write().promote(id);
            }
            Some(id)
        }
        Reach::InPlace | Reach::NewTab => Some(open.open_tab(stop, false)),
        Reach::Preview => match temporal {
            Some(id) => {
                docs.write().push(id, stop);
                raise(open, id);
                Some(id)
            }
            None => Some(open.open_tab(stop, true)),
        },
    }
}

/// Make the open tab `id` the active one. The reader moved between places already
/// open -- a tab in the bar's menu, the neighbour a close lands on, a restored session
/// -- so nothing is recorded and no trail moves. A no-op for a closed id.
pub(crate) fn raise(open: Open, id: DocId) {
    raise_tab(open, Tab::Document(id));
}

/// The same for any tab, a page included: what pressing a chip does, and what the bar's
/// own menu does with the row that was picked.
///
/// Asked before it is written: `State::write` notifies whether or not the value changes,
/// so re-raising the tab already on screen must not reach for it. The question itself is
/// [`Strip::would_raise`], the strip's own.
pub(crate) fn raise_tab(open: Open, tab: Tab) {
    let mut strip = open.strip;
    let raising = strip.peek().would_raise(tab);
    if raising {
        strip.write().raise(tab);
    }
}

/// Show the tab a step along the bar lands on: the key's twin of pressing the chip
/// beside the one on screen. The wrapping is [`Strip::stepped`]'s, and a bar of one tab
/// steps to itself, which raises nothing.
pub(crate) fn step_tab(open: Open, along: Along) {
    // Bound in a statement of its own: the raise below writes the state this read.
    let stepping = open.strip.peek().stepped(along);
    if let Some(tab) = stepping {
        raise_tab(open, tab);
    }
}

/// Show the `nth` tab along the bar, 9 being the last however many there are
/// ([`Strip::nth`]). A number the bar is too short for shows nothing.
pub(crate) fn show_nth(open: Open, nth: usize) {
    let numbered = open.strip.peek().nth(nth);
    if let Some(tab) = numbered {
        raise_tab(open, tab);
    }
}

/// Close the tab `id`, moving to a neighbouring one when it was the tab on screen and
/// to the placeholder when it was the last one open.
///
/// Everything kept by the tab's entries goes with it: an [`Entry`] key holds the
/// `Arc<Object>` its document points into, so one left behind holds the file's bytes for
/// the life of the app. The lines its entries were driven from go with it too, for
/// consistency and **not** for that reason: a [`Document::Source`] key holds no object,
/// so it holds nothing up.
pub(crate) fn close_tab(open: Open, places: Places, id: DocId) {
    let tab = Tab::Document(id);

    // The close and the landing in one write, `Strip` holding that rule, and the trail
    // with the chip, `Open` holding that one; nothing else is owed here but letting go of
    // what the tab kept. Nothing removed is a tab that was not open -- a menu left open
    // while its tab closed -- and the trail and the positions below belong to whatever
    // holds the id now.
    let closed = open.close_tabs(|open| *open == tab);
    if !closed {
        return;
    }
    places.forgetting(|(tab, _): &Entry| *tab != id, |tab| *tab != id);
}

/// Close the page `page`, which is a tab leaving the bar and nothing else: what it was
/// showing is state at the root of the app, so it is all there when the page comes back.
pub(crate) fn close_page(open: Open, page: Page) {
    let mut strip = open.strip;
    strip.write().close(|tab| *tab == Tab::Page(page));
}

/// Close the tab on screen, whichever kind it is: what the × on its chip does, and what
/// the window's close key does from wherever the keyboard is. Nothing on screen is
/// nothing to close.
///
/// Two doors and not one, because a page's close is not a document's: a page has no
/// trail and nothing kept per place, so [`close_page`] is the whole of what it is owed.
pub(crate) fn close_showing(open: Open, places: Places) {
    // Bound in a statement of its own: both closes write the state this read.
    let showing = open.strip.peek().active();
    match showing {
        Some(Tab::Document(id)) => close_tab(open, places, id),
        Some(Tab::Page(page)) => close_page(open, page),
        None => {}
    }
}

/// Close every tab except `keep`, whatever kind either is, landing on the kept tab when
/// the one on screen is among those closing.
///
/// The unit is the **tab** and not the binary, so this is [`close_tab`] many times over
/// rather than [`close_binary`] with another filter: what each of them lets go of is the
/// same -- the tab, its trail, everything kept by its entries -- and for the same reason,
/// an [`Entry`] key holding the `Arc<Object>` it points into. Done in one pass rather
/// than by calling [`close_tab`] in a loop: each of those would work out a landing of its
/// own and walk the bar through every intermediate state.
pub(crate) fn close_others(open: Open, places: Places, keep: Tab) {
    // Which documents go, worked out before anything is removed and in a scope of its own,
    // so no read guard is alive when the writes below start. A tab that is not in the bar
    // any more keeps its neighbours: this is the menu of a tab that was closed while the
    // menu was open. The pages closing need no working out -- a page is a tab and nothing
    // else -- so the predicate below says "every tab but the kept one" and this says what
    // has to be let go of.
    let closing: Vec<DocId> = {
        let strip = open.strip.peek();
        if !strip.contains(keep) {
            return;
        }
        strip
            .documents()
            .filter(|id| Tab::Document(*id) != keep)
            .collect()
    };

    let closed = open.close_tabs(|tab| *tab != keep);
    if !closed {
        return;
    }

    places.forgetting(
        |(tab, _): &Entry| !closing.contains(tab),
        |tab| !closing.contains(tab),
    );
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The unit is the **file** and never the object, so one path opened twice closes once.
/// A tab *showing* a place in the file is closed, its positions forgotten with it; every
/// other tab keeps its slot and loses the places in the file from its trail, the cursor
/// carried to the nearest older survivor -- a source-driven tab's binary entries go this
/// way, and the tab stands. The History panel drops those places rather than degrading
/// them; a load still running is cancelled, or its objects would put the file back one
/// member at a time.
///
/// All the writes happen in this one handler, so the save observer wakes once on a settled
/// state and never writes a binary the app has already let go of.
pub(crate) fn close_binary(states: ProjectStates, path: &Path) {
    let ProjectStates {
        mut objects,
        mut loading,
        open,
        places,
        mut visits,
        ..
    } = states;
    let mut docs = open.docs;
    // Every guard below is taken out of its own statement or its own scope, so none of
    // them is still alive when the next write is reached.

    // Which tabs go, worked out before anything is removed. A page is never in a file, so
    // this walk leaves them alone -- and a page on screen when a binary closes keeps the
    // screen, nothing it is showing having gone anywhere.
    let closing: Vec<DocId> = {
        let (strip_ref, docs_ref) = (open.strip.peek(), docs.peek());
        strip_ref
            .documents()
            .filter(|id| {
                docs_ref
                    .get(*id)
                    .is_some_and(|document| document.in_file(path))
            })
            .collect()
    };

    // Every tab into the file, and only then everything the file was holding up: the
    // strip may have none of it, and the objects still go.
    open.close_tabs(|tab| matches!(tab, Tab::Document(id) if closing.contains(id)));
    // The surviving tabs' trails, thinned: every tab whose current entry is in the file
    // has just been closed, so no trail is left with nothing on it.
    docs.write()
        .retain_entries(|document| !document.in_file(path));

    // Nothing kept by an entry can outlive the entry: not the closed tabs', and not the
    // ones a surviving trail just lost, which hold the file's bytes just the same.
    places.forgetting(
        |(tab, stop): &Entry| !closing.contains(tab) && !stop.document.in_file(path),
        |tab| !closing.contains(tab),
    );
    // A source-driven tab stands, but a symbol it chose out of this file is let go: the
    // line beside the choice is what survives a close, and the next ask answers out of
    // what is left. The one thing here that is not a forget.
    let mut driven = places.driven;
    driven.write().release(path);

    let remaining = visits.peek().retaining(|entry| !entry.in_file(path));
    visits.set(remaining);

    objects.write().retain(|object| object.path != path);
    // Dropping the entry is what makes the next batch of objects out of this file be
    // dropped and the worker itself stop; see `take_load`.
    loading.write().cancel(path);
}

/// Open `landing`'s tab on its line and its instruction: open it the way `reach` says,
/// pick the line out in the source pane with both panes owed the scroll, and put the
/// assembly pane's caret on the instruction. The line is left as the [`Landing`] for the
/// change of *place* the door makes -- an opening, a raise, or a move inside the document
/// already on top -- and picked out here only where the door moves nothing at all; the
/// instruction is always a [`Planting`], the listing it is a row of coming after the
/// document, left here in that one case and by `use_land` otherwise.
pub(crate) fn land(doors: Doors, landing: Landing, reach: Reach) -> Option<DocId> {
    let Doors {
        open,
        visits,
        marked,
        land: mut land_at,
        mut plant,
    } = doors;
    let stop = stop_of(&landing);
    if open.active().as_ref() == Some(&landing.tab) {
        // The document is already on top, so nothing is opened and `open_stop` never
        // runs: the push here is the only record that the reader was somewhere else in
        // it a moment ago.
        let id = open.active_id();
        let moved = id.is_some_and(|id| moved_to(open, id, &stop));
        // A move inside the document is a change of place, and every change of place is
        // `use_land`'s: it keeps the runs of the place being left and gives the arriving
        // place its own. So a landing that moves the tab is left for it, as one that opens
        // a document is. Picked out here instead, the run would be on screen when the
        // entry changes -- saved there under the place being left, and then wiped by the
        // arrival, which finds no landing and falls back to the place's own line, without
        // the columns the door named or the scroll it owed.
        if moved {
            land_at.set(Some(landing));
            return id;
        }
        // The same place again, or a stop naming the document alone: nothing changes, so
        // no effect runs and the line and the instruction are put here.
        if let Some(at) = landing.at {
            mark_line(
                marked,
                at.file,
                at.line,
                landing.columns.clone(),
                Owed::BOTH,
            );
        }
        if let Some(address) = landing.address {
            plant.set(Some(Planting {
                tab: landing.tab,
                address,
            }));
        }
        return id;
    }

    land_at.set(Some(landing));
    open_stop(open, visits, stop, reach)
}

/// The place `tab` is at: the stop under its trail's cursor, and the whole of `document`
/// for a tab whose trail is not there to ask.
///
/// **What a pane keys its position and its runs by, and what a click in it writes
/// under.** Two stops in one document are two places -- two addresses in an object's
/// code, two lines of a file -- and stepping between them is what Back does inside a
/// listing, so a key built as the document alone would name a place the trail does not
/// hold: the position would read as never seen and the write would be dropped as a
/// closed tab's.
///
/// The borrow is the caller's, which is the point: a pane takes it from a `read`, so a
/// step re-renders it, and a handler from a `peek`.
pub(crate) fn place_at(docs: &Docs, tab: DocId, document: &Document) -> Stop {
    docs.current(tab)
        .cloned()
        .unwrap_or_else(|| Stop::whole(document.clone()))
}

/// The place a landing arrives at, as the trail holds one.
///
/// A door into an object's code at an address, or into a file at a line, opens *that
/// place*, so the trail holds it and a later Back comes back to it. Each document says
/// where in its own terms, and a symbol's landing is the one place its document is: an
/// address there is a caret in it and a line is a row of the file beside it.
fn stop_of(landing: &Landing) -> Stop {
    match (&landing.tab, landing.address, &landing.at) {
        (Document::Code(object), Some(address), _) => Stop::at(object.clone(), address),
        (Document::Source(file), _, Some(at)) if at.file == *file => {
            Stop::on(file.clone(), at.line)
        }
        _ => Stop::whole(landing.tab.clone()),
    }
}

/// Put `stop` on the trail of `id`, a tab already showing its document, where it is a
/// place inside that document and not the place the tab is at. Whether it moved.
///
/// A move inside one document opens nothing, so this is the whole of what makes it a
/// place the reader has been -- and the place left keeps its own rows and runs, being an
/// entry of its own. A stop naming only its document has nothing to come back to that
/// the document is not, so it is no move at all.
fn moved_to(open: Open, id: DocId, stop: &Stop) -> bool {
    if !stop.inside() {
        return false;
    }
    // Bound before the write, as ever.
    let current = open.docs.peek().current(id).cloned();
    if current.as_ref() == Some(stop) {
        return false;
    }
    let mut docs = open.docs;
    docs.write().push(id, stop.clone())
}

/// Raise the open tab `id` on `at`: what a Locations row does for the source-driven tab
/// its question was asked from, whose assembly side it has just chosen for. The tab is
/// already open and shows the file, so this is a [`raise`] and not an opening -- nothing
/// is recorded -- with the line picked out the way [`land`] picks it.
pub(crate) fn land_on(doors: Doors, id: DocId, at: LinePos) {
    let Doors {
        open,
        marked,
        land: mut landing,
        ..
    } = doors;
    if open.active_id() == Some(id) {
        mark_line(marked, at.file, at.line, None, Owed::BOTH);
        return;
    }
    // Bound in a statement of its own: the guard is gone before the writes.
    let showing = open.docs.peek().get(id).cloned();
    let Some(tab) = showing else {
        return;
    };
    landing.set(Some(Landing {
        tab,
        at: Some(at),
        address: None,
        columns: None,
    }));
    raise(open, id);
}

/// A step along the trail of the tab on screen: the mouse's back and forward buttons, and
/// the toolbar's two chevrons.
#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Back,
    Forward,
}

impl Nav {
    /// The entry this step would land on along `trail`, or `None` when it would not
    /// move. What the toolbar's two buttons name in their tooltips.
    ///
    /// The trail is asked rather than read: where a step lands is [`History::behind`] and
    /// [`History::ahead`], which is what `back` and `forward` themselves move by, so a
    /// button that is live and a step that does something cannot disagree.
    pub(crate) fn destination(self, trail: &History) -> Option<&Stop> {
        match self {
            Self::Back => trail.behind(),
            Self::Forward => trail.ahead(),
        }
    }

    /// Move the cursor and hand back the entry it landed on.
    fn step(self, trail: &mut History) -> Option<Stop> {
        match self {
            Self::Back => trail.back(),
            Self::Forward => trail.forward(),
        }
    }
}

/// Move the active tab's cursor one entry back or forward along its trail. The tab is
/// already on top, so what it shows is the whole of what changes: nothing is opened,
/// nothing is recorded, and a temporal tab stays temporal -- walking a trail is not
/// going somewhere new in it.
///
/// A step *inside* one document -- between two places in an object's code -- moves the
/// view like any other: the stop is half of the key the panes keep their position, their
/// runs and their driven line under, so the step is a switch to them and their own hooks
/// put back what that place was left with.
pub(crate) fn navigate(open: Open, nav: Nav) {
    let mut docs = open.docs;
    let Some(id) = open.active_id() else {
        return;
    };
    // Asked before writing: `State::write` notifies whether or not the value changes, and
    // a no-op step has to wake nothing.
    let possible = docs
        .peek()
        .trail(id)
        .is_some_and(|trail| nav.destination(trail).is_some());
    if !possible {
        return;
    }
    if let Some(trail) = docs.write().trail_mut(id) {
        nav.step(trail);
    }
}

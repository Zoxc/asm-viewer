//! The walk through an object's code: the step that starts one, the walk itself, and what
//! it says as it goes.
//!
//! **Not a pass over a listing.** The other two code panes draw a file or a function, and
//! the find worker searches either whole (`find_bar.rs`). An object's code is neither: it
//! is decoded a stretch at a time, so there is nothing to pass over, and searching it means
//! reading on from where the reader is until a match turns up. Hence one line and no
//! count -- the bar shows how far the reading has got where a count would be.
//!
//! **Nothing is kept.** A stretch is decoded exactly as the view's own window ask decodes
//! one and thrown away again, so a walk over a whole binary leaves the app's memory where
//! it found it.
//!
//! **The bar over an object's code has no listing** (`Find::listing`), which is what keeps
//! the two mechanisms off each other's work: nothing is asked of the find worker, and
//! [`use_find_steps`] leaves the step for [`use_code_hunt`] to spend.

use super::*;
use crate::find::Direction;
use crate::section;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Weak;

/// A search through an object's code: what it is looking for, and where it has got to.
///
/// **No count and no list of hits**, which is what tells this apart from a listing searched
/// whole: the code is read a piece at a time, so what a step asks for is the *next* match
/// and the whole of the answer is one line.
#[derive(Clone, PartialEq)]
pub(crate) struct Hunt {
    /// Which walk this is: two asks with the same question are two walks, and only the
    /// newest one's events are taken.
    pub(crate) id: u64,
    /// The object whose code it walks. Its answer is a line of that code and nowhere
    /// else.
    pub(crate) object: Over,
    pub(crate) filter: Filter,
    pub(crate) direction: Direction,
    /// The line and the column it started from, which is where the pane was, or [`None`]
    /// where the pane had no caret.
    pub(crate) from: Option<(CodeLine, usize)>,
    /// Where it has got to.
    pub(crate) walked: Walked,
    /// How far its match has been landed in the pane. The bar says so and not the
    /// listing: the bar outlives a listing that a switch to another tab unmounts, and a
    /// listing mounted again would land the match again, taking the caret back to it.
    pub(crate) landed: HuntLanding,
}

impl Hunt {
    /// Still going. A walk that has stopped found something or found nothing.
    pub(crate) fn walking(&self) -> bool {
        matches!(self.walked, Walked::Walking(_))
    }
}

/// A line of an object's code as a walk names it: the placed address it stands at, and
/// which of the rows there it is. An address alone names no one row: a section's header,
/// a symbol's labels and its first instruction all stand at the symbol's address.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CodeLine {
    pub(crate) address: PlacedAddress,
    pub(crate) kind: section::Kind,
}

/// The object a walk is over: by pointer, and without holding it. A bar outlives the
/// object its pane showed, and a closed binary's bytes must not stay behind in one.
///
/// A `Weak` keeps the allocation, so no other object can come to sit at the same address
/// while a bar holds this.
#[derive(Clone)]
pub(crate) struct Over(Weak<Object>);

impl Over {
    pub(crate) fn of(object: &Arc<Object>) -> Self {
        Over(Arc::downgrade(object))
    }

    /// Whether this is `object`.
    pub(crate) fn is(&self, object: &Arc<Object>) -> bool {
        std::ptr::eq(self.0.as_ptr(), Arc::as_ptr(object))
    }
}

impl PartialEq for Over {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

/// Where a walk has got to: still going, stopped on a match, or stopped with none.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Walked {
    /// Still going, and how much of the code it has been through, none to all of it.
    Walking(f32),
    /// The match it stopped on: the line it is on, and its columns.
    Found(CodeLine, Range<usize>),
    /// All the way round, and nothing.
    Nothing,
}

/// How far a walk's match has been landed in the pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HuntLanding {
    /// Not at all.
    No,
    /// The view has been brought to the row it is guessed to be on, in a stretch not
    /// decoded yet, and the caret waits for the stretch. Put on the guess, the caret would
    /// be carried by that row's place, a share of the stretch's bytes and not an
    /// instruction, and come down on the instruction before the match.
    Guessed,
    /// Picked out on its own row.
    Yes,
}

/// What a walk says as it goes.
pub(crate) enum Hunted {
    /// How much of the code has been walked.
    Through(f32),
    /// The first match: the line it is on, and its columns.
    Found(CodeLine, Range<usize>),
}

/// How many stretches are walked between one word about the progress and the next. A
/// stretch is a function, and a word per function on a binary with 115k of them is a write
/// per function to a state the bar reads.
const SAID_EVERY: usize = 64;

/// Walk `object`'s code for the next match of `filter` from `from`, the way `direction`
/// says, and say how far it has got as it goes. With no `from` the walk starts at the top
/// going forward and at the bottom going back, and every line counts. `from`'s column is
/// read as [`crate::find::step`] reads a caret: going forward a hit starting there is
/// ahead of it, and going back one ending there is behind it.
///
/// **Stretch by stretch, and nothing kept.** Each is decoded exactly as the view's own
/// window ask decodes one (`answer`'s `Question::Code` arm) and thrown away again: what
/// comes back is a line, so a walk over a whole object leaves the app's memory where
/// it found it, and the landing pays for the one stretch it lands in through the ordinary
/// window ask.
///
/// **It wraps once.** The walk starts in the stretch the caret is in and ends there,
/// having been round the whole listing, so a match behind the reader is still found and no
/// match is found twice. That stretch is read twice: first for what is past the caret,
/// and last, back round, for the rest of it. With no caret it is read once, whole.
pub(crate) fn hunt(
    object: &Object,
    code: &Arc<CodeListing>,
    filter: &Filter,
    from: Option<(CodeLine, usize)>,
    direction: Direction,
    emit: &mut dyn FnMut(Hunted) -> ControlFlow<()>,
) {
    let matcher = filter.matcher();
    let total = code.stretch_count();
    let Some(last) = total.checked_sub(1) else {
        return;
    };
    let first = match (from, direction) {
        (Some((from, _)), _) => code.at(from.address).unwrap_or(0),
        (None, Direction::Forward) => 0,
        (None, Direction::Back) => last,
    };
    // One step more than there are stretches where there is a caret: the step back into
    // the first one.
    let steps = if from.is_some() { total + 1 } else { total };

    for step in 0..steps {
        let flat = match direction {
            Direction::Forward => (first + step) % total,
            Direction::Back => (first + total - step % total) % total,
        };
        if step % SAID_EVERY == 0 {
            let through = (step as f32 / total as f32).min(1.0);
            if emit(Hunted::Through(through)).is_break() {
                return;
            }
        }

        let mut lines = section_view::stretch_texts(object, code, flat);
        // In the order the listing draws them, and backwards for a walk that way, so the
        // match found is the nearest one behind the reader and not the first of a stretch.
        lines.sort_by_key(|(address, _, _)| *address);
        // In the stretch the walk started in, the reader's place is the caret's address,
        // which of the lines there it is on, and its column, and each hit is placed
        // against all three. A caret on a row that draws nothing sits just above the
        // lines at its address.
        let caret = from
            .filter(|_| step == 0 || step == total)
            .map(|(at, col)| {
                let rank = lines
                    .iter()
                    .position(|(address, kind, _)| *address == at.address && *kind == at.kind);
                (at.address, rank, col)
            });
        let mut ranked: Vec<_> = lines.into_iter().enumerate().collect();
        if direction == Direction::Back {
            ranked.reverse();
        }
        for (rank, (address, kind, line)) in ranked {
            let mut hits = matcher.marks(line.as_str());
            if direction == Direction::Back {
                hits.reverse();
            }
            let found = hits.into_iter().find(|columns| {
                let Some(caret) = caret else {
                    return true;
                };
                let past = match direction {
                    Direction::Forward => (address, Some(rank), columns.start) >= caret,
                    Direction::Back => (address, Some(rank), columns.end) <= caret,
                };
                // Only what is past the caret counts at the start, or a step would find
                // the match the pane is already on, and only what is not at the end.
                past == (step == 0)
            });
            if let Some(columns) = found {
                let _ = emit(Hunted::Found(CodeLine { address, kind }, columns));
                return;
            }
        }
    }
    let _ = emit(Hunted::Through(1.0));
}

/// The walk through an object's code that a step over one asks for: started here, taken
/// here, and landed by `land`.
///
/// **Its own hook and not [`use_find_steps`]**, which steps through an answer the pane
/// already holds. There is no such answer here: what a step asks for is one line, found
/// by reading on, and the bar shows how far the reading has got instead of a count. The
/// two divide the step between them by whether the bar has a listing.
///
/// `from` is where the pane is, as a line and a column, for a walk the way it is given,
/// and [`None`] where there is no caret in it yet. `land` is given the object, the match
/// and how far it has been landed, and is the section view's own: only it can put a caret
/// on the row a line is drawn in, the rows being counted afresh as stretches decode. It
/// says how far it has landed now, and it reads the rows, so a match found before there
/// are any, or in a stretch not decoded yet, is landed when they come.
///
/// **`at` and `object` reach every effect through its deps**, never as a capture: an
/// effect's callback is built once, and a switch of tab re-renders this list with another
/// tab's rather than mounting it again (`ui/split.rs`). A walk remembers the object it
/// walks, so a pane that moves to another object mid-walk starts it again over the new one.
pub(crate) fn use_code_hunt(
    at: Where,
    object: Arc<Object>,
    reading: State<Reading>,
    from: impl Fn(Direction) -> Option<(CodeLine, usize)> + 'static,
    mut land: impl FnMut(&Arc<Object>, CodeLine, Range<usize>, HuntLanding) -> HuntLanding + 'static,
) {
    let finds = use_try_consume::<Looking>().map(|looking| looking.0);
    let from = Rc::new(from);

    // A step over an object's code starts a walk rather than moving through an answer.
    let start = from.clone();
    use_side_effect_with_deps(
        &(at, ByPtr(object.clone())),
        move |(at, ByPtr(object)): &(Where, ByPtr<Object>)| {
            let (at, Some(finds)) = (*at, finds) else {
                return;
            };
            let bar = finds.read().get(&at).clone();
            // A bar with a listing is searched whole, and its step is `use_find_steps`'s.
            let Some(direction) = bar.step.filter(|_| bar.listing.is_none()) else {
                return;
            };
            // Nothing typed, or a pattern that will not compile, matches no line, and a
            // walk for it would decode the whole object to say so. The step is spent on
            // nothing, as a listing's is.
            if !bar.filter.searches() {
                edit_find(finds, at, |bar| bar.step = None);
                return;
            }
            let id = WALKS.fetch_add(1, Ordering::Relaxed);
            let from = start(direction);
            let object = Over::of(object);
            edit_find(finds, at, move |bar| {
                bar.step = None;
                bar.hunt = Some(Hunt {
                    id,
                    object,
                    filter: bar.filter.clone(),
                    direction,
                    from,
                    walked: Walked::Walking(0.0),
                    landed: HuntLanding::No,
                });
            });
        },
    );

    // The walk itself. A memo over which walk it is, not a read: every word it says about
    // its progress is a write to the state below, and an effect reading that would start
    // a walk per word.
    let over = use_reactive(&at);
    let asked = use_memo(move || {
        let at = *over.read();
        let finds = finds?;
        let bar = finds.read();
        let hunt = bar.get(&at).hunt.as_ref()?;
        hunt.walking().then_some((
            at,
            hunt.id,
            hunt.object.clone(),
            hunt.filter.clone(),
            hunt.from,
            hunt.direction,
        ))
    });
    let started = asked.read().clone();
    // The walks this scope has started. A switch away from a tab mid-walk and back again
    // hands the effect that walk a second time, and its taker is still running: the task
    // lives as long as this scope.
    let taking = use_hook(|| Rc::new(RefCell::new(HashSet::<u64>::new())));
    use_side_effect_with_deps(
        &(started, at, ByPtr(object.clone())),
        move |(walk, here, ByPtr(object)): &(Option<Walk>, Where, ByPtr<Object>)| {
            let (Some((at, id, walked, filter, place, direction)), Some(finds)) =
                (walk.clone(), finds)
            else {
                return;
            };
            // The memo is a render behind a switch of tab, when the walk is another bar's.
            // That walk is left alone, and out of `taking`: `object` is this tab's, and
            // walked here its match would be a line of the wrong code in the other bar. It
            // is started once the memo catches up with its tab.
            if at != *here {
                return;
            }
            if !walked.is(object) {
                // The pane has moved to another object mid-walk: start again over this
                // one, under a new id, which is what calls the old walk off.
                let id = WALKS.fetch_add(1, Ordering::Relaxed);
                let (object, from) = (Over::of(object), from(direction));
                edit_find(finds, at, move |bar| {
                    if let Some(hunt) = &mut bar.hunt {
                        *hunt = Hunt {
                            id,
                            object,
                            from,
                            walked: Walked::Walking(0.0),
                            landed: HuntLanding::No,
                            ..hunt.clone()
                        };
                    }
                });
                return;
            }
            if !taking.borrow_mut().insert(id) {
                return;
            }
            let object = object.clone();
            // The skeleton the view already has, where the reading is this object's.
            let code = {
                let reading = reading.peek();
                reading
                    .is_about(&object)
                    .then(|| reading.code.as_ref().map(|layout| layout.code().clone()))
                    .flatten()
            };
            let events = stream("the code search", Some(64), move |emit| {
                // Or one built here: it is free (`CodeListing`), and a walk asked for
                // before the view has one must not wait.
                let code = code.unwrap_or_else(|| Arc::new(CodeListing::new(&object)));
                hunt(&object, &code, &filter, place, direction, emit);
            });
            spawn(take_hunt(finds, at, id, events));
        },
    );

    // The match, landed once: the bar says it was, so an effect woken again -- by the rows
    // changing, or by a listing mounted again -- does not land it a second time.
    use_side_effect_with_deps(
        &(at, ByPtr(object)),
        move |(at, ByPtr(object)): &(Where, ByPtr<Object>)| {
            let (at, Some(finds)) = (*at, finds) else {
                return;
            };
            let hunt = finds.read().get(&at).hunt.clone();
            // A match in another object's code names no row of this one.
            let Some(hunt) = hunt.filter(|hunt| hunt.object.is(object)) else {
                return;
            };
            let Walked::Found(line, columns) = hunt.walked else {
                return;
            };
            if hunt.landed == HuntLanding::Yes {
                return;
            }
            // Marked as far as it got: a match with no row of its own yet is landed when
            // the rows have one, `land` having read them.
            let landed = land(object, line, columns, hunt.landed);
            if landed != hunt.landed {
                edit_find(finds, at, |bar| {
                    if let Some(held) = bar.hunt.as_mut().filter(|held| held.id == hunt.id) {
                        held.landed = landed;
                    }
                });
            }
        },
    );
}

/// A walk as a step starts it: the bar it is for, which walk, the object it walks, the
/// pattern, where it starts and which way it goes.
type Walk = (
    Where,
    u64,
    Over,
    Filter,
    Option<(CodeLine, usize)>,
    Direction,
);

/// Where the next walk's id comes from: one count for every listing, so no two walks
/// anywhere share an id, and a list mounted again cannot reuse one a bar still holds.
static WALKS: AtomicU64 = AtomicU64::new(0);

/// Take what a walk says, for as long as it is the walk the bar is on.
///
/// The receiver dropping is what stops the worker, so returning early is how a walk the
/// reader has moved on from is called off (`search_view.rs`).
///
/// Through [`edit_find`], as every other writer of a bar is: a bar closed under a walk is
/// not reopened by one, and a word that says what the bar already holds leaves the table
/// as it was (`find_bar.rs` on [`Finds`]).
async fn take_hunt(
    finds: State<Finds>,
    at: Where,
    id: u64,
    events: async_channel::Receiver<Hunted>,
) {
    while let Some(batch) = next_batch(&events).await {
        let held = finds.peek().get(&at).hunt.clone();
        let Some(mut hunt) = held.filter(|hunt| hunt.id == id) else {
            return;
        };
        for event in batch {
            // A walk says nothing after the match it found, so nothing here undoes one.
            hunt.walked = match event {
                Hunted::Through(through) => Walked::Walking(through),
                Hunted::Found(line, columns) => Walked::Found(line, columns),
            };
        }
        let done = !hunt.walking();
        edit_find(finds, at, move |bar| bar.hunt = Some(hunt));
        if done {
            return;
        }
    }
    // The walk ended without finding anything: the bar says so rather than going on
    // saying how far it has got.
    let held = finds.peek().get(&at).hunt.clone();
    if held.is_some_and(|hunt| hunt.id == id && hunt.walking()) {
        edit_find(finds, at, |bar| {
            if let Some(hunt) = &mut bar.hunt {
                hunt.walked = Walked::Nothing;
            }
        });
    }
}

#[cfg(test)]
mod tests;

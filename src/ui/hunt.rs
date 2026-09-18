//! The walk through an object's code: the step that starts one, the walk itself, and what
//! it says as it goes.
//!
//! **Not a pass over a listing.** The other two code panes draw a file or a function, and
//! the find worker searches either whole (`find_bar.rs`). An object's code is neither: it
//! is decoded a stretch at a time, so there is nothing to pass over, and searching it means
//! reading on from where the reader is until a match turns up. Hence one address and no
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Weak;

/// A search through an object's code: what it is looking for, and where it has got to.
///
/// **No count and no list of hits**, which is what tells this apart from a listing searched
/// whole: the code is read a piece at a time, so what a step asks for is the *next* match
/// and the whole of the answer is one address.
#[derive(Clone, PartialEq)]
pub(crate) struct Hunt {
    /// Which walk this is: two asks with the same question are two walks, and only the
    /// newest one's events are taken.
    pub(crate) id: u64,
    /// The object whose code it walks. Its answer is an address in that code and nowhere
    /// else.
    pub(crate) object: Over,
    pub(crate) filter: Filter,
    pub(crate) direction: Direction,
    /// The address it started from, which is where the pane was.
    pub(crate) from: u64,
    /// Where it has got to.
    pub(crate) walked: Walked,
}

impl Hunt {
    /// Still going. A walk that has stopped found something or found nothing.
    pub(crate) fn walking(&self) -> bool {
        matches!(self.walked, Walked::Walking(_))
    }
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
    /// The match it stopped on: the address of the line it is on, and its columns.
    Found(u64, Range<usize>),
    /// All the way round, and nothing.
    Nothing,
}

/// What a walk says as it goes.
pub(crate) enum Hunted {
    /// How much of the code has been walked.
    Through(f32),
    /// The first match: the placed address of the line it is on, and its columns.
    Found(u64, Range<usize>),
}

/// How many stretches are walked between one word about the progress and the next. A
/// stretch is a function, and a word per function on a binary with 115k of them is a write
/// per function to a state the bar reads.
const SAID_EVERY: usize = 64;

/// Walk `object`'s code for the next match of `filter` from `from`, the way `direction`
/// says, and say how far it has got as it goes.
///
/// **Stretch by stretch, and nothing kept.** Each is decoded exactly as the view's own
/// window ask decodes one (`answer`'s `Question::Code` arm) and thrown away again: what
/// comes back is an address, so a walk over a whole object leaves the app's memory where
/// it found it, and the landing pays for the one stretch it lands in through the ordinary
/// window ask.
///
/// **It wraps once.** The walk starts in the stretch the address is in and ends there,
/// having been round the whole listing, so a match behind the reader is still found and no
/// match is found twice.
pub(crate) fn hunt(
    object: &Object,
    code: &Arc<CodeListing>,
    filter: &Filter,
    from: u64,
    direction: Direction,
    emit: &mut dyn FnMut(Hunted) -> ControlFlow<()>,
) {
    let matcher = filter.matcher();
    let index = section::Flat::new(Arc::clone(code));
    let total = index.count();
    let Some(last) = total.checked_sub(1) else {
        return;
    };
    let first = code
        .at(from)
        .and_then(|place| index.index(place))
        .unwrap_or(0);

    for step in 0..total {
        let flat = match direction {
            Direction::Forward => (first + step) % total,
            Direction::Back => (first + total - step % total) % total,
        };
        if step % SAID_EVERY == 0 {
            let through = step as f32 / total as f32;
            if emit(Hunted::Through(through)).is_break() {
                return;
            }
        }

        let mut lines = section_view::stretch_texts(object, &index, flat);
        // In the order the listing draws them, and backwards for a walk that way, so the
        // match found is the nearest one behind the reader and not the first of a stretch.
        lines.sort_by_key(|(address, _)| *address);
        if direction == Direction::Back {
            lines.reverse();
        }
        for (address, line) in lines {
            // The stretch the walk started in holds the reader's own place: only what is
            // past it counts, or a step would find the match the pane is already on.
            if step == 0 || (step == last && flat == first) {
                let past = match direction {
                    Direction::Forward => address > from,
                    Direction::Back => address < from,
                };
                if !past {
                    continue;
                }
            }
            let mut hits = matcher.marks(line.as_str());
            if direction == Direction::Back {
                hits.reverse();
            }
            if let Some(columns) = hits.into_iter().next() {
                let _ = emit(Hunted::Found(address, columns));
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
/// already holds. There is no such answer here: what a step asks for is one address, found
/// by reading on, and the bar shows how far the reading has got instead of a count. The
/// two divide the step between them by whether the bar has a listing.
///
/// `from` is where the pane is, as an address; a listing with no caret in it yet starts at
/// the top. `land` is given the match, and is the section view's own: only it can put a
/// caret on the row an address is in, the rows being counted afresh as stretches decode.
///
/// **`at` and `object` reach every effect through its deps**, never as a capture: an
/// effect's callback is built once, and a switch of tab re-renders this list with another
/// tab's rather than mounting it again (`ui/split.rs`). A walk remembers the object it
/// walks, so a pane that moves to another object mid-walk starts it again over the new one.
pub(crate) fn use_code_hunt(
    at: Where,
    object: Arc<Object>,
    reading: State<Reading>,
    from: impl Fn() -> u64 + 'static,
    mut land: impl FnMut(u64, Range<usize>) -> bool + 'static,
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
            let id = WALKS.fetch_add(1, Ordering::Relaxed);
            let from = start();
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
            if at == *here && !walked.is(object) {
                // The pane has moved to another object mid-walk: start again over this
                // one, under a new id, which is what calls the old walk off.
                let id = WALKS.fetch_add(1, Ordering::Relaxed);
                let (object, from) = (Over::of(object), from());
                edit_find(finds, at, move |bar| {
                    if let Some(hunt) = &mut bar.hunt {
                        *hunt = Hunt {
                            id,
                            object,
                            from,
                            walked: Walked::Walking(0.0),
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
                    .then(|| reading.code.clone())
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

    // The match, landed once. The walk that found it is remembered, so an effect woken
    // again -- by the pane's own rows arriving, say -- does not land it a second time.
    let mut landed = use_state(|| None::<u64>);
    use_side_effect_with_deps(
        &(at, ByPtr(object)),
        move |(at, ByPtr(object)): &(Where, ByPtr<Object>)| {
            let Some(finds) = finds else {
                return;
            };
            let hunt = finds.read().get(at).hunt.clone();
            // A match in another object's code names no row of this one.
            let Some(hunt) = hunt.filter(|hunt| hunt.object.is(object)) else {
                return;
            };
            let Walked::Found(address, columns) = hunt.walked else {
                return;
            };
            if *landed.peek() == Some(hunt.id) {
                return;
            }
            // Marked as landed only where it was: a walk that answers before the pane has
            // rows to land in is landed by the wake the rows bring.
            if land(address, columns) {
                landed.set(Some(hunt.id));
            }
        },
    );
}

/// A walk as a step starts it: the bar it is for, which walk, the object it walks, the
/// pattern, where it starts and which way it goes.
type Walk = (Where, u64, Over, Filter, u64, Direction);

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
                Hunted::Found(address, columns) => Walked::Found(address, columns),
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

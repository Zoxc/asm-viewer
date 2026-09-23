//! Find in a code pane: what each pane's bar is asking, the worker that answers, and the
//! bar itself.
//!
//! **A bar per pane, and the panes are not the sidebar's lists.** Ctrl+F over a list means
//! the filter box above it (`filter_bar.rs`), which asks which *rows* belong; here it asks
//! where in the text a pattern is, because what comes back is washed and selected in the
//! code itself. The pattern is a [`Filter`] all the same, so the three toggles mean one
//! thing in both.
//!
//! **Nothing is searched on the UI thread.** A listing is a file or a function, either of
//! which is a regex pass long enough to be felt at the keystroke that starts it, so the
//! pass is a worker's and the pane holds its answer -- the shape every worker here has
//! (`agents/Worker.md`). A worker of its own and not the analysis one, for the reason the
//! source reader has one: a pattern supersedes on every keystroke and must not queue
//! behind the seconds of DWARF a click costs.
//!
//! **What a row wears is not that answer.** A drawn row asks the compiled matcher where it
//! hits, as every marked row in the app does (`Marking`), which is drawing and not
//! searching: no pass over the listing, and it is the only thing an object's code can
//! answer, a line there having text only once it has been read.
//!
//! **A listing the pane holds entire, and nothing else.** An object's code is not one: it
//! is decoded a stretch at a time, so a step over it reads on for the next match instead.
//! That is `hunt.rs`, which shares the bar with this and none of its mechanism. What tells
//! the two apart is [`Find::listing`], which such a bar has none of.

use super::*;
// `Direction` by name as well as through the glob: here it is which way a step goes, and
// freya's prelude has a layout `Direction` this file never asks for (`ui.rs` on `Panel`).
use crate::counter;
use crate::find::{self, Direction, Hit};
use std::cell::Cell;

/// Which bar: the listing it is drawn in -- a tab, or the scratchpad's -- and which of the
/// two panes.
pub(crate) type Where = (Placing, Pane);

/// A listing a pane can be searched whole, and what a worker is handed to do it.
///
/// **Only a listing the pane holds entire.** An object's code is not one: it is decoded a
/// stretch at a time, so a pane over it has no listing at all and its bar is walked instead
/// (`hunt.rs`). That is the one thing that tells the two mechanisms apart.
///
/// Both arms are `Send`: a `SourceText` is the `Arc<Highlighted>` the source reader
/// already built on a thread, and an `Arc<Assembly>` is what the analysis worker answers
/// with. Neither `source_line` nor `instruction_line` asks for a colour, which is what
/// lets the lines be built off the UI thread at all.
#[derive(Clone)]
pub(crate) enum Searchable {
    /// The file the Source pane is showing; a row is one of its lines.
    Source(SourceText),
    /// The symbol the Assembly pane is drawing. `lanes` because a listing row is not an
    /// instruction index: a separator sits above every branch target.
    Symbol {
        assembly: Arc<Assembly>,
        lanes: Arc<Lanes>,
    },
}

impl Searchable {
    /// Which listing this is, by pointer: what an answer is judged against, so one about
    /// the file or the symbol a pane has moved off is dropped.
    pub(crate) fn id(&self) -> usize {
        match self {
            Searchable::Source(source) => Arc::as_ptr(&source.0).addr(),
            Searchable::Symbol { assembly, .. } => Arc::as_ptr(assembly).addr(),
        }
    }
}

/// By the pointer, as everything else in the UI with an `Arc` behind it is: a listing holds
/// a file's bytes or a symbol's, and comparing those by value would walk the whole file.
impl PartialEq for Searchable {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

/// Every hit in `listed`, in the rows the pane draws. The whole of the work, and it
/// touches no UI state.
pub(crate) fn look(listed: &Searchable, filter: &Filter) -> Vec<Hit> {
    let matcher = filter.matcher();
    let mut hits = Vec::new();
    let mut push = |row: usize, line: &Line| {
        hits.extend(
            matcher
                .marks(line.as_str())
                .into_iter()
                .map(|columns| Hit { row, columns }),
        );
    };

    match listed {
        Searchable::Source(source) => {
            for row in 0..source.0.lines {
                // The file's own cut, which every keystroke here would otherwise make
                // again for every line of it (`Highlighted::text`).
                push(row, &source_line(source, row));
            }
        }
        Searchable::Symbol { assembly, lanes } => {
            for index in 0..assembly.instructions.len() {
                push(lanes.row_of(index), &instruction_line(assembly, index));
            }
        }
    }
    hits
}

/// What a search is about: which listing, by pointer, and which pattern.
///
/// One value and not two loose fields: the ask, the answer and the hits the bar draws are
/// judged by one `==`, where comparing the two halves in each of the three places is three
/// places to forget one and draw an answer about a listing the pane has left or a pattern
/// typed past.
#[derive(Clone, PartialEq)]
pub(crate) struct About {
    listing: usize,
    filter: Filter,
}

impl About {
    /// What a search of `listed` for `filter` is about.
    fn of(listed: &Searchable, filter: &Filter) -> Self {
        About {
            listing: listed.id(),
            filter: filter.clone(),
        }
    }
}

/// One pane's find bar: what is typed, what the worker said about it, and where the pane
/// has got to in the answer.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Find {
    /// The box and its three toggles.
    pub(crate) filter: Filter,
    /// The listing the pane is drawing, written by the pane. `None` where it has none to
    /// search whole -- a pane over an object's code, whose bar is walked instead
    /// (`hunt.rs`), and a pane with nothing in it. Which of the two mechanisms this bar
    /// is: nothing else is asked.
    pub(crate) listing: Option<Searchable>,
    /// What was last asked, so a question in flight is not asked again every render.
    asked: Option<About>,
    /// What came back about a listing searched whole: what the hits are about, and the
    /// hits in order.
    found: Option<(About, Shared<Hit>)>,
    /// The walk through an object's code a step over one asked for (`hunt.rs`), where the
    /// bar is on one.
    ///
    /// A bar has this or [`found`](Self::found) and never both: which of the two it is, is
    /// whether it has a listing, and an object's code is the one that has none.
    pub(crate) hunt: Option<Hunt>,
    /// Which hit the pane is on, `None` until a step has landed. A new pattern clears it,
    /// so the next step starts from the caret and not from wherever the last one ended;
    /// and a step reads it only while the pane's run is still that hit ([`find::step`]).
    pub(crate) at: Option<usize>,
    /// A step the bar has asked for and the pane has not made yet, and which way. Spent
    /// by the list, which is what knows where its rows are and can scroll to one, once
    /// the answer it steps through has come.
    pub(crate) step: Option<Direction>,
    /// The caret the opening asked for, spent by the box once it is mounted.
    pub(crate) focus: bool,
}

impl Find {
    /// What a search of the listing the pane is drawing, for what is typed now, is about.
    /// `None` where the pane has no listing, there being nothing to search.
    fn about(&self) -> Option<About> {
        Some(About::of(self.listing.as_ref()?, &self.filter))
    }

    /// The hits, where the answer is about the listing and pattern being asked about now.
    /// A stale answer draws nothing rather than the last file's marks.
    pub(crate) fn hits(&self) -> Option<&Shared<Hit>> {
        let about = self.about()?;
        match &self.found {
            Some((found, hits)) if *found == about => Some(hits),
            _ => None,
        }
    }

    /// Start again: the pane is on no hit, and the walk, where there is one, is given up.
    ///
    /// What a new pattern and a new listing both owe. A walk says next what is about a
    /// pattern or a listing that has gone, and the take that finds it gone drops the
    /// receiver, which is what calls the walk off (`hunt.rs`). Hits are left where they
    /// are, stale or not: [`hits`](Self::hits) is what refuses a stale answer.
    pub(crate) fn reset(&mut self) {
        self.at = None;
        self.hunt = None;
    }

    /// Whether an answer is owed about the listing and pattern asked about now: asked for
    /// or still to be. A step asked meanwhile waits for it rather than finding no hits.
    ///
    /// Nothing typed marks nothing, so nothing is owed: an empty pattern is not a search
    /// that finds everything (`Matcher::Everything`).
    fn owed(&self) -> bool {
        self.filter.asks() && self.listing.is_some() && self.hits().is_none()
    }

    /// What a question is owed for, or `None` where the answer is already about it or one
    /// is in flight. No `pending` flag of its own: a listing is being searched exactly
    /// while it is wanted and neither the answer nor the ask is about it ([`Coded`]).
    fn pending(&self) -> Option<(Searchable, Filter)> {
        if !self.owed() {
            return None;
        }
        let listed = self.listing.clone()?;
        if self.asked.as_ref() == Some(&About::of(&listed, &self.filter)) {
            return None;
        }
        Some((listed, self.filter.clone()))
    }

    /// Take `hits` as the answer `about` says it is, and say whether anything changed.
    /// Refused where the pane has moved on, which is the whole of the supersession rule:
    /// a comparison and not a generation count.
    fn take(&mut self, about: About, hits: Shared<Hit>) -> bool {
        if self.about().as_ref() != Some(&about) {
            return false;
        }
        self.found = Some((about, hits));
        true
    }
}

/// Every pane's bar, keyed by the pane. An entry is what "the bar is open" means: opening
/// puts one in and closing takes it out.
///
/// Keyed by the **tab** and not by a place on its trail, so a step Back leaves the bar as
/// the reader left it: what was typed lasts as long as the tab is open.
///
/// **Written only where it changed.** A `set` notifies whether or not it changed anything,
/// and this is the state a keystroke in a find box writes: every bar, every listing's
/// [`use_marking`] and every effect over a bar wakes on any write to it. Hence the
/// `PartialEq` -- cheap, a bar being patterns, pointers and flags -- and hence a writer
/// that can leave the table as it was ending in `set_if_modified`. It is the rule
/// `marks::update` and `write_if` (`ui/worker.rs`) state, for a state the reader writes.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Finds {
    bars: HashMap<Where, Find>,
    /// Where a write for a bar that has gone goes.
    ///
    /// **A bar can close while its box is still taking events.** freya emits every event
    /// of one press against the tree it measured before any of them ran, so the press
    /// that closes a bar is followed, in that same batch, by the box's own global press
    /// writing through a `Writable` mapped through this table by a key that has gone. The
    /// index has to answer for it, and it answers here -- into a bar nothing draws, and
    /// which cannot reopen one, an entry and not a flag being what says a bar is open.
    gone: Find,
}

impl Finds {
    pub(crate) fn get(&self, at: &Where) -> &Find {
        self.bars.get(at).unwrap_or(&self.gone)
    }

    fn get_mut(&mut self, at: &Where) -> &mut Find {
        let Finds { bars, gone } = self;
        bars.get_mut(at).unwrap_or(gone)
    }

    pub(crate) fn open(&self, at: &Where) -> bool {
        self.bars.contains_key(at)
    }

    /// Let go of every bar whose tab `keep` answers false for. What a closer owes
    /// ([`Places::forgetting`]): a bar holds the listing it is about, which is the file's
    /// bytes or the symbol's. Whether any went.
    pub(crate) fn forgetting(&mut self, keep: impl Fn(&Placing) -> bool) -> bool {
        let before = self.bars.len();
        self.bars.retain(|(placing, _), _| keep(placing));
        self.bars.len() != before
    }
}

/// The compiled matcher `at`'s rows wash themselves with: made once per pattern and shared by every row of it, since compiling a regex per row is not free.
/// `None` where no bar is open, which is what leaves a listing with no bar over it
/// untouched -- and what a pane mounted without the context draws with.
///
/// Two memos, so the regex is compiled only when this bar's pattern changes. The first
/// reads the whole table, as every read of one does, and wakes on every write to it -- a
/// keystroke in another bar, a step, each progress word of a hunt -- but it notifies only
/// when this bar's filter changed. The second compiles, and every compile is a new `Rc`
/// that redraws every row of the listing. `at` goes through `use_reactive`: a memo's
/// callback is built once, and a switch of tab hands this list another `at` without
/// mounting it again (`ui/split.rs`).
pub(crate) fn use_marking(at: Where) -> Option<Marking> {
    let finds = use_try_consume::<Looking>().map(|looking| looking.0);
    let at = use_reactive(&at);
    let filter = use_memo(move || {
        let at = *at.read();
        let bars = finds?.read();
        bars.open(&at).then(|| bars.get(&at).filter.clone())
    });
    let marking = use_memo(move || {
        filter
            .read()
            .as_ref()
            .map(|filter| Marking::new(filter.matcher()))
    });
    marking.read().clone()
}

/// Every pane's find bar, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Looking(pub(crate) State<Finds>);

/// Open `at`'s bar over `listing`, seeded with `seed` where the pane had a run inside one
/// line, and ask for the caret. A bar already open keeps what is in it unless there is a
/// seed. `listing` is `None` over an object's code, which is walked and not searched whole.
///
/// **The listing goes in here and not only through [`use_searching`]**, whose claim is
/// only made while a bar is open: a bar opened over a listing that was already drawn would
/// otherwise never learn what it searches, the claim having been declined before there was
/// anything to tell.
pub(crate) fn open_find(
    mut finds: State<Finds>,
    at: Where,
    seed: Option<String>,
    listing: Option<Searchable>,
) {
    let mut next = finds.peek().clone();
    let bar = next.bars.entry(at).or_default();
    bar.listing = listing;
    if let Some(seed) = seed.filter(|seed| !seed.is_empty()) {
        if bar.filter.pattern != seed {
            bar.filter.pattern = seed;
            bar.reset();
        }
    }
    bar.focus = true;
    finds.set_if_modified(next);
}

/// Close it, and with it the marks: nothing is left to draw them from.
pub(crate) fn close_find(mut finds: State<Finds>, at: Where) {
    let mut next = finds.peek().clone();
    if next.bars.remove(&at).is_some() {
        finds.set(next);
    }
}

/// Write into `at`'s bar, where it is open. The one way anything but the box edits one.
pub(crate) fn edit_find(mut finds: State<Finds>, at: Where, edit: impl FnOnce(&mut Find)) {
    let mut next = finds.peek().clone();
    if !next.open(&at) {
        return;
    }
    edit(next.get_mut(&at));
    finds.set_if_modified(next);
}

/// Claim `searchable` as what `at`'s bar searches, for as long as this scope is mounted:
/// how a pane asks for an answer, a view having no way to reach the request channel
/// (`agents/Worker.md`). A listing that is walked rather than searched whole claims
/// [`None`], which is what an object's code does and what makes its bar the walk's.
///
/// **The list claims it and not the pane**, as the object of a listing that is no tab is
/// claimed (`use_code_beside`): a list is mounted exactly while there is something to
/// search, so the claim being let go is a listing going away, and no pane has to work out
/// that it is drawing nothing.
///
/// A list mounted without the context -- a harness drawing one listing and nothing else --
/// claims nothing and has no bar.
///
/// **What is claimed is what the render hands in**, never what the scope was mounted with:
/// a switch of tab re-renders the list with another `at`, and following a call re-renders
/// it with another listing (`ui/split.rs`). A switch leaves the claim on the bar it left:
/// that tab still draws that listing.
pub(crate) fn use_searching(at: Where, searchable: Option<Searchable>) {
    let finds = use_try_consume::<Looking>().map(|looking| looking.0);
    let claim = move |at: Where, listing: Option<Searchable>| {
        let Some(mut finds) = finds else {
            return;
        };
        let mut next = finds.peek().clone();
        let same = match (&next.get(&at).listing, &listing) {
            (None, None) => true,
            (Some(a), Some(b)) => a.id() == b.id(),
            _ => false,
        };
        if !next.open(&at) || same {
            return;
        }
        let bar = next.get_mut(&at);
        bar.listing = listing;
        // Another listing is another set of hits, so the pane is on none of them.
        bar.reset();
        finds.set(next);
    };
    // The bar the drop lets go of: the one this list is over now.
    let over = use_hook(|| Rc::new(Cell::new(at)));
    over.set(at);
    use_side_effect_with_deps(
        &(at, searchable),
        move |(at, listing): &(Where, Option<Searchable>)| claim(*at, listing.clone()),
    );
    use_drop(move || claim(over.get(), None));
}

/// The slot a pane keeps for its find bar: the bar over `at` where one is open, and
/// nothing where none is.
///
/// **A component of its own, so that the question is asked in a scope of its own.**
/// Whether a bar is open is one key of [`Finds`], and there is no reading one key of a
/// table: the read is of the whole of it, and a keystroke in any bar wakes it. Asked in
/// the pane, that put the pane itself on the list, its heading and its listing with it.
/// Asked here, a keystroke in one pane's bar redraws this slot
/// and stops, the bar under it comparing equal.
///
/// Keyed by the pane, so a tab switch remounts it -- and the [`FindBar`] under it, whose
/// box is then seeded again from that pane's own bar rather than going on with the last
/// tab's.
#[derive(Clone, PartialEq)]
pub(crate) struct FindSlot {
    pub(crate) at: Where,
    pub(crate) key: DiffKey,
}

keyed!(FindSlot);

impl Component for FindSlot {
    fn render(&self) -> impl IntoElement {
        let finds = use_try_consume::<Looking>().map(|looking| looking.0);
        let open = finds.is_some_and(|finds| finds.read().open(&self.at));
        match open {
            true => FindBar {
                at: self.at,
                key: DiffKey::None,
            }
            .into_element(),
            // A box of no size rather than no child: the pane's column is built once and
            // the slot is what comes and goes inside it.
            false => rect().into_element(),
        }
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// That slot, as a pane puts it last in its own column.
pub(crate) fn find_bar_over(at: Where) -> Element {
    FindSlot {
        at,
        key: DiffKey::None,
    }
    .key(at)
    .into_element()
}

/// What the worker is asked and what it answers.
pub(crate) struct FindAsk {
    pub(crate) at: Where,
    pub(crate) listed: Searchable,
    pub(crate) filter: Filter,
}

pub(crate) struct FindAnswer {
    pub(crate) at: Where,
    pub(crate) about: About,
    pub(crate) hits: Shared<Hit>,
}

/// The whole of the work, so the hook below is the wiring and nothing else.
pub(crate) fn find_work(ask: FindAsk) -> FindAnswer {
    FindAnswer {
        at: ask.at,
        about: About::of(&ask.listed, &ask.filter),
        hits: look(&ask.listed, &ask.filter).into(),
    }
}

pub(crate) fn use_find(finds: State<Finds>) {
    use_find_with(finds, find_work);
}

/// The find worker, and the effect that asks it. `work` is an argument so a test can hold
/// it still, as every worker here does.
///
/// **Drained to the newest question per pane** ([`newest_by`]), since a pattern supersedes
/// on every keystroke, and the two panes of a tab do not drop each other's.
pub(crate) fn use_find_with(
    finds: State<Finds>,
    work: impl Fn(FindAsk) -> FindAnswer + Send + 'static,
) {
    let requests = use_worker(
        "the find worker",
        |first: FindAsk, queued| newest_by(first, queued, |ask| Some(ask.at)),
        move |ask| Some(work(ask)),
        move |answer: FindAnswer, _| {
            let mut finds = finds;
            let mut next = finds.peek().clone();
            if !next.open(&answer.at) {
                return;
            }
            if next.get_mut(&answer.at).take(answer.about, answer.hits) {
                finds.set(next);
            }
        },
    );

    // Every bar's owed question is one job, the bars being one state: what goes out is
    // whichever of them a question is owed for, and all of those are marked before any is
    // sent ([`use_asking`]).
    use_asking(
        move || {
            let wanted: Vec<(Where, Searchable, Filter)> = finds
                .read()
                .bars
                .iter()
                .filter_map(|(at, bar)| {
                    let (listed, filter) = bar.pending()?;
                    Some((*at, listed, filter))
                })
                .collect();
            (!wanted.is_empty()).then_some(wanted)
        },
        move |wanted: &Vec<(Where, Searchable, Filter)>| {
            write_if(finds, |next| {
                for (at, listed, filter) in wanted {
                    next.get_mut(at).asked = Some(About::of(listed, filter));
                }
                true
            });
        },
        move |wanted| {
            for (at, listed, filter) in wanted {
                requests.send(FindAsk { at, listed, filter });
            }
        },
    );
}

/// The bar along the bottom of a code pane: the box, the three toggles a filter bar has,
/// the count, and a button each way.
///
/// **Under the code and not over it.** It is the last child of the pane's own flex column,
/// so the listing above it is given what is left and the code makes room rather than being
/// covered -- and the listing's `on_sized` reports the shorter viewport, which is what a
/// page of rows and every reveal are measured in.
///
/// Keyed by the pane it is over, so moving to another tab remounts it and the box is
/// seeded again from that pane's own bar.
#[derive(Clone, PartialEq)]
pub(crate) struct FindBar {
    pub(crate) at: Where,
    pub(crate) key: DiffKey,
}

keyed!(FindBar);

counter!(
    /// Test-only: how many find bars this thread has drawn. Both bars of a tab can be
    /// open, and a bar drawn again draws what it drew before.
    pub(crate) fn bars_drawn() = BARS_DRAWN
);

impl Component for FindBar {
    fn render(&self) -> impl IntoElement {
        #[cfg(test)]
        BARS_DRAWN.set(BARS_DRAWN.get() + 1);

        let at = self.at;
        let finds = use_consume::<Looking>().0;
        let keyboard = use_consume::<Keyboard>();
        let box_id = use_hook(AccessibilityId::new_unique);
        // **This bar's own entry and not the table.** Both bars of a tab can be open, and
        // a read of [`Finds`] is a read of every bar in the app, so typing in one drew the
        // other. A memo notifies only when this entry changes; it is safe to key the
        // closure on `at` because the slot above is keyed by it, which resets these hooks
        // when it moves.
        let held = use_memo(move || finds.read().get(&at).clone());
        let bar = held.read().clone();

        // The box's own copy of what is typed, seeded from the bar when this mounts and
        // written back below. The editing buffer and not the answer: an `Input` wants a
        // `Writable<String>` of its own, and the bar is one entry of a table.
        let filter = use_state({
            let seed = bar.filter.clone();
            move || seed
        });
        let typed = filter.read().clone();
        use_side_effect_with_deps(&typed, move |typed: &Filter| {
            let typed = typed.clone();
            edit_find(finds, at, move |bar| {
                if bar.filter == typed {
                    return;
                }
                bar.filter = typed;
                // A new pattern is a new set of hits, so the pane is on none of them and
                // the next step reads the caret -- and the walk before it is given up,
                // its answer being about what was typed then.
                bar.reset();
            });
        });

        // The caret the opening asked for, spent here: the bar is mounted by now, which is
        // why the ask is a flag and not a `request_focus` in the handler that opened it.
        use_side_effect_with_deps(&bar.focus, move |wanted: &bool| {
            if *wanted {
                box_id.request_focus();
                edit_find(finds, at, |bar| bar.focus = false);
            }
        });

        // Compiled once per pattern: the bar is drawn on every write to its entry, which
        // is each progress word of a hunt.
        let marking = use_list_marking(filter);
        let error = marking.read().matcher().error().map(str::to_owned);
        let step = move |direction| edit_find(finds, at, move |bar| bar.step = Some(direction));
        let close = move || {
            close_find(finds, at);
            // Out of the guard first: an `if let` holds its scrutinee for the whole body.
            let pane_box = keyboard.keys.peek().pane_box(at.1);
            if let Some(pane) = pane_box {
                pane.request_focus();
            }
        };

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(top_hairline())
            // The three chords the toggles are pressed by, the same three in all four
            // boxes. On the bar and not in the box's own hook: `box_keys` declines every
            // chord before it, so a chord arrives here by bubbling out of the box, and
            // this rect is the ancestor it bubbles to (`ui/filter_bar.rs`).
            .on_key_down(move |e: Event<KeyboardEventData>| {
                if let Some(toggle) = Toggle::pressed(&e.key, e.modifiers) {
                    let mut filter = filter;
                    toggle.flip(&mut filter.write());
                }
            })
            .child(
                rect()
                    .width(Size::fill())
                    // Exactly as a filter bar's row is.
                    .height(Size::px(text_box_height()))
                    .horizontal()
                    .content(Content::Flex)
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 5.0))
                    .spacing(2.0)
                    .child(
                        Input::new(
                            filter
                                .into_writable()
                                .map(|filter| &filter.pattern, |filter| &mut filter.pattern),
                        )
                        .placeholder("Find")
                        .compact()
                        .width(Size::flex(1.0))
                        .a11y_id(box_id)
                        // Enter steps and Escape closes; the window's chords are
                        // declined for the whole app in one place (`chords.rs`).
                        .on_pre_key_down(box_keys(
                            Boxed::Input,
                            &[],
                            move |key, modifiers: Modifiers| match key {
                                Key::Named(NamedKey::Enter) => {
                                    step(match modifiers.contains(Modifiers::SHIFT) {
                                        true => Direction::Back,
                                        false => Direction::Forward,
                                    })
                                }
                                Key::Named(NamedKey::Escape) => close(),
                                _ => {}
                            },
                        ))
                        .maybe(error.is_some(), |input| {
                            input
                                .color(palette().invalid_fg)
                                .focus_border_fill(palette().invalid_fg)
                        }),
                    )
                    .children(Toggle::ALL.map(|toggle| {
                        FilterToggle {
                            filter,
                            toggle,
                            on: toggle.is_on(&typed),
                        }
                        .into()
                    }))
                    .child(
                        label()
                            .text(counted(&bar, error.is_some()))
                            .color(faded(palette().text_fg, palette().header_bg))
                            .max_lines(1),
                    )
                    .child(StepButton {
                        at,
                        direction: Direction::Back,
                    })
                    .child(StepButton {
                        at,
                        direction: Direction::Forward,
                    }),
            )
            .maybe_child(error.map(invalid_line))
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// What the bar says about the answer: which match the pane is on and how many there are,
/// the count alone before a step has landed, and that a pattern matched nothing.
///
/// Nothing at all where there is no answer yet: a bar just opened, and one over a listing
/// the worker has not been asked about, would otherwise say "No matches" about a search
/// that has not run.
///
/// `broken` is whether the pattern in the box will not compile, as the error under it says.
fn counted(bar: &Find, broken: bool) -> String {
    // An object's code has no count: it is read a piece at a time, so what there is to
    // say is how far the walk has got, and that one came back with nothing.
    if let Some(hunt) = &bar.hunt {
        return match &hunt.walked {
            Walked::Walking(through) => format!("{}%", (through * 100.0).round() as u32),
            Walked::Nothing => "No matches".to_owned(),
            Walked::Found(..) => String::new(),
        };
    }
    // A pattern that will not compile has the error under the box already; a count beside
    // it saying nothing matched would read as an answer about the pattern.
    let Some(hits) = bar.hits().filter(|_| !broken) else {
        return String::new();
    };
    match (bar.at, hits.len()) {
        (_, 0) => "No matches".to_owned(),
        (Some(at), total) => format!("{} of {total}", at + 1),
        (None, total) => total.to_string(),
    }
}

/// One of the bar's two step buttons.
///
/// It writes the step itself rather than being handed a closure to call. An
/// `EventHandler` prop never compares equal, so a button holding one was re-rendered by
/// every render of the bar -- which is every write to `Finds`: every keystroke, every
/// progress word of a hunt, every step. The pane and the direction are all the write
/// needs, and both are the same on every render.
#[derive(Clone, PartialEq)]
struct StepButton {
    at: Where,
    direction: Direction,
}

impl Component for StepButton {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        // Consumed in the render, as every context is: the press below runs no hook.
        let finds = use_consume::<Looking>().0;
        let (at, direction) = (self.at, self.direction);
        let (glyph, says) = match self.direction {
            Direction::Back => ("\u{2039}", "Previous match"),
            Direction::Forward => ("\u{203a}", "Next match"),
        };

        TooltipContainer::new(Tooltip::new(says)).child(
            bar_button(hovering, true, Glow::No)
                .on_press(move |e: Event<PressEventData>| {
                    // As a toggle does: the box beside this gives up its keyboard focus
                    // from the global press, which is cancellable and sorts last.
                    e.prevent_default();
                    edit_find(finds, at, move |bar| bar.step = Some(direction));
                })
                .child(label().text(glyph).max_lines(1)),
        )
    }
}

/// What a code pane answers over and above its own keys: **Ctrl+F opens the bar over it**,
/// seeded with the run picked out inside one line, and **F3 and Shift+F3 step through
/// what it found** without the reader leaving the code.
///
/// Wrapped around the pane's own handler rather than folded into it: the chords belong to
/// the bar, and `on_listing_key` goes on being the whole of what a listing's keys are.
/// They are answered on the pane's own focusable box for the reason a list answers Ctrl+F
/// on its rows (`filter_bar.rs`): a key event reaches the node that has the keyboard, so
/// the bar a reader opens, and the one they step through, is the one they were reading.
///
/// A step is the same one [`FindBar`]'s Enter asks for -- the bar owns it, and this only
/// puts the ask in -- and it is [`edit_find`] that makes a pane with no bar do nothing at
/// all: there is no entry to write the ask into.
///
/// A hook, reaching for [`Looking`], so [`use_listing_keys`] calls it once and on every
/// render.
pub(crate) fn use_find_chord(
    at: Where,
    marked: State<Marks>,
    listing: Option<Searchable>,
    text: Rc<dyn Fn(usize) -> Line>,
    mut keys: impl FnMut(Event<KeyboardEventData>) + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    let finds = use_try_consume::<Looking>().map(|looking| looking.0);
    move |e: Event<KeyboardEventData>| {
        let Some(finds) = finds else {
            return keys(e);
        };
        let direction = match Chord::of(&e.key, e.modifiers) {
            Some(Chord::Find) => {
                let seed = seed_of(&marked.peek(), at.1, &*text);
                return open_find(finds, at, seed, listing.clone());
            }
            Some(Chord::FindNext) => Direction::Forward,
            Some(Chord::FindPrevious) => Direction::Back,
            _ => return keys(e),
        };
        edit_find(finds, at, move |bar| bar.step = Some(direction));
    }
}

/// What Ctrl+F puts in the box: the characters picked out in `pane`, where they are all on
/// one line.
///
/// **One line and no more.** A run of rows is a page of disassembly and not a search term,
/// so a selection crossing lines leaves the box as it was -- as does none at all, and a
/// caret, which selects nothing.
fn seed_of(marks: &Marks, pane: Pane, text: impl Fn(usize) -> Line) -> Option<String> {
    let picked = marks.of(pane).as_ref()?;
    if picked.chars.is_empty() {
        return None;
    }
    let (from, to) = picked.chars.ends();
    if from.row != to.row {
        return None;
    }
    let line = text(from.row);
    let seed = line.slice(from.col, to.col);
    (!seed.is_empty()).then(|| seed.to_owned())
}

/// Make the step the bar over `at` asked for: move to the next hit, pick it out, and bring
/// it into view.
///
/// **In the list and not in the bar**, because the answer is in rows: only the list knows
/// how far to scroll to reach one. `file` is what a run of this listing is a run of -- the
/// source list's own file, and `None` for the assembly's, where a run's file is the row's.
///
/// **A bar with no listing has no step of this one's to spend.** [`use_listing_keys`] calls
/// this for all three listings, a hook having to run on every render, and an object's code
/// is searched by walking it rather than by a pass over a listing the pane holds: that step
/// is [`use_code_hunt`]'s, and the two tell theirs apart by whether there is a listing.
pub(crate) fn use_find_steps<R: FnMut(usize) + 'static>(
    at: Where,
    marked: State<Marks>,
    file: Option<Arc<Path>>,
    reveal: R,
) {
    let finds = use_try_consume::<Looking>().map(|looking| looking.0);
    // The reveal this render made, which knows how long the listing is now. The effect's
    // callback is built once, so a reveal it captured would clamp against the first
    // listing's rows; `at` and `file` come in as its deps for the same reason.
    let latest = use_hook(|| Rc::new(RefCell::new(None::<R>)));
    *latest.borrow_mut() = Some(reveal);
    use_side_effect_with_deps(
        &(at, file),
        move |(at, file): &(Where, Option<Arc<Path>>)| {
            let at = *at;
            let Some(mut finds) = finds else {
                return;
            };
            // Bound before the write below, the read being a guard.
            let bar = finds.read().get(&at).clone();
            let Some(direction) = bar.step.filter(|_| bar.listing.is_some()) else {
                return;
            };
            // A step asked before the answer came is left for the answer's write to wake
            // this again, or the reader's Enter straight after typing would go nowhere.
            if bar.owed() {
                return;
            }
            // Where the pane is: its run, or a caret at the top of the listing where there
            // is no run at all.
            let run = marked
                .peek()
                .of(at.1)
                .as_ref()
                .map(|picked| picked.chars)
                .unwrap_or(CharSelection::at(Caret { row: 0, col: 0 }));
            let hits = bar.hits().cloned();
            let next = hits
                .as_ref()
                .and_then(|hits| find::step(hits, bar.at, run, direction));

            let mut state = finds.peek().clone();
            let entry = state.get_mut(&at);
            entry.step = None;
            entry.at = next;
            finds.set(state);

            // A pattern nothing matched moves nothing: the bar says so instead.
            let Some(hit) = next.and_then(|next| hits.as_ref().and_then(|hits| hits.get(next)))
            else {
                return;
            };
            mark_columns(marked, at.1, file.clone(), hit.row, hit.columns.clone());
            if let Some(reveal) = latest.borrow_mut().as_mut() {
                reveal(hit.row);
            }
        },
    );
}

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

use super::*;
use crate::find::{self, Hit};

/// Which bar: the listing it is drawn in -- a tab, or the scratchpad's -- and which of the
/// two panes.
pub(crate) type Where = (Placing, Pane);

/// A listing a pane can be searched whole, and what a worker is handed to do it.
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
    /// An object's whole code, which is **not** searched whole: it is read a piece at a
    /// time, so a step walks on from where the pane is until it finds a match. The
    /// skeleton is no part of this: what is claimed here says only *which* listing the
    /// bar is over, and [`use_code_hunt`] is handed the one the view holds.
    Code(Arc<Object>),
}

impl Searchable {
    /// Which listing this is, by pointer: what an answer is judged against, so one about
    /// the file or the symbol a pane has moved off is dropped.
    pub(crate) fn id(&self) -> usize {
        match self {
            Searchable::Source(source) => Arc::as_ptr(&source.0).addr(),
            Searchable::Symbol { assembly, .. } => Arc::as_ptr(assembly).addr(),
            Searchable::Code(object) => Arc::as_ptr(object).addr(),
        }
    }

    /// Whether this listing is searched by walking it rather than by a pass over it.
    fn walked(&self) -> bool {
        matches!(self, Searchable::Code(_))
    }
}

/// Every hit in `listed`, in the rows the pane draws. The whole of the work, and it
/// touches no UI state.
pub(crate) fn look(listed: &Searchable, filter: &Filter) -> Vec<Hit> {
    let matcher = filter.matcher();
    let mut hits = Vec::new();
    let mut push = |row: usize, line: &Line| {
        hits.extend(
            find::hits_in(line, &matcher)
                .into_iter()
                .map(|columns| Hit { row, columns }),
        );
    };

    match listed {
        Searchable::Source(source) => {
            for row in 0..source.0.lines {
                push(row, &source_line(source, row));
            }
        }
        Searchable::Symbol { assembly, lanes } => {
            for index in 0..assembly.instructions.len() {
                push(lanes.row_of(index), &instruction_line(assembly, index));
            }
        }
        // Never asked: an object's code is walked, not passed over ([`Find::pending`]).
        Searchable::Code(_) => {}
    }
    hits
}

/// What a bar has of its listing: the hits of one searched whole, or the walk through an
/// object's code.
///
/// One field and not two, so a listing cannot be both: it is searched whole or walked.
#[derive(Clone, Default)]
enum Sought {
    /// Neither: nothing has answered, and no walk has been asked for.
    #[default]
    Nothing,
    /// A listing searched whole: which listing and pattern the hits are about, and the
    /// hits in order.
    Hits(usize, Filter, Shared<Hit>),
    /// The walk a step through an object's code asked for ([`Hunt`]).
    Walk(Hunt),
}

/// One pane's find bar: what is typed, what the worker said about it, and where the pane
/// has got to in the answer.
#[derive(Clone, Default)]
pub(crate) struct Find {
    /// The box and its three toggles.
    pub(crate) filter: Filter,
    /// The listing the pane is drawing, written by the pane. `None` while it has none to
    /// search -- an object's code, or a pane with nothing in it.
    pub(crate) listing: Option<Searchable>,
    /// What was last asked, so a question in flight is not asked again every render.
    asked: Option<(usize, Filter)>,
    /// What came back, whichever way this listing is searched.
    sought: Sought,
    /// Which hit the pane is on, `None` until a step has landed. A new pattern clears it,
    /// so the next step starts from the caret and not from wherever the last one ended.
    pub(crate) at: Option<usize>,
    /// A step the bar has asked for and the pane has not made yet, and which way. Spent
    /// by the list, which is what knows where its rows are and can scroll to one.
    pub(crate) step: Option<bool>,
    /// The caret the opening asked for, spent by the box once it is mounted.
    pub(crate) focus: bool,
}

impl Find {
    /// The hits, where the answer is about the listing and pattern being asked about now.
    /// A stale answer draws nothing rather than the last file's marks.
    pub(crate) fn hits(&self) -> Option<&Shared<Hit>> {
        let listing = self.listing.as_ref()?.id();
        match &self.sought {
            Sought::Hits(about, filter, hits) if *about == listing && *filter == self.filter => {
                Some(hits)
            }
            _ => None,
        }
    }

    /// The walk, where the bar is on one.
    pub(crate) fn hunt(&self) -> Option<&Hunt> {
        match &self.sought {
            Sought::Walk(hunt) => Some(hunt),
            _ => None,
        }
    }

    fn hunt_mut(&mut self) -> Option<&mut Hunt> {
        match &mut self.sought {
            Sought::Walk(hunt) => Some(hunt),
            _ => None,
        }
    }

    /// Let go of the walk, where the bar is on one: what it says next is about a pattern
    /// or a listing that has gone, and the take that finds it gone drops the receiver,
    /// which is what calls the walk off. Hits are left where they are, stale or not:
    /// `hits` is what refuses a stale answer.
    fn stop_walking(&mut self) {
        if matches!(self.sought, Sought::Walk(_)) {
            self.sought = Sought::Nothing;
        }
    }

    /// What a question is owed for, or `None` where the answer is already about it or one
    /// is in flight. No `pending` flag of its own: a listing is being searched exactly
    /// while it is wanted and neither the answer nor the ask is about it ([`Coded`]).
    fn pending(&self) -> Option<(Searchable, Filter)> {
        // Nothing typed marks nothing, so there is nothing to ask: an empty pattern is
        // not a search that finds everything (`Matcher::Everything`).
        if self.filter.pattern.is_empty() {
            return None;
        }
        let listed = self.listing.clone().filter(|listed| !listed.walked())?;
        let about = (listed.id(), self.filter.clone());
        if self.asked.as_ref() == Some(&about) || self.hits().is_some() {
            return None;
        }
        Some((listed, self.filter.clone()))
    }

    /// Take `hits` as the answer about `listing` and `filter`, and say whether anything
    /// changed. Refused where the pane has moved on, which is the whole of the
    /// supersession rule: a comparison and not a generation count.
    fn take(&mut self, listing: usize, filter: Filter, hits: Shared<Hit>) -> bool {
        if self.listing.as_ref().map(Searchable::id) != Some(listing) || self.filter != filter {
            return false;
        }
        self.sought = Sought::Hits(listing, filter, hits);
        true
    }
}

/// Every pane's bar, keyed by the pane. An entry is what "the bar is open" means: opening
/// puts one in and closing takes it out.
///
/// Keyed by the **tab** and not by a place on its trail, so a step Back leaves the bar as
/// the reader left it: what was typed lasts as long as the tab is open.
#[derive(Clone, Default)]
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
    /// bytes or the symbol's.
    pub(crate) fn forgetting(&mut self, keep: impl Fn(&Placing) -> bool) {
        self.bars.retain(|(placing, _), _| keep(placing));
    }
}

/// The compiled matcher `at`'s rows wash themselves with: made once per render of the
/// list and shared by every row of it, since compiling a regex per row is not free.
/// `None` where no bar is open, which is what leaves a listing with no bar over it
/// untouched -- and what a pane mounted without the context draws with.
///
/// A `Memo`, so a render that changed nothing about the pattern hands the rows the same
/// one and leaves their props as they were.
pub(crate) fn use_marking(at: Where) -> Option<Marking> {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    let marking = use_memo(move || {
        let bars = finds?.read();
        bars.open(&at)
            .then(|| Marking::new(bars.get(&at).filter.matcher()))
    });
    marking.read().clone()
}

/// Every pane's find bar, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Looking(pub(crate) State<Finds>);

/// Open `at`'s bar over `listing`, seeded with `seed` where the pane had a run inside one
/// line, and ask for the caret. A bar already open keeps what is in it unless there is a
/// seed.
///
/// **The listing goes in here and not only through [`use_searching`]**, whose claim is
/// only made while a bar is open: a bar opened over a listing that was already drawn would
/// otherwise never learn what it searches, the claim having been declined before there was
/// anything to tell.
pub(crate) fn open_find(
    mut finds: State<Finds>,
    at: Where,
    seed: Option<String>,
    listing: Searchable,
) {
    let mut next = finds.peek().clone();
    let bar = next.bars.entry(at).or_default();
    bar.listing = Some(listing);
    if let Some(seed) = seed.filter(|seed| !seed.is_empty()) {
        if bar.filter.pattern != seed {
            bar.filter.pattern = seed;
            bar.at = None;
        }
    }
    bar.focus = true;
    finds.set(next);
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
    finds.set(next);
}

/// Claim `searchable` as what `at`'s bar searches, for as long as this scope is mounted:
/// how a pane asks for an answer, a view having no way to reach the request channel
/// (`agents/Worker.md`).
///
/// **The list claims it and not the pane**, as the object of a listing that is no tab is
/// claimed (`use_code_beside`): a list is mounted exactly while there is something to
/// search, so the claim being let go is a listing going away, and no pane has to work out
/// that it is drawing nothing.
///
/// A list mounted without the context -- a harness drawing one listing and nothing else --
/// claims nothing and has no bar.
pub(crate) fn use_searching(at: Where, searchable: Searchable) {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    let claim = move |listing: Option<Searchable>| {
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
        bar.at = None;
        bar.stop_walking();
        finds.set(next);
    };
    let held = claim.clone();
    use_side_effect_with_deps(&searchable.id(), move |_: &usize| {
        held(Some(searchable.clone()))
    });
    use_drop(move || claim(None));
}

/// The bar over `at`, where one is open: what a pane puts last in its own column.
///
/// Keyed by the pane, so a tab switch remounts it and its box is seeded again from that
/// pane's own bar rather than going on with the last tab's.
pub(crate) fn find_bar_over(at: Where) -> Option<Element> {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    let open = finds.is_some_and(|finds| finds.read().open(&at));
    open.then(|| {
        FindBar {
            at,
            key: DiffKey::None,
        }
        .key(at)
        .into_element()
    })
}

/// What the worker is asked and what it answers.
pub(crate) struct FindAsk {
    pub(crate) at: Where,
    pub(crate) listed: Searchable,
    pub(crate) filter: Filter,
}

pub(crate) struct FindAnswer {
    pub(crate) at: Where,
    pub(crate) listing: usize,
    pub(crate) filter: Filter,
    pub(crate) hits: Shared<Hit>,
}

/// The whole of the work, so the hook below is the wiring and nothing else.
pub(crate) fn find_work(ask: FindAsk) -> FindAnswer {
    FindAnswer {
        at: ask.at,
        listing: ask.listed.id(),
        filter: ask.filter.clone(),
        hits: look(&ask.listed, &ask.filter).into(),
    }
}

pub(crate) fn use_find(finds: State<Finds>) {
    use_find_with(finds, find_work);
}

/// The find worker, and the effect that asks it. `work` is an argument so a test can hold
/// it still, as every worker here does.
///
/// **Drained to the newest question per pane**, since a pattern supersedes on every
/// keystroke: what the reader has typed past is dropped without being started, and the two
/// panes of a tab do not drop each other's.
pub(crate) fn use_find_with(
    finds: State<Finds>,
    work: impl Fn(FindAsk) -> FindAnswer + Send + 'static,
) {
    let requests = use_worker(
        "the find worker",
        |first: FindAsk, queued, _| {
            let mut newest: Vec<FindAsk> = vec![first];
            while let Some(ask) = queued() {
                match newest.iter_mut().find(|kept| kept.at == ask.at) {
                    Some(kept) => *kept = ask,
                    None => newest.push(ask),
                }
            }
            newest
        },
        move |ask| Some(work(ask)),
        move |answer: FindAnswer, _| {
            let mut finds = finds;
            let mut next = finds.peek().clone();
            if !next.open(&answer.at) {
                return;
            }
            if next
                .get_mut(&answer.at)
                .take(answer.listing, answer.filter, answer.hits)
            {
                finds.set(next);
            }
        },
    );

    use_side_effect(move || {
        // Read and not peeked: a pattern typed and a listing written are the two things
        // that wake this. Bound before the writes, the guard being a read.
        let wanted: Vec<(Where, Searchable, Filter)> = finds
            .read()
            .bars
            .iter()
            .filter_map(|(at, bar)| {
                let (listed, filter) = bar.pending()?;
                Some((*at, listed, filter))
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        let mut finds = finds;
        let mut next = finds.peek().clone();
        for (at, listed, filter) in wanted {
            next.get_mut(&at).asked = Some((listed.id(), filter.clone()));
            requests.send(FindAsk { at, listed, filter });
        }
        finds.set(next);
    });
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

impl KeyExt for FindBar {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for FindBar {
    fn render(&self) -> impl IntoElement {
        let at = self.at;
        let finds = use_consume::<Looking>().0;
        let keyboard = use_consume::<Keyboard>().0;
        let box_id = use_hook(AccessibilityId::new_unique);
        let bar = finds.read().get(&at).clone();

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
                bar.at = None;
                bar.stop_walking();
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

        let error = typed.matcher().error().map(str::to_owned);
        let step = move |back: bool| edit_find(finds, at, move |bar| bar.step = Some(back));
        let close = move || {
            close_find(finds, at);
            if let Some(pane) = pane_box(keyboard, at.1) {
                pane.request_focus();
            }
        };

        rect()
            .width(Size::fill())
            .background(palette().header_bg)
            .border(top_hairline())
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
                                    step(modifiers.contains(Modifiers::SHIFT))
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
                            .text(counted(&bar))
                            .color(faded(palette().text_fg, palette().header_bg))
                            .max_lines(1),
                    )
                    .child(StepButton {
                        back: true,
                        step: EventHandler::new(move |_| step(true)),
                    })
                    .child(StepButton {
                        back: false,
                        step: EventHandler::new(move |_| step(false)),
                    }),
            )
            .maybe_child(error.map(|error| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 6.0, 5.0, 6.0))
                    .overflow(Overflow::Clip)
                    .child(label().text(error).color(palette().invalid_fg).max_lines(1))
            }))
    }
}

/// What the bar says about the answer: which match the pane is on and how many there are,
/// the count alone before a step has landed, and that a pattern matched nothing.
///
/// Nothing at all where there is no answer yet: a bar just opened, and one over a listing
/// the worker has not been asked about, would otherwise say "No matches" about a search
/// that has not run.
fn counted(bar: &Find) -> String {
    // An object's code has no count: it is read a piece at a time, so what there is to
    // say is how far the walk has got, and that one came back with nothing.
    if let Some(hunt) = bar.hunt() {
        return match &hunt.walked {
            Walked::Walking(through) => format!("{}%", (through * 100.0).round() as u32),
            Walked::Nothing => "No matches".to_owned(),
            Walked::Found(..) => String::new(),
        };
    }
    // A pattern that will not compile has the error under the box already; a count beside
    // it saying nothing matched would read as an answer about the pattern.
    let Some(hits) = bar
        .hits()
        .filter(|_| bar.filter.matcher().error().is_none())
    else {
        return String::new();
    };
    match (bar.at, hits.len()) {
        (_, 0) => "No matches".to_owned(),
        (Some(at), total) => format!("{} of {total}", at + 1),
        (None, total) => total.to_string(),
    }
}

/// One of the bar's two step buttons.
#[derive(Clone, PartialEq)]
struct StepButton {
    back: bool,
    step: EventHandler<()>,
}

impl Component for StepButton {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let step = self.step.clone();
        let (glyph, says) = match self.back {
            true => ("\u{2039}", "Previous match"),
            false => ("\u{203a}", "Next match"),
        };

        TooltipContainer::new(Tooltip::new(says)).child(
            rect()
                .width(Size::px(toggle_size()))
                .height(Size::px(toggle_size()))
                .center()
                .corner_radius(4.0)
                .background(match hovering() {
                    true => palette().toggle_hover_bg,
                    false => Color::TRANSPARENT,
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |e: Event<PressEventData>| {
                    // As a toggle does: the box beside this gives up its keyboard focus
                    // from the global press, which is cancellable and sorts last.
                    e.prevent_default();
                    step.call(());
                })
                .child(label().text(glyph).max_lines(1)),
        )
    }
}

/// What a code pane answers over and above its own keys: **Ctrl+F opens the bar over it**,
/// seeded with the run picked out inside one line.
///
/// Wrapped around the pane's own handler rather than folded into it: the chord belongs to
/// the bar, and `on_listing_key` goes on being the whole of what a listing's keys are.
/// The chord is answered on the pane's own focusable box for the reason a list answers it
/// on its rows (`filter_bar.rs`): a key event reaches the node that has the keyboard, so
/// the bar a reader opens is the one they were reading.
pub(crate) fn find_chord(
    at: Where,
    marked: State<Marks>,
    listing: Searchable,
    text: impl Fn(usize) -> Line + 'static,
    mut keys: impl FnMut(Event<KeyboardEventData>) + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    move |e: Event<KeyboardEventData>| {
        let Some(finds) = finds.filter(|_| Chord::Find.is(&e.key, e.modifiers)) else {
            return keys(e);
        };
        let seed = seed_of(&marked.peek(), at.1, &text);
        open_find(finds, at, seed, listing.clone());
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
    Some(text(from.row).slice(from.col, to.col)).filter(|seed| !seed.is_empty())
}

/// Make the step the bar over `at` asked for: move to the next hit, pick it out, and bring
/// it into view.
///
/// **In the list and not in the bar**, because the answer is in rows: only the list knows
/// how far to scroll to reach one. `file` is what a run of this listing is a run of -- the
/// source list's own file, and `None` for the assembly's, where a run's file is the row's.
pub(crate) fn use_find_steps(
    at: Where,
    marked: State<Marks>,
    file: Option<Arc<str>>,
    mut reveal: impl FnMut(usize) + 'static,
) {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    use_side_effect(move || {
        let Some(mut finds) = finds else {
            return;
        };
        // Bound before the write below, the read being a guard.
        let bar = finds.read().get(&at).clone();
        let Some(back) = bar.step else {
            return;
        };
        // Where the pane is, for a first step: the caret, or the top of the listing where
        // there is no run at all.
        let caret = marked
            .peek()
            .of(at.1)
            .as_ref()
            .map(|picked| picked.chars.lead())
            .unwrap_or(Caret { row: 0, col: 0 });
        let hits = bar.hits().cloned();
        let next = hits
            .as_ref()
            .and_then(|hits| find::step(hits, bar.at, caret, back));

        let mut state = finds.peek().clone();
        let entry = state.get_mut(&at);
        entry.step = None;
        entry.at = next;
        finds.set(state);

        // A pattern nothing matched moves nothing: the bar says so instead.
        let Some(hit) = next.and_then(|next| hits.as_ref().and_then(|hits| hits.get(next))) else {
            return;
        };
        mark_columns(marked, at.1, file.clone(), hit.row, hit.columns.clone());
        reveal(hit.row);
    });
}

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
    pub(crate) filter: Filter,
    pub(crate) back: bool,
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

/// Walk `object`'s code for the next match of `filter` from `from`, in the direction
/// `back` says, and say how far it has got as it goes.
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
    back: bool,
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
        let flat = match back {
            false => (first + step) % total,
            true => (first + total - step % total) % total,
        };
        if step % SAID_EVERY == 0 {
            let through = step as f32 / total as f32;
            if emit(Hunted::Through(through)).is_break() {
                return;
            }
        }

        let mut lines = stretch_lines(object, &index, flat);
        // In the order the listing draws them, and backwards for a walk that way, so the
        // match found is the nearest one behind the reader and not the first of a stretch.
        lines.sort_by_key(|(address, _)| *address);
        if back {
            lines.reverse();
        }
        for (address, line) in lines {
            // The stretch the walk started in holds the reader's own place: only what is
            // past it counts, or a step would find the match the pane is already on.
            if step == 0 || (step == last && flat == first) {
                let past = match back {
                    false => address > from,
                    true => address < from,
                };
                if !past {
                    continue;
                }
            }
            let mut hits = find::hits_in(&line, &matcher);
            if back {
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

/// Every line stretch `flat` holds, as the pane draws them: the placed address each sits
/// at and its text.
///
/// **The same text the pane draws**, through the same builders (`section_view.rs`): a
/// search that built its own would find what the reader cannot see, or miss what they can.
fn stretch_lines(object: &Object, index: &section::Flat, flat: usize) -> Vec<(u64, Line)> {
    let Some((place, stretch)) = index.stretch(flat) else {
        return Vec::new();
    };
    let Some(placed) = index.code().sections().get(place.section) else {
        return Vec::new();
    };

    let start = placed.place(stretch.range.start);
    let mut lines: Vec<(u64, Line)> = Vec::new();
    // The section's own header stands over its first stretch.
    if place.stretch == 0 {
        lines.push((
            placed.range().start,
            Line::text(section_view::header_text(placed)),
        ));
    }
    for symbol in &stretch.symbols {
        lines.push((start, Line::text(section_view::label_text(symbol))));
    }

    let Some(decoded) = index.code().decode(object, place) else {
        return lines;
    };
    if let Some(assembly) = &decoded.code {
        let bias = placed.bias();
        for index in 0..assembly.instructions.len() {
            let address = assembly.instructions[index].address.wrapping_add(bias);
            lines.push((address, instruction_line(assembly, index)));
        }
    }
    if let Some(gap) = &decoded.gap {
        for index in 0.. {
            let Some((address, bytes)) = section_view::gap_row_bytes(placed, &gap.range, index)
            else {
                break;
            };
            let (mark, text) = section_view::dump_line(&bytes);
            lines.push((address, section_view::text_line(Some(mark), &text)));
        }
    }
    lines
}

/// The walk through an object's code that a step over one asks for: started here, taken
/// here, and landed by `land`.
///
/// **Its own hook and not [`use_find_steps`]**, which steps through an answer the pane
/// already holds. There is no such answer here: what a step asks for is one address, found
/// by reading on, and the bar shows how far the reading has got instead of a count.
///
/// `from` is where the pane is, as an address; a listing with no caret in it yet starts at
/// the top. `land` is given the match, and is the section view's own: only it can put a
/// caret on the row an address is in, the rows being counted afresh as stretches decode.
pub(crate) fn use_code_hunt(
    at: Where,
    object: Arc<Object>,
    code: Option<Arc<CodeListing>>,
    from: impl Fn() -> u64 + 'static,
    mut land: impl FnMut(u64, Range<usize>) -> bool + 'static,
) {
    let finds = try_consume_context::<Looking>().map(|looking| looking.0);
    let mut walks = use_state(|| 0u64);

    // A step over an object's code starts a walk rather than moving through an answer.
    use_side_effect(move || {
        let Some(mut finds) = finds else {
            return;
        };
        let bar = finds.read().get(&at).clone();
        let Some(back) = bar
            .step
            .filter(|_| bar.listing.as_ref().is_some_and(|l| l.walked()))
        else {
            return;
        };
        let id = walks.peek().wrapping_add(1);
        walks.set(id);
        let mut state = finds.peek().clone();
        let entry = state.get_mut(&at);
        entry.step = None;
        entry.sought = Sought::Walk(Hunt {
            id,
            filter: bar.filter.clone(),
            back,
            from: from(),
            walked: Walked::Walking(0.0),
        });
        finds.set(state);
    });

    // The walk itself. A memo over which walk it is, not a read: every word it says about
    // its progress is a write to the state below, and an effect reading that would start
    // a walk per word.
    let asked = use_memo(move || {
        let finds = finds?;
        let bar = finds.read();
        let hunt = bar.get(&at).hunt()?;
        hunt.walking()
            .then_some((hunt.id, hunt.filter.clone(), hunt.from, hunt.back))
    });
    let started = asked.read().clone();
    use_side_effect_with_deps(&started, move |walk: &Option<(u64, Filter, u64, bool)>| {
        let (Some((id, filter, from, back)), Some(finds)) = (walk.clone(), finds) else {
            return;
        };
        let object = object.clone();
        let code = code.clone();
        let events = stream("the code search", Some(64), move |emit| {
            // The skeleton the view already has, or one built here: it is free
            // (`CodeListing`), and a walk asked for before the view has one must not wait.
            let code = code.unwrap_or_else(|| Arc::new(CodeListing::new(&object)));
            hunt(&object, &code, &filter, from, back, emit);
        });
        spawn(take_hunt(finds, at, id, events));
    });

    // The match, landed once. The walk that found it is remembered, so an effect woken
    // again -- by the pane's own rows arriving, say -- does not land it a second time.
    let mut landed = use_state(|| None::<u64>);
    use_side_effect(move || {
        let Some(finds) = finds else {
            return;
        };
        let hunt = finds.read().get(&at).hunt().cloned();
        let Some(hunt) = hunt else {
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
    });
}

/// Take what a walk says, for as long as it is the walk the bar is on.
///
/// The receiver dropping is what stops the worker, so returning early is how a walk the
/// reader has moved on from is called off (`search_view.rs`).
async fn take_hunt(
    mut finds: State<Finds>,
    at: Where,
    id: u64,
    events: async_channel::Receiver<Hunted>,
) {
    while let Some(batch) = next_batch(&events).await {
        let mut state = finds.peek().clone();
        let Some(hunt) = state.get_mut(&at).hunt_mut().filter(|hunt| hunt.id == id) else {
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
        finds.set(state);
        if done {
            return;
        }
    }
    // The walk ended without finding anything: the bar says so rather than going on
    // saying how far it has got.
    let mut state = finds.peek().clone();
    let Some(hunt) = state
        .get_mut(&at)
        .hunt_mut()
        .filter(|hunt| hunt.id == id && hunt.walking())
    else {
        return;
    };
    hunt.walked = Walked::Nothing;
    finds.set(state);
}

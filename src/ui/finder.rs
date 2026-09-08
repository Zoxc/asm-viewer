//! The file finder: the box Ctrl+P opens over the app, the files of the project's
//! directory under it, and the one worker that walks them and picks them out.
//!
//! `SearchTab`'s shape over `src/walk.rs`'s walk -- a question, one answer that stands
//! until the next replaces it, and a thread of the app's own that answers it -- with three
//! things settled differently.
//!
//! **The walked files never reach the UI thread.** They are the worker's, and what
//! crosses is the rows it picked out for a query. A list of a project's files is tens of
//! thousands of paths, and both the things that were done with it here were paid for a
//! frame at a time: appending a batch of a walk to a shared `Arc` copied the whole list
//! per batch, and matching the box against it was a pass over every path per keystroke.
//!
//! **The list is kept.** A walk of a project's directory costs the same every time and
//! answers the same thing, so a finder that walked afresh on each Ctrl+P would make the
//! reader wait for what it already knew. The worker holds what it found between opens,
//! and an open walks again behind it; that walk replaces the list when it ends rather
//! than streaming into one the reader is already typing against. Only the first walk,
//! with nothing to show, streams as it goes.
//!
//! **Not freya's `Popup`.** Its background centres its content down the window, and the
//! finder belongs at the top where an editor's is, so its Escape and its press outside
//! are given up along with its layout and written here: one overlay-layer rect over the
//! whole window that closes when pressed, and the panel centred across it under a top
//! gap. `DocumentMenuButton` gives up `ContextMenu` for the same kind of reason.

use super::*;
use crate::fuzzy;
use crate::walk::{found_under, Found, WalkEvent};
use std::sync::atomic::{self, AtomicU64};
use std::time::Instant;

/// The finder's state, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Finding(pub(crate) State<Finder>);

/// What the finder holds.
///
/// `id` numbers the walks so that a file can say which one it belongs to, `Searched`'s own
/// rule: the answer arrives long after the question, and a reader who has opened the
/// finder again is not waiting for the walk before it.
#[derive(Clone, Default)]
pub(crate) struct Finder {
    /// Whether the overlay is drawn at all.
    pub(crate) open: bool,
    /// What is in the box.
    pub(crate) typed: String,
    /// Which row the keyboard is on, and what was in the box when it was moved there.
    ///
    /// The two together and not an index alone: a row chosen under one query means
    /// nothing under the next, and comparing them where the row is read is what keeps a
    /// key pressed in the same pass as the typing from being undone by it. An effect
    /// that reset the row when the box changed ran a render late, and ate the first
    /// Down after a query.
    pub(crate) at: usize,
    pub(crate) at_for: String,
    /// The rows the worker last picked out, and the query it picked them for.
    pub(crate) listed: Listed,
    /// The directory the walk is of, so another project's files are never offered: a
    /// directory that does not match the one asked for empties what the worker holds.
    pub(crate) root: Option<PathBuf>,
    /// Which walk is on.
    pub(crate) id: u64,
    /// Whether it is still going.
    pub(crate) walking: bool,
}

impl Finder {
    /// Take the worker's answer. Whether it was this walk's, so the caller writes only
    /// then ([`write_if`]).
    ///
    /// The id check is here and not in the task: an answer arrives long after the
    /// question, and a reader who has opened the finder again is not waiting for the walk
    /// before it -- `Searched`'s own rule. An answer is taken **whole**, so the rows and
    /// the query they were picked out for are never two different questions'.
    fn take(&mut self, answered: Answered) -> bool {
        if self.id != answered.id {
            return false;
        }
        self.listed = Listed {
            rows: answered.rows,
            for_query: answered.query,
        };
        self.walking = answered.walking;
        true
    }

    /// Which row the keyboard is on: the row it was moved to, while the box still says
    /// what it said then, and the first row otherwise.
    fn selected(&self) -> usize {
        if self.at_for == self.typed {
            self.at
        } else {
            0
        }
    }
}

/// Open the finder over `root`, and walk it again behind what is already listed.
///
/// The one writer of [`Finder::open`] going true, and the only place a walk is started
/// from: what actually runs it is the effect in [`use_finder_with`], so the chord writes
/// state and nothing else.
pub(crate) fn open_finder(mut finder: State<Finder>, root: Option<PathBuf>) {
    // Bound before the write, so the read guard is gone by then.
    let id = finder.peek().id.wrapping_add(1);
    finder.set(Finder {
        open: true,
        typed: String::new(),
        at: 0,
        at_for: String::new(),
        // Nothing, and not the last open's rows: the box opens empty and lists the visits
        // instead, so there is nothing for them to be shown as until the reader types --
        // by which time the worker, which is what keeps the walk between opens, has
        // answered.
        listed: Listed::default(),
        walking: root.is_some(),
        root,
        id,
    });
}

/// Close it. What Escape, a press outside and opening a file all end with.
///
/// The walk is left running: nobody is waiting for it, but the list it is filling is what
/// the next open draws, and a walk abandoned halfway would have to be made again.
pub(crate) fn close_finder(mut finder: State<Finder>) {
    finder.write().open = false;
}

/// What the finder's worker is told: the walk it is to make, what that walk found, and
/// what the box says.
///
/// One channel and not two, so that the worker can block on it: a walk answers in
/// thousands while a reader types in ones, and a thread reading two channels at once
/// either polls or needs a runtime. Every message carries the walk it belongs to, and the
/// worker drops the ones that are not the walk it is on -- `Searched`'s own rule, an
/// answer arriving long after the question.
enum Told {
    /// A walk of `root` is starting, under `id`.
    Walking { id: u64, root: PathBuf },
    /// It found a file.
    Found { id: u64, file: Found },
    /// It ended.
    Walked { id: u64 },
    /// The box says this.
    Asked { id: u64, query: String },
}

/// What the worker answers with: the rows it picked out, the query it picked them for,
/// and whether the walk behind them is still going.
struct Answered {
    id: u64,
    query: String,
    rows: Arc<Vec<Row>>,
    walking: bool,
}

/// How often the worker answers while a walk is still streaming into it. A walk of a
/// large tree finds files far faster than a window draws them, and a rank of everything
/// found so far is worth no more per file than it is per tenth of a second.
const WALK_REFRESH: Duration = Duration::from_millis(100);

/// What the worker holds: the walk it is on, the files it found, and the query.
#[derive(Default)]
struct Held {
    id: u64,
    root: Option<PathBuf>,
    /// The last complete walk's files, which is what the ranking reads.
    files: Vec<Found>,
    /// The walk on now, held back until it ends. Only the first walk, with nothing to
    /// show, goes straight into `files`: rows must not move under a reader who is
    /// already typing against them.
    building: Vec<Found>,
    streams: bool,
    walking: bool,
    query: String,
}

/// Whether a message changed the answer, and whether it can wait for the next one.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Change {
    None,
    /// More of a walk. The reader is not waiting on any one of these.
    Files,
    /// The box, or the end of a walk.
    Now,
}

impl Held {
    /// Take one message. Hands back what it changed.
    fn take(&mut self, told: Told) -> Change {
        match told {
            Told::Walking { id, root } => {
                self.id = id;
                // Another project's files are never offered.
                if self.root.as_deref() != Some(&*root) {
                    self.files.clear();
                }
                self.root = Some(root);
                self.building.clear();
                self.streams = self.files.is_empty();
                self.walking = true;
                // Every open empties the box, so the query the last one ended on is not
                // one to rank the files against again.
                self.query.clear();
                Change::Files
            }
            Told::Found { id, file } if id == self.id => {
                if self.streams {
                    self.files.push(file);
                } else {
                    self.building.push(file);
                }
                Change::Files
            }
            Told::Walked { id } if id == self.id => {
                if !self.streams {
                    self.files = std::mem::take(&mut self.building);
                }
                self.walking = false;
                Change::Now
            }
            Told::Asked { id, query } if id == self.id => {
                self.query = query;
                Change::Now
            }
            _ => Change::None,
        }
    }

    /// The rows the box picked out, best first.
    ///
    /// The box is prepared once, as a [`fuzzy::Query`], and asked of every walked path:
    /// a pass over the path and one small vector for where its characters fell, per file.
    /// An empty box picks out nothing here: what it lists is the files visited most
    /// recently, which is the UI's own to work out and cheap enough to be.
    fn answer(&self) -> Answered {
        let typed = self.query.trim();
        let mut hits: Vec<(fuzzy::Score, Row)> = Vec::new();
        if let Some(query) = fuzzy::Query::new(typed) {
            hits = self
                .files
                .iter()
                .filter_map(|file| {
                    let hit = query.find(&file.shown, file.name_at)?;
                    Some((
                        hit.score,
                        Row {
                            file: file.clone(),
                            marks: hit.marks,
                        },
                    ))
                })
                .collect();
            // Stable, so files that scored the same keep the order the walk found them in.
            hits.sort_by_key(|hit| hit.0);
        }
        Answered {
            id: self.id,
            query: self.query.clone(),
            rows: Arc::new(hits.into_iter().map(|(_, row)| row).collect()),
            walking: self.walking,
        }
    }
}

/// The worker: told of a walk and of the box, answering with the rows to draw.
fn rank_files(told: async_channel::Receiver<Told>, answers: async_channel::Sender<Answered>) {
    let mut held = Held::default();
    let mut answered: Option<Instant> = None;

    while let Ok(first) = told.recv_blocking() {
        // Drained to the end of what is waiting, so a burst of a walk is one ranking and
        // not one per file: `take_hits`' own rule, a batch per wake.
        let mut change = held.take(first);
        while let Ok(next) = told.try_recv() {
            change = change.max(held.take(next));
        }
        if change == Change::None {
            continue;
        }
        if change == Change::Files && answered.is_some_and(|at| at.elapsed() < WALK_REFRESH) {
            continue;
        }
        answered = Some(Instant::now());
        if answers.send_blocking(held.answer()).is_err() {
            // The app is closing.
            return;
        }
    }
}

/// Start the finder's worker, walk the directory it is asked about on a thread of the
/// app's own, and take the rows it picks out back into [`Finder`].
///
/// The work is an argument so that a test can put its own files in the walk's place: a
/// walk that answers as fast as it is asked can say nothing about batching, superseding
/// or the list that is kept, which is the whole of what there is here to get wrong.
pub(crate) fn use_finder_with(
    finder: State<Finder>,
    work: impl Fn(&Path, &mut dyn FnMut(WalkEvent) -> ControlFlow<()>) + Send + Clone + 'static,
) {
    // One worker for the app's lifetime, as `use_analysis`' is: what it holds is the
    // project's files, and a thread per open would walk them again for every Ctrl+P.
    // Unbounded, because the UI sends into it too and a UI thread parked in a send is the
    // freeze this exists to prevent; what stops a walk nobody is waiting for is `current`
    // rather than a full channel.
    let (tells, current) = use_hook(|| {
        let (tells, told) = async_channel::unbounded::<Told>();
        let (sends, answers) = async_channel::unbounded::<Answered>();
        // A `std::thread` and not a task: this walks a directory and ranks a project's
        // worth of paths, and freya's executor is the UI thread.
        thread("the file finder's worker", move || rank_files(told, sends));
        spawn(take_rows(finder, answers));
        (tells, Arc::new(AtomicU64::new(0)))
    });

    // A memo and not a read: the state is written for every answer, and an effect reading
    // it would start a walk for each answer to its own question.
    let asked = use_memo(move || {
        let state = finder.read();
        (state.id, state.root.clone())
    });
    let typed = use_memo(move || {
        let state = finder.read();
        (state.id, state.typed.clone())
    });

    use_side_effect({
        let tells = tells.clone();
        let current = current.clone();
        move || {
            // Reading the memo subscribes this to the question; the state it writes is
            // peeked.
            let (id, root) = asked.read().clone();
            let Some(root) = root else {
                return;
            };
            if id == 0 {
                return;
            }
            // Bumped before the walk is told of, so a walk already running reads it and
            // stops where it stands.
            current.store(id, atomic::Ordering::Relaxed);
            if tells
                .send_blocking(Told::Walking {
                    id,
                    root: root.clone(),
                })
                .is_err()
            {
                return;
            }

            // Not a [`stream`]: what a walk finds goes to the worker above and never to
            // the UI thread, over the one channel that worker blocks on, so there is no
            // receiver here for the walk to be stopped by dropping. `current` is what
            // stops it instead.
            let work = work.clone();
            let tells = tells.clone();
            let current = current.clone();
            thread("the file finder's walk", move || {
                work(&root, &mut |event| {
                    let told = match event {
                        WalkEvent::File(file) => Told::Found { id, file },
                        WalkEvent::Finished => Told::Walked { id },
                    };
                    if tells.send_blocking(told).is_err() {
                        return ControlFlow::Break(());
                    }
                    // This walk has been replaced, and nobody is waiting for the rest of
                    // it.
                    if current.load(atomic::Ordering::Relaxed) == id {
                        ControlFlow::Continue(())
                    } else {
                        ControlFlow::Break(())
                    }
                });
            });
        }
    });

    use_side_effect(move || {
        let (id, query) = typed.read().clone();
        if id == 0 {
            return;
        }
        let _ = tells.send_blocking(Told::Asked { id, query });
    });
}

/// Take the worker's answers into [`Finder`], which drops the ones belonging to a walk
/// the reader has moved on from ([`Finder::take`]).
async fn take_rows(finder: State<Finder>, answers: async_channel::Receiver<Answered>) {
    while let Ok(answered) = answers.recv().await {
        write_if(finder, |state| state.take(answered));
    }
}

/// What the list is drawn from: whether the finder is drawn at all, what is in the box,
/// where the walk is of, and the worker's last answer.
///
/// A memo of its own between [`Finder`] and the list, because a subscription is to a
/// whole state and not to a field of one. The row the keyboard is on lives in `Finder`
/// too, so a list worked out straight off the state was worked out again by every arrow
/// press. This memo does run per press; it hands back what it handed back last time, and
/// `set_if_modified` stops there.
pub(crate) struct Asking {
    open: bool,
    typed: String,
    root: Option<PathBuf>,
    listed: Listed,
}

impl PartialEq for Asking {
    fn eq(&self, other: &Self) -> bool {
        self.open == other.open
            && self.typed == other.typed
            && self.root == other.root
            && self.listed == other.listed
    }
}

/// What the finder is asking for, as the state stands.
pub(crate) fn asking(state: &Finder) -> Asking {
    Asking {
        open: state.open,
        typed: state.typed.clone(),
        root: state.root.clone(),
        listed: state.listed.clone(),
    }
}

/// The rows the finder draws, and the query they were picked out for.
///
/// The query is kept beside them because the worker answers a question the box has often
/// moved on from -- by a frame, which is what a rank of a large project costs. The panel
/// goes on drawing the rows it has meanwhile, `Analyzed`'s own rule; what the query is
/// for is the panel not saying *No files match* about a query nobody has answered yet.
#[derive(Clone, Default)]
pub(crate) struct Listed {
    rows: Arc<Vec<Row>>,
    for_query: String,
}

/// One file the box picked out: the file, and where the query hit its path.
#[derive(Clone)]
struct Row {
    file: Found,
    marks: Vec<Range<usize>>,
}

impl PartialEq for Listed {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.rows, &other.rows) && self.for_query == other.for_query
    }
}

impl Listed {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether these rows are the answer to what the box says.
    pub(crate) fn answers(&self, typed: &str) -> bool {
        self.for_query.trim() == typed.trim()
    }

    /// The `index`th row: the file, and the runs of its path that matched.
    fn row(&self, index: usize) -> Option<(&Found, &[Range<usize>])> {
        let row = self.rows.get(index)?;
        Some((&row.file, &row.marks))
    }

    /// What opening the `index`th row opens.
    fn path(&self, index: usize) -> Option<PathBuf> {
        self.row(index).map(|(file, _)| file.path.clone())
    }
}

/// The source files visited most recently, newest first: what an empty box lists.
///
/// The UI's own and not the worker's, because it is not the walk's answer: a file opened
/// before the walk finished is listed, and there are as many of these as the reader has
/// been places. Only the ones under the project's directory: a reader following debug
/// info into a binary lands in sources that are nobody's project -- the standard
/// library's, and a dependency's out of the registry -- and the finder is the project's
/// files.
fn recent(asking: &Asking, visits: &Visits) -> Listed {
    let Some(root) = asking.root.clone() else {
        return Listed::default();
    };
    let rows: Vec<Row> = visits
        .entries()
        .iter()
        .filter_map(|document| match document {
            Document::Source(path) => found_under(&root, Path::new(&**path)),
            _ => None,
        })
        .map(|file| Row {
            file,
            marks: Vec::new(),
        })
        .collect();
    Listed {
        rows: Arc::new(rows),
        for_query: String::new(),
    }
}

/// The overlay: the box, and the files under it. Mounted at the root and drawn as nothing
/// at all until Ctrl+P.
#[derive(PartialEq)]
pub(crate) struct FinderOverlay;

impl Component for FinderOverlay {
    fn render(&self) -> impl IntoElement {
        let finder = use_consume::<Finding>().0;
        let states = use_project_states();
        let visits = states.visits;
        let keyboard = use_consume::<Keyboard>().0;
        let box_id = use_hook(AccessibilityId::new_unique);
        // The panel's one focusable node is its box, so it is the box that answers for
        // the list under it: the rows are drawn live while the reader is typing at them,
        // and in the grey if the keyboard has gone elsewhere (`ui/picks.rs`).
        use_provide_context(|| RowsBox(box_id));
        // The list's own scroll. The arrows move a row the view knows nothing about, so
        // without a controller to follow it the row goes under the panel's edge at the
        // thirteenth press, and Enter opens a file the reader never saw named.
        let list = use_scroll_controller(ScrollConfig::default);

        // Every hook first and the early return below them: the overlay is drawn for a
        // fraction of the run, and a hook it skipped would be a hook the next render has
        // in a different place.
        let asking = use_memo(move || asking(&finder.read()));
        let listed = use_memo(move || {
            let asking = asking.read();
            if !asking.open {
                return Listed::default();
            }
            // The visits are read on this branch alone, because reading a state is what
            // subscribes this memo to it: a file being opened writes the visits, and a
            // memo subscribed to them while the box had text would be woken by every one.
            if asking.typed.trim().is_empty() {
                return recent(&asking, &visits.read());
            }
            asking.listed.clone()
        });

        let state = finder.read().clone();
        // The caret in the box, asked for whenever the box does not have it and the
        // overlay is up. When it opens, the box has no node to focus until then --
        // `reach_search`'s own reason for asking through the state. After that it is a
        // press on a row: a row cannot hold the keyboard, and freya takes the focus out
        // of the panel **after** the row's handler has run, so an Alt+press would leave a
        // finder nobody could type in and the row it picked drawn as a list nobody is in.
        // Both states are read and not peeked, which is what subscribes the effect, and
        // the focus is what it has to be woken by. Asking in the handler instead is too
        // early: the platform's focus is written at the end of the pass that handler ran
        // in, so the ask is made while the box still counts as focused and
        // `request_focus` declines it.
        use_side_effect(move || {
            let focused = box_id.is_focused();
            if finder.read().open && !focused {
                box_id.request_focus();
            }
        });
        if !state.open {
            return rect().into_element();
        }

        // The list as the panel is about to draw it. The memo itself goes to the key
        // handler below, which wants the same list and not a ranking of its own.
        let drawn = listed.read().clone();
        let rows = drawn.len();
        let at = state.selected().min(rows.saturating_sub(1));

        let body: Element = match (&state.root, rows) {
            (None, _) => note("No project directory. Set one in the Project view."),
            (Some(_), 0) if state.walking => note("Reading the project's directory\u{2026}"),
            (Some(_), 0) if state.typed.trim().is_empty() => {
                note("No files opened yet. Type to find one.")
            }
            // The worker is a frame behind the box. Nothing is said about a query it has
            // not answered: *No files match* under a query that does match is worse than
            // a panel with only its box in it for the frame it takes.
            (Some(_), 0) if !drawn.answers(&state.typed) => rect().into_element(),
            (Some(_), 0) => note("No files match."),
            (Some(_), _) => rect()
                .width(Size::fill())
                .height(Size::px(rows.min(FINDER_ROWS) as f32 * list_row_height()))
                .child(
                    VirtualScrollView::new_with_data_controlled(
                        (drawn, at, finder),
                        |index, (drawn, at, finder): &(Listed, usize, State<Finder>)| {
                            FoundRow {
                                listed: drawn.clone(),
                                index,
                                on_row: index == *at,
                                finder: *finder,
                                key: DiffKey::None,
                            }
                            .key(&index)
                            .into()
                        },
                        list,
                    )
                    .length(rows)
                    .item_size(list_row_height()),
                )
                .into_element(),
        };

        rect()
            // `Popup`'s own shape, which is load-bearing in two ways. The layer and the
            // global position go **here**, on the one rect over everything, and not on
            // the two under it: on the children instead, nothing in the overlay takes a
            // press at all -- not the rows either. And the press outside is a rect of its
            // own with nothing in it, the panel sitting in a second over that; nested
            // instead, the press outside never arrives.
            .layer(Layer::Overlay)
            .position(Position::new_global())
            .child(
                rect()
                    // Over the whole window with nothing drawn in it: what it is for is
                    // the press, which closes the finder. The app under it is not dimmed
                    // -- the reader is choosing a file by what they can see of it, and
                    // the panel's shadow is what lifts the finder off it.
                    .position(Position::new_global().top(0.0).left(0.0))
                    .width(Size::window_percent(100.0))
                    .height(Size::window_percent(100.0))
                    .on_press(move |_| close_finder(finder)),
            )
            .child(
                rect()
                    .position(Position::new_global().top(0.0).left(0.0))
                    .width(Size::window_percent(100.0))
                    .height(Size::window_percent(100.0))
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new(FINDER_TOP, 0.0, 0.0, 0.0))
                    .child(
                        rect()
                            .width(Size::px(FINDER_WIDTH))
                            .background(palette().pane_bg)
                            .border(
                                Border::new()
                                    .width(1.0)
                                    .fill(palette().hairline)
                                    .alignment(BorderAlignment::Outer),
                            )
                            .corner_radius(FINDER_RADIUS)
                            .shadow(
                                Shadow::new()
                                    .y(FINDER_PAD)
                                    .blur(FINDER_BLUR)
                                    .color(palette().panel_shadow),
                            )
                            .overflow(Overflow::Clip)
                            // The press that opened a row must not reach the rect
                            // behind, which would take the press for one outside.
                            .on_press(move |e: Event<PressEventData>| e.stop_propagation())
                            // Global, and on the panel rather than on the box: the keys
                            // below move a list the box does not hold, and the box
                            // declines them so that they arrive here at all.
                            .on_global_key_down(move |e: Event<KeyboardEventData>| {
                                finder_key(finder, states, keyboard, list, listed, &e.key);
                            })
                            .child(FinderBox {
                                finder,
                                a11y: box_id,
                            })
                            .child(body),
                    ),
            )
            .into_element()
    }
}

/// A line the panel says instead of a list. Not `placeholder`, which is `expanded`: the
/// panel is as tall as what is in it, and a body that filled its parent would make the
/// panel the height of the window -- covering the rect that takes the press outside, and
/// with it every press the finder answers.
fn note(text: &str) -> Element {
    rect()
        .width(Size::fill())
        .padding(FINDER_PAD)
        .child(label().text(text.to_owned()))
        .into()
}

/// The keys the finder answers: the list moved through, a file opened, and the overlay
/// closed. Every read is bound before any write.
///
/// The list is the memo's, not a ranking of its own: it is the one the panel drew, so
/// Enter opens the row the reader is looking at, and neither arrow asks the query of
/// every walked path again.
fn finder_key(
    finder: State<Finder>,
    states: ProjectStates,
    keyboard: State<Keys>,
    list: ScrollController,
    listed: Memo<Listed>,
    key: &Key,
) {
    let rows = listed.peek().len();
    match key {
        Key::Named(NamedKey::Escape) => close_finder(finder),
        Key::Named(NamedKey::ArrowDown) => followed(list, moved(finder, rows, 1)),
        Key::Named(NamedKey::ArrowUp) => followed(list, moved(finder, rows, -1)),
        Key::Named(NamedKey::Enter) => {
            let opened = {
                let at = finder.peek().selected().min(rows.saturating_sub(1));
                listed.peek().path(at)
            };
            if let Some(path) = opened {
                open_found(states, keyboard, &path);
                close_finder(finder);
            }
        }
        _ => {}
    }
}

/// Move the keyboard `by` rows of `rows`, and remember what the box said when it was
/// moved: the row is the list's as the query stands, and the list changes under it.
///
/// Both ends stop at the list, which is why the count is wanted at all: unclamped, Down
/// held past the last row counted on above it, and every Up after that was spent coming
/// back before the highlight moved at all. The count is the drawn list's and not a
/// ranking made here for it.
///
/// Hands back the row it moved to and how many there are, which is what the scroll
/// follows.
fn moved(mut finder: State<Finder>, rows: usize, by: isize) -> (usize, usize) {
    // Bound before the write, so the read guard is gone by then.
    let (at, typed) = {
        let state = finder.peek();
        (
            state.selected().min(rows.saturating_sub(1)),
            state.typed.clone(),
        )
    };
    let mut state = finder.write();
    state.at = at.saturating_add_signed(by).min(rows.saturating_sub(1));
    state.at_for = typed;
    (state.at, rows)
}

/// Put the keyboard on `index` and leave the finder open: what an Alt+press on a row
/// does. The box's text is remembered with it, as [`moved`] remembers it, so the row is
/// this list's as the query stands and not a row of the next one typed.
fn pick_row(mut finder: State<Finder>, index: usize) {
    // Bound before the write, so the read guard is gone by then.
    let typed = finder.peek().typed.clone();
    let mut state = finder.write();
    state.at = index;
    state.at_for = typed;
}

/// Scroll the list so the row the keyboard was moved to is one of the rows drawn: the
/// panel is [`FINDER_ROWS`] tall and the arrows walk past that, and a row nobody can see
/// is a file Enter opens unnamed.
fn followed(mut list: ScrollController, (at, rows): (usize, usize)) {
    if rows == 0 {
        return;
    }
    let height = list_row_height();
    reveal_caret(
        &mut list,
        rows.min(FINDER_ROWS) as f32 * height,
        height,
        rows,
        at,
    );
}

/// Open a file the finder listed: a source-driven tab of its own that stays, since a
/// reader who typed the path out and picked it off the list has chosen the file. A tab
/// already showing it is raised. The Files row's own door, so it carries that guard too:
/// a file the source pane would refuse opens nothing at all.
/// The keyboard goes with it, as it does out of every list a row is opened from
/// (`ui/picks.rs`), and here it has nowhere else to be: the panel is closing and the box it
/// was in goes with it.
fn open_found(states: ProjectStates, keyboard: State<Keys>, path: &Path) {
    open_source_file(states, path, Reach::NewTab);
    ask_for_keyboard(keyboard);
}

/// The box at the top of the overlay.
#[derive(Clone, PartialEq)]
struct FinderBox {
    finder: State<Finder>,
    a11y: AccessibilityId,
}

impl Component for FinderBox {
    fn render(&self) -> impl IntoElement {
        let finder = self.finder;
        let a11y = self.a11y;

        rect()
            .width(Size::fill())
            .padding(Gaps::new_symmetric(FINDER_PAD, FINDER_PAD))
            .child(
                Input::new(
                    finder
                        .into_writable()
                        .map(|finder| &finder.typed, |finder| &mut finder.typed),
                )
                .placeholder("Find a file")
                // The whole of the panel, less the air around it. Not `flex`: this rect
                // is a column, so a flex child would be given the main axis, which here
                // is the height -- the box kept the `Input`'s own default width, a third
                // of the panel it sits in. And not `compact`, which is for a bar the
                // width of a sidebar; there is room here for the text to sit in.
                .width(Size::fill())
                .a11y_id(a11y)
                // The four keys the panel's own handler answers, declined here so they
                // reach it (`chords.rs`).
                .on_pre_key_down(box_keys(
                    Boxed::Input,
                    &[
                        NamedKey::ArrowUp,
                        NamedKey::ArrowDown,
                        NamedKey::Enter,
                        NamedKey::Escape,
                    ],
                    |_, _| {},
                )),
            )
    }
}

/// One file in the list: its name, the directories above it dimmed, and what the query
/// matched marked in both.
#[derive(Clone)]
struct FoundRow {
    listed: Listed,
    index: usize,
    /// Whether the keyboard is on this row.
    on_row: bool,
    finder: State<Finder>,
    key: DiffKey,
}

impl PartialEq for FoundRow {
    fn eq(&self, other: &Self) -> bool {
        self.listed == other.listed && self.index == other.index && self.on_row == other.on_row
    }
}

impl KeyExt for FoundRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for FoundRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        // Consumed in the render, because the handler that uses them runs no hook.
        let states = use_project_states();
        let finder = self.finder;

        let alt = use_consume::<Alt>().0;
        let keyboard = use_consume::<Keyboard>().0;
        let index = self.index;

        let Some((file, marks)) = self.listed.row(self.index) else {
            return rect().into_element();
        };
        let pressed = file.path.clone();

        // Cut and not extra, though the strings differ: the row draws every part of the
        // path the tooltip holds, the name first and the directories after it.
        cut_tooltip(
            fitted.cut(),
            file.path.display().to_string(),
            // The keyboard's row is what is picked out here; the pointer's is the hover.
            list_row(hovering, chosen(self.on_row, keyboard_in_list()))
                .on_press(move |_| {
                    // Alt says this press is not a door, as it does on a link and in
                    // every list: the row is picked out and the finder stays open. The
                    // finder's pick *is* its keyboard row, so pointing at a row is
                    // moving the keyboard to it.
                    if *alt.peek() {
                        pick_row(finder, index);
                        return;
                    }
                    open_found(states, keyboard, &pressed);
                    close_finder(finder);
                })
                .child({
                    let (spans, drawn, hits) = row_line(file, marks);
                    fitted.measuring(marked(
                        paragraph()
                            .width(Size::fill())
                            .max_lines(1)
                            .text_overflow(TextOverflow::Ellipsis)
                            .spans_iter(spans.into_iter()),
                        &drawn,
                        &hits,
                    ))
                }),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// A row's one paragraph: the file's name, then the directories above it dimmed -- and the
/// line as it is drawn, with what the query matched in it, which the row washes.
///
/// The name first and the path after it, which is not the order the path is written in:
/// the name is what a reader is looking for down a list, and a column of names all
/// starting with `src/ui/` says nothing.
fn row_line(
    file: &Found,
    marks: &[Range<usize>],
) -> (Vec<Span<'static>>, String, Vec<Range<usize>>) {
    let name_at = file.name_at;
    let name = file.name();
    // Where the query hit the name, as offsets into the name -- which is where the drawn
    // line starts, so they are the drawn line's own.
    let mut drawn_marks: Vec<Range<usize>> = marks
        .iter()
        .filter(|mark| mark.end > name_at)
        .map(|mark| mark.start.max(name_at) - name_at..mark.end - name_at)
        .collect();

    // The trailing separator goes with the directories, and a file in the project's own
    // directory has neither.
    let directory = file.directory().trim_end_matches('/');
    if directory.is_empty() {
        return (
            vec![Span::new(name.to_owned())],
            name.to_owned(),
            drawn_marks,
        );
    }

    // And where it hit the directories, moved along by the name and the gap before them:
    // the line is drawn in neither the path's order nor its shape.
    let drawn = format!("{name}{GAP}{directory}");
    let along = name.len() + GAP.len();
    drawn_marks.extend(
        marks
            .iter()
            .filter(|mark| mark.start < directory.len())
            .map(|mark| along + mark.start..along + mark.end.min(directory.len())),
    );
    (
        dimmed_after(&drawn, name.len(), palette().address_fg),
        drawn,
        drawn_marks,
    )
}

/// What sits between the name and the directories above it.
const GAP: &str = "  ";

#[cfg(test)]
mod tests;

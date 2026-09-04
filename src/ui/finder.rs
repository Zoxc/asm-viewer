//! The file finder: the box Ctrl+P opens over the app, the files of the project's
//! directory under it, and the one worker that walks them.
//!
//! `SearchTab`'s shape over `src/walk.rs`'s walk -- a question, one answer that stands
//! until the next replaces it, and a thread of the app's own that answers it -- with two
//! differences the reader can see.
//!
//! **The list is kept.** A walk of a project's directory costs the same every time and
//! answers the same thing, so a finder that walked afresh on each Ctrl+P would make the
//! reader wait for what it already knew. The list stands between opens, and an open shows
//! it at once and walks again behind it; that walk replaces the list when it ends rather
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
    /// The files the last walk found, kept between opens.
    pub(crate) files: Arc<Vec<Found>>,
    /// The directory they were walked from, so another project's files are never offered:
    /// a directory that does not match the one asked for empties the list.
    pub(crate) root: Option<PathBuf>,
    /// Which walk is on.
    pub(crate) id: u64,
    /// Whether it is still going.
    pub(crate) walking: bool,
}

impl Finder {
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
    let (id, files) = {
        let state = finder.peek();
        let kept = state.root == root;
        (
            state.id.wrapping_add(1),
            if kept {
                state.files.clone()
            } else {
                Arc::default()
            },
        )
    };
    finder.set(Finder {
        open: true,
        typed: String::new(),
        at: 0,
        at_for: String::new(),
        files,
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

/// Walk the directory the finder is asked about, on a thread of the app's own, and take
/// the files back into [`Finder`] as they arrive.
///
/// The work is an argument so that a test can put its own files in the walk's place: a
/// walk that answers as fast as it is asked can say nothing about batching, superseding
/// or the list that is kept, which is the whole of what there is here to get wrong.
pub(crate) fn use_finder_with(
    finder: State<Finder>,
    work: impl Fn(&Path, &mut dyn FnMut(WalkEvent) -> ControlFlow<()>) + Send + Clone + 'static,
) {
    // A memo and not a read: every batch of files is a write to this state, and an effect
    // reading it would start a walk for each batch of its own answer.
    let asked = use_memo(move || {
        let state = finder.read();
        (state.id, state.root.clone())
    });

    use_side_effect(move || {
        // Reading the memo subscribes this to the question; the state it writes is peeked.
        let (id, root) = asked.read().clone();
        let Some(root) = root else {
            return;
        };
        if id == 0 {
            return;
        }

        // Bounded, and small: a walk finds files far faster than a window draws them, and
        // a worker parked in a send is one that learns the moment the reader has moved on.
        let (files, events) = async_channel::bounded::<WalkEvent>(512);
        let work = work.clone();
        // A `std::thread` and not a task: this walks a directory, and freya's executor is
        // the UI thread. Named, so a panic on it says which worker died (`crate::panics`).
        let started = std::thread::Builder::new()
            .name("the file finder's worker".to_owned())
            .spawn(move || {
                work(&root, &mut |event| match files.send_blocking(event) {
                    Ok(()) => ControlFlow::Continue(()),
                    // The receiver is gone: this walk has been replaced or the app is
                    // closing, and either way nobody is waiting for the rest of it.
                    Err(_) => ControlFlow::Break(()),
                });
            });
        if let Err(error) = started {
            log::warn!("the file finder's worker could not be started: {error}");
        }

        spawn(take_files(finder, id, events));
    });
}

/// Take the files of walk `id` as they arrive, until they stop or the walk is replaced.
///
/// A batch per wake and not a write per file, `take_hits`' own rule: each write is a
/// render, and a walk over a large tree answers in thousands. The batch is dropped whole
/// when the walk is no longer the one asked for, checked before the write.
async fn take_files(
    mut finder: State<Finder>,
    id: u64,
    events: async_channel::Receiver<WalkEvent>,
) {
    // What this walk has found. Written through as it grows only while there is nothing
    // listed yet; with a list already on screen it is held and put in place at the end,
    // so rows never move under a reader who is typing.
    let mut building: Vec<Found> = Vec::new();
    let streams = finder.peek().files.is_empty();

    while let Ok(first) = events.recv().await {
        let batch: Vec<WalkEvent> = std::iter::once(first)
            .chain(std::iter::from_fn(|| events.try_recv().ok()))
            .collect();

        // Bound in a statement of its own: the read guard is gone before the write.
        let mine = finder.peek().id == id;
        if !mine {
            // Returning drops the receiver, which is what stops the walk behind it.
            return;
        }

        let mut ended = false;
        for event in batch {
            match event {
                WalkEvent::File(file) => building.push(file),
                WalkEvent::Finished => ended = true,
            }
        }

        let mut state = finder.write();
        if streams {
            Arc::make_mut(&mut state.files).append(&mut building);
        } else if ended {
            state.files = Arc::new(std::mem::take(&mut building));
        }
        if ended {
            state.walking = false;
        }
    }
}

/// The rows the finder draws: the files they are, and which of them the box picked out.
///
/// The files are held whole and the hits index into them, so a query that lets everything
/// through allocates a mark list per row and not a path per row.
#[derive(Clone, Default)]
pub(crate) struct Listed {
    files: Arc<Vec<Found>>,
    hits: Arc<Vec<Picked>>,
}

/// One file the box picked out: which, and where the query hit it.
#[derive(Clone)]
struct Picked {
    at: usize,
    marks: Vec<Range<usize>>,
}

impl PartialEq for Listed {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.files, &other.files) && Arc::ptr_eq(&self.hits, &other.hits)
    }
}

impl Listed {
    pub(crate) fn len(&self) -> usize {
        self.hits.len()
    }

    /// The `index`th row: the file, and the runs of its path that matched.
    fn row(&self, index: usize) -> Option<(&Found, &[Range<usize>])> {
        let hit = self.hits.get(index)?;
        Some((self.files.get(hit.at)?, &hit.marks))
    }

    /// What opening the `index`th row opens.
    fn path(&self, index: usize) -> Option<PathBuf> {
        self.row(index).map(|(file, _)| file.path.clone())
    }
}

/// What the box is asking for, answered: the files it picked out, best first, or the
/// files opened most recently where nothing is typed.
///
/// On the UI thread, as a sidebar list's own filter is. A query is asked of every walked
/// path per keystroke, which is one pass over a string per file and no allocation for the
/// paths that do not match.
fn listed(state: &Finder, visits: &Visits) -> Listed {
    let typed = state.typed.trim();
    if typed.is_empty() {
        return recent(state, visits);
    }

    let mut hits: Vec<(fuzzy::Score, Picked)> = state
        .files
        .iter()
        .enumerate()
        .filter_map(|(at, file)| {
            let hit = fuzzy::find(typed, &file.shown, file.name_at)?;
            Some((
                hit.score,
                Picked {
                    at,
                    marks: hit.marks,
                },
            ))
        })
        .collect();
    // Stable, so files that scored the same keep the order the walk found them in.
    hits.sort_by_key(|hit| hit.0);

    Listed {
        files: state.files.clone(),
        hits: Arc::new(hits.into_iter().map(|(_, hit)| hit).collect()),
    }
}

/// The source files visited most recently, newest first: what an empty box lists.
///
/// Built from the visits and not from the walk, so a file opened before the walk finished
/// is listed. Only the ones under the project's directory: a reader following debug info
/// into a binary lands in sources that are nobody's project -- the standard library's,
/// and a dependency's out of the registry -- and the finder is the project's files.
fn recent(state: &Finder, visits: &Visits) -> Listed {
    let Some(root) = state.root.clone() else {
        return Listed::default();
    };
    let files: Vec<Found> = visits
        .recent()
        .filter_map(|document| match document {
            Document::Source(path) => found_under(&root, Path::new(&**path)),
            _ => None,
        })
        .collect();
    let hits = (0..files.len())
        .map(|at| Picked {
            at,
            marks: Vec::new(),
        })
        .collect();
    Listed {
        files: Arc::new(files),
        hits: Arc::new(hits),
    }
}

/// The overlay: the box, and the files under it. Mounted at the root and drawn as nothing
/// at all until Ctrl+P.
#[derive(PartialEq)]
pub(crate) struct FinderOverlay;

impl Component for FinderOverlay {
    fn render(&self) -> impl IntoElement {
        let finder = use_consume::<Finding>().0;
        let visits = use_consume::<Visited>().0;
        let states = use_project_states();
        let box_id = use_hook(AccessibilityId::new_unique);

        // Every hook first and the early return below them: the overlay is drawn for a
        // fraction of the run, and a hook it skipped would be a hook the next render has
        // in a different place.
        let listed = use_memo(move || {
            let state = finder.read();
            if !state.open {
                return Listed::default();
            }
            listed(&state, &visits.read())
        });

        let state = finder.read().clone();
        // The caret in the box, asked for once each time the overlay is drawn: the box
        // has no node to focus until then, `reach_search`'s own reason for asking through
        // the state.
        use_side_effect_with_deps(&state.open, move |open: &bool| {
            if *open {
                box_id.request_focus();
            }
        });
        if !state.open {
            return rect().into_element();
        }

        let listed = listed.read().clone();
        let rows = listed.len();
        let at = state.selected().min(rows.saturating_sub(1));

        let body: Element = match (&state.root, rows) {
            (None, _) => note("No project directory. Set one in the Project view."),
            (Some(_), 0) if state.walking => note("Reading the project's directory\u{2026}"),
            (Some(_), 0) if state.typed.trim().is_empty() => {
                note("No files opened yet. Type to find one.")
            }
            (Some(_), 0) => note("No files match."),
            (Some(_), _) => rect()
                .width(Size::fill())
                .height(Size::px(rows.min(FINDER_ROWS) as f32 * list_row_height()))
                .child(
                    VirtualScrollView::new_with_data(
                        (listed, at, finder),
                        |index, (listed, at, finder): &(Listed, usize, State<Finder>)| {
                            FoundRow {
                                listed: listed.clone(),
                                index,
                                on_row: index == *at,
                                finder: *finder,
                                key: DiffKey::None,
                            }
                            .key(&index)
                            .into()
                        },
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
                                let ctrl = e.modifiers.contains(Modifiers::ctrl_or_meta());
                                finder_key(finder, states, &e.key, ctrl);
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
fn finder_key(finder: State<Finder>, states: ProjectStates, key: &Key, ctrl: bool) {
    match key {
        Key::Named(NamedKey::Escape) => close_finder(finder),
        Key::Named(NamedKey::ArrowDown) => moved(finder, 1),
        Key::Named(NamedKey::ArrowUp) => moved(finder, -1),
        Key::Named(NamedKey::Enter) => {
            let opened = {
                let state = finder.peek();
                let listed = listed(&state, &states.visits.peek());
                let at = state.selected().min(listed.len().saturating_sub(1));
                listed.path(at)
            };
            if let Some(path) = opened {
                open_found(states, &path, ctrl);
                close_finder(finder);
            }
        }
        _ => {}
    }
}

/// Move the keyboard `by` rows, and remember what the box said when it was moved: the row
/// is the list's as the query stands, and the list changes under it.
fn moved(mut finder: State<Finder>, by: isize) {
    // Bound before the write, so the read guard is gone by then.
    let (at, typed) = {
        let state = finder.peek();
        (state.selected(), state.typed.clone())
    };
    let mut state = finder.write();
    state.at = at.saturating_add_signed(by);
    state.at_for = typed;
}

/// Open a file the finder listed: a source-driven tab, in the temporal one or a new one
/// as Ctrl says. What pressing a Files item does, and the same guard: a file the source
/// pane would refuse opens nothing at all.
fn open_found(states: ProjectStates, path: &Path, ctrl: bool) {
    if !shows_as_source(path) {
        return;
    }
    let file = Document::Source(Arc::from(&*path.to_string_lossy()));
    let reach = if ctrl { Reach::NewTab } else { Reach::Preview };
    open_document(states.open, states.visits, file, reach);
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
                // Declined, not answered: the keys that move the list and the chord that
                // opened the box belong to handlers beside this one, and the `_` arm
                // below calls `prevent_default`, which cancels the global key event they
                // arrive by. The rest is freya's default, which the hook replaces
                // wholesale (`notes/upstream/freya.md`).
                .on_pre_key_down(Callback::new(
                    move |e: Event<KeyboardEventData>| {
                        if is_finder_chord(&e.key, e.modifiers) {
                            return false;
                        }
                        match &e.key {
                            Key::Named(NamedKey::ArrowUp)
                            | Key::Named(NamedKey::ArrowDown)
                            | Key::Named(NamedKey::Enter)
                            | Key::Named(NamedKey::Escape) => false,
                            Key::Named(NamedKey::Shift) => true,
                            Key::Named(NamedKey::Tab) => false,
                            _ => {
                                e.stop_propagation();
                                e.prevent_default();
                                true
                            }
                        }
                    },
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
        let mut hovering = use_state(|| false);
        // Consumed in the render, because the handler that uses them runs no hook.
        let states = use_project_states();
        let ctrl = use_consume::<Ctrl>().0;
        let finder = self.finder;

        let Some((file, marks)) = self.listed.row(self.index) else {
            return rect().into_element();
        };
        let pressed = file.path.clone();

        let background = if self.on_row {
            palette().selected_bg
        } else if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        row_tooltip(
            file.path.display().to_string(),
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    open_found(states, &pressed, *ctrl.peek());
                    close_finder(finder);
                })
                .child(
                    paragraph()
                        .width(Size::fill())
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis)
                        .spans_iter(row_spans(file, marks).into_iter()),
                ),
        )
        .into_element()
    }
}

/// A row's one paragraph: the file's name, then the directories above it dimmed, with
/// what the query matched marked in whichever it fell in.
///
/// The name first and the path after it, which is not the order the path is written in:
/// the name is what a reader is looking for down a list, and a column of names all
/// starting with `src/ui/` says nothing.
fn row_spans(file: &Found, marks: &[Range<usize>]) -> Vec<Span<'static>> {
    let name_at = file.name_at;
    let in_name: Vec<Range<usize>> = marks
        .iter()
        .filter(|mark| mark.end > name_at)
        .map(|mark| mark.start.max(name_at) - name_at..mark.end - name_at)
        .collect();
    let mut spans = marked_spans_in(file.name(), &in_name, None);

    // The trailing separator goes with the directories, and a file in the project's own
    // directory has neither.
    let directory = file.directory().trim_end_matches('/');
    if directory.is_empty() {
        return spans;
    }
    let above: Vec<Range<usize>> = marks
        .iter()
        .filter(|mark| mark.start < directory.len())
        .map(|mark| mark.start..mark.end.min(directory.len()))
        .collect();
    spans.push(Span::new("  ".to_owned()).color(palette().address_fg));
    spans.extend(marked_spans_in(
        directory,
        &above,
        Some(palette().address_fg),
    ));
    spans
}

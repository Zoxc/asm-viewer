//! The Search panel: the project's directory searched for a pattern, the hits as they
//! arrive, and the rows they are drawn as.
//!
//! `LocationsPanel`'s shape over `src/search.rs`'s walk: a question the reader asks, one
//! answer that stands until the next question replaces it, and a `match` over the state
//! that decides in one place whether the pane says nothing was searched for, that a search
//! is running, that it found nothing, or draws the rows.
//!
//! The search itself is a thread of the app's own, started by the effect in
//! [`use_search_with`] rather than by the press, and its hits come back over a channel
//! [`take_hits`] drains in batches. **Cancellation is the receiver going**: a task whose
//! search has been replaced returns, the channel's other end fails on its next send, and
//! the walk breaks where it stands -- which is a second search, a project left, and the
//! app closing, all through one rule. That is `take_load`'s own (`ui/documents.rs`).

use super::*;
use crate::search::{Hit, SearchEvent, SearchHits, SearchQuery, SearchRow, SearchRows};
use std::ops::ControlFlow;

/// What has been searched for and what it came to, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Searching(pub(crate) State<Searched>);

/// The state of the one search.
///
/// `id` numbers the searches so that a hit can say which one it belongs to: the answer
/// arrives long after the question, and a reader who asked again is not waiting for the
/// first. There is no `capped` field beside the hits, since [`SearchHits`] knows.
#[derive(Clone, Default)]
pub(crate) struct Searched {
    /// Which search is on: bumped by every ask, and what a running task compares itself
    /// against before it writes anything.
    pub(crate) id: u64,
    /// What is being searched for, or [`None`] until anything has been.
    pub(crate) asked: Option<SearchQuery>,
    /// Whether the walk is still going.
    pub(crate) running: bool,
    pub(crate) hits: SearchHits,
    /// Whether the caret is wanted in the box: set by the chord and spent by the panel,
    /// which may not be mounted at the moment the chord is pressed.
    pub(crate) focus: bool,
}

/// Ask for `query` and bring the panel that will answer it to the front. The one writer of
/// [`Searched::asked`], and the only place a search is started from: what actually runs it
/// is the effect in [`use_search_with`], so a press writes state and nothing else.
///
/// Asking again for what is already on screen asks again: the files may have changed since,
/// and an answer is about the directory as it was when it was walked.
pub(crate) fn start_search(
    mut searched: State<Searched>,
    dock: State<DockArea>,
    query: SearchQuery,
) {
    if !query.is_askable() {
        return;
    }
    // Bound before the write, so the read guard is gone by then.
    let id = searched.peek().id.wrapping_add(1);
    searched.set(Searched {
        id,
        asked: Some(query),
        running: true,
        hits: SearchHits::default(),
        focus: false,
    });
    raise_panel(dock, Panel::Search);
}

/// Put the caret in the Search panel's box, raising the panel first. What Ctrl+Shift+F
/// does, from wherever the keyboard is.
///
/// The focus is asked for through the state and not here, because the panel is a dock tab
/// and an inactive one is not mounted: its box has no node to focus until the raise above
/// has been drawn. The panel's own effect spends the flag once it has one.
pub(crate) fn reach_search(mut searched: State<Searched>, dock: State<DockArea>) {
    searched.write().focus = true;
    raise_panel(dock, Panel::Search);
}

/// Run the searches the reader asks for, on a thread of the app's own, and take the hits
/// back into [`Searched`] as they arrive.
///
/// The work is an argument so that a test can put its own hits in the walk's place: a
/// search that answers as fast as it is asked can say nothing about batching, superseding
/// or cancelling, which is the whole of what there is here to get wrong.
pub(crate) fn use_search_with(
    searched: State<Searched>,
    work: impl Fn(&SearchQuery, &mut dyn FnMut(SearchEvent) -> ControlFlow<()>) + Send + Clone + 'static,
) {
    // A memo and not a read: every hit is a write to this state, and an effect reading it
    // would start a new search for each batch of its own answer. The memo recomputes for
    // each of them and wakes nothing, the question being unchanged.
    let asked = use_memo(move || {
        let searched = searched.read();
        (searched.id, searched.asked.clone())
    });

    use_side_effect(move || {
        // Reading the memo subscribes this to the question; the state it writes is peeked.
        let (id, query) = asked.read().clone();
        let Some(query) = query else {
            return;
        };

        // Bounded, and small: a grep finds hits far faster than a window draws them, and a
        // worker parked in a send is one that learns the moment the reader has moved on.
        let (hits, events) = async_channel::bounded::<SearchEvent>(512);
        let work = work.clone();
        // A `std::thread` and not a task: this walks a directory and reads every file in
        // it, and freya's executor is the UI thread.
        // Named, so that a panic on it says which worker died (`crate::panics`).
        let started = std::thread::Builder::new()
            .name("the search worker".to_owned())
            .spawn(move || {
                work(&query, &mut |event| match hits.send_blocking(event) {
                    Ok(()) => ControlFlow::Continue(()),
                    // The receiver is gone: this search has been replaced or the app is
                    // closing, and either way nobody is waiting for the rest of it.
                    Err(_) => ControlFlow::Break(()),
                });
            });
        if let Err(error) = started {
            log::warn!("the search worker could not be started: {error}");
        }

        spawn(take_hits(searched, id, events));
    });
}

/// Take the hits of search `id` as they arrive, until they stop or the search is replaced.
///
/// A batch per wake and not a write per hit: each write is a render, and a walk over a
/// large tree answers in thousands. The batch is dropped whole when the search is no
/// longer the one being asked for -- checked before the write and not only at the end of
/// the loop, or the last batch of the old search would land in the new one's rows.
async fn take_hits(
    mut searched: State<Searched>,
    id: u64,
    events: async_channel::Receiver<SearchEvent>,
) {
    while let Ok(first) = events.recv().await {
        let batch: Vec<SearchEvent> = std::iter::once(first)
            .chain(std::iter::from_fn(|| events.try_recv().ok()))
            .collect();

        // Bound in a statement of its own: the read guard is gone before the write.
        let mine = searched.peek().id == id;
        if !mine {
            // Returning drops the receiver, which is what stops the walk behind it.
            return;
        }

        let mut state = searched.write();
        for event in batch {
            match event {
                SearchEvent::Hit(hit) => state.hits.push(hit),
                SearchEvent::Finished => state.running = false,
            }
        }
    }
}

/// One row of the answer: a file, or one of its matched lines.
#[derive(Clone)]
struct HitRow {
    row: SearchRow,
    searched: State<Searched>,
    key: DiffKey,
}

impl PartialEq for HitRow {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row
    }
}

impl KeyExt for HitRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for HitRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        // Consumed in the render and peeked in the handler, where no hook may run.
        let states = use_project_states();
        let ctrl = use_consume::<Ctrl>().0;
        let land_at = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let marked = use_consume::<Marked>().0;
        let mut searched = self.searched;

        let background = if hovering() {
            palette().object_hover_bg
        } else {
            Color::TRANSPARENT
        };

        let row = self.row.clone();
        let pressed = row.clone();
        let tooltip = match &row {
            SearchRow::File { path, .. } => path.display().to_string(),
            SearchRow::Match(hit) => format!("{}:{}", hit.path.display(), hit.line),
        };

        row_tooltip(
            tooltip,
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .padding(Gaps::new_symmetric(0.0, 5.0))
                .spacing(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| match &pressed {
                    SearchRow::File { path, .. } => {
                        searched.write().hits.toggle(path);
                    }
                    SearchRow::Match(hit) => open_hit(states, land_at, plant, marked, ctrl, hit),
                })
                .children(row_children(&row)),
        )
    }
}

/// What a row draws: a file row is its fold, its name and its count; a match row is its
/// line number and the line, the matched parts of it bold and in `match_fg`.
fn row_children(row: &SearchRow) -> Vec<Element> {
    match row {
        SearchRow::File {
            name,
            count,
            folded,
            ..
        } => vec![
            label()
                .text(if *folded { "\u{25b8}" } else { "\u{25be}" })
                .width(Size::px(CHEVRON_WIDTH))
                .color(palette().icon_fg)
                .into_element(),
            tree_name(name.clone(), false).into_element(),
            label()
                .text(count.to_string())
                .margin(Gaps::new(0.0, 0.0, 0.0, COUNT_GUTTER))
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
        ],
        SearchRow::Match(hit) => vec![
            label()
                .text(hit.line.to_string())
                .width(Size::px(LINE_NUMBER_WIDTH))
                .text_align(TextAlign::Right)
                .color(palette().address_fg)
                .max_lines(1)
                .into_element(),
            rect()
                .width(Size::flex(1.0))
                .overflow(Overflow::Clip)
                .child(
                    paragraph()
                        .width(Size::fill())
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis)
                        .spans_iter(marked_spans(&hit.text, &hit.spans).into_iter()),
                )
                .into_element(),
        ],
    }
}

/// A line cut into the runs that were found and the runs that were not, the found ones
/// bold and in the palette's own colour for them. `marked` are byte ranges into `text`
/// and in order, so this is one walk.
///
/// Shared with the uses list, whose rows mark the name the same way (`ui::locations`).
pub(crate) fn marked_spans(text: &str, marked: &[Range<usize>]) -> Vec<Span<'static>> {
    marked_spans_in(text, marked, None)
}

/// The same over a `base` the unmatched runs are drawn in, for a row that draws part of
/// its text dimmed: the file finder's, whose directories are a step back from the name.
/// `None` leaves them the colour they inherit.
pub(crate) fn marked_spans_in(
    text: &str,
    marked: &[Range<usize>],
    base: Option<Color>,
) -> Vec<Span<'static>> {
    let plain = |text: &str| {
        let span = Span::new(text.to_owned());
        match base {
            Some(colour) => span.color(colour),
            None => span,
        }
    };
    let mut spans = Vec::new();
    let mut at = 0;
    for span in marked {
        if span.start > at {
            spans.push(plain(&text[at..span.start]));
        }
        spans.push(
            Span::new(text[span.clone()].to_owned())
                .color(palette().match_fg)
                .font_weight(FontWeight::BOLD),
        );
        at = span.end;
    }
    if at < text.len() {
        spans.push(plain(&text[at..]));
    }
    spans
}

/// Open a hit: its file as a source-driven tab, landed on the line it was found at, in
/// the temporal tab or a new one as `reach` says.
///
/// The path is spelled the way the Files view spells one -- the entry's own path, never
/// canonicalised -- since a `LinePos` is compared by text and the line would otherwise be
/// picked out in nothing. A file the source pane would refuse opens nothing at all, the
/// Files view's own bound.
fn open_hit(
    states: ProjectStates,
    land_at: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    marked: State<Marks>,
    ctrl: State<bool>,
    hit: &Hit,
) {
    if !shows_as_source(&hit.path) {
        return;
    }
    let file: Arc<str> = Arc::from(&*hit.path.to_string_lossy());
    land(
        states.open,
        states.visits,
        marked,
        land_at,
        plant,
        Landing {
            tab: Document::Source(file.clone()),
            at: Some(LinePos {
                file,
                line: hit.line,
            }),
            address: None,
            columns: hit.columns.clone(),
        },
        reach(ctrl),
    );
}

/// The Search view: a box over every hit the last search found.
#[derive(PartialEq)]
pub(crate) struct SearchPanel;

impl Component for SearchPanel {
    fn render(&self) -> impl IntoElement {
        let searched = use_consume::<Searching>().0;
        let dock = use_consume::<SidebarDock>().0;
        let proj = use_consume::<Proj>().0;
        // The box is the panel's own and not the session's, as a filter is; it starts as
        // whatever was last searched for, so a panel dragged between areas or reached
        // again keeps saying what is on screen under it.
        let filter = use_state(|| {
            searched
                .peek()
                .asked
                .as_ref()
                .map(|query| query.filter.clone())
                .unwrap_or_default()
        });
        let submits = use_state(|| 0u64);
        let directory = given(&proj.read().directory).map(str::to_owned);

        let rows = use_memo(move || searched.read().hits.rows());
        let rows = rows.read().clone();
        let state = searched.read().clone();

        // Enter in the box. Everything it needs is peeked, and nothing captured: an effect
        // that read the filter would run for every character typed and search for half a
        // pattern, and one holding the directory would hold the one the panel first
        // rendered with.
        use_side_effect_with_deps(&submits(), move |count: &u64| {
            if *count == 0 {
                return;
            }
            let directory = given(&proj.peek().directory).map(str::to_owned);
            let Some(directory) = directory else {
                return;
            };
            start_search(
                searched,
                dock,
                SearchQuery {
                    root: PathBuf::from(directory),
                    filter: filter.peek().clone(),
                },
            );
        });

        let body: Element = match (&directory, &state.asked) {
            (None, _) => placeholder("No project directory. Set one in the Project view."),
            (Some(_), None) => placeholder("Nothing searched for yet."),
            (Some(_), Some(query)) if state.running && state.hits.counts().0 == 0 => {
                placeholder(format!("Searching for {}\u{2026}", query.filter.pattern))
            }
            (Some(_), Some(query)) if state.hits.counts().0 == 0 => {
                placeholder(format!("No matches for {}", query.filter.pattern))
            }
            (Some(_), Some(_)) => {
                let length = rows.len();
                rect()
                    .expanded()
                    .content(Content::Flex)
                    .child(section_heading(&heading(&state), None))
                    .child(
                        rect().width(Size::fill()).height(Size::flex(1.0)).child(
                            VirtualScrollView::new_with_data(
                                (rows, searched),
                                |index, (rows, searched): &(SearchRows, State<Searched>)| {
                                    let row = rows.row(index);
                                    HitRow {
                                        row: row.clone(),
                                        searched: *searched,
                                        key: DiffKey::None,
                                    }
                                    .key(&index)
                                    .into()
                                },
                            )
                            .length(length)
                            .item_size(list_row_height()),
                        ),
                    )
                    .into_element()
            }
        };

        let (pane, box_id) = use_search_pane(filter, submits, palette().pane_bg, body);

        // The caret the chord asked for, spent here: the panel is mounted by now, which
        // is the whole reason the chord leaves a flag rather than asking for the focus
        // itself. Writing it back is what keeps a later mount from stealing the keyboard.
        use_side_effect_with_deps(&state.focus, move |wanted: &bool| {
            if *wanted {
                let mut searched = searched;
                box_id.request_focus();
                searched.write().focus = false;
            }
        });

        pane
    }
}

/// What is said over the rows: how much was found, and whether the search is still going
/// or stopped at its cap.
fn heading(state: &Searched) -> String {
    let (hits, files) = state.hits.counts();
    let matches = if hits == 1 { "match" } else { "matches" };
    let files_word = if files == 1 { "file" } else { "files" };
    if state.running {
        return format!("{hits} {matches} in {files} {files_word}\u{2026}");
    }
    if state.hits.capped() {
        return format!("First {hits} {matches} in {files} {files_word}");
    }
    format!("{hits} {matches} in {files} {files_word}")
}

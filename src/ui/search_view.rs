//! The Search panel: the project's directory searched for a pattern, the hits as they
//! arrive, and the rows they are drawn as.
//!
//! `LocationsPanel`'s shape over `src/search.rs`'s walk: a question the reader asks, one
//! answer that stands until the next question replaces it, and a `match` over the state
//! that decides in one place whether the pane says nothing was searched for, that a search
//! is running, that it found nothing, or draws the rows.
//!
//! The search itself is a [`stream`] (`src/ui/worker.rs`), started by the effect in
//! [`use_search_with`] rather than by the press, and its hits come back over a channel
//! [`take_hits`] drains in batches. **Cancellation is the receiver going**: a task whose
//! search has been replaced returns, the channel's other end fails on its next send, and
//! the walk breaks where it stands -- which is a second search, a project left, and the
//! app closing, all through one rule. It is the shape's rule and not this file's: the
//! binary loader (`take_load`, `ui/documents.rs`) is stopped by the same line.

use super::*;
use crate::search::{self, SearchEvent, SearchHits, SearchQuery, SearchRows};
use std::ops::ControlFlow;

/// What has been searched for and what it came to, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Searching(pub(crate) State<Searched>);

/// The state of the one search.
///
/// `id` numbers the searches so that a hit can say which one it belongs to: the answer
/// arrives long after the question, and a reader who asked again is not waiting for the
/// first. There is no `capped` field beside the hits: [`search::capped`] answers that
/// off the count.
///
/// **Not [`Clone`]**: this holds every hit a search found, up to [`search::MAX_HITS`] of
/// them, and everything that draws them reads what it needs under the guard.
#[derive(Default)]
pub(crate) struct Searched {
    /// Which search is on: bumped by every ask, and what a running task compares itself
    /// against before it writes anything.
    pub(crate) id: u64,
    /// What is being searched for, or [`None`] until anything has been.
    pub(crate) asked: Option<SearchQuery>,
    /// Whether the walk is still going.
    pub(crate) running: bool,
    pub(crate) hits: SearchHits,
}

/// What the panel says about an answer, which is all it needs of one besides the
/// question: how much was found, whether the walk is still going, and whether it stopped
/// at its cap. Taken under the one guard the render reads, so the hits themselves are
/// never copied to draw them.
#[derive(Clone, Copy)]
struct Summary {
    running: bool,
    hits: usize,
    files: usize,
    capped: bool,
}

impl Searched {
    /// The four counts the panel is drawn from.
    fn summary(&self) -> Summary {
        Summary {
            running: self.running,
            hits: self.hits.count(),
            files: self.hits.files(),
            capped: search::capped(&self.hits),
        }
    }

    /// Take a batch of events from search `id`. Whether they are this search's, so the
    /// caller goes on taking them only then.
    ///
    /// The id check is here and not in the task: the answer arrives long after the
    /// question, and a reader who asked again is not waiting for the first search. It is
    /// asked of the **batch** and not of each event, since one batch is one search's by
    /// construction.
    fn take(&mut self, id: u64, batch: Vec<SearchEvent>) -> bool {
        if self.id != id {
            return false;
        }
        for event in batch {
            match event {
                SearchEvent::Hit(path, hit) => self.hits.push(&path, hit),
                SearchEvent::Finished => self.running = false,
            }
        }
        true
    }
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
    });
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
    // Every hit is a write to this state, and what keeps a new search from being started
    // for each batch of its own answer is [`use_asking`]'s memo: it recomputes for each of
    // them and wakes nothing, the question being unchanged. The id is part of the
    // question, since asking again for what is already on screen is a question again.
    use_asking(
        move || {
            let searched = searched.read();
            let query = searched.asked.clone()?;
            Some((searched.id, query))
        },
        unmarked,
        move |(id, query)| {
            // A `std::thread` and not a task: this walks a directory and reads every file
            // in it, and freya's executor is the UI thread. What stops one search when
            // the next is asked for is [`take_hits`] letting go of the receiver.
            //
            // 512, and bounded, because a grep finds hits far faster than a window draws
            // them and a worker parked in a send is one that learns the moment the reader
            // has moved on.
            let work = work.clone();
            let events = stream("the search worker", Some(512), move |emit| {
                work(&query, emit)
            });

            spawn(take_hits(searched, id, events));
        },
    );
}

/// Take the hits of search `id` as they arrive, until they stop or the search is replaced.
///
/// A batch per wake and not a write per hit: each write is a render, and a walk over a
/// large tree answers in thousands. Whether a batch is this search's is
/// [`Searched::take`]'s to say, and it says so before taking any of it, or the last batch
/// of the old search would land in the new one's rows.
///
/// **Written through the guard and not by [`write_if`]**: what is held is the answer
/// itself, up to [`crate::search::MAX_HITS`] of it, and a clone per batch would copy every
/// hit found so far to add the few that have just arrived. The one batch that costs a
/// render for nothing is the first of a search the reader has replaced, and the return
/// below is the last thing this task does.
async fn take_hits(
    mut searched: State<Searched>,
    id: u64,
    events: async_channel::Receiver<SearchEvent>,
) {
    while let Some(batch) = next_batch(&events).await {
        let mut state = searched.write();
        if !state.take(id, batch) {
            // Returning drops the receiver, which is what stops the walk behind it.
            return;
        }
    }
}

/// The Search view: a box over every hit the last search found.
#[derive(PartialEq)]
pub(crate) struct SearchPanel;

impl Component for SearchPanel {
    fn render(&self) -> impl IntoElement {
        let searched = use_consume::<Searching>().0;
        let dock = use_consume::<SidebarDock>().0;
        let workspace = use_consume::<Workspace>().0;
        let pane = use_list_pane(Panel::Search);
        // What Enter on a row reaches through, which the pane has already consumed for
        // its rows: taken off that bundle rather than reached for again, so the panel's
        // keys and its rows cannot come to open a hit two ways ([`ListStates`]).
        let ListStates { doors, ctrl, .. } = pane.states;
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
        // The box filters nothing here -- Enter asks the question instead -- so the one
        // compiled filter is only what the bar prints a bad pattern from.
        let marking = use_list_marking(filter);
        let marking = marking.read().clone();
        let directory = workspace.read().clone();

        let rows = use_memo(move || searched.read().hits.rows(&Matcher::Everything));
        let rows = rows.read().clone();
        // The question and the counts under one guard, rather than a copy of the answer:
        // `Searched` holds every hit found so far, up to [`search::MAX_HITS`] of them,
        // and the panel renders once per batch while a search streams. A copy here is a
        // pointer bump per hit and an allocation per file, on the UI thread, per render
        // (`grouped.rs`). The rows are the memo above.
        let (asked, summary) = {
            let state = searched.read();
            (state.asked.clone(), state.summary())
        };

        // Enter in the box. Everything it needs is peeked, and nothing captured: an effect
        // that read the filter would run for every character typed and search for half a
        // pattern, and one holding the directory would hold the one the panel first
        // rendered with.
        use_side_effect_with_deps(&submits(), move |count: &u64| {
            if *count == 0 {
                return;
            }
            let directory = workspace.peek().clone();
            let Some(directory) = directory else {
                return;
            };
            start_search(
                searched,
                dock,
                SearchQuery {
                    root: directory,
                    filter: filter.peek().clone(),
                },
            );
        });

        // The rows the arrows step and Enter presses: a `SearchRows` is the rows behind an
        // `Arc`, so handing them over is a pointer.
        let keys = ListKeys::over(rows.clone(), place_pick, move |row| {
            press_place(doors, ctrl, Folding::Hits(searched), row)
        });
        let body: Element = match (&directory, &asked) {
            (None, _) => placeholder("No project directory. Set one in the Project view."),
            (Some(_), None) => placeholder("Nothing searched for yet."),
            (Some(_), Some(query)) if summary.running && summary.hits == 0 => {
                placeholder(format!("Searching for {}\u{2026}", query.filter.pattern))
            }
            (Some(_), Some(query)) if summary.hits == 0 => {
                placeholder(format!("No matches for {}", query.filter.pattern))
            }
            (Some(_), Some(_)) => {
                let length = rows.len();
                headed(
                    section_heading(&heading(summary), None).into_element(),
                    pane.virtual_rows(
                        length,
                        (rows, searched),
                        |index, (rows, searched): &(SearchRows, State<Searched>), states| {
                            PlaceRow {
                                row: rows[index].clone(),
                                folding: Folding::Hits(*searched),
                                at: index,
                                states,
                                key: DiffKey::None,
                            }
                            .key(&index)
                            .into()
                        },
                    ),
                )
                .into_element()
            }
        };

        // The caret Ctrl+Shift+F asks for is not asked for here: the box was registered
        // as this panel's by `use_list_pane`, and the one ask every chord and every
        // opened row leaves is spent on it at the root (`ui/keyboard.rs`).
        pane.searched(filter, submits, &marking, keys, body)
    }
}

/// What is said over the rows: how much was found, and whether the search is still going
/// or stopped at its cap.
fn heading(summary: Summary) -> String {
    let Summary {
        running,
        hits,
        files,
        capped,
    } = summary;
    let matches = if hits == 1 { "match" } else { "matches" };
    let files_word = if files == 1 { "file" } else { "files" };
    if running {
        return format!("{hits} {matches} in {files} {files_word}\u{2026}");
    }
    if capped {
        return format!("First {hits} {matches} in {files} {files_word}");
    }
    format!("{hits} {matches} in {files} {files_word}")
}

#[cfg(test)]
mod tests;

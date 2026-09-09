//! The app's root state: the contexts that belong to no one mechanism, and the bundles the
//! app is passed around in.
//!
//! **A context lives with the mechanism that owns it**, and so does the bundle that groups
//! it: [`Doors`] in `focus.rs`, [`Marked`] in `marks.rs`, [`Loading`] in `loading.rs`, the
//! `Pad*` family in `pad.rs`.
//! What is left is here -- the objects, the store, the project, the window's arrangement --
//! with [`Open`], [`Places`] and [`ProjectStates`], each of which spans three modules or
//! more and is owned by none.
//!
//! **A bundle is a context of its own**, provided once by `app()` and taken in one
//! `use_consume`, so a state added to a group is a field and not a parameter threaded
//! through every function of the group. A handle may sit in more than one bundle:
//! [`Doors`] and [`ProjectStates`] both carry [`Open`], and [`Doors`] carries the runs
//! [`Marked`] hands the panes.
//!
//! Two of the names are **derivations and not states**: `Active` is a `Memo` over the strip
//! and the document table, and `Symbols` a `Memo` over `Objects`.
//!
//! One name here is **no context at all**: [`Placing`], the plain key saying which of the
//! two places a code pane is drawn in. The find bars, the section listings and the pane
//! toggle are each told it and none of them owns it, so it is written down once, beside
//! the maps it keys.

use super::*;

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Objects(pub(crate) State<Vec<Arc<Object>>>);

/// Where everything this run stores goes, opened once in `app()` and handed down rather
/// than looked up again wherever a file is wanted. `None` on a system with no state or
/// local data directory, which is a run that keeps nothing and says so at each write.
///
/// A `State` and not a plain value so that the bundle below stays `Copy`; nothing ever
/// writes it, so it is always read with `peek`.
#[derive(Clone, Copy)]
pub(crate) struct Storage(pub(crate) State<Option<Store>>);

/// The projects the reader has had open, out of the store this run keeps — or none, on a
/// run that keeps nothing. The three views that draw the list ask through here.
pub(crate) fn recents_of(store: State<Option<Store>>) -> Vec<Recent> {
    store
        .peek()
        .as_ref()
        .map(project::recent_projects)
        .unwrap_or_default()
}

/// The active tab and the document it shows, shared through context.
///
/// **A derivation and not a state**: the strip's active tab, read through [`Docs`] -- see
/// [`active_tab`]. `None` means both "nothing is open" and "the tab on screen is a page",
/// and deliberately does not distinguish them. The id travels with the document because
/// the two are read together: the driven line and the viewing positions are kept per tab
/// *and* entry, and a document paired with an id read a beat apart would be another tab's
/// for that beat, which the worker would answer with a re-ask.
///
/// A [`Memo`] because the strip is written by more than the opening of a document -- a
/// page raised, a tab moved along the bar, a page closed -- and none of those changes what
/// any pane is drawing. **It is therefore a beat behind**, which is right for
/// anything that renders and wrong for anything that must be true inside one event
/// handler -- so the functions holding the invariants call [`active_tab`] on the states
/// directly instead of reading this.
#[derive(Clone, Copy)]
pub(crate) struct Active(pub(crate) Memo<Option<Entry>>);

/// What is open: the strip of tabs the reader arranged, and the table saying what each
/// document tab stands for.
///
/// The strip's tabs *are* the list of open tabs, in the reader's own order; [`Docs`] holds
/// no order, only the trail behind each document tab's id. Membership is the one thing the
/// two share, and `open_document`/`close_tab`/`close_binary` keep it true: a tab and its
/// trail are made together and closed together.
#[derive(Clone, Copy)]
pub(crate) struct Open {
    pub(crate) strip: State<Strip>,
    pub(crate) docs: State<Docs>,
}

/// Every open document tab's id, in the order the reader's tabs are in.
pub(crate) fn open_ids(strip: &Strip) -> Vec<DocId> {
    strip.documents().collect()
}

/// The active tab and what it shows: the tab on screen, when that tab is a document.
pub(crate) fn active_tab(strip: &Strip, docs: &Docs) -> Option<Entry> {
    match strip.active()? {
        Tab::Document(id) => docs.current(id).cloned().map(|stop| (id, stop)),
        Tab::Page(_) => None,
    }
}

/// The active document alone.
pub(crate) fn active_document(strip: &Strip, docs: &Docs) -> Option<Document> {
    active_tab(strip, docs).map(|(_, stop)| stop.document)
}

impl Open {
    /// The active document as of *now*, for the event handlers that cannot wait a beat
    /// for [`Active`] to catch up. `peek`, so asking subscribes nothing.
    pub(crate) fn active(&self) -> Option<Document> {
        self.active_stop().map(|(_, stop)| stop.document)
    }

    /// The active tab and the place on its trail it is at, as of now. The document is
    /// what most callers want ([`Open::active`]); this is for the few that key by place.
    pub(crate) fn active_stop(&self) -> Option<Entry> {
        let (strip, docs) = (self.strip.peek(), self.docs.peek());
        active_tab(&strip, &docs)
    }

    /// The active tab as of now, with the document it shows. `peek`, for the same reason.
    pub(crate) fn active_tab(&self) -> Option<(DocId, Document)> {
        self.active_stop().map(|(id, stop)| (id, stop.document))
    }

    /// The active tab's id as of now, a document or not.
    pub(crate) fn active_id(&self) -> Option<DocId> {
        self.active_tab().map(|(id, _)| id)
    }

    /// Every open document tab's id as of now, in tab order.
    pub(crate) fn ids(&self) -> Vec<DocId> {
        open_ids(&self.strip.peek())
    }
}

/// Which of the two places a code pane is drawn in: a document tab, or the Scratchpad's.
/// What every pane, bar and toggle that either can have is told apart by, and so what
/// says whether the pane's place, its runs and its find bar are filed under anything.
///
/// **A tab's are filed under its id** -- in [`Places`] here and in [`Finds`] -- and are
/// forgotten with the tab by the three closers. The Scratchpad's pane is no tab and has
/// no [`DocId`] to file them under; an entry under a made-up one would hold whatever its
/// document points into with nothing that would ever forget it, so it keeps what it keeps
/// beside the pad itself and closes with the app.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Placing {
    Tab(DocId),
    /// The Scratchpad's own pane, over the program its pad built.
    Pad,
}

/// Everything kept per **place** -- a tab and one of the stops on its trail, an [`Entry`]
/// -- and so everything a closer has to let go of together.
///
/// One type because an `Entry` key holds the `Arc<Object>` its document points into: a map
/// that keeps an entry a closed tab left behind holds that binary's bytes for as long as
/// the app runs. A closer that forgot four of the five compiled and leaked, so the forget
/// is one call ([`Places::forgetting`]) and not five lines to copy.
///
/// A place and not a document: two addresses in one object's code are two entries, which
/// is what makes a step inside that listing come back to the row it was left at as any
/// other step does. Kept at the root and never in a pane, which reuses one scroll
/// controller for every symbol and so would leave a newly opened function at the offset
/// the old one was at.
#[derive(Clone, Copy)]
pub(crate) struct Places {
    /// Which row each place had its **assembly** side left on.
    pub(crate) asm_at: State<Positions<Entry>>,
    /// The other half: which row its **source** side was left on, keyed by the same entry
    /// rather than by the file the pane happens to be showing.
    pub(crate) src_at: State<Positions<Entry>>,
    /// Where a listing of an object's whole code was left, as an address: its rows are
    /// counted afresh with every answer, so a row number would mean nothing for long.
    pub(crate) code_at: State<Positions<Entry, Spot>>,
    /// What each place had picked out in each pane when it was last shown. Never saved: a
    /// run is a view of a tab.
    pub(crate) marks_at: State<Positions<Entry, Kept>>,
    /// Which line each source-driven tab's assembly side is driven from, and which of the
    /// symbols that line compiles into it follows. The same kind of thing as the four
    /// above: a fact about a place, made by a click in it and forgotten with it.
    pub(crate) driven: State<Driven>,
    /// The find bar over each code pane, if any. Keyed by the **tab** where the five
    /// above are keyed by a place on its trail, so a step Back leaves a bar as the reader
    /// left it -- but forgotten by the same closers, a bar holding the file or the symbol
    /// it is about.
    pub(crate) finds: State<Finds>,
}

impl Places {
    /// The five maps, empty: what `app()` provides and what a test harness makes.
    pub(crate) fn create() -> Places {
        Places {
            asm_at: State::create(Positions::default()),
            src_at: State::create(Positions::default()),
            code_at: State::create(Positions::default()),
            marks_at: State::create(Positions::default()),
            driven: State::create(Driven::default()),
            finds: State::create(Finds::default()),
        }
    }

    /// Let go of every entry `keep` answers false for, and every find bar over a tab
    /// `keeps` answers false for, in all six maps and under one write each. What every
    /// closer ends with, and the whole of what it owes.
    ///
    /// Two predicates because the bars are keyed by the tab alone: a closer knows which
    /// tabs are going, and the two questions are that one asked of a place and of a tab.
    pub(crate) fn forgetting(self, keep: impl Fn(&Entry) -> bool, keeps: impl Fn(&DocId) -> bool) {
        let Places {
            mut asm_at,
            mut src_at,
            mut code_at,
            mut marks_at,
            mut driven,
            mut finds,
        } = self;
        // The scratchpad's listing is no tab and closes with the app, so it is kept
        // whatever a closer says about the tabs.
        finds.write().forgetting(|placing| match placing {
            Placing::Tab(tab) => keeps(tab),
            Placing::Pad => true,
        });
        asm_at.write().forgetting(&keep);
        src_at.write().forgetting(&keep);
        code_at.write().forgetting(&keep);
        marks_at.write().forgetting(&keep);
        // One guard rather than one write per tab: a write notifies whether or not it
        // changed anything, and a dozen tabs closing is one change.
        driven.write().forgetting(&keep);
    }
}

/// The sidebar's dock, so a panel can be brought to the front from anywhere -- the
/// Search box's chord, a Locations question asked in a code pane.
#[derive(Clone, Copy)]
pub(crate) struct SidebarDock(pub(crate) State<DockArea>);

/// How wide the **leading** side of a document is, as a percentage -- the side the tab is
/// driven from, which `DocumentBody` draws on the left in both kinds of tab. Kept by place
/// and not by pane, so switching from an assembly-driven tab to a source-driven one leaves
/// the handle where the reader put it instead of throwing the two widths across the split.
///
/// One number for the app, held out here because the container will not remember it: only
/// the active tab's content is mounted, and a `ResizablePanel` registers at its
/// `initial_size` in a `use_hook` and removes its entry in a `use_drop`, so a remount comes
/// back at the initial sizes under new panel ids.
#[derive(Clone, Copy)]
pub(crate) struct SplitRatio(pub(crate) State<f32>);

/// The `ResizableContext` the document's two panels register into, so a drag on the handle
/// can be read back out. See [`SplitRatio`].
#[derive(Clone, Copy)]
pub(crate) struct Splits(pub(crate) State<ResizableContext>);

/// How wide the sidebar is, in pixels. [`SplitRatio`]'s shape and for its reason: a
/// `ResizablePanel` registers at its `initial_size` and forgets on unmount, so a container
/// that is rebuilt -- which the window's body is, whenever a project arrives or goes --
/// comes back at the initial size unless the number is held out here.
///
/// Pixels and not a percentage: this panel is `PanelSize::px`, so what the context holds
/// after a drag is a literal width.
#[derive(Clone, Copy)]
pub(crate) struct SidebarWidth(pub(crate) State<f32>);

/// The `ResizableContext` the sidebar and the content register into. See [`SidebarWidth`].
#[derive(Clone, Copy)]
pub(crate) struct SidebarSplits(pub(crate) State<ResizableContext>);

/// Which tabs have the section under their Assembly pane's symbol bar open.
///
/// **Per tab and never in the pane**, which is mounted afresh for every document: a
/// `use_state` there would collapse the section at every switch of tab, and a reader who
/// opened it once would find it shut every time they came back.
///
/// **Keyed by [`DocId`] alone and not by [`Entry`]**, unlike everything in [`Places`],
/// and that is what makes it cost nothing: a `DocId` is `Copy + Hash` and
/// holds no `Arc<Object>`, where a document does and would have to be forgotten in all
/// three of `close_tab`, `close_others` and `close_binary` or a closed binary's bytes
/// would be held for as long as the app ran. Ids are never handed out twice
/// ([`Docs::open`]), so an entry a closed tab left behind can never be mistaken for
/// another tab's -- it is dead weight of four bytes, and a reopened tab correctly opens
/// with its section shut. The Objects tree's fold set makes the same argument. It follows
/// that the section stays open or shut across the whole of a tab's trail, which is a
/// fact about the tab and not about any one place on it.
///
/// Never saved: it is a view of a tab, like a filter.
#[derive(Clone, Copy)]
pub(crate) struct Expanded(pub(crate) State<HashSet<DocId>>);

/// What the reader has said about each tab's following pane -- the one it is not driven
/// from: `true` where they brought it back, `false` where they put it away. A tab with
/// nothing here has said nothing, and opens as its document says ([`following`]).
///
/// A `bool` per tab and not the set [`Expanded`] is, because there is no one default to
/// be absent from: a source-driven tab on a `Cargo.toml` opens with its assembly side
/// away and every other tab opens with both panes.
///
/// Keyed by [`DocId`] alone and never saved, for [`Expanded`]'s reasons: an id is
/// `Copy + Hash`, holds no `Arc<Object>` and is never handed out twice, so a closed tab
/// leaves a byte behind that no other tab can be given, and this is a view of a tab.
#[derive(Clone, Copy)]
pub(crate) struct Follows(pub(crate) State<HashMap<DocId, bool>>);

/// Where the reader chose to be able to come back to: the project's bookmarks, in their
/// saved shape and nothing more. Whether one is live is asked of [`Objects`] where it is
/// drawn, so a closed binary takes no bookmark with it and holds no `Arc` through one.
#[derive(Clone, Copy)]
pub(crate) struct Bookmarked(pub(crate) State<Bookmarks>);

/// The open project, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Proj(pub(crate) State<OpenProject>);

/// The settings, shared through context. A root context and not state inside the settings
/// page, which is a tab that may not be open at all. The page edits this;
/// `use_settings_with` is what notices.
#[derive(Clone, Copy)]
pub(crate) struct Prefs(pub(crate) State<EditedSettings>);

/// What the Shortcuts page's box is filtering by. A root context and not state inside the
/// page, for [`Prefs`]'s reason twice over: a page is a tab that may not be open, and only
/// the tab on screen is mounted -- so a filter owned by the page would be emptied by a
/// glance at another tab, which is exactly when a reader looks a gesture up.
///
/// Not saved with the session. It is what the reader is looking for now, and a box that
/// came back filtered from a restart would read as a page with most of its rows missing.
#[derive(Clone, Copy)]
pub(crate) struct Shortcuts(pub(crate) State<Filter>);

/// Where each file that would not parse was moved to, until the reader has been told: what
/// [`RescuedPopup`] draws, and empty for every run in which nothing was moved.
///
/// A state at the root and not one inside the popup, because what fills it is a *load*
/// (`store::moved`), and the two loads a run makes are the startup's and a project
/// switch's -- neither of them anywhere near a component that could own this.
#[derive(Clone, Copy)]
pub(crate) struct Rescued(pub(crate) State<Vec<PathBuf>>);

/// A project that would not open, and why, until the reader has been told.
///
/// A project file is never moved aside -- it may be their own file, beside their code -- so
/// one that will not parse is left exactly where it is and nothing is written over it. That
/// makes telling them the whole of what happens, and this is what carries the reason as
/// far as [`UnopenedPopup`].
#[derive(Clone, Copy)]
pub(crate) struct Unopened(pub(crate) State<Option<project::Failure>>);

/// Whether the reader is being asked to confirm deleting the open project, and what it is
/// called while they answer.
///
/// The label and not the path: it is what the question names, and reading it once when the
/// question is asked is what keeps [`DeleteProjectPopup`] from having to be told the
/// project again.
#[derive(Clone, Copy)]
pub(crate) struct Deleting(pub(crate) State<Option<String>>);

/// Every state a project owns, in one `Copy` bundle of handles: a project switch closes
/// all of them and reopens all of them. Provided by `app()` and taken whole
/// ([`use_project_states`]), so this list exists in the struct and in the one place that
/// builds it.
#[derive(Clone, Copy)]
pub(crate) struct ProjectStates {
    pub(crate) proj: State<OpenProject>,
    /// Where the project's own files go. Not a project's state either, and here for
    /// `arranged`'s reason: everything that opens, saves or leaves a project needs it.
    pub(crate) store: State<Option<Store>>,
    pub(crate) objects: State<Vec<Arc<Object>>>,
    /// The files on their way into `objects`. Leaving a project abandons them too,
    /// including the ones that have produced nothing yet and so are not in `objects` to be
    /// closed one by one.
    pub(crate) loading: State<Loads>,
    /// The strip and the id table: what is open, and in what order.
    pub(crate) open: Open,
    /// Everything kept per place, which a close forgets together.
    pub(crate) places: Places,
    /// Everywhere the reader has been, across every tab: what the History panel lists.
    pub(crate) visits: State<Visits>,
    pub(crate) bookmarks: State<Bookmarks>,
    /// What the project's directory was last searched for, and what was found in it.
    pub(crate) searched: State<Searched>,
    /// What the project's own workspace built, and what a build replaces.
    pub(crate) build: State<Builds>,
    /// How the window itself is arranged. Not a project's state, and here all the same:
    /// it is written into the session, and a restore has to put it back.
    pub(crate) arranged: Arrangement,
}

impl ProjectStates {
    /// Whether the app holds `path` already, out of the two states that between them say
    /// so: the objects read from it, and the loads still running. The rule itself is
    /// `tree::holds`; this is the peek in front of it, so that a handler asking the
    /// question does not spell the pair out again.
    ///
    /// Peeked and not read: this is asked in an event handler, where a subscription would
    /// belong to whatever scope happened to be rendering.
    pub(crate) fn holds_path(&self, path: &Path) -> bool {
        crate::tree::holds(&self.objects.peek(), &self.loading.peek(), path)
    }
}

/// The three states a session's `[ui]` is kept in: what a restore writes and what the
/// save observer reads back out.
///
/// **Held as states and not reached for.** They live in contexts of their own, and the one
/// way to a context is `use_consume`, which is a hook -- so a restore that asked for them
/// itself would be calling hooks from wherever it was called from. A restore runs inside
/// `use_hook` at startup and inside a press handler on a switch, and neither may
/// (`src/ui/session.rs`, `restore_ui`). Consumed by whoever is rendering and handed
/// down instead.
#[derive(Clone, Copy)]
pub(crate) struct Arrangement {
    pub(crate) dock: State<DockArea>,
    pub(crate) sidebar: State<f32>,
    pub(crate) split: State<f32>,
}

/// What is open, as a component sees it: the strip and the id table together.
pub(crate) fn use_open() -> Open {
    use_consume::<Open>()
}

/// Everything kept per place, as a component sees it.
pub(crate) fn use_places() -> Places {
    use_consume::<Places>()
}

/// The project's states as a component sees them: through the context the root provides,
/// so a view that switches projects needs none of them handed down to it.
pub(crate) fn use_project_states() -> ProjectStates {
    use_consume::<ProjectStates>()
}

/// The window's arrangement, out of the three contexts it is kept in.
pub(crate) fn use_arrangement() -> Arrangement {
    Arrangement {
        dock: use_consume::<SidebarDock>().0,
        sidebar: use_consume::<SidebarWidth>().0,
        split: use_consume::<SplitRatio>().0,
    }
}

/// Every object's text symbols flattened into one list, rebuilt only when the object list
/// changes and shared through context so the Symbols tab does not have to rebuild it. A
/// [`Shared`], so passing it around is a pointer and not a walk of a hundred thousand
/// symbols.
#[derive(Clone, Copy)]
pub(crate) struct Symbols(pub(crate) Memo<Shared<Symbol>>);

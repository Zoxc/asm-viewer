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
//! [`Doors`] and [`ProjectStates`] both carry [`Open`] and [`Places`], and [`Doors`]
//! carries the runs [`Marked`] hands the panes. They are the root's own handles either
//! way, taken in one render, so no route to one can disagree with another.
//!
//! [`RowStates`] and [`ListStates`] are the two bundles that are **not** contexts. Each is
//! gathered from half a dozen of them where a list renders -- a code listing's by
//! [`use_row_states`], a panel's by [`use_list_states`] -- and carried to the rows as data:
//! what they group is what a row's press and its menu reach for, and a handler may not run
//! a hook.
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

/// `f(before, now)` whenever the deps `now()` answers change, `before` being what the last
/// run saw and [`None`] on the **first run** -- which `f` may return early on, there being
/// nothing to compare with yet.
///
/// **That first run is not the mount.** An effect runs a beat after the render that
/// registered it, so the deps it first sees may already have moved. An `f` that is about
/// something the *render* made, rather than about the deps alone, seeds the comparison
/// with what the render used and never takes [`None`] for "unchanged" (`FilesPanel`).
///
/// **What wakes the effect is what `now()` reads**, and every other `.read()` `f` makes at
/// any depth: `use_side_effect` runs its closure inside a `ReactiveContext`, so a read is a
/// subscription wherever it is written. The deps are what a wake is *judged* by, and one
/// that finds them unmoved calls nothing. They are read inside the effect and not in the
/// render, which is what a [`Memo`] source needs: the effect follows it, and the scope that
/// called this follows nothing.
///
/// `now` hands back an owned value, so a read guard it took is over before `f` runs -- and
/// `f` may write the very state the deps came out of.
///
/// The one hook for "not on the mount", which two mechanisms each kept bookkeeping of their
/// own for ([`use_language`], `RecentsSection`).
pub(crate) fn use_on_change<D: PartialEq + 'static>(
    mut now: impl FnMut() -> D + 'static,
    mut f: impl FnMut(Option<&D>, &D) + 'static,
) {
    // A plain captured value and not an `Rc<RefCell>` or a `State`: the closure is `FnMut`
    // and owns it, and nothing but the effect ever looks at it.
    let mut seen: Option<D> = None;
    use_side_effect(move || {
        let now = now();
        if seen.as_ref() == Some(&now) {
            return;
        }
        f(seen.as_ref(), &now);
        seen = Some(now);
    });
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
/// two share, and the three methods below are the only way a **document** tab joins it or
/// leaves it: each does both halves, so a chip and its trail are made together and closed
/// together whatever the caller does, and no trail can outlive its chip. A page is written
/// on the strip alone, having no trail to keep in step.
///
/// They are the **mechanism and not a door**: what opens a document is `open_document`
/// and what closes one is `close_tab`, `close_others` or `close_binary`
/// (`src/ui/documents.rs`), where every rule about reaching or letting go of a place
/// lives. These say only what a tab is made of, and the restore calls them for the same
/// reason a closer does.
#[derive(Clone, Copy)]
pub(crate) struct Open {
    pub(crate) strip: State<Strip>,
    pub(crate) docs: State<Docs>,
}

/// The active tab and what it shows: the tab on screen, when that tab is a document.
///
/// **The one rule**, over borrowed guards so the caller says what asking costs: `read`
/// in a component, which subscribes it, and `peek` in an event handler, which does not.
/// [`Open::now`] is this over two peeks and [`Active`] this over two reads.
pub(crate) fn active_tab(strip: &Strip, docs: &Docs) -> Option<Entry> {
    match strip.active()? {
        Tab::Document(id) => docs.current(id).cloned().map(|stop| (id, stop)),
        Tab::Page(_) => None,
    }
}

impl Open {
    /// The active tab as of *now*, for the event handlers that cannot wait a beat for
    /// [`Active`] to catch up. `peek`, so asking subscribes nothing.
    ///
    /// The whole entry, id and place, as [`Active`] holds one: a caller wanting one
    /// half `.map`s for it, and a caller wanting both takes both from the one answer rather
    /// than asking twice. Only the document has a name of its own ([`Open::active`]), being
    /// asked for far more often than the rest.
    ///
    /// [`None`] where [`active_tab`] is: nothing open, **or a page on screen**. A page has
    /// no id here, which is what every caller wants -- `navigate` must do nothing on one,
    /// and the rest compare against document ids.
    pub(crate) fn now(&self) -> Option<Entry> {
        let (strip, docs) = (self.strip.peek(), self.docs.peek());
        active_tab(&strip, &docs)
    }

    /// The active document as of now: the half of [`Open::now`] most callers want.
    pub(crate) fn active(&self) -> Option<Document> {
        self.now().map(|(_, stop)| stop.document)
    }

    /// Every open document tab's id as of now, in the reader's tab order.
    pub(crate) fn ids(&self) -> Vec<DocId> {
        self.strip.peek().documents().collect()
    }

    /// Make a tab showing `stop` alone, temporal or not, and show it beside the tab on
    /// screen: the trail and the chip in one step. The id the tab is known by.
    ///
    /// For `open_stop` and nothing else -- see the type.
    pub(crate) fn open_tab(&self, stop: Stop, temporal: bool) -> DocId {
        let (mut strip, mut docs) = (self.strip, self.docs);
        // In a scope of its own, so the write guard is gone before the strip is written.
        let id = {
            let mut docs = docs.write();
            let id = docs.open(stop);
            if temporal {
                docs.mark_temporal(id);
            }
            id
        };
        strip.write().show(Tab::Document(id));
        id
    }

    /// Put a saved tab back: the whole of `trail` behind it, and its chip at `position`
    /// rather than beside the tab on screen, a restore stating the saved order outright.
    /// `None` for a trail with nothing on it, which is no tab at all and takes no chip.
    ///
    /// `filling` is handed the new id **before** the chip goes in the bar, for the maps a
    /// restore writes directly: a pane looks at them when it notices that what it shows
    /// has changed, so a row arriving after the tab is on screen arrives too late.
    ///
    /// For the session restore and nothing else -- see the type.
    pub(crate) fn insert_tab(
        &self,
        trail: History,
        temporal: bool,
        position: usize,
        filling: impl FnOnce(DocId),
    ) -> Option<DocId> {
        let (mut strip, mut docs) = (self.strip, self.docs);
        // A statement of its own, so the guard is gone before `filling` writes anything.
        let id = docs.write().open_trail(trail, temporal)?;
        filling(id);
        strip.write().insert(Tab::Document(id), position);
        Some(id)
    }

    /// Close every tab `closing` answers true for -- the chips and the trails behind them
    /// -- landing on the neighbour when the tab on screen was one of them. The documents
    /// that went, empty when the bar lost none.
    ///
    /// What and not whether, where [`Strip::close`] answers whether: the ids are worked
    /// out here anyway, to close the trails, and a closer needs the same list to let go
    /// of what those tabs kept ([`Places::forgetting`]). Working them out a second time
    /// is how a closer comes to close one set and forget another.
    ///
    /// For the three closers and nothing else -- see the type.
    pub(crate) fn close_tabs(&self, closing: impl Fn(&Tab) -> bool) -> Vec<DocId> {
        let (mut strip, mut docs) = (self.strip, self.docs);
        // Which trails go, read before anything is removed and in a scope of its own, so
        // no read guard is alive when the writes start. Only the documents among them: a
        // page has no trail, and the predicate goes to the strip whole.
        let going: Vec<DocId> = {
            let strip = strip.peek();
            strip
                .documents()
                .filter(|id| closing(&Tab::Document(*id)))
                .collect()
        };
        if strip.write().close(closing) {
            // One guard for however many tabs went: a write notifies whether or not it
            // changed anything.
            let mut docs = docs.write();
            for id in &going {
                docs.close(*id);
            }
        }
        going
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

    /// Let go of everything the tabs in `closed` kept -- their entries and their find
    /// bars -- and of every entry `also` answers false for, in all six maps and under one
    /// write each. What every closer ends with, and the whole of what it owes.
    ///
    /// The list and not a predicate, because it is the answer [`Open::close_tabs`] gave:
    /// what a closer forgets is what it closed, and the two cannot drift. `also` is the
    /// rest, which only a closing binary has -- the entries it takes off the trails of
    /// the tabs that stand.
    pub(crate) fn forgetting(self, closed: &[DocId], also: impl Fn(&Entry) -> bool) {
        let keep = |entry: &Entry| !closed.contains(&entry.0) && also(entry);
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
            Placing::Tab(tab) => !closed.contains(tab),
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

/// How wide the sidebar is: a [`Split`] like the document's and the Scratchpad's, and the
/// one of the three in [`Unit::Pixels`] -- the panel is a literal width, so what the
/// context holds after a drag is one too. The window's body is rebuilt whenever a project
/// arrives or goes, which is the unmount the number outlives.
#[derive(Clone, Copy)]
pub(crate) struct SidebarSplit(pub(crate) Split);

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
/// with its section shut. It follows that the section stays open or shut across the whole
/// of a tab's trail, which is a fact about the tab and not about any one place on it.
///
/// Never saved: it is a view of a tab, like a filter.
#[derive(Clone, Copy)]
pub(crate) struct Expanded(pub(crate) State<HashSet<DocId>>);

/// What the reader has said about each code pane's following pane -- the one its place is
/// not driven from: `true` where they brought it back, `false` where they put it away. A
/// place with nothing here has said nothing, and opens as its document says
/// ([`following`]).
///
/// A `bool` per place and not the set [`Expanded`] is, because there is no one default to
/// be absent from: a source-driven tab on a `Cargo.toml` opens with its assembly side
/// away and every other tab opens with both panes.
///
/// Keyed by [`Placing`] and never saved: the Scratchpad's pane has a following pane and
/// no [`DocId`], and one map with two kinds of key is one write for the gesture wherever
/// it is made ([`toggle_pane`]). A tab's key holds no `Arc<Object>` and is never handed
/// out twice, so a closed tab leaves a byte behind that no other tab can be given, and
/// this is a view of a place.
#[derive(Clone, Copy)]
pub(crate) struct Follows(pub(crate) State<HashMap<Placing, bool>>);

/// Where the reader chose to be able to come back to: the project's bookmarks, in their
/// saved shape and nothing more. Whether one is live is asked of [`Objects`] where it is
/// drawn, so a closed binary takes no bookmark with it and holds no `Arc` through one.
#[derive(Clone, Copy)]
pub(crate) struct Bookmarked(pub(crate) State<Bookmarks>);

/// The open project, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Proj(pub(crate) State<OpenProject>);

/// The file the open project is kept in, out of [`Proj`] and not read off it.
///
/// **A [`Memo`] because every box in the Project view writes `Proj` on every keystroke**,
/// and what reads this wants one field: it changes only as a project is opened, saved,
/// moved or closed. Made in [`roots`] beside the state it reads.
#[derive(Clone, Copy)]
pub(crate) struct ProjFile(pub(crate) Memo<Option<PathBuf>>);

/// The project's directory ([`OpenProject::workspace`]), a memo for [`ProjFile`]'s reason:
/// a keystroke in the Directory box changes it, and one in the other two does not.
#[derive(Clone, Copy)]
pub(crate) struct Workspace(pub(crate) Memo<Option<PathBuf>>);

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
/// (`store::moved`, through `note_moved`): the startup's, and a project switch's -- neither
/// of them anywhere near a component that could own this.
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
/// (`src/ui/session.rs`, `restore_ui`). Built where the three are made (`roots`) and handed
/// down instead.
#[derive(Clone, Copy)]
pub(crate) struct Arrangement {
    pub(crate) dock: State<DockArea>,
    pub(crate) sidebar: State<f32>,
    pub(crate) split: State<f32>,
}

/// What a code row's menu writes, and what the Source pane's four caret questions are
/// answered through, in one `Copy` bundle: where a door leads, where an answer lands, and
/// what a bookmark is added to.
///
/// **Consumed where the list renders and carried to the rows as data.** Reaching for a
/// context is a hook, and the handlers here are built by a render and run long after it,
/// so a row that consumed them itself paid a context walk per state per render for a
/// right-click that almost never comes.
///
/// The handles are the root's and are never replaced, so this **compares equal always**:
/// a row holding one is not re-rendered for it, where a bundle compared field by field
/// would trade the lookups for a render.
///
/// One bundle for both panes. The Source rows read the first three and the instruction
/// rows all five, and the two menus therefore cannot come to reach for one state two ways.
#[derive(Clone, Copy)]
pub(crate) struct RowStates {
    pub(crate) doors: Doors,
    /// Where a question about a line, a name or a function is answered.
    pub(crate) located: State<Located>,
    /// The dock the Locations panel is brought to the front of.
    pub(crate) dock: State<DockArea>,
    /// What "Bookmark symbol" adds to.
    pub(crate) bookmarked: State<Bookmarks>,
    /// The objects a bookmark is judged live against.
    pub(crate) objects: State<Vec<Arc<Object>>>,
}

impl PartialEq for RowStates {
    fn eq(&self, _: &RowStates) -> bool {
        true
    }
}

/// What a code row's menu and keys reach for, as the list drawing the rows sees them.
pub(crate) fn use_row_states() -> RowStates {
    RowStates {
        doors: use_doors(),
        located: use_consume::<Locations>().0,
        dock: use_consume::<SidebarDock>().0,
        bookmarked: use_consume::<Bookmarked>().0,
        objects: use_consume::<Objects>().0,
    }
}

/// [`RowStates`] for the lists outside the code panes: what a sidebar or panel row's
/// press and its menu reach for, in one `Copy` bundle.
///
/// The same rule and the same reason. It is consumed where the *list* renders -- on the
/// pane every one of them is drawn in ([`ListPane`]) -- and carried to the rows as data,
/// a handler being no place to call a hook. Each row reached for these itself before,
/// which was between four and eight context walks a render, every render, for a press
/// that comes once.
///
/// It **compares equal always**, the handles being the root's and never replaced, so
/// carrying it costs a row no render.
///
/// A union: no list's rows read all of it, and what they share is most of it. The
/// alternative is a bundle per list, which is six of these and six ways for two rows of
/// one panel to disagree about where a press leads.
///
/// **A press is handed it whole** ([`press_location`], [`symbol_keys`]) and never a set
/// built beside it, so a panel's rows and its Enter cannot be given two.
#[derive(Clone, Copy)]
pub(crate) struct ListStates {
    /// The list's own pick: what a row draws itself against, and what a press writes.
    pub(crate) picking: Picking,
    /// The door a press on a row goes through, which carries the places a chosen symbol
    /// is written to ([`Doors::places`]).
    pub(crate) doors: Doors,
    /// Whether Ctrl is held, which is whether a press opens a tab of its own.
    pub(crate) ctrl: State<bool>,
    /// The project's own states: what a bookmark is added to and judged live against, and
    /// what a binary is closed out of.
    pub(crate) project: ProjectStates,
    /// The two that go with [`ProjectStates`] wherever a project is switched, which is
    /// what a file row's "Open as project" does.
    pub(crate) rescued: State<Vec<PathBuf>>,
    pub(crate) unopened: State<Option<project::Failure>>,
}

impl PartialEq for ListStates {
    fn eq(&self, _: &ListStates) -> bool {
        true
    }
}

/// What a list's rows reach for, as the pane drawing them sees it.
pub(crate) fn use_list_states(panel: Panel) -> ListStates {
    ListStates {
        picking: use_picking(panel),
        doors: use_doors(),
        ctrl: use_consume::<Ctrl>().0,
        project: use_project_states(),
        rescued: use_consume::<Rescued>().0,
        unopened: use_consume::<Unopened>().0,
    }
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

/// Every object's text symbols flattened into one list, rebuilt only when the object list
/// changes and shared through context so the Symbols tab does not have to rebuild it. A
/// [`Shared`], so passing it around is a pointer and not a walk of a hundred thousand
/// symbols.
#[derive(Clone, Copy)]
pub(crate) struct Symbols(pub(crate) Memo<Shared<Symbol>>);

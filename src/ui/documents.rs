//! Opening a document, closing one, and moving between them.
//!
//! The active document is one of the open tabs or `None`, and that invariant is held by
//! [`activate`], [`close_tab`] and [`close_binary`] -- these three and nothing else, so a
//! new way to open something is a call to one of them rather than a fourth answer.
//!
//! [`open_binaries`] is `close_binary`'s opposite number and the one path by which
//! anything is ever added to `Objects`; [`navigate`] is the only thing that moves the
//! history's cursor, and it ends in `activate` because the history keeps an entry long
//! after its tab was closed. [`reopen_binary`] is a close followed by an open, in one
//! handler. The five `entry_*` helpers are what a `Document` is called wherever one is
//! named, which is why they sit beside the functions that open and close them rather than
//! beside the rows that draw them.

use super::*;

/// Why a document is becoming the active one, which is the whole of what decides whether
/// the history records it.
///
/// **The push follows the cause and not the state**, which is the rule Step 1e settled:
/// the history is where the reader *went*, and moving between places they already have
/// open is not going anywhere. Until then a single effect observed the active document
/// and pushed on every change, which could not tell the two apart — a strip click and a
/// symbol-list click look identical from there — so the answer has to come from the call
/// site, where it is known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Visit {
    /// The reader went somewhere: a sidebar row, a relocation link, the Source pane's
    /// companion header, or a restored session landing the app on a document. Recorded,
    /// unless the history's cursor is on it already.
    Went,
    /// The reader moved between places already open, or something moved them: a tab in
    /// the strip, the neighbour a close lands on, a tab the restore is merely reopening,
    /// and [`navigate`], which moves the cursor itself. Recorded nowhere.
    Moved,
}

/// Make `target` the active document, opening a tab for it if it has none, and record the
/// visit when there was one.
///
/// The one path by which [`Active`] ever changes, which is what makes "the active
/// document is the active tab" an invariant rather than a convention: the sidebar's
/// object and symbol rows, an assembly relocation link, the Source pane's companion
/// header, the history panel and the back/forward buttons (both through [`navigate`]) and
/// the startup restore all come through here, so none of them has to know that tabs
/// exist. `None` opens nothing and is how the content area goes back to its placeholder;
/// it is never a visit, having nowhere to be a visit to.
///
/// **One function for both kinds of tab**, where until Step 1 there were two — `activate`
/// for the content area's functions and `open_file` for the Source pane's files, each
/// holding its own strip's invariant. The strips are one, so the rule is one, and opening
/// a file and opening a function differ in nothing but the value handed over.
///
/// **And one function for the history too**, where until Step 1e that was an effect
/// observing the active document from the root. The effect was the wrong shape rather
/// than merely in the wrong place: it saw *that* the document had changed and could not
/// see *why*, so a click on a tab in the strip was indistinguishable from a click on a
/// symbol in the list. `visit` is that missing half, and it is why the recording moved to
/// the one place every change already goes through rather than to each caller.
///
/// `History::would_push` is still asked, and it is what keeps [`navigate`] honest without
/// a "we are navigating" flag: back and forward land the cursor on the entry they moved
/// to, so a push would dedup away even if one were attempted.
///
/// Re-focusing a tab that is already open writes nothing: `State::write` notifies its
/// subscribers whether or not the value changes, so both the list and the active document
/// are asked before they are touched.
pub(crate) fn activate(
    open: Open,
    mut history: State<History>,
    target: Option<Document>,
    visit: Visit,
) {
    let Open { mut dock, mut docs } = open;

    let Some(target) = target else {
        // Nothing to show. With views tabbed in the same panel there is usually still a
        // tab to be on, and falling to the first of them is what keeps a panel that has
        // tabs from having none of them active; an empty panel draws its own ground.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            let first = panel.tabs.first().copied();
            panel.active_tab_id = first;
        }
        return;
    };

    // The copy that is *in the table* where there is one, so the identity a position is
    // keyed by does not change when the same file is reached again through a different
    // symbol's `LineInfo`: two of them naming one path hold two `Arc<str>`s of it. The
    // read is bound and dropped before the write below, never held across it.
    let existing = docs.peek().id_of(&target);
    let id = match existing {
        Some(id) => id,
        None => docs.write().open(target.clone()),
    };
    let target = docs.peek().get(id).cloned();

    // Asked before it is written, which is what keeps re-focusing the tab that is
    // already on top from waking every pane that draws a document: `State::write`
    // notifies its subscribers whether or not the value it hands over changes. This is
    // `set_if_modified`'s job, done by hand because the value is a tree.
    let tab = Tab::Document(id);
    let settled = dock
        .peek()
        .document_panel()
        .is_some_and(|panel| panel.active_tab_id == Some(tab) && panel.tabs.contains(&tab));
    if !settled {
        dock.write().show_document(tab);
    }

    // `write()` notifies its subscribers before it hands the value over, whether or not
    // anything changes, so ask first: a push that would dedup away must not wake the
    // history panel. The guard from `peek` is gone before the write is reached.
    let Some(target) = target.filter(|_| visit == Visit::Went) else {
        return;
    };
    if history.peek().would_push(&target) {
        history.write().push(target);
    }
}

/// Close the tab showing `entry`, moving to a neighbouring one when it was the tab on
/// screen and to the placeholder when it was the last one open.
///
/// Landing on the neighbour is a [`Visit::Moved`] and records nothing: it is a place the
/// reader already had open, which is exactly what the strip is, and closing a tab is not
/// a way of visiting the one beside it.
///
/// Where the tab was left goes with it, **both sides of it**. A closed tab is not a tab,
/// so a position kept for one is both a lie — reopening it from the sidebar is a fresh
/// tab, which starts at the top — and a leak, since a [`Document::Assembly`] holds the
/// `Arc<Object>` it points into.
pub(crate) fn close_tab(
    open: Open,
    history: State<History>,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    entry: &Document,
) {
    let Open { mut dock, mut docs } = open;
    let Some(id) = docs.peek().id_of(entry) else {
        return;
    };
    let tab = Tab::Document(id);

    // Worked out before anything is removed, which is what [`tabs::landing`] wants, and
    // in a scope of its own so no read guard is alive when the writes below start.
    let (was_showing, next) = {
        let dock = dock.peek();
        let Some(panel) = dock.document_panel() else {
            return;
        };
        (
            panel.active_tab_id == Some(tab),
            tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), |open| {
                *open == tab
            }),
        )
    };

    {
        // Removed by hand rather than through freya's `remove_tab_except`, which sets the
        // panel's active tab to `tabs.first()` when it takes the active one out. Landing
        // on the *neighbour* is a rule of this app, older than the list it is written
        // against, and letting the removal choose would quietly replace it.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel.tabs.retain(|open| *open != tab);
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    docs.write().close(id);
    asm_at.write().forget(entry);
    src_at.write().forget(entry);

    // A document landed on goes through `activate`, even though it is by construction
    // already open: it is a change of active document and there is one way to make one.
    // A *view* landed on is not a document at all, and the write above has already put
    // the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
    }
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The third of the functions that hold the app's invariants, beside [`activate`] and
/// [`close_tab`], and the only one that ever *removes* an object -- until 8c the app could
/// open a binary and never let go of one. The unit is the **file** and never the object:
/// an archive member is not something the reader opened, closing one member of 196 would
/// leave a file half-present with no row able to say so, and the saved `binaries` are a
/// list of paths, so half a file is not a thing the session could even record. One path
/// opened twice is therefore also one close: the objects list holds both copies,
/// `Object::path` cannot tell them apart, and neither could the file it would be written
/// to.
///
/// What each of the things pointing at those objects does with the news:
///
/// - The **assembly-driven tabs** whose document was in the file are closed, all of them
///   at once ([`Tabs::close_all`]), which is what closing the one tab the reader was on
///   would have done had its neighbours not gone with it. **Source-driven tabs survive**
///   ([`Document::in_file`] answers false for one): a file chip outlives the binary that
///   led the reader to it, because the text stands on its own and nothing records which
///   object opened it. That was the Source pane's separate strip being left alone; it is
///   now a rule of the one strip.
/// - The **active document** follows the tabs rather than degrading the way a restore's
///   does. Degrading has nothing to fall back *to* here: a file takes its objects and
///   their symbols together, so `resolve_or_degrade`'s symbol-to-object step would land on
///   an object that is going away in the same breath. What is left is the tab rule -- the
///   neighbouring tab, or nothing at all when the close emptied the strip -- and that is
///   also the only answer that keeps "the active document is the active tab" true, since
///   the placeholder with tabs still open would be a fourth state.
/// - The **history** drops its entries rather than degrading them ([`History::retaining`]),
///   which is the same walk and the same reasoning as a restore whose binaries have
///   changed: a list of places the reader cannot get back to is worse than a short list.
///   A visited source file is kept, by the same rule its tab is. It is *read* here too,
///   since the tab this lands on goes through `activate`.
/// - The **viewing positions** of the tabs that closed go with them, both sides of each
///   ([`Positions`]), which is not tidiness: every entry is keyed by a [`Document`], which
///   for an assembly-driven one holds the `Arc<Object>` it points into, so one left behind
///   would hold the file's bytes -- 331 MB of them, for `viewer-sample` -- for as long as
///   the app ran.
/// - **The file's load**, if it is still being read, is cancelled ([`Loads::cancel`]) —
///   which is not tidiness either: without it the objects still coming out of the worker
///   would arrive after the close and put the file back, one member at a time. The unit
///   there is the path for the same reason it is here, so one file opened twice closes
///   once and stops loading once.
/// - **The saved `binaries`** need nothing here at all. They are derived from the objects
///   by `project::binaries`, so removing them removes the path, and `project::record` sees
///   a *binaries* change and writes it to disk at once rather than marking it pending --
///   which is what `Goals.md` asks of a change the user made, and the first thing since
///   opening a file to take that path.
///
/// All the writes happen here, in one event handler, before anything can render: the
/// save observer therefore wakes once, with all of it settled, so the file that reaches
/// the disk never names a binary the app has already let go of.
pub(crate) fn close_binary(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    open: Open,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    mut history: State<History>,
    path: &Path,
) {
    let Open { mut dock, mut docs } = open;
    // Every guard below is taken out of its own statement or its own scope, so none of
    // them is still alive when the next write -- or `activate` at the end -- is reached.
    let showing = open.active();

    // Which tabs go, and what is left to be on, both worked out before anything is
    // removed. `closing` is asked of a *tab*: a view is never in a file, so the same walk
    // that closes a binary's functions leaves Project, Settings and the Scratchpad alone
    // for the same reason it leaves a source-driven tab alone.
    let (closing, next) = {
        let dock_ref = dock.peek();
        let docs_ref = docs.peek();
        let Some(panel) = dock_ref.document_panel() else {
            return;
        };
        let in_file = |tab: &Tab| match tab {
            Tab::Document(id) => docs_ref
                .get(*id)
                .is_some_and(|document| document.in_file(path)),
            Tab::View(_) => false,
        };
        let closing: Vec<Tab> = panel.tabs.iter().copied().filter(in_file).collect();
        let next = tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), in_file);
        (closing, next)
    };

    let was_showing = showing
        .as_ref()
        .is_some_and(|showing| showing.in_file(path));
    {
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel.tabs.retain(|tab| !closing.contains(tab));
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    {
        let mut docs = docs.write();
        for tab in &closing {
            if let Tab::Document(id) = tab {
                docs.close(*id);
            }
        }
    }

    // The same walk over the same rule, so the positions cannot outlive the tabs they
    // belong to.
    asm_at.write().forgetting(|tab| !tab.in_file(path));
    src_at.write().forgetting(|tab| !tab.in_file(path));

    let remaining = history.peek().retaining(|entry| !entry.in_file(path));
    history.set(remaining);

    objects.write().retain(|object| object.path != path);
    // Whatever is still being parsed out of this file is for a file the app has just let
    // go of. Dropping the entry is what makes the next batch of objects out of it be
    // dropped and the worker itself stop; see `take_load`.
    loading.write().cancel(path);

    // Through `activate` like every other change of active document, even though the tab
    // it lands on is by construction already open — which is what makes it a
    // [`Visit::Moved`], exactly as closing one tab by hand is. A view landed on is not a
    // document and the write above has already put the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
    }
}

/// The menu a file row opens on a right-click: the one thing that can be done to a file
/// once it is open.
///
/// Built per press rather than once, because it closes over the path of the row it was
/// opened on -- freya's `ContextMenu` takes a whole `Menu` and places it at the pointer
/// (`freya-components/src/context_menu.rs`), so there is nothing to keep. The states come
/// in as an argument for the reason every row's do: this is called from an event handler,
/// where no hook may run.
pub(crate) fn close_menu(states: ProjectStates, path: PathBuf) -> Menu {
    let ProjectStates {
        objects,
        loading,
        open,
        asm_at,
        src_at,
        history,
        ..
    } = states;

    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| close_binary(objects, loading, open, asm_at, src_at, history, &path))
            // "file" and not "object", because the row a reader right-clicks may be one
            // object of one file or the archive above 196 of them, and the same word has
            // to be true of both.
            .child("Close file"),
    )
}

/// Read and parse `paths`, putting each object into the list **as it is parsed**.
///
/// The opposite number of [`close_binary`], and the one path by which anything is ever
/// added to `objects`: the toolbar's Open, a session restore and a scratchpad's rebuild
/// all come through here, so they cannot differ about what opening a file means.
///
/// **A worker thread and a channel**, which is the shape `use_analysis` and the
/// scratchpad's worker already have and for the same reason: reading and parsing is
/// seconds of CPU on a large file and freya's executor is the UI thread. What is new is
/// that the answers come back one at a time (`analysis`'s `open_files_streaming`) rather
/// than as one `Vec` at the end — which is the whole of "explore while a binary is
/// processed". On `libanalysis-sample.rlib` that is 196 members arriving over the parse
/// instead of after it; on the 331 MB `viewer-sample`, which is one object, it is the row
/// in [`Loads`] appearing at once where the sidebar used to sit empty for the duration.
///
/// The channel is **unbounded and drained in batches**. Unbounded because backpressure
/// would be exactly wrong here — the worker is the thing that should run flat out, and the
/// objects it hands over are `Arc`s of bytes that already exist — and batched because a
/// write per member is a re-render per member, which for an archive whose members parse in
/// a millisecond is a hundred renders nobody sees. Draining what has already arrived
/// collapses each burst into one write.
pub(crate) async fn open_binaries(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    paths: Vec<PathBuf>,
) {
    // Registered before a byte is read, so the rows are on screen for the whole of the
    // wait rather than from whenever the first answer lands.
    let id = {
        let mut loading = loading;
        loading.write().begin(&paths)
    };

    let (sender, events) = async_channel::unbounded::<Progress>();
    std::thread::spawn(move || {
        open_files_streaming(paths, |progress| match sender.send_blocking(progress) {
            Ok(()) => ControlFlow::Continue(()),
            // The receiver has gone, which is `take_load` deciding that nothing more from
            // this load is wanted. Stopping here is what keeps a closed 331 MB file from
            // being parsed to the end into a value that will be dropped.
            Err(_) => ControlFlow::Break(()),
        });
    });

    take_load(objects, loading, id, events).await;
}

/// Take one load's answers until it has nothing left to say.
///
/// Split from [`open_binaries`] because it is the half with the rules in it, and because
/// a test can feed it by hand: what has to be asserted is what happens to an answer that
/// arrives *after* the reader has closed the file or left the project, which is a race
/// against a real worker and a fact against a channel the test writes into.
///
/// **An object nobody asked for any more is dropped rather than prevented.** That is
/// `use_analysis`'s rule in a second place, and it has to be: the worker is already
/// parsing when the file is closed, so the answer exists whatever the app does. It is
/// checked against [`Loads::holds`] — the load *and* the path, not the path alone, since a
/// file closed and reopened while the first parse ran is two loads and only the second
/// one's objects belong on screen.
///
/// Returning is what stops the worker: it drops the receiver, the next `send_blocking`
/// fails, and the walk breaks where it stands.
pub(crate) async fn take_load(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    id: LoadId,
    events: async_channel::Receiver<Progress>,
) {
    while let Ok(first) = events.recv().await {
        // Whatever else has arrived while the UI thread was elsewhere, taken in the same
        // pass so a burst of members costs one write.
        let mut batch = vec![first];
        while let Ok(more) = events.try_recv() {
            batch.push(more);
        }

        // Both lists are worked out under one read guard and the guard is gone before
        // anything writes -- the `peek`/`write` rule, and the reason this is not a single
        // loop that pushes and writes as it goes.
        let (parsed, finished) = {
            let held = loading.peek();
            let mut parsed: Vec<Arc<Object>> = Vec::new();
            let mut finished: Vec<PathBuf> = Vec::new();
            for progress in batch {
                match progress {
                    Progress::Parsed(object) if held.holds(id, &object.path) => parsed.push(object),
                    // An object for a file this load no longer holds: the reader closed
                    // it, or left the project, while it was being parsed.
                    Progress::Parsed(_) => {}
                    Progress::Finished(path) => finished.push(path),
                }
            }
            (parsed, finished)
        };

        if !parsed.is_empty() {
            objects.write().extend(parsed);
        }
        if !finished.is_empty() {
            let mut held = loading.write();
            for path in finished {
                held.finished(id, &path);
            }
        }

        // Nothing left that this load could still be asked about, either because it is
        // done or because everything it was reading has been closed. Returning drops the
        // receiver, which is the only thing that tells the worker.
        if !loading.peek().active(id) {
            return;
        }
    }
}

/// What a document is called where it is named in a list: the same demangled name the
/// symbol list shows for a function, the object's name for an object, and the file's own
/// last path component for a source file. The history rows and the tabs both draw this,
/// which is what makes a place read the same wherever it is named.
///
/// A file's *name* and not its path, because the strip is narrow and every one of these
/// paths shares most of its directory with the others. The whole of it is in the tooltip
/// ([`entry_tooltip`]), which is what the Source pane's header used to say.
pub(crate) fn entry_text(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Object(object)) => object.name.clone(),
        Document::Assembly(Selection::Symbol(symbol)) => symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone(),
        Document::Source(file) => file_name(file),
    }
}

/// What hovering a document's tab or row says. The whole path for a file, where the row
/// itself has only room for its name; everything else says what it draws, elided or not.
pub(crate) fn entry_tooltip(entry: &Document) -> String {
    match entry {
        Document::Source(file) => file.to_string(),
        entry => entry_text(entry),
    }
}

/// Which kind of tab this is, as the one glyph that tells the two apart.
///
/// The same two glyphs the dock's own Assembly and Source views wear (`Tab::icon`), and
/// deliberately so: the tab says which pane is in charge of it, so it should be named by
/// the pane it is about.
pub(crate) fn entry_icon(entry: &Document) -> Element {
    let (name, svg) = match entry {
        Document::Assembly(_) => ("binary", lucide::binary()),
        Document::Source(_) => ("file-code", lucide::file_code()),
    };

    let side = icon_size();
    SvgViewer::new((name, svg))
        .width(Size::px(side))
        .height(Size::px(side))
        .color(palette().icon_fg)
        .show_loader(false)
        .into_element()
}

/// The identity of what a document points at, for keying the row or tab that names it.
///
/// A tab keys by this alone, its place in the strip being stable. A history row pairs it
/// with the entry's index, because a row's identity is its place in the list: the entry at
/// an index changes when a push truncates the forward entries, and again when a push
/// bumps an existing entry to the newest position and shifts the ones behind it down. The
/// pointer alone would be identity enough now that no two entries are equal, but then a
/// bumped row would keep the hover state of the one that used to sit where it now does;
/// with the index in the key the moved rows are simply rebuilt, which for a list this
/// short costs nothing.
///
/// The variant is part of the key and not only the pointer, since a file is keyed by its
/// text: a hash of an address and a hash of a path could otherwise collide into one key
/// for two tabs of different kinds.
#[derive(Hash)]
pub(crate) enum EntryKey<'a> {
    Object(usize),
    Symbol(usize),
    Source(&'a str),
}

pub(crate) fn entry_key(entry: &Document) -> EntryKey<'_> {
    match entry {
        Document::Assembly(Selection::Object(object)) => {
            EntryKey::Object(Arc::as_ptr(object).addr())
        }
        Document::Assembly(Selection::Symbol(symbol)) => {
            EntryKey::Symbol(Arc::as_ptr(&symbol.data).addr())
        }
        Document::Source(file) => EntryKey::Source(file),
    }
}

/// Open the binary at `path` in place of whatever the app already had from it.
///
/// **Not a sixth function holding the tab invariants**: it is [`close_binary`] followed by
/// exactly what the toolbar's Open button does, in that order and in one handler.
///
/// Replacing rather than accumulating is the only answer available, and the reason is the
/// app's own identity rule: a binary is a **path**, `close_binary` closes by path, and
/// `project::binaries` derives the saved list from the objects by path -- so two
/// generations of one file cannot both be in the objects list without every one of those
/// answering for which is which. A rebuild writes the same path with different bytes, so
/// what was open is a listing of instructions that no longer exist.
///
/// What it costs the reader, honestly: `close_binary` takes the chips for that file's
/// functions, their viewing positions and the history entries into them, so a rebuild
/// leaves the content strip empty of the scratchpad and the reader clicks their function
/// again. Keeping them would mean re-resolving each tab by name against the new objects,
/// which is exactly what a session restore does for a rebuilt binary (`project.rs`'s
/// `Rebuilt`) and is that machinery pointed at a live state rather than at a file.
///
/// The close happens **first** and the parse after it, which is the one thing streaming
/// turned around: objects arrive one at a time, so there is no moment at which the whole
/// answer is in hand to be swapped in under a single handler. What that costs is a beat in
/// which the project has let go of the file -- `record` writes `project.toml` without it
/// and again with it once the first object lands -- and what it buys is that the two
/// generations of one path can never be in the objects list together, which is the rule
/// everything else here rests on. The row does not blink either way: `close_binary` takes
/// the objects and `open_binaries` puts the file straight back as one being read.
pub(crate) fn reopen_binary(states: ProjectStates, path: PathBuf) {
    // Unconditionally, and before the new objects go in: whether or not the new build
    // parses, the objects the app is holding describe bytes that are no longer there.
    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.history,
        &path,
    );

    spawn(async move {
        open_binaries(states.objects, states.loading, vec![path]).await;
    });
}

/// A step through the navigation history.
///
/// Back and forward are what the mouse buttons ask for; `To` is the history panel
/// clicking an entry. All three are a cursor move over a `History` method, so that
/// everything which moves the cursor keeps going through `navigate`.
#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Back,
    Forward,
    /// Straight to the entry at this index, the one `History::recent` handed the row.
    To(usize),
}

impl Nav {
    /// Whether there is an entry to step to.
    fn possible(self, history: &History) -> bool {
        match self {
            Self::Back => history.can_back(),
            Self::Forward => history.can_forward(),
            Self::To(index) => history.can_jump(index),
        }
    }

    /// Move the cursor and hand back the entry it landed on.
    fn step(self, history: &mut History) -> Option<Document> {
        match self {
            Self::Back => history.back(),
            Self::Forward => history.forward(),
            Self::To(index) => history.jump(index),
        }
    }
}

/// Move the selection one entry back or forward through the history.
///
/// The one place navigation happens, so the input handler below and the history panel
/// share the same two steps: move the cursor, then make the entry it landed on the active
/// tab. Nothing is pushed -- it is a [`Visit::Moved`], and `would_push` would be false for
/// it in any case, that entry being exactly what the cursor now sits on.
///
/// It goes through [`activate`] rather than setting the selection itself because the
/// history and the open tabs are two different lists: the history is everywhere the reader
/// has been and keeps entries long after their tab was closed, so going back to one has to
/// be able to open a tab for it again.
pub(crate) fn navigate(open: Open, mut history: State<History>, nav: Nav) {
    // Ask before writing. `State::write` notifies its subscribers whether or not the
    // value it hands over changes, so back at the oldest entry -- or forward at the
    // newest -- must not reach for it at all: a no-op has to leave the history alone,
    // leave the document on screen alone, and wake nothing.
    if !nav.possible(&history.peek()) {
        return;
    }

    // The guard is released at the end of this statement, before the selection is set
    // and `activate` peeks the history back.
    let entry = nav.step(&mut history.write());
    if entry.is_some() {
        activate(open, history, entry, Visit::Moved);
    }
}

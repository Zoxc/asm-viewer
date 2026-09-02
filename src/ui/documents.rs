//! Opening a document, closing one, and moving between them.
//!
//! The invariant "the active document is one of the open tabs, or `None`" is held by
//! [`activate`], [`close_tab`] and [`close_binary`] and by nothing else, so every path
//! that opens a document -- [`navigate`] included -- goes through [`activate`].

use super::*;

/// Why a document is becoming the active one, which decides whether the history records
/// it. The cause is known at the call site and cannot be read off the state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Visit {
    /// The reader went somewhere: a sidebar row, a relocation link, the Source pane's
    /// companion header, or a restored session landing the app on a document. Recorded,
    /// unless the history's cursor is on it already.
    Went,
    /// The reader moved between places already open: a tab in the strip, the neighbour a
    /// close lands on, or [`navigate`], which moves the cursor itself. Recorded nowhere.
    Moved,
}

/// Make `target` the active document, opening a tab for it if it has none, and record the
/// visit when there was one.
///
/// The one path by which [`Active`] ever changes. `None` opens nothing and is how the
/// content area goes back to its placeholder; it is never a visit.
pub(crate) fn activate(
    open: Open,
    mut history: State<History>,
    target: Option<Document>,
    visit: Visit,
) {
    let Open { mut dock, mut docs } = open;

    let Some(target) = target else {
        // Falling to the first tab keeps a panel that has tabs from having none of them
        // active; an empty panel draws its own ground.
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
    // read is bound to a `let` and dropped before the write below, never held across it.
    let existing = docs.peek().id_of(&target);
    let id = match existing {
        Some(id) => id,
        None => docs.write().open(target.clone()),
    };
    let target = docs.peek().get(id).cloned();

    // Asked before it is written: `State::write` notifies whether or not the value
    // changes, so re-focusing the tab already on top must not reach for it.
    let tab = Tab::Document(id);
    let settled = dock
        .peek()
        .document_panel()
        .is_some_and(|panel| panel.active_tab_id == Some(tab) && panel.tabs.contains(&tab));
    if !settled {
        dock.write().show_document(tab);
    }

    // Asked first for the same reason: a push that would dedup away must not wake the
    // history panel.
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
/// Both of the tab's kept positions go with it: a [`Document::Assembly`] key holds the
/// `Arc<Object>` it points into, so one left behind holds the file's bytes for the life of
/// the app. The line it was driven from goes with it too, for consistency and **not** for
/// that reason: a [`Document::Source`] key holds no object, so it holds nothing up.
pub(crate) fn close_tab(
    open: Open,
    history: State<History>,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    mut driven: State<Driven>,
    entry: &Document,
) {
    let Open { mut dock, mut docs } = open;
    let Some(id) = docs.peek().id_of(entry) else {
        return;
    };
    let tab = Tab::Document(id);

    // Worked out before anything is removed, which is what `tabs::landing` wants, and in
    // a scope of its own so no read guard is alive when the writes below start.
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
        // Removed by hand and never through freya's `remove_tab_except`, which sets the
        // panel's active tab to `tabs.first()` when it takes the active one out. Landing
        // on the *neighbour* (`tabs::landing`) is this app's rule.
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
    driven.write().forget(entry);

    // A document landed on goes through `activate`, even though it is by construction
    // already open. A *view* landed on is not a document, and the write above has already
    // put the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
    }
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The third of the functions holding the tab invariants, beside [`activate`] and
/// [`close_tab`]. The unit is the **file** and never the object, so one path opened twice
/// closes once. Assembly-driven tabs in the file are closed and their positions forgotten;
/// source-driven tabs survive; the history drops those entries rather than degrading them;
/// a load still running is cancelled, or its objects would put the file back one member at
/// a time.
///
/// All the writes happen in this one handler, so the save observer wakes once on a settled
/// state and never writes a binary the app has already let go of.
pub(crate) fn close_binary(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    open: Open,
    mut asm_at: State<Positions<Document>>,
    mut src_at: State<Positions<Document>>,
    mut driven: State<Driven>,
    mut history: State<History>,
    path: &Path,
) {
    let Open { mut dock, mut docs } = open;
    // Every guard below is taken out of its own statement or its own scope, so none of
    // them is still alive when the next write -- or `activate` at the end -- is reached.
    let showing = open.active();

    // Which tabs go, and what is left to be on, both worked out before anything is
    // removed. A view is never in a file, so this walk leaves them alone.
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

    // The positions cannot outlive the tabs they belong to.
    asm_at.write().forgetting(|tab| !tab.in_file(path));
    src_at.write().forgetting(|tab| !tab.in_file(path));
    // A source-driven tab stands, but a symbol it chose out of this file is let go: the
    // line beside the choice is what survives a close, and the next ask answers out of
    // what is left.
    driven.write().release(path);

    let remaining = history.peek().retaining(|entry| !entry.in_file(path));
    history.set(remaining);

    objects.write().retain(|object| object.path != path);
    // Dropping the entry is what makes the next batch of objects out of this file be
    // dropped and the worker itself stop; see `take_load`.
    loading.write().cancel(path);

    // Through `activate` like every other change of active document. A view landed on is
    // not a document and the write above has already put the panel on it.
    if let (true, Some(Tab::Document(next))) = (was_showing, next) {
        let document = docs.peek().get(next).cloned();
        activate(open, history, document, Visit::Moved);
    }
}

/// Open `target` on `at`: activate it, and pin the line in it with both panes owed the
/// scroll -- at once when the document is already on top, since `activate` then changes
/// nothing and no effect would run, and otherwise as a [`Landing`] for the change of
/// document to turn into the pin. A visit either way, as any opening from a list is.
pub(crate) fn land(
    open: Open,
    history: State<History>,
    mut pinned: State<Option<Pin>>,
    mut landing: State<Option<Landing>>,
    target: Document,
    at: LinePos,
) {
    if open.active().as_ref() == Some(&target) {
        pinned.set(Some(Pin {
            at,
            reveal: Owed::BOTH,
            landed: true,
        }));
        return;
    }

    landing.set(Some(Landing {
        tab: target.clone(),
        at,
    }));
    activate(open, history, Some(target), Visit::Went);
}

/// The menu a file row opens on a right-click.
///
/// Built per press, since it closes over the row's path. The states come in as an argument
/// because this is called from an event handler, where no hook may run.
pub(crate) fn close_menu(states: ProjectStates, path: PathBuf) -> Menu {
    let ProjectStates {
        objects,
        loading,
        open,
        asm_at,
        src_at,
        driven,
        history,
        ..
    } = states;

    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                close_binary(
                    objects, loading, open, asm_at, src_at, driven, history, &path,
                )
            })
            // "file" and not "object": the row may be one object of a file or the archive
            // above 196 of them, and the word has to be true of both.
            .child("Close file"),
    )
}

/// Read and parse `paths` on a worker thread, putting each object into the list as it is
/// parsed.
///
/// The one path by which anything is ever added to `objects`. The channel is unbounded --
/// the worker should run flat out -- and drained in batches, a write per member being a
/// re-render per member.
pub(crate) async fn open_binaries(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    paths: Vec<PathBuf>,
) {
    // Registered before a byte is read, so the rows are on screen for the whole wait.
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
/// An object nobody asked for any more is dropped rather than prevented: the worker is
/// already parsing when the file is closed. It is checked against `Loads::holds` -- the
/// load *and* the path, since a file closed and reopened mid-parse is two loads.
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
        // Whatever else has arrived, taken in the same pass so a burst costs one write.
        let mut batch = vec![first];
        while let Ok(more) = events.try_recv() {
            batch.push(more);
        }

        // Both lists are worked out under one read guard and the guard is gone before
        // anything writes.
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

        // Nothing left that this load could be asked about: it is done, or everything it
        // was reading has been closed. Returning drops the receiver, which is what tells
        // the worker.
        if !loading.peek().active(id) {
            return;
        }
    }
}

/// What a document is called where it is named in a list. A source file's *name* only; the
/// whole path is in [`entry_tooltip`].
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

/// What hovering a document's tab or row says: the whole path for a file, what it draws
/// for everything else.
pub(crate) fn entry_tooltip(entry: &Document) -> String {
    match entry {
        Document::Source(file) => file.to_string(),
        entry => entry_text(entry),
    }
}

/// Which kind of tab this is, as the one glyph that tells the two apart.
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

/// The identity of what a document points at, for keying the row or tab that names it. A
/// history row pairs it with the entry's index, since a push can shift an entry to another
/// place in the list. The variant is part of the key so a pointer and a path cannot hash
/// into one key for two tabs of different kinds.
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

/// Open the binary at `path` in place of whatever the app already had from it:
/// [`close_binary`] and then what the toolbar's Open does, in one handler.
///
/// A binary is a **path** throughout the app, so two generations of one file must not be
/// in the objects list together. The close is therefore first and unconditional -- whether
/// or not the new build parses, the objects in hand describe bytes that are gone -- and it
/// takes that file's tabs, their positions and its history entries with it.
pub(crate) fn reopen_binary(states: ProjectStates, path: PathBuf) {
    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.driven,
        states.history,
        &path,
    );

    spawn(async move {
        open_binaries(states.objects, states.loading, vec![path]).await;
    });
}

/// A step through the navigation history: the mouse's back and forward buttons, and the
/// history panel clicking an entry.
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

/// Move the cursor one entry back or forward through the history, and make the entry it
/// landed on the active document.
///
/// The one place the cursor moves. It goes through [`activate`] because the history keeps
/// entries long after their tab was closed, so going back to one has to reopen a tab.
pub(crate) fn navigate(open: Open, mut history: State<History>, nav: Nav) {
    // Asked before writing: `State::write` notifies whether or not the value changes, and
    // a no-op step has to wake nothing.
    if !nav.possible(&history.peek()) {
        return;
    }

    // The guard is released at the end of this statement, before `activate` peeks the
    // history back.
    let entry = nav.step(&mut history.write());
    if entry.is_some() {
        activate(open, history, entry, Visit::Moved);
    }
}

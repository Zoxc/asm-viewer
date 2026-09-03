//! Opening a document, closing a tab, and moving between them.
//!
//! Two invariants are held here and nowhere else: "the active tab is one of the open
//! tabs, or `None`", and "a tab and its trail are made together and closed together".
//! [`open_document`], [`raise`], [`navigate`], [`close_tab`], [`close_others`] and
//! [`close_binary`] are the six functions that change what is open or what a tab shows,
//! and every path that opens a document -- [`land`] included -- goes through
//! [`open_document`].

use super::*;

/// How a document is reached: where it opens, which the click that opened it says and
/// nothing about the state can.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Reach {
    /// From inside the tab on screen -- a relocation link, the companion header over the
    /// Source pane. Pushed onto that tab's trail in place of what it showed, so the place
    /// left is one Back away. Promotes a temporal tab: the reader is reading in it. With
    /// no document tab on screen there is nothing to replace, and this is [`Reach::NewTab`].
    InPlace,
    /// In a tab of its own that stays, beside the tab on screen: Ctrl+click on anything,
    /// or a menu item. A tab already showing the place is raised instead, and promoted
    /// when it was the temporal one -- what was asked for is a tab of this place that
    /// stays, and it has one.
    NewTab,
    /// From outside the panes -- a row in a sidebar list. Into the one temporal tab,
    /// pushed onto its trail so that Back inside it walks the rows clicked, or a new
    /// temporal tab beside the tab on screen while there is none. A tab already showing
    /// the place is raised instead, the temporal one included, which promotes nothing.
    Preview,
}

/// Open `target` the way `reach` says, make the tab it lands in the active one, and
/// record the visit. The one path by which a document is ever opened.
///
/// The tab the document landed in, or `None` when there is no document panel to land
/// in. Every read of the states is bound before any write to them.
pub(crate) fn open_document(
    open: Open,
    mut visits: State<Visits>,
    target: Document,
    reach: Reach,
) -> Option<DocId> {
    let Open { mut dock, mut docs } = open;

    // Recorded whatever else happens, and only when it changes the record: `State::write`
    // notifies whether or not the value changes, and re-opening the place at the top must
    // not wake the History panel.
    if visits.peek().would_record(&target) {
        visits.write().record(target.clone());
    }

    // The tab on screen and the tab showing the target, the tab on screen preferred when
    // it is one of several: two tabs can show one place.
    let active = open.active_tab();
    let showing = match &active {
        Some((id, current)) if *current == target => Some(*id),
        _ => docs.peek().showing(&target),
    };
    let temporal = docs.peek().temporal();

    match reach {
        Reach::InPlace if active.is_some() => {
            let (id, current) = active?;
            // Already there: nothing to push, and a write would wake every header.
            if current != target {
                let mut docs = docs.write();
                if let Some(trail) = docs.trail_mut(id) {
                    trail.push(target);
                }
                docs.promote(id);
            }
            Some(id)
        }
        Reach::InPlace | Reach::NewTab => {
            if let Some(id) = showing {
                if temporal == Some(id) {
                    docs.write().promote(id);
                }
                raise(open, id);
                return Some(id);
            }
            let id = docs.write().open(target);
            dock.write().show_document(Tab::Document(id));
            Some(id)
        }
        Reach::Preview => {
            if let Some(id) = showing {
                raise(open, id);
                return Some(id);
            }
            match temporal {
                Some(id) => {
                    {
                        let mut docs = docs.write();
                        if let Some(trail) = docs.trail_mut(id) {
                            trail.push(target);
                        }
                    }
                    raise(open, id);
                    Some(id)
                }
                None => {
                    let id = {
                        let mut docs = docs.write();
                        let id = docs.open(target);
                        docs.mark_temporal(id);
                        id
                    };
                    dock.write().show_document(Tab::Document(id));
                    Some(id)
                }
            }
        }
    }
}

/// Make the open tab `id` the active one. The reader moved between places already
/// open -- a tab in the strip's menu, the neighbour a close lands on, a restored session
/// -- so nothing is recorded and no trail moves. A no-op for a closed id.
pub(crate) fn raise(open: Open, id: DocId) {
    let mut dock = open.dock;
    let tab = Tab::Document(id);
    // Asked before it is written: `State::write` notifies whether or not the value
    // changes, so re-raising the tab already on top must not reach for it.
    let (held, settled) = {
        let dock = dock.peek();
        match dock.document_panel() {
            Some(panel) => (
                panel.tabs.contains(&tab),
                panel.active_tab_id == Some(tab) && panel.tabs.contains(&tab),
            ),
            None => (false, false),
        }
    };
    if held && !settled {
        dock.write().show_document(tab);
    }
}

/// Close the tab `id`, moving to a neighbouring one when it was the tab on screen and
/// to the placeholder when it was the last one open.
///
/// Everything kept by the tab's entries goes with it: an [`Entry`] key holds the
/// `Arc<Object>` its document points into, so one left behind holds the file's bytes for
/// the life of the app. The lines its entries were driven from go with it too, for
/// consistency and **not** for that reason: a [`Document::Source`] key holds no object,
/// so it holds nothing up.
pub(crate) fn close_tab(
    open: Open,
    mut asm_at: State<Positions<Entry>>,
    mut src_at: State<Positions<Entry>>,
    mut code_at: State<Positions<Entry, Spot>>,
    mut driven: State<Driven>,
    mut marks_at: State<Positions<Entry, Kept>>,
    id: DocId,
) {
    let Open { mut dock, mut docs } = open;
    let tab = Tab::Document(id);

    // Worked out before anything is removed, which is what `tabs::landing` wants, and in
    // a scope of its own so no read guard is alive when the writes below start.
    let (held, was_showing, next) = {
        let dock = dock.peek();
        let Some(panel) = dock.document_panel() else {
            return;
        };
        (
            panel.tabs.contains(&tab),
            panel.active_tab_id == Some(tab),
            tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), |open| {
                *open == tab
            }),
        )
    };
    if !held {
        return;
    }

    {
        // Removed by hand and never through freya's `remove_tab_except`, which sets the
        // panel's active tab to `tabs.first()` when it takes the active one out. Landing
        // on the *neighbour* (`tabs::landing`) is this app's rule, and the write is the
        // whole of the landing: a tab landed on is already open, so there is nothing to
        // open and nothing to record.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel.tabs.retain(|open| *open != tab);
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    docs.write().close(id);
    let kept = |(tab, _): &Entry| *tab != id;
    asm_at.write().forgetting(kept);
    src_at.write().forgetting(kept);
    code_at.write().forgetting(kept);
    marks_at.write().forgetting(kept);
    driven.write().forget_tab(id);
}

/// Close every document tab except `keep`, leaving the views in the panel alone and
/// landing on the kept tab when the one on screen is among those closing.
///
/// The unit is the **tab** and not the binary, so this is [`close_tab`] many times over
/// rather than [`close_binary`] with another filter: what each of them lets go of is the
/// same -- the tab, its trail, everything kept by its entries -- and for the same reason,
/// an [`Entry`] key holding the `Arc<Object>` it points into. Done in one pass rather
/// than by calling [`close_tab`] in a loop: each of those would work out a landing of its
/// own and walk the panel through every intermediate state, and the landing here is known
/// from the start.
///
/// A view that shares the document panel is not a document and never closes; it also
/// keeps the screen when it is the tab on top, since nothing it is showing is going away.
pub(crate) fn close_others(
    open: Open,
    mut asm_at: State<Positions<Entry>>,
    mut src_at: State<Positions<Entry>>,
    mut code_at: State<Positions<Entry, Spot>>,
    mut driven: State<Driven>,
    mut marks_at: State<Positions<Entry, Kept>>,
    keep: DocId,
) {
    let Open { mut dock, mut docs } = open;
    let kept = Tab::Document(keep);

    // Which tabs go and whether the one on screen is among them, worked out before
    // anything is removed and in a scope of its own, so no read guard is alive when the
    // writes below start.
    let (closing, was_showing) = {
        let dock = dock.peek();
        let Some(panel) = dock.document_panel() else {
            return;
        };
        // A tab that is not in the panel any more keeps its neighbours: this is the menu
        // of a tab that was closed while the menu was open.
        if !panel.tabs.contains(&kept) {
            return;
        }
        let closing: Vec<DocId> = panel
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::Document(id) if *id != keep => Some(*id),
                _ => None,
            })
            .collect();
        let was_showing = matches!(panel.active_tab_id, Some(Tab::Document(id)) if id != keep);
        (closing, was_showing)
    };
    if closing.is_empty() {
        return;
    }

    {
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel
                .tabs
                .retain(|tab| !matches!(tab, Tab::Document(id) if closing.contains(id)));
            if was_showing {
                panel.active_tab_id = Some(kept);
            }
        }
    }
    {
        let mut docs = docs.write();
        for id in &closing {
            docs.close(*id);
        }
    }

    let held = |(tab, _): &Entry| !closing.contains(tab);
    asm_at.write().forgetting(held);
    src_at.write().forgetting(held);
    code_at.write().forgetting(held);
    marks_at.write().forgetting(held);
    // One guard rather than one write per tab: a write notifies whether or not it
    // changed anything, and a dozen tabs closing is one change.
    driven.write().forgetting(held);
}

/// Let go of the binary at `path`: drop every [`Object`] it contributed and answer for
/// everything that was pointing at them.
///
/// The unit is the **file** and never the object, so one path opened twice closes once.
/// A tab *showing* a place in the file is closed, its positions forgotten with it; every
/// other tab keeps its slot and loses the places in the file from its trail, the cursor
/// carried to the nearest older survivor -- a source-driven tab's binary entries go this
/// way, and the tab stands. The History panel drops those places rather than degrading
/// them; a load still running is cancelled, or its objects would put the file back one
/// member at a time.
///
/// All the writes happen in this one handler, so the save observer wakes once on a settled
/// state and never writes a binary the app has already let go of.
pub(crate) fn close_binary(
    mut objects: State<Vec<Arc<Object>>>,
    mut loading: State<Loads>,
    open: Open,
    mut asm_at: State<Positions<Entry>>,
    mut src_at: State<Positions<Entry>>,
    mut code_at: State<Positions<Entry, Spot>>,
    mut driven: State<Driven>,
    mut marks_at: State<Positions<Entry, Kept>>,
    mut visits: State<Visits>,
    path: &Path,
) {
    let Open { mut dock, mut docs } = open;
    // Every guard below is taken out of its own statement or its own scope, so none of
    // them is still alive when the next write is reached.
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
        let closing: Vec<DocId> = panel
            .tabs
            .iter()
            .filter_map(|tab| match tab {
                Tab::Document(id) if in_file(tab) => Some(*id),
                _ => None,
            })
            .collect();
        let next = tabs::landing(&panel.tabs, panel.active_tab_id.as_ref(), in_file);
        (closing, next)
    };

    let was_showing = showing
        .as_ref()
        .is_some_and(|showing| showing.in_file(path));
    {
        // The write is the whole of the landing, as in `close_tab`.
        let mut dock = dock.write();
        if let Some(panel) = dock.document_panel_mut() {
            panel
                .tabs
                .retain(|tab| !matches!(tab, Tab::Document(id) if closing.contains(id)));
            if was_showing {
                panel.active_tab_id = next;
            }
        }
    }
    {
        let mut docs = docs.write();
        for id in &closing {
            docs.close(*id);
        }
        // The surviving tabs' trails, thinned: every tab whose current entry is in the
        // file has just been closed, so no trail is left with nothing on it.
        docs.retain_entries(|document| !document.in_file(path));
    }

    // Nothing kept by an entry can outlive the entry: not the closed tabs', and not the
    // ones a surviving trail just lost, which hold the file's bytes just the same.
    let kept = |(tab, document): &Entry| !closing.contains(tab) && !document.in_file(path);
    asm_at.write().forgetting(kept);
    src_at.write().forgetting(kept);
    code_at.write().forgetting(kept);
    marks_at.write().forgetting(kept);
    {
        // A source-driven tab stands, but a symbol it chose out of this file is let go:
        // the line beside the choice is what survives a close, and the next ask answers
        // out of what is left.
        let mut driven = driven.write();
        driven.release(path);
        driven.forgetting(kept);
    }

    let remaining = visits.peek().retaining(|entry| !entry.in_file(path));
    visits.set(remaining);

    objects.write().retain(|object| object.path != path);
    // Dropping the entry is what makes the next batch of objects out of this file be
    // dropped and the worker itself stop; see `take_load`.
    loading.write().cancel(path);
}

/// Open `landing`'s tab on its line and its instruction: open it the way `reach` says,
/// pick the line out in the source pane with both panes owed the scroll, and put the
/// assembly pane's caret on the instruction. The line is picked out at once when the
/// document is already on top, since opening then changes nothing and no effect would
/// run, and otherwise left as the [`Landing`] for the change of document to turn into
/// the run; the instruction is always a [`Planting`], the listing it is a row of coming
/// after the document -- left here for a tab on top, and by `use_land` otherwise.
pub(crate) fn land(
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    mut land: State<Option<Landing>>,
    mut plant: State<Option<Planting>>,
    landing: Landing,
    reach: Reach,
) -> Option<DocId> {
    if open.active().as_ref() == Some(&landing.tab) {
        if let Some(at) = landing.at {
            mark_line(marked, at.file, at.line, Owed::BOTH);
        }
        if let Some(address) = landing.address {
            plant.set(Some(Planting {
                tab: landing.tab,
                address,
            }));
        }
        return open.active_id();
    }

    let tab = landing.tab.clone();
    land.set(Some(landing));
    open_document(open, visits, tab, reach)
}

/// Raise the open tab `id` on `at`: what a Locations row does for the source-driven tab
/// its question was asked from, whose assembly side it has just chosen for. The tab is
/// already open and shows the file, so this is a [`raise`] and not an opening -- nothing
/// is recorded -- with the line picked out the way [`land`] picks it.
pub(crate) fn land_on(
    open: Open,
    marked: State<Marks>,
    mut landing: State<Option<Landing>>,
    id: DocId,
    at: LinePos,
) {
    if open.active_id() == Some(id) {
        mark_line(marked, at.file, at.line, Owed::BOTH);
        return;
    }
    // Bound in a statement of its own: the guard is gone before the writes.
    let showing = open.docs.peek().get(id).cloned();
    let Some(tab) = showing else {
        return;
    };
    landing.set(Some(Landing {
        tab,
        at: Some(at),
        address: None,
    }));
    raise(open, id);
}

/// The menu a document's tab opens on a right-click.
///
/// The menu a document's header opens on a right-click: **Close other tabs** where the tab
/// has company, and the bookmark item for the tab's document always. Built per press, as
/// [`close_menu`] is, closing over the tab it was opened on; the states come in as
/// arguments because this is called from an event handler, where no hook may run. The
/// header says whether there is another document to close, so the one row that would do
/// nothing is left out rather than drawn dead.
pub(crate) fn tab_menu(
    states: ProjectStates,
    keep: DocId,
    others: bool,
    document: Document,
) -> Menu {
    let ProjectStates {
        open,
        asm_at,
        src_at,
        code_at,
        driven,
        marks_at,
        bookmarks,
        objects,
        ..
    } = states;

    Menu::new()
        .maybe_child(others.then(|| {
            MenuButton::new()
                .on_press(move |_| {
                    close_others(open, asm_at, src_at, code_at, driven, marks_at, keep)
                })
                // "tabs" and not "documents": the strip is what the reader is pointing at,
                // and a view sharing the panel is a tab this leaves alone.
                .child("Close other tabs")
        }))
        .child(bookmark_item(bookmarks, objects, document, "Add bookmark"))
}

/// The menu a Files row over an object that is not loaded opens on a right-click: one
/// item, opening it the way the toolbar's Open does. Built per press, like every menu
/// here, and the states come in as arguments because no hook may run in a handler.
pub(crate) fn open_menu(
    objects: State<Vec<Arc<Object>>>,
    loading: State<Loads>,
    path: PathBuf,
) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                let path = path.clone();
                // `spawn_forever`, not `spawn`: a task belongs to the scope that spawned
                // it, and this one's is the menu's button, which the press closes -- the
                // load would be dropped before its first poll.
                spawn_forever(async move {
                    open_binaries(objects, loading, vec![path]).await;
                });
            })
            // The opposite of the Objects row's "Close file", in the same word.
            .child("Open file"),
    )
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
        code_at,
        driven,
        marks_at,
        visits,
        ..
    } = states;

    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                close_binary(
                    objects, loading, open, asm_at, src_at, code_at, driven, marks_at, visits,
                    &path,
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

/// What a document is called where it is named in a list. A source file's *name* only and
/// a symbol's `module::fn_name` only ([`short_name`]); the whole of either is in
/// [`entry_tooltip`].
pub(crate) fn entry_text(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Symbol(_)) => short_name(&entry_name(entry)),
        entry => entry_name(entry),
    }
}

/// The whole of what a document is called: the demangled symbol name, the object's name,
/// or the source file's path. What a filter reads, so that a generic argument is still
/// something a reader can search for after the tab stopped drawing it.
pub(crate) fn entry_name(entry: &Document) -> String {
    match entry {
        Document::Assembly(Selection::Object(object)) | Document::Code(object) => {
            object.name.clone()
        }
        Document::Assembly(Selection::Symbol(symbol)) => symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone(),
        Document::Source(file) => file_name(file),
    }
}

/// What hovering a document's tab or row says: the whole path for a file and for an
/// object's code, whose name says nothing about where it came from; the whole name for
/// everything else -- which is where the rest of a shortened symbol name is.
pub(crate) fn entry_tooltip(entry: &Document) -> String {
    match entry {
        Document::Source(file) => file.to_string(),
        Document::Code(object) => object.path.display().to_string(),
        entry => entry_name(entry),
    }
}

/// Which kind of tab this is, as the one glyph that tells the three apart.
pub(crate) fn entry_icon(entry: &Document) -> Element {
    let (name, svg) = match entry {
        Document::Assembly(_) => ("binary", lucide::binary()),
        Document::Source(_) => ("file-code", lucide::file_code()),
        Document::Code(_) => ("scroll-text", lucide::scroll_text()),
    };

    document_glyph((name, svg))
}

/// One of the three glyphs above, at the interface font's own size and in the palette's
/// `icon_fg`; what [`entry_icon`] and a bookmark row's icon are both made of.
pub(crate) fn document_glyph(source: impl Into<ImageSource>) -> Element {
    let side = icon_size();
    SvgViewer::new(source)
        .width(Size::px(side))
        .height(Size::px(side))
        .color(palette().icon_fg)
        .show_loader(false)
        .into_element()
}

/// The identity of what a document points at, for keying the row or tab that names it.
/// The variant is part of the key so a pointer and a path cannot hash into one key for
/// two tabs of different kinds.
#[derive(Hash)]
pub(crate) enum EntryKey<'a> {
    Object(usize),
    Symbol(usize),
    Source(&'a str),
    Code(usize),
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
        Document::Code(object) => EntryKey::Code(Arc::as_ptr(object).addr()),
    }
}

/// Open the binary at `path` in place of whatever the app already had from it:
/// [`close_binary`] and then what the toolbar's Open does, in one handler.
///
/// A binary is a **path** throughout the app, so two generations of one file must not be
/// in the objects list together. The close is therefore first and unconditional -- whether
/// or not the new build parses, the objects in hand describe bytes that are gone -- and it
/// takes that file's tabs, their positions and its visits with it.
pub(crate) fn reopen_binary(states: ProjectStates, path: PathBuf) {
    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.code_at,
        states.driven,
        states.marks_at,
        states.visits,
        &path,
    );

    spawn(async move {
        open_binaries(states.objects, states.loading, vec![path]).await;
    });
}

/// A step along the trail of the tab on screen: the mouse's back and forward buttons, and
/// the toolbar's two chevrons.
#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Back,
    Forward,
}

impl Nav {
    /// The entry this step would land on along `trail`, or `None` when it would not
    /// move. What the toolbar's two buttons name in their tooltips, and the one place the
    /// answer is worked out, so a button that is live and a step that does something
    /// cannot disagree.
    pub(crate) fn destination(self, trail: &History) -> Option<&Document> {
        let cursor = trail.cursor()?;
        let index = match self {
            Self::Back => cursor.checked_sub(1)?,
            Self::Forward => cursor + 1,
        };
        trail.entries().get(index)
    }

    /// Move the cursor and hand back the entry it landed on.
    fn step(self, trail: &mut History) -> Option<Document> {
        match self {
            Self::Back => trail.back(),
            Self::Forward => trail.forward(),
        }
    }
}

/// Move the active tab's cursor one entry back or forward along its trail. The tab is
/// already on top, so what it shows is the whole of what changes: nothing is opened,
/// nothing is recorded, and a temporal tab stays temporal -- walking a trail is not
/// going somewhere new in it.
pub(crate) fn navigate(open: Open, nav: Nav) {
    let mut docs = open.docs;
    let Some(id) = open.active_id() else {
        return;
    };
    // Asked before writing: `State::write` notifies whether or not the value changes, and
    // a no-op step has to wake nothing.
    let possible = docs
        .peek()
        .trail(id)
        .is_some_and(|trail| nav.destination(trail).is_some());
    if !possible {
        return;
    }
    if let Some(trail) = docs.write().trail_mut(id) {
        nav.step(trail);
    }
}

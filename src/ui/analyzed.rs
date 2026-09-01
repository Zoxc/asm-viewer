//! The one question the UI asks off its own thread, and everything that comes back from
//! it: the worker `use_analysis` owns, the `Studied` it hands over, and the `Analyzed` the
//! panes draw out of while it works.
//!
//! The two ends are one file because they are one mechanism. Requests **supersede** rather
//! than accumulate -- a reader down the symbol list wants the last answer and none of the
//! ones before it -- and a superseded answer is *recognised* and not prevented: every
//! answer carries the `Symbol` it is about, which is what a comparison against the current
//! selection settles without a counter to keep in step.

use super::*;

/// Everything the analysis crate has to say about the selected symbol, shared through
/// context so every pane that maps between source and assembly reads the same answer --
/// and worked out on a thread of its own, so no pane waits for it.
///
/// See [`use_analysis`] for where the work runs, how an answer nobody wants any more is
/// dropped, and why the state has three fields rather than one.
#[derive(Clone, Copy)]
pub(crate) struct Analysis(pub(crate) State<Analyzed>);

/// What the two panes are drawing, and what is being worked out for them.
///
/// Three fields and not "the answer for the current selection", because the answer for
/// the current selection is exactly what there is not while it is being worked out, and
/// the panes have to draw *something* in the meantime.
#[derive(Clone, Default)]
pub(crate) struct Analyzed {
    /// The symbol the panes are drawing and everything they draw it from.
    ///
    /// It is the selected symbol whenever the worker has caught up, and the one selected
    /// *before* it while it has not: a listing is replaced by the next listing, never by
    /// a blank pane. That ordering is the whole of the "quiet" requirement -- a symbol
    /// that decodes in two milliseconds still costs a frame or two to come back over a
    /// channel, and clearing the pane first would be a flash of empty on every single
    /// click. `None` is the selection not being a symbol at all, which is answered on the
    /// spot and never waits for anything.
    pub(crate) shown: Option<Studied>,
    /// The symbol the worker is working on, or `None` when it is idle. It is what tells
    /// the panes apart the two ways `shown` can be `None`: nothing selected, and nothing
    /// *yet*.
    pub(crate) pending: Option<Symbol>,
    /// Whether `pending` has been outstanding for [`SLOW_ANALYSIS`], and the only thing
    /// that ever puts a message on screen. A wait worth naming is one the reader has
    /// already noticed; anything shorter is noise, and a spinner that appears for one
    /// frame per click is worse than the wait it is describing.
    pub(crate) slow: bool,
}

impl PartialEq for Analyzed {
    fn eq(&self, other: &Self) -> bool {
        self.shown == other.shown && self.pending == other.pending && self.slow == other.slow
    }
}

/// What a pane draws, which is one decision and not two panes' worth of `if`s.
pub(crate) enum Showing<'a> {
    /// This analysis, which is the only state that has one.
    Listing(&'a Studied),
    /// Nothing to draw and a word for why.
    Message(&'static str),
    /// Nothing to draw and nothing worth saying: a wait too short to name, with no
    /// previous listing to leave up. Only reachable before the first symbol of a session
    /// has been analysed, since after that there always is one.
    Nothing,
}

impl Analyzed {
    /// What the panes draw. One answer for both of them, so they cannot disagree about
    /// which of the "nothing here" states the app is in.
    ///
    /// The order of the arms is the design. A wait long enough to name wins over the
    /// listing still on screen, because leaving the previous function up for a second
    /// under the next function's tab is a lie the reader would read; anything shorter
    /// loses to it, because replacing a listing with a blank for one frame is a flash of
    /// white on every click.
    pub(crate) fn showing(&self) -> Showing<'_> {
        match (&self.shown, &self.pending, self.slow) {
            (_, Some(_), true) => Showing::Message("Analysing..."),
            (Some(shown), _, _) => Showing::Listing(shown),
            (None, Some(_), false) => Showing::Nothing,
            (None, None, _) => Showing::Message("No symbol selected"),
        }
    }
}

/// Everything worked out about one symbol, in one value because it is worked out in one
/// go.
///
/// The disassembly and the line info travel together deliberately: they are asked for at
/// the same moment, they are read by the same two panes, and `AsmData` needs both to say
/// which source position an instruction came from. Handing them over separately is what
/// the `Lines` memo used to do, and it cost every selection change a second render -- the
/// disassembly arriving in one and the line info in the next.
#[derive(Clone)]
pub(crate) struct Studied {
    /// Which symbol this is the analysis of. The panes key their viewing position, their
    /// rows and their chip on it, so it travels with the answer rather than being read
    /// back out of `Sel`, which by then may be somewhere else entirely.
    pub(crate) symbol: Symbol,
    /// [`None`] for a symbol with no bytes to decode at all; the pane says so.
    pub(crate) assembly: Option<Arc<Assembly>>,
    /// Where this symbol's branches are drawn in the gutter. Derived from `assembly` and
    /// from nothing else, and built here beside it -- a lane layout that arrived a beat
    /// after the disassembly it belongs to would be drawn over the wrong rows.
    pub(crate) lanes: Arc<Lanes>,
    pub(crate) lines: SymbolLines,
}

impl PartialEq for Studied {
    fn eq(&self, other: &Self) -> bool {
        let same_assembly = match (&self.assembly, &other.assembly) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };

        self.symbol == other.symbol
            && same_assembly
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Studied {
    /// The whole of the expensive work, in the order it costs: `assembly` decodes and
    /// formats every instruction of the symbol, `line_info` builds this object's DWARF
    /// context on the first call against it (267 MB of it for `viewer-sample`) and walks
    /// the line program of every unit covering the symbol on each one.
    ///
    /// Nothing in here touches any UI state, which is what lets it run on a plain
    /// `std::thread`: it is handed a [`Symbol`] and hands back a value. See
    /// [`use_analysis`].
    pub(crate) fn new(symbol: Symbol) -> Studied {
        let assembly = symbol.data.assembly(&symbol.object);
        // An `Assembly`-less symbol has no rows to draw a gutter over, and `Lanes` is
        // built from the edges rather than from the assembly, so this needs no branch of
        // its own beyond the one that gets the edges.
        let lanes = Arc::new(match &assembly {
            Some(assembly) => Lanes::new(&assembly.edges, assembly.instructions.len()),
            None => Lanes::new(&[], 0),
        });
        let lines = SymbolLines::new(&symbol);

        Studied {
            symbol,
            assembly,
            lanes,
            lines,
        }
    }
}

/// What DWARF says about the selected symbol's instructions, or `None` when it says
/// nothing, and which of the files it names the Source pane draws beside it.
///
/// Worked out once for all its readers rather than once per pane: `Object::line_info`
/// walks the line program of every unit covering the symbol again on each call, even
/// though the DWARF context itself is built only once.
///
/// The file is worked out *here*, beside the info it comes from, rather than by whoever
/// wants it. The answer arrives from a worker thread, so anything reading `Sel` and this
/// together sees them disagree for as long as the work takes -- and asking the previous
/// symbol's `LineInfo` where the new symbol starts answers with the previous symbol's
/// file, which would open a tab for a file that has nothing to do with what was clicked.
/// Inside one value the two cannot disagree.
#[derive(Clone)]
pub(crate) struct SymbolLines {
    pub(crate) info: Option<Arc<LineInfo>>,
    /// Which of the files the symbol touches the Source pane draws: the one its first
    /// instruction was compiled from, which is the function's own file rather than one of
    /// the headers it inlined further in. A symbol whose entry instructions belong to no
    /// row at all -- a compiler-generated prologue is enough for that -- falls back to the
    /// first file the rows name, and one whose rows name no file at all has none.
    pub(crate) file: Option<Arc<str>>,
}

impl PartialEq for SymbolLines {
    fn eq(&self, other: &Self) -> bool {
        let same_info = match (&self.info, &other.info) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };

        // The file is compared by its text, not by pointer, for the reason `LinePos` is:
        // a path is a value. Two `LineInfo`s naming one file hold two `Arc<str>`s of it.
        same_info && self.file == other.file
    }
}

impl SymbolLines {
    /// The line info for `symbol`, with the file the Source pane draws beside it.
    fn new(symbol: &Symbol) -> SymbolLines {
        let info = symbol.data.line_info(&symbol.object);
        let file = info.as_ref().and_then(|info| {
            info.row_at(symbol.data.address)
                .and_then(|row| row.file)
                .and_then(|file| info.files().get(file))
                .or_else(|| info.files().first())
                .cloned()
        });

        SymbolLines { info, file }
    }
}

/// Work the selected symbol out on a thread of its own, and hand the answer to the panes
/// through [`Analysis`].
///
/// **Where the work runs: one worker thread, for the app's lifetime.** Not a thread per
/// request and not a pool, because requests here *supersede* each other rather than
/// accumulating: a reader holding the down-arrow through a symbol list issues one per
/// row and wants exactly the last one's answer. A thread per request would put the whole
/// run of them through the most expensive call in the crate at once — the first
/// `line_info` against an object builds its entire DWARF context — with every answer but
/// one thrown away, and `DwarfCache` is a `OnceLock`, so the losers would not even be
/// racing usefully: they block on the winner. A pool has the same shape with a bound on
/// it. One worker instead, with the queue drained to its newest entry each time round, so
/// the requests the reader clicked past are dropped *before* they are started rather than
/// after. It also gives the answers an order — request order — which is what makes a stale
/// answer always an old one and never a new one.
///
/// This is deliberately not the multi-threading `notes/Goals.md` asks for under
/// "lightweight and multi threaded": that one is about parsing many objects at once, which
/// is [`open_binaries`]' worker and its own answer. This is one reader looking at one
/// function, where the useful number of threads is one and the point is only that it is
/// not the one drawing the window.
///
/// **How a superseded answer is dropped.** Every answer carries the [`Symbol`] it is
/// about, and it is kept only when that symbol is the one selected *now* — a comparison,
/// not a generation counter, because `Selection` compares by `Arc` pointer identity and so
/// already answers this exactly. A counter would be a second identity to keep in step with
/// the first, and would get the ordinary A → B → A case wrong: the answer for the first A
/// is a perfectly good answer for the third selection, and this shows it rather than
/// working it out again. A dropped answer is the normal case and not an error — it is what
/// clicking twice quickly *means* — so nothing logs, warns or retries.
///
/// **What the panes show meanwhile** is in [`Analyzed`]: the listing they already have,
/// until either the next one arrives or [`SLOW_ANALYSIS`] passes.
/// What [`use_analysis_with`] needs of the active document: a **read**, which subscribes
/// the effect to it, and a **peek**, which does not. The distinction is load-bearing --
/// the effect must wake on a change of document and must not wake on its own writes -- so
/// it cannot collapse into one closure.
///
/// A trait so the hook can be driven by the [`Active`] memo in the app and by a plain
/// state in the tests, which are about the worker rather than about the tabs and have no
/// business building a dock to say which symbol is selected.
pub(crate) trait ReadsActive: Copy + 'static {
    fn read_active(self) -> Option<Document>;
    fn peek_active(self) -> Option<Document>;
}

impl ReadsActive for Memo<Option<Document>> {
    fn read_active(self) -> Option<Document> {
        self.read().clone()
    }

    fn peek_active(self) -> Option<Document> {
        self.peek().clone()
    }
}

impl ReadsActive for State<Option<Document>> {
    fn read_active(self) -> Option<Document> {
        self.read().clone()
    }

    fn peek_active(self) -> Option<Document> {
        self.peek().clone()
    }
}

pub(crate) fn use_analysis(active: Memo<Option<Document>>, analysis: State<Analyzed>) {
    use_analysis_with(active, analysis, Studied::new);
}

/// The whole of [`use_analysis`], with the work itself as an argument so a test can hold
/// it still. Superseding is a race by construction — the answer that has to be dropped is
/// the one that arrives while the reader has already clicked on — and nothing can assert
/// it against a worker that answers as fast as it is asked.
pub(crate) fn use_analysis_with(
    active: impl ReadsActive,
    mut analysis: State<Analyzed>,
    study: impl Fn(Symbol) -> Studied + Send + 'static,
) {
    // The worker and the task that listens to it, started once and never restarted. Both
    // channels are unbounded, which costs nothing here: the request side holds at most
    // what the reader has clicked since the worker last looked, and the answer side at
    // most one per request.
    let requests = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<Symbol>();
        let (answered, answers) = async_channel::unbounded::<Studied>();

        // A `std::thread` and not a spawned task, exactly as `open_files` is: this is
        // seconds of decoding and DWARF parsing, and freya's executor is the UI thread.
        std::thread::spawn(move || {
            while let Ok(symbol) = jobs.recv_blocking() {
                // Everything the reader clicked past while the last job ran, dropped
                // without being started. Only the newest is wanted, and finding that out
                // here rather than after the fact is the difference between a stale
                // answer costing a comparison and costing a second of decoding.
                let mut symbol = symbol;
                while let Ok(newer) = jobs.try_recv() {
                    symbol = newer;
                }

                // A send that fails is the app shutting down and taking the receiver
                // with it.
                if answered.send_blocking(study(symbol)).is_err() {
                    return;
                }
            }
        });

        spawn(async move {
            let mut analysis = analysis;
            while let Ok(studied) = answers.recv().await {
                // The superseding rule. Cloned out of the guard first, since everything
                // below it writes.
                let current = active.peek_active();
                if !current
                    .as_ref()
                    .and_then(Document::symbol)
                    .is_some_and(|symbol| *symbol == studied.symbol)
                {
                    continue;
                }

                let mut next = analysis.peek().clone();
                if next.pending.as_ref() == Some(&studied.symbol) {
                    next.pending = None;
                    next.slow = false;
                }
                // Already on screen: the same symbol answered twice, which happens when
                // the reader clicks away and straight back before the worker has looked
                // at the queue. Keeping the listing that is up rather than replacing it
                // with an identical one saves re-rendering every row for nothing.
                if !next
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.symbol == studied.symbol)
                {
                    next.shown = Some(studied);
                }
                analysis.set_if_modified(next);
            }
        });

        requests
    });

    use_side_effect(move || {
        // Reading subscribes this to the active document, which is the only thing it
        // answers to; the state it writes is `peek`ed, so it cannot wake itself.
        let current = active.read_active();

        let Some(symbol) = current.as_ref().and_then(Document::symbol).cloned() else {
            // Not a function: an object, a source file, or nothing open at all. There
            // is nothing to work out and so nothing to wait for, and the panes are told
            // at once — clearing is instant even though replacing is not. Anything still
            // in flight is for a place the reader has left and is dropped when it lands.
            analysis.set_if_modified(Analyzed::default());
            return;
        };

        let state = analysis.peek().clone();

        if state
            .shown
            .as_ref()
            .is_some_and(|shown| shown.symbol == symbol)
        {
            // Already drawn. Nothing to ask for — and nothing left to wait for either:
            // whatever the worker is still chewing on is for somewhere the reader has
            // since come back from, so the pane must not go on to say it is waiting for
            // it.
            if state.pending.is_some() {
                let mut next = state;
                next.pending = None;
                next.slow = false;
                analysis.set(next);
            }
            return;
        }
        if state.pending.as_ref() == Some(&symbol) {
            return;
        }

        let mut next = state;
        next.pending = Some(symbol.clone());
        next.slow = false;
        analysis.set(next);
        // Unbounded, so this cannot fail for any reason but the worker being gone.
        let _ = requests.try_send(symbol.clone());

        // The wait, started by the request and by nothing else. A timer per request
        // rather than something polled: a symbol that comes back inside `SLOW_ANALYSIS`
        // — which is nearly all of them — costs one task that wakes up, finds the request
        // it belongs to already answered, and writes nothing.
        spawn(async move {
            Timer::after(SLOW_ANALYSIS).await;
            let mut analysis = analysis;
            let still = analysis.peek().pending.as_ref() == Some(&symbol);
            if still {
                analysis.write().slow = true;
            }
        });
    });
}

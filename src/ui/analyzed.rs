//! The analysis worker `use_analysis` owns, the `Studied` it hands over, and the
//! `Analyzed` the panes draw out of while it works.

use super::*;

/// Everything the analysis crate has to say about the selected symbol, shared through
/// context.
#[derive(Clone, Copy)]
pub(crate) struct Analysis(pub(crate) State<Analyzed>);

/// What the two panes are drawing, and what is being worked out for them.
#[derive(Clone, Default)]
pub(crate) struct Analyzed {
    /// The symbol the panes are drawing: the selected one once the worker has caught up,
    /// and the one selected *before* it while it has not.
    pub(crate) shown: Option<Studied>,
    /// The symbol the worker is working on, or `None` when it is idle -- which is what
    /// tells the two ways `shown` can be `None` apart: nothing selected, and nothing yet.
    pub(crate) pending: Option<Symbol>,
    /// Whether `pending` has been outstanding for [`SLOW_ANALYSIS`].
    pub(crate) slow: bool,
}

impl PartialEq for Analyzed {
    fn eq(&self, other: &Self) -> bool {
        self.shown == other.shown && self.pending == other.pending && self.slow == other.slow
    }
}

/// What a pane draws, which is one decision and not two panes' worth of `if`s.
pub(crate) enum Showing<'a> {
    Listing(&'a Studied),
    /// Nothing to draw and a word for why.
    Message(&'static str),
    /// A wait too short to name, with no previous listing to leave up.
    Nothing,
}

impl Analyzed {
    /// What the panes draw, one answer for both of them.
    ///
    /// **The order of the arms is the mechanism**: a listing beats a short wait, so a
    /// click never flashes the pane empty; a wait past [`SLOW_ANALYSIS`] beats the stale
    /// listing, so the previous function is not left up under the next one's tab.
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
#[derive(Clone)]
pub(crate) struct Studied {
    /// Which symbol this is the analysis of. It travels with the answer, which is what
    /// the supersession check in [`use_analysis_with`] compares against.
    pub(crate) symbol: Symbol,
    /// [`None`] for a symbol with no bytes to decode at all; the pane says so.
    pub(crate) assembly: Option<Arc<Assembly>>,
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
    /// The expensive work: decoding the symbol and building the object's DWARF context.
    /// Touches no UI state, which is what lets it run on a plain `std::thread`.
    pub(crate) fn new(symbol: Symbol) -> Studied {
        let assembly = symbol.data.assembly(&symbol.object);
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

/// What DWARF says about the selected symbol's instructions, and which of the files it
/// names the Source pane draws beside it. The file is carried here, beside the info it
/// comes from, so the two cannot disagree while the worker is still running.
#[derive(Clone)]
pub(crate) struct SymbolLines {
    pub(crate) info: Option<Arc<LineInfo>>,
    /// The file the symbol's first instruction was compiled from, falling back to the
    /// first file its rows name.
    pub(crate) file: Option<Arc<str>>,
}

impl PartialEq for SymbolLines {
    fn eq(&self, other: &Self) -> bool {
        let same_info = match (&self.info, &other.info) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };

        // The file compares by its text, not by pointer, for the reason `LinePos` does:
        // two `LineInfo`s naming one file hold two `Arc<str>`s of it.
        same_info && self.file == other.file
    }
}

impl SymbolLines {
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

/// What [`use_analysis_with`] needs of the active document: a **read**, which subscribes
/// the effect to it, and a **peek**, which does not -- the effect must wake on a change of
/// document and must not wake on its own writes, so the two cannot collapse into one.
///
/// A trait so the hook can be driven by the [`Active`] memo in the app and by a plain
/// state in the tests.
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

/// Work the selected symbol out on the app's one worker thread and hand the answer to the
/// panes through [`Analysis`]. Requests supersede: the queue is drained to its newest
/// entry, so what the reader clicked past is dropped before it is started.
///
/// The work itself is an argument so a test can hold it still: superseding is a race by
/// construction and cannot be asserted against a worker that answers as fast as it is
/// asked.
pub(crate) fn use_analysis_with(
    active: impl ReadsActive,
    mut analysis: State<Analyzed>,
    study: impl Fn(Symbol) -> Studied + Send + 'static,
) {
    // The worker and the task that listens to it, started once and never restarted.
    let requests = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<Symbol>();
        let (answered, answers) = async_channel::unbounded::<Studied>();

        // A `std::thread` and not a spawned task: this is seconds of decoding and DWARF
        // parsing, and freya's executor is the UI thread.
        std::thread::spawn(move || {
            while let Ok(symbol) = jobs.recv_blocking() {
                // Everything the reader clicked past while the last job ran, dropped
                // without being started rather than after the fact.
                let mut symbol = symbol;
                while let Ok(newer) = jobs.try_recv() {
                    symbol = newer;
                }

                // A send that fails is the app shutting down.
                if answered.send_blocking(study(symbol)).is_err() {
                    return;
                }
            }
        });

        spawn(async move {
            let mut analysis = analysis;
            while let Ok(studied) = answers.recv().await {
                // **The supersession rule**: an answer is kept only if its symbol is the
                // one selected *now* -- a comparison and not a generation counter, since
                // `Selection` already compares by `Arc` identity, and since the answer
                // for the first A of an A -> B -> A is good for the third selection. A
                // dropped answer is what clicking twice quickly means; nothing retries.
                // Cloned out of the guard first, since everything below it writes.
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
                // Already on screen: the same symbol answered twice. Keeping the listing
                // that is up saves re-rendering every row for nothing.
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
        // Reading subscribes this to the active document; the state it writes is
        // `peek`ed, so it cannot wake itself.
        let current = active.read_active();

        let Some(symbol) = current.as_ref().and_then(Document::symbol).cloned() else {
            // Not a function: nothing to work out and nothing to wait for. Anything
            // still in flight is dropped when it lands.
            analysis.set_if_modified(Analyzed::default());
            return;
        };

        let state = analysis.peek().clone();

        if state
            .shown
            .as_ref()
            .is_some_and(|shown| shown.symbol == symbol)
        {
            // Already drawn, so nothing to ask for and nothing left to wait for: the
            // pane must not go on saying it is waiting.
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
        let _ = requests.try_send(symbol.clone());

        // The wait, started by the request and never polled.
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

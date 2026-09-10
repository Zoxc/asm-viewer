//! The analysis worker `use_analysis` owns, the questions it is asked, the `Studied` it
//! hands over, and the `Analyzed` the panes draw out of while it works.
//!
//! Four kinds of job go to the one worker: a **listing** -- the symbol the panes draw,
//! named outright or resolved from a source line -- a **window** of an object's code for
//! the section view, a **locate**, every symbol a line or a function was compiled into,
//! for the Locations panel, and **marks**, the lines of one file that produced code at
//! all, for the Source pane's gutter. They supersede separately: the queue is drained to
//! the newest of *each*, since a reader who asked for a line's locations and then clicked
//! a symbol wants both answers.

use super::*;

/// Everything the analysis crate has to say about what the panes are drawing, shared
/// through context.
#[derive(Clone, Copy)]
pub(crate) struct Analysis(pub(crate) State<Analyzed>);

/// What the panes are being asked to draw.
///
/// Two kinds because a tab has two: an assembly-driven one names its symbol outright,
/// while a source-driven one names a line and the symbol is whatever that line was
/// compiled into -- or, where the reader chose among the many, the one they chose, since
/// a different choice is a different question. Equality is still identity, but of two
/// kinds -- [`Ask::Symbol`] by the `Arc` pointers [`Symbol`] compares, [`Ask::Source`] by
/// [`LinePos`], which is the one `Arc` in the UI compared by its text, and by the choice.
#[derive(Clone, PartialEq)]
pub(crate) enum Ask {
    Symbol(Symbol),
    Source {
        at: LinePos,
        /// See [`Driven::choice`].
        chosen: Option<Symbol>,
    },
}

/// The tab an answer to `ask` belongs to. One definition, used by the pane that keeps its
/// row, by the run of rows a listing change drops, and by the rule that decides whether a
/// question naming no symbol may leave the listing that is up.
pub(crate) fn asked_of(ask: &Ask) -> Document {
    match ask {
        Ask::Symbol(symbol) => Document::Assembly(Selection::Symbol(symbol.clone())),
        Ask::Source { at, .. } => Document::Source(at.file.clone()),
    }
}

/// What the panes are being asked to draw for `active`: the symbol an assembly-driven tab
/// names, or the symbol the line a source-driven tab is driven from was compiled into.
///
/// `None` for an object (which is not a place with a listing), for a tab that is not a
/// document, and for a source-driven tab nothing has been clicked in and which arrived
/// at no line.
///
/// The line the reader clicked wins over the one the place itself is: a place naming a
/// line is where they arrived, and a click after that is what they are reading now.
/// Falling back to it is what makes a door that lands on a line drive the assembly side
/// without a word from anything else, a restored session included.
pub(crate) fn ask(active: Option<&Entry>, driven: &Driven) -> Option<Ask> {
    let entry = active?;
    match &entry.1.document {
        Document::Assembly(Selection::Symbol(symbol)) => Some(Ask::Symbol(symbol.clone())),
        Document::Assembly(Selection::Object(_)) | Document::Code(_) => None,
        Document::Source(file) => driven
            .line(entry)
            .or(entry.1.line())
            .map(|line| Ask::Source {
                at: LinePos {
                    file: file.clone(),
                    line,
                },
                chosen: driven.choice(entry),
            }),
    }
}

/// One job for the worker, and everything answering it needs.
///
/// `objects` and `recent` travel with the job because a worker thread can read no UI
/// state. They are **not** the question: two asks that differ only in what was open are
/// the same question asked twice, which is why [`Ask`] and not this is what supersession
/// compares.
pub(crate) enum Question {
    Study(Symbol),
    Resolve {
        at: LinePos,
        /// The reader's own choice among the many, which outranks everything below.
        chosen: Option<Symbol>,
        /// The listing the panes are drawing, which travels with the question so that
        /// it can be the answer: a reader moving down a function asks a question per
        /// line and every one of them resolves to the symbol already decoded.
        standing: Option<Studied>,
        objects: Vec<Arc<Object>>,
        /// Where the reader has been, newest first, with the symbol on screen at its
        /// head. See [`compiled::pick`].
        recent: Vec<Symbol>,
    },
    /// Every symbol `query`'s lines were compiled into, for the Locations panel.
    Locate {
        query: Query,
        objects: Vec<Arc<Object>>,
    },
    /// A window of an object's code for the section view: the first [`CHUNK`] of the
    /// stretches it names, decoded.
    Code(CodeAsk),
    /// Every line of `file` the open objects have code from, for the Source pane's
    /// gutter marks.
    Marks {
        file: Arc<str>,
        objects: Vec<Arc<Object>>,
    },
}

/// The four kinds of job, which supersede separately.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Listing,
    Code,
    Locate,
    Marks,
}

impl Question {
    fn kind(&self) -> Kind {
        match self {
            Question::Study(_) | Question::Resolve { .. } => Kind::Listing,
            Question::Code(_) => Kind::Code,
            Question::Locate { .. } => Kind::Locate,
            Question::Marks { .. } => Kind::Marks,
        }
    }
}

/// The newest question of each kind out of `first` and whatever is `queued` behind it,
/// in the order they are worked: the listing first, since it is what is on screen, then
/// the window, then the locate, and the gutter's marks last -- the one whose absence
/// costs the reader least while they wait.
///
/// What the reader clicked past is dropped here, without being started. Per kind and not
/// overall, because a locate is not a newer version of the listing question -- drained to
/// one, a symbol click after asking for a line's locations would silently cancel the
/// locations, or the other way round -- and a window the reader scrolled past is the one
/// thing here that *should* go, the next window asking for whatever of it still matters.
pub(crate) fn newest(first: Question, queued: impl Iterator<Item = Question>) -> Vec<Question> {
    let mut listing = None;
    let mut code = None;
    let mut locate = None;
    let mut marks = None;
    for question in std::iter::once(first).chain(queued) {
        match question.kind() {
            Kind::Listing => listing = Some(question),
            Kind::Code => code = Some(question),
            Kind::Locate => locate = Some(question),
            Kind::Marks => marks = Some(question),
        }
    }
    listing
        .into_iter()
        .chain(code)
        .chain(locate)
        .chain(marks)
        .collect()
}

/// What the worker sends back: the question, and what it came to.
pub(crate) enum Answer {
    /// `studied` is `None` only for a source line no open object holds code from -- the
    /// one listing question that can name no symbol at all.
    Listing { ask: Ask, studied: Option<Studied> },
    /// The symbols `query` was compiled into, over the objects the question carried.
    Located { query: Query, symbols: Vec<Symbol> },
    /// The lines of `file` the objects the question carried have code from, and which
    /// objects those were.
    Marked {
        file: Arc<str>,
        lines: Arc<HashSet<u32>>,
        over: Vec<usize>,
    },
    /// The skeleton -- the ask's own, or built for it -- and the stretches decoded, by
    /// flat index: the first [`CHUNK`] the ask named that the listing has.
    Code {
        ask: CodeAsk,
        code: Arc<CodeListing>,
        decoded: Vec<(usize, Stretched)>,
    },
}

/// The expensive work, and the one definition of what an answer is: a third kind of
/// listing question cannot grow a second `Studied::new` call site. Touches no UI state,
/// which is what lets it run on a plain `std::thread`.
pub(crate) fn answer(question: Question) -> Answer {
    match question {
        Question::Code(ask) => {
            let (code, decoded) = ask.decode();
            Answer::Code { ask, code, decoded }
        }
        Question::Study(symbol) => Answer::Listing {
            ask: Ask::Symbol(symbol.clone()),
            studied: Some(Studied::new(symbol)),
        },
        Question::Resolve {
            at,
            chosen,
            standing,
            objects,
            recent,
        } => {
            let candidates = compiled::compiled_from(&objects, &at.file, at.line..=at.line);
            // The choice at the head of the ranking: it wins where the line compiled
            // into it, and where it did not the pick falls back as if none were made.
            let ranked: Vec<Symbol> = chosen.iter().cloned().chain(recent).collect();
            let studied = compiled::pick(&candidates, &ranked).map(|symbol| match standing {
                // The listing that is up is this symbol's already. It is handed back
                // untouched -- the same `Arc<Assembly>`, which is what says to the pane
                // that nothing changed -- rather than decoded a second time.
                Some(standing) if standing.symbol == symbol => standing,
                _ => Studied::new(symbol),
            });
            Answer::Listing {
                ask: Ask::Source { at, chosen },
                studied,
            }
        }
        Question::Locate { query, objects } => Answer::Located {
            symbols: query.symbols_wanted().map_or_else(Vec::new, |lines| {
                compiled::compiled_from(&objects, &query.at.file, lines)
            }),
            query,
        },
        Question::Marks { file, objects } => Answer::Marked {
            lines: Arc::new(
                objects
                    .iter()
                    .flat_map(|object| object.lines_from_source(&file))
                    .collect(),
            ),
            over: object_ids(&objects),
            file,
        },
    }
}

/// A listing, and the question it was worked out for.
#[derive(Clone, PartialEq)]
pub(crate) struct Shown {
    pub(crate) ask: Ask,
    pub(crate) studied: Studied,
}

impl Shown {
    /// Whether the object this listing points into is still open.
    ///
    /// **The one thing in the analysis that can outlive the document that named it.** A
    /// symbol question is a tab into one object and that tab closes with its file, so the
    /// ordinary change of active document has always taken care of it -- which is why
    /// nothing here needed asking before a source question could name a symbol. A
    /// source-driven tab survives a binary close by doctrine, so its answer would go on
    /// being drawn, and a [`Studied`] holds a [`Symbol`] holds the `Arc<Object>` holds
    /// the whole file's bytes: [`Positions::forgetting`]'s leak in a second place.
    ///
    /// Asked in the two places an answer is judged: by the effect, so a closed binary is
    /// a question asked again out of what is left, and by the task taking answers, so the
    /// one already in flight when the file closed is not taken either.
    fn still_open(&self, objects: &[Arc<Object>]) -> bool {
        match self.ask {
            Ask::Symbol(_) => true,
            Ask::Source { .. } => objects
                .iter()
                .any(|object| Arc::ptr_eq(object, &self.studied.symbol.object)),
        }
    }

    /// Whether this listing is an answer to `ask` as well as to the one it was worked out
    /// for. It is what keeps "the answer for the first A of an A -> B -> A is a good
    /// answer for the third" true across the two kinds: a source line that resolved to a
    /// symbol has already answered a later ask for that symbol outright, and
    /// re-disassembling it would be most of a second for nothing.
    fn answers(&self, ask: &Ask) -> bool {
        match ask {
            Ask::Symbol(symbol) => self.studied.symbol == *symbol,
            Ask::Source { .. } => self.ask == *ask,
        }
    }
}

/// What the two panes are drawing, and what is being worked out for them.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Analyzed {
    /// The listing the panes draw, and the question it answers. Replaced by the next
    /// listing and never by a blank, so its question can be older than `answered`.
    pub(crate) shown: Option<Shown>,
    /// The last question answered, whatever it answered *with*. What stops the effect
    /// asking one question twice, and the one thing a listing cannot say for itself: a
    /// source line no object holds code from leaves the listing that is up and is
    /// recorded only here.
    pub(crate) answered: Option<Ask>,
    /// What the worker is working on, or `None` when it is idle -- which is what tells
    /// the two ways `shown` can be `None` apart: nothing asked, and nothing yet.
    pub(crate) pending: Option<Pending>,
}

/// A question the worker has been sent and has not answered yet.
#[derive(Clone, PartialEq)]
pub(crate) struct Pending {
    pub(crate) ask: Ask,
    /// Whether it has been outstanding for [`SLOW_ANALYSIS`] -- long enough to say so,
    /// which is what displaces the listing that is up. A property of the wait and so a
    /// field of it: there is nothing to be slow about while nothing is being waited for.
    slow: bool,
}

impl Pending {
    /// A question just sent: waited for, and not yet long enough to say so.
    fn asked(ask: Ask) -> Pending {
        Pending { ask, slow: false }
    }
}

/// What a pane draws, which is one decision and not two panes' worth of `if`s.
pub(crate) enum Showing<'a> {
    /// The listing and the question it answers: a pane needs both, the question being
    /// what says which tab the listing belongs to.
    Listing(&'a Shown),
    /// Nothing to draw and a word for why. Owned and not `&'static`: a line that came
    /// to nothing is named in the words.
    Message(String),
    /// A wait too short to name, with no previous listing to leave up.
    Nothing,
}

impl Analyzed {
    /// What the panes draw, one answer for both of them. The **document** and not a word
    /// from the caller, so that this stays the one place either pane decides what it is
    /// drawing.
    ///
    /// **The order of the arms is the mechanism**: a listing beats a short wait, so a
    /// click never flashes the pane empty; a wait past [`SLOW_ANALYSIS`] beats the stale
    /// listing, so the previous function is not left up under the next one's tab; and a
    /// wait beats a question that named nothing, because a stale *listing* is doctrine
    /// and a stale *sentence* is not.
    pub(crate) fn showing(&self, document: &Document) -> Showing<'_> {
        match (&self.shown, &self.pending) {
            (_, Some(pending)) if pending.slow => Showing::Message("Analysing...".to_owned()),
            (Some(shown), _) => Showing::Listing(shown),
            (None, Some(_)) => Showing::Nothing,
            // Asked, and answered with no symbol at all. Only a source line can, and
            // the message names it: the answer outlives the click, so which line came
            // to nothing is not otherwise on screen.
            (None, None) if self.answered.is_some() => Showing::Message(match &self.answered {
                Some(Ask::Source { at, .. }) => format!("No code compiled from {}", at.spell()),
                _ => "No code compiled from this line".to_owned(),
            }),
            (None, None) => Showing::Message(
                match document {
                    Document::Assembly(_) => "No symbol selected",
                    Document::Source(_) => "Click a source line",
                    // The listing beside this asks nothing; its source side follows the
                    // instruction picked out in it.
                    Document::Code(_) => "Click an instruction",
                }
                .to_owned(),
            ),
        }
    }

    /// Take the answer `studied` to `ask`, `wanted` being the question asked *now* and
    /// `open` the binaries the project has. Whether anything changed, so the hook writes
    /// only then ([`write_if`]).
    ///
    /// **The supersession rule**: an answer is kept only if its question is the one being
    /// asked now -- a comparison and not a generation counter, since an [`Ask`] already
    /// compares by identity, and since the answer for the first A of an A -> B -> A is a
    /// perfectly good answer for the third. A dropped answer is what clicking twice
    /// quickly means, so nothing logs or retries.
    ///
    /// And an answer out of a binary closed since it was asked for is not taken either
    /// ([`Shown::still_open`]) -- the same rule [`Analyzed::asked`] applies to the listing
    /// that is up, so the two cannot drift.
    pub(crate) fn take(
        &mut self,
        ask: Ask,
        studied: Option<Studied>,
        wanted: Option<&Ask>,
        open: &[Arc<Object>],
    ) -> bool {
        if wanted != Some(&ask) {
            return false;
        }
        // What is held, kept to compare against: an answer the effect has already
        // settled -- a listing retagged while this one was in flight -- leaves everything
        // as it was, and must not cost a render for it.
        let before = self.clone();

        let landed = studied.map(|studied| Shown {
            ask: ask.clone(),
            studied,
        });
        let landed = landed.filter(|shown| shown.still_open(open));

        if self.waiting() == Some(&ask) {
            self.pending = None;
        }
        self.answered = Some(ask.clone());

        match landed {
            Some(shown) => self.shown = Some(shown),
            // A question that named no symbol leaves the listing that is up -- the click
            // lights no pair in it and nothing else, which is what says it landed nowhere
            // -- but **only when that listing is this tab's own**, or a source line
            // holding no code would leave another tab's function on screen for good.
            None => {
                let mine = self
                    .shown
                    .as_ref()
                    .is_some_and(|shown| asked_of(&shown.ask) == asked_of(&ask));
                if !mine {
                    self.shown = None;
                }
            }
        }

        *self != before
    }

    /// Bring this up to date with the question `ask`, asked over the binaries `open`, and
    /// answer with the question the worker is owed -- [`None`] where it is owed none.
    ///
    /// Three things happen here and each is a rule of its own. A listing whose binary has
    /// been closed is not in hand whatever question it answered, so it goes and the
    /// question is asked again out of what is left. A question already **held** is not
    /// asked again: either the listing that is up answers it, in which case it is
    /// *retagged* rather than worked out afresh -- a source question that resolved to a
    /// symbol has already answered a later ask for that symbol outright, and
    /// re-disassembling it would be most of a second for nothing -- or the question has
    /// been asked and answered with nothing, which is an answer. What is left is asked,
    /// and marked pending so that it is not asked twice.
    ///
    /// `visits` is where the reader has been, which ranks the candidates a source line
    /// resolves among ([`compiled::pick`]). It is an input to an answer and never part of
    /// a question, which is why a visit must not make this ask again.
    pub(crate) fn asked(
        &mut self,
        ask: Option<&Ask>,
        open: &[Arc<Object>],
        visits: &Visits,
    ) -> Option<Question> {
        let Some(ask) = ask else {
            // Not a place with a listing: nothing to work out and nothing to wait for.
            // Anything still in flight is dropped when it lands.
            *self = Analyzed::default();
            return None;
        };

        // Dropped here rather than by `close_binary`, so that a close, a rebuild and a
        // project switch are one line instead of three.
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| !shown.still_open(open))
        {
            *self = Analyzed::default();
        }

        let held = self.shown.as_ref().is_some_and(|shown| shown.answers(ask))
            || self.answered.as_ref() == Some(ask);
        if held {
            // Retagged, so the same listing is not asked for again under its new
            // question, and so nothing goes on saying it is waiting.
            if let Some(shown) = self.shown.as_mut().filter(|shown| shown.answers(ask)) {
                shown.ask = ask.clone();
            }
            self.answered = Some(ask.clone());
            self.pending = None;
            return None;
        }
        if self.waiting() == Some(ask) {
            return None;
        }

        let question = match ask {
            Ask::Symbol(symbol) => Question::Study(symbol.clone()),
            Ask::Source { at, chosen } => Question::Resolve {
                at: at.clone(),
                chosen: chosen.clone(),
                standing: self.shown.as_ref().map(|shown| shown.studied.clone()),
                objects: open.to_vec(),
                recent: recent_symbols(self.shown.as_ref(), visits),
            },
        };
        self.pending = Some(Pending::asked(ask.clone()));
        Some(question)
    }

    /// The question the worker is working on, and nothing while it is idle.
    pub(crate) fn waiting(&self) -> Option<&Ask> {
        self.pending.as_ref().map(|pending| &pending.ask)
    }

    /// [`SLOW_ANALYSIS`] has passed since `ask` was sent. Whether anything changed, so
    /// the caller writes only then ([`write_if`]): a question answered since, or one the
    /// reader has moved on from, is nothing to say the app is still working on.
    fn slowed(&mut self, ask: &Ask) -> bool {
        let Some(pending) = self.pending.as_mut().filter(|held| held.ask == *ask) else {
            return false;
        };
        if pending.slow {
            return false;
        }
        pending.slow = true;
        true
    }
}

/// Everything worked out about one symbol, in one value because it is worked out in one
/// go.
#[derive(Clone)]
pub(crate) struct Studied {
    /// Which symbol this is the analysis of.
    pub(crate) symbol: Symbol,
    /// [`None`] for a symbol with no bytes to decode at all; the pane says so.
    pub(crate) assembly: Option<Arc<Assembly>>,
    pub(crate) lanes: Arc<Lanes>,
    pub(crate) lines: SymbolLines,
}

impl PartialEq for Studied {
    fn eq(&self, other: &Self) -> bool {
        self.symbol == other.symbol
            && same_arc(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Studied {
    /// Decode the symbol and build the object's DWARF context.
    pub(crate) fn new(symbol: Symbol) -> Studied {
        let assembly = symbol.data.assembly(&symbol.object);
        Studied::with_assembly(symbol, assembly)
    }

    /// The rest of the analysis over a listing already decoded -- the section view
    /// decodes a stretch through the crate's listing, which is the same decode, and must
    /// not pay for it twice.
    pub(crate) fn with_assembly(symbol: Symbol, assembly: Option<Arc<Assembly>>) -> Studied {
        let lanes = match &assembly {
            Some(assembly) => Arc::new(Lanes::new(&assembly.edges, assembly.instructions.len())),
            None => Lanes::none(),
        };
        let lines = SymbolLines::new(&symbol);

        Studied {
            symbol,
            assembly,
            lanes,
            lines,
        }
    }
}

impl Studied {
    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    pub(crate) fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.info.as_ref()?;
        // `get` and not an index: a row's neighbour below can be past the listing.
        let address = self.assembly.as_ref()?.instructions.get(index)?.address;
        let row = lines.row_at(address)?;
        Some(LinePos {
            file: lines.files().get(row.file?)?.clone(),
            line: row.line?,
        })
    }

    /// Whether the instruction at `index` is the same place as a line of the source pane's
    /// picked-out run `pair`: compiled from that file, on one of those lines. The **one**
    /// pairing rule -- both listings light rows with it and both scroll to a row it picks
    /// out, and a second spelling of it would light one row and scroll to another. One
    /// source line is many instructions and every one of them is lit, so this asks each
    /// row's own position rather than looking for the first match. An instruction the debug
    /// info places nowhere is never paired.
    pub(crate) fn paired(&self, index: usize, pair: &Picked) -> bool {
        let Some(at) = self.position(index) else {
            return false;
        };
        pair.file.as_ref() == Some(&at.file)
            && (at.line as usize)
                .checked_sub(1)
                .is_some_and(|row| pair.chars.contains_row(row))
    }

    /// The first instruction of this symbol paired with `pair`, or [`None`] where none is:
    /// what a pane owing that run a scroll reveals, once `Lanes` has made a listing row of
    /// it.
    pub(crate) fn first_paired(&self, pair: &Picked) -> Option<usize> {
        let assembly = self.assembly.as_ref()?;
        (0..assembly.instructions.len()).find(|&index| self.paired(index, pair))
    }

    /// The positions the instructions drawn in the listing rows `rows` were compiled
    /// from, `base` being the listing row this symbol's first instruction row is drawn
    /// at. One per instruction placed somewhere, in listing order; a run of rows that is
    /// separators alone answers nothing.
    pub(crate) fn places(&self, rows: RangeInclusive<usize>, base: usize) -> Vec<LinePos> {
        let first = rows.start().saturating_sub(base);
        let Some(last) = rows.end().checked_sub(base) else {
            return Vec::new();
        };
        let Some(indices) = self.lanes.instructions_in(first..=last) else {
            return Vec::new();
        };
        indices.filter_map(|index| self.position(index)).collect()
    }
}

/// What DWARF says about the selected symbol's instructions, and where in them the Source
/// pane opens: which of the files it names, and which line of that file. Both are carried
/// here, beside the info they come from, so none of the three can disagree while the
/// worker is still running.
#[derive(Clone)]
pub(crate) struct SymbolLines {
    pub(crate) info: Option<Arc<LineInfo>>,
    /// The file the symbol's first instruction was compiled from, falling back to the
    /// first file its rows name.
    pub(crate) file: Option<Arc<str>>,
    /// The line of that file the symbol opens at -- where the Source pane lands a tab it
    /// is showing for the first time, a symbol's own lines being what selecting it asked
    /// for. `None` where the opening row names no line at all, and the pane then opens at
    /// the top of the file as it did before.
    pub(crate) line: Option<u32>,
}

impl PartialEq for SymbolLines {
    fn eq(&self, other: &Self) -> bool {
        // The file compares by its text, not by pointer, for the reason `LinePos` does:
        // two `LineInfo`s naming one file hold two `Arc<str>`s of it.
        same_arc(&self.info, &other.info) && self.file == other.file && self.line == other.line
    }
}

impl SymbolLines {
    fn new(symbol: &Symbol) -> SymbolLines {
        let info = symbol.data.line_info(&symbol.object);
        // The row the symbol's first instruction was compiled from, falling back to the
        // first row that names a file at all: a prologue DWARF places on no line leaves
        // `row_at` with nothing to say. **One row for both answers**, so the line the
        // pane opens at is a line of the file it is showing and not of another.
        let opening = info.as_ref().and_then(|info| {
            info.row_at(symbol.data.address)
                .filter(|row| row.file.is_some())
                .or_else(|| info.rows().iter().find(|row| row.file.is_some()))
        });
        let file = info.as_ref().and_then(|info| {
            opening
                .and_then(|row| row.file)
                .and_then(|file| info.files().get(file))
                .or_else(|| info.files().first())
                .cloned()
        });
        let line = opening.and_then(|row| row.line);

        SymbolLines { info, file, line }
    }

    /// The checksum the debug info recorded for `file`, one of the files these rows name, or
    /// [`None`] when it names no such file or recorded none for it. Looked up by the name
    /// the pane is showing rather than carried per file, so a landed run's file and the
    /// symbol's own are answered the same way.
    pub(crate) fn hash_for(&self, file: &str) -> Option<analysis::SourceHash> {
        let info = self.info.as_ref()?;
        let index = info.files().iter().position(|named| **named == *file)?;
        info.hash_of(index)
    }
}

/// What [`use_analysis_with`] needs of the question: a **read**, which subscribes the
/// effect to it, and a **peek**, which does not -- the effect must wake on a change of
/// question and must not wake on its own writes, so the two cannot collapse into one.
///
/// A trait so the hook can be driven by [`Asked`] in the app and by a plain state in the
/// tests.
pub(crate) trait ReadsAsk: Copy + 'static {
    fn read_ask(self) -> Option<Ask>;
    fn peek_ask(self) -> Option<Ask>;
}

/// The question the app asks, out of the two states it is a function of.
///
/// **Not a `Memo`**: [`Active`] is already one, recomputed by a task woken on a notify, so
/// a memo over it would be two beats behind -- and the lag is not only a rendering matter,
/// [`ReadsAsk::peek_ask`] being what decides whether an answer that has landed is still
/// wanted.
#[derive(Clone, Copy)]
pub(crate) struct Asked {
    pub(crate) active: Memo<Option<Entry>>,
    pub(crate) driven: State<Driven>,
}

impl ReadsAsk for Asked {
    fn read_ask(self) -> Option<Ask> {
        let active = self.active.read();
        ask(active.as_ref(), &self.driven.read())
    }

    fn peek_ask(self) -> Option<Ask> {
        let active = self.active.peek();
        ask(active.as_ref(), &self.driven.peek())
    }
}

impl ReadsAsk for State<Option<Ask>> {
    fn read_ask(self) -> Option<Ask> {
        self.read().clone()
    }

    fn peek_ask(self) -> Option<Ask> {
        self.peek().clone()
    }
}

/// Where the reader has been, newest first, with the symbol on screen at its head --
/// which is what keeps reading down the lines of a generic function inside one
/// instantiation, nothing being recorded between two clicks in one.
fn recent_symbols(shown: Option<&Shown>, visits: &Visits) -> Vec<Symbol> {
    shown
        .map(|shown| shown.studied.symbol.clone())
        .into_iter()
        .chain(
            visits
                .entries()
                .iter()
                .filter_map(|entry| entry.symbol().cloned()),
        )
        .collect()
}

/// Work the question out on the app's one worker thread and hand the answer to the panes
/// through [`Analysis`], and a locate's to the Locations panel through `located`.
/// Requests supersede: the queue is drained to its newest entry of each kind
/// ([`newest`]), so what the reader clicked past is dropped before it is started.
///
/// **One worker and not two**, now that there are two kinds of question: `DebugInfo::index`
/// is a `OnceLock` and the source index's build holds the same backend mutex `line_info`
/// and `extent` take, so a second thread asking a source question would block in
/// `get_or_init` rather than race usefully -- and two producers writing one [`Analyzed`]
/// would break the single `shown`/`pending` the panes read.
///
/// The work itself is an argument so a test can hold it still: superseding is a race by
/// construction and cannot be asserted against a worker that answers as fast as it is
/// asked.
pub(crate) fn use_analysis_with(
    asked: impl ReadsAsk,
    objects: State<Vec<Arc<Object>>>,
    beside: State<Option<Arc<Object>>>,
    visits: State<Visits>,
    mut analysis: State<Analyzed>,
    located: State<Located>,
    coded: State<Coded>,
    reading: State<Reading>,
    window: State<Option<CodeAsk>>,
    work: impl Fn(Question) -> Answer + Send + 'static,
) {
    // The worker and the task that listens to it, started once and never restarted.
    //
    // A `std::thread` and not a spawned task: this is seconds of decoding, DWARF parsing
    // and index building, and freya's executor is the UI thread.
    let requests = use_worker(
        "the analysis worker",
        // Everything the reader clicked past while the last job ran, dropped without
        // being started rather than after the fact.
        |question, queued, _| newest(question, std::iter::from_fn(queued)),
        move |question| Some(work(question)),
        // Each answer is judged by the state it lands in and written only where that
        // state says it changed something: the rules are the four types' and not this
        // closure's ([`write_if`]).
        move |answer, _| match answer {
            Answer::Listing { ask, studied } => {
                // The question being asked *now*, which is what an answer is kept for.
                let wanted = asked.peek_ask();
                let open = objects.peek().clone();
                write_if(analysis, |next| {
                    next.take(ask, studied, wanted.as_ref(), &open)
                });
            }
            Answer::Code { ask, code, decoded } => {
                // Taken whenever it is about the object on screen -- a decoded stretch is
                // never stale, see `Reading::take` -- and never out of a binary closed
                // since it was asked for, `Shown::still_open`'s rule once more. Held by
                // the app and not open in the project: a pad's program is neither, and
                // `holding` is the one rule for the two.
                if !holding(&objects.peek(), &beside.peek(), &ask.object) {
                    return;
                }
                write_if(reading, |next| next.take(&ask, code, decoded));
            }
            Answer::Marked { file, lines, over } => {
                write_if(coded, |next| next.take(file, lines, over));
            }
            Answer::Located { query, symbols } => {
                let open = objects.peek().clone();
                write_if(located, |next| next.take(query, symbols, &open));
            }
        },
    );

    // The window question: what the section view wants next, asked once. Reading the
    // window subscribes this to it; the reading it writes is peeked, so it cannot wake
    // itself. Whether the ask is one to send is the reading's ([`Reading::asking`]).
    let requests_for_code = requests.clone();
    use_side_effect(move || {
        let wanted = window.read().clone();
        let Some(ask) = wanted else {
            return;
        };
        if write_if(reading, |next| next.asking(&ask)) {
            requests_for_code.send(Question::Code(ask));
        }
    });

    let requests_for_locate = requests.clone();
    let requests_for_marks = requests.clone();
    use_side_effect(move || {
        // Reading subscribes this to the question; the state it writes is `peek`ed, so it
        // cannot wake itself.
        let current = asked.read_ask();
        // **Read and not peek**, unlike the visits below: a question asked of a different
        // set of objects is a different question, so this effect has to run again when
        // they change. For a symbol it costs nothing -- the run hits the already-in-hand
        // branch of `Analyzed::asked` and answers with no question.
        let open: Vec<Arc<Object>> = objects.read().clone();

        // The whole of the rule is the state's ([`Analyzed::asked`]); what is left here
        // is the writing and the sending. The visits are **peeked**, not read: the
        // ranking is an input to an answer and a visit must not re-ask a question that
        // has been answered.
        let mut next = analysis.peek().clone();
        let question = next.asked(current.as_ref(), &open, &visits.peek());
        analysis.set_if_modified(next);

        let (Some(ask), Some(question)) = (current, question) else {
            return;
        };
        requests.send(question);

        // The wait, started by the request and never polled.
        spawn(async move {
            Timer::after(SLOW_ANALYSIS).await;
            write_if(analysis, |held| held.slowed(&ask));
        });
    });

    // The locate question, as the finder's and the search's are asked: **a memo and not a
    // read**. A fold of the panel's rows and a binary closed under an older answer are
    // writes to this state too, and an effect reading the state would send the question
    // again for each of them while the worker is still on it -- a second run of seconds of
    // work, under the lock every listing question waits on. The memo recomputes for those
    // writes and wakes nothing, the question being unchanged.
    //
    // A question about a name's uses is the language server's, asked where it was pressed
    // and answered into the same panel; nothing here can answer it.
    let locating = use_memo(move || {
        located
            .read()
            .pending()
            .filter(|query| query.symbols_wanted().is_some())
            .cloned()
    });

    // The objects are **peeked** where the listing reads them: an answer stands until
    // replaced, so a file opened afterwards is not searched until the line is asked again
    // -- the panel says which objects it answered for by saying when. A file closed
    // afterwards is the effect below.
    use_side_effect(move || {
        // Reading the memo subscribes this to the question.
        let Some(query) = locating.read().clone() else {
            return;
        };
        requests_for_locate.send(Question::Locate {
            query,
            objects: objects.peek().clone(),
        });
    });

    // The gutter marks' question, asked for whichever file the Source pane last said it
    // was showing. The objects are **read** and not peeked, unlike the locate's: an
    // answer here is a set of bare line numbers with nothing in it to sweep for a closed
    // binary, so the way it stays true is to drop it and ask again whenever the open
    // objects change -- which a load finishing also is, and which is what puts marks in
    // a gutter that was drawn before its binary had been read.
    use_side_effect(move || {
        let open = objects.read().clone();
        // Read and not peeked: the pane writing the file it moved to is the other half
        // of what wakes this. Bound before the send, the guard being a read.
        let pending = coded.read().pending(&open).cloned();
        let Some(file) = pending else {
            return;
        };
        requests_for_marks.send(Question::Marks {
            file,
            objects: open,
        });
    });

    // A closed binary takes its locations with it, at once and whatever the panel is
    // doing: `Found::retain_open` answers whether anything went, so a load that added an
    // object writes nothing.
    use_side_effect(move || {
        let open = objects.read().clone();
        write_if(located, |next| next.retain_open(&open));
    });
}

#[cfg(test)]
mod tests;

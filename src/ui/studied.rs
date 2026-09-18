//! What the two panes draw, and the rules an answer is judged by.
//!
//! The listing is one of the four jobs the analysis worker (`analyzed.rs`) does. This is
//! its state: the question [`Ask`] the panes put, the [`Studied`] that answers it, the
//! [`Analyzed`] the two are held in, and [`Showing`], the one decision about what a pane
//! draws right now.
//!
//! Three rules live here and nowhere else. [`Analyzed::take`] says which answers are
//! kept: the one asked for now, out of a binary still open. [`Analyzed::asked`] says what
//! is worth asking, a listing already in hand being retagged rather than worked out
//! again. And [`Studied::paired`] says when an instruction and a source line are the same
//! place, which is what lights a row in one pane from a run picked out in the other.

use super::*;
use crate::counter;
use std::borrow::Cow;

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
/// row, by the run of rows a listing change drops, and by the two rules below that ask
/// whether the listing on screen is the reader's own.
pub(crate) fn asked_of(ask: &Ask) -> Document {
    match ask {
        Ask::Symbol(symbol) => Document::Symbol(symbol.clone()),
        Ask::Source { at, .. } => Document::Source(at.file.clone()),
    }
}

/// Whether the listing that is up is one the tab asking `ask` may be left showing: what
/// says a question has something of the reader's own on screen already.
///
/// Two ways it can be, and the second is not a widening but a repair. The listing is
/// tagged with the question it answers *now*, which a retag moves onto another tab --
/// the symbol tab a file's line resolved to is the same listing, and opening it retags
/// rather than decodes again. Coming back to the file, the tag then says the symbol's
/// tab, and the listing the file tab worked out would be taken down by the first line of
/// it holding no code. So a source line keeps a listing its own **file** compiled into,
/// however the listing is tagged; what that leaves out is the case the rule is for, a
/// function of another file left up under a tab that never asked for it.
fn keeps_listing(shown: Option<&Shown>, ask: &Ask) -> bool {
    let Some(shown) = shown else {
        return false;
    };
    if asked_of(&shown.ask) == asked_of(ask) {
        return true;
    }
    match ask {
        Ask::Source { at, .. } => shown.studied.lines.names(&at.file),
        Ask::Symbol(_) => false,
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
        Document::Symbol(symbol) => Some(Ask::Symbol(symbol.clone())),
        Document::Object(_) | Document::Code(_) => None,
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
#[derive(Default, PartialEq)]
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

impl Clone for Analyzed {
    /// Hand-written only to count. A whole answer -- a listing, the symbol it is of, and
    /// the questions either side of it -- so the panes read it through a guard and copy
    /// none of it, and [`copies`] is what says a render made no copy.
    fn clone(&self) -> Analyzed {
        #[cfg(test)]
        COPIES.set(COPIES.get() + 1);
        Analyzed {
            shown: self.shown.clone(),
            answered: self.answered.clone(),
            pending: self.pending.clone(),
        }
    }
}

counter!(
    /// Test-only: how many whole answers this thread has copied, which is what says a
    /// render copied none.
    pub(crate) fn copies() = COPIES
);

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
    /// Nothing to draw and a word for why. A `Cow`: four of the five are fixed
    /// sentences, and the fifth names the line that came to nothing.
    Message(Cow<'static, str>),
    /// A wait too short to name, with no previous listing to leave up.
    Nothing,
}

impl Analyzed {
    /// What the panes draw, one answer for both of them. The **document** and not a word
    /// from the caller, so that this stays the one place either pane decides what it is
    /// drawing.
    ///
    /// **The order of the arms is the mechanism**: a listing beats a short wait, so a
    /// click never flashes the pane empty; a wait past [`SLOW_ANALYSIS`] beats a listing
    /// it may take down ([`keeps_listing`]), so a function of a file nobody here is
    /// reading is not left up under the next tab, and loses to one it may not -- reading
    /// down a file is a question per line, and a word that displaces the listing between
    /// two of them is the pane blinking for a keypress, over a listing the bar above it
    /// names honestly; and a **sentence** is left up over a wait exactly as a listing is,
    /// for the same reason and no other -- clicking down a file's comments and braces is
    /// one sentence after another, and a pane that blanks between two of them flashes on
    /// every click. The line it names is the line before this one for as long as the
    /// answer takes, which is what a listing left up is too.
    pub(crate) fn showing(&self, document: &Document) -> Showing<'_> {
        match (&self.shown, &self.pending) {
            (shown, Some(pending))
                if pending.slow && !keeps_listing(shown.as_ref(), &pending.ask) =>
            {
                Showing::Message(Cow::Borrowed("Analysing..."))
            }
            (Some(shown), _) => Showing::Listing(shown),
            // Answered with no symbol at all, whether or not the next question is out
            // yet. Only a source line can be, and the message names it: the answer
            // outlives the click, so which line came to nothing is not otherwise on
            // screen.
            (None, _) if self.answered.is_some() => Showing::Message(match &self.answered {
                Some(Ask::Source { at, .. }) => {
                    Cow::Owned(format!("No code compiled from {}", at.spell()))
                }
                _ => Cow::Borrowed("No code compiled from this line"),
            }),
            // Asked with nothing behind it: the first question of a tab, which has no
            // sentence to leave up either.
            (None, Some(_)) => Showing::Nothing,
            (None, None) => Showing::Message(Cow::Borrowed(match document {
                Document::Object(_) | Document::Symbol(_) => "No symbol selected",
                Document::Source(_) => "Click a source line",
                // The listing beside this asks nothing; its source side follows the
                // instruction picked out in it.
                Document::Code(_) => "Click an instruction",
            })),
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
        // Each field says whether it moved, so that an answer the effect has already
        // settled -- a listing retagged while this one was in flight -- costs no render.
        let landed = studied.map(|studied| Shown {
            ask: ask.clone(),
            studied,
        });
        let landed = landed.filter(|shown| shown.still_open(open));

        let mut changed = false;
        if self.waiting() == Some(&ask) {
            self.pending = None;
            changed = true;
        }
        changed |= put(&mut self.answered, Some(ask.clone()));

        match landed {
            Some(shown) => changed |= put(&mut self.shown, Some(shown)),
            // A question that named no symbol leaves the listing that is up -- the click
            // lights no pair in it and nothing else, which is what says it landed nowhere
            // -- but **only one this line may be left looking at** ([`keeps_listing`]),
            // or a line holding no code would leave a function of another file on screen
            // for good.
            None => {
                if !keeps_listing(self.shown.as_ref(), &ask) {
                    changed |= self.shown.take().is_some();
                }
            }
        }

        changed
    }

    /// Bring this up to date with the question `ask`, asked over the binaries `open`, and
    /// answer with the question the worker is owed -- [`None`] where it is owed none --
    /// and whether anything changed, so the effect writes only then ([`write_if`]).
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
    ) -> (Option<Question>, bool) {
        let Some(ask) = ask else {
            // Not a place with a listing: nothing to work out and nothing to wait for.
            // Anything still in flight is dropped when it lands.
            let changed = *self != Analyzed::default();
            *self = Analyzed::default();
            return (None, changed);
        };

        // Dropped here rather than by `close_binary`, so that a close, a rebuild and a
        // project switch are one line instead of three.
        let mut changed = false;
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| !shown.still_open(open))
        {
            *self = Analyzed::default();
            changed = true;
        }

        let held = self.shown.as_ref().is_some_and(|shown| shown.answers(ask))
            || self.answered.as_ref() == Some(ask);
        if held {
            // Retagged, so the same listing is not asked for again under its new
            // question, and so nothing goes on saying it is waiting.
            if let Some(shown) = self.shown.as_mut().filter(|shown| shown.answers(ask)) {
                changed |= put(&mut shown.ask, ask.clone());
            }
            changed |= put(&mut self.answered, Some(ask.clone()));
            changed |= self.pending.take().is_some();
            return (None, changed);
        }
        if self.waiting() == Some(ask) {
            return (None, changed);
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
        (Some(question), true)
    }

    /// The question the worker is working on, and nothing while it is idle.
    pub(crate) fn waiting(&self) -> Option<&Ask> {
        self.pending.as_ref().map(|pending| &pending.ask)
    }

    /// [`SLOW_ANALYSIS`] has passed since `ask` was sent. Whether anything changed, so
    /// the caller writes only then ([`write_if`]): a question answered since, or one the
    /// reader has moved on from, is nothing to say the app is still working on.
    pub(crate) fn slowed(&mut self, ask: &Ask) -> bool {
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

/// Write `value` into `slot`, and whether that changed it: how [`Analyzed::take`] and
/// [`Analyzed::asked`] say what they did without a copy of the whole state to compare.
fn put<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
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
        let lanes = Lanes::over(assembly.as_deref());
        let lines = SymbolLines::new(&symbol, assembly.as_deref());

        Studied {
            symbol,
            assembly,
            lanes,
            lines,
        }
    }
}

impl Studied {
    /// How many bytes of code this listing was decoded over: the extent the crate worked
    /// out for the symbol, and 0 for a symbol with nothing to decode.
    ///
    /// **Read off the answer and never asked again.** `SymbolData::extent` is the crate's
    /// most expensive decision -- an unwind lookup, or a DWARF DIE walk under the debug
    /// backend's mutex -- and the bar over the pane prints this number in a render.
    pub(crate) fn extent(&self) -> u64 {
        self.assembly.as_ref().map_or(0, |assembly| {
            assembly.range.end.saturating_sub(assembly.range.start)
        })
    }

    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    pub(crate) fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.info.as_ref()?;
        // `get` and not an index: a row's neighbour below can be past the listing.
        let address = self.assembly.as_ref()?.instructions.get(index)?.address;
        let row = lines.row_at(address)?;
        Some(LinePos {
            file: lines.file(row.file?)?.clone(),
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

    /// The instructions this listing draws in the listing rows `rows`, `base` being the
    /// listing row its first instruction is drawn at. The **one** place a run of listing
    /// rows is crossed into instruction indices with a base: two answers about the same
    /// run cannot then land a row apart.
    ///
    /// The start saturates and the end is checked. A run opening above this listing
    /// starts at its first row; one ending above it holds none of it at all.
    /// [`Lanes::instructions_in`] settles the ends from there -- a separator opening the
    /// run is inside it, one closing it is not.
    fn instructions_in(
        &self,
        rows: RangeInclusive<usize>,
        base: usize,
    ) -> Option<RangeInclusive<usize>> {
        let first = rows.start().saturating_sub(base);
        let last = rows.end().checked_sub(base)?;
        self.lanes.instructions_in(first..=last)
    }

    /// The positions the instructions drawn in the listing rows `rows` were compiled
    /// from, `base` being the listing row this symbol's first instruction row is drawn
    /// at. One per instruction placed somewhere, in listing order; a run of rows that is
    /// separators alone answers nothing.
    pub(crate) fn places(&self, rows: RangeInclusive<usize>, base: usize) -> Vec<LinePos> {
        let Some(indices) = self.instructions_in(rows, base) else {
            return Vec::new();
        };
        indices.filter_map(|index| self.position(index)).collect()
    }

    /// The edges starting or ending at an instruction drawn in the listing rows `rows`,
    /// `base` as in [`Studied::places`]: what the gutter lights for the run. Empty for a
    /// run that is separators alone.
    pub(crate) fn touching(&self, rows: RangeInclusive<usize>, base: usize) -> Vec<PlacedEdge> {
        self.instructions_in(rows, base)
            .map(|indices| self.lanes.touching_any(indices))
            .unwrap_or_default()
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
    /// The lines of the rows `assembly` holds, asked over the very range it was decoded
    /// over: the extent behind that range is the most expensive answer in the crate, and it
    /// has been paid for once already. A symbol with nothing to decode has no range and so
    /// no lines -- there are no rows to pair them with.
    fn new(symbol: &Symbol, assembly: Option<&Assembly>) -> SymbolLines {
        let info = symbol
            .data
            .section
            .as_ref()
            .zip(assembly)
            .and_then(|(section, assembly)| {
                symbol.object.line_info(section, assembly.range.clone())
            });
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
                .and_then(|file| info.file(file))
                .or_else(|| info.files().next())
                .cloned()
        });
        let line = opening.and_then(|row| row.line);

        SymbolLines { info, file, line }
    }

    /// Whether the symbol these are of has code from `file`: the file it opens at, or
    /// any of the files its rows name -- code inlined into it from a header is the
    /// symbol's own as much as the body is. Compared by text, as every file the UI
    /// passes around is.
    pub(crate) fn names(&self, file: &str) -> bool {
        self.file.as_deref() == Some(file)
            || self
                .info
                .as_ref()
                .is_some_and(|info| info.files().any(|named| **named == *file))
    }

    /// The checksum the debug info recorded for `file`, one of the files these rows name, or
    /// [`None`] when it names no such file or recorded none for it. Looked up by the name
    /// the pane is showing rather than carried per file, so a landed run's file and the
    /// symbol's own are answered the same way.
    pub(crate) fn hash_for(&self, file: &str) -> Option<analysis::SourceHash> {
        self.info.as_ref()?.hash_for(file)
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

#[cfg(test)]
mod tests;

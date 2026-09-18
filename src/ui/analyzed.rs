//! The app's one analysis worker, and the questions it is asked.
//!
//! Four kinds of job go to the one worker: a **listing** -- the symbol the panes draw,
//! named outright or resolved from a source line -- a **window** of an object's code for
//! the section view, a **locate**, every symbol a line or a function was compiled into,
//! for the Locations panel, and **marks**, the lines of one file that produced code at
//! all, for the Source pane's gutter. They supersede separately: the queue is drained to
//! the newest of *each*, since a reader who asked for a line's locations and then clicked
//! a symbol wants both answers.
//!
//! Only the listing is asked for here. Each of the other three is asked beside the state
//! it is about, on the worker this hook hands back: `use_code_asks` (`reading.rs`),
//! `use_locate_asks` (`locations.rs`) and `use_mark_asks` (`coded.rs`). Every answer is
//! still taken in the one closure, there being one worker and one [`Answer`].
//!
//! The listing's own state is `studied.rs`, beside the panes it is drawn in. What is left
//! here is the asking: [`Asked`], the two states the question is a function of, and the
//! effect that sends it.

use super::*;

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
///
/// `JobKind` and not `Kind`: the prelude's [`Kind`] is a document's, which a type of that
/// name here would put out of reach.
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Listing,
    Code,
    Locate,
    Marks,
}

impl Question {
    fn job_kind(&self) -> JobKind {
        match self {
            Question::Study(_) | Question::Resolve { .. } => JobKind::Listing,
            Question::Code(_) => JobKind::Code,
            Question::Locate { .. } => JobKind::Locate,
            Question::Marks { .. } => JobKind::Marks,
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
        match question.job_kind() {
            JobKind::Listing => listing = Some(question),
            JobKind::Code => code = Some(question),
            JobKind::Locate => locate = Some(question),
            JobKind::Marks => marks = Some(question),
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

/// The question the app asks, out of the two states it is a function of.
///
/// **Not a `Memo`**: [`Active`] is already one, recomputed by a task woken on a notify, so
/// a memo over it would be two beats behind -- and the lag is not only a rendering matter,
/// [`Asked::peek_ask`] being what decides whether an answer that has landed is still
/// wanted.
#[derive(Clone, Copy)]
pub(crate) struct Asked {
    pub(crate) active: Memo<Option<Entry>>,
    pub(crate) driven: State<Driven>,
}

impl Asked {
    /// The question, **read**, which subscribes whoever asks to it. What
    /// [`use_analysis_with`]'s effect wakes on.
    pub(crate) fn read_ask(self) -> Option<Ask> {
        let active = self.active.read();
        ask(active.as_ref(), &self.driven.read())
    }

    /// The question, **peeked**, which does not subscribe: the effect must not wake on
    /// its own writes, so the two cannot collapse into one.
    pub(crate) fn peek_ask(self) -> Option<Ask> {
        let active = self.active.peek();
        ask(active.as_ref(), &self.driven.peek())
    }
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
///
/// **Only the listing question is asked here**; the other three are asked beside their
/// states ([`use_code_asks`], [`use_locate_asks`], [`use_mark_asks`]), which is what the
/// way to ask is handed back for. It stays not because its state is here -- that is
/// `studied.rs` -- but because it is not [`use_asking`]'s shape, for the reasons the
/// effect below gives. Every answer is taken here all the same, there being one worker
/// and one [`Answer`].
pub(crate) fn use_analysis_with(
    asked: Asked,
    objects: State<Vec<Arc<Object>>>,
    sectioned: Sectioned,
    visits: State<Visits>,
    analysis: State<Analyzed>,
    located: State<Located>,
    coded: State<Coded>,
    showing: State<Option<Arc<str>>>,
    work: impl Fn(Question) -> Answer + Send + 'static,
) -> Requests<Question> {
    let (beside, reading) = (sectioned.beside, sectioned.reading);
    // The worker and the task that listens to it, started once and never restarted.
    //
    // A `std::thread` and not a spawned task: this is seconds of decoding, DWARF parsing
    // and index building, and freya's executor is the UI thread.
    let requests = use_worker(
        "the analysis worker",
        // Everything the reader clicked past while the last job ran, dropped without
        // being started rather than after the fact.
        |question, queued| newest(question, queued),
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
                // The file the pane is showing *now*, which is what an answer is kept
                // for -- the listing's rule, and `Coded::take`'s to apply.
                let showing = showing.peek().clone();
                write_if(coded, |next| next.take(showing.as_ref(), file, lines, over));
            }
            Answer::Located { query, symbols } => {
                let open = objects.peek().clone();
                write_if(located, |next| next.take(query, symbols, &open));
            }
        },
    );

    let asking = requests.clone();
    // The listing question, and the one of the four that is **not** [`use_asking`]'s:
    // what is pending and the mark for it are one call ([`Analyzed::asked`] answers with
    // the question and records it in the same pass), and the objects it is asked of have
    // no equality for a memo to compare.
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
        let mut question = None;
        write_if(analysis, |held| {
            let (asked, changed) = held.asked(current.as_ref(), &open, &visits.peek());
            question = asked;
            changed
        });

        let (Some(ask), Some(question)) = (current, question) else {
            return;
        };
        asking.send(question);

        // The wait, started by the request and never polled.
        spawn(async move {
            Timer::after(SLOW_ANALYSIS).await;
            write_if(analysis, |held| held.slowed(&ask));
        });
    });

    requests
}

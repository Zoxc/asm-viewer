//! What the worker has decoded of an object's code for the section view, and the window
//! of it the view is asking for next.
//!
//! A listing of a whole object's code is read in **windows**: the skeleton is free
//! (`CodeListing`, `agents/Analysis.md`) and every stretch of it is decoded only when the
//! reader is near it. The answers land here, in [`Reading`], and never in [`Analyzed`]:
//! that state is one symbol's, and everything reading it -- the symbol bar, the source
//! side, the Locations panel -- would have to learn a second shape. What is held is
//! **bounded**: a stretch farther than [`KEEP`] from the last window is let go when an
//! answer lands, so a scroll through the app's own binary does not pile up its whole
//! `.text`; and it is the view's answer rather than a cache, dropped whole when the reader
//! leaves the object's tab and decoded again when they come back, which is
//! `Analyzed`'s own rule for a symbol.

use super::*;
use crate::section::Body;
use analysis::{CodeListing, Gap};
use std::collections::BTreeMap;

/// How many stretches the worker decodes of one ask before answering. The queue is
/// drained to its newest question only *between* jobs, so a window decoded whole would
/// hold a symbol click behind every stretch in it; a chunk at a time keeps that wait to a
/// few functions, and the view asks for the rest once the chunk has landed.
pub(crate) const CHUNK: usize = 8;

/// How far from the last window a held stretch may be, in stretches, before it is let go.
/// Well past the view's buffer, so filling the buffer never evicts it.
pub(crate) const KEEP: usize = 512;

/// The reading of one object's code, as everything that touches it is given it: what has
/// been decoded, what is wanted next, whose listing it is when no tab's, and the rows the
/// view last built.
///
/// **One bundle and not four contexts**, because the four are one mechanism and are read
/// together: [`use_reading_of`] keeps the first three in step, an answer is taken into two
/// of them ([`use_analysis_with`]), and the rows mean nothing apart from the reading they
/// were counted from. That last is the rule [`Sectioned::rows_of`] is -- written once here
/// rather than checked again at each of the three readers.
#[derive(Clone, Copy)]
pub(crate) struct Sectioned {
    /// The decoded stretches of the object whose code is on screen.
    pub(crate) reading: State<Reading>,
    /// The stretches the view wants next. Its own state and not a field of [`Reading`]:
    /// the effect working out the window reads what is held and would wake itself on
    /// writing beside it.
    pub(crate) window: State<Option<CodeAsk>>,
    /// The object a listing that is **no document tab** is drawing, or `None` while there
    /// is none.
    ///
    /// The Scratchpad's pane claims it while it is mounted and lets go on the way out,
    /// the way a pane registers its focusable box (`use_tab_keyboard`,
    /// `src/ui/keyboard.rs`). It is a claim and not a question asked of the pads, because
    /// the pane drawing the listing is the only thing that knows there is one: a general
    /// mechanism asking would have to know about pages and pads, and would hold a
    /// skeleton for a pad's program while the reader sat on the Settings page.
    pub(crate) beside: State<Option<Arc<Object>>>,
    /// The rows the section view is drawing: [`None`] until the skeleton has come, and
    /// rebuilt by the view's place-keeping effect with every answer. One slot and not a
    /// map, since one code listing is mounted at a time; which listing it is about is the
    /// [`Reading`] the rows were counted from, and asking for them is
    /// [`Sectioned::rows_of`]. Here and not in the view because the Source pane beside an
    /// object's code reads them too, to find the lines the picked-out instructions were
    /// compiled from.
    pub(crate) rows: State<Option<Arc<Built>>>,
}

impl Sectioned {
    /// The rows on screen, and only where they are `object`'s: one slot holds the rows of
    /// whichever listing is mounted, and for the pass between a switch and the rebuild it
    /// holds the last one's.
    ///
    /// Judged by the [`Built`]'s own reading and not by the state, which the rebuild is a
    /// pass behind. Reading the rows is what redraws the caller as answers land.
    pub(crate) fn rows_of(&self, object: &Arc<Object>) -> Option<Arc<Built>> {
        self.rows
            .read()
            .clone()
            .filter(|built| built.reading.is_about(object))
    }

    /// The same, peeked: for a handler or an effect that must not be woken by a window
    /// decoding.
    pub(crate) fn peek_rows_of(&self, object: &Arc<Object>) -> Option<Arc<Built>> {
        self.rows
            .peek()
            .clone()
            .filter(|built| built.reading.is_about(object))
    }
}

/// The reading as a component sees it.
pub(crate) fn use_sectioned() -> Sectioned {
    use_consume::<Sectioned>()
}

/// Claim `object` as the listing that is no tab, for as long as this scope is mounted.
/// See [`Sectioned::beside`].
pub(crate) fn use_code_beside(mut beside: State<Option<Arc<Object>>>, object: &Arc<Object>) {
    // By pointer identity and written from the render, so a rebuild's new object is
    // claimed the moment the pane draws it. `set_if_modified` would compare `Option`s by
    // value, which for an object is every byte of the file.
    let claimed = beside
        .peek()
        .as_ref()
        .is_some_and(|held| Arc::ptr_eq(held, object));
    if !claimed {
        beside.set(Some(object.clone()));
    }
    let object = object.clone();
    use_drop(move || {
        let mine = beside
            .peek()
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &object));
        if mine {
            beside.set(None);
        }
    });
}

/// Whether the app is still holding `object`: one of the open binaries, or the listing
/// that is no tab.
///
/// **A pad's program is deliberately not one of the binaries** -- it is the pad's and not
/// the project's -- so this is the one place that says the two together, and an answer
/// about either is judged by one rule.
pub(crate) fn holding(
    objects: &[Arc<Object>],
    beside: &Option<Arc<Object>>,
    object: &Arc<Object>,
) -> bool {
    objects.iter().any(|open| Arc::ptr_eq(open, object))
        || beside
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, object))
}

/// One window of an object's code to decode.
///
/// `window` is the stretches wanted, by flat index over every section
/// (`section::Flat`), **nearest the reader first**: the worker takes the first
/// [`CHUNK`] of them. `code` is the skeleton once the view has one and `None` on the first
/// ask, when the worker builds it and answers with it.
#[derive(Clone)]
pub(crate) struct CodeAsk {
    pub(crate) object: Arc<Object>,
    pub(crate) code: Option<Arc<CodeListing>>,
    pub(crate) window: Vec<usize>,
}

impl PartialEq for CodeAsk {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && same_arc(&self.code, &other.code)
            && self.window == other.window
    }
}

impl CodeAsk {
    /// The whole answer to this ask: the skeleton -- its own, or built for it -- and the
    /// first [`CHUNK`] stretches it named that the listing has, by flat index.
    ///
    /// A pure function of the object and the stretches, touching no UI state, which is
    /// what lets the worker run it on a plain thread.
    pub(crate) fn decode(&self) -> (Arc<CodeListing>, Vec<(usize, Stretched)>) {
        let code = self
            .code
            .clone()
            .unwrap_or_else(|| Arc::new(CodeListing::new(&self.object)));
        let index = section::Flat::new(code.clone());
        let decoded = self
            .window
            .iter()
            .take(CHUNK)
            .filter_map(|&flat| {
                let (place, stretch) = index.stretch(flat)?;
                let decoded = code.decode(&self.object, place)?;
                // The symbol's listing exactly as its own tab would work it out -- one
                // decode, the crate's, with the lanes and the line info put beside it as
                // `Studied::new` puts them.
                let studied = stretch.symbol().map(|data| {
                    Studied::with_assembly(
                        Symbol {
                            object: self.object.clone(),
                            data: data.clone(),
                        },
                        decoded.code,
                    )
                });
                Some((
                    flat,
                    Stretched {
                        code: studied,
                        gap: decoded.gap,
                    },
                ))
            })
            .collect();
        (code, decoded)
    }
}

/// One stretch, decoded: the symbol's listing worked out exactly as its own tab's is,
/// and the bytes between its extent and the next label.
#[derive(Clone)]
pub(crate) struct Stretched {
    /// [`None`] for a stretch with no symbol -- the bytes before a section's first one.
    pub(crate) code: Option<Studied>,
    pub(crate) gap: Option<Gap>,
}

impl Stretched {
    /// What the rows are counted and drawn from.
    pub(crate) fn body(&self) -> Body {
        Body {
            assembly: self.code.as_ref().and_then(|code| code.assembly.clone()),
            lanes: self
                .code
                .as_ref()
                .map(|code| code.lanes.clone())
                .unwrap_or_else(Lanes::none),
            gap: self.gap.as_ref().map(|gap| gap.range.clone()),
        }
    }
}

/// What has been decoded of the object whose code is on screen.
#[derive(Clone, Default)]
pub(crate) struct Reading {
    /// The object whose code the view is drawing, or [`None`] while no code tab is on
    /// top. Everything below is about this object and is dropped with it.
    pub(crate) object: Option<Arc<Object>>,
    /// The skeleton, once the first answer has brought it.
    pub(crate) code: Option<Arc<CodeListing>>,
    /// The decoded stretches, by flat index.
    pub(crate) held: BTreeMap<usize, Arc<Stretched>>,
    /// Bumped whenever `code` or `held` changes: what the view's rows are keyed on.
    pub(crate) generation: u64,
}

impl Reading {
    /// A reading of `object`'s code with nothing decoded yet.
    pub(crate) fn of(object: Option<Arc<Object>>) -> Reading {
        Reading {
            object,
            ..Reading::default()
        }
    }

    /// Whether this reading is of `object`.
    pub(crate) fn is_about(&self, object: &Arc<Object>) -> bool {
        self.object
            .as_ref()
            .is_some_and(|own| Arc::ptr_eq(own, object))
    }

    /// The body of stretch `flat`, if it has been decoded: what `Rows::new` asks.
    pub(crate) fn body(&self, flat: usize) -> Option<Body> {
        self.held.get(&flat).map(|stretched| stretched.body())
    }

    /// Take an answer to `ask`. Whether anything was taken.
    ///
    /// **A decoded stretch is a pure function of the object and the stretch and is never
    /// stale**, unlike a listing answer, which is stale the moment the ask moves on: so
    /// an answer is taken whenever it is about this object and this skeleton, whichever
    /// window asked for it -- what a scroll superseded is exactly what the next window
    /// will ask for again.
    pub(crate) fn take(
        &mut self,
        ask: &CodeAsk,
        code: Arc<CodeListing>,
        decoded: Vec<(usize, Stretched)>,
    ) -> bool {
        if !self.is_about(&ask.object) {
            return false;
        }
        match &self.code {
            Some(held) if !Arc::ptr_eq(held, &code) => return false,
            Some(_) => {}
            None => self.code = Some(code),
        }
        for (flat, stretched) in decoded {
            self.held.insert(flat, Arc::new(stretched));
        }
        self.let_go(&ask.window);
        self.generation += 1;
        true
    }

    /// Drop every held stretch farther than [`KEEP`] from the stretches `window` asked
    /// for, which is where the reader is.
    fn let_go(&mut self, window: &[usize]) {
        let (Some(&near), Some(&far)) = (window.iter().min(), window.iter().max()) else {
            return;
        };
        let keep = near.saturating_sub(KEEP)..=far.saturating_add(KEEP);
        self.held.retain(|flat, _| keep.contains(flat));
    }
}

/// Keep [`Reading`] about the object whose code is on top, and about nothing while none
/// is: the reading is reset when the active document stops being that object's code, and
/// when the object is closed under it -- the latter here and not in `close_binary`, since
/// the skeleton holds every section's bytes and a rebuild or a project switch has to drop
/// it too. The window goes with it, so nothing is asked for an object that is not on
/// screen.
pub(crate) fn use_reading_of(
    active: Memo<Option<Entry>>,
    objects: State<Vec<Arc<Object>>>,
    sectioned: Sectioned,
) {
    let Sectioned {
        mut reading,
        mut window,
        beside,
        ..
    } = sectioned;
    use_side_effect(move || {
        let active = active.read().clone().map(|(_, stop)| stop.document);
        let open = objects.read();
        let wanted = match active {
            // One of the project's binaries, and only while it is still open: a closed one
            // cannot be resurrected by a document that outlived it.
            Some(Document::Code(object)) if open.iter().any(|o| Arc::ptr_eq(o, &object)) => {
                Some(object)
            }
            // A listing that is no tab. Only one tab is ever on screen, so a code tab and
            // the Scratchpad's pane are never mounted at once and the two cannot both
            // answer here. Nothing checks it against `objects`: a pad's program is
            // deliberately not one of them, and the claim is let go of when the pane that
            // made it goes.
            None => beside.read().clone(),
            _ => None,
        };
        if !same_arc(&reading.peek().object, &wanted) {
            reading.set(Reading::of(wanted));
            window.set(None);
        }
    });
}

/// The window question: what the section view wants next, asked once ([`use_asking`]).
/// It leaves no mark: an answer is taken whichever window asked for it ([`Reading::take`]).
///
/// Called at the root beside [`use_analysis_with`], which starts the worker and hands
/// back `requests`, the way to ask it.
pub(crate) fn use_code_asks(sectioned: Sectioned, requests: Requests<Question>) {
    let window = sectioned.window;
    use_asking(
        move || window.read().clone(),
        unmarked,
        move |ask| requests.send(Question::Code(ask)),
    );
}

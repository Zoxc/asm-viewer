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

/// The decoded stretches of the object whose code is on screen, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Sections(pub(crate) State<Reading>);

/// The stretches the view wants next, shared through context. Its own state and not a
/// field of [`Reading`]: the effect working out the window reads what is held and would
/// wake itself on writing beside it.
#[derive(Clone, Copy)]
pub(crate) struct Window(pub(crate) State<Option<CodeAsk>>);

/// One window of an object's code to decode.
///
/// `window` is the stretches wanted, by flat index over every section
/// (`section::place_of`), **nearest the reader first**: the worker takes the first
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
        let same_code = match (&self.code, &other.code) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        Arc::ptr_eq(&self.object, &other.object) && same_code && self.window == other.window
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
                .unwrap_or_else(|| Arc::new(Lanes::new(&[], 0))),
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
    /// The ask the worker is working on, or [`None`] when it is idle.
    pub(crate) pending: Option<CodeAsk>,
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
    /// will ask for again. Only `pending` is judged against the ask.
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
        if self.pending.as_ref() == Some(ask) {
            self.pending = None;
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
    mut reading: State<Reading>,
    mut window: State<Option<CodeAsk>>,
) {
    use_side_effect(move || {
        let active = active.read().clone().map(|(_, stop)| stop.document);
        let open = objects.read();
        let wanted = match active {
            Some(Document::Code(object)) if open.iter().any(|o| Arc::ptr_eq(o, &object)) => {
                Some(object)
            }
            _ => None,
        };
        let same = match (&reading.peek().object, &wanted) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        if !same {
            reading.set(Reading::of(wanted));
            window.set(None);
        }
    });
}

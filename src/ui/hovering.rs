//! What the pointer is resting on in the source pane, and what the server says it is.
//!
//! One name at a time, from wherever the pointer is: the row it is over says which of its
//! names it is on and where that name is drawn, and the question goes out from the root as
//! every other question about a place does.
//!
//! **The pointer holds still before anything is asked** ([`HOVER_DELAY`]). A pointer
//! crossing a line of code passes over a name every few pixels, and each one is a round
//! trip to another process; the wait is what makes a hover something the reader asked for
//! rather than something the pointer did on its way somewhere else. **Any move puts the
//! wait back to the beginning**, one inside the same name included, so what is waited on
//! is the pointer stopping. A name already answered draws its box again at once: the wait
//! is for the question and not for the box.
//!
//! **The box is drawn when the answer arrives and not before.** Nothing is drawn while the
//! question is out: a box that appeared empty and filled in afterwards would move under the
//! pointer as it grew, and a name the server has nothing to say about draws nothing at all.
//!
//! **Whether the pointer has left is two flags and not one.** The box sits flush against
//! the name's row, so one platform move takes the pointer out of the row and into the box,
//! and the leave and the enter are emitted in the same batch against the tree measured
//! before either ran (`notes/upstream/freya.md`). One flag would take the box away on the
//! very move that reached it, half the time, depending on which handler ran first. Both
//! are the name's ([`Pointing`]), being about where the pointer is with respect to it.

use super::*;

/// The name the box is about, and where it is drawn.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Pointed {
    /// What the server is asked, in the units it takes: the same [`Lookup`] a press on a
    /// link is followed with.
    pub(crate) at: Lookup,
    /// The name's box, in the window's own **logical** pixels -- what `on_sized` reports
    /// and what a `Position` offset is taken in.
    pub(crate) drawn: Area,
}

/// The name the pointer came to, and everything that is only true while there is one:
/// where the pointer is with respect to it, and the wait before the question about it.
#[derive(Clone, PartialEq)]
struct Pointing {
    /// The name and where it is drawn.
    pointed: Pointed,
    /// Whether the pointer is on the name, and whether it is inside the box. Two flags
    /// for the reason the module doc gives, and both about this name: neither can be
    /// written down with no name for them to be about.
    on_name: bool,
    in_box: bool,
    /// When the wait before the question runs out, which every move pushes back: the
    /// waiting is a task, and this is what it wakes to read rather than a timer it would
    /// have to be told to start again.
    until: Instant,
    /// Whether a task is waiting that out, so the effect below arms one wait per name and
    /// not one per pointer move. Here beside the deadline, the two being one wait: it
    /// begins and ends with the name it is for.
    resting: bool,
}

/// The pointer's name, the question about it, and the answer.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Hover {
    /// The name the box is about. It outlives the pointer leaving the name, which is what
    /// lets the pointer move into the box.
    about: Option<Pointing>,
    /// The question in flight: the run it went out in, its id, and the place it is about.
    /// `Linked`'s reason for holding one -- an answer to a question nobody is waiting for
    /// is an answer to nobody -- and here it also keeps a second question about the one
    /// name from going out.
    asked: Option<(u64, u64, Lookup)>,
    /// What came back, and which place it was about.
    said: Option<(Lookup, String)>,
}

impl Hover {
    /// The pointer is on `pointed`. Whether anything changed, so the caller writes only
    /// then: this is called from a pointer move, which arrives many times over one name.
    pub(crate) fn enter(&mut self, pointed: Pointed) -> bool {
        let held = self.about.take();
        let same = held
            .as_ref()
            .is_some_and(|about| about.pointed == pointed && about.on_name);
        // A name is left by moving onto another as often as by moving off the row, so
        // what was asked about the last one goes here rather than only in `gone`. The wait
        // goes with it: it was for the name the pointer has left.
        let moved = held.as_ref().map(|about| &about.pointed.at) != Some(&pointed.at);
        if moved {
            self.asked = None;
            self.said = None;
        }
        // Every move puts the wait back to the beginning -- and only while there is
        // nothing to show, so a box already up is not written afresh by a pointer moving
        // about inside the name it is about.
        let pushed = self.said.is_none();
        // What the pointer being in the box says is about the box and not about the name
        // under it, so it survives a move onto another name the way it survives a move
        // off the row; the wait and the wait's task do not.
        let kept = held.as_ref().filter(|_| !moved);
        self.about = Some(Pointing {
            pointed,
            on_name: true,
            in_box: held.as_ref().is_some_and(|about| about.in_box),
            until: match kept.filter(|_| !pushed) {
                Some(about) => about.until,
                None => Instant::now() + HOVER_DELAY,
            },
            resting: kept.is_some_and(|about| about.resting),
        });
        pushed || !same
    }

    /// When the wait runs out, for the task waiting it out. [`None`] with no name under
    /// the pointer, which is what stops that task.
    pub(crate) fn until(&self) -> Option<Instant> {
        self.about.as_ref().map(|about| about.until)
    }

    /// The pointer has left the name. The box may still have it.
    pub(crate) fn left_name(&mut self) -> bool {
        let Some(about) = &mut self.about else {
            return false;
        };
        let held = about.on_name;
        about.on_name = false;
        held
    }

    /// The pointer is inside the box, or has left it.
    pub(crate) fn over_box(&mut self, over: bool) -> bool {
        let Some(about) = &mut self.about else {
            return false;
        };
        let held = about.in_box;
        about.in_box = over;
        held != over
    }

    /// Nothing is being hovered at all: the row moved under the pointer, something was
    /// pressed, a key was struck, or the server went away.
    pub(crate) fn gone(&mut self) -> bool {
        let held = self.about.is_some() || self.said.is_some() || self.asked.is_some();
        *self = Hover::default();
        held
    }

    /// The place a wait is owed for: a name is hovered, nothing held answers it, none is
    /// already on its way in this run, and none is already being waited out.
    pub(crate) fn resting(&self, run: u64) -> Option<&Lookup> {
        let armed = self.about.as_ref()?.resting;
        let at = self.pending(run)?;
        (!armed).then_some(at)
    }

    /// The wait for `at` has begun. Whether anything changed, so the caller writes only
    /// then.
    pub(crate) fn resting_on(&mut self, at: Lookup) -> bool {
        let Some(about) = &mut self.about else {
            return false;
        };
        if about.resting || about.pointed.at != at {
            return false;
        }
        about.resting = true;
        true
    }

    /// Whether the pointer is still on `at` with the wait it armed run out, which is what
    /// says the question is worth putting now.
    pub(crate) fn rested(&self, at: &Lookup) -> bool {
        self.about
            .as_ref()
            .is_some_and(|about| about.resting && about.pointed.at == *at)
    }

    /// The place a question is owed for: a name is hovered, nothing held answers it, and
    /// none is already on its way in this run.
    fn pending(&self, run: u64) -> Option<&Lookup> {
        let at = &self.about.as_ref()?.pointed.at;
        if matches!(&self.asked, Some((asked, _, about)) if *asked == run && about == at) {
            return None;
        }
        match &self.said {
            Some((about, _)) if about == at => None,
            _ => Some(at),
        }
    }

    /// The question has gone out. Whether anything changed, so the caller writes only
    /// then.
    pub(crate) fn asking(&mut self, run: u64, id: u64, at: Lookup) -> bool {
        let going = Some((run, id, at));
        if self.asked == going {
            return false;
        }
        self.asked = going;
        true
    }

    /// Take what the server said, and `None` where it said nothing. Whether anything
    /// changed, so the caller writes only then.
    pub(crate) fn answer(&mut self, run: u64, id: u64, said: Option<String>) -> bool {
        // An answer to a question nobody is waiting for: one about a name the pointer has
        // since left, or one from a server that has been restarted since.
        let Some((asked, at, about)) = self.asked.take() else {
            return false;
        };
        if (asked, at) != (run, id) {
            self.asked = Some((asked, at, about));
            return false;
        }
        // A name the server has nothing to say about is a name with no box, and not a
        // question to ask again: `pending` is answered by the empty answer as much as by
        // a full one.
        self.said = Some((about, said.unwrap_or_default()));
        true
    }

    /// What the box draws: the name it is about and the server's words, once the pointer
    /// is on one of the two and there is something to say.
    pub(crate) fn showing(&self) -> Option<(&Pointed, &str)> {
        let about = self.about.as_ref()?;
        if !about.on_name && !about.in_box {
            return None;
        }
        let (_, said) = self
            .said
            .as_ref()
            .filter(|(at, _)| *at == about.pointed.at)?;
        (!said.is_empty()).then_some((&about.pointed, said.as_str()))
    }
}

/// The box goes, and the question and the answer with it. Nothing is written where there
/// was nothing to take away, so an occasion may call this on every press and every key.
///
/// Peek, clone, change, set -- and not read and write. A `State`'s read hands back a
/// guard, and holding one across the write panics the moment it runs (`AGENTS.md`);
/// `gone` needs `&mut` besides. So the value is taken out, changed, and put back, and
/// only where it changed.
pub(crate) fn hover_gone(mut hover: State<Hover>) {
    let mut waiting = hover.peek().clone();
    if waiting.gone() {
        hover.set(waiting);
    }
}

/// Something was pressed: the box goes, wherever the press landed. The reader is doing
/// something else now, and the box is over what they pressed.
pub(crate) fn hover_pressed(hover: State<Hover>) {
    hover_gone(hover);
}

/// A key was struck: the box goes, unless the key is a bare modifier.
///
/// Ctrl is held to open a link's definition in a tab of its own, and Alt to select the
/// name rather than follow it, so both are struck with the pointer resting on the very
/// name the box is about; taking the box away as the reader reaches for the modifier
/// would be answering them with a flinch.
pub(crate) fn hover_struck(hover: State<Hover>, key: &Key) {
    if matches!(
        key,
        Key::Named(NamedKey::Control | NamedKey::Shift | NamedKey::Alt | NamedKey::Meta)
    ) {
        return;
    }
    hover_gone(hover);
}

/// What the source rows write and the box reads.
#[derive(Clone, Copy)]
pub(crate) struct Hovering(pub(crate) State<Hover>);

/// Ask the server about the name the pointer is on. Called once, at the root, beside
/// `use_linking`.
///
/// Asked while the server is still reading the project as well, unlike the question about
/// a whole file: it is one position and not a walk of every name in the file, so it does
/// not park the conversation, and rust-analyzer answers many of them before it has
/// finished. What it does not answer is nothing shown, and the pointer resting on the name
/// again is what asks anew.
pub(crate) fn use_hovering(language: State<Language>, hover: State<Hover>, jobs: LspJobs) {
    use_side_effect(move || {
        // Read and not peeked, both of them: the row writing the name under the pointer is
        // one half of what wakes this, and a server starting is the other.
        let held = language.read().clone();
        if !held.started() {
            // Nothing to answer for, so nothing is held about a name: the box would
            // otherwise go on saying what a server that is gone once said.
            hover_gone(hover);
            return;
        }
        let resting = hover.read().resting(held.run).cloned();
        let Some(at) = resting else {
            return;
        };
        // The wait, and the question after it. Armed before the task, so a pointer moving
        // inside the one name arms one wait and not one per move; the task is what asks,
        // and only where the pointer is still on the name it was armed for.
        let mut hover = hover;
        let mut waiting = hover.peek().clone();
        if waiting.resting_on(at.clone()) {
            hover.set(waiting);
        }
        let jobs = jobs.clone();
        spawn(async move {
            // Waited out rather than slept through: every move of the pointer pushes the
            // end of it back, so what this wakes to is the time to wait *now*, and a
            // pointer travelling slowly over one name wakes it as often as it moves.
            loop {
                let Some(until) = hover.peek().until() else {
                    return;
                };
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                Timer::after(left).await;
            }
            let held = language.peek().clone();
            if !held.started() || !hover.peek().rested(&at) {
                return;
            }
            let Some((run, id)) = ask_hover(language, &jobs, at.clone()) else {
                return;
            };
            // Written after the send and bound before the write, as ever.
            let mut waiting = hover.peek().clone();
            if waiting.asking(run, id, at) {
                hover.set(waiting);
            }
        });
    });
}

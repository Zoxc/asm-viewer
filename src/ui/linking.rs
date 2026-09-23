//! Which names in the file the Source pane is showing are links: what it has asked the
//! language server, and what came back.
//!
//! The pane writes the file it draws into one state and an effect here turns that into a
//! question, which is how its text and the gutter's marks are asked for too
//! (`ShowingFile`, `src/ui/source_view.rs`).
//!
//! **The question is only ever put to a server that has finished reading the project.**
//! Not for tidiness: a request holds the one conversation until it is answered and there
//! is no timeout on it (`src/lsp.rs`), so a whole-file question put to a server that is
//! still indexing would park the worker and every click queued behind it. Waiting also
//! answers what to do about the beat before the server is ready -- there are no links,
//! because nothing has said there are, and the effect asks again when it becomes ready.
//!
//! Nothing is memoized here: the worker's answer is held (`AGENTS.md`), and it is held
//! against the server run it came back under, so a server that has been restarted is
//! never answered for by the one before it. **A server that has stopped or failed leaves
//! nothing drawn.** The names it classified are still the right names, but there is
//! nobody left to answer a press on one, and a link that does nothing is what waiting for
//! a ready server exists to prevent.

use super::*;

/// The links in the file the pane is showing, and the question owed for it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Linked {
    /// The question in flight: the file it is about and the ticket it went out under.
    ///
    /// Held for the reason `Follow` and `Located` hold theirs -- **an answer to a
    /// question nobody is waiting for is an answer to nobody** -- and here it is what
    /// keeps a second question from going out at all. The server says how far through the
    /// project it has got over and over, each word of it a reason for the effect below to
    /// look again; without this, every one of them sent the same question afresh, and the
    /// one that came back refused wrote over the one that had not.
    ///
    /// **A ticket and not the run**, so a question dropped while in flight stays dropped
    /// when the same file is asked about again in the same run ([`Linked::forget_answer`]).
    asked: Option<(Arc<Path>, Ticket)>,
    /// The file the links below are of, the server run they came back under, and them --
    /// `None` where the server refused to answer.
    ///
    /// **A refusal is not an answer.** rust-analyzer refuses a question about a file it
    /// has not read yet, and there is a beat before it says it is working in which it is
    /// asked; filed as an empty answer, that beat cost the file its links for the whole
    /// life of the server. Held as a refusal instead, it is what [`Linked::forget_refusal`]
    /// drops when the server has read more of the project.
    found: Option<(Arc<Path>, u64, Option<links::Links>)>,
}

impl Linked {
    /// Whether a question is owed for `showing`: nothing held answers it, and none is
    /// already on its way.
    pub(crate) fn pending(&self, showing: &Arc<Path>, run: u64) -> bool {
        let asked = self.asked.as_ref();
        if asked.is_some_and(|(file, ticket)| file == showing && ticket.run == run) {
            return false;
        }
        !matches!(&self.found, Some((file, at, _)) if file == showing && *at == run)
    }

    /// The question has gone out. Whether anything changed, so the caller writes only
    /// then.
    pub(crate) fn asking(&mut self, ticket: Ticket, file: Arc<Path>) -> bool {
        let going = Some((file, ticket));
        if self.asked == going {
            return false;
        }
        self.asked = going;
        true
    }

    /// The links in `file`, and nothing where what is held is about another or is a
    /// refusal -- which is what a pane draws in the beat between moving and being
    /// answered.
    pub(crate) fn links_in(&self, file: &Path) -> Option<&links::Links> {
        match &self.found {
            Some((of, _, Some(links))) if &**of == file => Some(links),
            _ => None,
        }
    }

    /// Take `links` as the answer about `file` to the question `ticket`. Whether anything
    /// changed, so the caller writes only then.
    pub(crate) fn answer(&mut self, ticket: Ticket, file: Arc<Path>, links: links::Links) -> bool {
        self.take(ticket, file, Some(links))
    }

    /// The server refused to answer about `file` to the question `ticket`. Nothing is
    /// drawn for it, and it is asked again once the server has read more of the project.
    pub(crate) fn answer_refused(&mut self, ticket: Ticket, file: Arc<Path>) -> bool {
        self.take(ticket, file, None)
    }

    /// Both answers, the guard being the same one.
    fn take(&mut self, ticket: Ticket, file: Arc<Path>, links: Option<links::Links>) -> bool {
        // An answer to a question nobody is waiting for: one already answered, one about
        // a file the pane has since left, one from a server that has been restarted
        // since, or one dropped because the server settled after it was asked. Taking it
        // would let a second question's refusal land on top of the names the first one
        // came back with.
        if self.asked.as_ref() != Some(&(file.clone(), ticket)) {
            return false;
        }
        self.asked = None;
        self.found = Some((file, ticket.run, links));
        true
    }

    /// Forget the answer and the question both: with no server there is nothing either is
    /// about. Whether anything changed, so the caller writes only then.
    fn forget(&mut self) -> bool {
        let held = self.found.is_some() || self.asked.is_some();
        self.found = None;
        self.asked = None;
        held
    }

    /// Drop what the server said, and the question still in flight, so the next turn of
    /// [`use_linking`] asks again. Whether anything changed, so the caller writes only
    /// then.
    ///
    /// Called where the server says it has **settled**, and what it answered before then
    /// was as far as it had got: fewer names, and some of them classified wrongly -- a
    /// `builtinType` the server had not resolved yet arrives as something the app draws
    /// as a link, and a link that leads nowhere is worse than no link at all.
    ///
    /// **The question in flight goes too.** It was asked before the server settled, and
    /// its answer comes back on the worker's channel while the news came on the notes
    /// channel, with nothing ordering the two: kept, an answer worked out before the
    /// server settled could land after this and be held for the life of the server.
    pub(crate) fn forget_answer(&mut self) -> bool {
        let held = self.found.is_some() || self.asked.is_some();
        self.found = None;
        self.asked = None;
        held
    }

    /// Whether what is held is the server's answer about `file` in run `run` -- its
    /// names or its refusal, both being the server having spoken -- with no question
    /// still in flight.
    ///
    /// Test-only: it is what a headless test waits for before asserting about links
    /// (`serving`, `src/ui/tests.rs`). The app asks the other way round, which is
    /// [`Linked::pending`].
    #[cfg(test)]
    pub(crate) fn answered(&self, file: &Path, run: u64) -> bool {
        self.asked.is_none()
            && matches!(&self.found, Some((of, at, _)) if &**of == file && *at == run)
    }

    /// Drop what is held about `file`, so the next turn of [`use_linking`] asks about it
    /// again. Whether anything changed, so the caller writes only then.
    ///
    /// Called where the server has just been told about the file: what it said before
    /// that is what it could work out of the disk on its own, which is nothing at all
    /// until its own scan of the directory has reached the file.
    pub(crate) fn forget_file(&mut self, file: &Path) -> bool {
        let held = matches!(&self.found, Some((of, _, _)) if &**of == file);
        if held {
            self.found = None;
        }
        held
    }

    /// Drop a refusal, so the next turn of [`use_linking`] asks again. Whether anything
    /// changed, so the caller writes only then.
    ///
    /// Called where the server says it has gone quiet, and not on every word it says: a
    /// server that keeps refusing would otherwise be asked in a tight loop.
    pub(crate) fn forget_refusal(&mut self) -> bool {
        let refused = matches!(&self.found, Some((_, _, None)));
        if refused {
            self.found = None;
        }
        refused
    }
}

/// What the Source pane's rows read to know which of their names are links.
#[derive(Clone, Copy)]
pub(crate) struct Linking(pub(crate) State<Linked>);

/// Ask the server about the file the pane is showing, once it is ready to be asked.
/// Called once, at the root, beside `use_follow`.
pub(crate) fn use_linking(
    language: State<Language>,
    linked: State<Linked>,
    showing: State<Option<Arc<Path>>>,
    opened: State<Opened>,
    jobs: LspJobs,
) {
    // What the two readers below want of the state, each a memo over it: a remark from
    // the server writes the state, and wakes neither unless it changed that.
    let started = use_memo(move || language.read().started());
    let answering = use_memo(move || language.read().answering());

    // A server that has stopped or failed has nothing left to answer for, so what it said
    // goes with it: the rows are handed the links as data and would go on drawing every
    // name as one a press follows. A server that is *working* keeps them -- they are
    // still the right names while it reads more of the project -- and so does one that is
    // starting, which is the beat before its first answer.
    use_side_effect(move || {
        if !*started.read() {
            write_if(linked, |waiting| waiting.forget());
        }
    });

    use_asking(
        // All four are read and none peeked, and read in the memo, which is what
        // subscribes it to them: the pane moving to another file is one of the things
        // that wakes this, and the server saying it has finished reading the project is
        // another. A file opened while it was still reading has no links until then, and
        // gets them without the reader doing anything.
        move || {
            let run = (*answering.read())?;
            let file = showing.read().clone()?;
            if !linked.read().pending(&file, run) {
                return None;
            }
            // Only about a file the server has been told the app is showing, which is
            // what `Opened` decides -- and which leaves out a file of a language the
            // server is not for, since one asked about it answers as if it were its own
            // (`serves`).
            opened.read().holds(run, &file).then_some((run, file))
        },
        // The ticket is minted as the question goes out, and not in the memo: a memo that
        // minted one would be a new question every time it looked. So the mark is made
        // here, still before the send.
        unmarked,
        move |(run, file)| {
            let ticket = jobs.ticket(run);
            write_if(linked, |waiting| waiting.asking(ticket, file.clone()));
            jobs.send(LspJob::Tokens { ticket, file });
        },
    );
}

//! Which names in the file the Source pane is showing are links: what it has asked the
//! language server, and what came back.
//!
//! The pane writes the file it draws and an effect turns that into a question, which is
//! how the gutter's marks are asked for too (`Coded`, `src/ui/source_view.rs`). One file,
//! because one is drawn; a pane that moves to another asks again.
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
//! never answered for by the one before it.

use super::*;

/// The links in the file the pane is showing, and the question owed for it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Linked {
    /// The file the Source pane is showing, written by it.
    pub(crate) wanted: Option<Arc<str>>,
    /// The question in flight: the file it is about and the run it went out in.
    ///
    /// Held for the reason `Follow` and `Located` hold theirs -- **an answer to a
    /// question nobody is waiting for is an answer to nobody** -- and here it is what
    /// keeps a second question from going out at all. The server says how far through the
    /// project it has got over and over, each word of it a reason for the effect below to
    /// look again; without this, every one of them sent the same question afresh, and the
    /// one that came back refused wrote its empty answer over the one that had not.
    asked: Option<(Arc<str>, u64)>,
    /// The file the links below are of, the server run they came back under, and them.
    found: Option<(Arc<str>, u64, links::Links)>,
}

impl Linked {
    /// The file a question is owed for: one is wanted, nothing held answers it, and none
    /// is already on its way.
    pub(crate) fn pending(&self, run: u64) -> Option<&Arc<str>> {
        let wanted = self.wanted.as_ref()?;
        let about = |held: &Option<(Arc<str>, u64)>| {
            held.as_ref()
                .is_some_and(|(file, at)| file == wanted && *at == run)
        };
        if about(&self.asked) {
            return None;
        }
        match &self.found {
            Some((file, at, _)) if file == wanted && *at == run => None,
            _ => Some(wanted),
        }
    }

    /// The question has gone out. Whether anything changed, so the caller writes only
    /// then.
    pub(crate) fn asking(&mut self, run: u64, file: Arc<str>) -> bool {
        let going = Some((file, run));
        if self.asked == going {
            return false;
        }
        self.asked = going;
        true
    }

    /// The links in `file`, and nothing where what is held is about another -- which is
    /// what a pane draws in the beat between moving and being answered.
    pub(crate) fn links_in(&self, file: &str) -> Option<&links::Links> {
        match &self.found {
            Some((of, _, links)) if &**of == file => Some(links),
            _ => None,
        }
    }

    /// Take `links` as the answer about `file` in run `run`. Whether anything changed, so
    /// the caller writes only then.
    pub(crate) fn answer(&mut self, run: u64, file: Arc<str>, links: links::Links) -> bool {
        // An answer to a question nobody is waiting for: one already answered, one about
        // a file the pane has since left, or one from a server that has been restarted
        // since. Taking it would let a refusal, whose answer is empty, land on top of the
        // names that came back for the same file.
        if self.asked.as_ref() != Some(&(file.clone(), run)) {
            return false;
        }
        self.asked = None;
        self.found = Some((file, run, links));
        true
    }
}

/// What the Source pane's rows read to know which of their names are links.
#[derive(Clone, Copy)]
pub(crate) struct Linking(pub(crate) State<Linked>);

/// Ask the server about the file the pane is showing, once it is ready to be asked.
/// Called once, at the root, beside `use_follow`.
pub(crate) fn use_linking(language: State<Language>, linked: State<Linked>, jobs: LspJobs) {
    use_side_effect(move || {
        // Read and not peeked, both of them: the pane writing the file it moved to is one
        // half of what wakes this, and the server saying it has finished reading the
        // project is the other. A file opened while it was still reading has no links
        // until then, and gets them without the reader doing anything.
        let held = language.read().clone();
        if !held.ready() {
            return;
        }
        let pending = linked.read().pending(held.run).cloned();
        let Some(file) = pending else {
            return;
        };
        jobs.send(LspJob::Tokens {
            run: held.run,
            file: file.clone(),
        });
        // Written after the send and bound before the write, as ever. This is what the
        // next turn of the effect reads to see that the question is already on its way.
        let mut waiting = linked.peek().clone();
        if waiting.asking(held.run, file) {
            let mut linked = linked;
            linked.set(waiting);
        }
    });
}

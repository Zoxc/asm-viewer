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
//! never answered for by the one before it. **A server that has stopped or failed leaves
//! nothing drawn.** The names it classified are still the right names, but there is
//! nobody left to answer a press on one, and a link that does nothing is what waiting for
//! a ready server exists to prevent.

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
    /// one that came back refused wrote over the one that had not.
    asked: Option<(Arc<str>, u64)>,
    /// The file the links below are of, the server run they came back under, and them --
    /// `None` where the server refused to answer.
    ///
    /// **A refusal is not an answer.** rust-analyzer refuses a question about a file it
    /// has not read yet, and there is a beat before it says it is working in which it is
    /// asked; filed as an empty answer, that beat cost the file its links for the whole
    /// life of the server. Held as a refusal instead, it is what [`Linked::forget_refusal`]
    /// drops when the server has read more of the project.
    found: Option<(Arc<str>, u64, Option<links::Links>)>,
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

    /// The links in `file`, and nothing where what is held is about another or is a
    /// refusal -- which is what a pane draws in the beat between moving and being
    /// answered.
    pub(crate) fn links_in(&self, file: &str) -> Option<&links::Links> {
        match &self.found {
            Some((of, _, Some(links))) if &**of == file => Some(links),
            _ => None,
        }
    }

    /// Take `links` as the answer about `file` in run `run`. Whether anything changed, so
    /// the caller writes only then.
    pub(crate) fn answer(&mut self, run: u64, file: Arc<str>, links: links::Links) -> bool {
        self.take(run, file, Some(links))
    }

    /// The server refused to answer about `file` in run `run`. Nothing is drawn for it,
    /// and it is asked again once the server has read more of the project.
    pub(crate) fn answer_refused(&mut self, run: u64, file: Arc<str>) -> bool {
        self.take(run, file, None)
    }

    /// Both answers, the guard being the same one.
    fn take(&mut self, run: u64, file: Arc<str>, links: Option<links::Links>) -> bool {
        // An answer to a question nobody is waiting for: one already answered, one about
        // a file the pane has since left, or one from a server that has been restarted
        // since. Taking it would let a second question's refusal land on top of the names
        // the first one came back with.
        if self.asked.as_ref() != Some(&(file.clone(), run)) {
            return false;
        }
        self.asked = None;
        self.found = Some((file, run, links));
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

    /// Drop what the server said, so the next turn of [`use_linking`] asks again. Whether
    /// anything changed, so the caller writes only then.
    ///
    /// Called where the server says it has **settled**, and what it answered before then
    /// was as far as it had got: fewer names, and some of them classified wrongly -- a
    /// `builtinType` the server had not resolved yet arrives as something the app draws
    /// as a link, and a link that leads nowhere is worse than no link at all.
    pub(crate) fn forget_answer(&mut self) -> bool {
        let held = self.found.is_some();
        self.found = None;
        held
    }

    /// Drop what is held about `file`, so the next turn of [`use_linking`] asks about it
    /// again. Whether anything changed, so the caller writes only then.
    ///
    /// Called where the server has just been told about the file: what it said before
    /// that is what it could work out of the disk on its own, which is nothing at all
    /// until its own scan of the directory has reached the file.
    pub(crate) fn forget_file(&mut self, file: &str) -> bool {
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

/// The files the app has told the server it is showing, and the server it told.
///
/// The protocol has the client own the documents it shows: a server answers about the
/// text it was given until it is told the file has closed, and a file it was never told
/// about it can only answer for out of its own reading of the directory -- which
/// rust-analyzer does, four seconds into a two-file crate and longer for anything real.
/// Measured over the same file: its names at 0.0s where it had been opened, and at 4.3s
/// where it had not.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Opened {
    /// Which server holds them. A new one holds nothing, whatever this last said.
    run: u64,
    files: Vec<Arc<str>>,
    /// The ones the app has read afresh since it told the server about them, so what the
    /// server holds of those is the text from before.
    stale: Vec<Arc<str>>,
}

impl Opened {
    /// Whether the server has been told about `file`, and so whether it is a file this
    /// app asks it anything about.
    pub(crate) fn holds(&self, run: u64, file: &str) -> bool {
        self.run == run && self.files.iter().any(|held| &**held == file)
    }

    /// The files under `root` have been read again, so the server is holding the text
    /// from before them. Whether anything changed, so the caller writes only then.
    ///
    /// **This is the cost of opening documents at all.** The protocol has the server
    /// answer about the text it was given until it is told otherwise, so a file rewritten
    /// under an open tab -- which a build does, and a scratchpad's on every build -- is a
    /// file the server goes on answering about as it was. The app re-reads such files in
    /// one place (`forget_source_under`), and this is that place told.
    pub(crate) fn reread(&mut self, root: &Path) -> bool {
        let stale: Vec<Arc<str>> = self
            .files
            .iter()
            .filter(|file| Path::new(&***file).starts_with(root))
            .filter(|file| !self.stale.contains(file))
            .cloned()
            .collect();
        if stale.is_empty() {
            return false;
        }
        self.stale.extend(stale);
        true
    }

    /// What to tell the server, given the files the reader has open: the ones it has not
    /// been told about, and the ones it holds that are open no longer.
    ///
    /// A server that has been restarted holds nothing, so everything open is new and
    /// nothing is worth closing -- the process those files were open in is gone.
    fn against(&self, run: u64, open: &[Arc<str>]) -> (Vec<Arc<str>>, Vec<Arc<str>>) {
        let held: &[Arc<str>] = match self.run == run {
            true => &self.files,
            false => &[],
        };
        let opened = open
            .iter()
            .filter(|file| !held.iter().any(|had| had == *file))
            .cloned()
            .collect();
        let closed = held
            .iter()
            .filter(|file| !open.iter().any(|have| have == *file))
            .cloned()
            .collect();
        (opened, closed)
    }
}

/// What the project's server is told `path` is, and `None` for a file it is not for at
/// all.
///
/// **A server answers about a file whatever language it is.** rust-analyzer, asked about a
/// C file, reads it as Rust and answers with what a Rust lexer made of it -- measured: a
/// small C file came back with three `struct` tokens and a `property`, every one of which
/// this app would draw as a link and follow to nowhere. So a file the server is not for is
/// neither opened nor asked about.
///
/// Which files those are is the **project's** to say, the app knowing the program and not
/// what it serves: the extensions named in the Project view, in the reader's own spelling.
/// Where they named none it is the program's own answer -- the one program this app knows
/// by name is Rust's, and a project that named its own gets asked about whatever it opens,
/// that being the reader's business.
///
/// The identifier is [`source::Language::spoken`] where the app knows the language, and
/// the extension itself where the reader named one it does not: the specification says to
/// send the extension for a language it has no name for, and a server that does not know
/// the identifier ignores the file, which is what it would have done anyway.
fn spoken_as(chosen: &[String], program: &str, path: &Path) -> Option<String> {
    let known = source::Language::of(path);
    let extension = path.extension().and_then(|extension| extension.to_str());
    if chosen.is_empty() {
        let known = known?;
        // The one program this app knows by name is Rust's (`source::Language::server`);
        // anything else is the project's own.
        return match source::Language::Rust.server() == Some(program) {
            true => (known.server() == Some(program)).then(|| known.spoken().to_owned()),
            false => Some(known.spoken().to_owned()),
        };
    }
    let named = extension.is_some_and(|extension| chosen.iter().any(|one| one == extension));
    named.then(|| match known {
        Some(known) => known.spoken().to_owned(),
        None => extension.unwrap_or_default().to_owned(),
    })
}

/// Every open tab's source file the project's server is for, in the reader's own order,
/// each with what the server is told it is.
fn shown(open: Open, chosen: &[String], program: &str) -> Vec<(Arc<str>, String)> {
    let strip = open.strip.read();
    let docs = open.docs.read();
    open_ids(&strip)
        .into_iter()
        .filter_map(|id| docs.get(id))
        .filter_map(|document| match document {
            Document::Source(file) => Some(file.clone()),
            // A symbol in a binary is a place in no file, and an object's code is the
            // whole of one: neither is a document a server has anything to say about.
            Document::Assembly(..) | Document::Code(..) => None,
        })
        .filter_map(|file| {
            let spoken = spoken_as(chosen, program, Path::new(&*file))?;
            Some((file, spoken))
        })
        .collect()
}

/// Tell the server which files the reader has open, and which they have closed. Called
/// once, at the root, beside [`use_linking`].
pub(crate) fn use_opened(
    language: State<Language>,
    opened: State<Opened>,
    open: Open,
    proj: State<OpenProject>,
    jobs: LspJobs,
) {
    use_side_effect(move || {
        // Every one of these is read and not peeked: a tab opened or closed, a server
        // started, and a project whose server is another program are each half of what
        // this is about.
        let held = language.read().clone();
        let held_project = proj.read();
        let (program, chosen) = (held_project.server(), held_project.server_files());
        drop(held_project);
        if !held.started() {
            let mut waiting = opened.peek().clone();
            if !waiting.files.is_empty() || !waiting.stale.is_empty() {
                waiting.files.clear();
                waiting.stale.clear();
                let mut opened = opened;
                opened.set(waiting);
            }
            return;
        }
        let shown = shown(open, &chosen, &program);
        let files: Vec<Arc<str>> = shown.iter().map(|(file, _)| file.clone()).collect();
        let spoken = |file: &Arc<str>| {
            shown
                .iter()
                .find(|(shown, _)| shown == file)
                .map(|(_, spoken)| spoken.clone())
                .unwrap_or_default()
        };
        // A file read afresh is closed and opened again, which is how the server is given
        // the new text: it holds one version of a file, this app having one to give.
        let stale: Vec<Arc<str>> = opened
            .read()
            .stale
            .iter()
            .filter(|file| files.contains(file))
            .cloned()
            .collect();
        for file in &stale {
            jobs.send(LspJob::Closed { file: file.clone() });
            jobs.send(LspJob::Opened {
                run: held.run,
                language: spoken(file),
                file: file.clone(),
            });
        }
        let (opening, closing) = opened.read().against(held.run, &files);
        if opening.is_empty() && closing.is_empty() && opened.read().stale.is_empty() {
            return;
        }
        for file in closing {
            jobs.send(LspJob::Closed { file });
        }
        for file in opening {
            jobs.send(LspJob::Opened {
                run: held.run,
                language: spoken(&file),
                file,
            });
        }
        // Written after the sends and bound before the write, as ever.
        let mut opened = opened;
        opened.set(Opened {
            run: held.run,
            files,
            stale: Vec::new(),
        });
    });
}

/// What the app has told the server it is showing.
#[derive(Clone, Copy)]
pub(crate) struct Documents(pub(crate) State<Opened>);

/// What the Source pane's rows read to know which of their names are links.
#[derive(Clone, Copy)]
pub(crate) struct Linking(pub(crate) State<Linked>);

/// Ask the server about the file the pane is showing, once it is ready to be asked.
/// Called once, at the root, beside `use_follow`.
pub(crate) fn use_linking(
    language: State<Language>,
    linked: State<Linked>,
    opened: State<Opened>,
    jobs: LspJobs,
) {
    use_side_effect(move || {
        // Read and not peeked, both of them: the pane writing the file it moved to is one
        // half of what wakes this, and the server saying it has finished reading the
        // project is the other. A file opened while it was still reading has no links
        // until then, and gets them without the reader doing anything.
        let held = language.read().clone();
        if !held.ready() {
            // A server that has stopped or failed has nothing left to answer for, so what
            // it said goes with it: the rows are handed the links as data and would go on
            // drawing every name as one a press follows. A server that is *working* keeps
            // them -- they are still the right names while it reads more of the project --
            // and so does one that is starting, which is the beat before its first answer.
            if matches!(held.state, Lsp::Off | Lsp::Failed(_)) {
                write_if(linked, |waiting| waiting.forget());
            }
            return;
        }
        let pending = linked.read().pending(held.run).cloned();
        let Some(file) = pending else {
            return;
        };
        // Only about a file the server has been told the app is showing, which is what
        // `Opened` decides -- and which leaves out a file of a language the server is not
        // for, since one asked about it answers as if it were its own (`serves`).
        if !opened.read().holds(held.run, &file) {
            return;
        }
        jobs.send(LspJob::Tokens {
            run: held.run,
            file: file.clone(),
        });
        // Written after the send. This is what the next turn of the effect reads to see
        // that the question is already on its way.
        write_if(linked, |waiting| waiting.asking(held.run, file));
    });
}

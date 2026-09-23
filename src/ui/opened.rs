//! The files the app has told the language server it has open, and the effect that keeps
//! that set in step with the bar.
//!
//! **The client owns the documents it shows.** A server answers about the text it was
//! given until it is told the file has closed, and a file it was never told about it can
//! only answer for out of its own reading of the directory -- which rust-analyzer does,
//! four seconds into a two-file crate and longer for anything real. Measured over the
//! same file: its names at 0.0s where it had been opened, and at 4.3s where it had not.
//!
//! What is opened is what the reader has in tabs, less the files this project's server is
//! not for ([`spoken_as`]). A server that has stopped holds nothing, so the app lets go
//! of the whole set and the next one is told everything afresh.

use super::*;

/// The files the app has told the server it is showing, and the server it told.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Opened {
    /// Which server holds them. A new one holds nothing, whatever this last said.
    run: u64,
    files: Vec<Arc<Path>>,
    /// The ones the app has read afresh since it told the server about them, so what the
    /// server holds of those is the text from before.
    stale: Vec<Arc<Path>>,
}

impl Opened {
    /// Whether the server has been told about `file`, and so whether it is a file this
    /// app asks it anything about.
    pub(crate) fn holds(&self, run: u64, file: &Path) -> bool {
        self.run == run && self.files.iter().any(|held| &**held == file)
    }

    /// The files under `root` have been read again, so the server is holding the text
    /// from before them. Whether anything changed, so the caller writes only then.
    ///
    /// **This is the cost of opening documents at all.** The protocol has the server
    /// answer about the text it was given until it is told otherwise, so a file rewritten
    /// under an open tab -- which a build does, and a scratchpad's on every build -- is a
    /// file the server goes on answering about as it was. The app re-reads such files in
    /// one place (`Sourced::forget_under`), and this is that place told.
    pub(crate) fn reread(&mut self, root: &Path) -> bool {
        let stale: Vec<Arc<Path>> = self
            .files
            .iter()
            .filter(|file| file.starts_with(root))
            .filter(|file| !self.stale.contains(file))
            .cloned()
            .collect();
        if stale.is_empty() {
            return false;
        }
        self.stale.extend(stale);
        true
    }

    /// Forget the lot: with no server there is nothing holding any of it. Whether
    /// anything changed, so the caller writes only then.
    ///
    /// The run is left alone. It names the server the files were sent to, and with no
    /// files it says nothing; 0 is a run a real server could have.
    fn forget(&mut self) -> bool {
        let held = !self.files.is_empty() || !self.stale.is_empty();
        self.files.clear();
        self.stale.clear();
        held
    }

    /// What to tell the server, given the files the reader has open: the ones it has not
    /// been told about, each with what it is to be told they are, and the ones it holds
    /// that are open no longer, which need no language to close.
    ///
    /// A server that has been restarted holds nothing, so everything open is new and
    /// nothing is worth closing -- the process those files were open in is gone.
    fn against(
        &self,
        run: u64,
        open: &[(Arc<Path>, String)],
    ) -> (Vec<(Arc<Path>, String)>, Vec<Arc<Path>>) {
        let held: &[Arc<Path>] = match self.run == run {
            true => &self.files,
            false => &[],
        };
        let opened = open
            .iter()
            .filter(|(file, _)| !held.iter().any(|had| had == file))
            .cloned()
            .collect();
        let closed = held
            .iter()
            .filter(|file| !open.iter().any(|(have, _)| have == *file))
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
/// what it serves: the extensions named in the Project view, in the reader's own spelling,
/// as they were when the server started ([`Serving`]).
/// Where they named none it is the program's own answer -- the one program this app knows
/// by name is Rust's, and a project that named its own gets asked about whatever it opens,
/// that being the reader's business.
///
/// The identifier is [`languages::Language::spoken`] where the app knows the language, and
/// the extension itself where the reader named one it does not: the specification says to
/// send the extension for a language it has no name for, and a server that does not know
/// the identifier ignores the file, which is what it would have done anyway.
fn spoken_as(serving: &Serving, path: &Path) -> Option<String> {
    let Serving {
        program,
        files: chosen,
    } = serving;
    let known = languages::Language::of(path);
    let extension = path.extension().and_then(|extension| extension.to_str());
    if chosen.is_empty() {
        let known = known?;
        // The one program this app knows by name is Rust's (`languages::Language::server`);
        // anything else is the project's own.
        return match languages::Language::Rust.server() == Some(program.as_str()) {
            true => (known.server() == Some(program.as_str())).then(|| known.spoken().to_owned()),
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
///
/// **Each file once**, however many tabs show it: two tabs can show one place, and a
/// server is told a file is open once until it is told it has closed.
fn shown(open: Open, serving: &Serving) -> Vec<(Arc<Path>, String)> {
    let strip = open.strip.read();
    let docs = open.docs.read();
    let mut seen = HashSet::new();
    strip
        .documents()
        .filter_map(|id| docs.get(id))
        .filter_map(|document| match document {
            Document::Source(file) => Some(file.clone()),
            // A symbol in a binary is a place in no file, and an object's code is the
            // whole of one: neither is a document a server has anything to say about.
            Document::Object(..) | Document::Symbol(..) | Document::Code(..) => None,
        })
        .filter(|file| seen.insert(file.clone()))
        .filter_map(|file| {
            let spoken = spoken_as(serving, &file)?;
            Some((file, spoken))
        })
        .collect()
}

/// Tell the server which files the reader has open, and which they have closed. Called
/// once, at the root, beside [`use_linking`].
///
/// **A file is carried about with what the server is told it is**, from [`shown`] all the
/// way to the send. The pair is worked out once, and the three lists below are cut out of
/// it, so nothing has to find a file's language a second time.
pub(crate) fn use_opened(
    language: State<Language>,
    opened: State<Opened>,
    open: Open,
    jobs: LspJobs,
) {
    // The run and what it was started with, out of the state and not read off it: a
    // remark from the server writes the state, and would wake the effect below for
    // nothing to send.
    let started = use_memo(move || {
        let held = language.read();
        let serving = held.state.serving()?;
        Some((held.run, serving.clone()))
    });
    use_side_effect(move || {
        // Both read and not peeked: a tab opened or closed and a server started are each
        // half of what this is about. Which files the server is for is what it was started
        // with, and not what the Project view's boxes say now.
        let started = started.read().clone();
        let Some((run, serving)) = started else {
            write_if(opened, |waiting| waiting.forget());
            return;
        };
        let shown = shown(open, &serving);
        // One read, bound before any write: the three lists are all cut out of what it
        // said.
        let told = opened.read().clone();
        // A file read afresh is closed and opened again, which is how the server is given
        // the new text: it holds one version of a file, this app having one to give.
        let stale: Vec<(Arc<Path>, String)> = shown
            .iter()
            .filter(|(file, _)| told.stale.contains(file))
            .cloned()
            .collect();
        for (file, spoken) in &stale {
            jobs.send(LspJob::Closed {
                run,
                file: file.clone(),
            });
            jobs.send(LspJob::Opened {
                run,
                language: spoken.clone(),
                file: file.clone(),
            });
        }
        let (opening, closing) = told.against(run, &shown);
        // The whole stale list and not the pairs above: an entry naming a file the reader
        // has since closed is one the write below drops, and there is nothing to send for
        // it.
        if opening.is_empty() && closing.is_empty() && told.stale.is_empty() {
            return;
        }
        for file in closing {
            jobs.send(LspJob::Closed { run, file });
        }
        for (file, spoken) in opening {
            jobs.send(LspJob::Opened {
                run,
                language: spoken,
                file,
            });
        }
        // Written after the sends, as ever.
        let mut opened = opened;
        opened.set(Opened {
            run,
            files: shown.into_iter().map(|(file, _)| file).collect(),
            stale: Vec::new(),
        });
    });
}

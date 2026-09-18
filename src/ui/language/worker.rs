//! The blocking half of the language server: what the worker is asked, what it answers,
//! and which of those a newer question takes the place of.
//!
//! The only part of the app that names `lsp::Server`. Talking to one blocks, so it happens
//! on a thread of its own; there is one server and one conversation, so the work is a
//! closure holding that conversation rather than the plain `Fn` the other workers are
//! (`src/ui/worker.rs`). What the app holds of the server, and the hook that starts this
//! thread, are `src/ui/language.rs`'s.

use super::*;

/// What the worker is asked to do.
pub(crate) enum LspJob {
    /// Start a server over `directory` and shake hands with it. The channel is what the
    /// server's own remarks come back on, since they arrive long after this is answered.
    Start {
        run: u64,
        directory: PathBuf,
        /// The program to run, which is the project's when it named one.
        program: String,
        /// What to tell it about the project. Carried in the job because the worker
        /// thread may read no UI state, exactly as `program` and `directory` are.
        settings: lsp::Settings,
        notes: async_channel::Sender<(u64, lsp::Note)>,
        /// Where [`LspAnswer::Spawned`] goes, which is the app's own answer channel. A
        /// start is the one job with something to say before it is done.
        spawned: async_channel::Sender<LspAnswer>,
    },
    /// Read the project's own `.vscode/settings.json`. A file read blocks, so it happens
    /// here rather than on the UI thread; it is this worker's and not the build worker's
    /// because what it answers is what a start has to carry.
    ReadSettings { directory: PathBuf },
    /// What is at a place: which of the four questions is in `want` (`lsp::Question`).
    /// The [`Ticket`] is minted by [`ask_where`] and copied into the answer, which is what
    /// lets the asker tell its own answer from the one before it.
    Ask {
        ticket: Ticket,
        at: Lookup,
        want: lsp::Question,
    },
    /// What every name in one file is, which is a question about the file and not about
    /// a place in it. The file travels as the `Arc<str>` a document is named by, since
    /// that is what the answer has to be matched against.
    Tokens { run: u64, file: Arc<str> },
    /// What the name under the pointer is. A question about a place like [`LspJob::Ask`]'s
    /// four, and **not** a fifth `lsp::Question`: those are bucketed by consumer, of which
    /// this is a third, and a pointer crossing a name must neither take back a definition
    /// the reader clicked for nor be taken back by one.
    Hover { ticket: Ticket, at: Lookup },
    /// The app is showing this file, or has stopped showing it. Not a question: the
    /// server answers neither, and what they change is what every other question about
    /// the file is answered out of (`lsp::Talk::opened`).
    ///
    /// The text is not carried: the file is read on the worker, which is the thread that
    /// may block, and read at all only for a server that takes documents. The language is,
    /// since what a file is told to be is the project's to say (`src/ui/opened.rs`).
    Opened {
        run: u64,
        file: Arc<str>,
        language: String,
    },
    /// The other half, and the one job that names **no run**: a job's run is what stamps
    /// the answer it comes back as, and a close is answered with nothing.
    Closed { file: Arc<str> },
    /// Let go of the server: it has been stopped already, and this is what reaps it.
    Stop,
}

/// What came of it. Every answer names the run it is about, and one whose run has moved
/// on is not the answer to any question anybody still has.
pub(crate) enum LspAnswer {
    /// The process exists. Sent from inside the `Start` job and before the handshake,
    /// which is what puts the handle where a stop can reach it: until this the worker is
    /// in a read that only the pipes closing ends, and the pipes close with the process.
    Spawned { run: u64, handle: process::Handle },
    /// The handshake is over: it is answering, or `started` says why there is none. No
    /// handle of its own -- [`LspAnswer::Spawned`] carried the one there is, down this
    /// same channel and before this.
    Started {
        run: u64,
        started: Result<(), lsp::Failure>,
    },
    /// What one question about a place came back with. Which question it was is the
    /// [`Reply`]'s to say; the ticket is the [`LspJob::Ask`]'s, carried through untouched.
    Answered { ticket: Ticket, reply: Reply },
    /// What every name in one file is, and which file. Its own answer and not a `Reply`,
    /// since it is the one question about a file rather than about a place in one.
    Linked {
        run: u64,
        file: Arc<str>,
        links: Result<links::Links, lsp::Failure>,
    },
    /// The server has been told about a file, so what it said about that file before is
    /// what it could work out without it. Not an answer to a question -- an opening is
    /// not one -- but the same shape, since what it does is put the question again.
    Reopened { run: u64, file: Arc<str> },
    /// What the server says the name at one place is. Its own answer and not a `Reply`,
    /// for the reason the links are: it is contents and a range where the four are places.
    Hovered {
        ticket: Ticket,
        said: Result<Option<lsp::Hovered>, lsp::Failure>,
    },
    /// What the project's own settings file said. Named by the directory it was read in
    /// and not by a run: it is about a project and not about a process.
    Settings {
        directory: PathBuf,
        settings: Result<lsp::Settings, lsp::Unreadable>,
    },
}

/// What an answer holds, which is what was asked for: one variant per consumer, each
/// carrying the shape that consumer takes.
///
/// The kind is the variant and not a field beside it, so a question of one kind cannot
/// come back as the other's answer, and neither consumer needs an arm for one that did.
/// A listed one carries the text of every line it names, **read on the worker**: the read
/// blocks, and that is the thread that may block.
pub(crate) enum Reply {
    Followed(Result<Vec<lsp::Place>, lsp::Failure>),
    Listed(Result<references::References, lsp::Failure>),
}

/// The answer to `want`, out of what the server said. The one place the shape of an
/// answer is decided, and it is decided by the question.
///
/// `lines` is the answer's own reader, the one its columns came back off the wire
/// through: a named file's text is got with it, [`source::read_text`] on the worker. A
/// path a server answers with is file input, and two rules for what a source file is
/// would be two ideas of which files this app can show.
pub(crate) fn replied(
    want: lsp::Question,
    places: Result<Vec<lsp::Place>, lsp::Failure>,
    lines: &mut lsp::Lines,
) -> Reply {
    match want {
        lsp::Question::Followed(_) => Reply::Followed(places),
        // Grouped and their lines read with the ask, since that is what the panel draws.
        lsp::Question::Listed(_) => {
            Reply::Listed(places.map(|places| references::of(&places, lines)))
        }
    }
}

/// Put one question to the server there is, and let go of a conversation that has ended.
///
/// [`None`] with no server: there is nobody to ask, so there is no answer to send. A
/// [`lsp::Failure::Broken`] is the conversation itself ending, so the server is dropped
/// here -- the one place that decision is made -- and reported as the failure it is. Every
/// job that says anything to a server goes through this, so a question added later cannot
/// leave a dead conversation in `talking` for the next one to fail against.
fn asked<T>(
    talking: &mut Option<lsp::Server>,
    ask: impl FnOnce(&mut lsp::Server) -> Result<T, lsp::Failure>,
) -> Option<Result<T, lsp::Failure>> {
    let answer = ask(talking.as_mut()?);
    if matches!(answer, Err(lsp::Failure::Broken(_))) {
        *talking = None;
    }
    Some(answer)
}

/// The blocking half, and the only part that talks to a server.
///
/// A closure holding the conversation rather than a plain function, since unlike the other
/// three workers this one has something to keep between jobs. The lock is never contended
/// -- one thread calls this -- and is what lets the seam stay the `Fn` the others are.
pub(crate) fn language_work() -> impl Fn(LspJob) -> Option<LspAnswer> + Send + 'static {
    let talking: Mutex<Option<lsp::Server>> = Mutex::new(None);

    move |job| {
        let mut talking = talking.lock().unwrap_or_else(|held| held.into_inner());
        match job {
            LspJob::Start {
                run,
                directory,
                program,
                settings,
                notes,
                spawned,
            } => {
                // Whatever was there is dropped first, which kills it: two servers over
                // one project would be twice the memory for one answer.
                *talking = None;
                // `send_blocking` and not `try_send`: the channel is bounded, and a note
                // dropped because the app was busy is a control that never stops saying
                // the server is working.
                let told = move |note| {
                    let _ = notes.send_blocking((run, note));
                };
                // The handle is sent on before the handshake, which is what puts it where
                // a stop can reach it; `lsp::start` says why.
                let started =
                    lsp::start(&program, &directory, settings.options(), told, |handle| {
                        let _ = spawned.send_blocking(LspAnswer::Spawned {
                            run,
                            handle: handle.clone(),
                        });
                    });
                let started = match started {
                    Ok(server) => {
                        *talking = Some(server);
                        Ok(())
                    }
                    Err(failure) => Err(failure),
                };
                Some(LspAnswer::Started { run, started })
            }
            LspJob::Ask { ticket, at, want } => {
                // One reader for the whole answer: the columns come back off the wire
                // through it and the rows it will be drawn as are counted through it, so
                // a file an answer names is read once and not once per conversion.
                let mut lines = lsp::Lines::reading(source::read_text);
                let places = asked(&mut talking, |talk| talk.places(want, &at, &mut lines))?;
                Some(LspAnswer::Answered {
                    ticket,
                    reply: replied(want, places, &mut lines),
                })
            }
            LspJob::Tokens { run, file } => {
                // Classified here rather than on the UI thread: it is a walk of every
                // name in the file, and this is the thread that may take its time. Done
                // while the conversation is still in hand, the legend being its.
                let links = asked(&mut talking, |talk| {
                    talk.semantic_tokens(Path::new(&*file))
                        .map(|tokens| links::Links::of(talk.legend(), &tokens))
                })?;
                Some(LspAnswer::Linked { run, file, links })
            }
            LspJob::Hover { ticket, at } => {
                let said = asked(&mut talking, |talk| talk.hover(&at))?;
                Some(LspAnswer::Hovered { ticket, said })
            }
            LspJob::Opened {
                run,
                file,
                language,
            } => {
                let path = PathBuf::from(&*file);
                let told = asked(&mut talking, |talk| {
                    // Asked before the file is read: a server that takes no documents is
                    // one this reads nothing for.
                    if !talk.opens() {
                        return Ok(false);
                    }
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        return Ok(false);
                    };
                    talk.opened(&path, &language, &text).map(|()| true)
                })?;
                // Everything the server said about this file before it had it is what it
                // could work out from the disk, which may have been nothing at all.
                matches!(told, Ok(true)).then_some(LspAnswer::Reopened { run, file })
            }
            LspJob::Closed { file } => {
                asked(&mut talking, |talk| talk.closed(Path::new(&*file)));
                None
            }
            LspJob::ReadSettings { directory } => Some(LspAnswer::Settings {
                settings: lsp::settings_in(&directory),
                directory,
            }),
            LspJob::Stop => {
                *talking = None;
                None
            }
        }
    }
}

/// Whose answer a job is, for [`superseded_as`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum JobKind {
    Following,
    Listing,
    Linking,
    Hovering,
    Settings,
}

/// Which consumer a job's answer is for, and [`None`] for a job that is never dropped.
///
/// **A kind is a consumer and not a question**: `ui::follow` takes a definition or a
/// declaration, never both at once, and the Locations panel draws implementations or
/// references in the one place. A hover is a third consumer and not a fifth question --
/// the pointer crossing a line asks about every name on the way, and only the one it came
/// to rest on is worth a round trip, but none of them is a reader taking back the
/// definition they clicked for. The source pane holds one file's links; a directory typed
/// a letter at a time asks for the project's settings once a keystroke, and only the last
/// of those is about the project that is open.
///
/// The `match` is exhaustive on purpose, and this is the only place the rule is written:
/// a job added with nothing said about superseding would otherwise queue behind every one
/// of its own kind in silence.
fn superseded_as(job: &LspJob) -> Option<JobKind> {
    match job {
        LspJob::Ask {
            want: lsp::Question::Followed(_),
            ..
        } => Some(JobKind::Following),
        LspJob::Ask {
            want: lsp::Question::Listed(_),
            ..
        } => Some(JobKind::Listing),
        LspJob::Tokens { .. } => Some(JobKind::Linking),
        LspJob::Hover { .. } => Some(JobKind::Hovering),
        LspJob::ReadSettings { .. } => Some(JobKind::Settings),
        // Never dropped. The two documents are not questions: they are the difference
        // between what the server holds and what the reader has open, and a dropped one
        // leaves the two disagreeing for good. A start and a stop are what the reader
        // pressed.
        LspJob::Opened { .. } | LspJob::Closed { .. } | LspJob::Start { .. } | LspJob::Stop => None,
    }
}

/// The jobs worth doing, of the one taken off the channel and everything queued behind it.
///
/// Only the last question **of each kind** is kept: a reader clicking twice wants the
/// second answer, and the first is a conversation the second would only wait behind -- but
/// a reader who asks for a name's references has not taken back the definition they asked
/// for, and the two are answered by different parts of the app. What a kind is,
/// [`superseded_as`] says.
pub(crate) fn worth_doing(first: LspJob, queued: impl Iterator<Item = LspJob>) -> Vec<LspJob> {
    let jobs: Vec<LspJob> = std::iter::once(first).chain(queued).collect();
    let mut last: HashMap<JobKind, usize> = HashMap::new();
    for (at, job) in jobs.iter().enumerate() {
        if let Some(kind) = superseded_as(job) {
            last.insert(kind, at);
        }
    }
    jobs.into_iter()
        .enumerate()
        .filter(|(at, job)| superseded_as(job).is_none_or(|kind| last.get(&kind) == Some(at)))
        .map(|(_, job)| job)
        .collect()
}

#[cfg(test)]
mod tests;

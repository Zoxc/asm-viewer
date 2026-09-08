//! Following a name in the source to what it names: whom the question is put to
//! ([`Server`]), the question itself, and the place its answer opens.
//!
//! Two workers stand between the press and the tab moving, so what to do with the answer
//! cannot be worked out when it lands: the reader may have moved on, and Ctrl may no
//! longer be held. It is decided at the press and kept here, `Asking`'s rule in
//! `ui::language` -- what was asked for is what was asked for. Which includes **where**:
//! a press asking for the definition in place asked for it in the tab it was made in,
//! and that tab is raised to take it however the reader has moved between tabs since.
//!
//! One question is remembered, so a reader who clicks twice gets the second answer: the
//! worker already drops all but the last still queued (`worth_doing`), and an answer
//! arriving for a question this no longer holds is an answer to nobody. Which is why the
//! question is held by its **id** and not by the server run it was asked in: a run lasts
//! as long as the server, so two clicks inside one are the ordinary case, and the first
//! click's answer would otherwise be taken for the second's.

use super::*;

/// The question in flight and the place its answer named, as the app holds them.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Follow {
    asked: Option<Asked>,
    /// Where the answer said the name is defined, until [`use_follow`] has taken it: the
    /// place the server named, where the press said it should open, and the tab it was
    /// made in.
    arrived: Option<(Arrival, Reach, Option<DocId>)>,
}

/// A place a followed answer named, with the caret opening it plants already worked out.
///
/// The caret goes on the **name** and not at the head of its line, the reader being taken
/// there to read it. The server counts that column in bytes ([`lsp::Place`]) where a pane
/// counts UTF-16 units, so converting takes the line's text -- which is why the answer
/// carries the caret and not the columns alone. The line is read where the ask is, on the
/// language worker, for the reason the Locations panel's lines are (`src/references.rs`):
/// a read blocks, and that is the thread that may block.
#[derive(Clone, PartialEq)]
pub(crate) struct Arrival {
    pub(crate) place: lsp::Place,
    /// An empty run at the name's first column, in the units the source pane draws in: a
    /// caret there, selecting nothing.
    pub(crate) caret: Range<usize>,
}

impl Arrival {
    /// The places an answer named, each with its caret. `read` answers a file's whole
    /// text and is asked **once per file**, [`source::read_text`] on the worker: a path a
    /// server answers with is file input, and two rules for what a source file is would
    /// be two ideas of which files this app can show.
    ///
    /// A file that will not read leaves the column alone, which is the same column on any
    /// line of ASCII.
    ///
    /// Every place is counted, though [`Follow::answer`] opens only the first: which one
    /// that is, is its rule, and an answer names one place for nearly every name.
    pub(crate) fn of(
        places: Vec<lsp::Place>,
        read: impl Fn(&Path) -> Option<String>,
    ) -> Vec<Arrival> {
        let mut texts: HashMap<PathBuf, Option<String>> = HashMap::new();
        places
            .into_iter()
            .map(|place| {
                let text = texts
                    .entry(place.file.clone())
                    .or_insert_with(|| read(&place.file));
                let at = place.columns.start as usize;
                let start = text
                    .as_deref()
                    .and_then(|text| text.lines().nth((place.line as usize).checked_sub(1)?))
                    .map(|row| chars::columns_of(row, at..at).start)
                    .unwrap_or(at);
                Arrival {
                    place,
                    caret: start..start,
                }
            })
            .collect()
    }
}

/// A question put and not yet answered: which server run it was asked in and which
/// question of that run it is, where it was asked about, and where its answer is to open.
#[derive(Clone, PartialEq)]
struct Asked {
    run: u64,
    id: u64,
    at: Lookup,
    /// Which question was put, which only matters for what an answer naming the line it
    /// was asked on means. See [`Follow::answer`].
    want: lsp::Followed,
    reach: Reach,
    /// The tab the press was made in, so that a [`Reach::InPlace`] answer replaces what
    /// **that** tab shows and not what the tab on screen when it lands does. `None` with
    /// no document tab on screen, which a press inside a pane always has.
    tab: Option<DocId>,
}

impl Follow {
    /// Take the answer to the question this is waiting for, `places` being what the
    /// server said. Whether anything changed, so the caller writes only then.
    ///
    /// An answer for another run, for another question of this one, or for a question
    /// already answered, is an answer to nobody: the reader clicked again, and what they
    /// are owed is the second click's answer. A name the server places nowhere clears the
    /// question and opens nothing: the click was a question and never a promise. So does
    /// a **declaration** placed on the line the question was asked on, which is somewhere
    /// the reader already is.
    pub(crate) fn answer(&mut self, run: u64, id: u64, places: &[Arrival]) -> bool {
        let waiting = self
            .asked
            .as_ref()
            .filter(|asked| asked.run == run && asked.id == id);
        let Some(asked) = waiting else {
            return false;
        };
        let (reach, tab) = (asked.reach, asked.tab);
        // A **declaration** naming the line it was asked on is nowhere to go. A trait's
        // own method declaration is the case: it is a link, since nothing the server says
        // of it tells it from the `impl` item that has the trait to go to, and asking
        // where it is declared then answers with itself. Opening it would put a step on
        // the trail that goes nowhere and a Back that undoes nothing.
        //
        // Only a declaration. A *definition* in the file already shown is an ordinary
        // door -- the same file is a different path through `land` and not a different
        // outcome -- and one that lands on the line it was asked from is a name defined
        // where it is used, which is a place like any other.
        let nowhere = |arrival: &&Arrival| {
            asked.want == lsp::Followed::Declaration
                && arrival.place.file == asked.at.file
                && arrival.place.line == asked.at.line
        };
        self.arrived = places
            .first()
            .filter(|arrival| !nowhere(arrival))
            .map(|arrival| (arrival.clone(), reach, tab));
        self.asked = None;
        true
    }

    /// Give up on the question this is waiting for: the server refused it, or is gone.
    pub(crate) fn give_up(&mut self, run: u64, id: u64) -> bool {
        let waiting = self
            .asked
            .as_ref()
            .is_some_and(|asked| asked.run == run && asked.id == id);
        if waiting {
            self.asked = None;
        }
        waiting
    }
}

/// The question the source rows put, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Following(pub(crate) State<Follow>);

/// Whom a question about a name is put to, in one bundle: the control's state, where a
/// followed name's answer lands, and the way to the worker.
///
/// Not an incidental grouping. Every question about a name -- a followed link, and the
/// three a row's menu offers -- needs all three, and a pane with any of them missing has
/// no server to ask: it draws its text and no links at all, so no link is ever drawn that
/// could not be followed. Taken in one [`try_use_server`], which is why a row's render
/// says "the server is here" once rather than zipping three contexts.
///
/// `Clone` where [`Doors`] is `Copy`: [`LspJobs`] carries channels.
#[derive(Clone)]
pub(crate) struct Server {
    pub(crate) language: State<Language>,
    pub(crate) follow: State<Follow>,
    pub(crate) jobs: LspJobs,
}

/// The server as a component sees it, and [`None`] where any part of it is missing.
///
/// All three contexts are asked for before any is dropped, so this takes the same slots
/// on every render whatever it answers.
pub(crate) fn try_use_server() -> Option<Server> {
    let language = try_consume_context::<Talking>();
    let follow = try_consume_context::<Following>();
    let jobs = try_consume_context::<LspJobs>();
    Some(Server {
        language: language?.0,
        follow: follow?.0,
        jobs: jobs?,
    })
}

/// Follow the name at byte `column` of row `at`, as a press on a link in the source pane
/// does, opening what the answer names where `reach` says.
///
/// Which question it asks is the link's: an item in a trait `impl` asks for the
/// declaration, since its definition is itself and the trait is where a reader following
/// it wants to go (`src/links.rs`). A column over no link asks for a definition, the
/// question nearly every name asks.
pub(crate) fn follow_link(
    server: &Server,
    links: &links::Links,
    open: Open,
    at: &LinePos,
    column: u32,
    reach: Reach,
) {
    let want = links
        .at(at.line, column)
        .and_then(|link| link.asks)
        .unwrap_or(lsp::Followed::Definition);
    follow_name(server, open, Lookup::at(at, column), want, reach);
}

/// Ask where the name at `at` is, to be opened `reach` says when the answer comes. The
/// one writer of [`Follow::asked`].
///
/// `want` is which question: a definition for nearly every name, and a declaration for an
/// item in a trait `impl`, whose definition is itself (`src/links.rs`). Both answers open
/// the same door, which is why both arrive here.
///
/// The tab the press was made in is kept with the question. The pane is the active tab's,
/// which is what `open` is read for, and it is read at the press for the reason the rest
/// of it is.
///
/// With no server there is nobody to ask and nothing is remembered: a question is not
/// what starts one, that being the control the reader presses.
pub(crate) fn follow_name(
    server: &Server,
    open: Open,
    at: Lookup,
    want: lsp::Followed,
    reach: Reach,
) {
    let asked = ask_where(
        server.language,
        &server.jobs,
        at.clone(),
        lsp::Question::Followed(want),
    );
    let Some((run, id)) = asked else {
        return;
    };
    // Bound before the write, the reads above being of other states.
    let tab = open.active_id();
    let mut follow = server.follow;
    let held = follow.peek().clone();
    follow.set(Follow {
        asked: Some(Asked {
            run,
            id,
            at,
            want,
            reach,
            tab,
        }),
        ..held
    });
}

/// Open what the answer named. Called once, at the root, beside `use_land`.
///
/// The arrival itself is [`open_source_place`], which a row of the references panel makes too.
pub(crate) fn use_follow(mut follow: State<Follow>, doors: Doors, places: Places) {
    let open = doors.open;
    use_side_effect(move || {
        // Reading is what wakes this; the write below clears what it read, so the run
        // it wakes finds nothing and stops.
        let arrived = follow.read().arrived.clone();
        let Some((arrival, reach, tab)) = arrived else {
            return;
        };
        follow.write().arrived = None;

        // In place means in the tab the press was made in. The round trip is seconds long
        // while the server reads the project, so the reader may be in another tab by now,
        // and pushing the definition onto that one would replace what a tab nobody asked
        // about is showing. The asking tab is raised to take it instead. An answer to a
        // tab that has closed is an answer to nobody: a tab of its own would be a place
        // nothing is waiting for.
        let asking = tab.filter(|_| reach == Reach::InPlace && tab != open.active_id());
        if let Some(tab) = asking {
            // Bound before the raise below, which writes the state this read.
            let still_open = open.strip.peek().contains(Tab::Document(tab));
            if !still_open {
                return;
            }
            raise(open, tab);
        }

        // The caret is the worker's, counted off the line it sits on there.
        let place = &arrival.place;
        open_source_place(
            doors,
            places,
            &place.file,
            place.line,
            Some(arrival.caret.clone()),
            reach,
        );
    });
}

/// Open `path` as a source-driven tab on `line`, `columns` of it selected, and let the
/// assembly side follow that line.
///
/// The landing is `land`'s, so this is the same arrival every other door makes: the source
/// pane on the line, both panes owed the scroll, and the place on the tab's trail so Back
/// returns to where the reader pressed. What `land` does not do is say which line the
/// assembly side follows, so the drive is written here -- under the place the tab is
/// **at**, which the landing has just made, and not under the file.
///
/// Every door into a *place* in a source file goes through this: the definition an answer
/// named, a row of the references the Locations panel lists, a hit the Search panel found,
/// and the companion a source row's menu offers. A path with no line to land on -- a Files
/// row, a finder row -- is [`open_source_file`]'s instead.
pub(crate) fn open_source_place(
    doors: Doors,
    places: Places,
    path: &Path,
    line: u32,
    columns: Option<Range<usize>>,
    reach: Reach,
) {
    let open = doors.open;
    let file = spelling(open, path);
    let document = Document::Source(file.clone());
    let id = land(
        doors,
        Landing {
            tab: document.clone(),
            at: Some(LinePos {
                file: file.clone(),
                line,
            }),
            // A file and a line: the compiler named no instruction here, and which symbol
            // the line is in is the assembly side's own question.
            address: None,
            columns,
        },
        reach,
    );
    let Some(id) = id else {
        return;
    };
    // Bound to a `let` of its own, so the table's guard is gone before the write.
    let entry = place_at(&open.docs.peek(), id, &document);
    let mut driven = places.driven;
    driven.write().remember((id, entry), line);
}

/// What to name the document opening `path`: the spelling an open source tab already has
/// for that file, and `path`'s own where no tab has one.
///
/// A [`Document::Source`] is compared as text and never canonicalised, so one file reached
/// two ways is one tab only where both ways spell it alike (`src/project.rs`). The server
/// answers with canonical absolute paths; the app's own spelling is a project directory as
/// the reader typed it joined with a Files row, or whatever the debug info said. So a
/// directory typed with a `..`, a `./` or through a symlink -- and on Windows every answer,
/// whose separators are the URI's -- would open a second tab of the file the reader is
/// already reading, splitting its trail, its positions and its driven line across the two.
fn spelling(open: Open, path: &Path) -> Arc<str> {
    // `path` is the same path every time round, so it is reduced once for the whole walk
    // and not once per tab. What is left is one filesystem call per open source tab, on
    // the UI thread. There are a handful of them and this is a press behind a round trip
    // to the server, so it costs what following a link already costs.
    let real = path.canonicalize().ok();
    let held = {
        let docs = open.docs.peek();
        open.ids()
            .into_iter()
            .filter_map(|id| match docs.get(id) {
                Some(Document::Source(file)) => Some(file.clone()),
                _ => None,
            })
            .find(|file| same_file(Path::new(&**file), real.as_deref(), path))
    };
    held.unwrap_or_else(|| Arc::from(path.to_string_lossy().as_ref()))
}

/// Whether `one` names the file `path` does, `real` being `path` reduced or [`None`] where
/// it will not reduce -- a file that is not there to be looked up.
///
/// Spelled alike is the answer without asking. Otherwise both have to reduce to one path,
/// so two that will not reduce are the same only when they are spelled alike.
///
/// The reduction of `path` is the caller's and not taken here: it is one path against
/// every open tab, and taking it per tab is the same call over again.
fn same_file(one: &Path, real: Option<&Path>, path: &Path) -> bool {
    one == path || matches!((one.canonicalize(), real), (Ok(one), Some(real)) if one == real)
}

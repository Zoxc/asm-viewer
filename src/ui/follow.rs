//! Following a name in the source to what it names: whom the question is put to
//! ([`Server`]), the question itself, and where its answer opens.
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
//! question is held by its whole [`Ticket`] and not by the server run alone: a run lasts
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
    arrived: Option<(lsp::Place, Reach, Option<DocId>)>,
}

/// A question put and not yet answered: the [`Ticket`] it went out under, where it was
/// asked about, and where its answer is to open.
#[derive(Clone, PartialEq)]
struct Asked {
    ticket: Ticket,
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
    pub(crate) fn answer(&mut self, ticket: Ticket, places: &[lsp::Place]) -> bool {
        let waiting = self.asked.as_ref().filter(|asked| asked.ticket == ticket);
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
        //
        // The file is compared as the doors compare one, the server's spelling and the
        // tab's being two spellings of one file as often as not.
        let nowhere = |place: &&lsp::Place| {
            asked.want == lsp::Followed::Declaration
                && place.line == asked.at.line
                && same_file(&asked.at.file, reduced(&place.file).as_deref(), &place.file)
        };
        self.arrived = places
            .first()
            .filter(|place| !nowhere(place))
            .map(|place| (place.clone(), reach, tab));
        self.asked = None;
        true
    }

    /// The question has gone out. Whether anything changed, so the caller writes only
    /// then -- always, [`ask_where`] minting a ticket per question. An answer that has
    /// landed and not yet been taken stays: it is the last press's, and this question is
    /// not answered yet.
    fn asking(&mut self, asked: Asked) -> bool {
        self.asked = Some(asked);
        true
    }

    /// Give up on the question this is waiting for: the server refused it, or is gone.
    pub(crate) fn give_up(&mut self, ticket: Ticket) -> bool {
        let waiting = self
            .asked
            .as_ref()
            .is_some_and(|asked| asked.ticket == ticket);
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
    let language = use_try_consume::<Talking>();
    let follow = use_try_consume::<Following>();
    let jobs = use_try_consume::<LspJobs>();
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
    column: usize,
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
    let Some(ticket) = asked else {
        return;
    };
    let tab = open.now().map(|(id, _)| id);
    let asked = Asked {
        ticket,
        at,
        want,
        reach,
        tab,
    };
    write_if(server.follow, |follow| follow.asking(asked));
}

/// Open what the answer named. Called once, at the root, beside `use_land`.
///
/// The arrival itself is [`open_source_place`], which a row of the references panel makes too.
pub(crate) fn use_follow(mut follow: State<Follow>, doors: Doors) {
    let open = doors.open;
    use_side_effect(move || {
        // Reading is what wakes this; the write below clears what it read, so the run
        // it wakes finds nothing and stops.
        let arrived = follow.read().arrived.clone();
        let Some((place, reach, tab)) = arrived else {
            return;
        };
        follow.write().arrived = None;

        // In place means in the tab the press was made in. The round trip is seconds long
        // while the server reads the project, so the reader may be in another tab by now,
        // and pushing the definition onto that one would replace what a tab nobody asked
        // about is showing. The asking tab is raised to take it instead. An answer to a
        // tab that has closed is an answer to nobody: a tab of its own would be a place
        // nothing is waiting for.
        let asking = tab.filter(|_| reach == Reach::InPlace && tab != open.now().map(|(id, _)| id));
        if let Some(tab) = asking {
            // Bound before the raise below, which writes the state this read.
            let still_open = open.strip.peek().contains(Tab::Document(tab));
            if !still_open {
                return;
            }
            raise(open, tab);
        }

        // The caret goes on the name and not at the head of its line, the reader being
        // taken there to read it.
        let at = place.columns.start;
        open_source_place(doors, &place.file, place.line, Some(at..at), reach);
    });
}

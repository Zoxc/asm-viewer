//! Following a name in the source to what it names: the question put to the language
//! server, and the place its answer opens.
//!
//! Two workers stand between the press and the tab moving, so what to do with the answer
//! cannot be worked out when it lands: the reader may have moved on, and Ctrl may no
//! longer be held. It is decided at the press and kept here, `Asking`'s rule in
//! `ui::language` -- what was asked for is what was asked for.
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
    /// place the server named, and where the press said it should open.
    arrived: Option<(lsp::Place, Reach)>,
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
    want: Wanted,
    reach: Reach,
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
    pub(crate) fn answer(&mut self, run: u64, id: u64, places: &[lsp::Place]) -> bool {
        let waiting = self
            .asked
            .as_ref()
            .filter(|asked| asked.run == run && asked.id == id);
        let Some(asked) = waiting else {
            return false;
        };
        let reach = asked.reach;
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
        let nowhere = |place: &&lsp::Place| {
            asked.want == Wanted::Declaration
                && place.file == asked.at.file
                && place.line == asked.at.line.saturating_add(1)
        };
        self.arrived = places
            .first()
            .filter(|place| !nowhere(place))
            .map(|place| (place.clone(), reach));
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

/// Ask where the name at `at` is, to be opened `reach` says when the answer comes. The
/// one writer of [`Follow::asked`].
///
/// `want` is which question: a definition for nearly every name, and a declaration for an
/// item in a trait `impl`, whose definition is itself (`src/links.rs`). Both answers open
/// the same door, which is why both arrive here.
///
/// With no server there is nobody to ask and nothing is remembered: a question is not
/// what starts one, that being the control the reader presses.
pub(crate) fn follow_name(
    language: State<Language>,
    mut follow: State<Follow>,
    jobs: &LspJobs,
    at: Lookup,
    want: Wanted,
    reach: Reach,
) {
    let Some((run, id)) = ask_where(language, jobs, at.clone(), want) else {
        return;
    };
    // Bound before the write, the read above being of another state.
    let held = follow.peek().clone();
    follow.set(Follow {
        asked: Some(Asked {
            run,
            id,
            at,
            want,
            reach,
        }),
        ..held
    });
}

/// Open what the answer named. Called once, at the root, beside `use_land`.
///
/// The arrival itself is [`open_source_place`], which a row of the references panel makes too.
pub(crate) fn use_follow(
    mut follow: State<Follow>,
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    driven: State<Driven>,
) {
    use_side_effect(move || {
        // Reading is what wakes this; the write below clears what it read, so the run
        // it wakes finds nothing and stops.
        let arrived = follow.read().arrived.clone();
        let Some((place, reach)) = arrived else {
            return;
        };
        follow.write().arrived = None;

        // An empty run at the column the name starts at: a caret at the head of the
        // definition and not at the head of its line, the reader being taken there to
        // read it and not to copy it. Those columns and a row's are both UTF-16 units,
        // so nothing is converted.
        let start = place.columns.start as usize;
        let caret = start..start;
        open_source_place(
            open,
            visits,
            marked,
            landing,
            plant,
            driven,
            &place.file,
            place.line,
            Some(caret),
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
/// Both doors into a source file go through this: the definition an answer named, and a
/// row of the references the Locations panel lists.
pub(crate) fn open_source_place(
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    mut driven: State<Driven>,
    path: &Path,
    line: u32,
    columns: Option<Range<usize>>,
    reach: Reach,
) {
    let file: Arc<str> = Arc::from(path.to_string_lossy().as_ref());
    let document = Document::Source(file.clone());
    let id = land(
        open,
        visits,
        marked,
        landing,
        plant,
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
    driven.write().remember((id, entry), line);
}

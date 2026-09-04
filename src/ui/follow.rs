//! Following a name in the source to what it names: the question put to the language
//! server, and the place its answer opens.
//!
//! Two workers stand between the press and the tab moving, so what to do with the answer
//! cannot be worked out when it lands: the reader may have moved on, and Ctrl may no
//! longer be held. It is decided at the press and kept here, `Asking`'s rule in
//! `ui::language` -- what was asked for is what was asked for.
//!
//! One question is remembered, so a reader who clicks twice gets the second answer: the
//! worker already drops all but the last (`worth_doing`), and an answer arriving for a
//! question this no longer holds is an answer to nobody.

use super::*;

/// The question in flight and the place its answer named, as the app holds them.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Follow {
    asked: Option<Asked>,
    /// Where the answer said the name is defined, until [`use_follow`] has taken it: the
    /// place the server named, and where the press said it should open.
    arrived: Option<(lsp::Place, Reach)>,
}

/// A question put and not yet answered: which server run it was asked in, where it was
/// asked about, and where its answer is to open.
#[derive(Clone, PartialEq)]
struct Asked {
    run: u64,
    at: Lookup,
    reach: Reach,
}

impl Follow {
    /// Take the answer to the question this is waiting for, `places` being what the
    /// server said. Whether anything changed, so the caller writes only then.
    ///
    /// An answer for another run, or for a question already answered, is an answer to
    /// nobody. A name the server places nowhere clears the question and opens nothing:
    /// the click was a question and never a promise.
    pub(crate) fn answer(&mut self, run: u64, places: &[lsp::Place]) -> bool {
        let Some(asked) = self.asked.as_ref().filter(|asked| asked.run == run) else {
            return false;
        };
        let reach = asked.reach;
        self.arrived = places.first().map(|place| (place.clone(), reach));
        self.asked = None;
        true
    }

    /// Give up on the question this is waiting for: the server refused it, or is gone.
    pub(crate) fn give_up(&mut self, run: u64) -> bool {
        let waiting = self.asked.as_ref().is_some_and(|asked| asked.run == run);
        if waiting {
            self.asked = None;
        }
        waiting
    }
}

/// The question the source rows put, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Following(pub(crate) State<Follow>);

/// Ask where the name at `at` is defined, to be opened `reach` says when the answer
/// comes. The one writer of [`Follow::asked`].
///
/// With no server there is nobody to ask and nothing is remembered: a question is not
/// what starts one, that being the control the reader presses.
pub(crate) fn follow_name(
    language: State<Language>,
    mut follow: State<Follow>,
    jobs: &LspJobs,
    at: Lookup,
    reach: Reach,
) {
    let Some(run) = ask_definition(language, jobs, at.clone()) else {
        return;
    };
    // Bound before the write, the read above being of another state.
    let held = follow.peek().clone();
    follow.set(Follow {
        asked: Some(Asked { run, at, reach }),
        ..held
    });
}

/// Open what the answer named. Called once, at the root, beside `use_land`.
///
/// The landing is `land`'s, so following a name is the same arrival as every other door
/// makes: the source pane on the line, its caret at the column the server named, both
/// panes owed the scroll, and the place on the tab's trail so Back returns to the call.
/// What `land` does not do is say which line the assembly side follows, so the drive is
/// written here -- under the place the tab is **at**, which the landing has just made,
/// and not under the file.
pub(crate) fn use_follow(
    mut follow: State<Follow>,
    open: Open,
    visits: State<Visits>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    mut driven: State<Driven>,
) {
    use_side_effect(move || {
        // Reading is what wakes this; the write below clears what it read, so the run
        // it wakes finds nothing and stops.
        let arrived = follow.read().arrived.clone();
        let Some((place, reach)) = arrived else {
            return;
        };
        follow.write().arrived = None;

        let line = place.line;
        let file: Arc<str> = Arc::from(place.file.to_string_lossy().as_ref());
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
                // A file and a line: the compiler named no instruction here, and which
                // symbol the line is in is the assembly side's own question.
                address: None,
                // An empty run at the column the server named: a caret at the start
                // of the name and not at the start of its line. That column and a
                // row's are both UTF-16 units, so nothing is converted.
                columns: Some(place.column as usize..place.column as usize),
            },
            reach,
        );
        let Some(id) = id else {
            return;
        };
        // Bound to a `let` of its own, so the table's guard is gone before the write.
        let entry = place_at(&open.docs.peek(), id, &document);
        driven.write().remember((id, entry), line);
    });
}

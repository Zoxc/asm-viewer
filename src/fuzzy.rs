//! Characters in order: what the file finder's box asks of a path, where it hit, and how
//! well. Framework-free.
//!
//! Not `filter.rs`. A filter bar asks whether a name *contains* what was typed, and
//! compiles to one regex; this asks whether a path holds the characters typed **in
//! order**, gaps allowed, so that `srcuivw` reaches `src/ui/files_view.rs` -- a question
//! no regex a reader would type says. The two live apart because they answer different
//! things and are worth pinning separately.
//!
//! Every path a query lets through gets a [`Score`], whose `Ord` puts the best first.

use std::iter::once;
use std::ops::Range;

/// What was typed, as the characters it asks of a path, folded on the way in: the finder
/// asks the same query of every walked file on every keystroke, so the fold is paid once
/// per box and not once per path.
pub struct Query {
    wanted: Vec<Wanted>,
}

/// One character a query asks for, lower-cased.
enum Wanted {
    /// What it folds to, for a character that folds to one.
    Folded(char),
    /// What it folds to, for the few that fold to more than one: `İ` folds to an `i` and a
    /// combining dot.
    Several(Box<[char]>),
}

/// Where a query hit a path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    /// How well, for the list to order its rows by.
    pub score: Score,
    /// The runs that matched, as byte ranges into the path and in order: what the row
    /// marks.
    pub marks: Vec<Range<usize>>,
}

/// How well a path matched. `Ord` puts the best first, comparing field by field in the
/// order they are written: inside the file's own name beats reaching into a directory
/// above it, a run beats the same characters spread out, a start beats inside a word,
/// and between two of a kind the shorter path wins -- the path the query says most of.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Score {
    place: Place,
    /// How many runs the matched characters fall into; one is contiguous.
    runs: usize,
    start: Start,
    /// The whole path's length in bytes.
    length: usize,
}

/// How much of the path the match reached into. The order of the variants is the order of
/// the scores.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Place {
    /// Every matched character is in the file's own name.
    Name,
    /// The match reaches into a directory above it.
    Above,
}

/// Where the match's first character sits in the word it is in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Start {
    /// A word's first character: the path's own start, one after a separator, or the
    /// capital that starts a part of a camel-cased name.
    Word,
    /// Anywhere else.
    Inside,
}

impl Query {
    /// What was typed, or [`None`] where nothing was.
    ///
    /// Nothing typed is not a query that matches everything but no query at all, which is
    /// the caller's own case to draw: `filter.rs` draws the same line.
    pub fn new(typed: &str) -> Option<Query> {
        let wanted: Vec<Wanted> = typed.chars().map(Wanted::new).collect();
        (!wanted.is_empty()).then_some(Query { wanted })
    }

    /// Whether the query's characters appear in `shown` in order, and how well, with
    /// `name_at` the byte the file's own name starts at ([`crate::walk::Found`]).
    pub fn find(&self, shown: &str, name_at: usize) -> Option<Hit> {
        let forward = self.forward(shown)?;
        let end = forward.last().map(|&at| at + width(shown, at))?;
        let forward = scored(shown, name_at, forward);
        // The pass back cannot score better, and a tie would keep this one anyway.
        if forward.score.unbeatable() {
            return Some(forward);
        }
        let tightened = scored(shown, name_at, self.tightened(&shown[..end]));

        // Both, and the better of the two. Reading the path once takes each character as
        // early as it can go, which is what puts `sv`'s `s` on `src`; walking back from
        // there takes each as late as it can, which is what pulls `ui` together into the
        // directory it names. Neither wins everywhere, and scoring is what says which.
        [forward, tightened]
            .into_iter()
            .min_by(|a, b| a.score.cmp(&b.score))
    }

    /// Where each of the query's characters matched reading the path once, each as early
    /// as it can go, or [`None`] where the query does not fit at all.
    fn forward(&self, shown: &str) -> Option<Vec<usize>> {
        let mut places = Vec::with_capacity(self.wanted.len());
        for (index, character) in shown.char_indices() {
            if self.wanted[places.len()].matches(character) {
                places.push(index);
                if places.len() == self.wanted.len() {
                    return Some(places);
                }
            }
        }
        None
    }

    /// Where each of the query's characters matched, walking back from the end of the
    /// earliest whole match so that each sits as late as it can: what pulls them together
    /// into runs. Walking back from the end of the *path* instead would take `ui`'s `i`
    /// from `files_view` four words past the directory the reader was typing.
    fn tightened(&self, upto: &str) -> Vec<usize> {
        let mut places = vec![0; self.wanted.len()];
        let mut at = self.wanted.len();
        for (index, character) in upto.char_indices().rev() {
            if at > 0 && self.wanted[at - 1].matches(character) {
                at -= 1;
                places[at] = index;
            }
        }
        places
    }
}

impl Wanted {
    /// A typed character, folded here so that no path pays for it.
    fn new(character: char) -> Wanted {
        let mut folded = character.to_lowercase();
        match (folded.next(), folded.next()) {
            (Some(one), None) => Wanted::Folded(one),
            _ => Wanted::Several(character.to_lowercase().collect()),
        }
    }

    /// Whether a path's character is the one asked for, to a reader who did not hold
    /// Shift. Only the path's side is folded, and not even that where the character is
    /// already what was asked for: a character a fold produced folds to itself.
    fn matches(&self, character: char) -> bool {
        match self {
            Wanted::Folded(wanted) => {
                *wanted == character || character.to_lowercase().eq(once(*wanted))
            }
            Wanted::Several(wanted) => character.to_lowercase().eq(wanted.iter().copied()),
        }
    }
}

impl Score {
    /// Whether no other placement of the query in the path can score better: the best
    /// value of each of the three fields a placement decides, one run being the fewest a
    /// match can fall into. The fourth is the path's own length, the same whichever
    /// placement is scored.
    fn unbeatable(&self) -> bool {
        self.place == Place::Name && self.runs == 1 && self.start == Start::Word
    }
}

/// A path, its matched runs and how well they scored.
fn scored(shown: &str, name_at: usize, places: Vec<usize>) -> Hit {
    let marks = runs(shown, &places);
    let first = places[0];
    Hit {
        score: Score {
            place: if first >= name_at {
                Place::Name
            } else {
                Place::Above
            },
            runs: marks.len(),
            start: start_at(shown, first),
            length: shown.len(),
        },
        marks,
    }
}

/// The matched characters gathered into the runs they form, as byte ranges into `shown`.
fn runs(shown: &str, places: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &at in places {
        let end = at + width(shown, at);
        match runs.last_mut() {
            Some(run) if run.end == at => run.end = end,
            _ => runs.push(at..end),
        }
    }
    runs
}

/// How many bytes the character at `at` takes, one for a byte that starts nothing.
fn width(shown: &str, at: usize) -> usize {
    shown[at..].chars().next().map(char::len_utf8).unwrap_or(1)
}

/// Whether the match starting at `start` starts a word: the path's own start, a character
/// after a separator, or the capital a camel-cased name's next part begins with.
fn start_at(shown: &str, start: usize) -> Start {
    if start == 0 {
        return Start::Word;
    }
    let before = shown[..start].chars().next_back();
    let first = shown[start..].chars().next();
    match (before, first) {
        (Some(before), _) if separates(before) => Start::Word,
        (Some(before), Some(first)) if !before.is_uppercase() && first.is_uppercase() => {
            Start::Word
        }
        _ => Start::Inside,
    }
}

/// What ends a word in a path: the separator, and the punctuation a file name is built of.
fn separates(character: char) -> bool {
    matches!(character, '/' | '\\' | '_' | '-' | '.' | ' ')
}

#[cfg(test)]
mod tests;

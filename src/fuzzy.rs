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

use std::ops::Range;

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

/// Whether `query`'s characters appear in `shown` in order, and how well, with `name_at`
/// the byte the file's own name starts at ([`crate::walk::Found`]).
///
/// Nothing typed is not a query that matches everything but no query at all, which is the
/// caller's own case to draw: `filter.rs` draws the same line.
pub fn find(query: &str, shown: &str, name_at: usize) -> Option<Hit> {
    let wanted: Vec<char> = query.chars().collect();
    if wanted.is_empty() {
        return None;
    }

    let forward = forward(&wanted, shown)?;
    let end = forward.last().map(|&at| at + width(shown, at))?;
    let tightened = tightened(&wanted, &shown[..end]);

    // Both, and the better of the two. Reading the path once takes each character as
    // early as it can go, which is what puts `sv`'s `s` on `src`; walking back from
    // there takes each as late as it can, which is what pulls `ui` together into the
    // directory it names. Neither wins everywhere, and scoring is what says which.
    [forward, tightened]
        .into_iter()
        .map(|places| scored(shown, name_at, places))
        .min_by(|a, b| a.score.cmp(&b.score))
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

/// Where each of the query's characters matched reading the path once, each as early as
/// it can go, or [`None`] where the query does not fit at all.
fn forward(wanted: &[char], shown: &str) -> Option<Vec<usize>> {
    let mut places = Vec::with_capacity(wanted.len());
    for (index, character) in shown.char_indices() {
        if same(wanted[places.len()], character) {
            places.push(index);
            if places.len() == wanted.len() {
                return Some(places);
            }
        }
    }
    None
}

/// Where each of the query's characters matched, walking back from the end of the
/// earliest whole match so that each sits as late as it can: what pulls them together
/// into runs. Walking back from the end of the *path* instead would take `ui`'s `i` from
/// `files_view` four words past the directory the reader was typing.
fn tightened(wanted: &[char], upto: &str) -> Vec<usize> {
    let mut places = vec![0; wanted.len()];
    let mut at = wanted.len();
    for (index, character) in upto.char_indices().rev() {
        if at > 0 && same(wanted[at - 1], character) {
            at -= 1;
            places[at] = index;
        }
    }
    places
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

/// Whether two characters are the same to a reader who did not hold Shift.
fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests;

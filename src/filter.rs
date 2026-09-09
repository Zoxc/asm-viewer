//! What the filter bar under each of the sidebar lists is asking for, what answers it, and
//! how well each answer matched.
//!
//! Every filter compiles to one [`regex::Regex`], the plain ones included, because the
//! three toggles *are* three regex constructs — so they compose instead of being four
//! hand-written search loops. It is also the faster answer over 151k demangled names. The
//! same regex ranks: where its first match starts in a name is the [`Rank`] a list under a
//! filter orders its rows by. [`Filtered`] is that ordering: a list, and where in it the
//! names that matched are, best first.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::shared::Shared;

/// One list's filter: what was typed, and the three toggles that say how to read it.
#[derive(Clone, Default, PartialEq)]
pub struct Filter {
    pub pattern: String,
    pub case_sensitive: bool,
    /// The pattern has to be a whole word: `\b` on both ends of the *whole* pattern.
    pub whole_word: bool,
    /// The pattern is a regular expression rather than text to be found literally.
    pub regex: bool,
}

impl Filter {
    /// The pattern as a regular expression: what the two toggles that are *written* into
    /// the expression come to, leaving the third to the builder below.
    ///
    /// Its own function because the source search compiles the same expression with
    /// another crate's builder (`src/search.rs`), and the two searches must agree about
    /// what a toggle means. `grep-regex` has a `word` flag of its own and it is
    /// deliberately looser than `\b`, so this is what is handed over instead.
    pub fn expression(&self) -> String {
        let expression = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            // The group is load-bearing and must be non-capturing: `\ba|b\b` would bind
            // the boundaries to the first and last branch only.
            return format!(r"\b(?:{expression})\b");
        }
        expression
    }

    /// `case_insensitive` is a flag on the builder rather than a `(?i)` prefix, so a regex
    /// carrying its own `(?i)`/`(?-i)` still overrides it for the part it covers.
    pub fn matcher(&self) -> Matcher {
        if self.pattern.is_empty() {
            return Matcher::Everything;
        }

        match RegexBuilder::new(&self.expression())
            .case_insensitive(!self.case_sensitive)
            .build()
        {
            Ok(regex) => Matcher::Pattern(regex),
            Err(error) => Matcher::Invalid(message(&error)),
        }
    }
}

/// A [`Filter`] compiled into the question it asks of each row.
pub enum Matcher {
    /// Nothing was typed. Kept apart from a pattern that happens to match everything so
    /// that a list with no filter on it can skip the pass entirely.
    Everything,
    Pattern(Regex),
    /// A pattern that will not compile, with what is wrong with it. A state of its own,
    /// because both of the others are lies a half-typed `(` would tell: matching everything
    /// hides the mistake and matching nothing looks like an empty list.
    Invalid(String),
}

impl Matcher {
    pub fn matches(&self, text: &str) -> bool {
        match self {
            Matcher::Everything => true,
            Matcher::Pattern(regex) => regex.is_match(text),
            Matcher::Invalid(_) => false,
        }
    }

    /// Where in `text` the pattern matched, as byte ranges in order. What a row marks, so
    /// a reader can see why it is in the list.
    ///
    /// Empty for a filter that matches everything -- nothing was typed, so nothing is
    /// marked -- and for one that will not compile. An empty match is dropped rather than
    /// marked: a pattern like `a*` matches nothing at every position, and a mark of no
    /// width is a wash of no width.
    pub fn marks(&self, text: &str) -> Vec<Range<usize>> {
        match self {
            Matcher::Everything | Matcher::Invalid(_) => Vec::new(),
            Matcher::Pattern(regex) => regex
                .find_iter(text)
                .filter(|found| !found.is_empty())
                .map(|found| found.range())
                .collect(),
        }
    }

    /// Whether the pattern marks anything in `text`: [`marks`](Self::marks) asked for a
    /// yes or no, which allocates nothing and stops at the first mark.
    ///
    /// **Not [`matches`](Self::matches)**, which says `true` twice where this says
    /// `false`: for a filter that matches everything, nothing having been typed and so
    /// nothing marked, and for a zero-width match like `a*`, which is a match with no
    /// width to wash.
    pub fn marked(&self, text: &str) -> bool {
        match self {
            Matcher::Everything | Matcher::Invalid(_) => false,
            Matcher::Pattern(regex) => regex.find_iter(text).any(|found| !found.is_empty()),
        }
    }

    /// What is wrong with the pattern, for the bar to show.
    pub fn error(&self) -> Option<&str> {
        match self {
            Matcher::Invalid(message) => Some(message),
            _ => None,
        }
    }

    /// How well `text` matches, `None` where it does not: [`matches`](Self::matches) with
    /// an order on its `true`. One `find` and not an `is_match` first -- a name that does
    /// not match is scanned whole either way, and the match's start is what the rank is.
    pub fn rank(&self, text: &str) -> Option<Rank> {
        let tier = match self {
            Matcher::Everything => Tier::Inside,
            Matcher::Invalid(_) => return None,
            Matcher::Pattern(regex) => {
                let found = regex.find(text)?;
                tier_at(text, found.start(), found.is_empty())
            }
        };

        Some(Rank {
            tier,
            length: text.len(),
        })
    }
}

/// How well a name matched, for a list under a filter to order its rows by: `Ord` puts
/// the best first. A match at the start of the name beats one at the start of a word,
/// which beats one inside a word, and between two of a kind the shorter name wins -- the
/// name the pattern says most of. Every name a filter lets through has one, so a list
/// sorted by it and then by its own order is total and deterministic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Rank {
    tier: Tier,
    length: usize,
}

/// Where the first match starts. The order of the variants is the order of the ranks.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Tier {
    /// At the name's first character.
    Prefix,
    /// At a word boundary -- regex's `\b`, the Word toggle's own notion, so `::`, `<` and
    /// a space bound a word and `_` does not. The boundary is asked of the match's start
    /// only; where it ends is the toggle's business.
    Word,
    /// Anywhere else. Also an empty match, which starts nowhere in particular.
    Inside,
}

fn tier_at(text: &str, start: usize, empty: bool) -> Tier {
    if empty {
        return Tier::Inside;
    }
    if start == 0 {
        return Tier::Prefix;
    }
    let before = text[..start].chars().next_back();
    let first = text[start..].chars().next();
    match (before, first) {
        (Some(before), Some(first)) if is_word(before) != is_word(first) => Tier::Word,
        _ => Tier::Inside,
    }
}

/// A word character as regex's `\b` counts them.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What a filter leaves of a list: the list itself, and where in it the names that
/// matched are, best match first. Indices rather than a second `Vec<T>` (115k entries in
/// the app's own symbol list), and `None` for no filter at all, which costs no pass, no
/// sort and no allocation, and keeps the list in its own order.
///
/// Generic over the element, since more than one list is ranked this way and the only
/// thing a filter asks of one is the name it is drawn under.
///
/// Two are equal only where both halves are the same build, which is [`Shared`]'s rule:
/// a fresh one is what tells a list to draw its rows again.
pub struct Filtered<T> {
    list: Shared<T>,
    matches: Option<Shared<usize>>,
}

/// Written out rather than derived: a derived `Clone` would ask `T: Clone` and a derived
/// `PartialEq` `T: PartialEq`, and neither half needs either. A [`Shared`] is cloned and
/// compared by its pointer whatever it holds.
impl<T> Clone for Filtered<T> {
    fn clone(&self) -> Self {
        Filtered {
            list: self.list.clone(),
            matches: self.matches.clone(),
        }
    }
}

impl<T> PartialEq for Filtered<T> {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list && self.matches == other.matches
    }
}

impl<T> Filtered<T> {
    /// Filters on the name `name` gives each element -- for a symbol the one the row
    /// shows, demangled where it has one -- and orders what is left by
    /// its [`Rank`], the list's own order breaking ties, so the sort is deterministic and
    /// `sort_unstable` is safe.
    pub fn new(list: Shared<T>, matcher: &Matcher, name: impl Fn(&T) -> &str) -> Self {
        let matches = match matcher {
            Matcher::Everything => None,
            matcher => {
                let mut ranked: Vec<(Rank, usize)> = list
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| Some((matcher.rank(name(item))?, index)))
                    .collect();
                ranked.sort_unstable();
                Some(
                    ranked
                        .into_iter()
                        .map(|(_, index)| index)
                        .collect::<Vec<usize>>()
                        .into(),
                )
            }
        };

        Filtered { list, matches }
    }

    /// The whole list, filter or none: what a row is handed beside its index.
    pub fn list(&self) -> &Shared<T> {
        &self.list
    }

    /// How many rows there are, which is what the `VirtualScrollView` is given.
    pub fn len(&self) -> usize {
        self.matches
            .as_ref()
            .map_or(self.list.len(), |matches| matches.len())
    }

    /// Which element the row at `row` is.
    pub fn index(&self, row: usize) -> usize {
        self.matches.as_ref().map_or(row, |matches| matches[row])
    }

    /// The element the row at `row` draws, `None` past the end -- which is where a
    /// keyboard step off the last row asks.
    pub fn at(&self, row: usize) -> Option<&T> {
        (row < self.len()).then(|| &self.list[self.index(row)])
    }
}

/// The one line of a `regex` error worth putting in a filter bar: its `Display` is a
/// four-line report, of which the sentence is the last non-empty line, prefixed `error:`.
fn message(error: &regex::Error) -> String {
    let text = error.to_string();
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(&text)
        .trim();

    line.strip_prefix("error:")
        .unwrap_or(line)
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests;

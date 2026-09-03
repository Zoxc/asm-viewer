//! What the filter bar under each of the sidebar lists is asking for, what answers it, and
//! how well each answer matched.
//!
//! Every filter compiles to one [`regex::Regex`], the plain ones included, because the
//! three toggles *are* three regex constructs — so they compose instead of being four
//! hand-written search loops. It is also the faster answer over 151k demangled names. The
//! same regex ranks: where its first match starts in a name is the [`Rank`] a list under a
//! filter orders its rows by.

use regex::{Regex, RegexBuilder};

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
    /// `case_insensitive` is a flag on the builder rather than a `(?i)` prefix, so a regex
    /// carrying its own `(?i)`/`(?-i)` still overrides it for the part it covers.
    pub fn matcher(&self) -> Matcher {
        if self.pattern.is_empty() {
            return Matcher::Everything;
        }

        let mut expression = if self.regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        if self.whole_word {
            // The group is load-bearing and must be non-capturing: `\ba|b\b` would bind
            // the boundaries to the first and last branch only.
            expression = format!(r"\b(?:{expression})\b");
        }

        match RegexBuilder::new(&expression)
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

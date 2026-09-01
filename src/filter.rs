//! What the filter bar under each of the sidebar lists is asking for, and what answers it.
//!
//! Every filter compiles to one [`regex::Regex`], the plain ones included, because the
//! three toggles *are* three regex constructs — so they compose instead of being four
//! hand-written search loops. It is also the faster answer over 151k demangled names.

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

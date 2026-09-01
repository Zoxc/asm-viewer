//! What the filter bar under each of the sidebar lists is asking for, and what answers it.
//!
//! This module is deliberately **framework-free** — no freya types appear here — so it can
//! move into a crate of its own alongside the rest of the non-UI code. A [`Filter`] is a
//! view of a list and never part of the session, so nothing here is serialized either.
//!
//! Every filter compiles to a [`regex::Regex`], the plain ones included: the three toggles
//! the goal asks for *are* three regex constructs — `(?i)` is "match case" turned off,
//! `\b…\b` is "whole word", and the third one is whether the pattern is read as a pattern
//! at all — so escaping a literal pattern and letting one engine answer all four
//! combinations is what makes them compose instead of four hand-written search loops that
//! agree with each other by inspection. It is also the faster answer, measured on
//! `viewer-sample`'s 151k demangled names (18 MB of text): 3 ms for a case-sensitive
//! literal against 3.7 ms for `str::contains`, and 8 ms case-insensitively against 7.4 ms
//! for lowercasing both sides — with the worst case seen, a whole-word single letter, at
//! 22 ms, which is one keystroke's worth of work and not one frame's.

use regex::{Regex, RegexBuilder};

/// One list's filter: what was typed, and the three toggles that say how to read it.
///
/// The default is what a list that has never been filtered has, and `matcher` answers
/// [`Matcher::Everything`] for it — so "no filter" costs no pass over the list at all.
#[derive(Clone, Default, PartialEq)]
pub struct Filter {
    pub pattern: String,
    /// Case matters. Off by default, which is what a search box is expected to do.
    pub case_sensitive: bool,
    /// The pattern has to be a whole word: `\b` on both ends of it. With `regex` on it
    /// wraps the user's whole pattern rather than being abandoned, which is what an
    /// alternation needs (`\b(?:a|b)\b`, never `\ba|b\b`).
    pub whole_word: bool,
    /// The pattern is a regular expression rather than text to be found literally.
    pub regex: bool,
}

impl Filter {
    /// The three toggles compose, so this is one expression built in three steps and
    /// never four cases.
    ///
    /// `case_insensitive` is set as a flag on the builder rather than as a `(?i)` prefix
    /// on purpose: a flag is the pattern's starting state, so a regex carrying its own
    /// `(?i)`/`(?-i)` still overrides it for the part it covers, which a prefix wrapped
    /// around the whole thing could not.
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
    /// A pattern that will not compile, with what is wrong with it.
    ///
    /// A state of its own rather than a fallback to one of the other two, because both of
    /// those are lies a half-typed `(` would tell: matching everything hides the mistake
    /// and matching nothing looks like a list with nothing in it. The bar shows the
    /// message instead.
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

/// The one line of a `regex` error worth putting in a filter bar.
///
/// `regex::Error`'s own `Display` is a four-line report — the pattern, a caret under the
/// offending byte, and then the sentence — which is right for a terminal and far too tall
/// for a strip under a list. The sentence is the last non-empty line of it, and the crate
/// prefixes that with `error:` where the whole thing is already labelled as one.
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

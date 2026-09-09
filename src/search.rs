//! The project's directory searched for a pattern: the walk, the match, and the hits the
//! panel draws. Framework-free.
//!
//! The walk is [`crate::walk`]'s, shared with the file finder so that both readers of a
//! project's directory agree about what is in it. What is here is the reading: ripgrep's
//! own `grep-searcher` and `grep-regex`, which recognise a binary file rather than
//! printing its bytes at a reader. Hits leave here through a
//! **callback** and not a channel, [`analysis::open_files_streaming`]'s shape and for its
//! reason: whoever draws the result is who should decide what to do when they arrive faster
//! than they can be drawn. The callback answering [`ControlFlow::Break`] is how a search
//! nobody is waiting for stops where it stands.
//!
//! The pattern is [`Filter::expression`], the same expression the sidebar's filter bars
//! compile, so a toggle means one thing in both places.

use crate::chars;
use crate::filter::{Filter, Matcher};
use crate::grouped::{self, Grouped};
use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use std::{
    io,
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
};

/// The most hits a search reports. A pattern like `.` matches every line of every file, so
/// the walk stops here and the panel says that there are more.
pub const MAX_HITS: usize = 10_000;

/// The most characters of a matched line kept for the row that draws it. A generated file
/// can hold a line of megabytes, and a row can draw a sidebar's width of it.
const MAX_LINE: usize = 300;

/// What a search is asked for: where to look, and what to look for.
#[derive(Clone, PartialEq)]
pub struct SearchQuery {
    /// The project's directory, the whole of what is searched.
    pub root: PathBuf,
    /// The pattern and its three toggles, as the box spells them.
    pub filter: Filter,
}

impl SearchQuery {
    /// Whether this is a question at all: something typed, and a pattern that compiles.
    /// Nothing typed is not an empty search but no search, `Matcher::Everything`'s own
    /// rule, and an invalid pattern is said under the box rather than searched for.
    pub fn is_askable(&self) -> bool {
        matches!(self.filter.matcher(), Matcher::Pattern(_))
    }
}

/// One matched line. The file it is in is the group it is held under
/// ([`crate::grouped`]) and not a field of its own.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    /// Numbered from one, as an editor numbers them.
    pub line: u32,
    /// The line as the row draws it: leading whitespace gone and cut to [`MAX_LINE`]
    /// characters, since a row has one line's height and a sidebar's width.
    pub text: String,
    /// Where the matches are in `text`, as byte ranges into it. Empty when the cut left
    /// none of them in view.
    pub spans: Vec<Range<usize>>,
    /// The first match's place in the **file's own line**, in the UTF-16 units a pane
    /// counts columns in (`chars::columns_of`): what opening the hit picks out in the
    /// source. Kept apart from `spans`, which are offsets into the text a row draws and
    /// say nothing about the whitespace trimmed off the front of it.
    pub columns: Option<Range<usize>>,
}

/// What a running search says.
pub enum SearchEvent {
    /// One matched line, and the file it was found in.
    Hit(PathBuf, Hit),
    /// The walk is over, whether it ended, was capped, or found nothing.
    Finished,
}

/// Search `query`, handing each hit to `emit` as it is found and `Finished` when the walk
/// is over. `emit` answering [`ControlFlow::Break`] stops the walk where it stands, and
/// nothing is emitted after it.
///
/// `&mut dyn` rather than a generic, so that this is exactly the shape the UI's worker
/// takes and a test can put its own answer in its place.
pub fn search(query: &SearchQuery, emit: &mut dyn FnMut(SearchEvent) -> ControlFlow<()>) {
    let Some(matcher) = compile(&query.filter) else {
        let _ = emit(SearchEvent::Finished);
        return;
    };

    let mut searcher = SearcherBuilder::new()
        // Its default is to search a binary file like any other, which would put a row of
        // an object file's bytes in the list. `quit` abandons the file at the first NUL.
        .binary_detection(BinaryDetection::quit(0))
        .build();

    let mut progress = Progress {
        sent: 0,
        ended: None,
    };
    for entry in crate::walk::files(&query.root) {
        let mut sink = Hits {
            path: entry.path(),
            matcher: &matcher,
            emit,
            progress: &mut progress,
        };
        // A file that cannot be read is not an error the reader is asked about: it is one
        // of thousands being walked, and the panel is a list of what was found.
        let _ = searcher.search_path(&matcher, entry.path(), &mut sink);
        if progress.ended.is_some() {
            break;
        }
    }

    // A capped search is a search that ended, and the panel must stop saying that it is
    // running; a stopped one is a search nobody is listening to any more.
    if progress.ended != Some(Ended::Stopped) {
        let _ = emit(SearchEvent::Finished);
    }
}

/// The matcher a filter compiles to, or [`None`] where there is nothing to search for.
///
/// Nothing typed is no search and not an empty one, [`Matcher::Everything`]'s own rule:
/// an empty pattern handed to `grep-regex` matches every line of every file. A pattern the
/// builder refuses is no search either; what is wrong with it has already been said under
/// the box ([`SearchQuery::is_askable`]).
///
/// `word` and `fixed_strings` are deliberately left off the builder: the expression comes
/// from [`Filter::expression`] with the escaping and the `\b(?:…)\b` already in it, so that
/// the Word toggle means what it means in the sidebar. `grep-regex`'s own `word` is looser
/// than `\b` on purpose -- its docs have `-2` matching inside `foo -2 bar` -- and one
/// toggle must not mean two things in two boxes.
fn compile(filter: &Filter) -> Option<RegexMatcher> {
    if filter.pattern.is_empty() {
        return None;
    }
    RegexMatcherBuilder::new()
        .case_insensitive(!filter.case_sensitive)
        .build(&filter.expression())
        .ok()
}

/// What ended a search short of the end of the walk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ended {
    /// The callback said to stop. Nobody is listening, so nothing more is emitted --
    /// `Finished` included.
    Stopped,
    /// [`MAX_HITS`] was reached. A search that ended, so `Finished` is emitted and the
    /// panel stops saying that it is running.
    Capped,
}

/// How far the whole search has got. Owned by [`search`] and lent to the sink each file
/// is read into, so the two ways it can end are one value and cannot both hold.
struct Progress {
    /// How many hits have been emitted, against [`MAX_HITS`].
    sent: usize,
    /// What ended the walk, while it is still walking.
    ended: Option<Ended>,
}

/// One file's matches on their way out: the sink `grep-searcher` reports to.
struct Hits<'a> {
    path: &'a Path,
    matcher: &'a RegexMatcher,
    emit: &'a mut dyn FnMut(SearchEvent) -> ControlFlow<()>,
    progress: &'a mut Progress,
}

impl Sink for Hits<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, matched: &SinkMatch<'_>) -> io::Result<bool> {
        // One `SinkMatch` is one line while multi-line search is off, but the type does not
        // promise it, so the lines are walked and numbered rather than assumed to be one.
        let first = matched.line_number().unwrap_or(1);
        for (offset, line) in matched.lines().enumerate() {
            let number = first.saturating_add(offset as u64);
            let hit = hit_from(self.matcher, line, number);
            if (self.emit)(SearchEvent::Hit(self.path.to_path_buf(), hit)).is_break() {
                self.progress.ended = Some(Ended::Stopped);
                return Ok(false);
            }
            self.progress.sent += 1;
            if self.progress.sent >= MAX_HITS {
                self.progress.ended = Some(Ended::Capped);
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// One matched line as the row will draw it.
///
/// The order is the whole of it. The bytes are decoded **first**, so that the offsets that
/// come back are indices into the string the row holds -- a match found in the raw bytes
/// and applied to a lossy decode is off by two per replaced byte. The matches are then
/// found over the **whole** line, since a pattern's `^` and `\b` are answers about where in
/// the line they are asked, and only after that is the line trimmed and cut and the spans
/// moved with it.
fn hit_from(matcher: &RegexMatcher, line: &[u8], number: u64) -> Hit {
    let text = String::from_utf8_lossy(trim_terminator(line));

    let mut spans: Vec<Range<usize>> = Vec::new();
    // A zero-width match -- `\b`, `x*` -- marks nothing, and a row that drew it would show
    // a bold nothing. The walk over the line is the matcher's, so a failure to run it is a
    // line with no marks and not a line that did not match.
    let _ = matcher.find_iter(text.as_bytes(), |found| {
        if found.start() < found.end() {
            spans.push(found.start()..found.end());
        }
        true
    });

    // The first match is the one a press on the row goes to, and it is wanted whole and
    // over the file's line, where the spans below are cut down to what the row draws.
    let first = spans.first().cloned();

    let columns = first.map(|found| chars::columns_of(&text, found));
    let (text, spans) = drawn(&text, spans);

    Hit {
        line: u32::try_from(number).unwrap_or(u32::MAX),
        text,
        spans,
        columns,
    }
}

/// A line as a list row draws it: its leading whitespace gone and cut to [`MAX_LINE`]
/// characters, since a row has one line's height and a sidebar's width; and `spans` --
/// byte ranges into the whole line -- moved to what is left of it.
///
/// A span is clamped to what is drawn rather than dropped for reaching past it: a pattern
/// that takes in the indentation -- `^\s*needle` -- matches from before the text the row
/// shows, and the part of it in view is still what was found. One left with nothing in
/// view goes.
///
/// Shared with the references list, whose rows are these rows (`src/references.rs`).
pub fn drawn(line: &str, spans: Vec<Range<usize>>) -> (String, Vec<Range<usize>>) {
    let start = line.len() - line.trim_start().len();
    let end = chars::byte_of_char(&line[start..], MAX_LINE) + start;
    let spans = spans
        .into_iter()
        .map(|span| span.start.max(start) - start..span.end.clamp(start, end) - start)
        .filter(|span| span.start < span.end)
        .collect();
    (line[start..end].to_owned(), spans)
}

/// A line without the newline the searcher hands back with it, `\r\n` included.
fn trim_terminator(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Every hit a search has found, under the file each is in.
///
/// Files are kept in the order they arrived, which is the order [`crate::walk`] walked
/// them in, so the list only ever grows at its end and nothing a reader is looking at
/// moves.
pub type SearchHits = Grouped<Hit>;

/// The rows the Search panel draws, in order.
pub type SearchRows = grouped::Rows<Hit>;

/// Whether the cap was reached, so the panel can say that there are more.
pub fn capped(hits: &SearchHits) -> bool {
    hits.count() >= MAX_HITS
}

#[cfg(test)]
mod tests;

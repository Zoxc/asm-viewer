//! Every reference to a name the language server answered with, under the file each is
//! in.
//!
//! The server answers a flat list of places in whatever order it found them, so unlike a
//! search -- which reports a file's hits together, as it walks -- the grouping is done
//! here, once, when the answer lands. Files by path and references by line: the whole
//! answer arrives at once, so there is no order of arrival to keep, and the reader needs
//! to be able to find a file in the list. The grouping itself is [`crate::grouped`],
//! which the Search panel's hits are held in too.
//!
//! A row draws its line's text, as a search hit's does and cut the same way
//! ([`search::drawn`]) -- a list of line numbers says where a name is used and not how.
//! The server says nothing about the text, so the lines are **read off the disk here**,
//! each file once; the read blocks, which is why it happens with the ask on the language
//! worker and never on the UI thread. A file that will not read leaves its references with
//! the line number they already have.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use crate::chars;
use crate::grouped::{self, Grouped};
use crate::lsp;
use crate::search;

/// One reference: the line it is on, 1-based as every line in the app is, the columns of
/// the name on it, and that line as a row draws it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reference {
    pub line: u32,
    /// Where the name is in the **file's own line**, in the UTF-16 units a pane counts
    /// columns in: what opening the reference selects there. The server's own columns are
    /// bytes (`lsp::Place`), and this is the one place they are converted, the line
    /// having been read here anyway. Kept apart from `spans`, which are offsets into the
    /// text a row draws and say nothing about the whitespace trimmed off the front of it
    /// (`search::Hit`'s rule, for its reason).
    pub columns: Range<usize>,
    /// The line as the row draws it, and empty where the file would not read.
    pub text: String,
    /// Where the name is in `text`, as byte ranges into it. Empty where the cut left none
    /// of it in view, and where there is no text.
    pub spans: Vec<Range<usize>>,
}

/// Every reference, under the file it is in, and which files are folded away.
pub type References = Grouped<Reference>;

/// The rows the panel draws, in order.
pub type ReferenceRows = grouped::Rows<Reference>;

/// The places the server named, grouped, each with the text of the line it is on.
/// Two references on one line are two rows: a name used twice there is used twice, and
/// each selects its own.
///
/// `read` answers a file's whole text, and is asked **once per file** however many
/// references are in it. It is an argument so that the read is the caller's -- the
/// worker passes [`crate::source::read_text`], the app's one rule for reading a source
/// file, and a test passes what it wrote -- and so that nothing here blocks unless the
/// caller's read does.
pub fn of(places: &[lsp::Place], read: impl Fn(&Path) -> Option<String>) -> References {
    let mut by_file: BTreeMap<&Path, Vec<&lsp::Place>> = BTreeMap::new();
    for place in places {
        by_file.entry(&place.file).or_default().push(place);
    }
    Grouped::from_files(by_file.into_iter().map(|(path, places)| {
        let text = read(path);
        let source: Vec<&str> = text.iter().flat_map(|text| text.lines()).collect();
        let mut lines: Vec<Reference> = places
            .into_iter()
            .map(|place| {
                // Checked, not `line - 1`: a line is 1-based by the server's answer and
                // nothing here can hold that constructor to it, so a 0 is a line the
                // file does not have and not a panic.
                let at = (place.line as usize)
                    .checked_sub(1)
                    .and_then(|at| source.get(at))
                    .copied();
                reference(place, at)
            })
            .collect();
        lines.sort_by(|one, other| {
            (one.line, one.columns.start).cmp(&(other.line, other.columns.start))
        });
        (path, lines)
    }))
}

/// One place as a row of it: its line, and that line's text where the file gave one, cut
/// as a search hit's is with the name's own columns turned into spans over what is left.
///
/// A line the file does not have is a line the file has changed since the server read it;
/// the row is then the number alone, which is what it would be for a file that would not
/// read at all.
fn reference(place: &lsp::Place, line: Option<&str>) -> Reference {
    let bytes = place.columns.start as usize..place.columns.end as usize;
    let (columns, text, spans) = match line {
        Some(line) => {
            let name = span_of(line, bytes.clone());
            let (text, spans) = search::drawn(line, name.into_iter().collect());
            (chars::columns_of(line, bytes), text, spans)
        }
        // No line to count in, so the bytes stand: the right answer for a line of ASCII
        // and the nearest one for the rest.
        None => (bytes, String::new(), Vec::new()),
    };
    Reference {
        line: place.line,
        columns,
        text,
        spans,
    }
}

/// `bytes` as a range of `line` really has, and `None` where it names nothing of it: an
/// empty run, one the line is too short for -- which is a line that has changed under the
/// answer -- or one whose ends are not character boundaries.
///
/// Checked rather than trusted: what is cut out of the line here is cut by these numbers,
/// a slice taken off a character boundary panics, and a server's answer is input
/// (`AGENTS.md`).
fn span_of(line: &str, bytes: Range<usize>) -> Option<Range<usize>> {
    let fits = bytes.start < bytes.end
        && bytes.end <= line.len()
        && line.is_char_boundary(bytes.start)
        && line.is_char_boundary(bytes.end);
    fits.then_some(bytes)
}

#[cfg(test)]
mod tests;

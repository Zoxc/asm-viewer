//! Every use of a name the language server answered with, under the file each is in.
//!
//! The server answers a flat list of places in whatever order it found them, so unlike a
//! search -- which reports a file's hits together, as it walks -- the grouping is done
//! here, once, when the answer lands. Files by path and uses by line: the whole answer
//! arrives at once, so there is no order of arrival to keep, and the reader needs to be
//! able to find a file in the list.
//!
//! The rows a `VirtualScrollView` asks for are flattened ([`UseRows`]), for the reason
//! `search.rs` and `files.rs` flatten theirs: the shape is in the data and never in the
//! elements.
//!
//! A row draws its line's text, as a search hit's does and cut the same way
//! ([`search::drawn`]) -- a list of line numbers says where a name is used and not how.
//! The server says nothing about the text, so the lines are **read off the disk here**,
//! each file once; the read blocks, which is why it happens with the ask on the language
//! worker and never on the UI thread. A file that will not read leaves its uses with the
//! line number they already have.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::filter::Matcher;
use crate::lsp;
use crate::search;

/// One use: the line it is on, 1-based as every line in the app is, the columns of the
/// name on it, and that line as a row draws it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Use {
    pub line: u32,
    /// Where the name is in the **file's own line**, in UTF-16 units, which is what a
    /// pane counts columns in: what opening the use selects there. Kept apart from
    /// `spans`, which are offsets into the text a row draws and say nothing about the
    /// whitespace trimmed off the front of it (`search::Hit`'s rule, for its reason).
    pub columns: Range<u32>,
    /// The line as the row draws it, and empty where the file would not read.
    pub text: String,
    /// Where the name is in `text`, as byte ranges into it. Empty where the cut left none
    /// of it in view, and where there is no text.
    pub spans: Vec<Range<usize>>,
}

/// Every use, by the file it is in, and which files are folded away.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Uses {
    files: Vec<InFile>,
    count: usize,
}

/// One file and what is used in it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct InFile {
    path: PathBuf,
    name: String,
    lines: Vec<Use>,
    folded: bool,
}

impl Uses {
    /// The places the server named, grouped, each with the text of the line it is on.
    /// Two uses on one line are two rows: a name used twice there is used twice, and each
    /// selects its own.
    ///
    /// `read` answers a file's whole text, and is asked **once per file** however many
    /// uses are in it. It is an argument so that the read is the caller's -- the worker
    /// passes the filesystem and a test passes what it wrote -- and so that nothing here
    /// blocks unless the caller's read does.
    pub fn of(places: &[lsp::Place], read: impl Fn(&Path) -> Option<String>) -> Uses {
        let mut by_file: BTreeMap<&Path, Vec<&lsp::Place>> = BTreeMap::new();
        for place in places {
            by_file.entry(&place.file).or_default().push(place);
        }
        let mut count = 0;
        let files = by_file
            .into_iter()
            .map(|(path, places)| {
                let text = read(path);
                let source: Vec<&str> = text.iter().flat_map(|text| text.lines()).collect();
                let mut lines: Vec<Use> = places
                    .into_iter()
                    .map(|place| used(place, source.get(place.line as usize - 1).copied()))
                    .collect();
                lines.sort_by(|one, other| {
                    (one.line, one.columns.start).cmp(&(other.line, other.columns.start))
                });
                count += lines.len();
                InFile {
                    path: path.to_path_buf(),
                    name: name_of(path),
                    lines,
                    folded: false,
                }
            })
            .collect();
        Uses { files, count }
    }

    /// Fold the file at `path`, or unfold it. Whether anything changed, so the caller
    /// writes only then.
    pub fn toggle(&mut self, path: &Path) -> bool {
        let Some(file) = self.files.iter_mut().find(|file| file.path == path) else {
            return false;
        };
        file.folded = !file.folded;
        true
    }

    /// How many uses there are, over every file.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Everything found whose file `matcher` keeps, flattened in the order it is drawn: a
    /// file and then its uses, unless it is folded.
    ///
    /// A file is what a filter here matches: a use is a line of one, and a line number is
    /// nothing to type at.
    pub fn rows_matching(&self, matcher: &Matcher) -> UseRows {
        let mut rows = Vec::new();
        for file in self
            .files
            .iter()
            .filter(|file| matcher.matches(&file.path.to_string_lossy()))
        {
            rows.push(UseRow::File {
                path: file.path.clone(),
                name: file.name.clone(),
                count: file.lines.len(),
                folded: file.folded,
            });
            if !file.folded {
                rows.extend(file.lines.iter().map(|used| UseRow::Use {
                    path: file.path.clone(),
                    used: used.clone(),
                }));
            }
        }
        UseRows(Arc::new(rows))
    }
}

/// One row of the flattened uses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UseRow {
    /// A file, and how many uses are under it.
    File {
        path: PathBuf,
        name: String,
        count: usize,
        folded: bool,
    },
    /// One use, with the file it is in: a row opens a place, and the place is both.
    Use { path: PathBuf, used: Use },
}

/// The rows the panel draws, in order. Built once per change and shared by an `Arc`,
/// compared by that pointer, so handing them to a scroll view is one comparison and not a
/// walk ([`crate::search::SearchRows`]'s rule).
#[derive(Clone, Default)]
pub struct UseRows(Arc<Vec<UseRow>>);

impl PartialEq for UseRows {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl UseRows {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn row(&self, index: usize) -> &UseRow {
        &self.0[index]
    }
}

/// One place as a row of it: its line, and that line's text where the file gave one, cut
/// as a search hit's is with the name's own columns turned into spans over what is left.
///
/// A line the file does not have is a line the file has changed since the server read it;
/// the row is then the number alone, which is what it would be for a file that would not
/// read at all.
fn used(place: &lsp::Place, line: Option<&str>) -> Use {
    let (text, spans) = match line {
        Some(line) => {
            let name = bytes_of(line, &place.columns);
            search::drawn(line, name.into_iter().collect())
        }
        None => (String::new(), Vec::new()),
    };
    Use {
        line: place.line,
        columns: place.columns.clone(),
        text,
        spans,
    }
}

/// Where `columns` -- UTF-16 units into `line` -- is in its bytes, and `None` where they
/// name nothing of it: an empty run, or one the line is too short for, which is a line
/// that has changed under the answer.
fn bytes_of(line: &str, columns: &Range<u32>) -> Option<Range<usize>> {
    if columns.start >= columns.end {
        return None;
    }
    let (mut from, mut to) = (None, None);
    let mut units = 0u32;
    for (at, character) in line.char_indices() {
        if units == columns.start {
            from = Some(at);
        }
        if units == columns.end {
            to = Some(at);
        }
        units += search::units(character.encode_utf8(&mut [0; 4]) as &str) as u32;
    }
    if units == columns.end {
        to = Some(line.len());
    }
    Some(from?..to?)
}

/// A file as the list names it.
fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;

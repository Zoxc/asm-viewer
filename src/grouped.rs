//! Items under the file each is in, with a fold per file. Framework-free.
//!
//! What the Search panel's hits and the Locations panel's references are both held in.
//! The rows a `VirtualScrollView` asks for are flattened here, for the reason `files.rs`
//! flattens its tree: the shape is in the data and never in the elements.
//!
//! What differs between the two is the caller's: how a list is built -- a search appends
//! as it walks, a server's answer is grouped whole when it lands -- and what an item is.
//!
//! **An item, and the path and name of the file it is under, are held under an `Arc` from
//! the moment they are pushed**, so building a row is pointer bumps and nothing else. The
//! rows are made again whole every time the list grows, and a search grows a batch at a
//! time up to [`crate::search::MAX_HITS`]: copying an item, or copying a path into every
//! row of every rebuild, would be work that squares over one search, on the UI thread, for
//! rows whose contents never change once pushed.
//!
//! The `Arc`s here are for the copying and never for identity: two rows are the same row
//! when they name the same file, whichever `Arc` each spells it with, so every comparison
//! in this module -- the derived ones included -- is of what a path says (`AGENTS.md`).
//! [`Grouped::push`] tries `Arc::ptr_eq` before the paths, but only as a shortcut past a
//! comparison whose answer is already known.
//!
//! The text a row draws is cut here too ([`drawn`]). Both panels draw a line of a file
//! with part of it marked, so how much of that line a row keeps is one rule for both.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use crate::chars;
use crate::filter::Matcher;
use crate::shared::Shared;

/// Items under the file each is in, and which files are folded away.
#[derive(PartialEq, Eq, Debug)]
pub struct Grouped<T> {
    files: Vec<InFile<T>>,
    count: usize,
}

impl<T: Clone> Clone for Grouped<T> {
    /// Hand-written only to count. The `Arc`s above are what make a copy of a whole
    /// answer unnecessary, and [`copies`] is what says a render made none.
    fn clone(&self) -> Grouped<T> {
        #[cfg(test)]
        COPIES.with(|copies| copies.set(copies.get() + 1));
        Grouped {
            files: self.files.clone(),
            count: self.count,
        }
    }
}

/// Test-only: how many whole answers this thread has copied.
///
/// [`crate::source::touches`]'s shape and its reason. A thread-local, because
/// `freya-testing` runs the whole app on the test's own thread, which makes this the one
/// thing that can settle that a render copied nothing. Nothing resets it -- a test takes
/// the count before and after what it is about.
#[cfg(test)]
pub fn copies() -> usize {
    COPIES.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    static COPIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One file and what was found in it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct InFile<T> {
    path: Arc<Path>,
    name: Arc<str>,
    items: Vec<Arc<T>>,
    folded: bool,
}

impl<T> InFile<T> {
    /// A file and the items found in it, unfolded. The one place a name is taken off a
    /// path, and the one place an item is put under its `Arc`.
    fn new(path: Arc<Path>, items: impl IntoIterator<Item = T>) -> InFile<T> {
        InFile {
            name: crate::source::name_of(&path).into(),
            path,
            items: items.into_iter().map(Arc::new).collect(),
            folded: false,
        }
    }
}

impl<T> Default for Grouped<T> {
    /// Hand-written: a derived one would ask `T` to be [`Default`] too, and neither a
    /// hit nor a reference has an empty value.
    fn default() -> Self {
        Grouped {
            files: Vec::new(),
            count: 0,
        }
    }
}

impl<T> Grouped<T> {
    /// The files in the order given, each with its items: what an answer that arrives at
    /// once is grouped from.
    ///
    /// Generic over the path so a caller hands over whichever it holds: a `PathBuf` it is
    /// done with, or a `&Path` it is only borrowing. Either becomes the one `Arc` every
    /// row of that file is built from.
    pub fn from_files<P: Into<Arc<Path>>>(
        files: impl IntoIterator<Item = (P, Vec<T>)>,
    ) -> Grouped<T> {
        let mut grouped = Grouped::default();
        for (path, items) in files {
            grouped.add_file(InFile::new(path.into(), items));
        }
        grouped
    }

    /// A new file at the end, with `count` kept in step: the one place a whole file is
    /// added, so nothing can add one and forget the count.
    fn add_file(&mut self, file: InFile<T>) {
        self.count += file.items.len();
        self.files.push(file);
    }

    /// Add an item under `path`: the last file when it is the same one, and a new one
    /// otherwise. A search reports a file's items together, so this is a comparison
    /// against the last and not a lookup, and the files stay in the order they arrived.
    ///
    /// The `Arc` is the caller's, made once per file, and it is what a new file's rows
    /// are built from: nothing here allocates a path. `Arc::ptr_eq` is tried first, so
    /// every item after a file's first skips comparing the paths. That is a shortcut and
    /// not the rule -- two `Arc`s spelling the same path are still the same file.
    pub fn push(&mut self, path: &Arc<Path>, item: T) {
        if let Some(last) = self.files.last_mut() {
            if Arc::ptr_eq(&last.path, path) || *last.path == **path {
                last.items.push(Arc::new(item));
                self.count += 1;
                return;
            }
        }
        self.add_file(InFile::new(path.clone(), [item]));
    }

    /// Fold the file at `path`, or unfold it. Whether anything changed, so the caller
    /// writes only then.
    pub fn toggle(&mut self, path: &Path) -> bool {
        let Some(file) = self.files.iter_mut().find(|file| *file.path == *path) else {
            return false;
        };
        file.folded = !file.folded;
        true
    }

    /// How many items there are, over every file.
    pub fn count(&self) -> usize {
        self.count
    }

    /// How many files they are in.
    pub fn files(&self) -> usize {
        self.files.len()
    }

    /// Everything whose file `keep` matches, flattened in the order it is drawn: a file
    /// and then its items, unless it is folded.
    ///
    /// A file is what a filter matches here: an item is a line of one, and a line number
    /// is nothing to type at.
    ///
    /// Called for the whole list every time it grows, so every row is `Arc` clones and
    /// never a copy of the item or of the path it is under.
    pub fn rows(&self, keep: &Matcher) -> Rows<T> {
        let mut rows = Vec::new();
        for file in self
            .files
            .iter()
            .filter(|file| keep.matches(&file.path.to_string_lossy()))
        {
            rows.push(Row::File {
                path: file.path.clone(),
                name: file.name.clone(),
                count: file.items.len(),
                folded: file.folded,
            });
            if !file.folded {
                rows.extend(file.items.iter().map(|item| Row::Item {
                    path: file.path.clone(),
                    item: item.clone(),
                }));
            }
        }
        rows.into()
    }
}

/// One row of a flattened list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Row<T> {
    /// A file, and how many items are under it.
    File {
        path: Arc<Path>,
        name: Arc<str>,
        count: usize,
        folded: bool,
    },
    /// One item, with the file it is in: a row opens a place, and the place is both.
    Item { path: Arc<Path>, item: Arc<T> },
}

/// The rows a panel draws, in order.
pub type Rows<T> = Shared<Row<T>>;

/// The most characters of a line kept for the row that draws it. A generated file can hold
/// a line of megabytes, and a row can draw a sidebar's width of it.
pub const MAX_LINE: usize = 300;

/// A line as a list row draws it: its leading whitespace gone and cut to [`MAX_LINE`]
/// characters, since a row has one line's height and a sidebar's width; and `spans` --
/// byte ranges into the whole line -- moved to what is left of it.
///
/// A span is clamped to what is drawn rather than dropped for reaching past it: a pattern
/// that takes in the indentation -- `^\s*needle` -- matches from before the text the row
/// shows, and the part of it in view is still what was found. One left with nothing in
/// view goes.
///
/// Here and not in either panel: a search's hits and a name's references are drawn by one
/// row, so where a line is cut for it is one rule.
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

#[cfg(test)]
mod tests;

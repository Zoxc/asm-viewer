//! The project's directory as a tree, read off disk one level at a time. Framework-free.
//!
//! A project directory is arbitrarily large, so nothing here walks it: [`FileTree::new`]
//! reads the root's own entries and [`FileTree::toggle`] reads one directory's when it is
//! unfolded, and forgets them when it is folded again — so a refold is a re-read, which is
//! the whole of how the tree is refreshed. The tree **is** the fold state: a directory is
//! unfolded exactly when its children have been read, and there is no second set to keep
//! in step with it. [`FileTree::rows`] flattens what has been read into [`FileRow`]s, the
//! shape a `VirtualScrollView` asks for, as `tree.rs` does for the Objects list.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::shared::Shared;
use crate::source;
use crate::walk;

/// What is known of a directory's contents.
#[derive(Clone, Debug)]
enum Children {
    /// Not read, or read and folded away again: the two are the same thing here.
    Unread,
    Read(Vec<Node>),
    /// Asked and refused, which is a row of its own: a directory the reader cannot list is
    /// still there, and is drawn dimmed rather than dropped.
    Failed,
}

impl Children {
    /// The same three states, with the children dropped: what a row is drawn from.
    fn fold(&self) -> Fold {
        match self {
            Children::Unread => Fold::Folded,
            Children::Read(_) => Fold::Unfolded,
            Children::Failed => Fold::Failed,
        }
    }
}

/// What an entry is. Children hang off the directory arm, so a file cannot carry any.
#[derive(Clone, Debug)]
enum Kind {
    File,
    Directory(Children),
}

impl Kind {
    /// Whether this is a directory, which rows are sorted by.
    fn is_directory(&self) -> bool {
        matches!(self, Kind::Directory(_))
    }
}

/// One entry, and for a directory whatever of its contents has been read.
#[derive(Clone, Debug)]
struct Node {
    name: String,
    path: PathBuf,
    kind: Kind,
}

/// Whether a directory row's contents are on screen. A file row has none of this.
///
/// [`Children`] with the children dropped, which is what it is for: a row is cloned and
/// compared once per render, so it says the state and carries none of the contents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fold {
    Folded,
    Unfolded,
    /// The read failed. Drawn as a directory still, since toggling it tries again.
    Failed,
}

/// One row of the flattened tree. Flattened because a `VirtualScrollView` is told a length
/// and asked for row *n*: the tree is a shape in the data, never in the element tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileRow {
    /// The entry's name, without its directory.
    pub name: String,
    /// The whole path, spelled as the root joined with each entry's own name and never
    /// canonicalised: a source file opened from here has to be the string the debug info
    /// spells, and that is `DW_AT_comp_dir` joined with the file's entry.
    pub path: PathBuf,
    /// How many directories deep under the root, the root itself being `0`.
    pub depth: usize,
    /// The fold of a directory row, or [`None`] for a file.
    pub fold: Option<Fold>,
}

/// The rows the Files view draws, in order.
pub type FileRows = Shared<FileRow>;

/// A project directory and whatever of it has been read.
#[derive(Clone, Debug)]
pub struct FileTree {
    root: Node,
}

impl FileTree {
    /// A tree over `root`, its own entries read and every directory under it folded. [`None`]
    /// when `root` is not a directory this process can list, which is a placeholder's job
    /// to say and not a row's.
    pub fn new(root: &Path) -> Option<FileTree> {
        let children = read_level(root).ok()?;
        Some(FileTree {
            root: Node {
                name: source::name_of(root),
                path: root.to_path_buf(),
                kind: Kind::Directory(Children::Read(children)),
            },
        })
    }

    /// Fold the directory at `path` if it is unfolded, and otherwise read it — again, if
    /// it was read and folded away, or if the last read failed. Whether anything changed:
    /// a file, or a path that is not in the tree, changes nothing.
    pub fn toggle(&mut self, path: &Path) -> bool {
        let Some(node) = self.root.find_mut(path) else {
            return false;
        };
        let Kind::Directory(children) = &node.kind else {
            return false;
        };
        let next = match children {
            Children::Read(_) => Children::Unread,
            Children::Unread | Children::Failed => match read_level(&node.path) {
                Ok(children) => Children::Read(children),
                Err(_) => Children::Failed,
            },
        };
        node.kind = Kind::Directory(next);
        true
    }

    /// Everything read so far, flattened depth-first in the order it is drawn.
    pub fn rows(&self) -> FileRows {
        let mut rows = Vec::new();
        // An explicit stack rather than recursion: how deep this goes is how deep the reader
        // unfolded, which is bounded, but the bound is theirs and not the file's.
        let mut stack = vec![(&self.root, 0)];
        while let Some((node, depth)) = stack.pop() {
            let fold = match &node.kind {
                Kind::File => None,
                Kind::Directory(children) => Some(children.fold()),
            };
            rows.push(FileRow {
                name: node.name.clone(),
                path: node.path.clone(),
                depth,
                fold,
            });
            if let Kind::Directory(Children::Read(children)) = &node.kind {
                stack.extend(children.iter().rev().map(|child| (child, depth + 1)));
            }
        }
        rows.into()
    }
}

impl Node {
    /// The node at `path`, among what has been read. Only a read directory's children are
    /// searched, since nothing else has a node.
    fn find_mut(&mut self, path: &Path) -> Option<&mut Node> {
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            if node.path == path {
                return Some(node);
            }
            if let Kind::Directory(Children::Read(children)) = &mut node.kind {
                stack.extend(children.iter_mut());
            }
        }
        None
    }
}

/// One directory's entries: directories first, then files, each by [`walk::by_name`] --
/// the walk's own ordering, under a different leading term.
///
/// **A symlink is not an entry**, whatever it points at: the kind is the one the read
/// hands back, and nothing here follows one. That is [`source::showable`]'s rule and the
/// walk's (`crate::walk`), so a row here is a row a press opens rather than one drawn
/// dead. An entry whose kind cannot be read is dropped too: nothing is known of what its
/// row would open.
fn read_level(directory: &Path) -> io::Result<Vec<Node>> {
    #[cfg(test)]
    READS.with(|reads| reads.set(reads.get() + 1));
    let mut nodes: Vec<Node> = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            if kind.is_symlink() {
                return None;
            }
            Some(Node {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                kind: if kind.is_dir() {
                    Kind::Directory(Children::Unread)
                } else {
                    Kind::File
                },
            })
        })
        .collect();
    nodes.sort_by(|a, b| {
        b.kind
            .is_directory()
            .cmp(&a.kind.is_directory())
            .then_with(|| walk::by_name(&a.name, &b.name))
    });
    Ok(nodes)
}

/// Test-only: how many directories this thread has read into a tree.
///
/// Every level a tree holds comes from [`read_level`], so counting there counts them all.
/// A thread-local because `freya-testing` runs the whole app on the test's own thread,
/// which is what lets a headless test settle how often the view read the root. Nothing
/// resets it -- a test takes the count before and after what it is about.
#[cfg(test)]
pub fn reads() -> usize {
    READS.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;

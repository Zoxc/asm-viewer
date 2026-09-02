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
    sync::Arc,
};

/// Whether a press on `path` opens it as a source file: a regular file within the
/// [`source::MAX_SIZE`](crate::source::MAX_SIZE) the source cache will read, asked of the
/// metadata and never of the bytes. What a file *is* is not judged here at all: a press
/// opens anything the pane could show, and whether it is an object is the parser's
/// question, asked when the reader chooses to open it as one.
pub fn shows_as_source(path: &Path) -> bool {
    shows_as_source_within(path, crate::source::MAX_SIZE)
}

/// [`shows_as_source`] with the bound as a parameter, so a test need not write 16 MiB.
fn shows_as_source_within(path: &Path, max_size: u64) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() <= max_size)
        .unwrap_or(false)
}

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

/// One entry, and for a directory whatever of its contents has been read.
#[derive(Clone, Debug)]
struct Node {
    name: String,
    path: PathBuf,
    directory: bool,
    children: Children,
}

/// Whether a directory row's contents are on screen. A file row has none of this.
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

/// The rows the Files view draws, in order. Built once per change and shared by an `Arc`,
/// compared by that pointer.
#[derive(Clone, Debug)]
pub struct FileRows(Arc<Vec<FileRow>>);

impl PartialEq for FileRows {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl FileRows {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn row(&self, index: usize) -> &FileRow {
        &self.0[index]
    }

    #[cfg(test)]
    fn rows(&self) -> &[FileRow] {
        &self.0
    }
}

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
                name: file_name(root),
                path: root.to_path_buf(),
                directory: true,
                children: Children::Read(children),
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
        if !node.directory {
            return false;
        }
        node.children = match node.children {
            Children::Read(_) => Children::Unread,
            Children::Unread | Children::Failed => match read_level(&node.path) {
                Ok(children) => Children::Read(children),
                Err(_) => Children::Failed,
            },
        };
        true
    }

    /// Everything read so far, flattened depth-first in the order it is drawn.
    pub fn rows(&self) -> FileRows {
        let mut rows = Vec::new();
        // An explicit stack rather than recursion: how deep this goes is how deep the reader
        // unfolded, which is bounded, but the bound is theirs and not the file's.
        let mut stack = vec![(&self.root, 0)];
        while let Some((node, depth)) = stack.pop() {
            let fold = match (node.directory, &node.children) {
                (false, _) => None,
                (true, Children::Unread) => Some(Fold::Folded),
                (true, Children::Read(_)) => Some(Fold::Unfolded),
                (true, Children::Failed) => Some(Fold::Failed),
            };
            rows.push(FileRow {
                name: node.name.clone(),
                path: node.path.clone(),
                depth,
                fold,
            });
            if let Children::Read(children) = &node.children {
                stack.extend(children.iter().rev().map(|child| (child, depth + 1)));
            }
        }
        FileRows(Arc::new(rows))
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
            if let Children::Read(children) = &mut node.children {
                stack.extend(children.iter_mut());
            }
        }
        None
    }
}

/// One directory's entries: directories first, then files, each by name without regard to
/// case and then with it, so `Makefile` and `main.rs` sit where a reader looks for them.
/// A symlink is whichever kind of thing it points at, and nothing is followed further than
/// that: a cycle costs one click per level.
fn read_level(directory: &Path) -> io::Result<Vec<Node>> {
    let mut nodes: Vec<Node> = fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            Node {
                name: entry.file_name().to_string_lossy().into_owned(),
                directory: path.is_dir(),
                path,
                children: Children::Unread,
            }
        })
        .collect();
    nodes.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(nodes)
}

/// What a path is called without its directory, falling back to the whole path when it has
/// no file name at all — a root like `/`, which must not come out empty.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests;

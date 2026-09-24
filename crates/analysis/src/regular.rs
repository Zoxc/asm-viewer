//! A file named by a path someone else chose, opened and read without trusting the path.
//!
//! Every path the app reads that some file named -- a binary a project file lists, a `.pdb` a
//! binary records, a source file its debug info names -- goes through here, so there is one
//! answer to what such a read may cost. A stat in front of the open is not that answer: what
//! the stat saw and what the open finds can differ, since the path may be swapped for a fifo
//! in between. So the open does not wait on a fifo, the handle it opened is what is asked,
//! and the read stops one byte past its bound.

use std::{
    fs::{File, OpenOptions},
    io::{self, ErrorKind, Read},
    path::Path,
};

/// Whether [`open_regular`] follows a symlink the path ends in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Links {
    /// Follow it: a binary the reader chose may well be a link.
    Follow,
    /// Refuse it, as the app refuses every symlink as source (`source::showable`).
    Refuse,
}

/// A regular file, open, and the length it stated when it was opened.
#[derive(Debug)]
pub struct Regular {
    pub file: File,
    pub len: u64,
}

/// `path` opened for reading, if it is a regular file, and otherwise why not: the open's own
/// error, or [`ErrorKind::InvalidInput`] for anything that opened and is not a regular file.
///
/// The open is non-blocking, so a fifo is opened at once rather than when some writer
/// appears, and is then refused, as a device like `/dev/zero` is: both are asked of the
/// handle and not of the path. `O_NONBLOCK` does not change how a regular file reads. Off
/// Unix the open is a plain one.
pub fn open_regular(path: &Path, links: Links) -> io::Result<Regular> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = match links {
            Links::Follow => 0,
            Links::Refuse => libc::O_NOFOLLOW,
        };
        options.custom_flags(libc::O_NONBLOCK | nofollow);
    }
    #[cfg(not(unix))]
    let _ = links;
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(Regular {
        file,
        len: metadata.len(),
    })
}

impl Regular {
    /// All of the file, or [`ErrorKind::FileTooLarge`] if it states or holds more than
    /// `limit` bytes, or the read's own error.
    ///
    /// A stated length is not what a read returns: a `/proc` file states 0 and
    /// `/proc/self/pagemap` then reads as hundreds of gigabytes. So the read stops one byte
    /// past `limit`, and that byte refuses the file.
    pub fn read(self, limit: u64) -> io::Result<Vec<u8>> {
        let too_large = || io::Error::new(ErrorKind::FileTooLarge, format!("over {limit} bytes"));
        if self.len > limit {
            return Err(too_large());
        }
        let mut bytes = Vec::with_capacity(usize::try_from(self.len).unwrap_or(0));
        self.file
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        match bytes.len() as u64 <= limit {
            true => Ok(bytes),
            false => Err(too_large()),
        }
    }

    /// All of the file, if it holds no more than it stated.
    pub fn read_stated(self) -> io::Result<Vec<u8>> {
        let len = self.len;
        self.read(len)
    }
}

/// [`open_regular`] and [`Regular::read`] in one: the bytes of `path` if it is a regular file
/// holding no more than `limit` of them.
pub fn read_regular(path: &Path, links: Links, limit: u64) -> io::Result<Vec<u8>> {
    open_regular(path, links)?.read(limit)
}

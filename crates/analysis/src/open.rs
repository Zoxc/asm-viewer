//! The crate's entry point: each file read, tried as an archive and as an object, and every
//! object in it handed over as it is parsed.

use crate::{open_regular, parse_object, Links, Object, ObjectData, Regular};
use object::read::archive::ArchiveFile;
use std::{
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

/// What [`open_files_streaming`] has to say as it goes. There is deliberately no *start*:
/// the caller supplied the paths and they are walked in order.
pub enum Progress {
    /// One object, parsed and ready to be read.
    Parsed(Arc<Object>),
    /// Nothing more will come out of this path. Emitted for **every** path the walk reaches,
    /// including one that could not be read and one that yielded nothing at all.
    Finished(PathBuf),
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a plain
/// object file, handing each object to `emit` **as it is parsed** rather than collecting
/// them. Anything that fails to read or parse is silently skipped.
///
/// A callback rather than a channel or an iterator: a channel would make this crate pick a
/// backpressure policy belonging to whoever draws the result, and an iterator would mean
/// self-borrowing the file's bytes across a yield.
///
/// **`emit` answers whether to go on**, which is how work nobody is waiting for stops. The
/// one thing a single answer cannot express is "skip the rest of *this* file but go on to the
/// next", so a multi-file request in which one file is closed goes on parsing that file's
/// remaining members and drops them at the caller.
pub fn open_files_streaming(
    paths: Vec<PathBuf>,
    mut emit: impl FnMut(Progress) -> ControlFlow<()>,
) {
    for path in paths {
        // The path may be one a project file lists, so it is read as any path a file chose
        // is: a fifo or a device is refused, and so is a file that reads more than it states.
        let Ok(bytes) = open_regular(&path, Links::Follow).and_then(Regular::read_stated) else {
            // Unreadable is still an end: whoever asked for this file is drawing it as
            // pending until told otherwise.
            if emit(Progress::Finished(path)).is_break() {
                return;
            }
            continue;
        };
        if open_one_file(&path, Arc::from(bytes), &mut emit).is_break() {
            return;
        }
    }
}

/// The same for files already in memory, each with the path it is to be called by:
/// everything [`open_files_streaming`] does except the reading.
///
/// Bytes in hand cannot fail to be read, so this is that walk without its unreadable-path
/// case. It is for a caller holding a file it has no reason to write out first — the
/// tests, which build their archives with the `object` writer and would otherwise need a
/// directory to put them in.
pub fn open_data_streaming(
    files: Vec<(PathBuf, Arc<[u8]>)>,
    mut emit: impl FnMut(Progress) -> ControlFlow<()>,
) {
    for (path, bytes) in files {
        if open_one_file(&path, bytes, &mut emit).is_break() {
            return;
        }
    }
}

/// One file's worth of either, split out so `?` on the caller's answer reads as what it
/// is: abandon this file.
fn open_one_file(
    path: &Path,
    bytes: Arc<[u8]>,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    // One allocation per file, shared by every object parsed out of it and held for as long
    // as they live. Built here rather than where it is used at the bottom, because it is what
    // the members are cut from: the hash it takes is then one pass over the file however many
    // objects come out of it.
    let file = ObjectData::whole_file(bytes);

    if let Ok(archive) = ArchiveFile::parse(file.bytes()) {
        for member in archive.members() {
            let Ok(member) = member else {
                continue;
            };
            let name = String::from_utf8_lossy(member.name()).into_owned();
            // The same bytes `member.data(..)` would return, addressed as a range into the
            // archive so the member stays reachable without re-scanning the archive.
            let (offset, size) = member.file_range();
            let Some(data) = ObjectData::member(&file, offset, size) else {
                continue;
            };
            if let Some(object) = parse_object(data, name, path.to_path_buf()) {
                emit(Progress::Parsed(object))?;
            }
        }
    }

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
        .into_owned();
    if let Some(object) = parse_object(file, name, path.to_path_buf()) {
        emit(Progress::Parsed(object))?;
    }

    emit(Progress::Finished(path.to_path_buf()))
}

/// [`open_files_streaming`] with the objects collected, for a caller with nowhere to put them
/// one at a time.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();
    open_files_streaming(paths, |progress| {
        if let Progress::Parsed(object) = progress {
            objects.push(object);
        }
        ControlFlow::Continue(())
    });
    objects
}

//! The crate's entry point: each file read, tried as an archive and as an object, and every
//! object in it handed over as it is parsed.

use crate::parse::parse_unshared;
use crate::{open_regular, Links, LoadMessage, Object, ObjectData, Regular};
use object::read::archive::ArchiveFile;
use object::FileKind;
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

/// Parse each path as an archive (contributing one [`Object`] per member) or, if it is not
/// one, as a plain object file, handing each object to `emit` **as it is parsed** rather than
/// collecting them.
///
/// A file that yields no object is handed over all the same, as an object with no format
/// saying why: it could not be read ([`LoadMessage::CouldNotRead`]), it is not an object
/// ([`LoadMessage::NotAnObject`]), or it would not parse ([`LoadMessage::Malformed`]). An
/// archive's members come one member late, so the last can say what was left out: members
/// that stopped early ([`LoadMessage::ArchiveCutShort`]), members that are not objects
/// ([`LoadMessage::UnreadableMembers`]), or a thin archive's, which are not read
/// ([`LoadMessage::ThinArchive`]). An archive with no member shown is handed over alone, with
/// no format, to say why, if only that it holds none ([`LoadMessage::EmptyArchive`]).
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
        let flow = match open_regular(&path, Links::Follow).and_then(Regular::read_stated) {
            Ok(bytes) => open_one_file(&path, Arc::from(bytes), &mut emit),
            Err(error) => unread_file(&path, &error, &mut emit),
        };
        if flow.is_break() {
            return;
        }
    }
}

/// A path that could not be read, handed over as an object saying why. There are no bytes,
/// so it holds none.
fn unread_file(
    path: &Path,
    error: &std::io::Error,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let message = LoadMessage::CouldNotRead {
        error: error.to_string(),
    };
    let data = ObjectData::from(&[][..]);
    let object = Object::unread(
        path.to_path_buf(),
        name_of(path),
        data,
        false,
        vec![message],
    );
    emit(Progress::Parsed(Arc::new(object)))?;
    emit(Progress::Finished(path.to_path_buf()))
}

/// The same for files already in memory, each with the path it is to be called by:
/// everything [`open_files_streaming`] does except the reading.
///
/// Bytes in hand cannot fail to be read, so this is that walk without its unreadable-path
/// case ([`LoadMessage::CouldNotRead`]). It is for a caller holding a file it has no reason
/// to write out first — the tests, which build their archives with the `object` writer and
/// would otherwise need a directory to put them in.
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
    let name = name_of(path);

    // An archive's magic is no object's, so a file that parses as one is not tried as both.
    let archive = match ArchiveFile::parse(file.bytes()) {
        Ok(archive) => archive,
        Err(error) => {
            // A file that is not an archive is an object file or nothing, and one that is
            // an archive whose tables would not read is only that.
            let kind = FileKind::parse(file.bytes());
            let archive = matches!(kind, Ok(FileKind::Archive));
            let object = match archive {
                true => Err(error),
                false => parse_unshared(file.clone(), name.clone(), path.to_path_buf()),
            };
            let object = object.unwrap_or_else(|error| {
                let message = match kind {
                    Ok(_) => LoadMessage::Malformed {
                        error: error.to_string(),
                    },
                    Err(_) => LoadMessage::NotAnObject,
                };
                Object::unread(path.to_path_buf(), name, file, archive, vec![message])
            });
            emit(Progress::Parsed(Arc::new(object)))?;
            return emit(Progress::Finished(path.to_path_buf()));
        }
    };

    // Each object is handed over one member late, so the last one can still be told what
    // went wrong with the members.
    let mut held: Option<Object> = None;
    // How many members a thin archive names, none of which is read here.
    let mut thin = 0usize;
    // How many members were not objects this reader can read.
    let mut unreadable = 0usize;
    let mut cut_short = None;
    for (at, member) in archive.members().enumerate() {
        let stopped = LoadMessage::ArchiveCutShort {
            member: at.saturating_add(1),
        };
        // `object` ends the walk at the first member it cannot read, though the size
        // in its header may be good and the members after it fine.
        let Ok(member) = member else {
            cut_short = Some(stopped);
            break;
        };
        // Its bytes are in a file of its own, and `file_range` says offset 0 and that
        // file's size: a range into this archive's own first bytes.
        if member.is_thin() {
            thin = thin.saturating_add(1);
            continue;
        }
        // The same bytes `member.data(..)` would return, addressed as a range into the
        // archive so the member stays reachable without re-scanning the archive. A range
        // past the end is a file cut short inside this member, which `object`'s walk ends
        // at as well.
        let (offset, size) = member.file_range();
        let Some(data) = ObjectData::member(&file, offset, size) else {
            cut_short = Some(stopped);
            break;
        };
        let expected = not_code(member.name(), data.bytes());
        let name = String::from_utf8_lossy(member.name()).into_owned();
        match parse_unshared(data, name, path.to_path_buf()) {
            Ok(object) => {
                if let Some(previous) = held.replace(object) {
                    emit(Progress::Parsed(Arc::new(previous)))?;
                }
            }
            Err(_) if expected => {}
            Err(_) => unreadable = unreadable.saturating_add(1),
        }
    }

    let mut said = Vec::new();
    if thin > 0 {
        said.push(LoadMessage::ThinArchive { members: thin });
    }
    if unreadable > 0 {
        said.push(LoadMessage::UnreadableMembers { count: unreadable });
    }
    said.extend(cut_short);
    match held {
        Some(mut last) => {
            last.messages.extend(said);
            emit(Progress::Parsed(Arc::new(last)))?;
        }
        // An archive does not parse as an object, so with no member shown there is no row
        // to say this on but one made for it.
        None => {
            if said.is_empty() {
                said.push(LoadMessage::EmptyArchive);
            }
            let object = Object::unread(path.to_path_buf(), name, file, true, said);
            emit(Progress::Parsed(Arc::new(object)))?;
        }
    }
    emit(Progress::Finished(path.to_path_buf()))
}

/// What an object out of `path` is called when it is the whole file: its file name, or the
/// whole path where it has none.
fn name_of(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// Whether an archive member that did not parse was never meant to hold code, so leaving it
/// out is nothing to report: rustc's metadata in an rlib, which is an object on the targets
/// rustc knows how to wrap it for and bare bytes elsewhere, and an import library's short
/// entries, each only a name the DLL exports. The archive's symbol and name tables are not
/// among the members `object` hands over at all.
fn not_code(name: &[u8], bytes: &[u8]) -> bool {
    name == b"lib.rmeta"
        || name == b"lib.rmeta-link"
        || matches!(FileKind::parse(bytes), Ok(FileKind::CoffImport))
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

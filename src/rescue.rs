//! A stored file that will not parse, moved aside before the app writes over it.
//!
//! Every one of these files is read back into a default when it will not parse, and the
//! next write puts a good file over it — so without this the reader loses whatever was in
//! it and is never told. That is the one place where "persisted formats need no backward
//! compatibility" costs something real, and this is the whole of the answer to it: the
//! file is copied under an `incompatible` directory that mirrors the path it came from,
//! the original is removed, and the destination is remembered for the UI to name.
//!
//! Framework-free, like the two modules it is called from. What it moves is
//! [`crate::settings`]'s file and [`crate::project`]'s three; a scratchpad's own files are
//! not covered, since a pad whose manifest will not parse never loads and the app writes
//! over no pad it has not loaded.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::de::DeserializeOwned;

/// Where a file that will not parse goes, under the directory everything is stored in.
pub(crate) const INCOMPATIBLE_DIR: &str = "incompatible";

/// How many copies of one name there can be before a rescue gives up. Bounded so that a
/// directory refusing every create for a reason other than collision cannot spin, which is
/// [`crate::project::ProjectId::anonymous`]'s bound and its reasoning.
const MAX_COPIES: u32 = 1000;

/// Where each file moved aside was put, until the UI asks. A `static` because what fills
/// it is `project.rs`'s loads and `settings.rs`'s, which know nothing about the component
/// tree — the same reason the save policy is one.
static MOVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Read `path` as TOML, moving it aside if it will not parse.
///
/// `base` is the directory everything is stored in, which is what the copy's path is
/// mirrored under; a `path` outside it is not this app's to move.
///
/// The bytes and not a string: a file that is not UTF-8 will not parse either, and is lost
/// in exactly the same way. A file the system will not hand over at all is left where it
/// is — nothing can be salvaged from it, and nothing is about to write over it either.
pub fn parse<T: DeserializeOwned>(base: &Path, path: &Path) -> Option<T> {
    let data = fs::read(path).ok()?;
    let parsed = std::str::from_utf8(&data)
        .ok()
        .and_then(|text| toml::from_str(text).ok());
    if parsed.is_some() {
        return parsed;
    }

    if let Some(moved) = move_aside(base, path, &data) {
        log::warn!(
            "{} will not parse; moved to {}",
            path.display(),
            moved.display()
        );
        list().push(moved);
    }
    None
}

/// The paths moved aside since this was last asked, handed over rather than copied: what
/// the reader has already been told about is not told again.
pub fn moved() -> Vec<PathBuf> {
    std::mem::take(&mut *list())
}

fn list() -> MutexGuard<'static, Vec<PathBuf>> {
    // Take the list back rather than propagate: a poisoned lock must not turn a rescue
    // into a crashed app.
    MOVED.lock().unwrap_or_else(|error| error.into_inner())
}

/// Put `data` under `incompatible/` at the path `path` had, and take `path` away. The
/// destination, or `None` if nothing was written.
fn move_aside(base: &Path, path: &Path, data: &[u8]) -> Option<PathBuf> {
    let relative = path.strip_prefix(base).ok()?;
    let name = relative.file_name()?;
    let directory = base.join(INCOMPATIBLE_DIR).join(relative.parent()?);
    if let Err(error) = fs::create_dir_all(&directory) {
        log::warn!("could not make {}: {error}", directory.display());
        return None;
    }

    let moved = claim(&directory, &name.to_string_lossy(), data)?;
    if let Err(error) = fs::remove_file(path) {
        // The copy is what matters, and it is already made. A file still here is one more
        // copy on the next run, which is the harmless half of this going wrong.
        log::warn!("could not remove {}: {error}", path.display());
    }
    Some(moved)
}

/// Write `data` under a name in `directory` that nothing has: `name`, then `2-name`,
/// `3-name` and up.
///
/// The `create_new` *is* the claim — one operation that fails with `AlreadyExists` rather
/// than opening what is there — so nothing here is ever overwritten, not by an earlier
/// rescue and not by a second copy of the app rescuing the same file at the same moment.
fn claim(directory: &Path, name: &str, data: &[u8]) -> Option<PathBuf> {
    for n in 1..=MAX_COPIES {
        let path = match n {
            1 => directory.join(name),
            n => directory.join(format!("{n}-{name}")),
        };
        match File::create_new(&path) {
            Ok(mut file) => {
                return match file.write_all(data) {
                    Ok(()) => Some(path),
                    Err(error) => {
                        log::warn!("could not write {}: {error}", path.display());
                        None
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                log::warn!("could not make {}: {error}", path.display());
                return None;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;

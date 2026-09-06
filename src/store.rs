//! Everything the app stores, in one place: the directory it all goes under, how a file
//! under it is written, how one is read back when it may be bad, and how a name under it
//! is claimed.
//!
//! [`Store`] is that directory. Every module that keeps a file — the projects and their
//! sessions, the recent order, the settings, the scratchpads, the panic records — is
//! handed one rather than looking the place up for itself, which is what keeps the
//! lookup, the variable that overrides it, and the four rules below in one module.
//!
//! Framework-free, like every module that calls it: no freya types appear here.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// The one directory everything this app stores lives under: the projects, the recent
/// list, the settings, the scratchpads and the panic records.
const APP_DIR: &str = "assembly-viewer";

/// The variable that says where all of that goes, in place of the desktop's own state
/// directory.
///
/// It is there because **more than one copy of this app otherwise shares one directory**:
/// two checkouts, or a build somebody is trying something in beside the window the reader
/// actually uses. They do not merely take turns -- one writing a file the other's build
/// cannot parse is one moving the reader's file aside as unreadable, since that is what
/// every load on the way to a write does ([`Store::read`]). Pointing a second copy
/// somewhere of its own is the whole of the answer, and it is a variable rather than a
/// flag because it has to reach every process the app starts.
pub const STATE_VARIABLE: &str = "ASSEMBLY_VIEWER_STATE";

pub(crate) const PROJECTS_DIR: &str = "projects";
const SCRATCHPADS_DIR: &str = "scratchpads";
const PANICS_DIR: &str = "panics";

/// Where a file that will not parse goes, under the directory everything is stored in.
pub(crate) const INCOMPATIBLE_DIR: &str = "incompatible";

/// What an [`Order`] is kept in, wherever one is kept: the projects' is at the top of the
/// store and the scratchpads' is beside the pads.
pub const RECENTS_FILE: &str = "recents.toml";

/// How many names a claim may try before giving up, so a directory refusing every create
/// for a reason other than collision cannot spin.
const MAX_CLAIMS: u32 = 1000;

/// How many ids an [`Order`] is written with. What is lost past this is an *order*, never
/// a project and never a pad: both listings put back what the file did not name.
pub const MAX_ORDER: usize = 50;

/// Where each file moved aside was put, until the UI asks. A `static` because what fills
/// it is a load and not a component — the same reason the save policy is one.
static MOVED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The directory everything the app stores goes in, and the rules every file under it
/// follows.
///
/// One is opened per run and handed down. A path given to any of these is taken as
/// **relative to the store**, and an absolute one passes through untouched — which is
/// [`Path::join`]'s own rule, and is what lets a project file the reader gave a place go
/// through the same writer as the app's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    base: PathBuf,
}

impl Store {
    /// The store this run keeps its files in, or `None` on a system with no state or
    /// local data directory to put one in.
    ///
    /// Read at the moment it is asked for and never cached, so nothing anywhere has to be
    /// sequenced against the first ask. It is an environment lookup, and the app opens
    /// one store per run.
    pub fn open() -> Option<Store> {
        let base = given_base(std::env::var_os(STATE_VARIABLE)).or_else(desktop_base)?;
        Some(Store { base })
    }

    /// A store at a given directory: what a test points at one of its own, in place of
    /// the twin every operation under here used to have for exactly that.
    #[cfg(test)]
    pub fn at(base: impl AsRef<Path>) -> Store {
        Store {
            base: base.as_ref().to_path_buf(),
        }
    }

    /// The directory itself, for the two questions that are about a path's shape rather
    /// than about a file.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Where `relative` is under this store. An absolute path is left alone.
    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.base.join(relative)
    }

    /// Where the projects the reader has not given a place are kept.
    pub fn projects(&self) -> PathBuf {
        self.path(PROJECTS_DIR)
    }

    /// Where the scratchpads are kept: one directory each, under one of their own.
    pub fn scratchpads(&self) -> PathBuf {
        self.path(SCRATCHPADS_DIR)
    }

    /// Where a run appends what it panicked with.
    pub fn panics(&self) -> PathBuf {
        self.path(PANICS_DIR)
    }

    /// Write `contents` at `path`, atomically. See [`write_atomically`].
    pub fn write(&self, path: impl AsRef<Path>, contents: &[u8]) -> std::io::Result<()> {
        write_atomically(&self.path(path), contents)
    }

    /// The same for a value written as TOML — which is every file the app owns.
    ///
    /// TOML cannot spell a path that is not UTF-8 and serde's `PathBuf` impl fails rather
    /// than mangling one, so such a project is simply not written: the error is logged and
    /// swallowed by the caller, leaving the previous good file in place. This is a
    /// *runtime* failure, not a compile-time one.
    pub fn write_toml(
        &self,
        path: impl AsRef<Path>,
        value: &impl Serialize,
    ) -> std::io::Result<()> {
        let data = toml::to_string_pretty(value)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        self.write(path, data.as_bytes())
    }

    /// Read `path` as TOML, **moving it aside if it will not parse**.
    ///
    /// The one durability rule the store has, and the reason it is a method rather than a
    /// plain `toml::from_str`: every one of these files is read back into a default when
    /// it will not parse, and the next write puts a good file over it — so without this
    /// the reader loses whatever was in it and is never told. That is the one place where
    /// "persisted formats need no backward compatibility" costs something real, and this
    /// is the whole of the answer to it. **Every load that is on the way to a write goes
    /// through here.**
    ///
    /// The bytes and not a string: a file that is not UTF-8 will not parse either, and is
    /// lost in exactly the same way. A file the system will not hand over at all is left
    /// where it is — nothing can be salvaged from it, and nothing is about to write over
    /// it either. A path outside the store is read and not moved: it is not this app's to
    /// take away.
    pub fn read<T: DeserializeOwned>(&self, path: impl AsRef<Path>) -> Option<T> {
        let path = self.path(path);
        let data = fs::read(&path).ok()?;
        let parsed = std::str::from_utf8(&data)
            .ok()
            .and_then(|text| toml::from_str(text).ok());
        if parsed.is_some() {
            return parsed;
        }

        if let Some(moved) = self.move_aside(&path, &data) {
            log::warn!(
                "{} will not parse; moved to {}",
                path.display(),
                moved.display()
            );
            list().push(moved);
        }
        None
    }

    /// Claim a name under `parent` that nothing has, and hand back the path it went to.
    ///
    /// `name` spells the *n*th candidate and `create` is the claim itself — one atomic
    /// operation that fails with `AlreadyExists` rather than opening what is there, so
    /// nothing here is ever taken from under a second copy of the app doing the same
    /// thing at the same moment. Bounded, so a directory refusing every create for a
    /// reason other than collision cannot spin. `parent` is made if it is not there.
    pub fn claim(
        &self,
        parent: impl AsRef<Path>,
        name: impl Fn(u32) -> String,
        create: impl Fn(&Path) -> std::io::Result<()>,
    ) -> Option<PathBuf> {
        let parent = self.path(parent);
        if let Err(error) = fs::create_dir_all(&parent) {
            log::warn!("could not make {}: {error}", parent.display());
            return None;
        }

        for n in 1..=MAX_CLAIMS {
            let path = parent.join(name(n));
            match create(&path) {
                Ok(()) => return Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    log::warn!("could not make {}: {error}", path.display());
                    return None;
                }
            }
        }

        log::warn!("no free name under {}", parent.display());
        None
    }

    /// Put `data` under `incompatible/` at the path `path` had, and take `path` away. The
    /// destination, or `None` if nothing was written.
    ///
    /// The mirror is so that a moved file keeps the shape of the path it had rather than
    /// being flattened into one heap of `session.toml`s. The original is **removed**
    /// rather than copied, since nothing writes over `settings.toml` until a setting
    /// changes and a file left in place would be rescued again on every launch.
    fn move_aside(&self, path: &Path, data: &[u8]) -> Option<PathBuf> {
        let relative = path.strip_prefix(&self.base).ok()?;
        let name = relative.file_name()?.to_string_lossy().into_owned();
        let directory = Path::new(INCOMPATIBLE_DIR).join(relative.parent()?);

        let moved = self.claim(
            directory,
            |n| match n {
                1 => name.clone(),
                n => format!("{n}-{name}"),
            },
            |path| File::create_new(path)?.write_all(data),
        )?;
        if let Err(error) = fs::remove_file(path) {
            // The copy is what matters, and it is already made. A file still here is one
            // more copy on the next run, which is the harmless half of this going wrong.
            log::warn!("could not remove {}: {error}", path.display());
        }
        Some(moved)
    }
}

/// The directory the variable names, or `None` where it names nothing.
///
/// **Unset and empty are one answer.** A variable set to nothing is what a script that
/// meant to set it and did not looks like, and taking that as a path would put the reader's
/// projects in whatever directory the app was started from.
fn given_base(given: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let given = given?;
    match given.is_empty() {
        true => None,
        false => Some(PathBuf::from(given)),
    }
}

/// Where the desktop says an application's state goes, which is where this app keeps it
/// when nobody has said otherwise.
fn desktop_base() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join(APP_DIR))
}

/// Write `contents` to `path` by writing `path.tmp` first and renaming it over the top,
/// so an interrupted write cannot leave a half-written file behind and a concurrent reader
/// sees either the old file or the new one, never a truncated one. The parent directory is
/// made if it is not there, which is what lets a project's first write create its
/// directory.
///
/// The temporary is **synced before the rename**. A rename is atomic against a crash of
/// the process, but not against a power loss: the directory entry can reach the disk
/// before the data does, and the file the next launch then reads is zero bytes or a
/// truncated tail -- which will not parse, so [`Store::read`] moves the reader's project
/// or session aside and answers a default. The cost is one fsync per save, at most one
/// every 30 s. The directory entry itself is left unsynced: losing the rename costs the
/// last save, where losing the data costs the file.
///
/// The one atomic writer, and free rather than a method because one file it writes is not
/// the app's at all: `cargo.rs` edits the manifest of the workspace being read, which is
/// nowhere near a [`Store`]. Everything the app owns goes through [`Store::write`].
pub fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let mut file = fs::File::create(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temporary, path)
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

/// A most-recent-first order of ids, capped where it is written: `recents.toml`, whichever
/// of the two it is.
///
/// Which one to reopen is the first entry and not a field of its own. This is an *order*
/// and not an index of what exists — the project files and the pad directories are that —
/// which is why nothing here prunes an id whose file has gone; each listing does that at
/// the point of use, where the repair is free.
///
/// The cap is the **file's** and not this list's ([`Order::capped`]), so a pad the panel
/// is holding is not dropped by someone else being shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order<Id> {
    /// `Vec::new` and not a plain `default`, which serde's derive would spell as a
    /// `Default` bound on `Id` — a file's default is the empty order whatever is in it.
    #[serde(default = "Vec::new")]
    order: Vec<Id>,
}

impl<Id> Default for Order<Id> {
    fn default() -> Order<Id> {
        Order { order: Vec::new() }
    }
}

impl<Id> FromIterator<Id> for Order<Id> {
    fn from_iter<I: IntoIterator<Item = Id>>(ids: I) -> Order<Id> {
        Order {
            order: ids.into_iter().collect(),
        }
    }
}

impl<Id: PartialEq> Order<Id> {
    pub fn ids(&self) -> &[Id] {
        &self.order
    }

    pub fn into_ids(self) -> Vec<Id> {
        self.order
    }

    pub fn first(&self) -> Option<&Id> {
        self.order.first()
    }

    /// Put `id` at the front, and say whether that changed anything — which is what keeps
    /// a startup that reopens what was already at the front from writing a file.
    pub fn touch(&mut self, id: impl Into<Id>) -> bool {
        let id = id.into();
        if self.first() == Some(&id) {
            return false;
        }
        self.order.retain(|other| *other != id);
        self.order.insert(0, id);
        true
    }

    /// Drop `id`, and say whether it was there. Nothing else prunes the file, so something
    /// that has gone for good is taken out here.
    pub fn forget(&mut self, id: &Id) -> bool {
        let before = self.order.len();
        self.order.retain(|other| other != id);
        self.order.len() != before
    }

    /// The front of the order, at most [`MAX_ORDER`] of it: what is written out.
    pub fn capped(mut self) -> Order<Id> {
        self.order.truncate(MAX_ORDER);
        self
    }
}

#[cfg(test)]
mod tests;

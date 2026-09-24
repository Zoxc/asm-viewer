//! The directories the reader has agreed to a language server being run over:
//! `agreed.toml`, in the store.
//!
//! **In the store and not beside the project.** Everything beside a project file can
//! arrive with it -- checked in, or in an archive -- and so can the id that ties a session
//! to it. An agreement kept there is one a stranger can write, and it would run the program
//! their project names over their tree without asking. Only this app writes the store.
//!
//! Keyed by the directory, since that is what the question names and what is agreed to.

use std::path::{Path, PathBuf};

use crate::cargo;
use crate::order::Order;
use crate::store::Store;

const AGREED_FILE: &str = "agreed.toml";

/// The directories agreed to, most recently first. An order file, so it keeps
/// [`crate::store::MAX_ORDER`] of them, and one that falls off is asked about again.
type Agreed = Order<PathBuf>;

/// `directory` as it is kept: absolute, with its `.` and `..` taken out, so `/src/app/.`
/// out of a project file and `/src/app` out of the Project view's box are one directory.
pub(super) fn kept(directory: &Path) -> PathBuf {
    cargo::lexical(&std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned()))
}

/// Whether the reader has agreed to `directory`.
pub(super) fn agreed(store: &Store, directory: &Path) -> bool {
    let agreed: Agreed = store.read(AGREED_FILE).unwrap_or_default();
    agreed.position(&kept(directory)).is_some()
}

/// Note that the reader agreed to `directory`, or took that back. A write that fails is
/// logged and swallowed: what it costs is being asked again.
pub(super) fn agree(store: &Store, directory: &Path, agree: bool) {
    let mut agreed: Agreed = store.read(AGREED_FILE).unwrap_or_default();
    let directory = kept(directory);
    let changed = match agree {
        true => agreed.touch(directory),
        false => agreed.forget(&directory),
    };
    if changed {
        store.save_order(AGREED_FILE, agreed);
    }
}

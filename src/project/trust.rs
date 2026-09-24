//! The language servers the reader has agreed to being run, each over a directory:
//! `agreed.toml`, in the store.
//!
//! **In the store and not beside the project.** Everything beside a project file can
//! arrive with it -- checked in, or in an archive -- and so can the id that ties a session
//! to it. An agreement kept there is one a stranger can write, and it would run the program
//! their project names over their tree without asking. Only this app writes the store.
//!
//! Keyed by the directory and the program together, since the question names both: an
//! agreement to one program over a directory is no agreement to another project over the
//! same directory running a program of its own.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cargo;
use crate::order::Order;
use crate::store::Store;

use super::files::Details;

const AGREED_FILE: &str = "agreed.toml";

/// One agreement: a program, over a directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Agreement {
    /// As [`kept`] spells it.
    directory: PathBuf,
    /// As the project file names it, trimmed; `None` for the usual one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    program: Option<String>,
}

impl Agreement {
    /// What agreeing to `details` would agree to, or `None` where it names no directory.
    pub(super) fn of(details: &Details) -> Option<Agreement> {
        Some(Agreement {
            directory: kept(details.directory.as_deref()?),
            program: (details.language_server.as_deref())
                .map(str::trim)
                .filter(|program| !program.is_empty())
                .map(str::to_owned),
        })
    }
}

/// The agreements, most recently first. An order file, so it keeps
/// [`crate::store::MAX_ORDER`] of them, and one that falls off is asked about again.
type Agreed = Order<Agreement>;

/// `directory` as it is kept: absolute, with its `.` and `..` taken out, so `/src/app/.`
/// out of a project file and `/src/app` out of the Project view's box are one directory.
fn kept(directory: &Path) -> PathBuf {
    cargo::lexical(&std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned()))
}

/// Whether the reader has agreed to the program `details` names over its directory.
pub(super) fn agreed(store: &Store, details: &Details) -> bool {
    let Some(agreement) = Agreement::of(details) else {
        return false;
    };
    let agreed: Agreed = store.read(AGREED_FILE).unwrap_or_default();
    agreed.position(&agreement).is_some()
}

/// Note that the reader agreed to `agreement`, or took that back. A write that fails is
/// logged and swallowed: what it costs is being asked again.
pub(super) fn agree(store: &Store, agreement: &Agreement, agree: bool) {
    let mut agreed: Agreed = store.read(AGREED_FILE).unwrap_or_default();
    let changed = match agree {
        true => agreed.touch(agreement.clone()),
        false => agreed.forget(agreement),
    };
    if changed {
        store.save_order(AGREED_FILE, agreed);
    }
}

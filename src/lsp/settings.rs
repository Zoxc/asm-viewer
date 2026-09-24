//! What a server is told about the project, which is the other half of the handshake.
//! [`wanted`] is what this app asks of every server; a project's own
//! `.vscode/settings.json` is read by [`settings_in`] and laid over it, since some trees --
//! `rust-lang/rust` is the one the notes use -- cannot be read by a server that was told
//! nothing.
//!
//! Nothing here speaks to a server: this is the file, and what it says put the way a
//! server takes it.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::Path;

use jsonc_parser::ParseOptions;
use serde_json::{json, Value};

/// What this app asks of a language server whatever the project: the options it sends at
/// every handshake, and what a project's own settings are laid over.
///
/// **One line, and it turns something off.** Nothing is turned on: what navigation needs is
/// what rust-analyzer already does -- build scripts run and proc macros expand unless a
/// client says otherwise, and a name inside a macro that was not expanded resolves to
/// nothing -- and saying so again would only be a chance to say it wrongly, which is the
/// rule the capabilities follow too.
///
/// The check is off because the server runs one **on loading the workspace**, and not only
/// when a document is saved: watched, it opens a `rust-analyzer/flycheck/0` progress token
/// over a client that has opened no document and saved nothing. This app runs cargo itself
/// from the Project view and shows what came of it, so leaving it alone is a second build
/// of the reader's project whose output goes nowhere.
///
/// **The second turns the server's own diagnostics off**, which it publishes for every
/// document a client opens -- and this one opens what the reader has in tabs
/// (`Talk::opened`). Measured: 41 notifications for 41 files, every one of them read and
/// thrown away, since nothing here draws a diagnostic a server found. It used to need no
/// turning off because the app opened nothing.
///
/// What a project needs beyond this -- which manifests are its workspaces, where a tree
/// keeps its own proc-macro server, sysroot sources or toolchain -- depends on the tree and
/// not on this app, and nothing here can guess it: that is what a project's own settings
/// file is read for.
pub fn wanted() -> Value {
    json!({ "checkOnSave": false, "diagnostics": { "enable": false } })
}

/// The file a project's own settings for the server are in, which is VS Code's:
/// `.vscode/settings.json` under the project's directory. Most projects have none, and
/// that is not a failure.
///
/// Spelled once: [`settings_in`] joins it onto the directory, and the Project view names
/// it to the reader.
pub const SETTINGS: &str = ".vscode/settings.json";

/// The prefix a key in that file carries when it is meant for the server. Everything else
/// there is the editor's (`git.*`, `files.associations`) and is passed over in silence.
const PREFIX: &str = "rust-analyzer.";

/// The one variable a value may be written with. `${workspaceFolder}` is what a tree uses
/// to point at its own proc-macro server and its own toolchain, and it is the only one
/// this app has an answer for.
const FOLDER: &str = "${workspaceFolder}";

/// The most parts a name may be spelled in. Nothing rust-analyzer takes is more than four
/// deep, and the tree the names build is walked by recursion: how deep that goes is not
/// for a file to say (`AGENTS.md`, never panic on file input).
const DEEPEST: usize = 16;

/// What a project's own settings file said, ready to be handed to a server.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// One per key taken from the file, in name order: the name with `rust-analyzer.` off
    /// it, and the value written back out as it will be sent. What the Project view lists.
    pub overrides: Vec<(String, String)>,
    /// The same, as a server takes it: names split on their dots into a tree, laid over
    /// [`wanted`].
    options: Value,
}

impl Settings {
    /// A project that said nothing, which is what one with no such file has.
    pub fn none() -> Settings {
        Settings {
            overrides: Vec::new(),
            options: wanted(),
        }
    }

    /// What to send as `initializationOptions`.
    pub fn options(&self) -> &Value {
        &self.options
    }
}

/// Why a settings file could not be used. Every one of these **stops a start**: what a
/// server would otherwise be given is a name it ignores or a path that silently does not
/// exist, and either is worse than saying so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// It could not be read at all. A file that is not there is not this: that is the
    /// ordinary case and answers with [`Settings::none`].
    Unread(String),
    /// Not the JSONC an editor would read ([`JSONC`]).
    NotJson(String),
    /// Read, but not an object. A file with nothing in it is this one.
    NotAnObject,
    /// A name given a value and made a table by a longer name: `cargo` beside
    /// `cargo.features`. Which was meant is not for this app to pick.
    Both(String),
    /// A `${...}` that is not `${workspaceFolder}`.
    Variable(String),
    /// A name spelled in more parts than [`DEEPEST`].
    Deep(String),
}

impl fmt::Display for Unreadable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{SETTINGS}: ")?;
        match self {
            Unreadable::Unread(error) => write!(formatter, "{error}"),
            Unreadable::NotJson(error) => write!(formatter, "not JSON ({error})"),
            Unreadable::NotAnObject => write!(formatter, "not an object"),
            Unreadable::Both(name) => {
                write!(formatter, "{PREFIX}{name} is given a value and a table")
            }
            Unreadable::Variable(name) => {
                write!(formatter, "${{{name}}} is not a variable this can resolve")
            }
            Unreadable::Deep(name) => write!(formatter, "{PREFIX}{name} has too many parts"),
        }
    }
}

/// Read the project's own settings out of `directory`.
///
/// The thin half: everything below this is a function of the file's text. **No file is no
/// overrides**, since most projects have none and a viewer that warned about it would be
/// warning about every project.
///
/// `${workspaceFolder}` stands for the directory made absolute, as the server's root is
/// ([`super::rooted`]). The box takes any spelling, and a server resolves a relative path
/// in its settings against that root, so `dev/viewer` would name a file under
/// `dev/viewer/dev/viewer` that is not there.
pub fn settings_in(directory: &Path) -> Result<Settings, Unreadable> {
    let file = directory.join(SETTINGS);
    match crate::source::read_text_in(&file) {
        Ok(text) => settings_from(&text, &super::rooted(directory)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::none()),
        Err(error) => Err(Unreadable::Unread(error.to_string())),
    }
}

/// What that file says, as a server would be told it.
///
/// The two halves that matter are both silent when they are wrong, which is why they are
/// done here and tested rather than trusted: a server ignores a key that kept its
/// `rust-analyzer.` prefix, and ignores one whose dots were not split into a tree. Both
/// were watched happening against a real server. The rest of the file is the editor's own
/// keys, and they are skipped without a word.
pub fn settings_from(text: &str, directory: &Path) -> Result<Settings, Unreadable> {
    let read: Value = jsonc_parser::parse_to_serde_value(text, &JSONC)
        .map_err(|error| Unreadable::NotJson(error.to_string()))?;
    let Value::Object(read) = read else {
        return Err(Unreadable::NotAnObject);
    };

    let mut overrides = Vec::new();
    let mut root = BTreeMap::new();
    for (key, value) in &read {
        let Some(name) = key.strip_prefix(PREFIX) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if name.split('.').count() > DEEPEST {
            return Err(Unreadable::Deep(name.to_owned()));
        }
        let value = substituted(value, directory)?;
        overrides.push((name.to_owned(), value.to_string()));
        put(&mut root, name, value)?;
    }

    Ok(Settings {
        overrides,
        options: merged(wanted(), object_of(root)),
    })
}

/// How the file's text is read: **JSONC**, which is what VS Code reads it as and what the
/// files in the wild are written in -- the tree this whole thing is for opens with nine
/// lines of `//`.
///
/// Comments and a trailing comma, and **nothing else**. `jsonc-parser`'s own defaults go
/// on to take what JSON5 takes -- a single-quoted string, a name without quotes, a hex
/// number, a comma left out -- and no editor reading this file takes any of it, so each is
/// turned off here. A file this app read and the reader's editor would not is the two
/// disagreeing in silence about what a server was told.
///
/// Nothing is stripped before the parse. A `//` is half of every URL and a string can end
/// in an escaped quote, so a pass that blanks comments has to track every string in the
/// file or it cuts a path short without a word; a parser that reads JSONC has done that
/// already.
const JSONC: ParseOptions = ParseOptions {
    allow_comments: true,
    allow_trailing_commas: true,
    allow_loose_object_property_names: false,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
};

/// A name being built out of the file's dotted keys: the value the file gave under exactly
/// this name, or the table a longer name made of it. The two are what tells a clash from a
/// merge -- `cargo.features` and `cargo.noDeps` make one table between them, and `cargo`
/// with a value of its own beside either of them is a file saying two things.
enum Node {
    Value(Value),
    Table(BTreeMap<String, Node>),
}

/// Put one of the file's keys in the tree, under the name split on its dots.
///
/// Iterative, and not for elegance: the name comes from a file, and a recursion whose depth
/// it decided is a stack overflow, which cannot be caught.
fn put(root: &mut BTreeMap<String, Node>, name: &str, value: Value) -> Result<(), Unreadable> {
    let mut parts = name.split('.').peekable();
    let mut table = root;
    // Where the name being put reaches to, which is the name a clash is about however
    // the two keys were written and in whichever order the file wrote them.
    let mut at = 0;
    while let Some(part) = parts.next() {
        let clash = || Unreadable::Both(name[..at + part.len()].to_owned());
        if parts.peek().is_none() {
            // A name the file gave twice cannot reach here -- JSON keeps one of them --
            // so anything already under this name is the table a longer name made.
            if table.contains_key(part) {
                return Err(clash());
            }
            table.insert(part.to_owned(), Node::Value(value));
            return Ok(());
        }
        let node = table
            .entry(part.to_owned())
            .or_insert_with(|| Node::Table(BTreeMap::new()));
        let Node::Table(under) = node else {
            return Err(clash());
        };
        table = under;
        at += part.len() + 1;
    }
    Ok(())
}

/// The tree as JSON. Recursion bounded by [`DEEPEST`], which is what `put` refused a
/// deeper name for.
fn object_of(table: BTreeMap<String, Node>) -> Value {
    Value::Object(
        table
            .into_iter()
            .map(|(name, node)| {
                let value = match node {
                    Node::Value(value) => value,
                    Node::Table(under) => object_of(under),
                };
                (name, value)
            })
            .collect(),
    )
}

/// `over` laid on `base`, **leaf by leaf**: two objects are merged key by key and anything
/// else replaces what was under it.
///
/// Per leaf and not per name, so a project setting `cargo.features` keeps whatever else
/// this app sent under `cargo` rather than standing in for the whole of it. Recursion is
/// bounded by the two values' own depth, and a parsed one is bounded by the nesting the
/// parse allowed ([`JSONC`]).
fn merged(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Object(mut base), Value::Object(over)) => {
            for (name, value) in over {
                let under = base.remove(&name).unwrap_or(Value::Null);
                base.insert(name, merged(under, value));
            }
            Value::Object(base)
        }
        // Anything that is not two objects is a leaf, and the project's own stands.
        (_, over) => over,
    }
}

/// Every string in a value with its variables resolved, in place: this walks objects and
/// arrays and changes nothing but strings, which is what VS Code's own pass does.
fn substituted(value: &Value, directory: &Path) -> Result<Value, Unreadable> {
    Ok(match value {
        Value::String(text) => Value::String(resolved(text, directory)?),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| substituted(value, directory))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(name, value)| Ok((name.clone(), substituted(value, directory)?)))
                .collect::<Result<_, _>>()?,
        ),
        value => value.clone(),
    })
}

/// One string with its variables resolved.
///
/// `${workspaceFolder}` becomes the project's directory and **every other variable is a
/// failure**. VS Code leaves a name it does not know as it was written, which here would
/// be a path reaching the server that silently does not exist; saying so is the better of
/// the two. A `${` that is never closed is not a variable and is left alone.
fn resolved(text: &str, directory: &Path) -> Result<String, Unreadable> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        let Some(end) = rest[at..].find('}').map(|end| at + end) else {
            break;
        };
        let variable = &rest[at..=end];
        if variable != FOLDER {
            return Err(Unreadable::Variable(rest[at + 2..end].to_owned()));
        }
        out.push_str(&rest[..at]);
        out.push_str(&directory.to_string_lossy());
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests;

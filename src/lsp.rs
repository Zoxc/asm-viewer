//! The language server: rust-analyzer started over the project's directory and asked four
//! questions of one shape -- where the thing under a source position is defined, where it
//! is declared, what implements it, and where it is used -- and one of another: what every
//! name in a file **is**, which is what says which of them are links at all.
//!
//! Hand-rolled over `serde_json` rather than a protocol crate. What is spoken here is
//! eight messages wide -- the handshake's two, those five, and a reply to whatever the
//! server asks of us -- and a crate for it would bring a type for every request in the
//! specification and an async runtime's worth of machinery to drive them (`AGENTS.md`'s
//! pinning rules; `serde_json` is already in the tree and the manifest already blesses it
//! for a protocol rather than a file).
//!
//! **One request is in flight at a time**, so there is no table of outstanding ids: a
//! request writes its message and waits for the answer to that id. Two callers wanting
//! answers at once is what would end that, so which question an answer is to is the
//! caller's to keep (`src/ui/language.rs`).
//!
//! What the server says when nothing was asked is the other half. A reader thread owns the
//! server's output: an answer goes to whoever is waiting for it, a request is replied to,
//! and a notification is acted on -- which is how the app knows the server is busy reading
//! the project, since that arrives as `$/progress` and at no other time. Both threads
//! write to the server, so its input is behind a lock; the reader has to write because
//! declaring an interest in progress is what makes rust-analyzer ask this app to make a
//! progress token.
//!
//! [`Talk`] is generic over the two streams, so the conversation is tested against a fake
//! server over a pipe and the only part needing a real program is [`start_in`].
//!
//! What a server is told about the project is the other half of the handshake. [`wanted`]
//! is what this app asks of every server; a project's own `.vscode/settings.json` is read
//! by [`settings_in`] and laid over it, since some trees -- `rust-lang/rust` is the one
//! the notes use -- cannot be read by a server that was told nothing.
//!
//! The process is started and ended the way every program this app runs is
//! (`src/process.rs`): in a group of its own, and stopped by killing that group rather
//! than by asking the server to leave. A `shutdown` request is what the specification
//! offers, and a server that is indexing may take seconds to answer it; a stop must be
//! over when it returns, and rust-analyzer has nothing to lose by being killed.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::chars;
use crate::process::{self, Handle};

/// The largest message that will be read, so a server that says it is about to send a
/// gigabyte is a broken conversation and not an allocation.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

/// How many bytes of what the server writes to stderr are kept. The **first** of them,
/// since what is wanted is why a program that would not run said no.
const MAX_SAID: usize = 4096;

/// How long a handshake that failed waits for the program to be gone before deciding it is
/// still there. A pipe that has closed means it is on its way out, and this is only the
/// moment between that and the kernel agreeing.
const ENDING: Duration = Duration::from_millis(200);

/// Why there is no answer. All three are ordinary and none is a bug here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The program could not be started -- not installed, most often.
    NoServer(String),
    /// The conversation ended: the pipe closed, or what came back was not a message.
    Broken(String),
    /// The server answered, with an error: the code it gave and what it said.
    Refused { code: i64, said: String },
}

/// The notification a server sends about itself, which is rust-analyzer's own and is in
/// no specification: `quiescent` says it has finished reading the project. Asked for by
/// name in the handshake's `experimental` bag, and never sent by a server that has none.
const SETTLED: &str = "experimental/serverStatus";

/// The two error codes that mean "not now" rather than "no": ContentModified, which is
/// what a server still reading the project says, and RequestCancelled. Both are answers a
/// reader gets by asking again, and neither is worth reporting.
const NOT_NOW: [i64; 2] = [-32801, -32800];

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NoServer(error) => {
                write!(formatter, "could not start the language server: {error}")
            }
            Failure::Broken(error) => {
                write!(formatter, "the language server stopped answering: {error}")
            }
            Failure::Refused { said, .. } => {
                write!(formatter, "the language server refused: {said}")
            }
        }
    }
}

/// What the server said while nothing was being asked of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Whether it is working: reading the project, priming its caches, or anything else it
    /// reports progress on. An answer asked for while this is true can be empty because
    /// the server has not read the file yet.
    Busy(bool),
    /// Whether it has **settled**: everything it was going to read, read, and an answer
    /// now the answer it will keep giving.
    ///
    /// Progress cannot say this. A server opens and closes a token per piece of work --
    /// rust-analyzer runs eight of them in the first two seconds of a small crate -- so
    /// the gaps between them are not readiness, and a question asked in one comes back
    /// with fewer names than the same question a second later. Measured: 492 names in
    /// such a gap against 540 once settled, with `builtinType` and `parameter` among
    /// what the early answer had not worked out yet.
    ///
    /// The protocol has nothing for this at all: `initialized` is the whole of its
    /// lifecycle, and every large server has invented its own notification. This is
    /// rust-analyzer's, asked for in the handshake's `experimental` bag and simply never
    /// sent by a server that has no such thing -- which is why what is held of it is
    /// "what the server last said, if it has ever said anything".
    Settled(bool),
}

/// How the server counts a column, agreed on in the handshake ([`Talk::initialize`]).
///
/// The app counts a column in **bytes**, so a server that took `utf-8` leaves nothing to
/// convert. UTF-16 is the protocol's own default and what a server that says nothing has
/// kept -- `positionEncoding` arrived in 3.17 -- so it is what anything else is read as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    /// A column is a byte offset into the line, which is what the app counts in.
    Utf8,
    /// A column is a UTF-16 unit, which is what every column crossing this module is
    /// converted from and to.
    Utf16,
}

/// Where something is: a file, a **1-based** line in it, and the columns of the name on
/// that line.
///
/// The protocol counts lines from zero and this counts from one, the unit line information
/// is in everywhere else in the app (`Object::symbols_from_lines`), so the conversion
/// happens here and once. The columns are **byte offsets into that line**, whichever way
/// the server counted them ([`Encoding`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: PathBuf,
    pub line: u32,
    /// The columns of the name on `line`, in bytes. Empty where the answer's range spans
    /// lines: a name does not, and an empty run selects nothing.
    pub columns: Range<u32>,
}

/// Which of the four questions about a place is being asked.
///
/// One type from the link the reader presses to the method that goes out, so nothing is
/// mapped from one spelling of "which question" to another by hand (`src/links.rs`,
/// `src/ui/language.rs`). It splits by **what an answer is for**, which is what tells the
/// consumers apart: a followed question names one place to open, a listed one a list to
/// draw, and an answer splits the same way (`ui::language::Reply`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Question {
    /// One place to open, which `ui::follow` opens.
    Followed(Followed),
    /// A list to draw, which the Locations panel draws.
    Listed(Listed),
}

/// A question whose answer is a door. Only one of the two is ever asked of a name, and
/// both open the same one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Followed {
    /// Where the name is defined, which is nearly every link.
    Definition,
    /// Where it is **declared**: an item in a trait `impl`, whose definition is itself and
    /// whose declaration is the trait's (`src/links.rs`).
    Declaration,
}

/// A question whose answer is a list. The panel draws one of the two at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Listed {
    /// What implements the name.
    Implementations,
    /// Everywhere it is used.
    References,
}

impl Question {
    /// The method it goes out under.
    fn method(self) -> &'static str {
        match self {
            Question::Followed(Followed::Definition) => "textDocument/definition",
            Question::Followed(Followed::Declaration) => "textDocument/declaration",
            Question::Listed(Listed::Implementations) => "textDocument/implementation",
            Question::Listed(Listed::References) => "textDocument/references",
        }
    }

    /// What is sent with it: the place, and for references the one thing that is not
    /// asked for -- where the name is **defined**. A reader looking at the name has that
    /// under the pointer already, and following the link is the door to it.
    fn params(self, file: &Path, line: u32, column: u32) -> Value {
        let mut params = asked_at(file, line, column);
        if matches!(self, Question::Listed(Listed::References)) {
            params["context"] = json!({ "includeDeclaration": false });
        }
        params
    }
}

/// What a name is, in the server's own words: what it wrote about it, and the columns of
/// the name it answered about.
///
/// The text is **markdown**, which is what the handshake asks for and what rust-analyzer
/// sends when it is asked: a fenced block for the path and another for the signature, a
/// rule, and the doc comment under it. A server told nothing sends the same thing as plain
/// text with its structure flattened, which is why the handshake names the format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hovered {
    pub text: String,
    /// 1-based, as a [`Place`]'s line is and for its reason.
    pub line: u32,
    /// The columns of the name the answer is about, in bytes as a [`Place`]'s are. The
    /// server need not say, and where it does not these are the columns the question was
    /// asked at, empty.
    pub columns: Range<u32>,
}

/// The names a server gives the semantic token types and modifiers it will send, in the
/// order their indices count from.
///
/// **Read off the handshake and never assumed.** The order is the server's own -- for
/// rust-analyzer it is the order an enum happens to be written in -- so an index means
/// nothing without the list it was sent with, and a version that adds a type renumbers
/// everything after it. Asking by name is also what makes a server that sends fewer types
/// than another simply say less rather than say something wrong.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Legend {
    types: Vec<String>,
    modifiers: Vec<String>,
}

impl Legend {
    /// What the server calls `token`'s type, and `None` for an index it never declared.
    pub fn kind<'a>(&'a self, token: &Token) -> Option<&'a str> {
        self.types.get(token.kind as usize).map(String::as_str)
    }

    /// Whether the server said `modifier` of `token`. A modifier it never declared is one
    /// it cannot have said.
    pub fn says(&self, token: &Token, modifier: &str) -> bool {
        let Some(at) = self.modifiers.iter().position(|name| name == modifier) else {
            return false;
        };
        // The bitset is 32 bits wide on the wire, so a legend longer than that has
        // modifiers no answer can carry.
        u32::try_from(at).is_ok_and(|at| at < 32 && token.modifiers & (1 << at) != 0)
    }

    /// Whether the server declared any of this at all: an empty legend is a server that
    /// answers no semantic tokens, and every question about one is nothing found.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// A legend as a test spells one, the real ones coming off a handshake.
    #[cfg(test)]
    pub fn of(types: &[&str], modifiers: &[&str]) -> Legend {
        let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Legend {
            types: owned(types),
            modifiers: owned(modifiers),
        }
    }
}

/// One name the server classified, as [`Legend`] spells out: where it is, and the type
/// and modifier indices it was sent under.
///
/// The indices are kept rather than the names: a file is thousands of these, the names are
/// a few dozen, and what asks about one has the legend to hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// 1-based, as a [`Place`]'s line is and for its reason.
    pub line: u32,
    /// The columns of the name on `line`, in bytes as a [`Place`]'s are.
    pub columns: Range<u32>,
    /// Which of the legend's types, by index.
    pub kind: u32,
    /// Which of its modifiers, one bit each, by index.
    pub modifiers: u32,
}

/// A started server: the conversation, and the process it is with.
pub struct Server {
    talk: Talk<ChildStdin>,
    /// What ends it, which is also what says whether it has ended by itself.
    handle: Handle,
    /// What it wrote to stderr, which is where a program that will not run says why.
    /// Bytes, and decoded once when they are read: a `read` of a pipe returns whatever is
    /// there, so a character decoded chunk by chunk is two replacement characters
    /// wherever the writer's own buffering split it.
    said: Arc<Mutex<Vec<u8>>>,
    /// The thread filling `said`. Kept so a handshake that failed can wait for it to
    /// reach EOF before reading what it collected.
    stderr: Option<std::thread::JoinHandle<()>>,
}

/// Start rust-analyzer over `directory` and hand back the conversation and a handle that
/// can end it.
///
/// The handle is registered by [`process::start`], so [`process::stop_all`] reaches a
/// server whose [`Server`] has been lost -- the window's close hook can read no UI state
/// and has only this.
pub fn start_in(
    program: &str,
    directory: &Path,
    told: impl FnMut(Note) + Send + 'static,
) -> Result<(Server, Handle), Failure> {
    start_program_in(program, directory, told)
}

/// [`start_in`] under the name the tests use, which is what lets the failing half of it be
/// tested without a language server anywhere on the machine.
fn start_program_in(
    program: &str,
    directory: &Path,
    told: impl FnMut(Note) + Send + 'static,
) -> Result<(Server, Handle), Failure> {
    let mut command = Command::new(program);
    command
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Read on a thread of its own and kept, up to a point: a pipe nobody reads fills
        // and then blocks the program in a write, and what a program that will not run
        // writes there is the only account of why. `rust-analyzer` is often a rustup
        // proxy, and a toolchain without the component is a line on stderr and an exit.
        .stderr(Stdio::piped());
    let (handle, pipes) =
        process::start(&mut command).map_err(|error| Failure::NoServer(error.to_string()))?;

    let said = Arc::new(Mutex::new(Vec::new()));
    let stderr = keep_stderr(pipes.stderr, &said);

    // `Stdio::piped()` was asked for above, so both are there; a server started without
    // them could not be talked to at all.
    let (Some(to), Some(from)) = (pipes.stdin, pipes.stdout) else {
        handle.stop();
        return Err(Failure::NoServer("it has no pipes".to_owned()));
    };

    let server = Server {
        talk: Talk::over(to, BufReader::new(from), told),
        handle: handle.clone(),
        said,
        stderr,
    };
    Ok((server, handle))
}

/// Read the program's stderr on a thread of its own, keeping the first [`MAX_SAID`] bytes
/// of it and dropping the rest on the floor.
///
/// The thread is handed back, since when it has reached EOF is what says the program's
/// last words are all in.
fn keep_stderr(
    pipe: Option<impl Read + Send + 'static>,
    said: &Arc<Mutex<Vec<u8>>>,
) -> Option<std::thread::JoinHandle<()>> {
    let pipe = pipe?;
    let said = said.clone();
    process::read_on_thread("the language server's stderr", pipe, move |mut pipe| {
        let mut buffer = [0; 1024];
        loop {
            let Ok(read) = pipe.read(&mut buffer) else {
                return;
            };
            if read == 0 {
                return;
            }
            let mut said = said.lock().unwrap_or_else(|held| held.into_inner());
            if said.len() < MAX_SAID {
                said.extend_from_slice(&buffer[..read]);
            }
        }
    })
    .map_err(|error| log::warn!("the language server's stderr could not be read: {error}"))
    .ok()
}

/// Wait for the stderr thread to reach EOF, up to [`ENDING`], and let it go either way.
///
/// Bounded and not a plain join: stderr is inherited, so a grandchild the program left
/// behind holds the pipe open after the program itself is gone, and this is the failure
/// path of a handshake rather than somewhere to wait for ever.
fn all_said(reader: std::thread::JoinHandle<()>) {
    let until = std::time::Instant::now() + ENDING;
    while !reader.is_finished() {
        if std::time::Instant::now() >= until {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = reader.join();
}

impl Server {
    /// The handshake. Until it returns, the server has been asked nothing else.
    ///
    /// A handshake against a program that has already ended is not a conversation that
    /// broke: it is a program that would not run, and saying so is the difference between
    /// "rust-analyzer stopped answering" and the line it wrote on its way out.
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        self.talk.initialize(directory, options).map_err(|failure| {
            // In this order. The program's last words reach `said` on a thread of its
            // own, and both its pipes close at the same instant, so reading `said` first
            // -- and holding its lock over the wait -- is a race the stderr thread loses
            // about half the time. What it costs is the one line saying why the program
            // would not run, which is what this path is here to carry.
            let ended = self.handle.ending(ENDING);
            if ended.is_some() {
                if let Some(reader) = self.stderr.take() {
                    all_said(reader);
                }
            }
            let said = self.said.lock().unwrap_or_else(|held| held.into_inner());
            gone_instead(failure, ended, &String::from_utf8_lossy(&said))
        })
    }

    /// Put `question` about what is at `line` and `column` of `file`.
    ///
    /// `line` counts from zero, as the protocol does, and `column` is a byte offset into
    /// that line, as every column in the app is ([`Place`]).
    pub fn places(
        &mut self,
        question: Question,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        self.talk.places(question, file, line, column)
    }

    /// Every name in `file`, as the server classifies them.
    pub fn semantic_tokens(&mut self, file: &Path) -> Result<Vec<Token>, Failure> {
        self.talk.semantic_tokens(file)
    }

    /// What it said it would spell those with.
    pub fn legend(&self) -> &Legend {
        self.talk.legend()
    }

    /// What the name at `line` and `column` of `file` is, in the same units.
    pub fn hover(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<Hovered>, Failure> {
        self.talk.hover(file, line, column)
    }

    /// Tell it the app is showing `file`, and that it is not any more.
    pub fn opened(&mut self, file: &Path, language: &str, text: &str) -> Result<(), Failure> {
        self.talk.opened(file, language, text)
    }

    pub fn closed(&mut self, file: &Path) -> Result<(), Failure> {
        self.talk.closed(file)
    }

    /// Whether it takes documents at all, so a file it would never hear about is not read
    /// off the disk for nothing.
    pub fn opens(&self) -> bool {
        self.talk.opens()
    }
}

/// The failure a handshake really was, given what became of the program and what it said.
fn gone_instead(failure: Failure, ended: Option<String>, said: &str) -> Failure {
    let Some(status) = ended else {
        return failure;
    };
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    Failure::NoServer(match said.is_empty() {
        true => format!("it ended at once ({status})"),
        false => elided(&said),
    })
}

/// What a program said, cut to a length a line of the interface can hold.
fn elided(said: &str) -> String {
    const MOST: usize = 200;
    match said.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}...", &said[..at]),
        None => said.to_owned(),
    }
}

impl Drop for Server {
    /// Kill it and reap it. `Child`'s own `Drop` neither waits nor kills, so a server
    /// merely dropped would go on running with nothing left that could find it.
    fn drop(&mut self) {
        self.handle.stop();
    }
}

/// The conversation itself: what is written to the server, what is read back, and the
/// messages this app knows how to say.
///
/// Generic over the two streams so it can be held against a fake server over a pipe, which
/// is what the tests do; the real one is over the process's own two.
pub struct Talk<W> {
    /// Behind a lock because the reader writes too: a request from the server is answered
    /// on its thread, whether or not this one is in the middle of asking something. An
    /// `Option` because the reader holds it as well, so this is what closes the server's
    /// input when the conversation is dropped.
    to: Arc<Mutex<Option<W>>>,
    /// The answers the reader has picked out of what the server said. A closed channel is
    /// a reader that has stopped, which is a conversation that is over.
    answers: std::sync::mpsc::Receiver<Result<Value, Failure>>,
    /// The id of the last request. Ids are this side's alone and only have to be distinct.
    id: i64,
    /// What the server said it would spell its semantic tokens with, from the handshake.
    /// Empty until then, and empty for a server that answers none.
    legend: Legend,
    /// Whether it said it takes documents from the client, from the same reply. False
    /// until then, so nothing is sent to a server that has not been asked yet.
    opens: bool,
    /// Which way it counts a column, from the same reply. [`Encoding::Utf16`] until then,
    /// which is the protocol's default and the one that is converted.
    encoding: Encoding,
    /// How a file a conversion needs is read. `source::read_text` outside the tests,
    /// which hand over text rather than write a file for it. Never called at all where
    /// the server took `utf-8`.
    read: fn(&Path) -> Option<String>,
}

impl<W: Write + Send + 'static> Talk<W> {
    /// A conversation over two streams that are already connected to a server, with `told`
    /// called for whatever the server says that nobody asked for.
    pub fn over(
        to: W,
        from: impl BufRead + Send + 'static,
        told: impl FnMut(Note) + Send + 'static,
    ) -> Self {
        let to = Arc::new(Mutex::new(Some(to)));
        let (answered, answers) = std::sync::mpsc::channel();
        read_from(from, to.clone(), answered, told);
        Talk {
            to,
            answers,
            id: 0,
            legend: Legend::default(),
            opens: false,
            encoding: Encoding::Utf16,
            read: crate::source::read_text,
        }
    }

    /// Read the files a conversion needs with `read` rather than off the disk: what a
    /// test hands over instead of writing one.
    #[cfg(test)]
    pub fn reading(&mut self, read: fn(&Path) -> Option<String>) {
        self.read = read;
    }

    /// The handshake: `initialize`, then the `initialized` notification, which the
    /// server waits for and which nothing may come before.
    ///
    /// The capabilities are **four lines long**, and what is left out is the decision.
    /// Every request rust-analyzer would make of a client -- for configuration, to
    /// register a watcher -- is opt-in through a capability, so declaring none of those
    /// leaves a conversation this app only ever speaks first in. Nothing is said about
    /// definitions: plain locations are the default and are what is wanted, and naming
    /// that would only be a chance to name it wrongly.
    ///
    /// The first is progress, because it is the only way to know the server is still
    /// reading the project -- an answer before that is done is empty and says nothing
    /// about why. It costs the `window/workDoneProgress/create` requests the reader
    /// answers.
    ///
    /// The second is the format a hover is written in, which is the one default not worth
    /// taking. A client that names none is answered in plain text, with the fences gone
    /// and the doc comment's list run together into one word; one that names markdown is
    /// answered with the signature fenced and the comment as it was written. Measured
    /// against a real server both ways, over the same name.
    ///
    /// The third is how a column is counted, and it is the other default not worth
    /// taking. The app counts a column in bytes, the protocol's default is UTF-16, and a
    /// server offered both takes the first it knows: `utf-8` costs nothing on either side
    /// and leaves nothing here to convert. **The order is the preference**, and a server
    /// that says nothing back has kept UTF-16 -- `positionEncoding` arrived in 3.17 -- so
    /// anything but a plain `utf-8` is read as UTF-16 and converted ([`Encoding`]).
    ///
    /// Semantic tokens are **not** declared either, though they are asked for: rust-analyzer
    /// offers them and sends its whole legend to a client that says nothing, which was
    /// measured against a real one before this was written. What the reply says it will
    /// send is kept ([`legend_of`]), since the indices in an answer mean nothing without
    /// it.
    ///
    /// The options are the caller's: [`wanted`] with whatever the project's own
    /// `.vscode/settings.json` said laid over it ([`settings_from`]).
    ///
    /// The directory is made absolute first ([`rooted`]). The box it was typed into takes
    /// any spelling, and a `rootUri` built out of `dev/viewer` or `.` names a place that
    /// is not there -- which a server reports through `window/showMessage` and this
    /// client only logs, leaving a control that says it is running and every question
    /// answering nothing.
    pub fn initialize(&mut self, directory: &Path, options: &Value) -> Result<(), Failure> {
        let directory = rooted(directory);
        let root = uri_of(&directory);
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let said = self.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "clientInfo": { "name": "Assembly Viewer" },
                "rootUri": root,
                "workspaceFolders": [{ "uri": root, "name": name }],
                "capabilities": {
                    "window": { "workDoneProgress": true },
                    "textDocument": { "hover": { "contentFormat": ["markdown"] } },
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "experimental": { "serverStatusNotification": true },
                },
                "initializationOptions": options,
            }),
        )?;
        self.legend = legend_of(&said);
        self.opens = opens_documents(&said);
        self.encoding = encoding_of(&said);
        self.notify("initialized", json!({}))
    }

    /// Put `question` about what is at `line` and `column` of `file`: the places it is
    /// answered with, and an empty answer where the server said "not now".
    ///
    /// The four are one shape -- a place in, places out -- so they are one method, and
    /// [`Question`] says which. They are genuinely four: an item in a trait `impl` is
    /// defined where it is written and declared in the trait, and a call to a trait method
    /// is defined in the `impl` that runs and declared in the trait as well, so no two of
    /// them can stand in for each other.
    ///
    /// The file has been opened first ([`Talk::opened`]), which is what makes a server
    /// answer about it at all rather than when its own reading of the directory catches
    /// up. A file the app never opened -- one outside the project, or of a language this
    /// server is not for -- answers whatever the server can work out on its own, which
    /// is often nothing, and nothing is what a question with no answer gets anyway.
    pub fn places(
        &mut self,
        question: Question,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Vec<Place>, Failure> {
        let mut lines = self.lines();
        let params = question.params(file, line, lines.out(file, line, column));
        let mut found = self
            .asked(question.method(), params)
            .map(|value| places(&value))?;
        for place in &mut found {
            let at = place.line.saturating_sub(1);
            place.columns = lines.back(&place.file, at, place.columns.clone());
        }
        Ok(found)
    }

    /// What the name at `line` and `column` of `file` is, in the server's own words, and
    /// nothing where it has none to say. The units are [`Talk::places`]'s.
    ///
    /// The file has been opened first, for [`Talk::places`]'s reason.
    ///
    /// **A refusal is an empty answer**, as it is for a place and not as it is for the
    /// names in a file: the pointer resting on the name again is what asks anew, and it
    /// costs nothing to wait for that.
    pub fn hover(
        &mut self,
        file: &Path,
        line: u32,
        column: u32,
    ) -> Result<Option<Hovered>, Failure> {
        let mut lines = self.lines();
        let column = lines.out(file, line, column);
        let mut said = self
            .asked("textDocument/hover", asked_at(file, line, column))
            .map(|value| hovered(&value, line, column))?;
        if let Some(said) = said.as_mut() {
            let at = said.line.saturating_sub(1);
            said.columns = lines.back(file, at, said.columns.clone());
        }
        Ok(said)
    }

    /// Every name in `file`, as the server classifies them.
    ///
    /// One request for the whole file rather than one per name: it is the only way to be
    /// told what a name **is** without asking about each in turn, and asking about each
    /// would be a round trip per name down a conversation that holds one question at a
    /// time. The file has been opened first, for [`Talk::places`]'s reason.
    ///
    /// A server that declared no legend is one that answers none of this, and is not
    /// asked.
    ///
    /// **A refusal is passed on and not read as an empty answer.** The two "not now"
    /// codes and the one a server gives for a file it has not read yet all mean "ask
    /// again", which only the caller can do -- and does, once the server says it has read
    /// more of the project (`src/ui/linking.rs`).
    pub fn semantic_tokens(&mut self, file: &Path) -> Result<Vec<Token>, Failure> {
        if self.legend.is_empty() {
            return Ok(Vec::new());
        }
        let params = json!({ "textDocument": { "uri": uri_of(file) } });
        let mut found = self
            .request("textDocument/semanticTokens/full", params)
            .map(|value| tokens(&value))?;
        let mut lines = self.lines();
        for token in &mut found {
            let at = token.line.saturating_sub(1);
            token.columns = lines.back(file, at, token.columns.clone());
        }
        Ok(found)
    }

    /// Tell the server the app is showing `file`, whose text is `text` and whose language
    /// a server calls `language` (`source::Language::spoken`).
    ///
    /// **This is what makes an answer about the file arrive at all.** The protocol has the
    /// client own the documents it shows: after this the server answers about the text
    /// given here, until [`Talk::closed`]. A file it was never told about is one it can
    /// only answer for out of its own reading of the directory, which rust-analyzer does
    /// -- once its scan has caught up, four seconds into a two-file crate and longer for
    /// anything real. Measured over the same file: names at 0.0s opened, and at 4.3s not.
    ///
    /// Nothing is sent to a server that did not say it takes documents.
    pub fn opened(&mut self, file: &Path, language: &str, text: &str) -> Result<(), Failure> {
        if !self.opens {
            return Ok(());
        }
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri_of(file),
                    "languageId": language,
                    // The one document the app ever has of a file: it shows what is on
                    // disk and edits nothing, so a version that counted would only ever
                    // count re-reads.
                    "version": 1,
                    "text": text,
                },
            }),
        )
    }

    /// Tell it the app is no longer showing `file`, so what is on disk is the truth about
    /// it again.
    pub fn closed(&mut self, file: &Path) -> Result<(), Failure> {
        if !self.opens {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri_of(file) } }),
        )
    }

    /// Whether it said it takes documents at all.
    pub fn opens(&self) -> bool {
        self.opens
    }

    /// What the server said it would spell its semantic tokens with.
    pub fn legend(&self) -> &Legend {
        &self.legend
    }

    /// The lines one question's conversion may need. Empty, and never filled at all where
    /// the server took `utf-8`.
    fn lines(&self) -> Lines {
        Lines {
            encoding: self.encoding,
            read: self.read,
            files: BTreeMap::new(),
        }
    }

    /// One request whose refusal may be a "not now", which is the layer between
    /// [`Talk::request`] and every question a reader asks.
    ///
    /// A [`NOT_NOW`] code is answered with [`Value::Null`] rather than an error: the
    /// server is still reading the project, or what was asked about changed under the
    /// question. Not a failure to report and not an answer -- a click is a question, not a
    /// promise -- and every reader of an answer takes a value of the wrong shape as
    /// nothing found.
    ///
    /// Two things go straight to [`Talk::request`] instead. The handshake needs the
    /// refusal itself, and [`Talk::semantic_tokens`] passes one on so the caller can ask
    /// again.
    fn asked(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        match self.request(method, params) {
            Err(Failure::Refused { code, .. }) if NOT_NOW.contains(&code) => Ok(Value::Null),
            answer => answer,
        }
    }

    /// One request, and the answer to it.
    ///
    /// Everything else the server says is the reader's: this waits for an answer carrying
    /// the id it asked under, and a conversation that ended arrives here as the reader
    /// letting go of its end.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.id += 1;
        let id = self.id;
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        loop {
            let message = self
                .answers
                .recv()
                .map_err(|_| Failure::Broken("it closed the connection".to_owned()))??;
            // An older request's answer cannot happen with one request in flight, and is
            // dropped rather than mistaken for this one's.
            if message.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(Failure::Refused {
                    code: error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                    said: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("no reason given")
                        .to_owned(),
                });
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// One notification: no id, so no answer is coming and none is waited for.
    fn notify(&mut self, method: &str, params: Value) -> Result<(), Failure> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    fn write(&mut self, body: &Value) -> Result<(), Failure> {
        write_to(&self.to, body)
    }
}

/// The lines one question's columns are converted through, each file read once.
///
/// A column crossing this module is a byte offset into its line and a column on the wire
/// is whatever the handshake agreed on, so converting between them takes the line's text.
/// The question's own file is one the app has open; an answer can name any file at all --
/// a definition in another crate, a reference in a file no tab shows -- so the text is
/// read rather than remembered, through the app's one rule for reading a source file
/// (`source::read_text`). The read blocks, which is why every question here is a worker's
/// (`src/ui/language.rs`).
///
/// **Nothing is read where the server took `utf-8`**: the numbers are already the app's,
/// and nothing here has an answer to give.
struct Lines {
    encoding: Encoding,
    read: fn(&Path) -> Option<String>,
    /// What each file said, a miss included, so an answer naming one file twenty times
    /// reads it once.
    files: BTreeMap<PathBuf, Option<String>>,
}

impl Lines {
    /// The text of `line` -- counted from zero, as the protocol counts -- of `file`, and
    /// nothing where there is no converting to do, the file would not read, or it is too
    /// short for the line.
    fn at(&mut self, file: &Path, line: u32) -> Option<&str> {
        if self.encoding == Encoding::Utf8 {
            return None;
        }
        let read = self.read;
        let text = self
            .files
            .entry(file.to_path_buf())
            .or_insert_with(|| read(file));
        text.as_deref()?.lines().nth(line as usize)
    }

    /// A byte column as the server counts one: what goes out with a question.
    fn out(&mut self, file: &Path, line: u32, column: u32) -> u32 {
        let Some(text) = self.at(file, line) else {
            return column;
        };
        let at = column as usize;
        narrowed(chars::columns_of(text, at..at).start)
    }

    /// The server's columns as bytes: what comes back with an answer.
    fn back(&mut self, file: &Path, line: u32, columns: Range<u32>) -> Range<u32> {
        let Some(text) = self.at(file, line) else {
            return columns;
        };
        let bytes = chars::bytes_of(text, columns.start as usize..columns.end as usize);
        narrowed(bytes.start)..narrowed(bytes.end)
    }
}

/// A column as it is held here. A line of four billion bytes is not one this app draws,
/// so the count clamps rather than wraps.
fn narrowed(column: usize) -> u32 {
    u32::try_from(column).unwrap_or(u32::MAX)
}

/// Which way the server said it counts a column.
///
/// Anything but a plain `utf-8` is UTF-16: it is the protocol's default, it is what a
/// server older than 3.17 has kept, and it is the reading that converts rather than the
/// one that trusts a word this app did not offer.
fn encoding_of(said: &Value) -> Encoding {
    let chosen = said
        .get("capabilities")
        .and_then(|value| value.get("positionEncoding"))
        .and_then(Value::as_str);
    match chosen {
        Some("utf-8") => Encoding::Utf8,
        _ => Encoding::Utf16,
    }
}

/// Write one message to a server two threads talk to, and that one of them may already
/// have said goodbye to.
fn write_to(to: &Mutex<Option<impl Write>>, body: &Value) -> Result<(), Failure> {
    let mut to = to.lock().unwrap_or_else(|held| held.into_inner());
    let Some(to) = to.as_mut() else {
        return Err(Failure::Broken("the conversation is over".to_owned()));
    };
    write_message(to, body).map_err(|error| Failure::Broken(error.to_string()))
}

/// Read everything the server says, on a thread of its own, until it stops saying
/// anything.
///
/// An answer goes to whoever asked; a request is replied to here, since the server may ask
/// while nothing is being asked of it; and a notification is what `told` is for. The
/// thread ends when the server's output does, and the closed channel is what tells a
/// waiting request that the conversation is over.
fn read_from<W: Write + Send + 'static>(
    from: impl BufRead + Send + 'static,
    to: Arc<Mutex<Option<W>>>,
    answered: std::sync::mpsc::Sender<Result<Value, Failure>>,
    mut told: impl FnMut(Note) + Send + 'static,
) {
    let closed = answered.clone();
    let reading =
        process::read_on_thread("the language server's answers", from, move |mut from| {
            let mut working = std::collections::HashSet::new();
            loop {
                let message = match read_message(&mut from) {
                    Ok(message) => message,
                    // The last word: whoever is waiting is told, and whoever asks next finds
                    // the channel closed.
                    Err(failure) => {
                        let _ = answered.send(Err(failure));
                        return;
                    }
                };
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                match method {
                    // A request of us if it carries an id, a notification if it does not.
                    Some(method) => match message.get("id") {
                        Some(asked) => {
                            let answer = answer_to(&method, &message);
                            if write_to(&to, &reply(asked.clone(), answer)).is_err() {
                                return;
                            }
                        }
                        None => {
                            for note in noted(&method, &message, &mut working) {
                                told(note);
                            }
                        }
                    },
                    // An answer, for whoever is waiting on one.
                    None if answered.send(Ok(message)).is_err() => return,
                    None => {}
                }
            }
        });
    if let Err(error) = reading {
        log::warn!("the language server could not be read: {error}");
        // Nobody will read the server, so the conversation is over before it began: what
        // asks first finds a closed channel rather than a wait with no end to it.
        let _ = closed.send(Err(Failure::Broken(error.to_string())));
    }
}

/// What one notification says about the server itself, and nothing for one that says
/// nothing.
fn noted(
    method: &str,
    message: &Value,
    working: &mut std::collections::HashSet<String>,
) -> Vec<Note> {
    // The one notification a client with no capabilities of its own is told when the
    // server cannot make sense of the project. Every definition after it will be empty,
    // and this is the only place it is said.
    if method == "window/showMessage" {
        log::warn!("the language server said: {message}");
        return Vec::new();
    }
    // rust-analyzer's own account of itself, asked for in the handshake and sent by
    // nothing else. `quiescent` is the whole of what is wanted: it has read what it is
    // going to read, and an answer now is the answer it will keep giving.
    if method == SETTLED {
        let settled = message
            .get("params")
            .and_then(|params| params.get("quiescent"))
            .and_then(Value::as_bool);
        return settled.map(Note::Settled).into_iter().collect();
    }
    busy_after(method, message, working)
        .map(Note::Busy)
        .into_iter()
        .collect()
}

/// Whether the server is working, if this notification changed the answer.
///
/// Progress arrives as a token that begins and ends, and several are open at once while
/// rust-analyzer reads a project -- so what is kept is the set of them, and what is said is
/// only that it went from empty to not or back.
///
/// **Not readiness.** The gaps between those tokens are not the server being done: a small
/// crate's start has nine of them in four seconds, and the file asked about in one comes
/// back with fewer names than the same file a moment later. [`Note::Settled`] is what
/// says done, where the server says it at all.
fn busy_after(
    method: &str,
    message: &Value,
    working: &mut std::collections::HashSet<String>,
) -> Option<bool> {
    if method != "$/progress" {
        return None;
    }

    let params = message.get("params")?;
    let token = match params.get("token")? {
        Value::String(token) => token.clone(),
        token => token.to_string(),
    };
    let was = !working.is_empty();
    match params.get("value")?.get("kind")?.as_str()? {
        "begin" => working.insert(token),
        "end" => working.remove(&token),
        // A report is progress within a token that has already begun.
        _ => false,
    };
    let now = !working.is_empty();
    (was != now).then_some(now)
}

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
    /// Not JSON, once the comments and trailing commas an editor allows are taken out
    /// of it ([`as_json`]).
    NotJson(String),
    /// JSON, but not an object.
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
pub fn settings_in(directory: &Path) -> Result<Settings, Unreadable> {
    let file = directory.join(".vscode").join("settings.json");
    match std::fs::read_to_string(&file) {
        Ok(text) => settings_from(&text, directory),
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
    let read: Value = serde_json::from_str(&as_json(text))
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

/// The JSON in a settings file.
///
/// VS Code reads that file as **JSONC**, and the files in the wild are written as one: the
/// tree the whole feature is for opens with nine lines of `//`. `serde_json` takes neither
/// comments nor a trailing comma, so both are taken out here, before it sees the text.
///
/// Comments become spaces rather than nothing, and a newline inside a block comment is
/// kept, so what `serde_json` says about the line and column of a real mistake is about
/// the file the reader wrote.
fn as_json(text: &str) -> String {
    without_trailing_commas(&without_comments(text))
}

/// Comments blanked. **Nothing inside a string is touched**: a `//` is half of every URL,
/// and a string can end in an escaped quote (`"a \" // b"`) or hold a backslash before its
/// closing one (`"c:\\"`), so this tracks whether it is inside a string and whether the
/// last character was an escape. Getting that wrong cuts a path short without a word, which
/// is the failure this whole feature is against.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut string = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if string {
            out.push(character);
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match (character, chars.peek()) {
            ('"', _) => {
                string = true;
                out.push('"');
            }
            // To the end of the line, which is left where it is.
            ('/', Some('/')) => {
                out.push_str("  ");
                chars.next();
                while chars.peek().is_some_and(|next| *next != '\n') {
                    out.push(' ');
                    chars.next();
                }
            }
            // To the next `*/`, keeping the newlines so the lines below still count.
            ('/', Some('*')) => {
                out.push_str("  ");
                chars.next();
                let mut star = false;
                for character in chars.by_ref() {
                    out.push(match character {
                        '\n' => '\n',
                        _ => ' ',
                    });
                    if star && character == '/' {
                        break;
                    }
                    star = character == '*';
                }
            }
            _ => out.push(character),
        }
    }
    out
}

/// The comma before a `}` or a `]` taken out, which VS Code's own parser allows and
/// `serde_json` does not. Over text the pass above has already blanked the comments in, so
/// what is between the comma and the bracket is whitespace or nothing.
fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut string = false;
    let mut escaped = false;
    // Where the last comma was written, while nothing but whitespace has followed it.
    let mut comma: Option<usize> = None;
    for character in text.chars() {
        if string {
            out.push(character);
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => {
                string = true;
                comma = None;
            }
            ',' => comma = Some(out.len()),
            '}' | ']' => {
                if let Some(at) = comma.take() {
                    out.replace_range(at..at + 1, " ");
                }
            }
            character if character.is_whitespace() => {}
            _ => comma = None,
        }
        out.push(character);
    }
    out
}

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
/// bounded by the two values' own depth, and a parsed one is bounded by `serde_json`'s
/// nesting limit.
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

/// What to answer a request the server made of us, as a result or as an error.
///
/// A client that declared no capabilities should be asked nothing, so every arm here is a
/// server going beyond what it was told: the two that have a harmless empty answer get it,
/// and the rest are told the method is not there rather than being left waiting.
fn answer_to(method: &str, message: &Value) -> Result<Value, Value> {
    match method {
        // One setting object per item asked about, each of them "nothing to override".
        "workspace/configuration" => {
            let items = message
                .get("params")
                .and_then(|params| params.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Ok(Value::Array(vec![json!({}); items]))
        }
        // Registering a capability and making a progress token both answer with nothing.
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => Ok(Value::Null),
        _ => Err(json!({ "code": -32601, "message": "not a method this client has" })),
    }
}

/// A response to a request the server made: what `answer_to` decided, under the id it was
/// asked with.
fn reply(id: Value, answer: Result<Value, Value>) -> Value {
    match answer {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    }
}

/// The position a question is about, as every question about one sends it.
fn asked_at(file: &Path, line: u32, column: u32) -> Value {
    json!({
        "textDocument": { "uri": uri_of(file) },
        "position": { "line": line, "character": column },
    })
}

/// Whether the server said it takes documents from the client: `textDocumentSync` as
/// either the table with `openClose` or the number the older spelling of it is, where
/// anything but `None` means open and close are sent.
///
/// Asked rather than assumed, unlike the semantic tokens beside it: this one is in the
/// specification, every server answers it, and a `didOpen` to a server that says it takes
/// none is a message it is entitled to treat as a broken client.
fn opens_documents(said: &Value) -> bool {
    let Some(sync) = said
        .get("capabilities")
        .and_then(|value| value.get("textDocumentSync"))
    else {
        return false;
    };
    match sync {
        // The table: `openClose` says outright.
        Value::Object(_) => sync
            .get("openClose")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        // The number: 0 is none, 1 is full text and 2 is incremental, and both of the
        // last two carry open and close.
        value => value.as_u64().is_some_and(|kind| kind != 0),
    }
}

/// What the handshake's reply said it would spell semantic tokens with, and an empty
/// legend where it offered none.
///
/// Both lists are taken as they came and in the order they came: an index in an answer is
/// a position in these.
fn legend_of(said: &Value) -> Legend {
    let names = |of: &str| -> Vec<String> {
        said.get("capabilities")
            .and_then(|value| value.get("semanticTokensProvider"))
            .and_then(|value| value.get("legend"))
            .and_then(|value| value.get(of))
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .map(|name| name.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    Legend {
        types: names("tokenTypes"),
        modifiers: names("tokenModifiers"),
    }
}

/// The tokens an answer holds, out of the flat array of numbers it sends them as.
///
/// Five numbers each, and **every one relative to the token before it**: the lines since
/// the last, the columns since the last where that is zero and from the start of the line
/// where it is not, the length, the type, and the modifiers as a bitset. The first token
/// counts from line zero, column zero.
///
/// A length that is not a multiple of five is a message this cannot read the end of, so
/// what it could read is kept and the rest dropped; that and a number too big for a `u32`
/// are the only ways an answer here is not an answer, and neither is worth a word to the
/// reader (`AGENTS.md`: never panic on any file input, and a server's answer is one).
fn tokens(answer: &Value) -> Vec<Token> {
    let Some(data) = answer
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| answer.as_array())
    else {
        return Vec::new();
    };
    let mut tokens = Vec::with_capacity(data.len() / 5);
    let (mut line, mut column) = (0u32, 0u32);
    for five in data.chunks_exact(5) {
        let read = |at: usize| -> Option<u32> { u32::try_from(five.get(at)?.as_u64()?).ok() };
        let (Some(down), Some(along), Some(length), Some(kind), Some(modifiers)) =
            (read(0), read(1), read(2), read(3), read(4))
        else {
            break;
        };
        line = line.saturating_add(down);
        // A token on the same line as the one before it carries on from where that one
        // started; one on a later line counts from the start of its own.
        column = match down {
            0 => column.saturating_add(along),
            _ => along,
        };
        tokens.push(Token {
            // The protocol counts lines from zero and everything else here counts from
            // one, as `places` converts them.
            line: line.saturating_add(1),
            columns: column..column.saturating_add(length),
            kind,
            modifiers,
        });
    }
    tokens
}

/// The line and the columns one `range` names: the line **1-based**, as a [`Place`]'s is
/// and for its reason, and the columns in the UTF-16 units they came in.
///
/// The columns are a name's only where the range is one line's: one that ends on another
/// names more than a name, and the empty run is what says so.
fn spanned(range: &Value) -> Option<(u32, Range<u32>)> {
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()?;
    let at = |place: &Value| -> u32 {
        place
            .get("character")
            .and_then(Value::as_u64)
            .and_then(|column| u32::try_from(column).ok())
            .unwrap_or(0)
    };
    let from = at(start);
    let ends_here = |end: &&Value| end.get("line").and_then(Value::as_u64) == Some(line);
    let to = range.get("end").filter(ends_here).map_or(from, at);
    // The protocol counts from zero and everything else here counts from one.
    Some((
        u32::try_from(line).ok()?.saturating_add(1),
        from..to.max(from),
    ))
}

/// What one hover answer says, and nothing for one that says nothing. `line` and `column`
/// are the question's, in the units it was asked in.
///
/// The columns are the answer's own where it named a range, and the question's otherwise:
/// a server need not say what it answered about, and what the box is drawn against has to
/// be something either way.
fn hovered(answer: &Value, line: u32, column: u32) -> Option<Hovered> {
    let text = contents(answer.get("contents")?);
    // rust-analyzer's own begins with a newline, and a box drawn around blank space is a
    // box about nothing.
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (line, columns) = answer
        .get("range")
        .and_then(spanned)
        .unwrap_or((line.saturating_add(1), column..column));
    Some(Hovered {
        text: text.to_owned(),
        line,
        columns,
    })
}

/// One `contents`, whichever of the three shapes it came in, as the markdown the box is
/// drawn from.
///
/// The handshake asks for markdown, so a `MarkupContent` is what should arrive; the bare
/// string and the `{language, value}` pair the specification has since deprecated are
/// read too, since a server that sends one costs a match arm here and would otherwise
/// cost the answer. A pair naming a language becomes a fenced block: what it holds is
/// code, and a fence is how markdown says so. An array is joined by a blank line, which
/// is a paragraph break.
fn contents(value: &Value) -> String {
    match value {
        Value::String(said) => said.clone(),
        Value::Array(values) => values
            .iter()
            .map(contents)
            .filter(|said| !said.trim().is_empty())
            .collect::<Vec<String>>()
            .join("\n\n"),
        Value::Object(_) => match (
            value.get("language").and_then(Value::as_str),
            value.get("value").and_then(Value::as_str),
        ) {
            (Some(language), Some(said)) => format!("```{language}\n{said}\n```"),
            (None, Some(said)) => said.to_owned(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// The places an answer names, whichever of the shapes it came in.
///
/// No `linkSupport` was declared, so a list of plain locations is what should arrive; the
/// bare location and the link are read too, since a server that sends one costs a `match`
/// arm here and would otherwise cost the answer.
fn places(answer: &Value) -> Vec<Place> {
    let one = |value: &Value| {
        let uri = value
            .get("uri")
            .or_else(|| value.get("targetUri"))
            .and_then(Value::as_str)?;
        let range = value.get("range").or_else(|| value.get("targetRange"))?;
        let (line, columns) = spanned(range)?;
        Some(Place {
            file: path_of(uri)?,
            line,
            columns,
        })
    };

    match answer {
        Value::Array(values) => values.iter().filter_map(one).collect(),
        Value::Object(_) => one(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

impl<W> Drop for Talk<W> {
    /// Close the server's input. It is how a server is told there is nothing more coming
    /// -- a language server reads its input to the end and then leaves -- and it is also
    /// what lets the reader thread go, since its own read ends when the server does.
    fn drop(&mut self) {
        *self.to.lock().unwrap_or_else(|held| held.into_inner()) = None;
    }
}

/// Write one message: the header the protocol frames with, and the body.
pub fn write_message(to: &mut impl Write, body: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(body)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    message.extend_from_slice(&body);
    // One write and not three: a message the server reads half of is one it waits on.
    to.write_all(&message)?;
    to.flush()
}

/// Read one message. The headers up to the blank line, then exactly the length they said.
pub fn read_message(from: &mut impl BufRead) -> Result<Value, Failure> {
    let mut length = None;
    loop {
        let mut header = String::new();
        let read = from
            .read_line(&mut header)
            .map_err(|error| Failure::Broken(error.to_string()))?;
        if read == 0 {
            return Err(Failure::Broken("it closed the connection".to_owned()));
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }

    let Some(length) = length.filter(|length| *length <= MAX_MESSAGE) else {
        return Err(Failure::Broken(
            "a message with no usable length".to_owned(),
        ));
    };
    let mut body = vec![0; length];
    from.read_exact(&mut body)
        .map_err(|error| Failure::Broken(error.to_string()))?;
    serde_json::from_slice(&body).map_err(|error| Failure::Broken(error.to_string()))
}

/// The project's directory as the server is told about it: absolute, since a `rootUri` is
/// a URI and a relative one names a place nobody has.
///
/// `path::absolute` and not the `fs::canonicalize` `src/cargo.rs` uses on Unix, because
/// nothing here has to match a spelling something else prints back: the server's answers
/// are reconciled against the tabs already open (`ui::follow`). Resolving would only cost:
/// the reader's own spelling of their project, and on Windows a verbatim prefix
/// (`\\?\C:\work`) that no `file:` URI can carry.
///
/// The process needs none of this: [`start_program_in`] hands the same relative directory
/// to `current_dir`, which the spawn resolves against the same working directory this
/// does.
fn rooted(directory: &Path) -> PathBuf {
    std::path::absolute(directory).unwrap_or_else(|_| directory.to_path_buf())
}

/// A path as the `file:` URI the protocol names files by.
///
/// Percent-encoded by hand rather than by a crate: what has to be escaped is every byte
/// that is not unreserved, and a path is the only thing this app ever puts in a URI.
fn uri_of(path: &Path) -> String {
    let path = path.to_string_lossy();
    let mut uri = String::from("file://");
    // A Windows path starts with a drive letter and not with a separator, and the
    // authority-less form needs the third slash either way.
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                uri.push(byte as char)
            }
            b'\\' => uri.push('/'),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The path a `file:` URI names, or nothing if it names something else.
fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // The path begins at the third slash. Anything between the second and the third is an
    // authority, and that names a file on somebody else's machine.
    if !rest.starts_with('/') {
        return None;
    }

    let mut bytes = Vec::with_capacity(rest.len());
    let mut characters = rest.bytes();
    while let Some(byte) = characters.next() {
        match byte {
            b'%' => {
                let (high, low) = (characters.next()?, characters.next()?);
                let digits = [high, low];
                let text = std::str::from_utf8(&digits).ok()?;
                bytes.push(u8::from_str_radix(text, 16).ok()?);
            }
            byte => bytes.push(byte),
        }
    }

    let path = String::from_utf8(bytes).ok()?;
    Some(PathBuf::from(spelled(&path).as_ref()))
}

/// A decoded URI path as the platform it names spells one.
///
/// `/C:/x/y.rs` is how a Windows path comes back: both the leading slash and the
/// separators are the URI's, where the app spells that file `C:\x\y.rs`. A
/// [`Document::Source`](crate::project::Document) is compared as text and never
/// canonicalised, so the two spellings are two tabs of one file.
///
/// The drive letter is what says a path is Windows', not a `cfg`, so the rule is the same
/// everywhere and can be tested from either platform -- no Unix path begins with one, and
/// one keeps its leading slash and every character after it.
fn spelled(path: &str) -> Cow<'_, str> {
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => {
            Cow::Owned(path[1..].replace('/', "\\"))
        }
        _ => Cow::Borrowed(path),
    }
}

#[cfg(test)]
mod tests;

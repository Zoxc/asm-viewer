use std::io::{Cursor, PipeReader, PipeWriter};
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::temporary::Temporary;

impl Legend {
    /// A legend as a test spells one, the real ones coming off a handshake.
    pub fn of(types: &[&str], modifiers: &[&str]) -> Legend {
        let owned = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Legend {
            types: owned(types),
            modifiers: owned(modifiers),
        }
    }
}

/// A place in the file every question here is about, in the app's units: the line
/// 1-based, as a [`Lookup`]'s is, and the column a byte offset into it.
fn at(line: u32, column: usize) -> Lookup {
    Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line,
        column,
    }
}

/// A message as it goes on the wire, for the tests that assert about bytes.
fn framed(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// What a `Talk` over two pipes writes, read back as messages.
struct Fake {
    to: PipeWriter,
    from: BufReader<PipeReader>,
}

/// Every remark the client's reader made of what the server said unasked.
type Notes = Arc<Mutex<Vec<Note>>>;

impl Fake {
    /// A conversation and the other end of it: what the client says is read with
    /// `read_message`, and what `Fake::say` writes is what the client reads. `read` is how
    /// the conversation reads a file it needs the text of.
    fn pair(read: ReadText) -> (Talk<PipeWriter>, Fake, Notes) {
        let (server_reads, client_writes) = std::io::pipe().expect("a pipe");
        let (client_reads, server_writes) = std::io::pipe().expect("a pipe");
        let notes: Notes = Arc::new(Mutex::new(Vec::new()));
        let told = {
            let notes = notes.clone();
            move |note| {
                notes
                    .lock()
                    .unwrap_or_else(|held| held.into_inner())
                    .push(note)
            }
        };
        (
            Talk::over(client_writes, BufReader::new(client_reads), told, read),
            Fake {
                to: server_writes,
                from: BufReader::new(server_reads),
            },
            notes,
        )
    }

    fn say(&mut self, body: Value) {
        write_message(&mut self.to, &body).expect("a written message");
    }
}

/// Answer whatever the client asks next, and hand back what it asked.
///
/// The conversation is blocking on both sides, so the fake server runs on a thread of its
/// own and the test drives the client.
fn against<T>(
    answer: impl Fn(&mut Fake, &Value) + Send + 'static,
    ask: impl FnOnce(&mut Talk<PipeWriter>) -> T,
) -> (Vec<Value>, T, Notes) {
    against_reading(|_| None, answer, ask)
}

/// The same over a conversation that reads the files it needs the text of with `read`:
/// what the tests about a column hand over instead of writing a file.
fn against_reading<T>(
    read: ReadText,
    answer: impl Fn(&mut Fake, &Value) + Send + 'static,
    ask: impl FnOnce(&mut Talk<PipeWriter>) -> T,
) -> (Vec<Value>, T, Notes) {
    let (mut talk, mut fake, notes) = Fake::pair(read);
    let server = std::thread::spawn(move || {
        let mut heard = Vec::new();
        // Every message the client sends until it drops its end.
        while let Ok(message) = read_message(&mut fake.from) {
            answer(&mut fake, &message);
            heard.push(message);
        }
        heard
    });
    let asked = ask(&mut talk);
    // The client's end goes, so the fake server's read ends and it hands back what it
    // heard.
    drop(talk);
    (server.join().expect("the fake server"), asked, notes)
}

// The wire format.

#[test]
fn a_message_is_written_with_the_header_the_protocol_frames_with() {
    let mut written = Vec::new();
    write_message(&mut written, &json!({ "id": 1 })).expect("a written message");

    // The separator is a colon *and a space*: the server's own reader splits on `": "`
    // and calls anything else a malformed header.
    assert_eq!(written, framed(r#"{"id":1}"#));
}

#[test]
fn a_written_message_is_read_back() {
    let mut written = Vec::new();
    let body = json!({ "jsonrpc": "2.0", "id": 7, "result": { "of": "it" } });
    write_message(&mut written, &body).expect("a written message");

    assert_eq!(read_message(&mut Cursor::new(written)), Ok(body));
}

#[test]
fn the_length_is_counted_in_bytes_and_not_in_characters() {
    let mut written = Vec::new();
    let body = json!({ "text": "hør" });
    write_message(&mut written, &body).expect("a written message");

    assert_eq!(read_message(&mut Cursor::new(written)), Ok(body));
}

#[test]
fn a_header_this_client_does_not_know_is_stepped_over() {
    let message =
        b"Content-Type: application/vscode-jsonrpc\r\ncontent-length: 8\r\n\r\n{\"a\":1}\x20";
    let read = read_message(&mut Cursor::new(&message[..]));

    // And the name is matched without regard to case, which the specification allows.
    assert_eq!(read, Ok(json!({ "a": 1 })));
}

#[test]
fn a_message_with_no_length_is_a_broken_conversation() {
    let message = b"Content-Type: text/plain\r\n\r\n{}";
    assert!(matches!(
        read_message(&mut Cursor::new(&message[..])),
        Err(Failure::Broken(_))
    ));
}

#[test]
fn a_body_that_stops_short_is_a_broken_conversation() {
    let mut message = framed(r#"{"id":1}"#);
    message.pop();
    assert!(matches!(
        read_message(&mut Cursor::new(message)),
        Err(Failure::Broken(_))
    ));
}

/// A program that writes to stdout with no newline is not kept whole: the read stops a
/// header's length in.
#[test]
fn a_header_line_with_no_end_is_a_broken_conversation() {
    let mut pipe = Cursor::new(vec![b'x'; 4 * MAX_HEADER as usize]);
    assert!(matches!(read_message(&mut pipe), Err(Failure::Broken(_))));
    assert!(pipe.position() <= MAX_HEADER);
}

#[test]
fn a_closed_connection_is_a_broken_conversation() {
    assert!(matches!(
        read_message(&mut Cursor::new(Vec::new())),
        Err(Failure::Broken(_))
    ));
}

// What an answer says.

#[test]
fn a_definition_answer_is_read_in_each_shape_it_may_come_in() {
    let range =
        json!({ "start": { "line": 11, "character": 4 }, "end": { "line": 11, "character": 9 } });
    let place = Place {
        file: PathBuf::from("/p/src/main.rs"),
        // The protocol's line 11 is the twelfth line, and its columns are the server's
        // until the conversion takes them ([`Talk::back`]).
        line: 12,
        columns: Wire::of(4..9),
    };

    let location = json!({ "uri": "file:///p/src/main.rs", "range": range });
    assert_eq!(places(&json!([location])), vec![place.clone()]);
    assert_eq!(places(&location), vec![place.clone()]);
    let link = json!({ "targetUri": "file:///p/src/main.rs", "targetRange": range });
    assert_eq!(places(&json!([link])), vec![place]);
}

/// The line is what opens the file, so an answer that leaves the columns out is still a
/// place -- read as an empty run at column 0, where a caret with nothing better to say
/// sits anyway.
#[test]
fn an_answer_with_no_column_is_read_at_column_zero() {
    let range = json!({ "start": { "line": 0 }, "end": { "line": 0 } });
    let location = json!({ "uri": "file:///p/x.rs", "range": range });
    assert_eq!(
        places(&location),
        vec![Place {
            file: PathBuf::from("/p/x.rs"),
            line: 1,
            columns: Wire::of(0..0),
        }]
    );
}

#[test]
fn a_range_that_ends_on_another_line_names_no_columns() {
    let answer = json!([{
        "uri": "file:///p/src/main.rs",
        "range": { "start": { "line": 3, "character": 7 },
                   "end": { "line": 5, "character": 2 } },
    }]);
    let found = places(&answer);
    assert_eq!(found[0].line, 4);
    // Empty, and where the name begins: a run that selects nothing.
    assert_eq!(found[0].columns, Wire::of(7..7));
}

#[test]
fn an_answer_that_names_nowhere_is_no_places() {
    assert_eq!(places(&Value::Null), Vec::new());
    assert_eq!(places(&json!([])), Vec::new());
    assert_eq!(places(&json!([{ "uri": "file:///p/x.rs" }])), Vec::new());
}

#[test]
fn a_request_from_a_server_this_client_told_nothing_is_answered_emptily() {
    let asked = json!({ "params": { "items": [{ "section": "rust-analyzer" }, {}] } });
    assert_eq!(
        answer_to("workspace/configuration", &asked),
        Ok(json!([{}, {}]))
    );
    assert_eq!(
        answer_to("window/workDoneProgress/create", &json!({})),
        Ok(Value::Null)
    );
    // And a method this client has no answer for is said not to be here, rather than
    // leaving the server waiting on a reply that is never coming.
    assert!(answer_to("workspace/applyEdit", &json!({})).is_err());
}

// The conversation.

#[test]
fn the_handshake_is_initialize_and_then_initialized() {
    let (said, (), _notes) = against(
        |fake, message| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "capabilities": {} },
                }));
            }
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
        },
    );

    let methods: Vec<&str> = said
        .iter()
        .map(|message| message["method"].as_str().expect("a method"))
        .collect();
    // In this order and with nothing in between: the server reads the notification
    // itself, and a message before it ends the conversation.
    assert_eq!(methods, ["initialize", "initialized"]);
    assert_eq!(said[0]["params"]["rootUri"], json!("file:///p"));
    // Four things are declared and nothing else: progress, which is how far a server that
    // says nothing else about itself has got; the format a hover is written in, which
    // rust-analyzer flattens to plain text for a client that names none; how a column is
    // counted, which the app counts in bytes; and the notification a server sends when it
    // has settled, which no specification has and which a server without one simply never
    // sends. None of them is what would have it ask this app for configuration or for a
    // file watcher.
    assert_eq!(
        said[0]["params"]["capabilities"],
        json!({
            "window": { "workDoneProgress": true },
            "textDocument": { "hover": { "contentFormat": ["markdown"] } },
            "general": { "positionEncodings": ["utf-8", "utf-16"] },
            "experimental": { "serverStatusNotification": true },
        })
    );
    // The options are what this app asks of every server, and what a project's own
    // settings will be laid over: two lines, turning off the check it would otherwise run
    // on loading the workspace and the diagnostics it would compute for every file the
    // app opens and this app never draws.
    assert_eq!(said[0]["params"]["initializationOptions"], wanted());
    assert_eq!(
        wanted(),
        json!({ "checkOnSave": false, "diagnostics": { "enable": false } })
    );
}

/// A conversation with a server whose handshake said `sync` about taking documents, and
/// every message it heard after the handshake.
fn opening(sync: Value) -> Vec<Value> {
    let (said, (), _notes) = against(
        move |fake, message| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "capabilities": { "textDocumentSync": sync.clone() } },
                }));
            }
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.opened(Path::new("/p/src/main.rs"), "rust", "fn main() {}")
                .expect("an opening");
            talk.closed(Path::new("/p/src/main.rs")).expect("a closing");
        },
    );
    said.into_iter().skip(2).collect()
}

/// The app owns the documents it shows: the server is told what is in them, and told when
/// it stops showing them.
#[test]
fn a_file_the_app_shows_is_opened_with_the_server_and_closed_after() {
    let heard = opening(json!({ "openClose": true, "change": 2 }));
    assert_eq!(
        heard,
        [
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": {
                    "uri": "file:///p/src/main.rs",
                    "languageId": "rust",
                    "version": 1,
                    "text": "fn main() {}",
                } },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": { "textDocument": { "uri": "file:///p/src/main.rs" } },
            }),
        ]
    );
}

/// **A server that says it takes no documents is told about none.** The specification has
/// this one, unlike the semantic tokens beside it, and a notification a server said it
/// does not take is a client it is entitled to call broken.
#[test]
fn a_server_that_takes_no_documents_is_told_about_none() {
    let none: Vec<Value> = Vec::new();
    assert_eq!(opening(json!({ "openClose": false, "change": 0 })), none);
    assert_eq!(opening(json!(0)), none);
    // Nothing said at all, which is the same answer.
    assert_eq!(opening(Value::Null), none);
    // The older spelling, a number: 1 is the whole text and 2 is incremental, and both
    // carry an open and a close.
    assert_eq!(opening(json!(1)).len(), 2);
    assert_eq!(opening(json!(2)).len(), 2);
}

/// A handshake against a fake server that says only that it has capabilities, and the
/// `initialize` it heard.
fn handshake_over(directory: &Path) -> Value {
    let directory = directory.to_path_buf();
    let (said, (), _notes) = against(
        |fake, message| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "capabilities": {} },
                }));
            }
        },
        move |talk| {
            talk.initialize(&directory, &wanted()).expect("a handshake");
        },
    );
    said[0]["params"].clone()
}

/// The directory box takes any spelling, and `.` is what a reader who launched the app
/// from their project types. A relative `rootUri` names a place the server cannot find,
/// which it says only in a message this client logs.
#[test]
fn a_relative_project_directory_is_named_to_the_server_as_an_absolute_one() {
    let typed = Path::new("dev/viewer");
    let params = handshake_over(typed);
    let root = params["rootUri"].as_str().expect("a root");

    assert_eq!(
        path_of(root),
        Some(std::path::absolute(typed).expect("an absolute path"))
    );
    // The folder the server is given is the root, and it is named after the directory --
    // which `.` has no name for until it has been resolved.
    assert_eq!(params["workspaceFolders"][0]["uri"], json!(root));
    let here = handshake_over(Path::new("."));
    assert_ne!(here["workspaceFolders"][0]["name"], json!(""));
}

/// The one line the conversion tests are about: `// \u{1f980} helper`, where the crab is
/// four bytes and two UTF-16 units, so `helper` begins at byte 8 and at column 6.
const WIDE_LINE: &str = "// \u{1f980} helper\n";

fn reads_wide(_: &Path) -> Option<String> {
    Some(WIDE_LINE.to_owned())
}

/// The places `question` about `at` is answered with, for a test whose files are not on
/// the disk: there is nothing to count the columns against, so the numbers stand.
fn asked_places(
    talk: &mut Talk<PipeWriter>,
    question: Question,
    at: &Lookup,
) -> Result<Vec<Place>, Failure> {
    talk.places(question, at, &mut Lines::reading(|_| None))
}

/// How many files [`counts_reads`] was asked for, for the test that a server counting in
/// bytes has nothing read for it.
static READS: AtomicU32 = AtomicU32::new(0);

fn counts_reads(path: &Path) -> Option<String> {
    READS.fetch_add(1, Ordering::SeqCst);
    reads_wide(path)
}

/// A handshake against a server that answers `positionEncoding` with `chose`, or with
/// nothing at all for [`Value::Null`]: what was offered, and what the client made of the
/// answer.
fn encoding_chosen(chose: Value) -> (Value, Encoding) {
    let (said, taken, _notes) = against(
        move |fake, message| {
            if message["method"] == json!("initialize") {
                let mut capabilities = json!({});
                if !chose.is_null() {
                    capabilities["positionEncoding"] = chose.clone();
                }
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "capabilities": capabilities },
                }));
            }
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.encoding
        },
    );
    (said[0]["params"]["capabilities"]["general"].clone(), taken)
}

#[test]
fn the_handshake_asks_for_bytes_and_believes_what_it_is_answered() {
    // Both are offered and bytes come first, the order being the preference. A server
    // that takes them leaves nothing here to convert.
    let (offered, taken) = encoding_chosen(json!("utf-8"));
    assert_eq!(offered, json!({ "positionEncodings": ["utf-8", "utf-16"] }));
    assert_eq!(taken, Encoding::Utf8);

    // UTF-16 is the protocol's default, so it is what a server that says nothing has
    // kept -- `positionEncoding` arrived in 3.17 -- and what anything this app never
    // offered is read as. Never an error: a server is not broken for being older.
    assert_eq!(encoding_chosen(Value::Null).1, Encoding::Utf16);
    assert_eq!(encoding_chosen(json!("utf-16")).1, Encoding::Utf16);
    assert_eq!(encoding_chosen(json!("utf-32")).1, Encoding::Utf16);
    assert_eq!(encoding_chosen(json!(8)).1, Encoding::Utf16);
}

/// A conversation with a server that answered `encoding`, asked about byte 8 of the first
/// line and answering with `columns` of it: what went out, and what came back.
fn asked_over_wide_line(
    encoding: &'static str,
    columns: Range<u32>,
    read: ReadText,
) -> (Value, Vec<Place>) {
    let (said, found, _notes) = against_reading(
        read,
        move |fake, message| {
            let result = match message["method"] == json!("initialize") {
                true => json!({ "capabilities": { "positionEncoding": encoding } }),
                false => json!([{
                    "uri": "file:///p/src/main.rs",
                    "range": {
                        "start": { "line": 0, "character": columns.start },
                        "end": { "line": 0, "character": columns.end },
                    },
                }]),
            };
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": result,
            }));
        },
        move |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.places(
                Question::Followed(Followed::Definition),
                &at(1, 8),
                &mut Lines::reading(read),
            )
            .expect("an answer")
        },
    );
    let asked = said
        .iter()
        .find(|message| message["method"] == json!("textDocument/definition"))
        .expect("the question");
    (asked["params"]["position"].clone(), found)
}

#[test]
fn a_server_that_kept_utf_16_has_every_column_converted() {
    // The app counts a column in bytes and this server counts in units, so the two
    // numbers differ by the crab: byte 8 is column 6, and the columns of `helper` are
    // 6..12 to the server and 8..14 here.
    let (asked, found) = asked_over_wide_line("utf-16", 6..12, reads_wide);

    assert_eq!(asked, json!({ "line": 0, "character": 6 }));
    assert_eq!(found[0].columns, 8..14);
}

#[test]
fn a_server_that_took_utf_8_is_asked_in_bytes_and_reads_nothing() {
    READS.store(0, Ordering::SeqCst);
    let (asked, found) = asked_over_wide_line("utf-8", 8..14, counts_reads);

    // Both numbers go through untouched, and no file is opened to touch them: what a
    // conversion would cost is what asking for bytes is for.
    assert_eq!(asked, json!({ "line": 0, "character": 8 }));
    assert_eq!(found[0].columns, 8..14);
    assert_eq!(READS.load(Ordering::SeqCst), 0);
}

/// A file is cut into lines where [`str::lines`] cuts it, and it is cut once: the `\r`
/// of a CRLF goes with the newline, a last line without one is a line, and there is
/// nothing past the end. A line of zero is a line no file has and not a panic.
#[test]
fn a_files_lines_are_the_lines_str_lines_finds() {
    fn reads_mixed(_: &Path) -> Option<String> {
        Some("one\r\ntwo\n\nfour".to_owned())
    }
    let file = Path::new("/p/src/main.rs");
    let mut lines = Lines::reading(reads_mixed);

    let found: Vec<Option<String>> = (0..6)
        .map(|line| lines.at(file, line).map(str::to_owned))
        .collect();
    let text: Vec<Option<&str>> = found.iter().map(Option::as_deref).collect();
    assert_eq!(
        text,
        [None, Some("one"), Some("two"), Some(""), Some("four"), None]
    );

    // A file that will not read has no lines at all.
    assert_eq!(Lines::reading(|_| None).at(file, 1), None);
}

/// How many files [`counts_answer_reads`] was asked for. Its own counter and not
/// [`READS`]: the tests run at the same time.
static ANSWER_READS: AtomicU32 = AtomicU32::new(0);

fn counts_answer_reads(path: &Path) -> Option<String> {
    ANSWER_READS.fetch_add(1, Ordering::SeqCst);
    reads_wide_lines(path)
}

/// **One answer reads each file it names once.** The columns come back off the wire
/// through the answer's own reader, and the rows drawn from them take their text from the
/// same one, so a file is opened once however many lines of it the answer needs.
#[test]
fn one_answer_reads_each_file_it_names_once() {
    ANSWER_READS.store(0, Ordering::SeqCst);
    let mut lines = Lines::reading(counts_answer_reads);
    let (_said, found, _notes) = against_reading(
        counts_answer_reads,
        |fake, message| {
            let result = match message["method"] == json!("initialize") {
                true => json!({ "capabilities": { "positionEncoding": "utf-16" } }),
                // `helper` on the first line, and `helper` on the second.
                false => json!([
                    {
                        "uri": "file:///p/src/main.rs",
                        "range": { "start": { "line": 0, "character": 6 },
                                   "end": { "line": 0, "character": 12 } },
                    },
                    {
                        "uri": "file:///p/src/main.rs",
                        "range": { "start": { "line": 1, "character": 8 },
                                   "end": { "line": 1, "character": 14 } },
                    },
                ]),
            };
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": result,
            }));
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.places(Question::Listed(Listed::References), &at(1, 8), &mut lines)
                .expect("an answer")
        },
    );

    // What came back off the wire: the server's units as the app's bytes.
    let columns: Vec<Range<usize>> = found.iter().map(|place| place.columns.clone()).collect();
    assert_eq!(columns, [8..14, 12..18]);

    // And the same places as the panel draws them, through the reader the wire's own
    // conversion has already filled.
    let references = crate::references::of(&found, &mut lines);
    let drawn: Vec<Range<usize>> = references
        .rows(&crate::filter::Matcher::Everything)
        .iter()
        .filter_map(|row| match row {
            crate::grouped::Row::Item { item, .. } => Some(item.columns.clone()),
            crate::grouped::Row::File { .. } => None,
        })
        .collect();
    assert_eq!(drawn, [8..14, 12..18]);

    assert_eq!(
        ANSWER_READS.load(Ordering::SeqCst),
        1,
        "the wire and the drawing read one text"
    );
}

#[test]
fn a_definition_is_asked_for_where_the_reader_pointed_and_answered_with_the_place() {
    let (said, found, _notes) = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": [{
                    "uri": "file:///p/src/other.rs",
                    "range": { "start": { "line": 3, "character": 8 },
                               "end": { "line": 3, "character": 14 } },
                }],
            }));
        },
        |talk| {
            asked_places(talk, Question::Followed(Followed::Definition), &at(42, 17))
                .expect("an answer")
        },
    );

    assert_eq!(said[0]["method"], json!("textDocument/definition"));
    assert_eq!(
        said[0]["params"]["textDocument"]["uri"],
        json!("file:///p/src/main.rs")
    );
    // Both ways of the one unit: line 42 was asked about and 41 went out, and the line 3
    // the answer named comes back as 4.
    assert_eq!(
        said[0]["params"]["position"],
        json!({ "line": 41, "character": 17 })
    );
    assert_eq!(
        found,
        vec![Place {
            file: PathBuf::from("/p/src/other.rs"),
            line: 4,
            columns: 8..14,
        }]
    );
}

/// The answer against a fake server that replies `result` to whatever is asked.
fn hover_of(result: Value) -> Option<Hovered> {
    let (_said, found, _notes) = against(
        move |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": result.clone(),
            }));
        },
        |talk| talk.hover(&at(11, 14)).expect("an answer"),
    );
    found
}

#[test]
fn a_hover_is_asked_where_the_pointer_rested_and_answered_with_the_name_it_is_about() {
    let (said, found, _notes) = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": {
                    "contents": { "kind": "markdown", "value": "```rust\npub fn helper()\n```" },
                    "range": { "start": { "line": 10, "character": 12 },
                               "end": { "line": 10, "character": 18 } },
                },
            }));
        },
        |talk| talk.hover(&at(11, 14)).expect("an answer"),
    );

    assert_eq!(said[0]["method"], json!("textDocument/hover"));
    assert_eq!(
        said[0]["params"]["position"],
        json!({ "line": 10, "character": 14 })
    );
    assert_eq!(
        found,
        Some(Hovered {
            text: "```rust\npub fn helper()\n```".to_owned(),
            // The answer's own range, and its line counted from one.
            line: 11,
            columns: 12..18,
        })
    );
}

/// The shape a client that named a format is sent, the two the specification has
/// deprecated, and a list of them.
#[test]
fn a_hover_answer_is_read_in_each_shape_it_may_come_in() {
    let text = |found: Option<Hovered>| found.expect("an answer").text;

    assert_eq!(
        text(hover_of(
            json!({ "contents": { "kind": "markdown", "value": "what it **is**" } })
        )),
        "what it **is**"
    );
    assert_eq!(
        text(hover_of(json!({ "contents": "what it is" }))),
        "what it is"
    );
    // A `MarkedString` naming a language is code, and a fence is how markdown says so.
    assert_eq!(
        text(hover_of(
            json!({ "contents": { "language": "rust", "value": "fn helper()" } })
        )),
        "```rust\nfn helper()\n```"
    );
    assert_eq!(
        text(hover_of(json!({ "contents": [
            { "language": "rust", "value": "fn helper()" },
            "what it is",
        ] }))),
        "```rust\nfn helper()\n```\n\nwhat it is"
    );
}

/// rust-analyzer's own answer begins with a newline, and a hover over a name it has
/// nothing to say about is an empty one.
#[test]
fn a_hover_that_says_nothing_is_no_answer_at_all() {
    assert_eq!(hover_of(Value::Null), None);
    assert_eq!(hover_of(json!({ "contents": "" })), None);
    assert_eq!(hover_of(json!({ "contents": "\n \n" })), None);
    assert_eq!(hover_of(json!({ "contents": [] })), None);
}

/// A hover from a server counting in UTF-16, over the wide line: the answer's columns and
/// the fallback to the question's both come back in bytes.
///
/// The answer's range is the crab's line, so the two units differ -- `helper` is 6..12 to
/// the server and 8..14 here -- and the question went out at column 6 for byte 8, so a
/// fallback that came back untouched would be 6..6 and not the name the pointer is on.
#[test]
fn a_hover_from_a_utf_16_server_is_answered_in_bytes() {
    let asked = |range: Value| {
        let (_said, found, _notes) = against_reading(
            reads_wide,
            move |fake, message| {
                let result = match message["method"] == json!("initialize") {
                    true => json!({ "capabilities": { "positionEncoding": "utf-16" } }),
                    false => {
                        let mut answer = json!({ "contents": "what it is" });
                        if !range.is_null() {
                            answer["range"] = range.clone();
                        }
                        answer
                    }
                };
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": result,
                }));
            },
            |talk| {
                talk.initialize(Path::new("/p"), &wanted())
                    .expect("a handshake");
                talk.hover(&at(1, 8)).expect("an answer")
            },
        );
        found.expect("an answer").columns
    };

    let range = json!({
        "start": { "line": 0, "character": 6 },
        "end": { "line": 0, "character": 12 },
    });
    assert_eq!(asked(range), 8..14);
    assert_eq!(asked(Value::Null), 8..8);
}

/// The outbound half of the same line: a question goes out at the column the server
/// counts in, never at the byte the app does.
///
/// The pointer rests on byte 8 of the wide line, where `helper` begins. That is column 6
/// to a server counting UTF-16 units, and a question sent at 8 would land it two units
/// late -- inside the word, where a server answers about something else or about nothing.
/// A server that took `utf-8` is asked at the byte, there being nothing to count.
#[test]
fn a_hover_goes_out_at_the_column_the_server_counts() {
    let position = |encoding: &'static str| {
        let (said, _found, _notes) = against_reading(
            reads_wide,
            move |fake, message| {
                let result = match message["method"] == json!("initialize") {
                    true => json!({ "capabilities": { "positionEncoding": encoding } }),
                    false => json!({ "contents": "what it is" }),
                };
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": result,
                }));
            },
            |talk| {
                talk.initialize(Path::new("/p"), &wanted())
                    .expect("a handshake");
                talk.hover(&at(1, 8)).expect("an answer")
            },
        );
        said.iter()
            .find(|message| message["method"] == json!("textDocument/hover"))
            .expect("the question")["params"]["position"]
            .clone()
    };

    assert_eq!(position("utf-16"), json!({ "line": 0, "character": 6 }));
    assert_eq!(position("utf-8"), json!({ "line": 0, "character": 8 }));
}

/// A server that answers without saying what it answered about: the box is drawn against
/// the name the question was asked at.
#[test]
fn a_hover_with_no_range_is_about_the_column_it_was_asked_at() {
    assert_eq!(
        hover_of(json!({ "contents": "what it is" })),
        Some(Hovered {
            text: "what it is".to_owned(),
            line: 11,
            columns: 14..14,
        })
    );
}

#[test]
fn a_server_still_reading_the_project_says_nothing_about_a_name_and_does_not_fail() {
    let _ = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "error": { "code": -32801, "message": "content modified" },
            }));
        },
        |talk| {
            assert_eq!(
                talk.hover(&at(1, 0)),
                Ok(None),
                "a refusal is no answer, and not a failure to report"
            );
        },
    );
}

/// The five numbers a token is sent as, and each of them relative to the token before.
#[test]
fn the_tokens_of_an_answer_are_read_out_of_its_deltas() {
    let read = tokens(&json!({
        "data": [
            // Line 0, column 5, four wide, type 1, no modifiers.
            0, 5, 4, 1, 0,
            // Same line: the column carries on from the last one's start, so 5 + 3 = 8.
            0, 3, 2, 7, 0b101,
            // Two lines down: the column counts from the start of its own line again.
            2, 4, 6, 2, 0,
        ],
    }));

    assert_eq!(
        read,
        vec![
            // Lines count from one here, where the protocol counts from zero.
            Token {
                line: 1,
                columns: Wire::of(5..9),
                kind: 1,
                modifiers: 0
            },
            Token {
                line: 1,
                columns: Wire::of(8..10),
                kind: 7,
                modifiers: 0b101
            },
            Token {
                line: 3,
                columns: Wire::of(4..10),
                kind: 2,
                modifiers: 0
            },
        ]
    );
}

/// Every way an answer is not one. None of them is worth a word to the reader, and none
/// of them may panic: a server's answer is file input like any other (`AGENTS.md`).
#[test]
fn an_answer_that_is_not_five_numbers_a_token_is_read_as_far_as_it_goes() {
    // A length that is not a multiple of five: what can be read is kept.
    let ragged = tokens(&json!({ "data": [0, 1, 2, 3, 4, 0, 1] }));
    assert_eq!(ragged.len(), 1);

    // Nothing at all, in each of the shapes nothing comes in.
    assert_eq!(tokens(&json!({ "data": [] })), Vec::new());
    assert_eq!(tokens(&json!({})), Vec::new());
    assert_eq!(tokens(&Value::Null), Vec::new());
    assert_eq!(tokens(&json!("not an answer")), Vec::new());

    // A number no `u32` holds ends the reading rather than wrapping into a column.
    let huge = tokens(&json!({ "data": [0, 0, 1, 0, 0, 0, 99999999999u64, 1, 0, 0] }));
    assert_eq!(huge.len(), 1);

    // Lines that would count past the end saturate instead of wrapping around.
    let far = tokens(&json!({ "data": [4294967295u32, 0, 1, 0, 0] }));
    assert_eq!(far.len(), 1);
    assert_eq!(far[0].line, u32::MAX);
}

/// The legend is read off the handshake, and an index means nothing without it.
#[test]
fn the_handshake_keeps_what_the_server_will_spell_its_tokens_with() {
    let (_said, legend, _notes) = against(
        |fake, message| {
            if message.get("method").and_then(Value::as_str) == Some("initialize") {
                fake.say(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "capabilities": { "semanticTokensProvider": { "legend": {
                        "tokenTypes": ["comment", "method", "builtinType"],
                        "tokenModifiers": ["declaration", "trait"],
                    } } } },
                }));
            }
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.legend().clone()
        },
    );

    let method = Token {
        line: 1,
        columns: 0..4,
        kind: 1,
        modifiers: 0b10,
    };
    // Index 1 is the type it called `method`, and the second modifier is `trait`.
    assert_eq!(legend.kinds_named(&["method"]), vec![false, true, false]);
    assert_eq!(method.modifiers & legend.bit("trait"), 0b10);
    assert_eq!(method.modifiers & legend.bit("declaration"), 0);
    // A type it never declared, which a server of another version may still send: the
    // table stops where the legend does.
    assert_eq!(legend.kinds_named(&["method"]).get(9), None);
    // A modifier it never declared is `0`, a bit no answer can have set.
    assert_eq!(legend.bit("async"), 0);
}

/// A server that offers no semantic tokens leaves an empty legend, and is never asked.
#[test]
fn a_server_that_offers_no_tokens_is_not_asked_for_any() {
    let (said, found, _notes) = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": { "capabilities": {} },
            }));
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            assert!(talk.legend().is_empty());
            talk.semantic_tokens(Path::new("/p/src/main.rs"))
                .expect("an answer")
        },
    );

    assert_eq!(found, Vec::new());
    // The handshake's two messages and nothing else: the question was never put.
    let methods: Vec<&str> = said
        .iter()
        .map(|message| message["method"].as_str().expect("a method"))
        .collect();
    assert_eq!(methods, ["initialize", "initialized"]);
}

/// The question names the file and no position: it is about all of it.
#[test]
fn the_tokens_of_a_file_are_asked_for_by_name() {
    let (said, found, _notes) = against(
        |fake, message| {
            let answer = match message["method"] == json!("initialize") {
                true => json!({ "capabilities": { "semanticTokensProvider": { "legend": {
                    "tokenTypes": ["method"], "tokenModifiers": [],
                } } } }),
                false => json!({ "data": [0, 2, 6, 0, 0] }),
            };
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": answer,
            }));
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.semantic_tokens(Path::new("/p/src/main.rs"))
                .expect("an answer")
        },
    );

    assert_eq!(said[2]["method"], json!("textDocument/semanticTokens/full"));
    assert_eq!(
        said[2]["params"],
        json!({ "textDocument": { "uri": "file:///p/src/main.rs" } })
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].columns, 2..8);
}

/// The two lines a token's conversion is asserted over: `helper` is UTF-16 6..12 and
/// bytes 8..14 on the first, 8..14 and bytes 12..18 on the second. Two crabs on the
/// second, so a token converted against the first comes out wrong rather than the same.
const WIDE_LINES: &str = "// \u{1f980} helper\n// \u{1f980}\u{1f980} helper\n";

fn reads_wide_lines(_: &Path) -> Option<String> {
    Some(WIDE_LINES.to_owned())
}

/// A token's columns are converted through its own line, the first line included.
#[test]
fn a_token_from_a_utf_16_server_is_converted_on_the_line_it_is_on() {
    let (_said, found, _notes) = against_reading(
        reads_wide_lines,
        |fake, message| {
            let answer = match message["method"] == json!("initialize") {
                true => json!({ "capabilities": {
                    "positionEncoding": "utf-16",
                    "semanticTokensProvider": { "legend": {
                        "tokenTypes": ["method"], "tokenModifiers": [],
                    } },
                } }),
                // `helper` on the first line, and `helper` on the second.
                false => json!({ "data": [0, 6, 6, 0, 0, 1, 8, 6, 0, 0] }),
            };
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": answer,
            }));
        },
        |talk| {
            talk.initialize(Path::new("/p"), &wanted())
                .expect("a handshake");
            talk.semantic_tokens(Path::new("/p/src/main.rs"))
                .expect("an answer")
        },
    );

    assert_eq!(found.len(), 2);
    assert_eq!((found[0].line, found[0].columns.clone()), (1, 8..14));
    assert_eq!((found[1].line, found[1].columns.clone()), (2, 12..18));
}

#[test]
fn implementations_are_asked_for_where_the_reader_pointed() {
    let (said, found, _notes) = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": [
                    {
                        "uri": "file:///p/src/one.rs",
                        "range": { "start": { "line": 1, "character": 5 },
                                   "end": { "line": 1, "character": 9 } },
                    },
                    {
                        "uri": "file:///p/src/two.rs",
                        "range": { "start": { "line": 2, "character": 5 },
                                   "end": { "line": 2, "character": 9 } },
                    },
                ],
            }));
        },
        |talk| {
            asked_places(talk, Question::Listed(Listed::Implementations), &at(5, 11))
                .expect("an answer")
        },
    );

    assert_eq!(said[0]["method"], json!("textDocument/implementation"));
    assert_eq!(
        said[0]["params"]["position"],
        json!({ "line": 4, "character": 11 })
    );
    // Every place the answer named, in the order it named them: what implements a trait
    // is a list, where a definition is one place.
    assert_eq!(
        found.iter().map(|place| place.line).collect::<Vec<u32>>(),
        vec![2, 3]
    );
}

#[test]
fn references_are_asked_for_where_the_reader_pointed_and_leave_the_definition_out() {
    let (said, found, _notes) = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "result": [{
                    "uri": "file:///p/src/other.rs",
                    "range": { "start": { "line": 6, "character": 12 },
                               "end": { "line": 6, "character": 16 } },
                }],
            }));
        },
        |talk| {
            asked_places(talk, Question::Listed(Listed::References), &at(42, 17))
                .expect("an answer")
        },
    );

    assert_eq!(said[0]["method"], json!("textDocument/references"));
    assert_eq!(
        said[0]["params"]["position"],
        json!({ "line": 41, "character": 17 })
    );
    // Where the name is defined is not a use of it.
    assert_eq!(
        said[0]["params"]["context"],
        json!({ "includeDeclaration": false })
    );
    assert_eq!(
        found,
        vec![Place {
            file: PathBuf::from("/p/src/other.rs"),
            line: 7,
            columns: 12..16,
        }]
    );
}

#[test]
fn what_arrives_before_the_answer_is_dealt_with_and_the_answer_is_still_the_answer() {
    let (said, (), _notes) = against(
        |fake, message| {
            let id = message["id"].clone();
            if message["method"] == json!("textDocument/definition") {
                // A notification, which is dropped.
                fake.say(json!({ "jsonrpc": "2.0", "method": "window/logMessage",
                                 "params": { "type": 3, "message": "indexing" } }));
                // A request, which has to be answered or the server waits for ever.
                fake.say(json!({ "jsonrpc": "2.0", "id": 900,
                                 "method": "window/workDoneProgress/create",
                                 "params": { "token": "t" } }));
                // An answer to a request that is not the one outstanding.
                fake.say(json!({ "jsonrpc": "2.0", "id": 4321, "result": [] }));
                fake.say(json!({ "jsonrpc": "2.0", "id": id, "result": null }));
            }
        },
        |talk| {
            let places = asked_places(talk, Question::Followed(Followed::Definition), &at(1, 0))
                .expect("an answer");
            assert_eq!(places, Vec::new());
        },
    );

    // The client answered the server's request, under the id it was asked with, and
    // asked its own question once.
    assert_eq!(said.len(), 2);
    assert_eq!(
        said[1],
        json!({ "jsonrpc": "2.0", "id": 900, "result": null })
    );
}

/// Progress is what the app knows the server by while it is reading the project: it opens
/// tokens and closes them, several at a time, and what is said here is only that it went
/// from working to not.
#[test]
fn what_the_server_says_unasked_is_whether_it_is_working() {
    let (_said, (), notes) = against(
        |fake, message| {
            let progress = |token: &str, kind: &str| {
                json!({ "jsonrpc": "2.0", "method": "$/progress",
                        "params": { "token": token, "value": { "kind": kind } } })
            };
            fake.say(progress("rustAnalyzer/Indexing", "begin"));
            fake.say(progress("rustAnalyzer/Roots Scanned", "begin"));
            fake.say(progress("rustAnalyzer/Indexing", "report"));
            fake.say(progress("rustAnalyzer/Indexing", "end"));
            // Still one open, so nothing is said yet.
            fake.say(progress("rustAnalyzer/Roots Scanned", "end"));
            fake.say(json!({ "jsonrpc": "2.0", "id": message["id"].clone(), "result": null }));
        },
        |talk| {
            asked_places(talk, Question::Followed(Followed::Definition), &at(1, 0))
                .expect("an answer");
        },
    );

    let notes = notes
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .clone();
    assert_eq!(notes, [Note::Busy(true), Note::Busy(false)]);
}

/// One notification is **at most one remark**, whatever it says. The reader passes on what
/// it answers and nothing else, so a token that opens under a name already open, a report
/// inside one, an `end` for a token that never began and a `$/progress` with no `params`
/// at all are each nothing to say -- and a token the server spells as a number is a token.
#[test]
fn one_notification_says_at_most_one_thing_about_the_server() {
    let progress = |token: Value, kind: &str| json!({ "params": { "token": token, "value": { "kind": kind } } });
    let mut reader = Progress::default();
    let mut noted = |method: &str, message: Value| reader.noted(method, &message);

    // What the server says of the project is logged and is not a remark.
    assert_eq!(noted("window/showMessage", json!({})), None);
    // The first token open is the one thing that changed.
    assert_eq!(
        noted(
            "$/progress",
            progress(json!("rustAnalyzer/Indexing"), "begin")
        ),
        Some(Note::Busy(true))
    );
    // A second, a report inside the first, and a token spelled as a number: still working.
    assert_eq!(
        noted("$/progress", progress(json!("rustAnalyzer/Roots"), "begin")),
        None
    );
    assert_eq!(
        noted(
            "$/progress",
            progress(json!("rustAnalyzer/Indexing"), "report")
        ),
        None
    );
    assert_eq!(noted("$/progress", progress(json!(7), "begin")), None);
    // Nothing a malformed notification can say, and nothing it can panic on.
    assert_eq!(noted("$/progress", json!({})), None);
    assert_eq!(noted("$/progress", json!({ "params": {} })), None);
    assert_eq!(
        noted("$/progress", progress(json!("never/opened"), "end")),
        None
    );
    // The three open ones closed, and only the last of them is the answer changing.
    for token in [json!("rustAnalyzer/Indexing"), json!("rustAnalyzer/Roots")] {
        assert_eq!(noted("$/progress", progress(token, "end")), None);
    }
    assert_eq!(
        noted("$/progress", progress(json!(7), "end")),
        Some(Note::Busy(false))
    );
}

/// The other thing a server may say about itself, which no specification has: that it has
/// settled. Progress says nothing about this -- the tokens above open and close all
/// through a start -- so the two are told apart and both are passed on.
#[test]
fn a_server_that_says_it_has_settled_is_heard_saying_so() {
    let (_said, (), notes) = against(
        |fake, message| {
            let status = |quiescent: bool| {
                json!({ "jsonrpc": "2.0", "method": "experimental/serverStatus",
                        "params": { "health": "ok", "quiescent": quiescent } })
            };
            fake.say(status(false));
            fake.say(status(true));
            fake.say(json!({ "jsonrpc": "2.0", "id": message["id"].clone(), "result": null }));
        },
        |talk| {
            asked_places(talk, Question::Followed(Followed::Definition), &at(1, 0))
                .expect("an answer");
        },
    );

    let notes = notes
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .clone();
    assert_eq!(notes, [Note::Settled(false), Note::Settled(true)]);
}

#[test]
fn a_server_that_is_still_reading_the_project_is_no_answer_and_not_a_failure() {
    let _ = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "error": { "code": -32801, "message": "content modified" },
            }));
        },
        |talk| {
            let places = asked_places(talk, Question::Followed(Followed::Definition), &at(1, 0))
                .expect("no failure");
            assert_eq!(places, Vec::new());
        },
    );
}

#[test]
fn any_other_error_is_the_failure_the_server_named() {
    let _ = against(
        |fake, message| {
            fake.say(json!({
                "jsonrpc": "2.0",
                "id": message["id"].clone(),
                "error": { "code": -32603, "message": "it panicked" },
            }));
        },
        |talk| {
            assert_eq!(
                asked_places(talk, Question::Followed(Followed::Definition), &at(1, 0)),
                Err(Failure::Refused {
                    code: -32603,
                    said: "it panicked".to_owned(),
                })
            );
        },
    );
}

#[test]
fn a_server_that_stops_answering_ends_the_conversation() {
    let (mut talk, fake, _notes) = Fake::pair(|_| None);
    // The server's end goes away with nothing said, which is what a killed one looks
    // like from here.
    drop(fake);

    assert!(matches!(
        asked_places(
            &mut talk,
            Question::Followed(Followed::Definition),
            &at(1, 0)
        ),
        Err(Failure::Broken(_))
    ));
}

// The process.

#[test]
fn a_server_that_is_not_installed_is_a_failure_and_not_a_panic() {
    let failure = start_in("no-such-language-server", Path::new("."), |_| ())
        .err()
        .expect("no server");

    assert!(matches!(failure, Failure::NoServer(_)));
    assert!(failure.to_string().starts_with("could not start"));
}

#[test]
fn a_program_that_ended_before_the_handshake_is_one_that_would_not_start() {
    // What a rustup proxy for a toolchain without the component writes and does.
    let said = "error: Unknown binary 'rust-analyzer' in official toolchain 'nightly'.\n";
    let failure = gone_instead(
        Failure::Broken("it closed the connection".to_owned()),
        Some("exit status: 1".to_owned()),
        said,
    );

    assert_eq!(
        failure,
        Failure::NoServer(
            "error: Unknown binary 'rust-analyzer' in official toolchain 'nightly'.".to_owned()
        )
    );
    assert!(failure
        .to_string()
        .starts_with("could not start the language server: error: Unknown binary"));
}

#[test]
fn a_program_that_ended_saying_nothing_is_still_one_that_would_not_start() {
    let failure = gone_instead(
        Failure::Broken("it closed the connection".to_owned()),
        Some("exit status: 101".to_owned()),
        "   \n",
    );

    assert_eq!(
        failure,
        Failure::NoServer("it ended at once (exit status: 101)".to_owned())
    );
}

/// The wait answers what became of the program, and only a program that ended **by
/// itself** is one that would not start. A stop is this app's own doing -- the handshake's
/// own failure path stops nothing, but a reader pressing the control does -- and a program
/// still going when the wait ran out has not ended at all.
#[test]
fn only_a_program_that_ended_by_itself_is_one_that_would_not_start() {
    assert_eq!(ended_by_itself(None), None, "still going");
    assert_eq!(
        ended_by_itself(Some(Ended::Stopped)),
        None,
        "this app took it"
    );
    assert_eq!(
        ended_by_itself(Some(Ended::Exited(Some(101)))),
        Some("exit status: 101".to_owned())
    );
    assert_eq!(
        ended_by_itself(Some(Ended::Failed("it could not be waited for".to_owned()))),
        Some("it could not be waited for".to_owned())
    );
}

#[test]
fn a_conversation_that_broke_against_a_server_still_running_is_still_broken() {
    let broken = Failure::Broken("it closed the connection".to_owned());
    assert_eq!(gone_instead(broken.clone(), None, "chatter"), broken);
}

#[test]
fn what_a_program_said_is_cut_to_something_a_line_can_hold() {
    let said = "no ".repeat(200);
    let Failure::NoServer(reason) = gone_instead(
        Failure::Broken(String::new()),
        Some("exit status: 1".to_owned()),
        &said,
    ) else {
        panic!("a program that ended was not read as one that would not start");
    };

    assert!(reason.ends_with('\u{2026}'), "{reason}");
    assert!(reason.chars().count() <= SAID_CHARS + 1, "{reason}");
}

/// A reader handing back one canned chunk per `read`, which is what a pipe does: a read
/// returns whatever is there, and where that falls is the writer's own buffering.
struct InChunks(Vec<Vec<u8>>);

impl Read for InChunks {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0.is_empty() {
            return Ok(0);
        }
        let chunk = self.0.remove(0);
        buffer[..chunk.len()].copy_from_slice(&chunk);
        Ok(chunk.len())
    }
}

/// The bytes are kept and decoded once. Decoded a chunk at a time, a character the reads
/// fell across came out as two replacement characters -- in the one message this whole
/// path exists to carry, about a path the reader can read for themselves.
#[test]
fn a_character_split_across_two_reads_is_still_the_character() {
    let line = "error: /home/j\u{f6}rg/bin/rust-analyzer: not found";
    // Between the two bytes of the character, where a read that returned what was there
    // would leave it.
    let at = line.find('\u{f6}').expect("a two-byte character") + 1;
    let said = Arc::new(Mutex::new(Vec::new()));
    let chunks = vec![
        line.as_bytes()[..at].to_vec(),
        line.as_bytes()[at..].to_vec(),
    ];

    let reader = keep_stderr(Some(InChunks(chunks)), &said).expect("a thread");
    reader.join().expect("the stderr thread");

    let said = said.lock().expect("what was said");
    assert_eq!(String::from_utf8_lossy(&said), line);
}

/// A directory of this test's own under the system temporary directory, holding a `server`
/// program that `does`. The directory goes when the test ends.
#[cfg(unix)]
fn program_that(does: &str) -> Temporary {
    use std::os::unix::fs::PermissionsExt;

    let directory = Temporary::fresh_directory("lsp-test");
    let program = directory.join("server");
    std::fs::write(&program, format!("#!/bin/sh\n{does}\n")).expect("a program");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("a mode");
    directory
}

/// What a program says on its way out is waited for, and not read out of whatever the
/// stderr thread happened to have collected by then.
///
/// The program here closes its output first and speaks afterwards, which is the order the
/// race is lost in: the handshake ends on that EOF while the line is still to come. A real
/// rustup proxy writes and exits in one breath and loses the same race about half the
/// time.
#[test]
#[cfg(unix)]
fn what_a_program_said_on_its_way_out_is_waited_for() {
    let said = "error: no rust-analyzer in this toolchain";
    let directory = program_that(&format!(
        "exec 1>&-\nsleep 0.05\necho \"{said}\" >&2\nexit 1"
    ));
    let program = directory.join("server");
    let mut server = start_in(&program.to_string_lossy(), Path::new("."), |_| ()).expect("spawned");

    let failure = server
        .initialize(Path::new("."), &wanted())
        .err()
        .expect("no server");
    server.handle().stop();

    assert_eq!(failure, Failure::NoServer(said.to_owned()));
}

/// A start hands the handle out **once**. `spawned` is given the server's own, which is
/// the only one there is: what a caller holds after the handshake it asks the server for,
/// and it ends the same process.
#[test]
#[cfg(unix)]
fn the_handle_a_start_hands_over_is_the_servers_own() {
    // A program that takes the pipe and answers nothing, so the handshake never returns
    // and the only way out is the handle.
    let directory = program_that("sleep 30");
    let program = directory.join("server");
    let server = start_in(&program.to_string_lossy(), Path::new("."), |_| ()).expect("spawned");

    assert!(
        !server.handle().finished(),
        "the server was over before anything asked it anything"
    );
    // And the conversation is reached through the server itself, with no forwarder in the
    // way: nothing has been asked yet, so it takes no documents and has no legend.
    assert!(!server.opens());
    assert_eq!(server.legend(), &Legend::default());

    server.handle().stop();
    assert!(
        server.handle().finished(),
        "the handle the server answers with did not end it"
    );
}

/// The whole of it against a real program: one that exits at once is a server that would
/// not start, and not a conversation that broke.
#[test]
#[cfg(unix)]
fn a_handshake_with_a_program_that_exits_at_once_says_it_would_not_start() {
    // The failure drops the server, which stops the process.
    let failure = start("true", Path::new("."), &wanted(), |_| (), |_| ())
        .err()
        .expect("no server");

    assert!(
        matches!(failure, Failure::NoServer(_)),
        "a program that ended was reported as {failure}"
    );
}

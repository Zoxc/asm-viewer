use std::io::{Cursor, PipeReader, PipeWriter};

use super::*;

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
    /// `read_message`, and what `Fake::say` writes is what the client reads.
    fn pair() -> (Talk<PipeWriter>, Fake, Notes) {
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
            Talk::over(client_writes, BufReader::new(client_reads), told),
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
    let (mut talk, mut fake, notes) = Fake::pair();
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

#[test]
fn a_closed_connection_is_a_broken_conversation() {
    assert!(matches!(
        read_message(&mut Cursor::new(Vec::new())),
        Err(Failure::Broken(_))
    ));
}

// Files, both ways.

#[test]
fn a_path_becomes_a_uri_and_comes_back() {
    for path in [
        "/home/reader/a project/src/main.rs",
        "/home/reader/hør/lib.rs",
        "/home/reader/plain.rs",
    ] {
        let uri = uri_of(Path::new(path));
        assert!(uri.starts_with("file:///"), "{uri}");
        assert_eq!(path_of(&uri), Some(PathBuf::from(path)), "{uri}");
    }
}

#[test]
fn a_space_in_a_path_is_escaped() {
    assert_eq!(uri_of(Path::new("/a b")), "file:///a%20b");
}

/// A Windows path comes back spelled the way it went out. `uri_of` writes every separator
/// as `/` and the URI carries a leading slash the path has not got, so a path that came
/// back as the URI spelled it named a file the app already had open under another
/// spelling -- and a `Document::Source` is compared as text.
///
/// The drive letter is what says the path is Windows', so the rule holds on either
/// platform and this test runs on both.
#[test]
fn a_windows_path_comes_back_with_its_own_separators() {
    let uri = uri_of(Path::new(r"C:\Users\reader\src\main.rs"));
    assert_eq!(uri, "file:///C:/Users/reader/src/main.rs");
    assert_eq!(
        path_of(&uri),
        Some(PathBuf::from(r"C:\Users\reader\src\main.rs"))
    );

    // A drive with nothing after it, and a share, which has no drive letter and keeps the
    // separators it came with.
    assert_eq!(path_of("file:///C:/"), Some(PathBuf::from(r"C:\")));
    assert_eq!(
        path_of("file:///a/b"),
        Some(PathBuf::from("/a/b")),
        "a unix path was respelled"
    );
}

#[test]
fn a_uri_of_something_that_is_not_a_local_file_names_no_path() {
    assert_eq!(path_of("https://example.invalid/x.rs"), None);
    assert_eq!(path_of("file://elsewhere/x.rs"), None);
}

// What an answer says.

#[test]
fn a_definition_answer_is_read_in_each_shape_it_may_come_in() {
    let range =
        json!({ "start": { "line": 11, "character": 4 }, "end": { "line": 11, "character": 9 } });
    let place = Place {
        file: PathBuf::from("/p/src/main.rs"),
        // The protocol's line 11 is the twelfth line, and its column is kept as it came.
        line: 12,
        columns: 4..9,
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
            columns: 0..0,
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
    assert_eq!(found[0].columns, 7..7);
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
    // One thing is declared and nothing else: progress, which is the only way to know the
    // server is still reading the project, and none of what would have it ask this app
    // for configuration or for a file watcher.
    assert_eq!(
        said[0]["params"]["capabilities"],
        json!({ "window": { "workDoneProgress": true } })
    );
    // The options are what this app asks of every server, and what a project's own
    // settings will be laid over: one line, turning off the check it would otherwise run
    // on loading the workspace.
    assert_eq!(said[0]["params"]["initializationOptions"], wanted());
    assert_eq!(wanted(), json!({ "checkOnSave": false }));
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
            talk.definition(Path::new("/p/src/main.rs"), 41, 17)
                .expect("an answer")
        },
    );

    assert_eq!(said[0]["method"], json!("textDocument/definition"));
    assert_eq!(
        said[0]["params"]["textDocument"]["uri"],
        json!("file:///p/src/main.rs")
    );
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
                columns: 5..9,
                kind: 1,
                modifiers: 0
            },
            Token {
                line: 1,
                columns: 8..10,
                kind: 7,
                modifiers: 0b101
            },
            Token {
                line: 3,
                columns: 4..10,
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
    assert_eq!(legend.kind(&method), Some("method"));
    assert!(legend.says(&method, "trait"));
    assert!(!legend.says(&method, "declaration"));
    // A type it never declared, which a server of another version may still send.
    assert_eq!(legend.kind(&Token { kind: 9, ..method }), None);
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
            talk.implementations(Path::new("/p/src/main.rs"), 4, 11)
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
            talk.references(Path::new("/p/src/main.rs"), 41, 17)
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
            let places = talk
                .definition(Path::new("/p/src/main.rs"), 0, 0)
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
            talk.definition(Path::new("/p/src/main.rs"), 0, 0)
                .expect("an answer");
        },
    );

    let notes = notes
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .clone();
    assert_eq!(notes, [Note::Busy(true), Note::Busy(false)]);
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
            let places = talk
                .definition(Path::new("/p/src/main.rs"), 0, 0)
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
                talk.definition(Path::new("/p/src/main.rs"), 0, 0),
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
    let (mut talk, fake, _notes) = Fake::pair();
    // The server's end goes away with nothing said, which is what a killed one looks
    // like from here.
    drop(fake);

    assert!(matches!(
        talk.definition(Path::new("/p/src/main.rs"), 0, 0),
        Err(Failure::Broken(_))
    ));
}

// The process.

#[test]
fn a_server_that_is_not_installed_is_a_failure_and_not_a_panic() {
    let failure = start_program_in("no-such-language-server", Path::new("."), |_| ())
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

    assert!(reason.ends_with("..."), "{reason}");
    assert!(reason.chars().count() <= 203, "{reason}");
}

/// The whole of it against a real program: one that exits at once is a server that would
/// not start, and not a conversation that broke.
#[test]
#[cfg(unix)]
fn a_handshake_with_a_program_that_exits_at_once_says_it_would_not_start() {
    let (mut server, handle) = start_program_in("true", Path::new("."), |_| ()).expect("spawned");
    let failure = server
        .initialize(Path::new("."), &wanted())
        .err()
        .expect("no server");
    handle.stop();

    assert!(
        matches!(failure, Failure::NoServer(_)),
        "a program that ended was reported as {failure}"
    );
}

// ---------------------------------------------------------------------------------------
// A project's own settings.

/// Read a settings file over `/p`, which is the directory `${workspaceFolder}` stands for
/// in every test below.
fn read(text: &str) -> Result<Settings, Unreadable> {
    settings_from(text, Path::new("/p"))
}

/// The two things a server is silent about getting wrong: the prefix has to come off and
/// the dots have to become a tree. What is not the server's is skipped without a word.
#[test]
fn a_name_loses_its_prefix_and_its_dots_become_a_tree() {
    let settings = read(
        r#"{
            "rust-analyzer.cargo.features": ["a"],
            "rust-analyzer.checkOnSave": true,
            "git.detectSubmodulesLimit": 20,
            "files.associations": { "*.rs": "rust" }
        }"#,
    )
    .expect("a file that reads");

    assert_eq!(
        settings.options(),
        &json!({ "checkOnSave": true, "cargo": { "features": ["a"] } })
    );
    assert_eq!(
        settings.overrides,
        vec![
            ("cargo.features".to_owned(), "[\"a\"]".to_owned()),
            ("checkOnSave".to_owned(), "true".to_owned()),
        ]
    );
}

/// A name given a value and made a table by a longer name is a file saying two things, and
/// which was meant is not this app's to pick -- whichever order they are written in.
#[test]
fn a_name_that_is_both_a_value_and_a_table_is_a_failure() {
    for text in [
        r#"{ "rust-analyzer.cargo": { "noDeps": true }, "rust-analyzer.cargo.features": [] }"#,
        r#"{ "rust-analyzer.cargo.features": [], "rust-analyzer.cargo": { "noDeps": true } }"#,
    ] {
        assert_eq!(read(text), Err(Unreadable::Both("cargo".to_owned())));
    }

    // Two names under one table are not that: they are the table.
    let settings =
        read(r#"{ "rust-analyzer.cargo.features": [], "rust-analyzer.cargo.noDeps": true }"#)
            .expect("a file that reads");
    assert_eq!(
        settings.options()["cargo"],
        json!({ "features": [], "noDeps": true })
    );
}

/// The one variable, wherever it is written: in a string, inside a table, and inside an
/// array. Nothing but strings is touched.
#[test]
fn the_workspace_folder_is_resolved_wherever_it_is_written() {
    let settings = read(
        r#"{
            "rust-analyzer.rustc.source": "${workspaceFolder}/Cargo.toml",
            "rust-analyzer.server.extraEnv": { "RUSTC": "${workspaceFolder}/build/rustc" },
            "rust-analyzer.linkedProjects": ["${workspaceFolder}/library/Cargo.toml", 7]
        }"#,
    )
    .expect("a file that reads");

    assert_eq!(
        settings.options(),
        &json!({
            "checkOnSave": false,
            "rustc": { "source": "/p/Cargo.toml" },
            "server": { "extraEnv": { "RUSTC": "/p/build/rustc" } },
            "linkedProjects": ["/p/library/Cargo.toml", 7],
        })
    );
}

/// Every other variable is a failure. VS Code leaves one it does not know as it was
/// written; here that is a path reaching the server that silently is not there.
#[test]
fn a_variable_this_cannot_resolve_is_a_failure() {
    for (text, named) in [
        (
            r#"{ "rust-analyzer.cargo.sysrootSrc": "${userHome}/rust" }"#,
            "userHome",
        ),
        (
            r#"{ "rust-analyzer.cargo.extraEnv": { "A": "${env:PATH}" } }"#,
            "env:PATH",
        ),
        (r#"{ "rust-analyzer.x": ["${execPath}"] }"#, "execPath"),
        (
            r#"{ "rust-analyzer.x": "${workspaceFolderBasename}" }"#,
            "workspaceFolderBasename",
        ),
    ] {
        assert_eq!(read(text), Err(Unreadable::Variable(named.to_owned())));
    }

    // A `${` that is never closed is not a variable and is left as it was written.
    let settings = read(r#"{ "rust-analyzer.x": "${workspaceFolder" }"#).expect("a file");
    assert_eq!(settings.options()["x"], json!("${workspaceFolder"));
}

/// A file that is not JSON, and one that is JSON but not an object.
#[test]
fn a_file_that_is_not_an_object_of_json_is_a_failure() {
    let Err(Unreadable::NotJson(_)) = read("{ \"rust-analyzer.x\": }") else {
        panic!("a file that is not JSON was read as some");
    };
    assert_eq!(read("[1, 2]"), Err(Unreadable::NotAnObject));
    assert_eq!(read("\"hello\""), Err(Unreadable::NotAnObject));
}

/// The file is read as **JSONC**, which is what VS Code reads it as and what the files in
/// the wild are written in: comments and a trailing comma are not failures.
#[test]
fn the_comments_and_trailing_commas_an_editor_allows_are_taken() {
    let settings = read(
        r#"{
            // which manifests are this tree's
            "rust-analyzer.linkedProjects": ["Cargo.toml",],
            /* and the compiler
               it is read with */
            "rust-analyzer.server.extraEnv": { "RUSTC": "stage0/rustc", },
        }"#,
    )
    .expect("a file an editor would take");

    assert_eq!(settings.options()["linkedProjects"], json!(["Cargo.toml"]));
    assert_eq!(
        settings.options()["server"]["extraEnv"]["RUSTC"],
        json!("stage0/rustc")
    );
}

/// Nothing inside a string is stripped. A `//` is half of every URL, and a string that
/// ends in an escaped quote or holds a backslash before its closing one must not swallow
/// what comes after it -- a path cut short without a word is the failure this is against.
#[test]
fn nothing_inside_a_string_is_taken_for_a_comment() {
    let settings = read(
        r#"{
            "rust-analyzer.a": "https://example.invalid/x",
            "rust-analyzer.b": "a \" // b",
            "rust-analyzer.c": "c:\\",
            "rust-analyzer.d": "/* not a comment */"
        }"#,
    )
    .expect("a file whose strings hold comment marks");

    assert_eq!(settings.options()["a"], json!("https://example.invalid/x"));
    assert_eq!(settings.options()["b"], json!("a \" // b"));
    assert_eq!(settings.options()["c"], json!("c:\\"));
    assert_eq!(settings.options()["d"], json!("/* not a comment */"));
}

/// The shape of the file the user's own tree keeps: a block of `//` lines and then the
/// object, with the editor's own keys in it beside the server's.
#[test]
fn a_header_of_comments_over_the_object_is_read() {
    let settings = read(
        r#"// This config uses a separate build directory for rust-analyzer,
// so that r-a's checks don't block user `x` commands and vice-verse.
//
// ```
// x fmt --check
// ```
{
    "git.detectSubmodulesLimit": 20,
    "rust-analyzer.linkedProjects": ["Cargo.toml"]
}
"#,
    )
    .expect("the shape of a real one");

    assert_eq!(settings.overrides.len(), 1);
    assert_eq!(settings.options()["linkedProjects"], json!(["Cargo.toml"]));
}

/// What a comment is blanked with keeps the lines under it where they were, so what
/// `serde_json` says about a real mistake is about the file the reader wrote.
#[test]
fn a_comment_leaves_the_lines_under_it_where_they_were() {
    let text = "/* one\n   two */\n{\n  \"a\" \"b\"\n}";
    let Err(Unreadable::NotJson(said)) = read(text) else {
        panic!("a file that is not JSON was read as some");
    };

    assert!(said.contains("line 4"), "{said}");
}

/// The merge is per leaf and not per name: what the project says about `cargo.features`
/// leaves what this app said about the rest of `cargo` where it was.
#[test]
fn a_projects_settings_are_laid_over_this_apps_leaf_by_leaf() {
    let over = json!({ "cargo": { "features": ["a"] } });
    let base = json!({ "checkOnSave": false, "cargo": { "noDeps": true, "features": [] } });

    assert_eq!(
        merged(base, over),
        json!({
            "checkOnSave": false,
            "cargo": { "noDeps": true, "features": ["a"] },
        })
    );

    // And what this app asks of every server is under everything a project did not say.
    let settings = read(r#"{ "rust-analyzer.cargo.features": ["a"] }"#).expect("a file");
    assert_eq!(settings.options()["checkOnSave"], json!(false));
}

/// A name spelled in more parts than the tree is walked with is refused rather than
/// recursed over: how deep that goes is not a thing a file gets to say.
#[test]
fn a_name_of_too_many_parts_is_refused() {
    let name = "a.".repeat(DEEPEST + 1);
    let text = format!(r#"{{ "rust-analyzer.{name}b": 1 }}"#);
    let Err(Unreadable::Deep(_)) = read(&text) else {
        panic!("a name of {} parts was taken", DEEPEST + 2);
    };
}

/// `rust-lang/rust`'s own file, which is the tree the whole thing is for: a server told
/// none of this cannot read it.
#[test]
fn the_settings_a_tree_cannot_be_read_without() {
    let settings = read(
        r#"{
            "rust-analyzer.linkedProjects": [
                "Cargo.toml",
                "src/tools/x/Cargo.toml",
                "src/bootstrap/Cargo.toml"
            ],
            "rust-analyzer.rustc.source": "./Cargo.toml",
            "rust-analyzer.cargo.sysrootSrc": "./library",
            "rust-analyzer.cargo.extraEnv": { "RUSTC_BOOTSTRAP": "1" },
            "rust-analyzer.server.extraEnv": {
                "RUSTC": "${workspaceFolder}/build/host/stage0/bin/rustc",
                "CARGO": "${workspaceFolder}/build/host/stage0/bin/cargo"
            },
            "rust-analyzer.procMacro.server": "${workspaceFolder}/build/host/stage0/libexec/rust-analyzer-proc-macro-srv",
            "rust-analyzer.cargo.buildScripts.overrideCommand": ["python3", "x.py", "check"],
            "rust-analyzer.cargo.buildScripts.invocationStrategy": "once",
            "rust-analyzer.check.invocationStrategy": "once",
            "rust-analyzer.rustfmt.overrideCommand": ["./build/host/rustfmt/bin/rustfmt"],
            "git.detectSubmodulesLimit": 20
        }"#,
    )
    .expect("the tree's own settings");

    let options = settings.options();
    assert_eq!(options["cargo"]["sysrootSrc"], json!("./library"));
    assert_eq!(options["cargo"]["extraEnv"]["RUSTC_BOOTSTRAP"], json!("1"));
    assert_eq!(
        options["cargo"]["buildScripts"]["invocationStrategy"],
        json!("once")
    );
    assert_eq!(
        options["server"]["extraEnv"]["CARGO"],
        json!("/p/build/host/stage0/bin/cargo")
    );
    assert_eq!(
        options["procMacro"]["server"],
        json!("/p/build/host/stage0/libexec/rust-analyzer-proc-macro-srv")
    );
    // The app's own is still under it, and the editor's own key is not there at all.
    assert_eq!(options["checkOnSave"], json!(false));
    assert_eq!(options.get("git"), None);
    // Every setting the server was given is one the reader can be shown.
    assert_eq!(settings.overrides.len(), 10);
}

/// No file at all is no overrides: it is what nearly every project has, and a viewer that
/// called it a failure would call every project one.
#[test]
fn a_project_with_no_settings_file_says_nothing() {
    let settings = settings_in(Path::new("/no/such/directory")).expect("no file is no failure");

    assert_eq!(settings, Settings::none());
    assert_eq!(settings.options(), &wanted());
    assert!(settings.overrides.is_empty());
}

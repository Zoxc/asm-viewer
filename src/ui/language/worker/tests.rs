//! What the worker answers with: the shape a question's answer comes back in.

use super::*;

/// Each of the four questions comes back in the shape its consumer takes: the places a
/// followed answer names, and the text of every line a listed one names, read on the
/// worker. A followed answer handed to the panel, or a listed one to `ui::follow`, would
/// leave a real mismatch looking like nothing found.
#[test]
fn an_answer_comes_back_in_the_shape_the_question_was_asked_in() {
    let place = lsp::Place {
        file: PathBuf::from("/p/src/main.rs"),
        line: 3,
        columns: 9..15,
    };
    // The answer's own reader, as the worker hands one over.
    let lines = || {
        lsp::Lines::reading(|_| {
            Some("fn main() {\n    let n = 1;\n    ø = helper(n);\n}\n".to_owned())
        })
    };

    for want in [lsp::Followed::Definition, lsp::Followed::Declaration] {
        let reply = replied(
            lsp::Question::Followed(want),
            Ok(vec![place.clone()]),
            &mut lines(),
        );
        let Reply::Followed(Ok(places)) = reply else {
            panic!("a followed question is answered with places");
        };
        assert_eq!(
            places,
            std::slice::from_ref(&place),
            "one place in, one place out"
        );
    }

    for want in [lsp::Listed::Implementations, lsp::Listed::References] {
        let reply = replied(
            lsp::Question::Listed(want),
            Ok(vec![place.clone()]),
            &mut lines(),
        );
        let Reply::Listed(Ok(found)) = reply else {
            panic!("a listed question is answered with a list");
        };
        // Grouped under the file, with the text of the line each is on: the read happens
        // with the ask, on the thread that may block.
        assert_eq!((found.count(), found.files()), (1, 1));
        let rows = found.rows(&crate::filter::Matcher::Everything);
        let texts: Vec<&str> = (0..rows.len())
            .filter_map(|at| match &rows[at] {
                crate::grouped::Row::Item { item, .. } => Some(item.text.as_str()),
                crate::grouped::Row::File { .. } => None,
            })
            .collect();
        assert_eq!(texts, ["ø = helper(n);"]);
    }
}

/// **A question with no server to ask is still answered**, failed: one queued behind a start
/// that failed, or behind a question that saw the conversation end, is held by whoever
/// asked it until an answer comes, and one that never came left the Locations panel
/// looking for ever.
#[test]
fn a_question_with_no_server_to_ask_is_answered_failed() {
    let work = language_work();
    let ticket = Ticket { run: 1, id: 7 };
    let at = Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line: 1,
        column: 0,
    };

    let answer = work(LspJob::Ask {
        ticket,
        at: at.clone(),
        want: lsp::Question::Listed(lsp::Listed::References),
    });
    let Some(LspAnswer::Answered {
        ticket: answered,
        reply: Reply::Listed(Err(lsp::Failure::Broken(_))),
    }) = answer
    else {
        panic!("a question with no server is answered as a broken conversation");
    };
    assert_eq!(answered, ticket);

    let answer = work(LspJob::Hover { ticket, at });
    let Some(LspAnswer::Hovered {
        ticket: answered,
        said: Err(lsp::Failure::Broken(_)),
    }) = answer
    else {
        panic!("a hover with no server is answered as a broken conversation");
    };
    assert_eq!(answered, ticket);
}

/// **Telling no server about a file is answered failed**, as a question is: the pipe
/// closing as a file is opened or closed drops the server, and this answer is the only
/// thing that tells the control.
#[test]
fn a_file_opened_or_closed_with_no_server_is_answered_failed() {
    let work = language_work();
    let file: Arc<Path> = Arc::from(Path::new("/p/src/main.rs"));

    let answer = work(LspJob::Opened {
        run: 3,
        file: file.clone(),
        language: "rust".to_owned(),
    });
    let Some(LspAnswer::Untold {
        run: 3,
        why: lsp::Failure::Broken(_),
    }) = answer
    else {
        panic!("an open with no server is not answered as a broken conversation");
    };

    let answer = work(LspJob::Closed { run: 3, file });
    let Some(LspAnswer::Untold {
        run: 3,
        why: lsp::Failure::Broken(_),
    }) = answer
    else {
        panic!("a close with no server is not answered as a broken conversation");
    };
}

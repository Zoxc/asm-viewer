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
        assert_eq!(places, [place.clone()], "one place in, one place out");
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

//! One line saying how something went, and whether that is bad news.
//!
//! Every pane that reports a build, a run or a language server says it this way: a
//! sentence, and the one bit a drawing site decodes into a colour (`verdict_line`,
//! `src/ui/parts.rs`). The bit and not the colour, so nothing outside the UI names one.

/// A line saying how something went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    pub text: String,
    /// Whether the line is bad news: a refusal, an error, a failure.
    pub bad: bool,
}

impl Verdict {
    /// A line that is not bad news.
    pub fn plain(text: impl Into<String>) -> Verdict {
        Verdict {
            text: text.into(),
            bad: false,
        }
    }

    /// And one that is.
    pub fn bad_news(text: impl Into<String>) -> Verdict {
        Verdict {
            text: text.into(),
            bad: true,
        }
    }
}

/// `count` with the word for it: "1 warning", "3 warnings". Both words are given, since
/// English does not derive the second from the first.
pub fn counted(count: usize, one: &str, many: &str) -> String {
    match count {
        1 => format!("1 {one}"),
        count => format!("{count} {many}"),
    }
}

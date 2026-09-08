//! Rust's functions by a scanner of its own, and not by the tree-sitter grammar.
//!
//! The grammar is behind the compiler and stays behind it: `tree-sitter-rust` 0.24 does
//! not know `const impl`, `const trait` or `[const]`, and its error recovery does not
//! contain the damage -- one such item and the whole file is an `ERROR` node with a
//! handful of functions inside it. Measured on this machine's nightly `library/core`: 98
//! of 289 files fail to parse, and about 1 200 function definitions go missing, half of
//! them still missing with those keywords blanked out before the parse. The source the
//! reader is most likely to be looking at is exactly that library.
//!
//! What a function *is* for this purpose needs almost none of the grammar: the `fn`
//! keyword, the name after it, and the block that follows the signature, from the line
//! of the keyword to the line of the closing brace. A signature ending in `;` is not
//! one. That much is decided by tokens -- comments, strings, character literals and
//! brackets -- and is what this scans for. Where it differs from the grammar it is on
//! purpose: a `fn` inside a `macro_rules!` body is token soup to the grammar and code to
//! the compiler, whose line info points into it, so it is a function here.
//!
//! Never panics on any text: every index is a byte offset into the text it was found in,
//! and the walk is one pass with a stack.

use super::Function;

/// Every function `text` defines, outer before inner, as [`super::functions`] answers.
pub fn functions(text: &str) -> Vec<Function> {
    let mut found = Vec::new();
    let mut scanner = Scanner::new(text);
    let mut open: Vec<Open> = Vec::new();

    while let Some(token) = scanner.next() {
        match token {
            Token::Fn => {
                let Some(name) = scanner.identifier() else {
                    continue;
                };
                let Some(line) = scanner.line_of(scanner.position) else {
                    continue;
                };
                found.push(Function {
                    name,
                    lines: line..=line,
                });
                open.push(Open {
                    found: found.len() - 1,
                    grouped_at: scanner.grouped,
                    body_depth: None,
                });
            }
            Token::Open(Bracket::Brace) => {
                if let Some(last) = open.last_mut() {
                    last.begins_body(&scanner);
                }
            }
            // A `(` or a `[` opens grouping and nothing else, and the scanner has
            // counted it.
            Token::Open(Bracket::Paren | Bracket::Square) => {}
            Token::Close => {
                // The stranded go first, so none of them stands between a closing
                // brace and the body it ends. From the top down, which is descending
                // index order, so the entries under them stay valid.
                while let Some(last) = open.pop_if(|last| last.stranded(&scanner)) {
                    last.forget(&mut found);
                }
                if let Some(last) = open.pop_if(|last| last.ends_at(&scanner)) {
                    last.ends_on(&mut found, scanner.line_of(scanner.position));
                }
            }
            Token::Semicolon => {
                // A declaration comes back off `found` where it is the last one there,
                // so that whatever was found inside the signature keeps its place.
                if let Some(last) = open.pop_if(|last| last.declared(&scanner)) {
                    if last.found + 1 == found.len() {
                        found.pop();
                    }
                }
            }
        }
    }

    // What the text ended in the middle of, finished from the top down: an entry names
    // a higher index in `found` than the ones under it, so a removal leaves theirs.
    while let Some(last) = open.pop() {
        match last.body_depth {
            // A body still open at the end of the text is unterminated; it reaches the
            // last line there is -- the one the final byte is on, which a trailing
            // newline is part of.
            Some(_) => {
                let last_line = scanner.line_of(text.len().saturating_sub(1));
                last.ends_on(&mut found, last_line);
            }
            // A signature that never began one is no function and goes, but only it.
            None => last.forget(&mut found),
        }
    }
    found
}

/// A `fn` whose signature has been seen and whose body has not closed.
struct Open {
    /// Where in `found` the function it names sits.
    found: usize,
    /// How many `(`, `[` or `<` were open at the `fn` keyword.
    grouped_at: usize,
    /// How deep the body's own brace put the scan, once the body has begun.
    body_depth: Option<usize>,
}

impl Open {
    /// The first brace after a signature at the grouping depth the `fn` was seen at is
    /// the body's: the signature's own `{` never comes inside a `(`, `[` or `<` of its
    /// own, a `{ N }` in a const generic argument does, and the depth is relative
    /// because a whole item can sit inside a macro invocation's parentheses
    /// (`const_eval_select!( ... )`).
    fn begins_body(&mut self, scanner: &Scanner) {
        if self.body_depth.is_none() && scanner.grouped == self.grouped_at {
            self.body_depth = Some(scanner.depth);
        }
    }

    /// Whether the bracket just closed was the body's: the close left the scan one
    /// bracket further out than the body's brace put it. A signature with no body yet
    /// ends on nothing.
    fn ends_at(&self, scanner: &Scanner) -> bool {
        self.body_depth == scanner.depth.checked_add(1)
    }

    /// A signature whose grouping has closed under it never had a body and never will:
    /// `fn` inside a macro invocation's parentheses.
    fn stranded(&self, scanner: &Scanner) -> bool {
        self.body_depth.is_none() && scanner.grouped < self.grouped_at
    }

    /// A signature that ends before its body began is a declaration -- a trait's, or an
    /// `extern` block's -- and has no lines of code. At the `fn`'s own grouping depth,
    /// since the `;` of `[u8; 4]` is a type's and not an end.
    fn declared(&self, scanner: &Scanner) -> bool {
        self.body_depth.is_none() && scanner.grouped == self.grouped_at
    }

    /// The function this names ends on `line`, where the text has one.
    fn ends_on(self, found: &mut [Function], line: Option<u32>) {
        if let (Some(function), Some(line)) = (found.get_mut(self.found), line) {
            function.lines = *function.lines.start()..=line;
        }
    }

    /// Take the function this names back out of `found`: it was no function.
    fn forget(self, found: &mut Vec<Function>) {
        if self.found < found.len() {
            found.remove(self.found);
        }
    }
}

/// The tokens the scan cares about; everything else is skipped over. An open says which
/// bracket it is, a body being written in a brace and not in the other two; a close says
/// nothing, what it does to the counting being the scanner's own.
enum Token {
    Fn,
    Open(Bracket),
    Close,
    Semicolon,
}

/// A bracket the scan counts: the two a type or an expression is grouped by, and the one
/// a body is written in.
#[derive(Clone, Copy)]
enum Bracket {
    Paren,
    Square,
    Brace,
}

impl Bracket {
    /// Which bracket `byte` is, whichever end of one it is.
    fn of(byte: u8) -> Bracket {
        match byte {
            b'(' | b')' => Bracket::Paren,
            b'[' | b']' => Bracket::Square,
            _ => Bracket::Brace,
        }
    }

    /// Whether it is grouping: the `(` and `[` a `fn`'s own body brace is never
    /// inside.
    fn groups(self) -> bool {
        matches!(self, Bracket::Paren | Bracket::Square)
    }
}

struct Scanner<'a> {
    text: &'a [u8],
    /// The byte after the token last handed out.
    position: usize,
    /// How many braces and brackets are open at the position: [`next`](Scanner::next)
    /// counts each as it hands it out, so the bracket a `Close` just named has come off.
    depth: usize,
    /// How many `(`, `[` or `<` are open: the grouping a `fn`'s own body brace is never
    /// inside. Counted by [`next`](Scanner::next) too, angle brackets as it passes them.
    grouped: usize,
    /// Where each line starts, for turning an offset into a 1-based line.
    lines: Vec<usize>,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Scanner<'a> {
        let lines = std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter(|(_, byte)| *byte == b'\n')
                    .map(|(at, _)| at + 1),
            )
            .collect();
        Scanner {
            text: text.as_bytes(),
            position: 0,
            depth: 0,
            grouped: 0,
            lines,
        }
    }

    /// The 1-based line `offset` is on, or `None` past a `u32`.
    fn line_of(&self, offset: usize) -> Option<u32> {
        let index = self.lines.partition_point(|start| *start <= offset);
        u32::try_from(index).ok()
    }

    /// The next token the scan cares about, skipping comments, strings, character
    /// literals, lifetimes and everything else.
    fn next(&mut self) -> Option<Token> {
        while self.position < self.text.len() {
            let byte = self.text[self.position];
            let next = self.text.get(self.position + 1).copied();
            match byte {
                b'/' if next == Some(b'/') => self.skip_line(),
                b'/' if next == Some(b'*') => self.skip_block_comment(),
                b'"' => self.skip_string(),
                b'b' if next == Some(b'"') => {
                    self.position += 1;
                    self.skip_string();
                }
                b'r' | b'b' if self.raw_string_hashes().is_some() => self.skip_raw_string(),
                b'\'' => self.skip_char_or_lifetime(),
                b'(' | b'[' | b'{' => {
                    let bracket = Bracket::of(byte);
                    self.position += 1;
                    self.depth += 1;
                    if bracket.groups() {
                        self.grouped += 1;
                    }
                    return Some(Token::Open(bracket));
                }
                b')' | b']' | b'}' => {
                    let bracket = Bracket::of(byte);
                    self.position += 1;
                    self.depth = self.depth.saturating_sub(1);
                    if bracket.groups() {
                        self.grouped = self.grouped.saturating_sub(1);
                    }
                    return Some(Token::Close);
                }
                b';' => {
                    self.position += 1;
                    return Some(Token::Semicolon);
                }
                // A type's angle brackets group like a parenthesis, so that the `{` of a
                // `{ N }` const argument in one is not read as a body. `>` after `-` is
                // the arrow of a return type; `>>` is two closes and arrives as two
                // bytes. A comparison miscounts, which is why the close saturates: those
                // only occur in a body, whose function already has its brace, and any
                // `fn` inside one is measured from the count as it stood at its keyword.
                b'<' => {
                    self.grouped += 1;
                    self.position += 1;
                }
                b'>' => {
                    if self.position == 0 || self.text[self.position - 1] != b'-' {
                        self.grouped = self.grouped.saturating_sub(1);
                    }
                    self.position += 1;
                }
                _ if is_identifier_start(byte) => {
                    let word = self.word();
                    if word == b"fn" {
                        return Some(Token::Fn);
                    }
                }
                _ => self.position += 1,
            }
        }
        None
    }

    /// The identifier after the `fn` just handed out, if what follows is one.
    fn identifier(&mut self) -> Option<String> {
        while self
            .text
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.position += 1;
        }
        // `r#try`: the raw prefix is not part of the name.
        if self.text[self.position..].starts_with(b"r#") {
            self.position += 2;
        }
        if !self
            .text
            .get(self.position)
            .is_some_and(|&byte| is_identifier_start(byte))
        {
            return None;
        }
        let word = self.word();
        std::str::from_utf8(word).ok().map(str::to_owned)
    }

    /// The identifier or keyword at the position, which is left after it.
    fn word(&mut self) -> &'a [u8] {
        let start = self.position;
        while self
            .text
            .get(self.position)
            .is_some_and(|&byte| is_identifier_start(byte) || byte.is_ascii_digit())
        {
            self.position += 1;
        }
        &self.text[start..self.position]
    }

    fn skip_line(&mut self) {
        while self.position < self.text.len() && self.text[self.position] != b'\n' {
            self.position += 1;
        }
    }

    /// Block comments nest in Rust.
    fn skip_block_comment(&mut self) {
        let mut depth = 0usize;
        while self.position < self.text.len() {
            if self.text[self.position..].starts_with(b"/*") {
                depth += 1;
                self.position += 2;
            } else if self.text[self.position..].starts_with(b"*/") {
                depth = depth.saturating_sub(1);
                self.position += 2;
                if depth == 0 {
                    return;
                }
            } else {
                self.position += 1;
            }
        }
    }

    /// A string from its opening quote to its closing one, escapes skipped.
    fn skip_string(&mut self) {
        self.position += 1;
        while self.position < self.text.len() {
            match self.text[self.position] {
                b'\\' => self.position += 2,
                b'"' => {
                    self.position += 1;
                    return;
                }
                _ => self.position += 1,
            }
        }
    }

    /// How many `#` a raw string at the position opens with -- `r"`, `r#"`, `br##"` --
    /// or `None` when there is no raw string here.
    fn raw_string_hashes(&self) -> Option<usize> {
        let mut at = self.position + 1;
        if self.text.get(self.position) == Some(&b'b') {
            if self.text.get(at) != Some(&b'r') {
                return None;
            }
            at += 1;
        }
        let mut hashes = 0;
        while self.text.get(at) == Some(&b'#') {
            hashes += 1;
            at += 1;
        }
        (self.text.get(at) == Some(&b'"')).then_some(hashes)
    }

    fn skip_raw_string(&mut self) {
        let Some(hashes) = self.raw_string_hashes() else {
            self.position += 1;
            return;
        };
        while self.text.get(self.position) != Some(&b'"') {
            self.position += 1;
        }
        self.position += 1;
        while self.position < self.text.len() {
            if self.text[self.position] == b'"'
                && self.text[self.position + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|&&byte| byte == b'#')
                    .count()
                    == hashes
            {
                self.position += 1 + hashes;
                return;
            }
            self.position += 1;
        }
    }

    /// `'{'` is a character and `'a` a lifetime; only the first has a closing quote to
    /// skip to, and a brace inside it is not a brace.
    fn skip_char_or_lifetime(&mut self) {
        let rest = &self.text[self.position + 1..];
        let closes_at = match rest.first() {
            // An escape: `'\n'`, `'\u{1F600}'`, `'\''`. The escaped byte is skipped with
            // the backslash, since in `'\''` it is the quote itself and closes nothing.
            Some(b'\\') => rest
                .iter()
                .skip(2)
                .position(|&byte| byte == b'\'')
                .map(|at| at + 2),
            // One character -- of however many bytes -- then the closing quote.
            Some(&first) => {
                let width = utf8_width(first);
                (rest.get(width) == Some(&b'\'')).then_some(width)
            }
            None => None,
        };
        self.position += match closes_at {
            Some(at) => at + 2,
            // A lifetime or a label: the quote alone.
            None => 1,
        };
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic() || byte >= 0x80
}

/// How many bytes the UTF-8 character starting with `first` takes.
fn utf8_width(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

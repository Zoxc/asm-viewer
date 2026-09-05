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
    // Functions whose signature has been seen and whose body has not closed: the index
    // into `found`, how many parentheses and brackets were open at the `fn` keyword, and
    // the depth at which the body will close once it has begun.
    let mut open: Vec<(usize, usize, Option<usize>)> = Vec::new();

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
                open.push((found.len() - 1, scanner.grouped, None));
            }
            Token::Open(bracket) => {
                scanner.depth += 1;
                // The first brace after a signature at the grouping depth the `fn` was
                // seen at is the body's: the signature's own `{` never comes inside a
                // `(`, `[` or `<` of its own, a `{ N }` in a const generic argument
                // does, and the depth is relative because a whole item can sit inside a
                // macro invocation's parentheses (`const_eval_select!( ... )`).
                if bracket == b'{' {
                    if let Some((_, at, body @ None)) = open.last_mut() {
                        if scanner.grouped == *at {
                            *body = Some(scanner.depth);
                        }
                    }
                }
                if bracket != b'{' {
                    scanner.grouped += 1;
                }
            }
            Token::Close(bracket) => {
                if bracket != b'{' {
                    scanner.grouped = scanner.grouped.saturating_sub(1);
                }
                // A signature whose grouping has closed under it never had a body and
                // never will: `fn` inside a macro invocation's parentheses. Dropped
                // here, so it does not hide the enclosing function's brace from the
                // match below. From the top down, which is descending index order, so
                // the indices still on the stack stay valid.
                while let Some(&(index, at, None)) = open.last() {
                    if scanner.grouped >= at {
                        break;
                    }
                    open.pop();
                    if index < found.len() {
                        found.remove(index);
                    }
                }
                if let Some((index, _, Some(depth))) = open.last() {
                    if *depth == scanner.depth {
                        if let (Some(function), Some(line)) =
                            (found.get_mut(*index), scanner.line_of(scanner.position))
                        {
                            function.lines = *function.lines.start()..=line;
                        }
                        open.pop();
                    }
                }
                scanner.depth = scanner.depth.saturating_sub(1);
            }
            // A signature that ends before its body began is a declaration -- a
            // trait's, or an `extern` block's -- and has no lines of code. At the `fn`'s
            // own grouping depth, since the `;` of `[u8; 4]` is a type's and not an end.
            Token::Semicolon => {
                if let Some((index, at, None)) = open.last() {
                    if scanner.grouped == *at {
                        if *index + 1 == found.len() {
                            found.pop();
                        }
                        open.pop();
                    }
                }
            }
        }
    }

    // A body still open at the end of the text is unterminated; it reaches the last
    // line there is -- the one the final byte is on, which a trailing newline is part of.
    // A signature that never began one is no function and goes, but only it: taken from
    // the back, so the entries under it keep their indices.
    for (index, _, body) in open.into_iter().rev() {
        if body.is_some() {
            if let (Some(function), Some(line)) = (
                found.get_mut(index),
                scanner.line_of(text.len().saturating_sub(1)),
            ) {
                function.lines = *function.lines.start()..=line;
            }
        } else if index < found.len() {
            found.remove(index);
        }
    }
    found
}

/// The tokens the scan cares about; everything else is skipped over.
enum Token {
    Fn,
    Open(u8),
    Close(u8),
    Semicolon,
}

struct Scanner<'a> {
    text: &'a [u8],
    /// The byte after the token last handed out.
    position: usize,
    /// How many braces and brackets are open.
    depth: usize,
    /// How many `(`, `[` or `<` are open: the grouping a `fn`'s own body brace is never
    /// inside. Angle brackets are counted by [`next`](Scanner::next) as it passes them,
    /// the rest by the walk over the tokens it hands out.
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
                    self.position += 1;
                    return Some(Token::Open(byte));
                }
                b')' | b']' | b'}' => {
                    self.position += 1;
                    return Some(Token::Close(if byte == b'}' { b'{' } else { byte }));
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

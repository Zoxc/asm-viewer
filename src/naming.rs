//! A demangled symbol name cut down to the `module::fn_name` a tab is called by.
//!
//! The whole name is what the sidebar lists and what a tooltip says; a tab has room for
//! about forty characters, and `<alloc::vec::Vec<T, A> as core::iter::traits::collect::
//! IntoIterator>::into_iter` spends all of them before reaching the function. Cutting at
//! forty characters keeps the *front*, which is the half that says the least.
//!
//! Reading a name is real parsing rather than a `split("::").last()`: generic arguments
//! nest and carry `::` of their own, C++ writes `operator<<` with an angle bracket that
//! opens nothing and `fn(..) -> *mut T` with one that closes nothing, and both languages
//! put a parenthesised argument list, a `const` qualifier or a return type around the
//! name. So it is a scanner, framework-free and unit-tested against names out of the
//! app's own binary, and it lives here rather than in `analysis` because the crate has no
//! use for it: it hands out the name the file states and the name the demangler made of
//! it, and *how much of one to draw* is a question only a view has.
//!
//! Byte scanning is safe on a name that is not ASCII: every byte compared here is an
//! ASCII one, which a UTF-8 continuation byte can never equal, so no index this cuts on
//! is ever inside a character.

/// The `module::fn_name` of a demangled name: its last two path segments, with generic
/// arguments, argument lists, `impl` qualifiers and rustc's legacy hash suffix gone.
///
/// A trailing `{{closure}}` or `{closure#0}` is kept as a third component -- it is the
/// difference between a function and the closure inside it, and a tab that dropped it
/// would name a symbol that is not the one it shows. Only the innermost is kept.
///
/// A name the app made up rather than read from a file comes back whole. Those are one
/// angle-bracket group and nothing else -- `<entry point>`, `<function 0x140001000>` --
/// so there is no path in one and no function name at its end. The shape is the whole
/// test: a real name opening with `<` is a `<Type as Trait>` qualifier, whose `::` puts
/// the group's end before the name's.
///
/// Never empty: a name nothing can be made of comes back as it went in.
pub fn short_name(name: &str) -> String {
    let name = name.trim();
    if is_made_up(name) {
        return name.to_owned();
    }

    let mut segments = split_path(name);

    // rustc's legacy mangling ends the path with the symbol's own hash. `{:#}` on
    // `rustc_demangle` drops it, which is what `analysis` asks for, so this is for the
    // names that reach a tab by some other route.
    if segments.len() > 1 && segments.last().is_some_and(|last| is_legacy_hash(last)) {
        segments.pop();
    }

    // The annotation is cleared by any ordinary segment after it: items *inside* a
    // closure exist (`foo::{closure#0}::inner`), and there the closure is not what the
    // symbol is.
    let mut path: Vec<&str> = Vec::new();
    let mut annotation: Option<&str> = None;
    for (index, segment) in segments.into_iter().enumerate() {
        match reduce(segment, index == 0) {
            Some(Part::Name(name)) => {
                path.push(name);
                annotation = None;
            }
            Some(Part::Annotation(text)) => annotation = Some(text),
            None => {}
        }
    }

    let mut short = path[path.len().saturating_sub(2)..].join("::");
    if let Some(annotation) = annotation {
        if !short.is_empty() {
            short.push_str("::");
        }
        short.push_str(annotation);
    }

    if short.is_empty() {
        name.to_owned()
    } else {
        short
    }
}

/// Whether a name is one of the app's own: a single angle-bracket group, closed, with
/// nothing on either side of it. See [`short_name`].
fn is_made_up(name: &str) -> bool {
    name.starts_with('<') && name.ends_with('>') && skip_group(name, 0) == name.len()
}

/// What one path segment turns out to say.
enum Part<'a> {
    /// A name to draw: a module, a type, or the function itself.
    Name(&'a str),
    /// One of the compiler's own `{...}` markers -- a closure, a shim, a drop glue.
    Annotation(&'a str),
}

/// The segments of a path, split on the `::` that are not inside a group. Each comes back
/// whole, generic arguments and all; [`reduce`] is what makes a name of one.
fn split_path(name: &str) -> Vec<&str> {
    let bytes = name.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        if let Some(end) = operator_token(name, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'<' | b'(' | b'[' => at = skip_group(name, at),
            b':' if bytes.get(at + 1) == Some(&b':') => {
                segments.push(&name[start..at]);
                at += 2;
                start = at;
            }
            _ => at += 1,
        }
    }
    segments.push(&name[start..]);
    segments
}

/// One segment as the name it contributes, or nothing when it contributes none -- an
/// empty segment, or C++'s `(anonymous namespace)`, which is noise a tab is better off
/// without.
///
/// `first` is whether the segment opens the path, and it is what tells a `<Type as
/// Trait>` qualifier from a turbofish: `drop_glue::<Vec<T>>` names `drop_glue`, and the
/// `<Vec<T>>` after it is an argument rather than the type the function is on.
fn reduce(segment: &str, first: bool) -> Option<Part<'_>> {
    let segment = segment.trim();
    match segment.as_bytes().first()? {
        b'{' => Some(Part::Annotation(segment)),
        b'<' if first => qualifier(segment).map(Part::Name),
        b'<' => None,
        _ => {
            let name = last_word(head(segment));
            (!name.is_empty()).then_some(Part::Name(name))
        }
    }
}

/// What to call a `<Type as Trait>::` or `<Type>::` qualifier: the last segment of the
/// type it is about, so `<alloc::vec::Vec<T, A> as ..::IntoIterator>::into_iter` is
/// `Vec::into_iter`.
///
/// A type with no name of its own -- `<(A, B) as Default>`, `<[T] as Clone>` -- falls
/// back to the trait's, which is the only word left that says anything.
fn qualifier(segment: &str) -> Option<&str> {
    let end = skip_group(segment, 0);
    // One `<` off the front, and the `>` off the end when there is one -- a name whose
    // brackets do not balance has the group running to its end instead.
    let closed = end > 1 && segment.as_bytes()[end - 1] == b'>';
    let inside = &segment[1..if closed { end - 1 } else { end }];

    let (subject, trait_name) = match split_as(inside) {
        Some((subject, trait_name)) => (subject, Some(trait_name)),
        None => (inside, None),
    };
    last_name(subject).or_else(|| trait_name.and_then(last_name))
}

/// The name at the end of a type: `&mut foo::bar::Baz<T>` is `Baz`. Not recursive -- a
/// type nested in a type is reached by taking the last segment first and stripping what
/// hangs off it after, so depth costs nothing.
fn last_name(text: &str) -> Option<&str> {
    let segments = split_path(text);
    let last = *segments.last()?;
    match reduce(last, segments.len() == 1) {
        Some(Part::Name(name)) => Some(name),
        _ => None,
    }
}

/// The two sides of a `<Type as Trait>`, split on the ` as ` that is not inside a group --
/// `<<A as B>::C as D>` has two and only the second one splits it.
fn split_as(inside: &str) -> Option<(&str, &str)> {
    const AS: &str = " as ";
    let bytes = inside.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if let Some(end) = operator_token(inside, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'<' | b'(' | b'[' => at = skip_group(inside, at),
            _ if inside.as_bytes()[at..].starts_with(AS.as_bytes()) => {
                return Some((&inside[..at], &inside[at + AS.len()..]))
            }
            _ => at += 1,
        }
    }
    None
}

/// A segment up to the first group that hangs off it: the generic arguments, the C++
/// argument list, and with them the ` const` and the `&` that follow one.
fn head(segment: &str) -> &str {
    let bytes = segment.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if let Some(end) = operator_token(segment, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'<' | b'(' | b'[' => return &segment[..at],
            _ => at += 1,
        }
    }
    segment
}

/// The last space-separated word of a segment, which is what drops a C++ return type
/// (`void std` in `void std::sort<..>(..)`) and an MSVC `public: void __cdecl`. An
/// `operator` is one word however many spaces it is written with.
fn last_word(text: &str) -> &str {
    let text = text.trim();
    match text.rfind(char::is_whitespace) {
        Some(space) if !is_operator(text) => text[space..].trim_start(),
        _ => text,
    }
}

/// Whether a name is rustc's legacy `h`-and-sixteen-hex-digits suffix. Sixteen exactly:
/// the point is to leave a function actually called `hffff` alone.
fn is_legacy_hash(segment: &str) -> bool {
    let Some(hex) = segment.trim().strip_prefix('h') else {
        return false;
    };
    hex.len() == 16 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The end of the group opening at `open`, one past its closing bracket -- or the end of
/// the string, for a name whose brackets do not balance. The three bracket kinds share
/// one depth, since nothing here has to tell a well-formed name from a broken one.
fn skip_group(name: &str, open: usize) -> usize {
    let bytes = name.as_bytes();
    let mut depth = 0usize;
    let mut at = open;
    while at < bytes.len() {
        if let Some(end) = operator_token(name, at) {
            at = end;
            continue;
        }
        match bytes[at] {
            b'"' => {
                at = skip_string(name, at);
                continue;
            }
            b'<' | b'(' | b'[' => depth += 1,
            // The `>` of a `->`: `fn(*mut c_void) -> *mut c_void` is a type, and its
            // arrow closes nothing.
            b'>' if at > open && bytes[at - 1] == b'-' => {}
            b'>' | b')' | b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

/// Past a `"..."`, which is where an `extern "C"` in a type puts a run of anything at
/// all. `'` is not a string here: it opens a Rust lifetime far more often than a
/// character.
fn skip_string(name: &str, open: usize) -> usize {
    let bytes = name.as_bytes();
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

/// One past a C++ `operator` and the symbol it names, when one starts at `at`. This is
/// what keeps `operator<<` from opening a group and `operator>` from closing one, so it
/// is asked before every bracket in this file.
fn operator_token(name: &str, at: usize) -> Option<usize> {
    const KEYWORD: &str = "operator";
    /// Longest first: `<<=` must not match as `<<`.
    const SYMBOLS: &[&str] = &[
        "delete[]", "new[]", "co_await", "delete", "new", "<=>", "<<=", ">>=", "->*", "()", "[]",
        "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=",
        "^=", "&=", "|=", "->", "\"\"", "<", ">", "+", "-", "*", "/", "%", "^", "&", "|", "~", "!",
        "=", ",",
    ];

    // On the bytes: `at` walks a name that need not be ASCII, and slicing a `str` at a
    // byte that is inside a character is a panic.
    if !name.as_bytes()[at..].starts_with(KEYWORD.as_bytes()) {
        return None;
    }
    // A word of its own, not the tail of `my_operator`.
    if at > 0 && is_word_byte(name.as_bytes()[at - 1]) {
        return None;
    }

    // Past the keyword is a character boundary, the eight bytes before it being ASCII.
    let rest = &name.as_bytes()[at + KEYWORD.len()..];
    let spaces = rest.iter().take_while(|byte| **byte == b' ').count();
    let symbol = &rest[spaces..];
    let taken = SYMBOLS
        .iter()
        .find(|candidate| symbol.starts_with(candidate.as_bytes()))
        .map(|candidate| spaces + candidate.len())
        // A conversion operator -- `operator Foo` -- names a type rather than a symbol.
        // The keyword alone is taken and the type is read as the rest of the segment.
        .unwrap_or(0);
    Some(at + KEYWORD.len() + taken)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// Whether a name is a C++ `operator`, which is a word however it is spelled and never
/// the return type of what follows it.
fn is_operator(text: &str) -> bool {
    operator_token(text, 0).is_some()
}

#[cfg(test)]
mod tests;

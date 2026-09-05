use super::*;

use tree_sitter::{Language, Parser};

/// `text` parsed with `language`, and the functions read off the parse.
fn spans(language: impl Into<Language>, text: &str) -> Vec<Function> {
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .expect("the grammar is on the parser's ABI");
    let tree = parser.parse(text, None).expect("a parse");
    functions(&tree, text.as_bytes())
}

/// A span as `(name, first, last)`, which is what the assertions read.
fn named(functions: &[Function]) -> Vec<(&str, u32, u32)> {
    functions
        .iter()
        .map(|function| {
            (
                function.name.as_str(),
                *function.lines.start(),
                *function.lines.end(),
            )
        })
        .collect()
}

const RUST: &str = "\
use std::fmt;

/// A doc comment is outside.
#[inline]
pub fn first<T: fmt::Debug>(x: T) -> String
where
    T: Clone,
{
    let f = |y: &T| format!(\"{y:?}\");
    fn inner(s: String) -> String {
        s
    }
    inner(f(&x))
}

impl Thing {
    fn new() -> Thing {
        Thing
    }
}

trait Named {
    fn name(&self) -> &str;
}
";

#[test]
fn rust_functions_are_listed_outer_first_with_their_lines() {
    let found = rust::functions(RUST);
    assert_eq!(
        named(&found),
        [("first", 5, 14), ("inner", 10, 12), ("new", 17, 19)]
    );
}

#[test]
fn the_innermost_function_around_a_line_wins() {
    let found = rust::functions(RUST);
    // The signature and the closing brace are both inside.
    assert_eq!(enclosing(&found, 5).map(|f| f.name.as_str()), Some("first"));
    assert_eq!(
        enclosing(&found, 14).map(|f| f.name.as_str()),
        Some("first")
    );
    // The closure is not a function; its line is `first`'s.
    assert_eq!(enclosing(&found, 9).map(|f| f.name.as_str()), Some("first"));
    // A nested function is its own on its lines.
    assert_eq!(
        enclosing(&found, 11).map(|f| f.name.as_str()),
        Some("inner")
    );
    assert_eq!(
        enclosing(&found, 13).map(|f| f.name.as_str()),
        Some("first")
    );
    // The attribute, the doc comment, the `impl` line and a bodiless trait method are
    // outside any function.
    assert_eq!(enclosing(&found, 3), None);
    assert_eq!(enclosing(&found, 4), None);
    assert_eq!(enclosing(&found, 16), None);
    assert_eq!(enclosing(&found, 23), None);
    assert_eq!(enclosing(&found, 0), None);
    assert_eq!(enclosing(&found, u32::MAX), None);
}

const C: &str = "\
int add(int a, int b);

int *ptr(void)
{
    return 0;
}

static int (*table(int n))(int)
{
    return 0;
}
";

#[test]
fn c_functions_are_named_through_their_declarators() {
    let found = spans(tree_sitter_c::LANGUAGE, C);
    assert_eq!(named(&found), [("ptr", 3, 6), ("table", 8, 11)]);
}

const CPP: &str = "\
namespace ns {
template <typename T>
T &Thing<T>::get() const {
    auto f = [](int x) { return x; };
    return value;
}
std::ostream &operator<<(std::ostream &out, const Thing<int> &) {
    return out;
}
Thing::~Thing() {}
}
";

#[test]
fn cpp_functions_keep_their_qualified_names() {
    let found = spans(tree_sitter_cpp::LANGUAGE, CPP);
    assert_eq!(
        named(&found),
        [
            ("Thing<T>::get", 3, 6),
            ("operator<<", 7, 9),
            ("Thing::~Thing", 10, 10)
        ]
    );
    // The lambda's line is the method's.
    assert_eq!(
        enclosing(&found, 4).map(|f| f.name.as_str()),
        Some("Thing<T>::get")
    );
}

#[test]
fn a_file_with_no_functions_answers_with_none() {
    let found = rust::functions("const X: u32 = 1;\n");
    assert!(found.is_empty());
    assert_eq!(enclosing(&found, 1), None);
    assert!(spans(tree_sitter_c::LANGUAGE, "").is_empty());
}

const RUST_ITEMS: &str = "\
pub trait Shape<T> {
    fn area(&self) -> T;
    fn describe(&self) -> String {
        format!(\"{}\", self.name())
    }
}

impl<T: Copy + Default> Shape<T> for Square<T> {
    fn area(&self) -> T {
        self.side
    }
}

impl<T> Square<T>
where
    T: Clone,
{
    pub async fn load(&self) -> T {
        self.side.clone()
    }
    pub const unsafe fn raw(&self) -> *const T {
        &self.side
    }
}

mod inner {
    pub(crate) extern \"C\" fn exported<const N: usize>() -> usize {
        N
    }
}

macro_rules! make {
    () => {
        fn made() {}
    };
}
";

#[test]
fn trait_and_impl_methods_are_functions_whatever_their_qualifiers() {
    let found = rust::functions(RUST_ITEMS);
    assert_eq!(
        named(&found),
        [
            ("describe", 3, 5),
            ("area", 9, 11),
            ("load", 18, 20),
            ("raw", 21, 23),
            ("exported", 27, 29),
            // Token soup to the grammar, code to the compiler, whose line info points
            // into the macro's body: a function here.
            ("made", 34, 34),
        ]
    );
    // A trait's bodiless signature, the `impl` header and a `where` clause are outside;
    // a line inside a generic impl's method is that method's.
    assert_eq!(enclosing(&found, 2), None);
    assert_eq!(enclosing(&found, 8), None);
    assert_eq!(enclosing(&found, 15), None);
    assert_eq!(
        enclosing(&found, 4).map(|f| f.name.as_str()),
        Some("describe")
    );
    assert_eq!(enclosing(&found, 10).map(|f| f.name.as_str()), Some("area"));
    assert_eq!(enclosing(&found, 19).map(|f| f.name.as_str()), Some("load"));
    assert_eq!(
        enclosing(&found, 28).map(|f| f.name.as_str()),
        Some("exported")
    );
}

const CPP_CLASSES: &str = "\
template <typename T>
class Box {
public:
    T get() const { return value; }
    template <typename U>
    Box<U> map(U (*f)(T)) {
        return Box<U>{f(value)};
    }
    static Box make(T v);
private:
    T value;
};

template <typename T>
Box<T> Box<T>::make(T v) {
    return Box<T>{v};
}

struct S {
    virtual ~S() = default;
    int (S::*member())(int) { return nullptr; }
};
";

#[test]
fn cpp_methods_in_a_class_body_and_out_of_it_are_functions() {
    let found = spans(tree_sitter_cpp::LANGUAGE, CPP_CLASSES);
    assert_eq!(
        named(&found),
        [
            ("get", 4, 4),
            ("map", 6, 8),
            ("Box<T>::make", 15, 17),
            // Defaulted, and so defined: the compiler emits it on that line.
            ("~S", 20, 20),
            // A pointer to member the grammar cannot parse, beside a declarator it can.
            ("member", 21, 21),
        ]
    );
    assert_eq!(enclosing(&found, 7).map(|f| f.name.as_str()), Some("map"));
    // The class's own lines, and a declaration without a body.
    assert_eq!(enclosing(&found, 2), None);
    assert_eq!(enclosing(&found, 9), None);
}

/// What the grammar cannot parse the scanner does not need to: a `const impl` -- which
/// turns the whole file into one error node for `tree-sitter-rust` -- is two functions
/// on their lines, and so is everything the tokens alone decide.
#[test]
fn rust_functions_survive_syntax_the_grammar_does_not_know() {
    let text = "\
#[rustc_const_unstable(feature = \"const_convert\", issue = \"143773\")]
const impl AsRef<str> for str {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self
    }
}

const trait Shape {
    fn area(&self) -> [u8; 4];
    fn sides(&self) -> usize { 4 }
}

fn apply(f: fn(i32) -> i32, g: extern \"C\" fn()) -> [u8; { 2 + 2 }] {
    // fn in_a_comment() {}
    /* fn in_a_block() { /* nested */ } */
    let s = \"fn in_a_string() {\";
    let r = r#\"fn in_a_raw_string() {\"#;
    let b = b\"fn in_bytes() {\";
    let brace = '{';
    let quote = '\\'';
    let escaped = '\\u{1F600}';
    let label = 'outer: loop { break 'outer; };
    f(0);
    [0; 4]
}

extern \"C\" {
    fn declared(x: i32) -> i32;
}

fn unterminated() {
    let x = 1;
";
    let found = rust::functions(text);
    assert_eq!(
        named(&found),
        [
            ("as_ref", 4, 6),
            ("sides", 11, 11),
            ("apply", 14, 26),
            ("unterminated", 32, 33),
        ]
    );
    assert_eq!(
        enclosing(&found, 21).map(|f| f.name.as_str()),
        Some("apply")
    );
    assert_eq!(enclosing(&found, 29), None);
}

/// Defect: the scanner looked for the closing quote of an escaped character one byte past
/// the backslash, so `'\''` ended on the escaped quote. The real one then opened a literal
/// of its own, `|'` was read as a character, and the `"` after it opened a string that ran
/// to the next one in the file -- swallowing every brace and `fn` in between.
#[test]
fn an_escaped_quote_ends_where_it_ends() {
    let text = "\
fn a(c: char) -> bool {
    matches!(c, '\\''|'\"')
}
fn b() {
    1
}
";
    assert_eq!(named(&rust::functions(text)), [("a", 1, 3), ("b", 4, 6)]);
}

/// Nothing in the text can make the scanner index past it: every shape of unfinished
/// token at the end of the file, and a file that is nothing but one of them.
#[test]
fn the_rust_scanner_ends_with_the_text() {
    for tail in [
        "fn a() {",
        "fn",
        "fn ",
        "fn b",
        "fn c(",
        "\"",
        "b\"",
        "r#\"",
        "r#",
        "br",
        "'",
        "'\\",
        "'a",
        "/*",
        "/* /*",
        "//",
        "\\",
        "b'",
        "r\"x",
        "fn d() -> [u8; 4",
    ] {
        let _ = rust::functions(tail);
        let _ = rust::functions(&format!("fn e() {{ {tail}"));
        let _ = rust::functions(&format!("{tail}\nfn f() {{}}"));
    }
    assert_eq!(named(&rust::functions("fn a() {")), [("a", 1, 1)]);
    assert_eq!(named(&rust::functions("fn a() -> T")), []);
}

use super::*;
use crate::source::Seeded;

/// Indentation the highlighter hands over as a length is the file's own characters in the
/// row's text, so a byte column is the same byte in the row and in the file. A no-break
/// space is two bytes: drawn as a plain space, every name after it would sit a byte left
/// of where a language server places it.
#[test]
fn indentation_keeps_the_files_own_characters() {
    let seeded = Seeded::directory("indent");
    let path = seeded.file("wide.rs", "fn f() {\n\u{a0}\u{3000}\tlet x = 1;\n}\n");
    let file = crate::source::load(&path).expect("the seeded file loads");

    let highlighted = Highlighted::new(file, Appearance::Light);

    assert_eq!(&*highlighted.text(1).whole, "\u{a0}\u{3000} let x = 1;");
}

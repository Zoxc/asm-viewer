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
    let file = source::load(&path).expect("the seeded file loads");

    let highlighted = Highlighted::new(file, Appearance::Light);

    assert_eq!(&*highlighted.text(1).whole, "\u{a0}\u{3000} let x = 1;");
}

/// A file that is not there is read once: the miss is filed, and no appearance asks for it
/// again until a forget covers it.
#[test]
fn a_missing_file_is_asked_about_until_it_is_forgotten() {
    let seeded = Seeded::directory("missing");
    let path = seeded.join("gone.rs");
    let showing: Arc<Path> = Arc::from(Path::new(&path));
    let sourced = Sourced::default();

    assert!(read(&SourceAsk {
        file: path.clone(),
        appearance: Appearance::Light,
    })
    .is_none());
    assert!(sourced.pending(&showing, Appearance::Light).is_none());
    assert!(sourced.pending(&showing, Appearance::Dark).is_none());

    forget_source_under(&seeded);
    assert!(sourced.pending(&showing, Appearance::Light).is_some());
}

/// A read is thrown away for a forget of its own directory made while it ran, and for no
/// other: a build of some other directory costs it nothing.
#[test]
fn a_read_is_dropped_only_for_a_forget_that_covers_it() {
    let file = Path::new("/built/src/main.rs");
    let mut forgets = Forgets::default();

    let at = forgets.count;
    forgets.add(Path::new("/elsewhere"));
    assert!(!forgets.since(at, file));
    forgets.add(Path::new("/built"));
    assert!(forgets.since(at, file));
    assert!(!forgets.since(forgets.count, file));

    // Outlived by more forgets than are kept: answered as if one of them covered it.
    let at = forgets.count;
    for _ in 0..=KEPT {
        forgets.add(Path::new("/elsewhere"));
    }
    assert!(forgets.since(at, file));
}

/// A row is a line as a compiler counts them, ended by `\n` alone. ropey also ends one at
/// a form feed, a vertical tab, a lone CR, NEL and the two Unicode separators, so a `^L`
/// on a line of its own, as GNU sources have, drew two rows and put every line below it
/// one row down. The row keeps each byte where the file has it; a copy takes the file's own.
#[test]
fn only_a_newline_ends_a_row() {
    let seeded = Seeded::directory("breaks");
    let path = seeded.file("breaks.c", "a;\n\x0c\nb;\n");
    let file = source::load(&path).expect("the seeded file loads");

    let highlighted = Highlighted::new(file, Appearance::Light);

    assert_eq!(highlighted.lines, 3);
    assert_eq!(&*highlighted.text(2).whole, "b;");
    assert_eq!(highlighted.line(1), "\x0c");

    let text = "x\x0by\rz\u{85}w\u{2028}v\u{2029}u\r\nt\n";
    let path = seeded.file("more.c", text);
    let file = source::load(&path).expect("the seeded file loads");

    let highlighted = Highlighted::new(file, Appearance::Light);

    assert_eq!(highlighted.lines, 2);
    let row = &*highlighted.text(0).whole;
    assert_eq!(row.len(), "x\x0by\rz\u{85}w\u{2028}v\u{2029}u".len());
    assert_eq!(row.encode_utf16().count(), 11);
    assert_eq!(&*highlighted.text(1).whole, "t");
    assert_eq!(highlighted.line(0), "x\x0by\rz\u{85}w\u{2028}v\u{2029}u");
}

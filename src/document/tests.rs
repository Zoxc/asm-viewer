use std::collections::HashMap;
use std::path::PathBuf;

use analysis::{Architecture, BinaryFormat, ObjectData, SectionAddress, SymbolData, SymbolIndex};

use super::*;

/// A bare `Object` with one text symbol — only the fields these tests read.
fn object(path: &str, name: &str) -> Arc<Object> {
    let caller = SymbolData::new(
        "caller".to_owned(),
        None,
        SectionAddress::new(0),
        None,
        None,
    );
    Arc::new(Object::new(
        PathBuf::from(path),
        name.to_owned(),
        BinaryFormat::Elf,
        Architecture::X86_64,
        HashMap::from([(SymbolIndex(0), Arc::new(caller))]),
        Vec::new(),
        ObjectData::from(&b"the first build"[..]),
    ))
}

/// The two members of one archive, which `path` alone cannot tell apart.
fn objects() -> Vec<Arc<Object>> {
    vec![object("/tmp/lib.a", "a.o"), object("/tmp/lib.a", "b.o")]
}

/// A member is not a file, so both members of `/tmp/lib.a` answer for it and a symbol
/// answers for the file its object came out of.
#[test]
fn everything_in_a_file_says_so() {
    let objects = objects();
    let lib = Path::new("/tmp/lib.a");
    let other = Path::new("/tmp/some.dll");

    let member = Document::Object(objects[1].clone());
    assert!(member.in_file(lib));
    assert!(!member.in_file(other));

    let symbol = Document::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[0].clone(),
    });
    assert!(symbol.in_file(lib));
    assert!(!symbol.in_file(other));
}

/// An object's code points into the file its object came out of, so it closes with it and
/// with nothing else.
#[test]
fn a_code_document_closes_with_its_file() {
    let objects = objects();
    let code = Document::Code(objects[1].clone());
    assert!(code.in_file(Path::new("/tmp/lib.a")));
    assert!(!code.in_file(Path::new("/tmp/some.dll")));
    assert!(code.symbol().is_none(), "no symbol to ask the worker about");
}

/// A file is in no binary: its chip outlives the binary that led the reader to it, so
/// closing that binary leaves it open — even where the two paths are spelt the same.
#[test]
fn a_source_document_closes_with_nothing() {
    let source = Document::Source(Arc::from(Path::new("/tmp/lib.a")));
    assert!(!source.in_file(Path::new("/tmp/lib.a")));
    assert_eq!(source.file(), Path::new("/tmp/lib.a"));
}

/// Which space a loose number is in is the document it came with, and the two documents
/// that are no place in any code give it none. **The one place a saved number is given a
/// space** (`src/project/restore.rs`), so a wrong answer here is a place restored in the
/// other listing's terms — right on a linked image, where every bias is 0, and wrong on
/// every relocatable object.
#[test]
fn a_loose_number_takes_the_space_of_the_document_it_came_with() {
    let object = object("/tmp/lib.a", "a.o");
    let symbol = Symbol {
        object: object.clone(),
        data: object.symbols_sorted[0].clone(),
    };

    assert_eq!(
        Address::in_document(&Document::Code(object.clone()), 0x40),
        Some(Address::Placed(PlacedAddress::new(0x40)))
    );
    assert_eq!(
        Address::in_document(&Document::Symbol(symbol), 0x40),
        Some(Address::Local(SectionAddress::new(0x40)))
    );

    // A file's assembly side is whichever symbol its line was compiled into, and an
    // object's symbol list is no place in any code: neither states a space of its own.
    assert_eq!(
        Address::in_document(&Document::Source(Arc::from(Path::new("main.rs"))), 0x40),
        None
    );
    assert_eq!(Address::in_document(&Document::Object(object), 0x40), None);
}

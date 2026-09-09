use std::collections::HashMap;
use std::path::PathBuf;

use analysis::{Architecture, BinaryFormat, ObjectData, SymbolData};

use super::*;

/// A bare `Object` with one text symbol — only the fields these tests read.
fn object(path: &str, name: &str) -> Arc<Object> {
    Arc::new(Object {
        path: PathBuf::from(path),
        name: name.to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols: HashMap::new(),
        symbols_sorted: vec![Arc::new(SymbolData {
            name: "caller".to_owned(),
            demangled: None,
            address: 0,
            section: None,
            size: 0,
        })],
        sections: Vec::new(),
        data: ObjectData::from(&b"the first build"[..]),
        debug_info: Default::default(),
        by_address: Default::default(),
    })
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

    let member = Document::Assembly(Selection::Object(objects[1].clone()));
    assert!(member.in_file(lib));
    assert!(!member.in_file(other));

    let symbol = Document::Assembly(Selection::Symbol(Symbol {
        object: objects[0].clone(),
        data: objects[0].symbols_sorted[0].clone(),
    }));
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
    let source = Document::Source(Arc::from("/tmp/lib.a"));
    assert!(!source.in_file(Path::new("/tmp/lib.a")));
    assert_eq!(source.file(), Path::new("/tmp/lib.a"));
}

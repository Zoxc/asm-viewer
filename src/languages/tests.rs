use super::*;

#[test]
fn a_language_is_the_extension_and_nothing_else() {
    let of = |name: &str| Language::of(Path::new(name));
    assert!(of("main.rs") == Some(Language::Rust));
    assert!(of("sum.c") == Some(Language::C));
    assert!(of("sum.h") == Some(Language::C));
    assert!(of("sum.hpp") == Some(Language::Cpp));
    assert!(of("Cargo.toml") == Some(Language::Toml));
    assert!(of("compile_commands.json") == Some(Language::Json));

    // Named for what they compile to, with no grammar behind them.
    assert!(of("main.zig") == Some(Language::Zig));
    assert!(of("main.go") == Some(Language::Go));
    assert!(of("start.s") == Some(Language::Assembly));
    assert!(of("start.S") == Some(Language::Assembly));
    assert!(of("view.m") == Some(Language::ObjC));
    assert!(of("kernel.cu") == Some(Language::Cuda));
    assert!(of("solve.f90") == Some(Language::Fortran));

    // No extension at all, and one nothing here knows.
    assert!(of("Makefile").is_none());
    assert!(of("notes.md").is_none());
    assert!(of("build.py").is_none());
    // The name is not read: a file called `rs` is not Rust.
    assert!(of("rs").is_none());
    // The extension is read as written: `.C` is C++ to one compiler and C to another, so
    // it is nothing here rather than a guess.
    assert!(of("sum.C").is_none());
}

/// What decides whether a tab opens with an assembly side: a language named here that
/// becomes machine code, and nothing else. A file the app cannot place is not one.
#[test]
fn only_a_named_compiled_language_is_compiled() {
    assert!(compiled(Path::new("main.rs")));
    assert!(compiled(Path::new("sum.c")));
    assert!(compiled(Path::new("sum.hpp")));
    assert!(!compiled(Path::new("Cargo.toml")));
    assert!(!compiled(Path::new("compile_commands.json")));

    // A language named for what it compiles to needs no grammar to be answered yes.
    for named in [
        "shader.zig",
        "server.go",
        "start.S",
        "view.mm",
        "solve.f90",
        "runtime.d",
        "App.swift",
        "kernel.cu",
    ] {
        assert!(compiled(Path::new(named)), "{named}");
    }

    // And an extension nothing here names is still no.
    assert!(!compiled(Path::new("Makefile")));
    assert!(!compiled(Path::new("notes.md")));
}

/// The narrow half of the policy above: five of the twenty are coloured, and every other
/// one is answered plainly rather than left out of the match.
#[test]
fn only_five_languages_have_a_grammar() {
    for coloured in [
        Language::Rust,
        Language::C,
        Language::Cpp,
        Language::Toml,
        Language::Json,
    ] {
        assert!(coloured.grammar().is_some(), "{coloured:?}");
    }

    for plain in [
        Language::ObjC,
        Language::Assembly,
        Language::Go,
        Language::Zig,
        Language::D,
        Language::Swift,
        Language::Nim,
        Language::Odin,
        Language::Fortran,
        Language::Ada,
        Language::Pascal,
        Language::Haskell,
        Language::OCaml,
        Language::Crystal,
        Language::Cuda,
    ] {
        assert!(plain.grammar().is_none(), "{plain:?}");
    }
}

/// Which of the three answers a language gets: Rust its own scanner, C and C++ a parse
/// with the grammar, and everything else nothing -- **including** a language that has a
/// grammar, since colouring a file is not finding functions in it.
#[test]
fn functions_follow_the_language_and_not_the_grammar() {
    let names = |found: Vec<Function>| {
        found
            .into_iter()
            .map(|function| function.name)
            .collect::<Vec<_>>()
    };

    assert!(names(Language::Rust.functions("fn one() {}\n")) == ["one"]);
    assert!(names(Language::C.functions("int two(void) { return 2; }\n")) == ["two"]);
    assert!(names(Language::Cpp.functions("struct S { void three() {} };\n")) == ["three"]);

    // Both have a grammar, and a configuration file defines no functions.
    assert!(Language::Toml
        .functions("[package]\nname = \"one\"\n")
        .is_empty());
    assert!(Language::Json.functions("{ \"one\": 1 }\n").is_empty());
    // And a language with no grammar has nothing to parse with.
    assert!(Language::Zig.functions("fn one() void {}\n").is_empty());
}

/// The one language server this app can name, and the one place it is named.
#[test]
fn rust_is_the_only_language_with_a_server() {
    assert!(Language::Rust.server() == Some("rust-analyzer"));
    assert!(Language::C.server().is_none());
    assert!(Language::Toml.server().is_none());
}

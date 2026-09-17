//! The language a file is written in, and every per-language question the app asks.
//!
//! One list of extensions, and one place a fact about a language is decided: what
//! compiles, which grammar colours it, how its functions are found, which language server
//! reads it. The panes, the highlighter and the Project view ask here rather than reading
//! an extension again.

use std::path::Path;
use tree_sitter_language::LanguageFn;

use crate::functions::{self, Function};

/// The language a file is written in, going by its extension, and every per-language
/// question the app asks: whether a compiler turns it into machine code, which grammar
/// colours it, how its functions are found, and which language server reads it.
///
/// **The one extension list.** `.h` is C and not C++, a header the C grammar misparses
/// being coloured oddly rather than dropped.
///
/// Most of these have no grammar here ([`grammar`](Language::grammar)) and are never
/// coloured. Naming them is still worth the lines: a grammar costs a dependency and a
/// parser generator's worth of generated C in the binary (`notes/Goals.md`), where
/// knowing that a `.zig` becomes machine code costs one arm and is what decides whether
/// a tab opens with an assembly side. So the list is generous about languages and stays
/// narrow about grammars, and a language that grows one later changes an arm rather than
/// joining the enum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Rust,
    C,
    Cpp,
    ObjC,
    Assembly,
    Go,
    Zig,
    D,
    Swift,
    Nim,
    Odin,
    Fortran,
    Ada,
    Pascal,
    Haskell,
    OCaml,
    Crystal,
    Cuda,
    Toml,
    Json,
}

impl Language {
    /// What a language server calls this language, which is what a file is opened with
    /// (`textDocument/didOpen`). The identifiers are the protocol's own list where it has
    /// one and the extension otherwise, which is what the specification says to do.
    ///
    /// Exhaustive on purpose, as every match over this is: a language added above is one
    /// a server has to be told the name of.
    pub fn spoken(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::ObjC => "objective-c",
            Language::Assembly => "asm",
            Language::Go => "go",
            Language::Zig => "zig",
            Language::D => "d",
            Language::Swift => "swift",
            Language::Nim => "nim",
            Language::Odin => "odin",
            Language::Fortran => "fortran",
            Language::Ada => "ada",
            Language::Pascal => "pascal",
            Language::Haskell => "haskell",
            Language::OCaml => "ocaml",
            Language::Crystal => "crystal",
            Language::Cuda => "cuda",
            Language::Toml => "toml",
            Language::Json => "json",
        }
    }

    /// The language of the file at `path`, or [`None`] for an extension this does not
    /// know, which is not the same as saying the file is not source.
    ///
    /// The extension is read as it is written. `.s` and `.S` are both assembly and are
    /// both named, but nothing is lower-cased on the way in: `.C` is C++ to a Unix
    /// compiler and a Windows C file to everyone else, so a fold would have to pick one
    /// and would be wrong half the time.
    pub fn of(path: &Path) -> Option<Language> {
        Some(match path.extension()?.to_str()? {
            "rs" => Language::Rust,
            "c" | "h" => Language::C,
            "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" | "inl" | "ipp" | "tcc" => {
                Language::Cpp
            }
            // `.m` is Objective-C here and not MATLAB: what reaches this app is a file a
            // debugger or a Mach-O's line info named, and MATLAB compiles to nothing a
            // symbol table lists.
            "m" | "mm" => Language::ObjC,
            "s" | "S" | "asm" => Language::Assembly,
            "go" => Language::Go,
            "zig" => Language::Zig,
            "d" => Language::D,
            "swift" => Language::Swift,
            "nim" => Language::Nim,
            "odin" => Language::Odin,
            "f" | "for" | "f90" | "f95" | "f03" | "f08" => Language::Fortran,
            "adb" | "ads" => Language::Ada,
            "pas" | "pp" => Language::Pascal,
            "hs" => Language::Haskell,
            "ml" => Language::OCaml,
            "cr" => Language::Crystal,
            "cu" | "cuh" => Language::Cuda,
            // The two configuration languages a project directory is full of, for the
            // tabs the Files view opens: nothing is compiled from them, but `Cargo.toml`
            // is read.
            "toml" => Language::Toml,
            "json" => Language::Json,
            _ => return None,
        })
    }

    /// Whether a compiler turns this language into machine code, so a file in it has
    /// assembly to show beside it.
    ///
    /// Assembly counts: it is assembled rather than compiled, and a binary is as much
    /// built from it either way. Haskell and OCaml count for their native back ends,
    /// which is what puts them in a symbol table at all.
    pub fn compiled(self) -> bool {
        match self {
            Language::Rust
            | Language::C
            | Language::Cpp
            | Language::ObjC
            | Language::Assembly
            | Language::Go
            | Language::Zig
            | Language::D
            | Language::Swift
            | Language::Nim
            | Language::Odin
            | Language::Fortran
            | Language::Ada
            | Language::Pascal
            | Language::Haskell
            | Language::OCaml
            | Language::Crystal
            | Language::Cuda => true,
            Language::Toml | Language::Json => false,
        }
    }

    /// C's grammar. Named here and not in [`grammar`]'s arm because two answers are made
    /// of it: the pair that colours a C file, and the parse [`functions`] finds one's
    /// functions with. The second is not asking whether there is a grammar, so it reads
    /// the constant and has no [`Option`] to unwrap.
    ///
    /// [`grammar`]: Language::grammar
    /// [`functions`]: Language::functions
    const C_GRAMMAR: LanguageFn = tree_sitter_c::LANGUAGE;

    /// C++'s, for the same two answers.
    const CPP_GRAMMAR: LanguageFn = tree_sitter_cpp::LANGUAGE;

    /// The tree-sitter grammar this is parsed with and the query that colours it, for the
    /// five that have one. [`None`] is not a failure: a file no grammar knows is drawn as
    /// one plain span per line.
    ///
    /// The match is exhaustive on purpose: a language added above is a language this has
    /// to answer for, and the answer for most of them is that a grammar costs a
    /// dependency and a parser generator's worth of generated C (`notes/Goals.md`).
    ///
    /// The type is tree-sitter's own and not the editor's, so that nothing here has to
    /// know the UI: `ui/highlight.rs` wraps the pair in freya's `EditorLanguage`.
    pub fn grammar(self) -> Option<(LanguageFn, &'static str)> {
        Some(match self {
            Language::Rust => (
                tree_sitter_rust::LANGUAGE,
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
            Language::C => (Language::C_GRAMMAR, tree_sitter_c::HIGHLIGHT_QUERY),
            Language::Cpp => (Language::CPP_GRAMMAR, tree_sitter_cpp::HIGHLIGHT_QUERY),
            Language::Toml => (
                tree_sitter_toml_ng::LANGUAGE,
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            ),
            Language::Json => (
                tree_sitter_json::LANGUAGE,
                tree_sitter_json::HIGHLIGHTS_QUERY,
            ),
            // Named for what they compile to and not for how they are drawn.
            Language::ObjC
            | Language::Assembly
            | Language::Go
            | Language::Zig
            | Language::D
            | Language::Swift
            | Language::Nim
            | Language::Odin
            | Language::Fortran
            | Language::Ada
            | Language::Pascal
            | Language::Haskell
            | Language::OCaml
            | Language::Crystal
            | Language::Cuda => return None,
        })
    }

    /// The functions a file of this language defines, by the lines each spans: Rust by
    /// the scanner of its own (`functions::rust`, the grammar being behind the compiler),
    /// C and C++ by a parse with the grammar that colours them.
    ///
    /// A grammar is not an answer on its own. TOML and JSON have one and define no
    /// functions, and the rest have no grammar to parse with, so both are no functions
    /// rather than a parse made to find that out.
    pub fn functions(self, text: &str) -> Vec<Function> {
        match self {
            Language::Rust => functions::rust::functions(text),
            Language::C => functions::parsed(Language::C_GRAMMAR, text),
            Language::Cpp => functions::parsed(Language::CPP_GRAMMAR, text),
            _ => Vec::new(),
        }
    }

    /// The program a project written in this is read with, where this app knows of one.
    ///
    /// Rust alone: rust-analyzer is the one such program named here, and a project on
    /// another toolchain says which to run instead (`Project::language_server`).
    pub fn server(self) -> Option<&'static str> {
        match self {
            Language::Rust => Some("rust-analyzer"),
            _ => None,
        }
    }
}

/// Whether the file at `path` is in a compiled language.
///
/// An extension [`Language::of`] does not know is not one. Only a language named here is
/// known to become machine code, so a tab opens with an assembly side where the file is
/// one this app can say that of, and with the source alone otherwise.
pub fn compiled(path: &Path) -> bool {
    Language::of(path).is_some_and(Language::compiled)
}

#[cfg(test)]
mod tests;

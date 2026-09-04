//! Source files, read off disk once and remembered — including the ones that are not
//! there.
//!
//! A path out of debug info is a weak thing to trust, so every failure is the same answer,
//! [`None`], and the pane draws a placeholder. The misses are cached too: a pane asks on
//! every render, and caching only the successes would make a path that is not on this
//! machine the expensive case.
//!
//! [`Language`] is here too: the one list of extensions the app knows, which the
//! highlighter and the panes both ask.

use analysis::{SourceDigests, SourceHash};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

/// The largest file this will read into memory. A bound on what a bad path can cost, not a
/// guess at what source looks like: a debug-info string that happens to name a disk image
/// must not be loaded to find that out.
pub const MAX_SIZE: u64 = 16 * 1024 * 1024;

/// The language a file is written in, going by its extension: which grammar colours it,
/// and whether a compiler turns it into machine code.
///
/// **The one extension list.** `.h` is C and not C++, a header the C grammar misparses
/// being coloured oddly rather than dropped.
///
/// Most of these have no grammar here and are never coloured. Naming them is still worth
/// the lines: a grammar costs a dependency and a parser generator's worth of generated C
/// in the binary (`notes/Goals.md`), where knowing that a `.zig` becomes machine code
/// costs one arm and is what decides whether a tab opens with an assembly side. So the
/// list is generous about languages and stays narrow about grammars, and a language that
/// grows one later changes an arm rather than joining the enum.
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
}

/// Whether the file at `path` is in a compiled language.
///
/// An extension [`Language::of`] does not know is not one. Only a language named here is
/// known to become machine code, so a tab opens with an assembly side where the file is
/// one this app can say that of, and with the source alone otherwise.
pub fn compiled(path: &Path) -> bool {
    Language::of(path).is_some_and(Language::compiled)
}

/// One source file: where it came from, and what it says. Splitting it into lines is the
/// UI's syntax highlighter's job, which works in whole files and hands its own line breaks
/// back.
pub struct SourceFile {
    path: PathBuf,
    text: String,
    /// The digests of the bytes as read — before the lossy decode, since the compiler
    /// hashed the bytes too — taken once with the file, so a pane asking on every render
    /// compares two arrays.
    digests: SourceDigests,
}

impl SourceFile {
    /// Where this was read from, i.e. the path the debug info named.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's contents, decoded lossily.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether this file is the one a checksum out of the debug info was taken of: the
    /// file the binary was built from, and not that file edited since.
    pub fn matches(&self, hash: SourceHash) -> bool {
        hash.matches(&self.digests)
    }

    /// Read a file, or [`None`] for anything that is not a readable text-sized regular
    /// file.
    ///
    /// The size is checked *before* the bytes are read, and `is_file` before that: a
    /// directory opens happily on Linux and a fifo blocks the reader until someone writes
    /// to it, and neither may reach a UI thread. `max_size` is a parameter only so the
    /// tests can set a small one.
    fn read(path: &Path, max_size: u64) -> Option<SourceFile> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > max_size {
            return None;
        }

        // Lossy rather than strict: a file with one bad byte in a comment is still a
        // source file.
        let bytes = fs::read(path).ok()?;

        Some(SourceFile {
            path: path.to_path_buf(),
            digests: SourceDigests::of(&bytes),
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

/// Every path asked about so far and what came back, `None` included. A `static` so that
/// two panes asking for one file get the same `Arc` rather than two copies of a megabyte.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<SourceFile>>>>> =
    LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<SourceFile>>>> {
    // A poisoned lock must not turn an unreadable file into a crashed app.
    CACHE.lock().unwrap_or_else(|error| error.into_inner())
}

/// The contents of `path`, read on the first call and answered from memory afterwards.
/// [`None`] means the file cannot be shown — missing, unreadable, not a file, or past
/// [`MAX_SIZE`] — and is remembered as such.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    if let Some(cached) = cache().get(path) {
        return cached.clone();
    }

    // Read outside the lock: holding it across the read would make every other pane wait
    // on this file. The cost is that two callers racing for one path may both read it, and
    // the second's copy is dropped when it loses the insert.
    let file = SourceFile::read(path, MAX_SIZE).map(Arc::new);

    cache().entry(path.to_path_buf()).or_insert(file).clone()
}

#[cfg(test)]
mod tests;

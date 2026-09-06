//! Source files, read off disk once and remembered — including the ones that are not
//! there.
//!
//! A path out of debug info is a weak thing to trust, so every failure is the same answer,
//! [`None`], and the pane draws a placeholder. The misses are cached too: a pane asks on
//! every render, and caching only the successes would make a path that is not on this
//! machine the expensive case.
//!
//! [`Language`] is here too: the one list of extensions the app knows, and the one place
//! a per-language fact is decided -- what compiles, which grammar colours it, how its
//! functions are found, which language server reads it. The panes, the highlighter and
//! the Project view all ask it rather than matching on it again.

use analysis::{SourceDigests, SourceHash};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};
use tree_sitter_language::LanguageFn;

use crate::functions::{self, Function};

/// The largest file this will read into memory. A bound on what a bad path can cost, not a
/// guess at what source looks like: a debug-info string that happens to name a disk image
/// must not be loaded to find that out.
pub const MAX_SIZE: u64 = 16 * 1024 * 1024;

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
            Language::C => (tree_sitter_c::LANGUAGE, tree_sitter_c::HIGHLIGHT_QUERY),
            Language::Cpp => (tree_sitter_cpp::LANGUAGE, tree_sitter_cpp::HIGHLIGHT_QUERY),
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
    /// C and C++ by a parse with [`grammar`].
    ///
    /// A grammar is not an answer on its own. TOML and JSON have one and define no
    /// functions, and the rest have no grammar to parse with, so both are no functions
    /// rather than a parse made to find that out.
    ///
    /// [`grammar`]: Language::grammar
    pub fn functions(self, text: &str) -> Vec<Function> {
        match self {
            Language::Rust => functions::rust::functions(text),
            Language::C | Language::Cpp => self
                .grammar()
                .map_or_else(Vec::new, |(grammar, _)| functions::parsed(grammar, text)),
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
    /// file: [`contents`]' rule, and the digests of the bytes it read.
    ///
    /// `max_size` is a parameter only so the tests can set a small one.
    fn read(path: &Path, max_size: u64) -> Option<SourceFile> {
        let (bytes, text) = contents(path, max_size)?;
        Some(SourceFile {
            path: path.to_path_buf(),
            digests: SourceDigests::of(&bytes),
            text,
        })
    }
}

/// The text of `path` by [`load`]'s rule -- a regular file within [`MAX_SIZE`], decoded
/// lossily -- read fresh and not remembered: for a reader that wants many files once
/// (`src/references.rs`), which the cache would otherwise hold for the life of the app.
///
/// It is here and not a `fs::read_to_string` at the caller so that there is one answer to
/// what a source file is. A second rule means the same file read two ways: a line a pane
/// draws that a list of references leaves blank, or a fifo a language server named opened
/// on a worker that then never returns.
pub fn read_text(path: &Path) -> Option<String> {
    contents(path, MAX_SIZE).map(|(_, text)| text)
}

/// The bytes of `path` and those bytes decoded, or [`None`] for anything that is not a
/// readable regular file within `max_size`. **The one rule** for reading a source file by
/// path; both readers above are this plus what they keep.
///
/// The size is checked *before* the bytes are read, and `is_file` before that: a directory
/// opens happily on Linux and a fifo blocks the reader until someone writes to it, and
/// neither may reach a UI thread.
///
/// The bytes come back beside the text because the digests are of the bytes as read: the
/// compiler hashed those, and a lossy decode is not reversible.
fn contents(path: &Path, max_size: u64) -> Option<(Vec<u8>, String)> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_size {
        return None;
    }

    let bytes = fs::read(path).ok()?;
    // Lossy rather than strict: a file with one bad byte in a comment is still a source
    // file.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some((bytes, text))
}

/// Every path asked about so far and what came back, `None` included. A `static` so that
/// two panes asking for one file get the same `Arc` rather than two copies of a megabyte.
static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<SourceFile>>>>> =
    LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<SourceFile>>>> {
    // A poisoned lock must not turn an unreadable file into a crashed app.
    CACHE.lock().unwrap_or_else(|error| error.into_inner())
}

/// What has been forgotten so far: how many times, and the last few directories it was.
///
/// Read before a file is and asked again before what was read is filed, so that a read
/// which began before a [`forget_under`] and finished after it is not put back as what is
/// on disk now. The reading is a worker thread's (`src/ui/highlight.rs`) and the
/// forgetting is a finished build's, on the UI thread, so the two do interleave: the file
/// is read, the build writes it and says so, and the copy from before the build is then
/// filed under the path nothing will ask about again.
///
/// **The directories and not the count alone**, since a forget is about one of them: a
/// file read while some other directory was being forgotten is a file nothing has said
/// anything about, and dropping it would cost a read for every build in a window the
/// reader is not even looking at. Only the last [`KEPT`] are held -- a bound on what this
/// costs, the answer for a read that has been outlived by that many forgets being that it
/// may well have been forgotten.
///
/// One record for both caches, since neither can be forgotten without the other.
static FORGOTTEN: LazyLock<Mutex<Forgets>> = LazyLock::new(Mutex::default);

/// How many directories back [`FORGOTTEN`] remembers.
const KEPT: usize = 16;

#[derive(Default)]
struct Forgets {
    /// How many times anything has been forgotten.
    count: u64,
    /// The roots of the last [`KEPT`] of them, oldest first.
    roots: VecDeque<PathBuf>,
}

fn forgets() -> MutexGuard<'static, Forgets> {
    FORGOTTEN.lock().unwrap_or_else(|error| error.into_inner())
}

/// How many times anything has been forgotten so far.
pub fn forgotten() -> u64 {
    forgets().count
}

/// Whether anything forgotten since `at` covers `path`, which is what says a copy read
/// then must not be filed now.
pub fn forgotten_since(at: u64, path: &Path) -> bool {
    let forgets = forgets();
    let since = forgets.count.saturating_sub(at);
    if since == 0 {
        return false;
    }
    // More forgets than are remembered: the ones this cannot answer for are answered as
    // if they were about this file.
    if since > forgets.roots.len() as u64 {
        return true;
    }
    forgets
        .roots
        .iter()
        .rev()
        .take(since as usize)
        .any(|root| path.starts_with(root))
}

/// The contents of `path`, read on the first call and answered from memory afterwards.
/// [`None`] means the file cannot be shown — missing, unreadable, not a file, or past
/// [`MAX_SIZE`] — and is remembered as such.
///
/// Nothing here notices a file that changed on disk. [`forget_under`] is how it is told.
pub fn load(path: &Path) -> Option<Arc<SourceFile>> {
    if let Some(cached) = cache().get(path) {
        return cached.clone();
    }

    // Read outside the lock: holding it across the read would make every other pane wait
    // on this file. The cost is that two callers racing for one path may both read it, and
    // the second's copy is dropped when it loses the insert.
    let at = forgotten();
    let file = SourceFile::read(path, MAX_SIZE).map(Arc::new);

    let mut cache = cache();
    // Forgotten while it was being read: what came back is the file as it was before
    // whatever said so, and is handed to the caller that asked for it rather than filed
    // for everyone after. Asked under this cache's lock, which [`forget_under`] takes
    // too, so a forget is either counted here or has yet to empty anything.
    if forgotten_since(at, path) {
        return file;
    }
    cache.entry(path.to_path_buf()).or_insert(file).clone()
}

/// Forget every file read from under `root`, misses included, so the next call reads them
/// again.
///
/// Checking on the way in would be a `stat` per lookup, and a pane asks on every render.
/// So a build is what calls this, a build being the app's one word that a directory's
/// files have changed. The parsed copies above these go with them
/// (`src/ui/highlight.rs`).
pub fn forget_under(root: &Path) {
    let mut forgets = forgets();
    forgets.count += 1;
    forgets.roots.push_back(root.to_path_buf());
    if forgets.roots.len() > KEPT {
        forgets.roots.pop_front();
    }
    // Written down and let go of before the cache is taken. A reader takes the two the
    // other way round -- the cache, then this, to ask what happened while it was reading
    // -- so holding both here is the one thing that would deadlock.
    drop(forgets);
    cache().retain(|path, _| !path.starts_with(root));
}

/// A source file the cache answers for with nothing on the disk: what a test uses when the
/// file is a fixture and not the thing under test.
///
/// [`Temporary`](crate::temporary::Temporary) is the other half of this, and what a test
/// is about is what decides between them. A real file when the reading is the point — a
/// file read once, a miss remembered, a directory forgotten and read again, a path that
/// only reduces through `canonicalize` — and one of these when the pane merely has to have
/// something to draw.
///
/// Nothing is made, so the directory is a name and not a place. The entries come out on
/// `Drop`, which unwinding runs: [`CACHE`] is a `static` that outlives every test in the
/// process, and a test that left its files in it would be paying for them in every test
/// after. What the drop takes is this cache and not the parsed copies above it, so a test
/// that reads a seeded file through `source_text` forgets it with `forget_source_under`
/// as it would a real one.
#[cfg(test)]
pub struct Seeded {
    directory: PathBuf,
}

#[cfg(test)]
impl Seeded {
    /// A directory of this call's own, named per process and per call so that tests seeding
    /// files can run in parallel, here and in another checkout at once. It is under the
    /// system temporary directory for one reason, that being an absolute path on every
    /// platform; nothing is written there.
    pub fn directory(name: &str) -> Seeded {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        Seeded {
            directory: std::env::temp_dir().join(format!(
                "assembly-viewer-seeded-{}-{unique}-{name}",
                std::process::id()
            )),
        }
    }

    /// Put `text` at `name` under it, and hand back the path a pane asks for it by.
    pub fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.directory.join(name);
        let bytes = text.as_bytes();
        let file = SourceFile {
            path: path.clone(),
            digests: SourceDigests::of(bytes),
            text: text.to_owned(),
        };
        cache().insert(path.clone(), Some(Arc::new(file)));
        path
    }

    /// The same, as the string a `Document::Source` names a file by.
    pub fn named(&self, name: &str, text: &str) -> Arc<str> {
        Arc::from(
            self.file(name, text)
                .to_str()
                .expect("the temporary directory is utf-8"),
        )
    }
}

#[cfg(test)]
impl std::ops::Deref for Seeded {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.directory
    }
}

#[cfg(test)]
impl Drop for Seeded {
    fn drop(&mut self) {
        forget_under(&self.directory);
    }
}

#[cfg(test)]
mod tests;

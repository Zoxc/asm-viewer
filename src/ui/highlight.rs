//! Source files as the reader sees them: read off disk, parsed and coloured once, since
//! the highlighter is stateful across lines and cannot be asked about one row at a time.

use super::*;

/// A loaded, highlighted source file, compared by pointer.
#[derive(Clone)]
pub(crate) struct SourceText(pub(crate) Arc<Highlighted>);

impl PartialEq for SourceText {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A source file ready to be drawn: its text as a rope, the coloured spans tree-sitter
/// produced for each of its lines, and the functions it defines by the lines they span.
///
/// `SyntaxBlocks` has two traps: `get_line` unwraps rather than answering `None`, and it
/// holds one block per `Rope::len_lines()` -- which counts a phantom line after a trailing
/// newline. Hence `lines`.
pub(crate) struct Highlighted {
    pub(crate) rope: Rope,
    pub(crate) blocks: SyntaxBlocks,
    /// How many rows the pane draws, which is *not* `blocks.len()`.
    pub(crate) lines: usize,
    /// Every function in the file, outer before inner, for a row to say which one it is
    /// a line of. Empty for a file no grammar parses.
    pub(crate) functions: Vec<Function>,
}

impl Highlighted {
    fn new(file: &SourceFile) -> Highlighted {
        let rope = Rope::from_str(file.text());
        let theme = palette().syntax();
        let language = language(file.path());

        let mut highlighter = SyntaxHighlighter::new();
        // A language of `None` is not a failure: the highlighter then hands back one
        // plain span per line in the theme's text colour.
        highlighter.set_language(language.as_ref(), &theme);

        let mut blocks = SyntaxBlocks::default();
        highlighter.parse(&rope, &mut blocks, None, &theme);

        let lines = blocks
            .len()
            .saturating_sub(usize::from(file.text().ends_with('\n')));

        // The function spans: Rust by the scanner of its own (`functions::rust`, the
        // grammar being behind the compiler), C and C++ parsed a second time with the
        // same grammar -- `SyntaxHighlighter` keeps its tree private, and what is wanted
        // of it is a few hundred bytes kept against a tree that would be most of the
        // file again. Milliseconds, once per file, in the same render the highlighting
        // already costs.
        let functions = match source::Language::of(file.path()) {
            Some(source::Language::Rust) => functions::rust::functions(file.text()),
            // A configuration file defines no functions, so the second parse is not made.
            Some(source::Language::Toml | source::Language::Json) => Vec::new(),
            _ => language
                .as_ref()
                .and_then(|language| {
                    let mut parser = tree_sitter::Parser::new();
                    parser.set_language(&language.language).ok()?;
                    let tree = parser.parse(file.text(), None)?;
                    Some(functions::functions(&tree, file.text().as_bytes()))
                })
                .unwrap_or_default(),
        };

        Highlighted {
            rope,
            blocks,
            lines,
            functions,
        }
    }
}

/// The tree-sitter grammar to parse a file with, where there is one, [`source::Language`]
/// being where the extensions are read.
///
/// The match is exhaustive on purpose: a language added there is a language this has to
/// answer for, and the answer for most of them is that a grammar costs a dependency and a
/// parser generator's worth of generated C, so they render plain (`notes/Goals.md`).
pub(crate) fn language(path: &Path) -> Option<EditorLanguage> {
    let (language, query) = match source::Language::of(path)? {
        source::Language::Rust => (
            tree_sitter_rust::LANGUAGE,
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        source::Language::C => (tree_sitter_c::LANGUAGE, tree_sitter_c::HIGHLIGHT_QUERY),
        source::Language::Cpp => (tree_sitter_cpp::LANGUAGE, tree_sitter_cpp::HIGHLIGHT_QUERY),
        source::Language::Toml => (
            tree_sitter_toml_ng::LANGUAGE,
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        source::Language::Json => (
            tree_sitter_json::LANGUAGE,
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        // Named for what they compile to and not for how they are drawn.
        source::Language::ObjC
        | source::Language::Assembly
        | source::Language::Go
        | source::Language::Zig
        | source::Language::D
        | source::Language::Swift
        | source::Language::Nim
        | source::Language::Odin
        | source::Language::Fortran
        | source::Language::Ada
        | source::Language::Pascal
        | source::Language::Haskell
        | source::Language::OCaml
        | source::Language::Crystal
        | source::Language::Cuda => return None,
    };

    Some(EditorLanguage::new(language, query))
}

/// Every file highlighted so far.
///
/// A `SyntaxBlocks` holds a `Color` per span, resolved against `palette().syntax()` at
/// parse time, so **a theme switch has to empty this map**: the entries are the wrong
/// theme rather than stale, and nothing a re-render does would repaint them. That clear
/// is inside [`set_appearance`], so it cannot be routed around.
static HIGHLIGHTED: LazyLock<Mutex<HashMap<PathBuf, Arc<Highlighted>>>> =
    LazyLock::new(Mutex::default);

pub(crate) fn highlighted() -> MutexGuard<'static, HashMap<PathBuf, Arc<Highlighted>>> {
    HIGHLIGHTED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Forget every file under `root`, in both caches: what was parsed here, and the text it
/// was parsed from. Neither can go without the other, a parsed copy holding the old text
/// in a `Rope` of its own.
///
/// **A build calls this**, with the directory it built (`ui/building.rs`, `ui/pad.rs`).
/// Both maps are keyed by path alone and neither is ever checked against the disk, so
/// without it the first text read for a file is the text every later render draws --
/// however often the file is rewritten, which a scratchpad's is on every build.
pub(crate) fn forget_source_under(root: &Path) {
    highlighted().retain(|path, _| !path.starts_with(root));
    source::forget_under(root);
}

/// The file at `path`, read and highlighted, or `None` when it cannot be shown at all.
pub(crate) fn source_text(path: &Path) -> Option<SourceText> {
    if let Some(cached) = highlighted().get(path) {
        return Some(SourceText(cached.clone()));
    }

    // Read and parsed outside the lock: this is the slow step, and a racing caller's
    // copy costs an allocation rather than a wait.
    let file = Arc::new(Highlighted::new(&*source::load(path)?));

    Some(SourceText(
        highlighted()
            .entry(path.to_path_buf())
            .or_insert(file)
            .clone(),
    ))
}

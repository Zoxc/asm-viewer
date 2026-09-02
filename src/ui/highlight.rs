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
        let functions = match file.path().extension().and_then(|ext| ext.to_str()) {
            Some("rs") => functions::rust::functions(file.text()),
            // A configuration file defines no functions, so the second parse is not made.
            Some("toml" | "json") => Vec::new(),
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

/// The tree-sitter grammar to parse a file with, chosen by extension. `.h` goes to C
/// rather than C++; a header the C grammar misparses is coloured oddly, never dropped.
pub(crate) fn language(path: &Path) -> Option<EditorLanguage> {
    let (language, query) = match path.extension()?.to_str()? {
        "rs" => (
            tree_sitter_rust::LANGUAGE,
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        ),
        "c" | "h" => (tree_sitter_c::LANGUAGE, tree_sitter_c::HIGHLIGHT_QUERY),
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hxx" | "hh" => {
            (tree_sitter_cpp::LANGUAGE, tree_sitter_cpp::HIGHLIGHT_QUERY)
        }
        // The two configuration languages a project directory is full of, for the tabs
        // the Files view opens: nothing is compiled from them, but `Cargo.toml` is read.
        "toml" => (
            tree_sitter_toml_ng::LANGUAGE,
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        ),
        "json" => (
            tree_sitter_json::LANGUAGE,
            tree_sitter_json::HIGHLIGHTS_QUERY,
        ),
        _ => return None,
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

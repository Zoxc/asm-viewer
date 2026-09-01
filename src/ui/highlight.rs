//! Source files as the reader sees them: read off disk, parsed and coloured once.
//!
//! A file is highlighted when it is loaded and never while a row is being drawn -- the
//! highlighter is stateful across lines, so it cannot be asked about one row at a time --
//! and the answer is kept in a `static` rather than in UI state, since it is the same
//! answer for every pane and outlives all of them.
//!
//! What a cache entry holds is not only the parse: the palette's colours are resolved
//! into the spans at parse time, so a theme switch leaves the entries wrong rather than
//! stale, and `set_appearance` empties the map.

use super::*;

/// A loaded, highlighted source file, compared by pointer.
#[derive(Clone)]
pub(crate) struct SourceText(pub(crate) Arc<Highlighted>);

impl PartialEq for SourceText {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A source file ready to be drawn: its text as a rope, and the coloured spans
/// tree-sitter produced for each of its lines.
///
/// The highlighter comes from `freya-code-editor`, whose `CodeEditor` component this pane
/// deliberately does not use: it paints a line background only for the cursor's own row
/// and keeps its scroll state private, so it can neither highlight the set of lines an
/// instruction maps to nor be scrolled to one. Its `SyntaxHighlighter` is public on its
/// own and is exactly the shape these rows want. (The Scratchpad pane *does* use the
/// component -- see [`SourceEditor`] -- because neither objection survives the pane being
/// one the reader is typing in.)
pub(crate) struct Highlighted {
    pub(crate) rope: Rope,
    pub(crate) blocks: SyntaxBlocks,
    /// How many rows the pane draws, which is *not* `blocks.len()`: a rope counts a
    /// phantom empty line after a trailing newline and the highlighter pushes a block
    /// for it, and no editor shows that line.
    pub(crate) lines: usize,
}

impl Highlighted {
    /// Parse and colour a whole file, once. The highlighter is stateful across lines --
    /// that is what makes it a parser rather than a regex -- so this happens when the
    /// file is loaded and never while a row is being drawn.
    fn new(file: &SourceFile) -> Highlighted {
        let rope = Rope::from_str(file.text());
        let theme = palette().syntax();

        let mut highlighter = SyntaxHighlighter::new();
        // A language of `None` -- an extension no grammar here parses -- is not a
        // failure: the highlighter then hands back one plain span per line, in the
        // theme's text colour, and the pane renders exactly as it would without any of
        // this. A highlights query that will not compile lands in the same place.
        highlighter.set_language(language(file.path()).as_ref(), &theme);

        let mut blocks = SyntaxBlocks::default();
        highlighter.parse(&rope, &mut blocks, None, &theme);

        let lines = blocks
            .len()
            .saturating_sub(usize::from(file.text().ends_with('\n')));

        Highlighted {
            rope,
            blocks,
            lines,
        }
    }
}

/// The tree-sitter grammar to parse a file with, chosen by extension.
///
/// `freya-code-editor` ships no grammars on purpose, so these are the app's own
/// dependencies, pinned against the `tree-sitter` its highlighter is built on. `.h` goes
/// to C rather than C++ because that is what it is more often; a header the C grammar
/// misparses is coloured oddly, never dropped.
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
        _ => return None,
    };

    Some(EditorLanguage::new(language, query))
}

/// Every file highlighted so far.
///
/// A second cache behind `source`'s, and a `static` for the same reason: parsing a file
/// is the expensive half of showing it, the pane asks again on every render, and a
/// failure needs no entry here because `source::load` already remembers its own.
///
/// What is cached is not just the parse: `SyntaxBlocks` holds a `Color` per span, resolved
/// against `palette().syntax()` when the file was loaded, so an entry here is spans in the
/// palette that was current at the time. **A theme switch therefore has to empty this
/// map** -- the entries are not stale, they are the wrong theme, and nothing else in the
/// app would repaint them, a `SyntaxBlocks` being the one thing here a re-render does not
/// rebuild. That clear is [`set_appearance`], which is the only way the appearance can
/// change at all, so it cannot be routed around by a later call site. Re-highlighting
/// every open file is what a switch costs, which is why the parse belongs where it is
/// rather than in `source::load`: `source`'s cache of the *text* survives it.
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

    // Read and parsed outside the lock, for the reason `source::load` does the same: this
    // is the slow step, and a racing caller's copy costs an allocation rather than a wait.
    // The `SourceFile` itself is not kept: the rope holds the text and the chip above the
    // pane holds the path, and `source`'s own cache is what keeps a second read from
    // touching the disk.
    let file = Arc::new(Highlighted::new(&*source::load(path)?));

    Some(SourceText(
        highlighted()
            .entry(path.to_path_buf())
            .or_insert(file)
            .clone(),
    ))
}

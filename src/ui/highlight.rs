//! Source files as the reader sees them: read off disk, parsed and coloured once, since
//! the highlighter is stateful across lines and cannot be asked about one row at a time.
//!
//! **None of that is the UI thread's.** Reading a file and parsing it are the two things a
//! source pane costs, and both used to run in `render`, so the frame that first drew a
//! file paid for them -- which is what a reader felt between picking a file out of Ctrl+P
//! and seeing it. [`use_source_reading`] moves them onto a worker of the app's own, the
//! shape `use_analysis` has: one thread for the app's lifetime, a channel drained to its
//! newest question, and a pane that draws what it has until the answer lands.
//!
//! **A worker of its own and not the analysis one**, which is the seconds of DWARF work a
//! click costs (`agents/Worker.md`): a file queued behind that would arrive long after the
//! tab it belongs to, and the two questions a source document opens with -- its text, and
//! which of its lines have code -- are then asked at once rather than one behind the other.
//!
//! **The answer is the cache and the state is the knock on the door.** [`HIGHLIGHTED`] is
//! where a parse lands, misses included; [`Sourced`] carries the file the pane wants and a
//! count of the answers, since nothing re-renders for a write to a `static`. That is what
//! keeps a file the reader has already seen instant: the pane finds it in the cache as it
//! renders, with no question asked and no frame lost.

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
///
/// All of it crosses from the worker thread, which the cache below has always proved it
/// can: a `static Mutex<HashMap<_, Arc<Highlighted>>>` is `Sync` only if this is `Send`
/// and `Sync`, so the compiler has been checking that since the cache was written.
pub(crate) struct Highlighted {
    /// The file as it was read: what the stale-source check compares its digests against,
    /// held here so that nothing in a render asks [`source::load`] for it.
    pub(crate) file: Arc<SourceFile>,
    /// The appearance the spans below were resolved in. A `SyntaxBlocks` holds a `Color`
    /// per span and not a name for one, so an entry parsed in the other theme is not
    /// stale but *wrong*, and this is what says so.
    appearance: Appearance,
    pub(crate) rope: Rope,
    pub(crate) blocks: SyntaxBlocks,
    /// How many rows the pane draws, which is *not* `blocks.len()`.
    pub(crate) lines: usize,
    /// Every function in the file, outer before inner, for a row to say which one it is
    /// a line of. Empty for a file no grammar parses.
    pub(crate) functions: Vec<Function>,
}

impl Highlighted {
    /// Parse `file` in `appearance`'s colours. The expensive half of the reader's work,
    /// and off the UI thread: [`colours`] is handed the theme where [`palette`] would ask
    /// the state for it.
    fn new(file: Arc<SourceFile>, appearance: Appearance) -> Highlighted {
        let rope = Rope::from_str(file.text());
        let theme = colours(appearance).syntax();
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

        // The function spans, which the language decides and not this
        // (`source::Language::functions`): a scanner of its own for Rust, a second parse
        // with the same grammar for C and C++, nothing for the rest. Milliseconds, once
        // per file, beside the highlighting and on the same thread.
        let functions = source::Language::of(file.path())
            .map_or_else(Vec::new, |language| language.functions(file.text()));

        Highlighted {
            file,
            appearance,
            rope,
            blocks,
            lines,
            functions,
        }
    }
}

/// The tree-sitter grammar to parse a file with, where there is one: what
/// [`source::Language::grammar`] answers, in freya's type.
///
/// The wrapping is the whole of it. Which grammar a file gets is a per-language fact and
/// is decided in `source.rs` with the rest of them; `EditorLanguage` is the editor's, so
/// putting one together is the one part that has to be up here.
pub(crate) fn language(path: &Path) -> Option<EditorLanguage> {
    let (grammar, query) = source::Language::of(path)?.grammar()?;
    Some(EditorLanguage::new(grammar, query))
}

/// Every file read so far and what came back, `None` included.
///
/// **The misses are cached for the reason `source::CACHE`'s are**: a path out of debug
/// info that is not on this machine is answered as often as it is drawn, and a miss that
/// was not written down would be a question asked again on every render of the pane.
///
/// A theme switch leaves this alone. Each entry says which appearance it was parsed in,
/// which is what [`Sourced::pending`] reads to have it read again -- where a clear would
/// leave the panes with nothing to draw until the reader had caught up.
static HIGHLIGHTED: LazyLock<Mutex<HashMap<PathBuf, Option<Arc<Highlighted>>>>> =
    LazyLock::new(Mutex::default);

pub(crate) fn highlighted() -> MutexGuard<'static, HashMap<PathBuf, Option<Arc<Highlighted>>>> {
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
///
/// The reading is a thread's, so a read that began before this and lands after it would
/// put back what was just forgotten. `source::forgotten_since` is what says it happened,
/// and [`read`] asks it before filing anything.
pub(crate) fn forget_source_under(root: &Path) {
    highlighted().retain(|path, _| !path.starts_with(root));
    source::forget_under(root);
}

/// What the Source pane is drawing: the file, read and parsed, or the two ways it has
/// nothing.
pub(crate) enum Drawing {
    Text(SourceText),
    /// Read, and there is nothing there to show: missing, unreadable, not a file, or past
    /// `source::MAX_SIZE`.
    Missing,
    /// Not read yet. The beat between a file being asked for and the answer landing, in
    /// which the pane draws its own background and no message: tens of milliseconds for
    /// an ordinary file, and a sentence that flashed up in that would say less than the
    /// blank does.
    Waiting,
}

/// One question for the reader: which file, and which appearance its colours are to be
/// resolved in.
///
/// The appearance is part of the question and not read where the answer is made, because
/// the answer is made on a thread that cannot ask (`ui/palette.rs`).
#[derive(Clone, PartialEq)]
pub(crate) struct SourceAsk {
    pub(crate) file: PathBuf,
    pub(crate) appearance: Appearance,
}

/// The reader, shared through context: the Source pane writes the file it is showing and
/// the worker writes the parse into [`HIGHLIGHTED`].
#[derive(Clone, Copy)]
pub(crate) struct Sourcing(pub(crate) State<Sourced>);

/// What has been asked of the reader and what it has answered.
///
/// There is no field holding the parse: it goes into the cache, where a file the reader
/// has seen before is already sitting. Reading a `static` wakes nothing, so [`answers`]
/// is the write that says to look in it again -- a count and not the file answered for,
/// because one file read twice (forgotten by a build, read afresh) has to be as much of a
/// change as two different ones.
///
/// [`answers`]: Sourced::answers
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Sourced {
    /// The file the Source pane is showing, written by it. One file, because one is
    /// drawn; a pane that moves to another asks again.
    pub(crate) wanted: Option<Arc<str>>,
    /// How many files the reader has answered for.
    answers: u64,
}

impl Sourced {
    /// What the pane draws for `file`.
    ///
    /// **Reading this is what subscribes the pane to the reader**, which is the whole of
    /// why it is a method: the answer itself comes out of the cache, and nothing renders
    /// again for a write to that.
    ///
    /// A parse made in the other appearance is still drawn. It is the file, in colours
    /// half a theme old, for the beat it takes to be read again -- where drawing nothing
    /// would blank every source pane on a theme switch.
    pub(crate) fn drawing(&self, file: &Path) -> Drawing {
        match highlighted().get(file) {
            Some(Some(text)) => Drawing::Text(SourceText(text.clone())),
            Some(None) => Drawing::Missing,
            None => Drawing::Waiting,
        }
    }

    /// The question owed: a file is wanted and the cache has no parse of it in this
    /// appearance.
    ///
    /// A miss answers for every appearance -- a file that is not there is not there in
    /// either theme -- so it is asked about once and not again.
    fn pending(&self, appearance: Appearance) -> Option<SourceAsk> {
        let file = PathBuf::from(&**self.wanted.as_ref()?);
        let owed = match highlighted().get(&file) {
            Some(Some(text)) => text.appearance != appearance,
            Some(None) => false,
            None => true,
        };
        owed.then_some(SourceAsk { file, appearance })
    }
}

/// Read and parse the file `ask` names into [`HIGHLIGHTED`], and hand back what was filed
/// there. **The worker's whole job**, and the one place a file becomes rows.
///
/// A file forgotten while it was being read is read again rather than filed: what is in
/// hand is the file as it was before whatever said it had changed, and filing it would
/// leave the pane drawing the text from before a build for as long as the tab is open.
/// `source::forgotten_since` is what says so, of this file and not of anything at all.
/// Bounded rather than a loop, since a build that kept finishing would hold whoever is
/// waiting on the read; giving up files nothing, and the pane asks again.
pub(crate) fn read(ask: &SourceAsk) -> Option<SourceText> {
    for _ in 0..TRIES {
        let at = source::forgotten();
        let parsed =
            source::load(&ask.file).map(|file| Arc::new(Highlighted::new(file, ask.appearance)));

        let mut cache = highlighted();
        // Asked under the lock the forgetting takes, so a forget is either counted here
        // or has yet to empty anything.
        if !source::forgotten_since(at, &ask.file) {
            cache.insert(ask.file.clone(), parsed.clone());
            return parsed.map(SourceText);
        }
    }
    None
}

/// How many times [`read`] will read one file that is forgotten under it.
const TRIES: usize = 4;

/// Read the files the Source pane asks for on a thread of the app's own. Called once, at
/// the root.
pub(crate) fn use_source_reading(sourced: State<Sourced>) {
    use_source_reading_with(sourced, |ask| {
        read(ask);
    });
}

/// The same, with the reading itself an argument.
///
/// One thread for the app's lifetime and a queue drained to its newest question: a reader
/// going down the file finder's list asks for one file per arrow press and wants only the
/// last, and what they pressed past is dropped before it is started.
///
/// The work is an argument so that a test can hold it still: what the pane draws while a
/// file is being read cannot be asserted against a reader that answers as fast as it is
/// asked.
pub(crate) fn use_source_reading_with(
    sourced: State<Sourced>,
    work: impl Fn(&SourceAsk) + Send + 'static,
) {
    let requests = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<SourceAsk>();
        let (answered, answers) = async_channel::unbounded::<SourceAsk>();

        // A `std::thread` and not a spawned task: this reads a file off disk and parses
        // it, and freya's executor is the UI thread.
        // Named, so that a panic on it says which worker died (`crate::panics`).
        let started = std::thread::Builder::new()
            .name("the source reader".to_owned())
            .spawn(move || {
                while let Ok(ask) = jobs.recv_blocking() {
                    // Everything the reader moved past while the last file was read,
                    // dropped without being started rather than after the fact. One kind
                    // of question, so the newest is the last of them.
                    let ask = std::iter::from_fn(|| jobs.try_recv().ok())
                        .last()
                        .unwrap_or(ask);
                    work(&ask);
                    // A send that fails is the app shutting down.
                    if answered.send_blocking(ask).is_err() {
                        return;
                    }
                }
            });
        if let Err(error) = started {
            log::warn!("the source reader could not be started: {error}");
        }

        spawn(async move {
            let mut sourced = sourced;
            while answers.recv().await.is_ok() {
                // The parse is in the cache; this is the write that has the pane look
                // there again. Bound before the write, the peek being a read.
                let answered = sourced.peek().answers.wrapping_add(1);
                sourced.write().answers = answered;
            }
        });

        requests
    });

    use_source_asking(sourced, move |ask| {
        let _ = requests.try_send(ask);
    });
}

/// Ask for whatever the pane is showing and has not been read: the effect both readers
/// share, the app's one and the tests' own.
fn use_source_asking(sourced: State<Sourced>, ask: impl Fn(SourceAsk) + 'static) {
    use_side_effect(move || {
        // Read and not peeked, both of them: the pane writing the file it moved to is one
        // half of what wakes this, and a theme switch is the other -- the spans carry
        // their colours, so a switch is a file to read again.
        let pending = sourced.read().pending(appearance());
        let Some(pending) = pending else {
            return;
        };
        ask(pending);
    });
}

/// The reader a test mounts: the same effect, answering where it stands rather than on a
/// thread.
///
/// A test that draws a source pane is about what the pane draws and not about where the
/// file was read, and a real worker would have every one of them pumping a channel for an
/// answer that is a millisecond's work. The tests that *are* about the reading mount
/// [`use_source_reading`] and pump.
#[cfg(test)]
pub(crate) fn use_source_reading_now(sourced: State<Sourced>) {
    use_source_asking(sourced, move |ask| {
        read(&ask);
        let mut sourced = sourced;
        let answered = sourced.peek().answers.wrapping_add(1);
        sourced.write().answers = answered;
    });
}

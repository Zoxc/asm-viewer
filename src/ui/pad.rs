//! The scratchpad itself: the model the app holds, the jobs its worker does, and what a
//! build, a run and a stop each mean. The drawing of it is the file beside this one.
//!
//! **One worker thread** ([`use_worker`], `src/ui/worker.rs`), so every pad's directory has
//! a single writer and a save cannot land inside the build that is reading what it writes.
//! **Saves supersede, per pad, and builds never do**: a save may be dropped only for a job
//! that writes the same pad's package, or that pad's disk copy goes quietly stale. **A run
//! does not sit on that worker and a stop does not go near it**: a run has no bound on how
//! long it takes, and a stop queued behind a build would arrive after the thing it was meant
//! to interrupt.
//!
//! **Everything here is per pad.** [`Pads`] is the table of them and which one is shown;
//! [`PadState`] is one pad's own, and every field of it — what was read, what is being
//! built, which run is going and what it has written — was already about one pad. So a
//! program started in one pad goes on running and goes on writing into **its own** list
//! while another pad is on screen.

use super::*;

/// Every scratchpad the app is holding, and which one is shown. A root context, since a
/// dock tab that is not the active one in its panel is unmounted and neither the buffer
/// the reader is typing into nor a program they started can live there.
#[derive(Clone, Copy)]
pub(crate) struct Pad(pub(crate) State<Pads>);

/// One `freya-code-editor` buffer per pad. The editor's copy is the live one and
/// `Scratchpad::source` follows it; [`use_scratchpad_with`]'s first effect is that
/// mirroring. A buffer each rather than one replaced on every switch, so a pad comes back
/// with the cursor, the selection and the undo history it was left with. A root context
/// for [`Pad`]'s reason, and because the theme effect has to reach every buffer whether or
/// not the pane is on screen.
#[derive(Clone, Copy)]
pub(crate) struct PadText(pub(crate) State<PadBuffers>);

/// The buffers, keyed by the pad each belongs to. A pad has one from the moment its source
/// has been read until it is deleted.
pub(crate) struct PadBuffers {
    buffers: HashMap<PadId, CodeEditorData>,
    /// Where an edit for a pad with no buffer goes.
    ///
    /// **A buffer can go while the editor drawing it is still taking events.** The editor
    /// is mounted only for a pad [`PadBuffers::holds`], and that is not enough on its own:
    /// freya emits every event of one press against the tree it measured before any of
    /// them ran. The press that confirms a delete lets go of the shown pad's buffer, and
    /// the editor's own global press is still in that batch — still mounted, still writing
    /// through a `Writable` mapped through this table by the deleted pad's id. So the
    /// index has to answer for a pad that has gone, and it answers here.
    ///
    /// **The tail of that batch is the whole of what it is for.** Nothing draws this
    /// buffer: it has no lines, and a row asked to draw one panics inside freya. What
    /// keeps a render off it is that the editor is keyed by its pad
    /// (`pad_view::SourceEditor`), so a pad change takes the editor down rather than
    /// leaving its rows mapped through this table by an id it no longer has.
    gone: CodeEditorData,
}

impl Default for PadBuffers {
    fn default() -> PadBuffers {
        PadBuffers {
            buffers: HashMap::new(),
            gone: CodeEditorData::new(Rope::new(), None::<EditorLanguage>),
        }
    }
}

impl PadBuffers {
    pub(crate) fn holds(&self, pad: &PadId) -> bool {
        self.buffers.contains_key(pad)
    }

    /// The buffer for `pad`, or [`PadBuffers::gone`] if it has none.
    pub(crate) fn get(&self, pad: &PadId) -> &CodeEditorData {
        self.buffers.get(pad).unwrap_or(&self.gone)
    }

    pub(crate) fn get_mut(&mut self, pad: &PadId) -> &mut CodeEditorData {
        let PadBuffers { buffers, gone } = self;
        buffers.get_mut(pad).unwrap_or(gone)
    }

    fn put(&mut self, pad: PadId, editor: CodeEditorData) {
        self.buffers.insert(pad, editor);
    }

    /// Let go of a deleted pad's buffer. The one thing that ever takes one away: a pad is
    /// otherwise in this table from the moment its source arrives until the app stops.
    fn forget(&mut self, pad: &PadId) {
        self.buffers.remove(pad);
    }

    fn theme(&mut self) {
        for editor in self.buffers.values_mut() {
            editor.set_theme(palette().syntax());
            editor.parse();
        }
    }
}

/// The way to ask the scratchpad's worker for something, shared through context.
///
/// Two senders: `jobs` is one message per press and unbounded; `events` is what a running
/// program is saying and is bounded, which is the app's half of the backpressure -- a full
/// channel blocks the pipe thread, which blocks the program. One event channel for the
/// app rather than one per pad, so an event says which pad it is about.
#[derive(Clone)]
pub(crate) struct PadJobs {
    jobs: Requests<PadJob>,
    events: async_channel::Sender<(PadId, u64, RunEvent)>,
}

/// Every pad the app is holding, and which of them the pane draws.
///
/// A pad is in `pads` from the moment it is first shown and leaves only when the reader
/// deletes it: it may have a program running in it, and it is holding the reader's own
/// source until the worker says the disk has it.
#[derive(Clone)]
pub(crate) struct Pads {
    /// The pads there are, in the order the panel draws them: most recently shown first,
    /// then the ones the order on disk does not name. Answered by the worker; until then it
    /// is the one pad the app boots holding.
    pub(crate) order: PadOrder,
    /// Whether the worker has said what pads there are. [`PadState::opened`]'s rule one
    /// level up: the order is not written back before it has been read.
    listed: bool,
    shown: PadId,
    pads: HashMap<PadId, PadState>,
    /// Why the last New or Delete did not happen, as the sentence the panel draws, or
    /// `None`. Here rather than in the view because a dock tab that is not the active one
    /// is unmounted, and an answer that arrived while the reader was elsewhere still has to
    /// be there when they come back.
    pub(crate) refused: Option<String>,
    /// Which pad the reader is being asked about deleting, or `None`. **A delete is the one
    /// operation here that destroys their own source**, so it is a question and not a
    /// press, and the question is held beside the pads for `refused`'s reason.
    pub(crate) confirming: Option<PadId>,
}

impl Default for Pads {
    /// What the app boots holding: the one pad a first run opens, before the worker has
    /// said whether there are others or whether that one has ever been written.
    fn default() -> Pads {
        let scratchpad = Scratchpad::default();
        let shown = scratchpad.id().clone();
        let mut order = PadOrder::default();
        order.touch(shown.clone());
        Pads {
            order,
            listed: false,
            refused: None,
            confirming: None,
            pads: HashMap::from([(shown.clone(), PadState::of(scratchpad))]),
            shown,
        }
    }
}

impl Pads {
    pub(crate) fn shown(&self) -> &PadId {
        &self.shown
    }

    /// The pad the pane draws. Never absent: [`Pads::show`] puts a state in the table
    /// before it names one, so no caller has to answer for a pad that is not there.
    pub(crate) fn state(&self) -> &PadState {
        &self.pads[&self.shown]
    }

    pub(crate) fn get(&self, pad: &PadId) -> Option<&PadState> {
        self.pads.get(pad)
    }

    fn get_mut(&mut self, pad: &PadId) -> Option<&mut PadState> {
        self.pads.get_mut(pad)
    }

    pub(crate) fn state_mut(&mut self) -> &mut PadState {
        let shown = self.shown.clone();
        self.pads.get_mut(&shown).expect("the shown pad")
    }

    /// Hold a pad the listing named, so the panel can draw a name for one that has never
    /// been shown. The name is the one out of that pad's own package, read when the list
    /// was asked for; everything else about it arrives when it is opened. A pad already
    /// open keeps its own name, that being the live one and this a snapshot.
    fn hold(&mut self, listed: &PadListing) {
        let state = self
            .pads
            .entry(listed.id.clone())
            .or_insert_with(|| PadState::of(Scratchpad::of(listed.id.clone())));
        if !state.opened() {
            state.scratchpad.name = listed.name.clone();
        }
    }

    /// Draw `pad` from now on, holding a state for it if this is the first time, and moving
    /// it to the front of the order the panel draws — the same `touch` the file goes
    /// through on the worker, so the two cannot say different things.
    fn show(&mut self, pad: PadId) {
        self.pads
            .entry(pad.clone())
            .or_insert_with(|| PadState::of(Scratchpad::of(pad.clone())));
        self.order.touch(pad.clone());
        self.shown = pad;
    }

    /// The pads the disk has, as the worker read them. Answers with the pad to open,
    /// which is the front of the order: what a restart comes back to is the pad the reader
    /// was last in.
    ///
    /// An empty answer keeps the one row the app booted with, which is the pad a first run
    /// is about to open; anything else replaces it outright, that pad being a placeholder
    /// and not a pad that exists. The pad the app boots holding is opened like any other --
    /// `opened_in` answers what was handed in when there is nothing there, so the baseline
    /// is seeded and nothing is written until there is something to say.
    fn listed(&mut self, listing: &[PadListing]) -> Scratchpad {
        self.listed = true;
        if !listing.is_empty() {
            self.order = PadOrder::of(listing);
            for listed in listing {
                self.hold(listed);
            }
        }
        if let Some(front) = listing.first() {
            self.show(front.id.clone());
        }
        self.state().scratchpad.clone()
    }

    /// What came of the reader pressing New: the pad to open, or the sentence the panel
    /// draws instead.
    ///
    /// A pad that was made is written already, so there is nothing to read: it is shown
    /// and opened at once, which is what seeds its baseline.
    fn created(&mut self, made: Result<Scratchpad, Failure>) -> Option<Scratchpad> {
        match made {
            Ok(scratchpad) => {
                self.show(scratchpad.id().clone());
                self.state_mut().scratchpad = scratchpad.clone();
                Some(scratchpad)
            }
            Err(failure) => {
                self.refused = Some(format!("Not made: {failure}"));
                None
            }
        }
    }

    /// What came of a delete: why the package is still there, or nothing where it is gone.
    fn deleted(&mut self, failure: Option<Failure>) {
        self.refused = failure.map(|failure| format!("Not deleted: {failure}"));
    }

    /// The pad the worker read, and the program its last build made. This is what seeds
    /// the pad's baseline; it answers whether it took the read, so the caller makes the
    /// buffer only then.
    ///
    /// **A pad that is open is read once and never again.** A pad shown, left and shown
    /// again before its first answer arrived was asked for twice, `show_pad` going by
    /// [`PadState::opened`] and the baseline behind it being what this seeds. The second
    /// answer is the disk as it was read *before* any save of what has been typed since,
    /// so taking it would put the older text back on screen and make it the baseline --
    /// leaving the disk ahead of the screen with no save owing, until the next keystroke
    /// wrote the older text over it.
    fn opened(&mut self, scratchpad: &Scratchpad, program: Option<Program>) -> bool {
        let already = self.get(scratchpad.id()).is_some_and(PadState::opened);
        if already {
            return false;
        }
        if let Some(state) = self.get_mut(scratchpad.id()) {
            state.scratchpad = scratchpad.clone();
            // The baseline, seeded by the answer and nowhere else: what the disk holds is
            // what the worker just read off it.
            state.disk = Some(scratchpad.clone());
            state.program = program;
        }
        true
    }

    /// A pad that could not be read, and why. It is left with no baseline, so nothing
    /// here is ever written back: the reason is all the app does with it.
    fn unopened(&mut self, name: &PadId, failure: Failure) {
        if let Some(state) = self.get_mut(name) {
            state.unsaved = Some(failure);
        }
    }

    /// What came of a save: why the package could not be written, or nothing where it was.
    fn saved(&mut self, name: &PadId, failure: Option<Failure>) {
        if let Some(state) = self.get_mut(name) {
            state.unsaved = failure;
        }
    }

    /// What `name` owes the disk, with its baseline moved to it, or `None` where the disk
    /// already has what is on screen.
    ///
    /// The one comparison, with two callers: the save effect, for the pad being typed
    /// into, and a switch, for the pad being left. The baseline moves to what is handed
    /// back, so a reader who changes a row and changes it back writes again. Nothing is
    /// owed by a pad whose disk copy has not been read yet ([`PadState::opened`]).
    ///
    /// **Ask [`PadState::unsaved`] first, under `peek`.** This takes `&mut Pads` and the
    /// save effect reads `Pads` to subscribe, so a guard taken for a keystroke that
    /// changed nothing -- a bare cursor move -- would wake that effect with its own write
    /// and never let go of the thread. See [`save_if_changed`].
    fn unsaved_change(&mut self, name: &PadId) -> Option<Scratchpad> {
        let state = self.get_mut(name)?;
        let scratchpad = state.unsaved()?.clone();
        state.disk = Some(scratchpad.clone());
        Some(scratchpad)
    }

    /// The build the worker ran and the program it made, read. Answers with the pad's
    /// directory when it is one whose source the app has read, since a build writes the
    /// package on its way and what a pane has read of it is the version before.
    ///
    /// A pad that asked for no build takes nothing: its id was handed out again after a
    /// delete -- [`Pads::forget`] comes back to the default pad -- and the answer belongs
    /// to the one that has gone.
    fn built(
        &mut self,
        name: &PadId,
        build: Build,
        program: Option<Program>,
        store: Option<&Store>,
    ) -> Option<PathBuf> {
        let state = self.get_mut(name).filter(|state| state.building)?;
        state.building = false;
        // What the build made, written into the package so a later run opens the pad on
        // its program rather than on nothing. Only a build that produced one replaces it,
        // which is what leaves a failed build showing the program before it -- and what
        // keeps the package naming an artifact that is still there.
        if let (Some(executable), Some(program)) = (build.executable(), &program) {
            state.scratchpad.built = Some(crate::scratchpad::Built {
                path: executable.to_path_buf(),
                digest: program.built_from.clone(),
            });
        }
        state.built = Some(build);
        if program.is_some() {
            state.program = program;
        }
        // A build writes the package on its way.
        if !matches!(
            state.built,
            Some(Build::Unavailable(Failure::Dependencies(_)))
        ) {
            state.unsaved = None;
        }
        store.map(|store| state.scratchpad.directory(store))
    }

    /// The handle to a started program, or why there is none.
    ///
    /// **A handle for a run the reader has already left is stopped here and nowhere
    /// else**, this being the first moment anything in the app is holding it: dropping it
    /// would leave a process running that nothing could ever name again.
    fn started(&mut self, name: &PadId, run: u64, started: Result<process::Handle, Failure>) {
        let state = self.get_mut(name);
        let mine = state
            .as_ref()
            .is_some_and(|state| state.run == run && matches!(state.run_state, RunState::Starting));
        match (started, state) {
            (Ok(running), Some(state)) if mine => state.run_state = RunState::Going(running),
            (Ok(running), _) => running.stop(),
            (Err(failure), Some(state)) if mine => {
                state.run_state = RunState::Over(Ended::Failed(failure.to_string()))
            }
            (Err(_), _) => {}
        }
    }

    /// Let go of a deleted pad: out of the table -- its save baseline with it, so an id
    /// handed out again is read before it is written, exactly as a pad the app has never
    /// seen is -- out of the order, and off the screen if it was the one being drawn.
    /// Answers with the pad to read, when what takes its place has never been shown.
    ///
    /// **There is always a pad to show**, which is what keeps [`Pads::state`] free of an
    /// `Option`. The next one in the order takes over; when the last pad goes, the table
    /// comes back to what a first run holds -- the default pad, opened like any other, so
    /// nothing is written until something is typed into it.
    fn forget(&mut self, name: &PadId) -> Option<Scratchpad> {
        self.pads.remove(name);
        self.order.forget(name);
        if &self.shown != name {
            return None;
        }

        let next = match self.order.first() {
            Some(next) => next.clone(),
            // The last pad. `show` holds a state for the default one, which is what
            // `Pads::default` starts with.
            None => Scratchpad::default().id().clone(),
        };
        self.show(next);
        (!self.state().opened()).then(|| self.state().scratchpad.clone())
    }
}

/// How wide the editor's side of the Scratchpad's split is, and its panels' own context.
///
/// At the root for `SplitRatio`'s reason (`agents/UI.md`): a `ResizablePanel` registers at
/// its `initial_size` in a `use_hook` and takes its entry out again on unmount, so a number
/// kept in the page would come back at 50/50 every time the reader looked at another tab.
/// **Not** `SplitRatio` itself: two containers sharing one context would carry the handle
/// across a switch between a document and this page, and the pad's drag would be written
/// into the project's session -- where a pad, which is outside every project, has no
/// business being. Saved nowhere for that same reason.
#[derive(Clone, Copy)]
pub(crate) struct PadSplit(pub(crate) State<f32>);

/// The context those two panels register into. See [`PadSplit`].
#[derive(Clone, Copy)]
pub(crate) struct PadSplits(pub(crate) State<ResizableContext>);

/// Whether the Scratchpad's listing is up, which its toggle writes. One flag and not one
/// per pad: the reader is arranging the window rather than saying something about a pad.
#[derive(Clone, Copy)]
pub(crate) struct PadFollows(pub(crate) State<bool>);

/// The program a pad's build made, read.
///
/// **The pad's own and not the project's.** It is deliberately not in `Objects`, so it is
/// in neither the Objects panel nor the paths a project saves: a pad lives beside
/// `projects/` rather than inside one, and its program lives where the pad does. Nothing
/// but the pad's own pane ever asks it anything, so nothing can open a tab into it and a
/// rebuild costs the reader none (`agents/Scratchpad.md`).
#[derive(Clone)]
pub(crate) struct Program {
    /// The parsed image. An `Arc<Object>` holds the whole file's bytes, so a pad holds its
    /// program's until the next build replaces it or the pad is deleted.
    pub(crate) object: Arc<Object>,
    /// The string **this program's** debug info spells the pad's own source with, which is
    /// the only spelling anything may compare against: every question about a file in this
    /// app is matched exactly, on the name the debug info said. `None` for a program whose
    /// debug info names no `src/main.rs` -- one built without debug info, or with its paths
    /// remapped -- which is a listing nothing can drive.
    pub(crate) file: Option<Arc<str>>,
    /// What this build was of, as the digest the package keeps beside the artifact
    /// ([`PadState::out_of_date`]). Taken from the scratchpad the **job** carried and never
    /// from what is on screen when the answer lands, so a build the reader typed during
    /// says it is out of date the moment it arrives -- and a digest rather than the value
    /// itself, so a program read back in a later run answers the same question the same
    /// way.
    built_from: String,
    /// Where the listing opens: the lowest placed address the pad's own code sits at, so
    /// what the reader scrolls down through from there is their own code before the
    /// runtime's. `None` with no `file`, and for a file that produced no code at all.
    pub(crate) opening: Option<u64>,
}

/// One pad: everything the Scratchpad pane draws about it.
#[derive(Clone, Default)]
pub(crate) struct PadState {
    pub(crate) scratchpad: Scratchpad,
    /// What the worker last read off the disk or was last handed for this pad, and so
    /// what its package says. `None` until that pad's own answer lands, which is what
    /// [`PadState::opened`] asks: the app boots holding [`Scratchpad::default`] and the
    /// reader's own source arrives a thread later, so a baseline seeded before then
    /// would make their scratchpad look like a change to write the default back over.
    disk: Option<Scratchpad>,
    /// Whether a build is running, which is the whole of "two builds cannot be started at
    /// once": a second job queued behind the first would build bytes the reader has since
    /// changed.
    pub(crate) building: bool,
    /// What the last build came back with. Not remembered across runs: it describes bytes
    /// the next `cargo build` will replace.
    pub(crate) built: Option<Build>,
    /// The program the last build made, read. Kept across a build that **failed**: only a
    /// build that produced one replaces it, which is what leaves the pane showing the
    /// program before a build that would not compile.
    pub(crate) program: Option<Program>,
    /// Why the package on disk is not what is on screen, or `None` when it is.
    /// [`Scratchpad::write_to`] refuses outright for a bad row, so a bad row stops the
    /// *source* being written too and the pane has to say so.
    pub(crate) unsaved: Option<Failure>,
    /// Which run the arriving output belongs to, counted up by [`request_run`].
    ///
    /// **Events carry a run number** where `use_analysis` compares identities instead: the
    /// thing an event is about is the process, which does not exist until the worker has
    /// forked, so there is nothing to compare against. Stopping one program and starting
    /// another is one keypress, and untagged the first one's last lines and its `Ended`
    /// would land in the second's output.
    run: u64,
    pub(crate) run_state: RunState,
    /// What the running program has written. Behind an `Arc` because this struct is cloned
    /// on every render and the deque under it holds thousands of lines.
    pub(crate) output: Arc<RunOutput>,
}

/// Where the program the reader started has got to.
///
/// Four states and not a `bool`, because [`RunState::Starting`] is the one a `bool` would
/// get wrong: a fork is not instant, and a Stop pressed in that window has to be
/// remembered rather than dropped.
#[derive(Clone, Default)]
pub(crate) enum RunState {
    #[default]
    Idle,
    /// Asked for; the worker has not come back with a handle yet.
    Starting,
    Going(process::Handle),
    Over(Ended),
}

impl PadState {
    /// A pad nothing is known about yet: the scratchpad it will be, and none of the rest.
    fn of(scratchpad: Scratchpad) -> PadState {
        PadState {
            scratchpad,
            ..PadState::default()
        }
    }

    /// Whether the worker has yet said what is on disk. **Nothing is saved until this is
    /// true**, which is the whole of what an absent baseline means: nothing to compare
    /// against, so nothing to write.
    pub(crate) fn opened(&self) -> bool {
        self.disk.is_some()
    }

    /// What is on screen, where the disk has been read and has fallen behind it; `None`
    /// for a pad the disk already has and for one never read. Asked under `peek` before
    /// [`Pads::unsaved_change`] takes a guard.
    fn unsaved(&self) -> Option<&Scratchpad> {
        let disk = self.disk.as_ref()?;
        (disk != &self.scratchpad).then_some(&self.scratchpad)
    }

    /// What the compiler said about the last build.
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        self.built
            .as_ref()
            .map(Build::diagnostics)
            .unwrap_or_default()
    }

    /// cargo's own words, when they are about the dependency rows.
    pub(crate) fn refusal(&self) -> Option<&str> {
        self.built.as_ref().and_then(Build::refusal)
    }

    /// The one line over the pane saying where the last build got to. The Project view's
    /// is the same line ([`Builds::verdict`]).
    pub(crate) fn verdict(&self) -> Option<Verdict> {
        match self.building {
            true => Some(Verdict::plain(cargo::BUILDING)),
            false => self.built.as_ref().map(Build::verdict),
        }
    }

    /// What the last build made, and so what there is to run.
    pub(crate) fn executable(&self) -> Option<&Path> {
        self.built.as_ref().and_then(Build::executable)
    }

    /// Whether what is on screen has moved on from the program that is: an edit since the
    /// build, which the pane says over the listing.
    ///
    /// `false` for a pad with no program -- there is nothing to be out of date -- and for a
    /// build that **failed** since, whose own program is still the one before it and still
    /// describes the source that made it.
    pub(crate) fn out_of_date(&self) -> bool {
        self.program
            .as_ref()
            .is_some_and(|program| program.built_from != self.scratchpad.compiled().digest())
    }

    /// Whether a program is on its way up or already going.
    pub(crate) fn is_running(&self) -> bool {
        matches!(self.run_state, RunState::Starting | RunState::Going(_))
    }

    /// The line over the output, saying where the run got to. `None` before anything has
    /// been run, which is what leaves the pane out.
    pub(crate) fn run_verdict(&self) -> Option<Verdict> {
        let dropped = match self.output.dropped() {
            0 => String::new(),
            1 => " (1 earlier line dropped)".to_owned(),
            count => format!(" ({count} earlier lines dropped)"),
        };

        let (text, bad) = match &self.run_state {
            RunState::Idle => return None,
            RunState::Starting => ("Starting...".to_owned(), false),
            RunState::Going(_) => ("Running".to_owned(), false),
            RunState::Over(Ended::Exited(Some(0))) => ("Exited".to_owned(), false),
            RunState::Over(Ended::Exited(Some(code))) => (format!("Exited with {code}"), true),
            // A signal on Unix; the number is not portable and the app has no use for it.
            RunState::Over(Ended::Exited(None)) => ("Ended with no exit code".to_owned(), true),
            RunState::Over(Ended::Stopped) => ("Stopped".to_owned(), false),
            RunState::Over(Ended::Failed(error)) => (format!("Could not run it: {error}"), true),
        };

        Some(Verdict {
            text: format!("{text}{dropped}"),
            bad,
        })
    }
}

/// What the scratchpad's worker thread is asked for. Each carries the whole scratchpad, so
/// nothing the worker touches can change under it while it is writing or building.
pub(crate) enum PadJob {
    /// What pads there are, asked once on mount. Disk work like the rest of these, and on
    /// the same thread for the same reason: it is that thread's directory to read.
    List,
    /// Make a pad nobody has named yet and write its package.
    New,
    /// Take a pad's package off the disk. An id and not a whole scratchpad like the rest:
    /// the app has already let the pad go, and what is deleted is the directory the id
    /// names.
    Delete(PadId),
    Open(Scratchpad),
    Save(Scratchpad),
    Build(Scratchpad),
    /// Start what the last build made. It goes to the worker because it *forks* and
    /// because the directory it hands the program is that thread's, not because it blocks.
    Run {
        /// Which run this is, so a handle arriving after the reader has moved on can be
        /// recognised and stopped rather than stored. See [`PadState::run`].
        run: u64,
        scratchpad: Scratchpad,
        executable: PathBuf,
        /// Where each line goes as it is written. A boxed callback rather than a channel,
        /// so `scratchpad.rs` never learns what the app carries its values in.
        emit: Box<dyn FnMut(RunEvent) + Send>,
    },
}

impl PadJob {
    /// Which pad this job is about, or `None` for the two that are about the set of them
    /// rather than about one. Every job that names a pad carries the whole scratchpad, so
    /// the name is already in hand; what reads it is the supersede rule, which is per pad.
    pub(crate) fn pad(&self) -> Option<&PadId> {
        match self {
            PadJob::List | PadJob::New => None,
            PadJob::Delete(pad) => Some(pad),
            PadJob::Open(scratchpad)
            | PadJob::Save(scratchpad)
            | PadJob::Build(scratchpad)
            | PadJob::Run { scratchpad, .. } => Some(scratchpad.id()),
        }
    }
}

/// What it answers with.
///
/// Every answer says which pad it is about, where a job says it by carrying the whole
/// scratchpad. It has to: an answer can land long after the reader has moved to another
/// pad, and it belongs to the pad that asked and to no other. The exception is
/// [`PadAnswer::Deleted`], which is about a pad the app let go of before it asked.
pub(crate) enum PadAnswer {
    /// The pads there are, in the order the panel draws them, each with the name out of
    /// its own package — which is what lets the panel draw a pad nothing has opened.
    Listed(Vec<PadListing>),
    /// The pad that was made, or why there is none.
    Created(Result<Scratchpad, Failure>),
    /// Why the package is still on the disk, or `None` when it is gone. Nothing waits for
    /// this: the app let the pad go when the reader said to.
    Deleted(Option<Failure>),
    Opened {
        scratchpad: Scratchpad,
        /// The program the package says the last build made, read back off the disk.
        /// [`None`] for a pad never built, one whose artifact has gone, and one built by
        /// a version of this app that wrote nothing down.
        program: Option<Program>,
    },
    /// A pad that could not be read, and why.
    ///
    /// **It is left unopened**, which is the whole of the answer: no buffer is made for
    /// it, its baseline is never seeded, and [`save_if_changed`] steps over a pad that is
    /// not open. So a package this module cannot read stays on the disk as it is instead
    /// of being written over by the pad the app boots holding.
    Unopened { pad: PadId, failure: Failure },
    /// Why the package could not be written, or `None` when it was.
    Saved {
        pad: PadId,
        failure: Option<Failure>,
    },
    Built {
        pad: PadId,
        build: Build,
        /// What the build made, read on the way back. `None` for a build that made
        /// nothing, and for one whose artifact could not be parsed.
        program: Option<Program>,
    },
    /// The handle to a started program, or why there is none. What the program then *says*
    /// arrives on the other channel.
    Started {
        pad: PadId,
        run: u64,
        started: Result<process::Handle, Failure>,
    },
}

/// The program a build made, read: the parsed image and what it was built from.
///
/// The scratchpad worker's, because that thread already owns the pad's directory --
/// `target/` is inside it -- so the single writer of what cargo wrote is also its single
/// reader. Blocking, like everything else on it.
///
/// `pop` and not `first`: `open_files` hands back an archive's members before the file
/// itself, and what a build made is one executable, so the last is the image either way.
pub(crate) fn read_program(executable: &Path, built_from: String) -> Option<Program> {
    let object = analysis::open_files(vec![executable.to_path_buf()]).pop()?;
    // The pad's own file as **this program** spells it, which is the only spelling
    // anything may compare against: a name in debug info is the file as the compiler was
    // handed it, joined onto the directory the compiler ran in.
    let files = object.source_files();
    let file: Option<Arc<str>> = own_source(files.iter().map(|file| &**file)).map(Arc::from);
    let opening = file
        .as_ref()
        .and_then(|file| compiled::lowest_placed(&object.symbols_from_lines(file, 0..=u32::MAX)));
    Some(Program {
        object,
        built_from,
        file,
        opening,
    })
}

/// The blocking work itself. Split out so [`use_scratchpad_with`] can be handed something
/// that answers without a disk or a compiler.
pub(crate) fn pad_work(job: PadJob) -> PadAnswer {
    // The store is opened here, once per job, rather than handed in: this runs on the
    // worker's own thread, off the end of a channel that carries the job and nothing
    // else. Every arm wants the same directory, and without one there is nowhere to
    // read, write, build or run in.
    let store = Store::open();
    match job {
        PadJob::List => PadAnswer::Listed(
            store
                .as_ref()
                .map(crate::scratchpad::pads)
                .unwrap_or_default(),
        ),
        PadJob::New => PadAnswer::Created(match &store {
            Some(store) => crate::scratchpad::new_pad(store),
            None => Err(Failure::NoDirectory),
        }),
        PadJob::Delete(name) => PadAnswer::Deleted(match &store {
            Some(store) => crate::scratchpad::delete_pad(store, &name).err(),
            None => Some(Failure::NoDirectory),
        }),
        PadJob::Open(scratchpad) => match &store {
            Some(store) => {
                let directory = scratchpad.directory(store);
                let pad = scratchpad.id().clone();
                match scratchpad.opened_in(&directory) {
                    Ok(opened) => {
                        // Opening a pad is what puts it at the front of the order, so the
                        // pad a restart comes back to is the one the reader was last in --
                        // and `touch` answering whether anything moved is what keeps a
                        // startup that reopens the pad already at the front from writing a
                        // file at all. Only a pad that was read: a restart may not come
                        // back to one that will not open.
                        crate::scratchpad::remember(store, opened.id());
                        // What the last build made, if it is still where cargo put it:
                        // read here, on the thread that owns this directory, so a pad
                        // opened in a later run shows its program without being built
                        // again. The digest travels with it, so a pad edited between the
                        // build and the restart still says it is out of date.
                        let program = opened
                            .built
                            .as_ref()
                            .and_then(|built| read_program(&built.path, built.digest.clone()));
                        PadAnswer::Opened {
                            scratchpad: opened,
                            program,
                        }
                    }
                    Err(failure) => PadAnswer::Unopened { pad, failure },
                }
            }
            // Nowhere to have been read from, so what was handed in is what there is.
            None => PadAnswer::Opened {
                scratchpad,
                program: None,
            },
        },
        PadJob::Save(scratchpad) => PadAnswer::Saved {
            pad: scratchpad.id().clone(),
            failure: match &store {
                Some(store) => scratchpad.write_to(&scratchpad.directory(store)).err(),
                None => Some(Failure::NoDirectory),
            },
        },
        PadJob::Build(scratchpad) => {
            let build = match &store {
                Some(store) => scratchpad.build_in(&scratchpad.directory(store)),
                None => Build::Unavailable(Failure::NoDirectory),
            };
            // Read here and not in a job of its own, so what the pane holds and the
            // program it describes cannot disagree: there is no pass in which the pad has
            // an executable it has not read. It is milliseconds against a build's seconds,
            // behind the same flag, so it delays nothing the build was not delaying
            // already.
            let program = build
                .executable()
                .and_then(|executable| read_program(executable, scratchpad.compiled().digest()));
            PadAnswer::Built {
                pad: scratchpad.id().clone(),
                build,
                program,
            }
        }
        PadJob::Run {
            run,
            scratchpad,
            executable,
            emit,
        } => PadAnswer::Started {
            pad: scratchpad.id().clone(),
            run,
            started: match &store {
                Some(store) => run_in(&executable, &scratchpad.directory(store), emit),
                None => Err(Failure::NoDirectory),
            },
        },
    }
}

/// The scratchpad's whole wiring: one worker thread, the editor's text mirrored into the
/// model, the model written out as it changes, and the theme carried into the editor's own
/// syntax blocks. See this file's header for why the worker is one thread, why saves
/// supersede, and why a run and a stop do not go through it.
///
/// The work and the store are handed in, so a test can drive the wiring without writing to
/// the machine's own state directory or waiting on a compiler.
pub(crate) fn use_scratchpad_with(
    mut pad: State<Pads>,
    mut text: State<PadBuffers>,
    store: State<Option<Store>>,
    work: impl Fn(PadJob) -> PadAnswer + Send + 'static,
) -> PadJobs {
    // What a running program is saying, on a channel of the app's own and taken by a task
    // of its own: a program that never ends would otherwise share a loop with every save.
    // Bounded, which is the app's half of the backpressure -- a full channel blocks the
    // pipe thread, which blocks the program. One channel for the app's lifetime rather than
    // one per run, which is what makes the run number on each event both necessary and
    // enough.
    //
    // 512: how many of a running program's lines may sit between the pipe and the pane.
    let emitted = use_hook(move || {
        let (emitted, events) = async_channel::bounded::<(PadId, u64, RunEvent)>(512);
        spawn(async move {
            while let Some(batch) = next_batch(&events).await {
                // Whether anything in the batch is for a pad that is still there and
                // still on the run it names, asked before the guard is taken: a write
                // notifies whether or not it changed anything, so a batch that is all a
                // deleted pad's or a left run's would cost a render for nothing. Bound to
                // a `let` and dropped, the write below being of this state.
                let pads = pad.peek();
                let wanted = batch
                    .iter()
                    .any(|(name, run, _)| pads.get(name).is_some_and(|state| state.run == *run));
                drop(pads);
                if !wanted {
                    continue;
                }

                // Through the guard, so a line lands in the deque the app is holding
                // rather than in a copy of every pad's source, diagnostics and output
                // taken to push it -- and so `Arc::make_mut` copies the lines once per
                // batch, for the pane's own hold on them, rather than twice.
                let mut pads = pad.write();
                for (name, run, event) in batch {
                    // Into the pad the program belongs to, which is very often not the
                    // pad on screen: leaving a pad does not stop what is running in it.
                    let Some(state) = pads.get_mut(&name) else {
                        continue;
                    };
                    // A run the reader has left: not this run's output, not its ending.
                    if run != state.run {
                        continue;
                    }
                    match event {
                        RunEvent::Wrote(line) => Arc::make_mut(&mut state.output).push(line),
                        RunEvent::Ended(ended) => state.run_state = RunState::Over(ended),
                    }
                }
            }
        });
        emitted
    });

    // The worker itself. See this file's header for why it is one thread and why saves
    // supersede; the answers task asks for more work of its own -- a listing names the
    // pad to open, and a pad just made is opened at once -- which is what the taker's
    // second argument is.
    let requests = use_worker(
        "the scratchpad worker",
        // A job the supersede rule may not step over -- a save of another pad -- is
        // handed back rather than dropped, and is done in its turn.
        |job, take, hold| vec![superseded(job, take, hold)],
        move |job| Some(work(job)),
        // Each answer is the state's to take, and what a taken one leaves the task to do
        // -- a pad to open, a buffer to make, a directory whose source is now stale -- is
        // what the method answers with. The writes are through the guard and not by
        // [`write_if`]: what is held is every pad's own source, output and program, and a
        // clone of all of it per answer would be paid for a keystroke at a time.
        move |answer, requests| match answer {
            PadAnswer::Listed(listing) => {
                // Bound out of a statement of its own, so the guard is gone before the
                // send.
                let opening = pad.write().listed(&listing);
                requests.send(PadJob::Open(opening));
            }
            PadAnswer::Created(made) => {
                let opening = pad.write().created(made);
                if let Some(scratchpad) = opening {
                    requests.send(PadJob::Open(scratchpad));
                }
            }
            PadAnswer::Deleted(failure) => {
                pad.write().deleted(failure);
            }
            PadAnswer::Opened {
                scratchpad,
                program,
            } => {
                // Whether the answer is wanted at all is the state's to say
                // ([`Pads::opened`]), which is also what seeds the baseline; the buffer is
                // made only where it was taken, so a second answer for a pad already open
                // changes neither.
                let taken = pad.write().opened(&scratchpad, program);
                if !taken {
                    return;
                }

                // The pad's own buffer, built once when its source arrives: this is the
                // first thing that happens to it, so there is no cursor and no undo
                // history to preserve. A later switch away and back leaves it exactly as
                // it is here.
                let mut editor = CodeEditorData::new(
                    Rope::from_str(&scratchpad.source),
                    language(Path::new(SOURCE_FILE)),
                );
                editor.set_theme(palette().syntax());
                // Without this the editor has no blocks at all and draws no lines:
                // `CodeEditorData::new` never runs the highlighter.
                editor.parse();
                text.write().put(scratchpad.id().clone(), editor);
            }
            PadAnswer::Unopened { pad: name, failure } => {
                pad.write().unopened(&name, failure);
            }
            PadAnswer::Saved { pad: name, failure } => {
                pad.write().saved(&name, failure);
            }
            PadAnswer::Built {
                pad: name,
                build,
                program,
            } => {
                let store_now = store.peek().clone();
                let directory = pad.write().built(&name, build, program, store_now.as_ref());
                // The build wrote the package on its way, to the same `src/main.rs` as
                // last time, so what a pane has read of this pad is the version before
                // it.
                if let Some(directory) = directory {
                    forget_source_under(&directory);
                }
            }
            PadAnswer::Started {
                pad: name,
                run,
                started,
            } => {
                pad.write().started(&name, run, started);
            }
        },
    );

    // How the pane asks for a build. A context, because the button that asks is inside a
    // dockable view that is handed nothing; returned as well, so a test can ask directly.
    let jobs = use_provide_context(move || PadJobs {
        jobs: requests,
        events: emitted,
    });

    // What pads there are, asked for once: `use_hook` runs on mount and never again. The
    // answer says which one to open, so the whole of startup is that one question -- the
    // front of the order, or the pad the app boots holding when there is no order.
    use_hook({
        let jobs = jobs.clone();
        move || jobs.jobs.send(PadJob::List)
    });

    // The editor's text into the model. Reading the buffers subscribes this to every edit;
    // the comparison is what makes a bare cursor move free. The **shown** pad and no other,
    // because it is the only buffer anything can be typed into -- and because a buffer that
    // is not on screen cannot have changed since it was last mirrored.
    use_side_effect(move || {
        let buffers = text.read();
        let shown = pad.peek().shown().clone();
        if !buffers.holds(&shown) {
            return;
        }
        let typed = buffers.get(&shown).rope.to_string();
        drop(buffers);

        let changed = pad.peek().state().scratchpad.source != typed;
        if changed {
            pad.write().state_mut().scratchpad.source = typed;
        }
    });

    // The model onto the disk. The shown pad for the same reason: nothing else can change
    // under a keystroke, and a switch flushes the pad it is leaving on its way out.
    use_side_effect({
        let jobs = jobs.clone();
        move || {
            let pads = pad.read();
            let shown = pads.shown().clone();
            drop(pads);
            save_if_changed(pad, &shown, &jobs);
        }
    });

    // The theme, carried into every editor's own blocks: a `SyntaxBlocks` holds colours
    // already resolved out of the palette, so after a switch the entries are the wrong
    // theme rather than stale, and `set_appearance`'s clear cannot reach inside a
    // `CodeEditorData`. Re-setting the theme and parsing again is what repaints them --
    // every buffer and not only the one on screen, or a pad switched to after a theme
    // change would come back in the theme it was left in.
    use_side_effect_with_deps(&appearance(), move |_: &Appearance| {
        text.write().theme();
    });

    jobs
}

/// Write `name`'s package out if what is on screen has moved on from its baseline
/// ([`Pads::unsaved_change`]).
///
/// **Asked under `peek` and answered under a guard only where there is something to
/// send.** The effect above reads `Pads` to subscribe to every keystroke and this writes
/// it. An effect is a loop that runs and then waits to be notified, so a write of its own
/// makes that wait return at once and the task never yields: a guard taken whatever the
/// answer said would not cost a render but lock the window up.
fn save_if_changed(mut pad: State<Pads>, name: &PadId, jobs: &PadJobs) {
    let owes = pad
        .peek()
        .get(name)
        .is_some_and(|state| state.unsaved().is_some());
    if !owes {
        return;
    }

    let Some(scratchpad) = pad.write().unsaved_change(name) else {
        return;
    };
    jobs.jobs.send(PadJob::Save(scratchpad));
}

/// Draw `name` from now on.
///
/// The pad being left is flushed **through the worker**, so its package is written before
/// the arriving pad's `Open` is even started: the jobs are one ordered queue, and the
/// supersede rule may not step over a save for another pad. Then the pad arrives — read
/// from disk if this is the first time it has been shown, and otherwise simply drawn from
/// what is held, buffer, run and all.
pub(crate) fn show_pad(mut pad: State<Pads>, jobs: &PadJobs, name: PadId) {
    let leaving = pad.peek().shown().clone();
    if leaving == name {
        return;
    }
    save_if_changed(pad, &leaving, jobs);

    let mut pads = pad.write();
    pads.show(name.clone());
    let opened = pads.state().opened();
    let arriving = pads.state().scratchpad.clone();
    drop(pads);

    if !opened {
        jobs.jobs.send(PadJob::Open(arriving));
    }
}

/// Put `pad`'s cursor where the compiler pointed: the line and column of a diagnostic's
/// span, in that pad's own buffer.
///
/// The offset is worked out against the buffer **as it is now** and not against whatever
/// was compiled, so it is always a place in this text and nothing here can be out of range
/// — `chars::offset_of` carries the clamping and the reasoning for it. Any selection is
/// cleared first: `TextSelection::move_to` moves only the far end of a range, so a jump
/// made while something was selected would stretch the selection to the span instead of
/// going there.
///
/// What this does **not** do is scroll the editor to the line. freya's `CodeEditor` keeps
/// its scroll inside its own `CodeEditorData` (`pub(crate)`, with no controller to hand
/// in), so nothing outside that crate can move it — the same objection that kept the
/// component out of the read-only source pane, which was expected never to matter here.
/// So the jump *marks* the line: the cursor's row takes the editor's own current-line
/// background and its number lights in the gutter, which is the whole of what the pane can
/// honestly promise until the editor exposes its scroll.
pub(crate) fn jump_to_span(mut text: State<PadBuffers>, pad: &PadId, span: &cargo::Span) {
    let buffers = text.peek();
    if !buffers.holds(pad) {
        return;
    }
    let source = buffers.get(pad).rope.to_string();
    // Bound to a `let` and dropped before the write below, which is the whole of the
    // guard hazard this repo's headless tests were first written for.
    drop(buffers);

    let offset = chars::offset_of(&source, span.line, span.column);
    let mut buffers = text.write();
    let editor = buffers.get_mut(pad);
    editor.clear_selection();
    editor.move_cursor_to(offset);
}

/// Ask for a pad nobody has named yet. What comes back is shown, this being a deliberate
/// act and not something that happens behind the reader.
pub(crate) fn request_new_pad(jobs: &PadJobs) {
    jobs.jobs.send(PadJob::New);
}

/// Delete `name`: let go of it here, then ask the worker to take its package off the disk.
/// Only ever reached through the pane's confirmation, this being the one operation that
/// destroys what the reader wrote.
///
/// **The app lets go first, and that is the ordering that matters.** Every job for this pad
/// still on the queue runs before the delete, the worker being one ordered thread, so a
/// build in flight finishes against a directory that is still there; what it then answers
/// arrives for a pad the table no longer has and is dropped, which is why the artifact of a
/// build the reader deleted their way out of is not opened.
///
/// Its program is stopped first: the directory it was started in is about to go, and a
/// program left behind by that is one nothing could ever find again. A run still forking is
/// stopped where it lands, its handle arriving for no pad in the table.
pub(crate) fn request_delete_pad(
    mut pad: State<Pads>,
    mut text: State<PadBuffers>,
    jobs: &PadJobs,
    name: PadId,
) {
    stop_run_of(pad, &name);

    let mut pads = pad.write();
    pads.confirming = None;
    let arriving = pads.forget(&name);
    drop(pads);

    text.write().forget(&name);

    jobs.jobs.send(PadJob::Delete(name));
    // Behind the delete, so a pad that has to be read is read after the directory has gone
    // rather than before -- which matters for the one id this can arrive at twice, the
    // default pad the last delete comes back to.
    if let Some(arriving) = arriving {
        jobs.jobs.send(PadJob::Open(arriving));
    }
}

/// The newest of a run of saves of one pad, dropping the ones it has overtaken before any
/// of them is started.
///
/// A save is superseded only by a job for the **same** pad that writes or removes its
/// package: a newer save, a build, which writes the package itself, or a delete, which is
/// about to take the package away and has nothing to want from a write. Everything else
/// goes to `hold` rather than being allowed to drop this save on the floor -- which is what
/// a rule that took whatever was next would do, leaving the package quietly behind what is
/// on screen. That is a job for another pad, which says nothing about this pad's disk copy;
/// and a run or an open of this one, neither of which writes anything. Anything that is not
/// a save supersedes nothing and is handed straight back.
pub(crate) fn superseded(
    job: PadJob,
    mut take: impl FnMut() -> Option<PadJob>,
    mut hold: impl FnMut(PadJob),
) -> PadJob {
    let mut job = job;
    while matches!(job, PadJob::Save(_)) {
        match take() {
            Some(newer)
                if newer.pad() == job.pad()
                    && matches!(
                        newer,
                        PadJob::Save(_) | PadJob::Build(_) | PadJob::Delete(_)
                    ) =>
            {
                job = newer
            }
            Some(newer) => {
                hold(newer);
                break;
            }
            None => break,
        }
    }
    job
}

/// Ask for a build of what is on screen, unless one is already running or the pad has
/// never been read. Both guards are here as well as on the button, so "two builds cannot
/// be started at once" and "nothing is written until the disk has been read" are properties
/// of the request rather than of one control's disabled state.
///
/// A build writes the package before it compiles it, so building a pad this app could not
/// read would put the default manifest and source over the reader's own two files -- the
/// loss leaving such a pad unopened is there to prevent, one deliberate press away.
pub(crate) fn request_build(mut pad: State<Pads>, jobs: &PadJobs) {
    let state = pad.peek().state().clone();
    if state.building || !state.opened() {
        return;
    }

    // A rebuild stops what **this** pad started: cargo is about to write over the very file
    // that process is running. Another pad's program is about another executable and goes
    // on. Editing stops nothing, deliberately.
    stop_run(pad);

    pad.write().state_mut().building = true;
    jobs.jobs.send(PadJob::Build(state.scratchpad));
}

/// Run what the last build made. Nothing happens without an executable, which is why the
/// button is unavailable until a build has succeeded. Whatever was running is stopped
/// first: two generations of output arriving into one list is a pane with no answer to
/// "what is this".
pub(crate) fn request_run(mut pad: State<Pads>, jobs: &PadJobs) {
    let state = pad.peek().state().clone();
    let Some(executable) = state.executable().map(Path::to_path_buf) else {
        return;
    };

    stop_run(pad);

    // The output starts empty and the run is numbered, so everything still on its way from
    // the run before this one is for a number nobody is listening to. The number is this
    // pad's own, which is enough because an event carries the pad beside it.
    let run = state.run + 1;
    let name = state.scratchpad.id().clone();
    let mut pads = pad.write();
    let shown = pads.state_mut();
    shown.run = run;
    shown.run_state = RunState::Starting;
    shown.output = Arc::new(RunOutput::default());
    drop(pads);

    let events = jobs.events.clone();
    jobs.jobs.send(PadJob::Run {
        run,
        scratchpad: state.scratchpad,
        executable,
        // `send_blocking` and not `try_send`: a full channel has to *stop* the thread
        // reading the pipe, which is what puts the brakes on the program itself.
        emit: Box::new(move |event| {
            let _ = events.send_blocking((name.clone(), run, event));
        }),
    });
}

/// Stop the shown pad's program, for real. Only the shown pad has a Stop button, and only
/// the shown pad's own rebuild or next run stops it -- another pad's program is about
/// another executable and is left alone.
pub(crate) fn stop_run(pad: State<Pads>) {
    let shown = pad.peek().shown().clone();
    stop_run_of(pad, &shown);
}

/// The whole of the above for a pad named outright, which a delete needs: the pad whose
/// program has to stop is the one being taken away, and it is very often the shown one but
/// need not be.
///
/// The state is *not* set to `Over` in the `Going` case: the run's own `Ended` event says
/// it, and that is emitted only once the process has been reaped, so the pane says
/// "Stopped" when the program is gone rather than when the button was pressed.
/// `Starting` is the case a `bool` would have lost -- the fork has not come back, so
/// leaving `Starting` behind is what makes the handle unwanted when it arrives.
fn stop_run_of(mut pad: State<Pads>, name: &PadId) {
    let run_state = pad.peek().get(name).map(|state| state.run_state.clone());
    match run_state {
        Some(RunState::Going(running)) => running.stop(),
        Some(RunState::Starting) => {
            let mut pads = pad.write();
            if let Some(state) = pads.get_mut(name) {
                state.run_state = RunState::Over(Ended::Stopped);
            }
        }
        None | Some(RunState::Idle | RunState::Over(_)) => {}
    }
}

#[cfg(test)]
mod tests;

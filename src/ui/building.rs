//! Building the project's own workspace: what the app holds about it, and the one worker
//! thread that runs cargo and edits its manifest.
//!
//! [`use_worker`]'s shape (`src/ui/worker.rs`), for the scratchpad's reasons
//! (`src/ui/pad.rs`): the work is blocking so it goes to a thread of its own, and it is
//! **one** thread so that the project's directory has a single writer — the debug-lines
//! edit cannot land inside the build that is reading the same manifest. It is the one
//! worker of the four that supersedes nothing.
//!
//! The state is a root context and not the Project tab's own, because a tab that is not on
//! screen is unmounted: a build has to survive the reader looking at something else while
//! it runs.

use super::*;

/// What the app holds about building the open project.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Builds {
    /// A build is going. Two cannot: a second would compile what the first is writing.
    pub(crate) building: bool,
    /// The last build, whatever came of it.
    pub(crate) built: Option<cargo::Run>,
    /// The manifest the project's directory holds, which is what cargo would be run over.
    /// `None` is a placeholder and not an error.
    pub(crate) manifest: Option<PathBuf>,
    /// The manifest the profile is read from and written to, when that is **not** the one
    /// above: cargo takes `[profile.*]` from the workspace root alone, so a member's own
    /// file is not where the offer below is taken. `None` when the two are the same, which
    /// is what leaves the row out for a project that is its own workspace.
    pub(crate) profiles: Option<PathBuf>,
    /// Whether the chosen profile carries the line information the source side is drawn
    /// from, as that manifest has it now.
    pub(crate) debug_lines: bool,
    /// Why the last "Turn on" did not take, in the words of whatever refused it. The row
    /// offering it is unchanged either way, so this is the only sign the press did
    /// anything. Cleared by the next read of the manifest.
    pub(crate) edit_refused: Option<String>,
    /// What the build before this one produced. **The set a build replaces**, which is why
    /// it is saved with the session: a binary the reader opened some other way is left
    /// alone, and the build before may have been in another run of the app.
    pub(crate) previous: Vec<PathBuf>,
    /// Of the files [`Builds::diagnostics`] names, the ones the Project view offers as
    /// targets: inside the project's directory, and readable as source. Absolute, as the
    /// view spells them.
    ///
    /// Worked out **on the worker** beside the build ([`openable`]), because deciding it
    /// at the row costs a `stat` per diagnostic per frame and a build says two hundred
    /// things as readily as two.
    pub(crate) sources: HashSet<PathBuf>,
}

impl Builds {
    /// What the last build produced, in the order cargo named them.
    pub(crate) fn artifacts(&self) -> &[cargo::Artifact] {
        match &self.built {
            Some(cargo::Run::Built { artifacts, .. }) => artifacts,
            _ => &[],
        }
    }

    /// What the compiler said about the last build. Warnings on a build that succeeded
    /// and errors on one that did not are the same list to a reader.
    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        self.built
            .as_ref()
            .map(cargo::Run::diagnostics)
            .unwrap_or_default()
    }

    /// Whether `file` is one the pane may offer as a target: [`Builds::sources`] asked,
    /// never the filesystem.
    pub(crate) fn shows(&self, file: &Path) -> bool {
        self.sources.contains(file)
    }

    /// cargo's own words, for the failures said there and nowhere else: a manifest error
    /// and a dependency that does not resolve both arrive with no compiler diagnostic
    /// behind them. Once the compiler has spoken, that same stderr says nothing the list
    /// below does not.
    pub(crate) fn refusal(&self) -> Option<&str> {
        match &self.built {
            Some(cargo::Run::Rejected {
                diagnostics,
                message,
            }) if diagnostics.is_empty() && !message.is_empty() => Some(message),
            Some(cargo::Run::NoCargo(message)) => Some(message),
            _ => None,
        }
    }

    /// What the manifest says, as the worker read it. Whether anything changed, so the
    /// hook writes only then ([`write_if`]).
    fn read(
        &mut self,
        manifest: Option<PathBuf>,
        profiles: Option<PathBuf>,
        lines: bool,
        refused: Option<String>,
    ) -> bool {
        let same = self.manifest == manifest
            && self.profiles == profiles
            && self.debug_lines == lines
            && self.edit_refused == refused;
        if same {
            return false;
        }
        self.manifest = manifest;
        self.profiles = profiles;
        self.debug_lines = lines;
        self.edit_refused = refused;
        true
    }

    /// A build is starting. Whether it is: a second build queued behind the first would
    /// compile bytes the reader has since changed, so this is what says the press did
    /// anything.
    fn start(&mut self) -> bool {
        if self.building {
            return false;
        }
        self.building = true;
        true
    }

    /// Take the finished build `run`, `open` being the binaries the project has open.
    /// Answers with the ones this build wrote over, which the hook is to close and open
    /// again.
    ///
    /// **Only the previous build's artifacts are replaced.** A binary is a path throughout
    /// the app, so two generations of one file cannot both be in the objects list; but a
    /// file the reader opened by hand is theirs, even where a build has just written the
    /// same path. A build that produced nothing leaves the previous list standing: those
    /// paths are still what is open, and still what the next build that succeeds replaces.
    fn finished(
        &mut self,
        run: cargo::Run,
        sources: HashSet<PathBuf>,
        open: &[PathBuf],
    ) -> Vec<PathBuf> {
        let produced: Vec<PathBuf> = match &run {
            cargo::Run::Built { artifacts, .. } => artifacts
                .iter()
                .map(|artifact| artifact.path.clone())
                .collect(),
            _ => self.previous.clone(),
        };
        self.building = false;
        self.built = Some(run);
        self.sources = sources;
        self.previous = produced;
        self.previous
            .iter()
            .filter(|path| open.contains(path))
            .cloned()
            .collect()
    }

    /// The one line under the button saying where the last build got to, and whether that
    /// line is bad news.
    pub(crate) fn status(&self) -> Option<(String, bool)> {
        if self.building {
            return Some(("Building...".to_owned(), false));
        }

        let count = |level: Level, one: &str, many: &str| {
            let count = self
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.level == level)
                .count();
            match count {
                0 => String::new(),
                1 => format!(": 1 {one}"),
                count => format!(": {count} {many}"),
            }
        };

        match self.built.as_ref()? {
            cargo::Run::Built { .. } => Some((
                format!("Built{}", count(Level::Warning, "warning", "warnings")),
                false,
            )),
            cargo::Run::Rejected { .. } => Some((
                format!("Not built{}", count(Level::Error, "error", "errors")),
                true,
            )),
            cargo::Run::NoCargo(_) => Some(("cargo could not be started".to_owned(), true)),
        }
    }
}

/// The build state, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Building(pub(crate) State<Builds>);

/// One thing to do in the project's directory. It carries what it needs, so nothing can
/// change under the worker between the ask and the answer.
pub(crate) struct BuildJob {
    pub(crate) directory: PathBuf,
    pub(crate) profile: Profile,
    pub(crate) what: BuildWhat,
}

/// Which of the three to do. The directory and the profile are the same question for each,
/// so they are the job's and only the verb is here.
pub(crate) enum BuildWhat {
    /// What the manifest says: whether there is one, and what it says about debug
    /// information for this profile.
    Read,
    Build,
    /// Ask the profile for line tables, in the reader's own manifest.
    AddDebugLines,
}

/// What the worker answers with.
pub(crate) enum BuildAnswer {
    Read {
        manifest: Option<PathBuf>,
        profiles: Option<PathBuf>,
        debug_lines: bool,
        /// Why the edit that asked for this read was refused, when one did. `None` for a
        /// plain read, which is what clears the last refusal.
        refused: Option<String>,
    },
    /// A finished build, with the diagnostic files the view may offer as targets already
    /// picked out ([`openable`]): the run alone would leave that to the rows.
    Done {
        run: cargo::Run,
        sources: HashSet<PathBuf>,
    },
}

/// The blocking half. Handed in rather than called directly, so a test can drive the whole
/// mechanism with no cargo on the machine.
pub(crate) fn build_work(job: BuildJob) -> BuildAnswer {
    let BuildJob {
        directory,
        profile,
        what,
    } = job;
    match what {
        BuildWhat::Read => read(&directory, profile, None),
        BuildWhat::Build => {
            let run = cargo::run(&directory, profile);
            BuildAnswer::Done {
                sources: openable(&directory, run.diagnostics()),
                run,
            }
        }
        BuildWhat::AddDebugLines => {
            // The answer is the file read back, whether or not the write worked: a write
            // that failed must not leave the view saying the lines are there. What
            // refused it goes back with the read, since the row it leaves standing says
            // nothing about the press.
            let refused = cargo::add_debug_lines(&directory, profile).err();
            read(&directory, profile, refused)
        }
    }
}

/// Of the files `diagnostics` name, the ones the Project view may offer as targets: under
/// `directory`, and readable as source.
///
/// cargo spells a file relative to where it ran, so the path is `directory` joined with
/// it; one outside -- a dependency's, out of the registry -- is a file the app has no
/// business opening, and one the source cache would refuse is a target that would do
/// nothing when pressed. Both questions are answered here, on the worker, and one `stat`
/// per **file** however many diagnostics name it.
fn openable(directory: &Path, diagnostics: &[Diagnostic]) -> HashSet<PathBuf> {
    let mut named: HashSet<PathBuf> = HashSet::new();
    for span in diagnostics.iter().filter_map(|one| one.span.as_ref()) {
        let file = directory.join(&span.file);
        if file.starts_with(directory) {
            named.insert(file);
        }
    }
    named.retain(|file| showable(file));
    named
}

fn read(directory: &Path, profile: Profile, refused: Option<String>) -> BuildAnswer {
    let manifest = cargo::manifest(directory);
    // Named only when it is not the file cargo is run over: a member's profiles are the
    // workspace root's, and the reader is being offered an edit to that file and not to
    // the one the row above names.
    let profiles = cargo::profile_manifest(directory);
    BuildAnswer::Read {
        debug_lines: cargo::debug_lines(directory, profile),
        profiles: (manifest.as_ref() != Some(&profiles)).then_some(profiles),
        manifest,
        refused,
    }
}

/// How the view reaches the worker.
pub(crate) type BuildJobs = Requests<BuildJob>;

/// Start the worker and keep the state in step with it. Called once, at the root.
pub(crate) fn use_building_with(
    build: State<Builds>,
    states: ProjectStates,
    opened: State<Opened>,
    work: impl Fn(BuildJob) -> BuildAnswer + Send + 'static,
) -> BuildJobs {
    let jobs = use_worker(
        "the build worker",
        // Nothing supersedes: a build takes seconds and is asked for by a press, and the
        // two manifest jobs are cheap and each of them is the answer to the one after it.
        |job, _, _| vec![job],
        move |job| Some(work(job)),
        move |answer, _| match answer {
            BuildAnswer::Read {
                manifest,
                profiles,
                debug_lines,
                refused,
            } => {
                write_if(build, |next| {
                    next.read(manifest, profiles, debug_lines, refused)
                });
            }
            BuildAnswer::Done { run, sources } => finished(build, states, opened, run, sources),
        },
    );

    // A context, because the button that asks is inside a tab that is handed
    // nothing; returned as well, so a test can ask directly.
    use_provide_context(|| jobs.clone())
}

/// Take a finished build: hold it, and put the binaries it wrote over back in the state
/// the reader had them in.
///
/// Which binaries those are is [`Builds::finished`]'s to say; what is left here is the
/// reopening. The close is unconditional for the ones that are replaced -- whether or not
/// the new bytes parse, the objects in hand describe bytes that are gone -- and takes
/// those files' tabs, positions and visits with it, exactly as a scratchpad's rebuild
/// does.
fn finished(
    mut build: State<Builds>,
    states: ProjectStates,
    opened: State<Opened>,
    run: cargo::Run,
    sources: HashSet<PathBuf>,
) {
    // What the panes have read of the workspace is from before the reader edited it and
    // pressed Build. Dropped whatever the build came to: a build that failed says the
    // files have changed just as one that did not.
    let directory = states.proj.peek().workspace();
    if let Some(directory) = directory {
        forget_source_under(&directory);
        // And the language server is holding the text from before it, for every file of
        // the reader's the build rewrote: it answers about what it was given until it is
        // told otherwise (`src/ui/linking.rs`).
        write_if(opened, |waiting| waiting.reread(&directory));
    }

    // Bound before the write, as ever.
    let open = project::binaries(&states.objects.peek());
    let mut next = build.peek().clone();
    let reopening = next.finished(run, sources, &open);
    build.set(next);

    if reopening.is_empty() {
        return;
    }

    for path in &reopening {
        close_binary(states, path);
    }

    // One load for all of them, rather than one spawn and one load each.
    spawn(async move {
        open_binaries(states.objects, states.loading, reopening).await;
    });
}

/// Ask for a build of the open project, if there is one to build and none going.
pub(crate) fn start_build(
    build: State<Builds>,
    jobs: &BuildJobs,
    directory: PathBuf,
    profile: Profile,
) {
    // The button's own `enabled` says this too. Both, because a second build queued behind
    // the first would compile bytes that have since changed.
    if write_if(build, |next| next.start()) {
        jobs.send(BuildJob {
            directory,
            profile,
            what: BuildWhat::Build,
        });
    }
}

#[cfg(test)]
mod tests;

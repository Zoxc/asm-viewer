//! Building the project's own workspace: what the app holds about it, and the one worker
//! thread that runs cargo and edits its manifest.
//!
//! `use_scratchpad_with`'s shape, and for its reasons (`src/ui/pad.rs`): the work is
//! blocking so it goes to a thread of its own, and it is **one** thread so that the
//! project's directory has a single writer — the debug-lines edit cannot land inside the
//! build that is reading the same manifest.
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
    /// Whether the chosen profile carries the line information the source side is drawn
    /// from, as the manifest has it now.
    pub(crate) debug_lines: bool,
    /// What the build before this one produced. **The set a build replaces**, which is why
    /// it is saved with the session: a binary the reader opened some other way is left
    /// alone, and the build before may have been in another run of the app.
    pub(crate) previous: Vec<PathBuf>,
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
        match &self.built {
            Some(cargo::Run::Built { diagnostics, .. }) => diagnostics,
            Some(cargo::Run::Rejected { diagnostics, .. }) => diagnostics,
            _ => &[],
        }
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

/// One thing to do in the project's directory. Each carries what it needs, so nothing can
/// change under the worker between the ask and the answer.
pub(crate) enum BuildJob {
    /// What the manifest says: whether there is one, and what it says about debug
    /// information for this profile.
    Read {
        directory: PathBuf,
        profile: Profile,
    },
    Build {
        directory: PathBuf,
        profile: Profile,
    },
    /// Ask the profile for line tables, in the reader's own manifest.
    AddDebugLines {
        directory: PathBuf,
        profile: Profile,
    },
}

/// What the worker answers with.
pub(crate) enum BuildAnswer {
    Read {
        manifest: Option<PathBuf>,
        debug_lines: bool,
    },
    Done(cargo::Run),
}

/// The blocking half. Handed in rather than called directly, so a test can drive the whole
/// mechanism with no cargo on the machine.
pub(crate) fn build_work(job: BuildJob) -> BuildAnswer {
    match job {
        BuildJob::Read { directory, profile } => read(&directory, profile),
        BuildJob::Build { directory, profile } => {
            BuildAnswer::Done(cargo::run(&directory, profile))
        }
        BuildJob::AddDebugLines { directory, profile } => {
            // The answer is the file read back, whether or not the write worked: a write
            // that failed must not leave the view saying the lines are there.
            if let Err(error) = cargo::add_debug_lines(&directory, profile) {
                log::warn!(
                    "could not add debug lines to {}: {error}",
                    directory.display()
                );
            }
            read(&directory, profile)
        }
    }
}

fn read(directory: &Path, profile: Profile) -> BuildAnswer {
    BuildAnswer::Read {
        manifest: cargo::manifest(directory),
        debug_lines: cargo::debug_lines(directory, profile),
    }
}

/// How the view reaches the worker.
#[derive(Clone)]
pub(crate) struct BuildJobs {
    jobs: async_channel::Sender<BuildJob>,
}

impl BuildJobs {
    pub(crate) fn send(&self, job: BuildJob) {
        // A full queue is impossible (it is unbounded) and a closed one is the app going
        // down, so there is nothing here to tell anybody about.
        let _ = self.jobs.try_send(job);
    }
}

/// Start the worker and keep the state in step with it. Called once, at the root.
pub(crate) fn use_building_with(
    mut build: State<Builds>,
    states: ProjectStates,
    work: impl Fn(BuildJob) -> BuildAnswer + Send + 'static,
) -> BuildJobs {
    let jobs = use_hook(move || {
        let (requests, jobs) = async_channel::unbounded::<BuildJob>();
        let (answered, answers) = async_channel::unbounded::<BuildAnswer>();

        // Named, so that a panic on it says which worker died (`crate::panics`).
        let started = std::thread::Builder::new()
            .name("the build worker".to_owned())
            .spawn(move || {
                // Nothing supersedes: a build takes seconds and is asked for by a press, and
                // the two manifest jobs are cheap and each of them is the answer to the one
                // after it.
                while let Ok(job) = jobs.recv_blocking() {
                    // A send that fails is the app shutting down and taking the receiver with
                    // it.
                    if answered.send_blocking(work(job)).is_err() {
                        return;
                    }
                }
            });
        if let Err(error) = started {
            log::warn!("the build worker could not be started: {error}");
        }

        spawn(async move {
            while let Ok(answer) = answers.recv().await {
                match answer {
                    BuildAnswer::Read {
                        manifest,
                        debug_lines,
                    } => {
                        let mut next = build.peek().clone();
                        next.manifest = manifest;
                        next.debug_lines = debug_lines;
                        build.set(next);
                    }
                    BuildAnswer::Done(run) => finished(build, states, run),
                }
            }
        });

        BuildJobs { jobs: requests }
    });

    // A context, because the button that asks is inside a tab that is handed
    // nothing; returned as well, so a test can ask directly.
    use_provide_context(|| jobs.clone())
}

/// Take a finished build: hold it, and put the binaries it wrote over back in the state
/// the reader had them in.
///
/// **Only the previous build's artifacts are replaced.** A binary is a path throughout the
/// app, so two generations of one file cannot both be in the objects list; but a file the
/// reader opened by hand is theirs, even where a build has just written the same path. The
/// close is unconditional for the ones that are replaced -- whether or not the new bytes
/// parse, the objects in hand describe bytes that are gone -- and takes those files' tabs,
/// positions and visits with it, exactly as a scratchpad's rebuild does.
fn finished(mut build: State<Builds>, states: ProjectStates, run: cargo::Run) {
    let produced: Vec<PathBuf> = match &run {
        cargo::Run::Built { artifacts, .. } => artifacts
            .iter()
            .map(|artifact| artifact.path.clone())
            .collect(),
        // A build that produced nothing leaves the previous list standing: those paths are
        // still what is open, and still what the next build that succeeds replaces.
        _ => build.peek().previous.clone(),
    };

    let mut next = build.peek().clone();
    next.building = false;
    next.built = Some(run);
    next.previous = produced.clone();
    let reopening: Vec<PathBuf> = {
        let open = project::binaries(&states.objects.peek());
        next.previous
            .iter()
            .filter(|path| open.contains(path))
            .cloned()
            .collect()
    };
    build.set(next);

    if reopening.is_empty() {
        return;
    }

    for path in &reopening {
        close_binary(
            states.objects,
            states.loading,
            states.open,
            states.asm_at,
            states.src_at,
            states.code_at,
            states.driven,
            states.marks_at,
            states.visits,
            path,
        );
    }

    // One load for all of them, rather than one spawn and one load each.
    spawn(async move {
        open_binaries(states.objects, states.loading, reopening).await;
    });
}

/// Ask for a build of the open project, if there is one to build and none going.
pub(crate) fn start_build(
    mut build: State<Builds>,
    jobs: &BuildJobs,
    directory: PathBuf,
    profile: Profile,
) {
    // The button's own `enabled` says this too. Both, because a second build queued behind
    // the first would compile bytes that have since changed.
    if build.peek().building {
        return;
    }

    let mut next = build.peek().clone();
    next.building = true;
    build.set(next);
    jobs.send(BuildJob::Build { directory, profile });
}

/// The project's directory as a path, or `None` when the reader has not named one.
pub(crate) fn workspace(proj: &OpenProject) -> Option<PathBuf> {
    given(&proj.directory).map(PathBuf::from)
}

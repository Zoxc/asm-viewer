//! The one worker mechanism, in the two shapes everything in the app is asked in.
//!
//! **Nothing is analysed on the UI thread** (`AGENTS.md`), and this is what that rule is
//! implemented by: a named thread doing the blocking half, a channel each way, and a task
//! on the UI thread taking what comes back.
//!
//! [`use_worker`] is the **request/answer** shape: one thread for the app's lifetime, fed
//! jobs and answering them one at a time. What is [`Queued`] behind the job in hand is the
//! `drain` policy's, which is where superseding lives and is per worker -- the analysis
//! keeps the newest question of each kind, the source reader the newest of the one kind it
//! has, the build drops nothing, the scratchpad supersedes a save only by a job writing the
//! same pad's package, and the language server keeps the last question of each consumer.
//! Dropped **before** it is started and not after the fact, which is the whole point of
//! doing it here.
//!
//! [`stream`] is the **one-shot** shape: one question, worked once, answering with events
//! until it has nothing more to say. Nothing has to arrange cancelling: the task taking
//! the events drops the receiver when its question has moved on, the next send fails, and
//! the work breaks where it stands.
//!
//! Both name the thread they start. A panic on an unnamed one reaches `crate::panics`
//! anonymously, which is the one thing the naming is for.

use super::*;

/// A job sender: what a hook, an effect or a handler asks a worker with.
///
/// A closed channel is the app going down and is ignored -- there is nobody left to tell
/// -- and the queue is unbounded, so a send cannot fail for any other reason. That is why
/// nothing here answers whether the job was taken.
pub(crate) struct Requests<J>(async_channel::Sender<J>);

// By hand, not derived: a derive would ask `J: Clone`, and a job is never cloned.
impl<J> Clone for Requests<J> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<J> Requests<J> {
    pub(crate) fn send(&self, job: J) {
        let _ = self.0.try_send(job);
    }
}

/// Start a worker thread under `name`, so that a panic on it says which worker died
/// (`crate::panics`).
///
/// A thread that will not start is logged and nothing else: the app goes on without
/// whatever that thread would have answered, which is a live app missing one pane's worth
/// of information rather than no app at all.
pub(crate) fn thread(name: &'static str, body: impl FnOnce() + Send + 'static) {
    let started = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(body);
    if let Err(error) = started {
        log::warn!("{name} could not be started: {error}");
    }
}

/// What is queued behind the job in hand, as a `drain` policy sees it: the jobs still on
/// the channel, taken in order, and the way to hand one back.
///
/// An iterator, because that is what a policy does with it: the newest question of each
/// kind, the last of them, the lot. It never blocks -- [`None`] is "nothing waiting
/// *now*", and a job sent a moment later is the next one the worker takes off the channel
/// rather than one this missed.
pub(crate) struct Queued<'a, J> {
    jobs: &'a async_channel::Receiver<J>,
    held: &'a mut VecDeque<J>,
}

impl<'a, J> Queued<'a, J> {
    pub(crate) fn new(jobs: &'a async_channel::Receiver<J>, held: &'a mut VecDeque<J>) -> Self {
        Queued { jobs, held }
    }

    /// Hand `later` back to be done in its turn: taken off the channel and not done, so it
    /// is kept **ahead** of the channel and a job that has waited its turn is not put
    /// behind whatever has arrived since.
    ///
    /// The scratchpad's is the one policy that needs it: a save of one pad may not be
    /// stepped over by a job for another, so the job that stopped the drain is held rather
    /// than dropped (`src/ui/pad.rs`).
    pub(crate) fn hold(&mut self, later: J) {
        self.held.push_back(later);
    }
}

impl<J> Iterator for Queued<'_, J> {
    type Item = J;

    fn next(&mut self) -> Option<J> {
        self.jobs.try_recv().ok()
    }
}

/// The request/answer worker: a named thread fed jobs over one channel and answering over
/// another, with the task that takes those answers on the UI thread. Started once, in a
/// [`use_hook`].
///
/// `drain` is handed the job taken off the queue and everything queued behind it
/// ([`Queued`]); what it returns is worked, in order, before anything else is taken.
/// `work` answering [`None`] is a job with nothing to say. `take` is handed each answer on
/// the UI thread, and the way to ask for more with it: an answer can be a question, as the
/// scratchpad's listing is.
///
/// The work is an argument rather than a call, on every worker, because it is the seam the
/// headless tests substitute a worker of their own through: superseding is a race by
/// construction and cannot be asserted against work that answers as fast as it is asked.
pub(crate) fn use_worker<J: Send + 'static, A: Send + 'static>(
    name: &'static str,
    drain: impl FnMut(J, &mut Queued<'_, J>) -> Vec<J> + Send + 'static,
    work: impl Fn(J) -> Option<A> + Send + 'static,
    take: impl FnMut(A, &Requests<J>) + 'static,
) -> Requests<J> {
    use_worker_answering(name, drain, work, take).0
}

/// [`use_worker`], handing back the answer sender beside the way to ask.
///
/// For the one work half that has to speak out of turn: starting a language server answers
/// that the process is there before the handshake it is in the middle of has finished
/// (`LspAnswer::Spawned`), because a server that reads its input and says nothing never
/// comes back from that handshake, and what ends it is the app dropping the handle.
pub(crate) fn use_worker_answering<J: Send + 'static, A: Send + 'static>(
    name: &'static str,
    mut drain: impl FnMut(J, &mut Queued<'_, J>) -> Vec<J> + Send + 'static,
    work: impl Fn(J) -> Option<A> + Send + 'static,
    mut take: impl FnMut(A, &Requests<J>) + 'static,
) -> (Requests<J>, async_channel::Sender<A>) {
    use_hook(move || {
        let (sender, jobs) = async_channel::unbounded::<J>();
        let (answered, answers) = async_channel::unbounded::<A>();
        let requests = Requests(sender);

        // A `std::thread` and not a spawned task: this is where the app's blocking work is
        // done -- seconds of it, in the worst case -- and freya's executor is the UI
        // thread.
        thread(name, {
            let answered = answered.clone();
            move || {
                // What `drain` handed back, kept ahead of the channel (`Queued::hold`).
                let mut held = VecDeque::<J>::new();
                loop {
                    let Some(job) = held.pop_front().or_else(|| jobs.recv_blocking().ok()) else {
                        return;
                    };
                    let doing = drain(job, &mut Queued::new(&jobs, &mut held));
                    for job in doing {
                        let Some(answer) = work(job) else {
                            continue;
                        };
                        // A send that fails is the app shutting down and taking the
                        // receiver with it.
                        if answered.send_blocking(answer).is_err() {
                            return;
                        }
                    }
                }
            }
        });

        spawn({
            let requests = requests.clone();
            async move {
                while let Ok(answer) = answers.recv().await {
                    take(answer, &requests);
                }
            }
        });

        (requests, answered)
    })
}

/// The one-shot worker: `work` run once on a named thread for one question, its events
/// arriving on the receiver.
///
/// The thread stops where it stands once the receiver is dropped, which is what the task
/// taking the events does when the question has moved on -- a second search, a project
/// left and the app closing, all through the one rule.
///
/// `ahead` is how much of the answer may sit between the two. [`None`] is unbounded, for a
/// worker that should run flat out; `Some(n)` parks it in its send once the taker is that
/// far behind, which is both the app's backpressure and how the worker learns the moment
/// nobody is waiting.
pub(crate) fn stream<E: Send + 'static>(
    name: &'static str,
    ahead: Option<usize>,
    work: impl FnOnce(&mut dyn FnMut(E) -> ControlFlow<()>) + Send + 'static,
) -> async_channel::Receiver<E> {
    let (sender, events) = match ahead {
        Some(ahead) => async_channel::bounded::<E>(ahead),
        None => async_channel::unbounded::<E>(),
    };
    thread(name, move || {
        work(&mut |event| match sender.send_blocking(event) {
            Ok(()) => ControlFlow::Continue(()),
            // Nobody is waiting for the rest of this.
            Err(_) => ControlFlow::Break(()),
        });
    });
    events
}

/// Everything a stream has said since the last look, in one go. [`None`] is the worker
/// having finished or gone.
///
/// A batch per wake and not a write per event: each write is a render, and a walk over a
/// large tree answers in thousands.
pub(crate) async fn next_batch<E>(events: &async_channel::Receiver<E>) -> Option<Vec<E>> {
    let first = events.recv().await.ok()?;
    let mut batch = vec![first];
    while let Ok(more) = events.try_recv() {
        batch.push(more);
    }
    Some(batch)
}

/// What a worker's state does with an answer: **the state judges it against what is asked
/// now and says whether anything changed; the hook only writes.**
///
/// The judging is the type's, so the rules `agents/Worker.md` states -- an answer is taken
/// only if its ask is the one asked now, a listing is retagged rather than reworked, a
/// closed binary takes its answers with it -- are methods a unit test can call rather than
/// lines inside an answer task. The writing is what is left, and it is the same three
/// lines every time: peek into a binding of its own, since a read guard held across a
/// write panics (`AGENTS.md`); judge; set only where the judge says so, since a write
/// notifies whether or not it changed anything. It answers whether it wrote, for the
/// caller with something else to do when it did -- a stop that has to tell the worker too.
///
/// The judge's `bool` and not `set_if_modified`: a state that says for itself what it did
/// needs no `PartialEq`, and several of these hold an `Arc<Object>` or a process handle
/// that has none to give. Where a state has one and the edit is the reader's rather than a
/// worker's, `marks::update` is this same mechanism with the comparison doing the judging.
///
/// Two states are written through the guard instead, [`Searched`] and [`Pads`]: what they
/// hold *is* the answer, and a clone per batch would copy all of it to add what has just
/// arrived. The rule is still the type's; only the writing differs.
pub(crate) fn write_if<S: Clone + 'static>(
    mut state: State<S>,
    judge: impl FnOnce(&mut S) -> bool,
) -> bool {
    let mut next = state.peek().clone();
    let moved = judge(&mut next);
    if moved {
        state.set(next);
    }
    moved
}

/// Send the question a state is owed, once per question: the asking effect every worker
/// is fed by, written once.
///
/// `pending` is what the state wants asked, and it is read **through a memo**. That is
/// the one choice this hook makes for every site, and it is the one that is easy to get
/// wrong: a state its own answers are written into would otherwise be asked again for
/// each of them -- a second run of the work the first answer is still arriving from. The
/// memo recomputes for those writes and wakes nothing, the question being unchanged.
///
/// Whether there is a question at all is `pending`'s too, so a gate -- a language server
/// that has to be ready before it is asked -- is read there, which is what subscribes the
/// memo to it.
///
/// `mark` records that the question has gone out, and is called **before** the send, so
/// that a question is never in flight unmarked: a worker answering where it stands, as a
/// test's does, would otherwise answer into a state that is then told the question has
/// just gone out. It is a [`write_if`] over the state, whose own rule says what a mark
/// is; a state with nothing to mark passes [`unmarked`].
pub(crate) fn use_asking<J: Clone + PartialEq + 'static>(
    pending: impl Fn() -> Option<J> + 'static,
    mark: impl Fn(&J) + 'static,
    send: impl Fn(J) + 'static,
) {
    let asked = use_memo(pending);
    use_side_effect(move || {
        // Reading the memo subscribes this to the question. Bound before the mark, the
        // read being a guard (`AGENTS.md`).
        let job = asked.read().clone();
        let Some(job) = job else {
            return;
        };
        mark(&job);
        send(job);
    });
}

/// The `mark` of a state with nothing to record, where the memo is the whole of what
/// stops a question being sent twice.
pub(crate) fn unmarked<J>(_: &J) {}

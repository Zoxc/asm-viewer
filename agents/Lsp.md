# The language server

A language server, started over the open project's directory and asked where the thing at a
source position is defined. `src/lsp.rs` is the client, `src/ui/language.rs` is the state,
the worker and the control in the top bar.

**Which program is the project's own setting** (`Project::language_server`, empty for
rust-analyzer). A project on a toolchain of its own is read by a server this app cannot
guess at, and the same box is where a wrapper like `ra-multiplex` goes; it is a plain value
in `project.toml` beside the name and the directory, and the Project view is where it is
typed. Nothing in `lsp.rs` names a program except the default, and the failures it reports
say "the language server" rather than a name this app did not choose.

## Why it is a control and not a lazy start

The step note asked for a server started on the first question. It is a control the reader
presses instead. A language server reads a whole project and holds it in memory, and most
of what this app is for -- reading a binary somebody else built -- never asks it anything;
a lazy start hides that cost behind a click that looks like navigation. Pressing it is also
what makes the two failures visible: not installed, and installed but unable to read the
project. Off at every launch, and not remembered per project, because a background process
nobody asked for on this run is what the control exists to prevent. What a project does
remember is the *agreement* below, which is permission to start one and never a server that
is already running.

## Asking before it is started

A press is not enough on its own the first time. Starting a server runs a program over the
reader's project and, by the capabilities section below, runs that project's own build
scripts and expands its proc macros -- which is code somebody else wrote. So a directory
the reader has not agreed to is **asked about instead of started**.

The answer is the project's, a plain `trusted` in `project.toml` beside the program's name,
and **absent is no**: a project nobody has been asked about writes no key.

Agreeing happens where the question is asked, at the start it holds up; **taking it back is
the Project view's**, beside the program and the status, because a reader who cannot see the
answer they gave cannot change their mind about it. Taking it back stops the server too:
somebody saying they did not mean to let a program read this directory has said something
about the program reading it *now*, and a control reading "not agreed to" over a server
going through their project would be answering them with a lie.

What it is about is a *directory*, and the effect that follows the project is where that is
kept honest -- but only one of the three things it sees is the agreement being outlived.
The reader typing a new directory into the box has pointed **this** project somewhere else,
and the agreement was to the old place, so it goes. A project *arriving* brings its own
answer with it, out of its own file, and taking that off it would not only ask again but
write the `false` straight back into the file it was read from, since the open project is
saved as it changes. And the mount is neither: the deps it mounts with are already the
reopened project's, the restore being an earlier hook of the same render. So the effect
remembers the id and directory it last saw, and clears only where the id stayed and the
directory moved. The server still stops for all three: it belonged to the project that is
being left.

The gate is in `start_server`, which both presses go through, and `run_server` is the half
that actually starts one; so neither the top bar's control nor the Project view's button can
grow a path around it. The question holds the directory and program it named rather than
working them out again when it is answered: what was agreed to is what was asked about, not
whatever the directory box says by then. Declining remembers nothing -- the answer was to
that press -- and a stop clears an unanswered question along with the server.

`TrustPrompt` draws it **at the root, under the top bar**, and not in the Project view's own
section beside the other Start button. The control is pressed from wherever the reader is,
and a question drawn in a tab that is not on screen is a press that did nothing. It is a
band in the bar's own style rather than a window over the app, it names the directory
because that is what is being agreed to, and it lays out as nothing while there is nothing
to ask. Drawn by the app and not by `rfd`, whose dialogs the headless runner cannot press:
the app answers questions about its UI with tests.

## The protocol, hand-rolled

Four messages: `initialize`, `initialized`, `textDocument/definition`, and a reply to
whatever the server asks of us. A protocol crate would bring a type per request in the
specification and a runtime to drive them, for those four. `cargo tree -d` is unchanged by
this step: `serde_json` was already in the tree, and the manifest comment on it already covers a
protocol rather than a file.

**One request is in flight at a time**, so there is no table of outstanding ids: a request
waits for an answer carrying the id it asked under. It is the limit on the surface too: a
second consumer wanting answers at the same time is what would end it.

**A reader thread owns the server's output.** It began without one -- a request read frames
until its own answer came back -- and that was enough right up to the moment the app needed
to know the server was *busy*, which arrives as `$/progress` while nothing is being asked
and at no other time. So the reader is the only thing that reads: an answer goes to
whoever is waiting for it over a channel, a request is replied to on the spot, and a
notification is acted on. Both threads write, so the server's input is behind a lock -- the
reader has to write because asking for progress is what makes rust-analyzer ask this app to
make a progress token. Dropping the conversation takes that input away and closes it, which
is how a server is told there is nothing more coming and what lets the reader thread go.

`Talk` is generic over its two streams, so the whole conversation is tested against a fake
server over `std::io::pipe()` and only `start_in` needs a program. `write_message` and
`read_message` are the wire format on their own, tested over `Cursor`s.

Things learned from rust-analyzer's own transport, each of which is a test:

- The header separator is a colon **and a space**. `lsp-server` splits on `": "` and calls
  anything else a malformed header, and dies.
- `initialized` must be the very next message after the `initialize` answer. Anything else
  first and the server gives up on the conversation.
- **The declared capabilities are one line long**, and what is left out is the decision.
  Every request rust-analyzer makes of a client -- for configuration, to register a file
  watcher -- is opt-in through a capability, so declaring none of those leaves a
  conversation this app only ever speaks first in. Nothing is said about positions or about
  definitions either: UTF-16 and plain locations are the defaults, both are what is wanted,
  and naming them would only be a chance to name them wrongly. The one thing asked for is
  progress, since it is the only account of a server that is still reading the project. A
  server that asks something anyway is answered -- an empty configuration, nothing for a
  progress token, and "not a method this client has" for the rest -- because a server
  waiting on a reply is a conversation that stops.
- The initialization options are `wanted()`, **one line**, and it turns the check off.
  Named rather than written into the handshake because it is also what a project's own
  settings are laid over (below). The check is
  not moot despite this client opening and saving nothing: watched over a real server, a
  `rust-analyzer/flycheck/0` token opens on loading the workspace, so the check runs there
  and would be a second build of the reader's project whose output nothing here shows. The
  server's own diagnostics need no turning off beside it, being published for open
  documents only.
- Nothing is turned **on**, because what navigation needs is what rust-analyzer already
  does. Build scripts run and proc macros expand unless a client says otherwise, and both
  matter for navigation and not only for building: a `cfg` a build script set decides which
  code exists, and a name inside a macro that was not expanded resolves to nothing. Naming
  a default again would only be a chance to name it wrongly, which is the rule the
  capabilities follow too. Checked against a real rust-analyzer over this repo: a name
  inside a `json!` body answers with the line it was bound on, and `Vec::push` answers with
  the sysroot's own sources, so navigation into `std` needs no setting either -- only the
  `rust-src` component, which is a toolchain thing and not something a client can send.
- What is left is what a project built by something other than plain cargo has to say for
  itself: which manifests are its workspaces, which compiler and proc-macro server its
  macros must be expanded with, where its own copy of the standard library is. That is
  read from the project's own file, below; `rust-lang/rust`'s tree is the one every part of
  it came from.
- Error code -32801 means the server is still reading the project, and -32800 that it
  dropped the question. Both are "ask again", so they are an empty answer rather than a
  failure: a click is a question, not a promise.
- stderr is **read on a thread of its own and the first few kilobytes kept**, not thrown
  away. A pipe nobody reads fills and blocks the program in a write, which is why it cannot
  simply be piped; and what a program that will not run writes there is the only account of
  why. `rust-analyzer` on the path is often rustup's proxy, and a toolchain without the
  component is one line on stderr and an exit -- which, with the pipe discarded, reached the
  reader as "rust-analyzer stopped answering". A handshake that fails against a process that
  has already ended is therefore reported as a program that would not start, carrying what
  it said (`gone_instead`). The wait for it to finish ending is bounded, and only paid by a
  handshake that has already failed.
- The one notification worth keeping is `window/showMessage`, which is all a client with no
  capabilities is told when the server cannot make sense of the project; it goes to the log.
  A question asked before the workspace is loaded can also come back as InternalError with
  "file not found", which is neither of the two "ask again" codes -- something for Step 24
  to decide about, since nothing asks yet.

Positions go out as the protocol takes them -- a line counted from zero and a column in
UTF-16 units, which is what `src/chars.rs` already counts in -- and a `Place` comes back
with a **1-based** line, the unit line information is in everywhere else in the app. The
conversion is in one place and happens once.

**The file is not opened first.** rust-analyzer reads the project's files itself, and this
app only ever shows what is on disk, so a `didOpen` would put an overlay over the file that
has to be taken off again and can only go stale. The cost is that a file outside the
project answers nothing, which is the same nothing a question with no answer gets.

## The project's own settings

Some trees cannot be read by a server that was told nothing, and what would fix that is
already in them: `.vscode/settings.json`, VS Code's own file. So it is read and passed
through, and **no such file is the ordinary case** rather than anything worth a word.

What is taken is the keys beginning `rust-analyzer.`, with the prefix off and the rest of
the name split on its dots into a tree. Both halves matter and **both are silent when they
are wrong**: watched against a real server, a key that kept its prefix and a key whose dots
were not split were each ignored without a sound. It is
also all this app has to understand -- never what an option means, only how a name is
spelled -- which is what makes passing a project's settings through cheap. Every other key
is the editor's own (`git.*`, `files.associations`) and is skipped in silence.

The result is `merged` over `wanted()`, **leaf by leaf and not per name**: a file setting
`cargo.features` must not throw away a `cargo.x` this app sent.

`${workspaceFolder}` is the only variable. VS Code resolves six and leaves a name it does
not know as written; here an unresolved `${...}` reaching the server is a path that silently
does not exist, so every other one is an error. So is a file that is not an object of JSON,
and a name given both a value and a table (`cargo` beside `cargo.features`) -- which of the
two was meant is not this app's to pick.

The file is read as **JSONC**, as VS Code reads it and as the files in the wild are
written: the tree this is all for opens with nine lines of `//`. `serde_json` takes neither
comments nor a trailing comma, so `as_json` blanks the first and drops the second before it
sees the text. Comments become spaces, and a newline in a block comment is kept, so what
`serde_json` says about the line and column of a real mistake is about the file the reader
wrote. Nothing inside a string is touched, and that is the whole difficulty: a `//` is half
of every URL, and a string can end in an escaped quote or hold a backslash before its
closing one, so both passes track the string and the escape. Getting it wrong cuts a path
short without a word, which is the failure this feature exists to prevent.

Two bounds, both because the input is a file (`AGENTS.md`): the tree is built iteratively so
a name of ten thousand dots cannot overflow the stack, and `DEEPEST` refuses a name of more
parts than the walk back out is written to recurse over.

**An error starts nothing.** The check is in `run_server`, which is where a start actually
happens, so neither press nor the agreement can grow a path around it -- the same reason
the trust gate is in `start_server`. It is reported as `Lsp::Failed`, which is where a
failure to start is already said.

The read is the LSP worker's (`LspJob::ReadSettings`, `BuildJob::Read`'s shape): reading a
file blocks and nothing is read on the UI thread, and what it answers is what a start has
to carry. It happens in the effect that follows the project, at the **root**, and not in
the Project view: that view is a dock tab and an inactive tab is unmounted, while the
control in the top bar is pressed from wherever the reader is. One read answers both, and
it happens whether or not a server is ever started, since the view lists what it found
either way. The settings travel in the `Start` job the way `program` and `directory` do --
the worker thread may read no UI state. An answer is matched to the project by the
**directory** it was read in and not by a run: it is about a project and not about a
process. `worth_doing` keeps only the last read, since a directory typed a letter at a time
asks for one a keystroke.

## The process

Owned the way a scratchpad's run is, and `Group` moved to `src/process.rs` so both can have
it: arranged before the spawn, claimed after it, and killed as a group, since rust-analyzer
forks `cargo`, `rustc` and a proc-macro server of its own. A stop **kills** rather than
sending `shutdown`: a server that is indexing can take seconds to answer that request, and
a stop has to be over when it returns. rust-analyzer ignores the client's `processId`, so
nothing about the app dying would end it by itself.

The stop **takes** the process out from under the lock, so the second stop of a server is
the no-op the first made it, and a killed server is waited for exactly once -- that wait is
what keeps it out of the process table until the app ends. `Server`'s own `Drop` stops it,
because `Child`'s neither waits nor kills, and every handle is in a process-global list so
the window's close hook -- a `Send` callback that can read no UI state -- can reach one the
UI has lost.

A stop is also how a worker parked in a read is let go: the pipes close with the process,
so the read ends instead of waiting on a server that will never answer. That is why the
kill happens on the UI thread and the worker is only told afterwards.

## The worker and what an answer is about

`use_building_with`'s shape: one `std::thread`, two `async_channel`s, one `spawn` draining
answers, and the blocking half handed in so the headless tests never touch rust-analyzer.
Unlike the other three workers this one keeps something between jobs -- the conversation --
so `language_work` is a closure holding it. A `Mutex` and not `FnMut`, so the seam stays
the `Fn` the others are and the test harness fits unchanged; one thread calls it, so the
lock is never contended.

Two things say which server an answer is about, and they are not the same thing:

- The **run** counts starts and stops. `use_analysis` compares questions instead, but what
  an answer here is about is a process, which does not exist until the worker has started
  it. An answer whose run has moved is dropped.
- The **handle** is what ends that process. A `Started` answer arriving for a dead run is
  **stopped**, not dropped: that is the first moment anything in the app holds it, and a
  handle dropped instead of stopped is a server nothing can ever find again (`pad.rs` has
  the same rule for a run's process).

What the server says unasked comes back on a bounded channel the `Start` job carries, under
the run it was started in, and the only thing said so far is whether it is working. Bounded
so that a server reporting progress in a tight loop cannot outrun the app: the reader thread
waiting is the whole of the backpressure. **Working is not a state**, it is what a running
server is doing, so a handshake's answer and a first `$/progress` can arrive in either order
without one undoing the other.

`worth_doing` drains the queue to the last question, keeping every start and stop: a reader
clicking twice wants the second answer, and a press is never dropped. A question asked
while the server is still starting is sent all the same -- it queues behind the start and
is answered once there is somebody to answer it, and finds nothing to talk to if the start
failed. A question with the server off is not sent: the control is what starts one.

Leaving the project ends its server, from a side effect on the project's id and directory
rather than from `clear_project`: the server was started over that directory, and a
directory typed into the Project view is a different project's as far as it is concerned.

## The control, and what the Project view says

In the top bar, left of the two history buttons, drawn as a link and three letters -- what
a language server is asked here is where a name leads, and a link is that question rather
than the machinery answering it -- and built like `NavButton` but **named and bordered**
rather than an icon alone: it is the only thing in the app that starts a process
the reader did not ask for by name. It says `LSP` and not `rust-analyzer`, which is what
that corner has room for beside two chevrons and is the part of the app being named rather
than the program; the program's own name is in the tooltip and in the Project view.

Off and untouched it is text alone with **no border**: a part of the app nobody has asked
anything of should not look like it is holding something. The border is what says a press
would do something, so it comes up under the pointer and stays while a server is there.
Running puts `server_bg` under it -- the app's one control with a colour of its own, since a
process the reader started is worth telling apart from a toggle that happens to be on -- and
a failure colours both the border and the text `invalid_fg`. Starting, or a server reading
the project, turns the icon into a loader: the only moving thing in the bar, and it says an
answer is not ready rather than not there.

**Nothing about it changes width.** The two history buttons are at the same corner, and a
label or an icon that grew would walk them out from under the pointer -- so the state is
said in the same three letters, the same square of icon, and the tooltip. The tooltip is
half a second of real time away under the runner (`agents/Headless.md`), so what the tests
assert is the words it would be given.

The Project view says the same thing where it stays: one line under a heading of its own,
in `invalid_fg` when it is a failure, with a Start/Stop button beside the heading -- the
same two presses, so a reader already in that view need not go looking for the bar, and the
same question first where the directory has not been agreed to. A tooltip is gone the moment
the pointer moves, and a reason worth reading is worth reading twice.

Under that line is what the project's own settings gave the server, one row per setting with
the name and the value as it will be sent, under the name of the file they came out of; and
the reason one could not be used, in the same `invalid_fg`. A reader who cannot see what
their server was told cannot tell a setting that was ignored from one that was never sent.
Nothing at all where a project said nothing, which is most of them.

## Not here yet

Nothing consumes a definition; Step 24 is the consumer. `$/progress` is not asked for, so
"running" means the handshake returned and not that indexing finished -- a first question
can come back empty while rust-analyzer is still reading the project.

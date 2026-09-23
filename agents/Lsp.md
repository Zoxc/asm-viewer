# The language server

A language server, started over the open project's directory and asked two things about a
source position: where what is there is defined, and everywhere it is used. `src/lsp.rs` is
the client, `src/ui/language.rs` the state and the presses, `src/ui/language/worker.rs`
the thread that does the talking, and `src/ui/language_view.rs` the control in the top bar.

**Which program is the project's own setting** (`Project::language_server`, empty for
rust-analyzer). A project on a toolchain of its own is read by a server this app cannot
guess at, and the same box is where a wrapper like `ra-multiplex` goes; it is a plain value
in `project.toml` beside the name and the directory, and the Project view is where it is
typed. Nothing in `lsp.rs` names a program at all -- the default is
`languages::Language::Rust.server()`, with the rest of the per-language facts -- and the
failures it reports say "the language server" rather than a name this app did not choose.

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

The answer is the project's, a plain `trusted`, and **absent is no**: a project nobody has
been asked about writes no key. It is in the **session** and not in the project file, which is
the one thing about it that is not obvious. A project file is something a reader may check in,
and a `trusted = true` travelling with it would run a language server over a stranger's tree
without ever asking; the agreement is this machine's. The cost is the session's own timing --
it is written on the 30 s timer rather than at once -- so an unclean exit inside that window
means being asked again, which is the mild half of getting this wrong.

Agreeing happens where the question is asked, at the start it holds up; **taking it back is
the Project view's**, beside the program and the status, because a reader who cannot see the
answer they gave cannot change their mind about it. Taking it back stops the server too:
somebody saying they did not mean to let a program read this directory has said something
about the program reading it *now*, and a control reading "not agreed to" over a server
going through their project would be answering them with a lie.

What it is about is a *directory*, and the effect that follows the project is where that is
kept honest -- but only one of the four things it sees is the agreement being outlived.
The reader typing a new directory into the box has pointed **this** project somewhere else,
and the agreement was to the old place, so it goes. A project *arriving* brings its own
answer with it, out of its own session, and taking that off it would not only ask again but
write the `false` straight back into the session it was read from, since the open project is
saved as it changes. A project *saved* moves only where it is kept. And the mount is none
of those: the deps it mounts with are already the reopened project's, the restore being an
earlier hook of the same render. So the effect is handed what it last saw beside what it
sees now (`use_on_change`, `agents/UI.md`), and clears only where the file stayed and the
directory moved.

**The server belongs to the project.** Leaving the project stops it, even for a project
over the same directory, and the next project starts nothing of its own, as opening it fresh
would not. Another project may name another program, and it has its own agreement to give;
and a question asked in the project left must not be answered in the next one. Only the
`Stay` says a project was left: the two paths cannot, since the next project can have the
same directory. Within one project the directory is what counts. One the server is no longer
over ends it, and the settings go with it, both being read from that directory; a file that
moved on its own is neither. That file is Save (`ask_where_to_save`, the only thing that
puts a project somewhere else while the tree stays, and it does not move the `Stay`), and
stopping there threw away a server that had read a whole project for a gesture about where
a `project.toml` is kept.

It sees those two paths through a **memo** rather than reading the project at the root, and
the `Stay` beside them (`agents/Sidebar.md`).
The hook is called from `app()`, and the Project view's boxes write the open project on
every keystroke, so a read there would rebuild the whole window for each character -- the
cost `WindowBody` is a component of its own to avoid (`agents/UI.md`). The memo is read in
the hook's deps, which run inside the effect, so the effect is subscribed to it and the root
to nothing: a keystroke that leaves the file and the directory alone wakes neither.

The gate is in `start_server`, which both presses go through, and `run_server` is the half
that actually starts one; so neither the top bar's control nor the Project view's button can
grow a path around it. The question holds the directory and program it named rather than
working them out again when it is answered: what was agreed to is what was asked about, not
whatever the directory box says by then. Declining remembers nothing -- the answer was to
that press -- and a stop clears an unanswered question along with the server. **Leaving the
project** is a stop, so it clears one too; left up, "Start it" ran the program the last
project named.

`TrustPrompt` draws it **at the root, under the top bar**, and not in the Project view's own
section beside the other Start button. The control is pressed from wherever the reader is,
and a question drawn in a tab that is not on screen is a press that did nothing. It is a
band in the bar's own style rather than a window over the app, on a ground of its own
(`prompt_bg`), it names the directory because that is what is being agreed to, and it lays
out as nothing while there is nothing to ask. Drawn by the app and not by `rfd`, whose dialogs the headless runner cannot press:
the app answers questions about its UI with tests.

## The protocol, hand-rolled

Nine messages: `initialize`, `initialized`, the four questions about a place --
`textDocument/definition`, `textDocument/declaration`, `textDocument/implementation`,
`textDocument/references` -- one about a whole file, `textDocument/semanticTokens/full`,
one about the name under the pointer, `textDocument/hover`, and a reply to whatever the
server asks of us. A protocol crate would bring a type per request in the specification
and a runtime to drive them, for those nine. `cargo tree -d`
is unchanged by this step: `serde_json` was already in the tree, and the manifest comment
on it already covers a protocol rather than a file.

**One request is in flight at a time**, so there is no table of outstanding ids: a request
waits for an answer carrying the id it asked under. The four questions about a place are
one shape -- a place in, places out -- so they are one method, `Talk::places`, and what
tells them apart is an `lsp::Question` it takes.

**`Question` is the one type for "which question", from the link to the wire.** A link
carries the half of it that can be followed (`links::Link::asks`), the job carries the
whole of it, `Question::method` and `Question::params` are what goes out, and nothing maps
between them by hand. It splits by **what an answer is for**, which is what the consumers
are: `Question::Followed` is a definition or a declaration, one place for `ui::follow` to
open; `Question::Listed` is implementations or references, a list for the Locations panel
to draw. The answer splits the same way (`Reply::Followed`, `Reply::Listed`), so a question
of one kind cannot come back as the other's answer -- which the older pairing left to
convention, and which both consumers had an unreachable arm for, turning a real mismatch
into "nothing found".

**A hover is a place in and contents out**, so it is none of those four: it has its own
job, its own answer and its own parse. Not a fifth `Question` for a plainer reason as well
-- those are bucketed by consumer, and a hover is a third consumer. The pointer crossing a
line asks about every name on the way, and only the last of them is worth a round trip; but
none of them is a reader taking back the definition they clicked for, and a click is not a
reader taking back the name under their pointer.

**A kind is a consumer and not a question**, which is what `worth_doing` supersedes by:
`ui::follow` takes a definition or a declaration, the Locations panel draws implementations
or references, and the source pane holds one file's links. A reader asking one of the
panel's two has taken back the other, since the panel shows one at a time; neither takes
back a name being followed. The rule is written once, in `superseded_as`, whose `match`
over `LspJob` is exhaustive on purpose -- the `_ => true` it once ended with would let a
question added later queue behind every one of its own kind in silence. `worth_doing` then
keeps the last job of each kind, and everything `superseded_as` gives no kind at all.

**"Not now" is decided once, and so is "gone".** `Talk::asked` sits between `request` and
every question a reader asks: it turns the `-32801` and `-32800` codes into a null answer,
which every reader of an answer already takes as nothing found. `request` itself is left
raw for the handshake, which needs the refusal, and for `semantic_tokens`, where a refusal
is a question to put again. On the worker, `language::worker::asked` wraps every job that
says anything to a server: no server is a `Broken` answer, and a `Broken` conversation is
dropped there rather than in each arm. Both were copied per question before, and the copy a new
question forgot would be the one that leaves a dead conversation in `talking`.

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
server over `std::io::pipe()` and only starting one needs a program. That is also why
`Server` is not simply a `Talk<ChildStdin>`: what it adds is the process -- the handle that
ends it, and the stderr where a program that would not run says why. It **derefs** to its
`Talk` rather than forwarding each question, so a question added to the conversation is
askable of a server with nothing written here; the rest of `Talk` is private, the wire
format and the raw conversation being nobody's outside this file. `write_message` and
`read_message` are that wire format on their own, tested over `Cursor`s.

Things learned from rust-analyzer's own transport, each of which is a test:

- The header separator is a colon **and a space**. `lsp-server` splits on `": "` and calls
  anything else a malformed header, and dies.
- `initialized` must be the very next message after the `initialize` answer. Anything else
  first and the server gives up on the conversation.
- **The declared capabilities are four lines long**, and what is left out is the decision.
  Every request rust-analyzer makes of a client -- for configuration, to register a file
  watcher -- is opt-in through a capability, so declaring none of those leaves a
  conversation this app only ever speaks first in. Nothing is said about definitions:
  plain locations are the default and are what is wanted, and naming that would only be a
  chance to name it wrongly. Semantic tokens are not
  declared either, though they are asked for: rust-analyzer offers them and sends its whole
  legend to a client that says nothing, which was watched against a real one. The one thing
  asked for is progress, since it is the only account of a server that is still reading the
  project -- and, since links wait on that account, the only reason they ever appear. A
  server that asks something anyway is answered -- an empty configuration, nothing for a
  progress token, and "not a method this client has" for the rest -- because a server
  waiting on a reply is a conversation that stops.
- **The fourth line asks the server to say when it has settled**, which is the one thing
  the protocol has no way to ask. See below.
- **The third line is how a column is counted**, and it is a default worth refusing.
  See below.
- **The second line is the format a hover is written in**, which is the one default not
  worth taking. Measured against a real server, over the same name, both ways: a client
  that names none is answered `plaintext`, with the fences gone and the doc comment's list
  run together into one word (`one` and `two` arriving as `onetwo`); one that names
  markdown is answered with the path and the signature each in a `rust` fence, a rule, and
  the comment as it was written. So this is the one place where saying nothing says
  something wrong.
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

  **The last words are waited for as well.** Both pipes close at the same instant, so
  whether the line is in `said` when the failure is built is a race between the reader
  thread seeing EOF on stdout and the stderr thread seeing the bytes -- and reading `said`
  first, its lock held over the wait, is losing it about half the time. So the order is:
  wait for the program to be gone, then for its stderr thread, then read. That wait is
  bounded too, since stderr is inherited and a grandchild left behind holds the pipe open
  after the program itself has gone. What is kept is **bytes**, decoded once when they are
  read: a read of a pipe returns whatever is there, and decoding chunk by chunk turned a
  character the reads fell across into two replacement characters.
- The one notification worth keeping is `window/showMessage`, which is all a client with no
  capabilities is told when the server cannot make sense of the project; it goes to the log.
  A question asked before the workspace is loaded can also come back as InternalError with
  "file not found", which is neither of the two "ask again" codes and is left as the
  refusal it is; what the consumer makes of that is below.

## How a column is counted

A line is **1-based** everywhere in this app, the unit line information is in: the
`Lookup` a question goes out as and the `Place` an answer comes back as both. The protocol
counts from zero, so that conversion is in one place and happens once, and both halves of
it are in `src/lsp.rs` -- `asked_at` counts a question's line down as it goes on the wire,
and an answer's is counted up as it is read. Nothing outside that file holds a line the
wire's way.

The column is asked about. The protocol's own unit is a UTF-16 code unit, but a byte
offset is what the text itself is indexed by and what every column in the app is
(`src/chars.rs`). So the handshake declares `positionEncodings: ["utf-8", "utf-16"]`, in
that order, because the order is the preference and a server takes the first it knows.
**Every column crossing `src/lsp.rs` is then a byte offset into its line**, whichever the
server chose, and the app has one meaning for a column.

`positionEncoding` arrived in **3.17**, so a server that says nothing has kept UTF-16 --
and so has one that answers something this app never offered. Both are read as UTF-16 and
converted, which is the reading that costs a conversion rather than the one that trusts a
word nobody said. Never a failure: a server is not broken for being older.

**The type is what makes each conversion happen.** A column in the server's own units is
a `Wire`: a newtype whose range belongs to a module of its own, with one way in and one
way out per direction. A parser reads an answer's columns into one, and `Wire::bytes` is
the only way back to a `Range<usize>`; it asks for the `Encoding` the handshake agreed on,
and `Talk::back` is its one caller. So a parser builds a `Place<Wire>`, a `Hovered<Wire>`
or a `Token<Wire>`, and the byte-unit `Place`, `Hovered` or `Token` a caller is given
exists only on the far side of that call. Going out, `Talk::out` is the only thing that
makes a `Wire`, and `asked_at`, which writes a question's `position`, is the only thing
that unwraps one.

Both directions were a convention the compiler could not see. Each parser built the
server's numbers into a field whose doc said bytes and a loop behind it repaired them, so
a parser that forgot the loop compiled; and `asked_at` took a bare `u32` with the app's
own byte column in scope beside it, so `asked_at(at, at.column)` compiled too. Either
mistake is invisible until a line holds a character wider than a byte.

**Converting takes the line's text**, and `Lines` is what reads it: the question's own
file, and, for an answer, whatever file it named -- a definition in another crate, a
reference in a file no tab shows. Each file once per answer, through `source::read_text`,
the app's one rule for reading a source file; the read blocks, which is why every question
here is a worker's. The reader is an argument to `Talk::over` and not a choice made inside
it, so that a test hands over text instead of writing a file: `source::read_text` reads the
disk, not the cache a seeded file lands in. Where a file's lines are is found when it is
read, so the nth is a lookup: an answer naming a name used a hundred times in one file
would otherwise walk that file from the top a hundred times. `Lines` is asked with a
1-based line, like everything here but the wire, and counts it down itself: no caller does that
for it. A file that will not read leaves the number alone, which is the right answer for a
line of ASCII and the nearest one for the rest. **Where the server took `utf-8` nothing is
read for the wire at all**: the numbers are already the app's. That is `Talk`'s rule and
not `Lines`', since the Locations panel below reads its lines whatever the server chose.

The Locations panel's rows are read on the worker too (`references::of`), through **the
same `Lines` the answer's own columns came back on the wire through**: `language_work`
builds one per answer and hands it to the question and then to the shape the answer is
taken in, so a file an answer names is read once. The conversion itself is
`chars::utf16_of` and `chars::byte_of_utf16` and is written once.

An answer that names no column at all is column 0 and not no place at all, the line being
what opens the file.

**A path comes back spelled the way it went out.** The `file:` URI is written and read
by hand (`src/uri.rs`), and a round trip does not give back what it took: a URI's
separator is `/` and its path carries a leading slash no drive letter has, so `C:\x\y.rs` goes out as
`file:///C:/x/y.rs` and came back `C:/x/y.rs`. A `Document::Source` was compared as text and is
never canonicalised (`src/project.rs`), so on Windows every place followed through the
server was a second tab of a file already open, with the trail, the positions and the
bookmarks' `matching` split across the two. `path_of` puts the separators back. The drive
letter is what says a path is Windows', not a `cfg`, so the rule is the same everywhere and
is tested from either platform. It is `cargo.rs`'s rule too: a letter, a colon, then the end
or a separator. A looser one, any `/X:`, read a Unix `/a:b/x.rs` back as `a:b\x.rs`.

**One encoder, and it works on bytes.** The file manager call (`src/reveal.rs`) had its own,
which read a Unix path's bytes. The server's read text, so a name that is not UTF-8 went out
as `%EF%BF%BD`, and it wrote every `\` as `/`, so a Unix `a\b.rs` went out as `a/b.rs`. Now a Unix
path is its bytes, a Windows path its text, and `\` is a separator only on a path with a
drive. A UNC path (`\\srv\share`) has none, so it goes out as `file:///%5C%5Csrv…` where it
once went out as `file://///srv/…`; `path_of` reads neither back. `path_of` decodes to
bytes too, and spells a drive's path on those, so on Unix a name that is not UTF-8 comes
back as it went out. Elsewhere a path is text, and bytes that are not UTF-8 name no file.
A `Document::Source` is an `Arc<Path>`, so such a name opens the file it names, and every
question to the server about it goes out as those bytes. Only the debug info cannot match
it: its file names are text, so such a file has no code (`compiled_from`).

**The root goes out absolute.** The directory box is free text, and `.` is what a reader
who launched the app from their project types; a `rootUri` built from that names a place
the server cannot find, and what it says about that is a `window/showMessage` this client
only logs -- a control that turns green and every question afterwards answering nothing.
`rooted` is `path::absolute` and not the `fs::canonicalize` `src/cargo.rs` uses on Unix,
because nothing here has to match a spelling something else prints back. Resolving would
only cost: the reader's own spelling of their project, and on Windows a verbatim prefix
that no `file:` URI can carry. The process needs none of it: `current_dir` resolves a
relative directory against the same place. `${workspaceFolder}` in a project's settings
stands for the same absolute directory: a server resolves a relative path there against
its root, so the typed spelling would be joined onto itself. The UI hands the directory over
absolute already (`OpenProject::workspace`, `agents/Persistence.md`); `rooted` is what makes
the module right whatever it is given.

The app's own spelling is not canonical either -- a project directory as the reader typed
it joined with a Files row, or whatever the debug info said -- and the server's is. The
directory is made absolute with its `.` and `..` taken out, but by text, so one reached
through a symlink still spells one file two ways. `open_source_place` names the document by
the spelling an **open source tab** already has for that file, and by the answer's only
where no tab has one.

**Reducing a path is not the same call on both platforms.** On Unix it is
`fs::canonicalize`: a project directory reached through a symlink is the case the walk is
for, and only the filesystem resolves one. On Windows it is `path::absolute`, which collapses
`.` and `..` by spelling, leaves the prefix plain and asks the filesystem nothing.
`fs::canonicalize` there answers verbatim (`\\?\C:\work\app`), a spelling nothing else in the
app uses -- not the debug info's, not a project directory joined with a Files row, and not
what a `file:` URI comes back as -- and `Path` reads that prefix as a different component, so
reducing to it would spell one file two ways, which is what the walk exists to prevent.

So on Unix the walk asks the filesystem, on the UI thread. The answer's path is reduced once for
the whole walk, so the cost is one `canonicalize` per open source tab: measured at 11 µs a call
on a warm local directory, ten source tabs come to 0.1 ms, behind a round trip to a server
that answers in hundreds of milliseconds. **Nothing is remembered between walks.** A
canonical path goes stale the way a read file does -- a symlink repointed, a file created
where the lookup found none -- and `src/source.rs` had to grow `forget_under` for exactly
that. Saving 0.1 ms once per followed link does not pay for a second thing a build has to
remember to evict.

## The documents the app has open

**The file is opened first**, and that is a reversal. The app used to open nothing: it
shows what is on disk and edits nothing, so a `didOpen` looked like an overlay to be taken
off again and kept in step for no gain, with rust-analyzer reading the project's files
itself anyway.

It does read them, and the cost of waiting for it is the whole of the bug this fixed.
Measured over a two-file crate: opened, the file answers its names at **0.0s**; not opened,
the first answer that is not empty comes at **4.3s**, and on anything the size of a real
project it is far worse. The protocol is built the other way round from the assumption --
the client owns the documents it shows, and a server answers about the text it was given
until it is told the file has closed -- so opening is the normal path and reading the disk
is the courtesy.

**What is opened is what the reader has in tabs**, which is what an editor does
(`src/ui/opened.rs`): `Opened` holds the set and the run it was sent to, `use_opened`
diffs it against the source documents in the strip and the file the Source pane is
showing, each file once however many tabs show it, and a server that has been restarted
holds nothing so everything open is new. The pane's file is there for a symbol's tab,
whose source side is no tab's document: left out, it got links only while a source tab
had the same file open, links being asked for only in a file the server has been told
about. A server that has *stopped* leaves the app
holding nothing either: a build under no server would otherwise mark those files stale,
and the server started after it would be sent a `didClose` for a file it had never been
given. Measured, opening is nearly free: 41 files in **7 ms**, and one megabyte of the
server's memory against the 686 it takes to sit there.

A file is carried about **with the identifier it is to be opened under**, from the walk of
the strip to the send: one pass works out both, and the files to open, to close and to
hand over again are cut out of that one list.

**Only files the server is for.** A server answers about a file whatever language it is:
asked about a C file, rust-analyzer reads it as Rust and answers with what a Rust lexer
made of it -- three `struct` tokens and a `property` in a nine-line file, every one of
which this app would draw as a link and follow to nowhere.

**Which files those are is the project's to say**, since what the app knows is the program
and not what it serves: a box of extensions in the Project view beside the program, in
whatever spelling the reader types them (`c, h` and `.c .h` are one answer). Where they say
nothing it is the program's own: the one program this app knows by name is Rust's, and a
project that named its own gets asked about whatever it opens, that being the reader's
business. The identifier the file is opened with is `languages::Language::spoken` where the
app knows the language, and the extension itself where the reader named one it does not --
which is what the specification says to send, and which a server that does not know it
ignores.

**Both boxes are read at the press** and held with the running server (`Serving`), as the
program already was. What they say after that is a draft for the next start. Read live,
deleting one letter of `rust-analyzer` made it a program of the project's own, and every
file the app knows was opened with the rust-analyzer still running, then closed again when
the letter came back.

**Nothing is sent to a server that says it takes no documents**, which unlike the semantic
tokens beside it is asked and not assumed: `textDocumentSync` is in the specification,
every server answers it, and a `didOpen` to a server that declined them is a client it may
call broken.

**The one document the app has of a file is version 1**, always. It shows what is on disk
and edits nothing, so a version that counted would only ever count re-reads. Its text is
`source::read_text`'s, the one rule for reading a source file: a file with one bad byte is
sent decoded lossily, as the pane draws it and the columns are counted in, rather than
never sent, and a path that has become a fifo is refused rather than read on the worker for
ever.

**A file read afresh is closed and opened again**, which is the overlay's cost and the
whole of it. The server answers about the text it was handed until told otherwise, so a
build that rewrites a file under an open tab leaves it answering about the version before
-- names at columns that have moved, a hover describing what a line used to say. The app
re-reads in one place (`Sourced::forget_under`, a build and a scratchpad's build), and that
place marks the open files under the directory stale; the effect that keeps the server's
set in step sends the pair. Closed and opened rather than a change notification, because
the app has one version of a file to give and no history of edits to describe.

**The server's own diagnostics are turned off** in the handshake's options, since it
computes them for open documents and this app draws none: 41 notifications for 41 files,
read and thrown away. It needed no turning off while the app opened nothing.

## The project's own settings

`src/lsp/settings.rs`, and a file of its own because it has nothing in common with the
conversation: no message, no process, no stream. It is the one part of the client that
reads a file.

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
comments nor a trailing comma, so `jsonc-parser` reads the text and `serde_json` never sees
it. That was eighty hand-written lines that blanked both before handing the text on, and
the difficulty was always the strings: a `//` is half of every URL, and a string can end in
an escaped quote or hold a backslash before its closing one, so a pass that blanks comments
has to track every string in the file. Getting it wrong cuts a path short without a word,
which is the failure this feature exists to prevent. A parser that reads the format has
done that already, and a file with no specification is a poor thing to keep a reader for.

**JSONC and not JSON5**, which is what the crate takes by default: a name without quotes, a
single-quoted string, a hex number, a leading plus, a comma left out. No editor reading
this file takes any of them, so each is turned off (`JSONC`, `lsp/settings.rs`) -- a file
this app read and the reader's editor would not is the two disagreeing in silence about
what a server was told. A file with nothing in it is not an object and starts nothing; a
file that is not there is the ordinary case and says nothing at all.

Three bounds, all because the input is a file (`AGENTS.md`): the tree is built iteratively
so a name of ten thousand dots cannot overflow the stack, `DEEPEST` refuses a name of more
parts than the walk back out is written to recurse over, and the parse itself stops at 512
levels of nesting, which is the crate's own.

**An error starts nothing.** The check is in `Language::starting`, the transition a start
goes through, so neither press nor the agreement can grow a path around it -- the same
reason the trust gate is in `start_server`. A start that is refused is a state that
changed, so the transition says it by answering with no settings to start under rather
than by a flag beside them. It is reported as `Lsp::Failed`, which is where a
failure to start is already said.

The read is the LSP worker's (`LspJob::ReadSettings`, the shape of the build worker's own
read): reading a file blocks and nothing is read on the UI thread, and what it answers is
what a start has to carry. It happens in the effect that follows the project, at the **root**, and not in
the Project tab: that tab is unmounted while it is not the one on screen, where the
control in the top bar is pressed from wherever the reader is. One read answers both, and
it happens whether or not a server is ever started, since the view lists what it found
either way. The settings travel in the `Start` job the way `program` and `directory` do --
the worker thread may read no UI state. An answer is matched to the project by the
**directory** it was read in and not by a run: it is about a project and not about a
process. `worth_doing` keeps only the last read, since a directory typed a letter at a time
asks for one a keystroke.

## The process

Started and ended the way every program this app runs is, `agents/Process.md`: in a group of its
own, because rust-analyzer forks `cargo`, `rustc` and a proc-macro server of its own; stopped by
killing that group, taken out from under one lock so a second stop is a no-op; and every handle on
the one list `shutdown::before_exit` walks, so a server the UI has lost is still stopped when the
app comes down.

A stop **kills** rather than sending `shutdown`: a server that is indexing can take seconds to
answer that request, and a stop has to be over when it returns. rust-analyzer ignores the client's
`processId`, so nothing about the app dying would end it by itself. `Server`'s own `Drop` stops it,
`Child`'s neither waiting nor killing.

The stop is also how a worker parked in a read is let go: the pipes close with the process, so the
read ends instead of waiting on a server that will never answer. That is why the kill happens on the
UI thread and the worker is only told afterwards -- and why the handle reaches the app at the spawn
rather than at the handshake, the handshake being one of the reads a worker can be parked in.
`lsp::start` is the spawn and the handshake in one call, and it takes a `spawned` callback for
that reason alone: it hands the handle over between the two. That callback is the only place
it hands one out. `start` answers with the `Server`, which answers for its own handle
(`Server::handle`), because dropping the server stops the process: the two are not
separable, and a signature that returned them side by side said they were.

A handshake that failed asks the process how it ended, waiting `ENDING` for it to finish doing so,
and that is what tells a program that would not start from a server that stopped answering. A
process this app stopped answers "not ended by itself", which is the same answer as a server still
running: the app killing it is not the program refusing to run.

## The worker and what an answer is about

`use_worker`'s shape (`agents/Worker.md`): one named thread, two `async_channel`s, one
`spawn` draining answers, and the blocking half handed in so the headless tests never touch
rust-analyzer. It is the one worker that also keeps the **answer sender**
(`use_worker_answering`), because starting a server has to say the process is there from
inside the handshake it is still in the middle of.
Unlike the other three workers this one keeps something between jobs -- the conversation --
so `language_work` is a closure holding it. A `Mutex` and not `FnMut`, so the seam stays
the `Fn` the others are and the test harness fits unchanged; one thread calls it, so the
lock is never contended.

Two things say which server an answer is about, and they are not the same thing:

- The **run** counts starts and stops. `use_analysis` compares questions instead, but what
  an answer here is about is a process, which does not exist until the worker has started
  it. An answer whose run has moved is dropped.
- The **handle** is what ends that process. It arrives in `Spawned`, the moment the
  process exists and **before** the handshake, since a stop while the server is starting
  has nothing else to kill: a program that takes the pipe and answers nothing -- a wrapper
  pointed at a daemon that is not there, a name that is no language server -- would
  otherwise hold the worker in that read for the life of the app, with every later start
  queued behind it and the control saying "starting" for ever. **`Spawned` is the only
  answer carrying one**, so there is one rule and not two: a `Spawned` arriving for a dead
  run is **stopped**, not dropped -- a handle dropped instead of stopped is a server
  nothing can ever find again (`pad.rs` has the same rule for a run's process). It is also
  what closes the race the other way: a stop pressed before the worker has even spawned
  finds nothing, and the `Spawned` that follows it is for a run that has moved, so the kill
  happens there. `Started` then says only whether the handshake succeeded, and the handle it
  turns `Lsp::Starting` into `Lsp::Running` with is the one already written there -- both
  answers come down the one channel and `Spawned` is sent first. The handle is a field of
  the state that has it and not one beside the state, so "there is a server" is written
  down once: `Lsp::Off` and `Lsp::Failed` have none to hold, and a `Spawned` that finds one
  of them stops what it was handed.

Which **question** an answer is to is a third thing, and the run cannot stand in for it: a
run lasts as long as the server, so two questions inside one is the ordinary case. The two
travel together as a `Ticket` -- the run, and an id minted per question -- which the `Ask`
and `Hover` jobs carry and their answers copy back. `Follow`, `Located` and `Hover` each
keep the ticket of the question they are waiting for, so "is this mine" is `== ticket` in
all three and neither half can be compared without the other. `worth_doing` drops the
duplicates still queued; the ticket is what makes an answer to a question the worker had
already taken land on nobody.

A ticket does not say the server is still the one the app has, so those two answers are
**run-checked at the root** as well. A question asked before a stop is held until an
answer comes, and a server can write its answer before the stop reaches it; unchecked,
that answer landed in the project the reader had switched to. One from a server that has
been stopped is taken as naming nothing, so the asker gives up on its question. The
answers that carry no ticket are checked through the same `is_run` and dropped -- a file's
names, and a file the server has just been told about.

Both asks take the run from `current`: one `peek` says whether there is a server and
which, and a `u64` comes back. A function and not a line in each, because the state holds
a process handle and the project's settings -- cloning it to read a `bool` and a `u64`
copied all of that on the UI thread, and a hover is put again at every pointer stop.

What the server says unasked comes back on a bounded channel the `Start` job carries,
under the run it was started in. Two things are said over it: whether it is working, and
whether it has settled (below). Bounded so that a server reporting progress in a tight
loop cannot outrun the app: the reader thread waiting is the whole of the backpressure.
**Neither is a state**, both are what a server that is there is doing, so they are carried
by the two states that have one and cannot be written beside a state with none. A
handshake's answer and a first `$/progress` arrive in either order without one undoing the
other, since the remark is kept across the change of state; a remark about a server the
app has let go of has nowhere to land.

`worth_doing` drains the queue to the last question of each kind, keeping every start and
stop: a reader clicking twice wants the second answer, a reader who asks for a name's
references has not taken back the definition they asked for, and a press is never dropped.
A question asked
while the server is still starting is sent all the same -- it queues behind the start and
is answered once there is somebody to answer it. If the start failed, or a conversation
ended with questions still queued behind it, each is answered `Broken` all the same: the
asker holds its ticket until an answer comes, and one that never came left the Locations
panel "Finding..." for good. `Language::failed` writes only over a server still starting
or running, so the control keeps the first reason and not "there is no server to ask". A
file opened or closed is no question and answers nothing, except that the conversation
ended there (`LspAnswer::Untold`): the worker drops the server, and without it the control
went on saying Running. A question with the server off is not sent: the control is what
starts one.

Leaving the project ends its server, from a side effect on the project's file and directory
rather than from `clear_project`: the server was started over that directory, and a
directory typed into the Project view is a different project's as far as it is concerned.
Saving the project is the change that ends nothing, the directory being where it was
(above).

## The control, and what the Project view says

In the top bar (`src/ui/language_view.rs`), left of the two history buttons, drawn as a
link and three letters -- what a language server is asked here is where a name leads, and a
link is that question rather than the machinery answering it -- and built on `bar_pill`,
the same frame `NavButton` takes as a square, but **named and bordered** rather than an
icon alone: it is the only thing in the app that starts a process
the reader did not ask for by name. It says `LSP` and not `rust-analyzer`, which is what
that corner has room for beside two chevrons and is the part of the app being named rather
than the program; the program's own name is in the tooltip and in the Project view. Which
program that is is the project's to say (`OpenProject::server`), so a project on `clangd`
is offered a `clangd` to start, and the name is passed to `Language::words` rather than
written there. A started server is named by what it was started as (`Serving`), since the
box may have been typed into since.

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
in `invalid_fg` when it is a failure, with a Start/Stop button beside the heading. It
presses `toggle_server`, the one the bar's control and the chord press, so a reader
already in that view need not go looking for the bar, the same question comes first where
the directory has not been agreed to, and a rule added to the toggle reaches all three. A
tooltip is gone the moment the pointer moves, and a reason worth reading is worth reading
twice.

Under that line is what the project's own settings gave the server, one row per setting with
the name and the value as it will be sent, under the name of the file they came out of; and
the reason one could not be used, in the same `invalid_fg`. A reader who cannot see what
their server was told cannot tell a setting that was ignored from one that was never sent.
Nothing at all where a project said nothing, which is most of them.

## What the two answers open

`src/ui/follow.rs`. A press on a call sends the row's place and the name's column, through
the one `Lookup::at` every question about a name is built with, and what its answer opens is
decided **at the press** and kept -- `Asking`'s rule once more. Two workers stand between
the press and the tab moving, so by the time the answer lands the reader may have moved on
and Ctrl may no longer be held; what was asked for is what was asked for.

The **tab** is half of that. `Reach::InPlace` means the tab the press was made in, and a
reader who presses a chip while the server is indexing is in another one by the time the
answer lands; so the asking tab is kept with the question and raised to take it, and an
answer to a tab that has closed opens nothing -- nobody is waiting for it. Resolved against
the tab on screen instead, as it was, the definition replaced what an unrelated tab showed.

One question is held, by the ticket it went out under. A reader clicking twice wants the
second answer: `worth_doing` drops all but the last still queued, and the id drops the
answer to one the worker had already taken. Matched by the run alone -- as it was -- the
first click's places opened under the second click's reach, and the second click's answer
was dropped as an answer to nobody. The Locations panel holds its two questions the same
way.

The place is opened through `land` like every other door: the source pane on the line, both
panes owed the scroll, and the place on the tab's trail so Back returns to the call --
which is what a source `Stop` carrying a line is for (`agents/UI.md`). What `land` does not
do is say which line the assembly side follows, so the drive is written beside it, under
the place the tab is **at** and never under the file.

The caret goes on the **name** and not at the start of its line, which is what the answer's
columns are for. Their start travels as the `Landing`'s `columns`, an empty run, so
`line_pick` leaves a caret there and selects nothing -- the same field a search hit selects
its match with (`agents/Sidebar.md`). A name defined in the file the tab already shows takes
the other path through `land`, which marks the line itself and leaves no landing; what keeps
the column there is in `agents/Panes.md`, under the doors.

**That column is the place's own.** The server counts in bytes and so does a pane, so
nothing reads the line to plant the caret -- a line of a file the reader has usually never
opened.

**Which names are links is the server's to say, and it is asked once per file.**
`textDocument/semanticTokens/full` classifies every name in a file at once -- one request,
about ten milliseconds warm against a file of a thousand lines -- where asking about each
name in turn would be a round trip apiece down a conversation that holds one question at a
time. The answer is a flat array of numbers, five per token and every one a delta from the
token before, decoded in `lsp::tokens`; what the indices in it *mean* is the legend the
handshake's reply carried, which is why `Talk::initialize` keeps that reply instead of
dropping it. **Never read an index without the legend**: the order is the server's own and
a new version renumbers it.

The rule turning that vocabulary into links is `src/links.rs`, framework-free and tested on
its own. Two things in it were measured against a real rust-analyzer rather than reasoned
out, and both are easy to get backwards: `defaultLibrary` marks a name from **std**, which
has a definition like anything else, so excluding it would kill every link into std -- the
marker of a built-in type is `builtinType`; and there is no `definition` modifier at all,
rust-analyzer folding its own into the standard `declaration`. A name the rule keeps but
cannot follow -- where one is defined -- is kept all the same, since the row's menu is
offered there too. That is why one token has **three** answers and not two: no name at
all, a name with nothing to follow, or a name and the question following it asks.
`Links::of` spells the first as a token it drops and the other two as a link whose `asks`
is `None` or a question.

**What the legend says is asked of it once per file, not per token.** Which of its type
indices are names is a table (`Legend::kinds_named`) and each of the two modifiers is a
bit (`Legend::bit`), both taken at the top of `Links::of`, so a token costs an index and
two `&`s. The legend is fixed for the life of the conversation and a file is thousands of
tokens; asking by name per token was a scan of the type list and of the modifier list
apiece, on the worker, every time a file was shown.

An item in a trait `impl` is the one name that asks a different question. Its *definition*
is itself, so `textDocument/definition` on it goes nowhere the reader is not already; its
*declaration* is the trait's. `declaration` and `trait` together say so, and that is the
only thing `lsp::Followed::Declaration` is for. The two genuinely disagree elsewhere,
which is why neither can replace the other: a **call** to a trait method is defined in the
`impl` that runs and declared in the trait, and a reader following it wants the code that
runs.

**The question is only put to a server that has finished reading the project**
(`Language::ready`, `src/ui/linking.rs`). Not for tidiness: a request holds the one
conversation until it is answered and there is no timeout on it, so a whole-file question
put to a server that is still indexing would park the worker and every click queued behind
it. Waiting also answers what the beat before looks like -- no links, because nothing has
said there are any -- and the effect asks again when the server says it is done, so a file
opened during indexing gets its links without the reader doing anything. This is why
nothing reads `started()` any more: a pane that lit links as soon as a server *started*
drew them through the minute it spends reading the project, and every one was a click that
did nothing.

The rule the other way round: a server that has **stopped or failed** leaves nothing
drawn. The rows are handed the links as data, so without this every name went on lighting
after the reader stopped the server, after it died, and after they left for another
project -- each a click that did nothing, which is the very thing the gate above is for.
One that is *working* keeps its links: they are still the right names while it reads more
of the project.

A **references** answer goes to the Locations panel instead (`agents/Sidebar.md`), and is
asked about the same place a definition is: the row's file and the name's own first column,
at the right-click rather than the press. All three questions a click cannot ask -- go to
definition, find references, find implementations -- are one `name_menu`
(`src/ui/locations.rs`), beside the menu of what the *line* was compiled into. The row finds
the name under the pointer, hands over a `NameAt`, and knows nothing else about them: which
questions a name can be asked is written where they are asked. It comes back grouped and with each line's text, both
done on the worker: the reply is `Reply::Listed` where a definition's is `Reply::Followed`,
and both carry what their lines said, reading a line being a file read and belonging on the
thread that already blocks. The
reader handed to `references::of` reads with `source::read_text` and not with a
`read_to_string` of its own: a path a server answers with is file input, and a second rule
for what a source file is would be a directory or a fifo opened on this thread, and a line
the pane draws that the panel leaves blank. It
follows the **name** and not the link, so the row a function is defined on offers it too --
there is nothing to follow there, and it is where a reader asks what refers to it. What comes
back is a place in a file and not a symbol, so a row of it opens a source-driven tab
through the same `open_source_place` the definition uses -- the same landing, the same
drive, and the answer's columns selected there, where a definition leaves only a caret at
their start.

The three are keys as well: `F12`, `Shift+F12` and `Ctrl+F12` put them about the name under
the **caret** rather than the pointer (`agents/Panes.md`). The `NameAt` is built by the same
rule either way, so a key and the menu item beside it cannot ask about two different places.

**An empty answer is nothing found; a refusal is not an answer at all.** A question about
a place takes both as nothing found -- a click is a question, not a promise -- and the
`-32801` and `-32800` "not now" codes arrive there as an empty answer. The question about a
file's names keeps them apart. `Refused` is where a question put before the workspace is
loaded arrives, those two codes and the InternalError "file not found" rust-analyzer gives
for a file it has not read; it goes to the log, leaves the control alone -- a server that
refuses is a server that is answering -- and is held as a refusal, drawn as no links and
put again once the server says it has gone quiet (`Linked::forget_refusal`).

Filed as an empty answer, as it was, it cost that file its links for the life of the
server. `ready()` is true for a beat before the first `$/progress`, the handshake's reply
and that note arriving in either order, and the file on screen when the control was pressed
is asked in exactly that beat. Asking again only where the server has been busy and gone
quiet is what keeps one that goes on refusing from being asked in a loop.

Only `Broken` still says the server stopped answering, which is the one thing the control
has to show.

`$/progress` says a server is reading the project, and "running" means the handshake
returned rather than that indexing finished -- so a first question can come back empty
while rust-analyzer is still working, and the reader presses again.

## What "ready" is, and what it was

**The gaps between progress tokens are not readiness.** A server opens and closes a token
per piece of work, and rust-analyzer runs eight of them in the first two seconds of a
forty-module crate: `Fetching`, `Building CrateGraph`, `Roots Scanned`, `Building
compile-time-deps`, `Loading proc-macros`, `cachePriming`, several of them twice. The set
of open tokens empties **nine times** in the first four seconds. Every one of those was a
moment `ready()` was true and the file on screen was asked about, and the answer was as far
as the server had got.

That is what the reader saw as *no links at all*. The first question is put in the beat
between the handshake's reply and the first progress token, when the file is not in the
server's own view of the project yet and it refuses; the refusal is dropped and put again
at the first gap, half a second later, and answered with **no names**. An empty answer is
the answer, so nothing asked again, and the file kept no links for the life of the server.
Closing the tab and opening it afresh changed nothing: what is held is keyed by the file,
and the file had been answered.

**So the server is asked to say.** rust-analyzer sends `experimental/serverStatus` to a
client that declares `experimental.serverStatusNotification`, carrying `quiescent` -- it
has read what it is going to read. Measured over the same crate: at `quiescent` the file
answers 540 names, and forty-five seconds later it answers 540. The same run without cache
priming, which a project's own settings may turn off, is quiescent at 1.0s where the
`cachePriming` token never arrives at all -- which is why the token is not what is watched
for.

**Nothing in the protocol says any of this.** `initialize` and `initialized` are the whole
of its lifecycle, and after them a server is simply expected to answer. `$/progress` is
defined as the progress of an operation, with nothing said about having none. The nearest
the specification comes is two error codes for a server that cannot answer yet
(`ContentModified`, `ServerCancelled`), and rust-analyzer uses neither here -- it answers,
with fewer names. Every large server has invented a notification of its own for this;
clangd's and the Java server's are different again.

**A server that says nothing is judged as it was**, by its progress. The capability is a
free-form `experimental` bag the specification allows anything in, and a server with no
such notification simply never sends one -- so what is held is `Option<bool>`, "what it
last said, if it has ever said anything", and `ready()` reads the progress only while that
is `None`. There is nothing to detect it by: rust-analyzer's `initialize` reply lists a
dozen experimental capabilities of its own and this is not among them, the notification
being something it *reads* rather than offers.

**An answer given before it settled is asked again.** `Linked::forget_answer` drops what
is held when the notification turns true, and the question goes out once more. Not
belt-and-braces: measured against a real server, the same file answered 492 names before
it settled and 540 after, and the difference is not only in the count -- seven names came
back `const` that are not const, five parameters were not parameters yet, and five
`builtinType`s were something else. `src/links.rs` draws a `const` as a link and a
`builtinType` as nothing, so the early answer is not a smaller set of links but a **wrong**
one, on names that lead nowhere.

**The question in flight is dropped with it.** The notification and the worker's answers
come down two channels with nothing ordering them, so an answer asked for before the server
settled can arrive after the news and find nothing held to drop. So the question carries a
`Ticket`, as a hover and a place do, and `forget_answer` lets go of it: the late answer names
a question nobody holds, and the one asked again is told apart from it though it is about
the same file in the same run.

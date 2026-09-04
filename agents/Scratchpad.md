# Scratchpads

A scratchpad is one source file the reader types into, built by cargo and opened as a binary like
any other. The first half is the model and its storage (`src/scratchpad.rs`); the second is the
view, the per-pad state and the one worker thread (`src/ui/pad.rs`, `src/ui/pad_view.rs`).

## The model and its storage

**A scratchpad is a generated cargo package, and the package is the storage** (`src/scratchpad.rs`).
Each scratchpad is one directory under `scratchpads/`, under the same base `projects/` and
`settings.toml` use. It holds exactly what cargo needs: a `Cargo.toml` naming the crate, its pinned
`edition` and its `[dependencies]`, and `src/main.rs`. Nothing describes a scratchpad *beside* that,
since every field of the model is already a field of the package. So `load_from` is the exact
inverse of `write_to` rather than a second format that could disagree with what cargo is handed.
Both files go down through the same `.tmp` + rename, which the source earns: `src/main.rs` is the
reader's document. The manifest carries an empty `[workspace]`, so a scratchpad is its own workspace
root wherever the state directory turns out to be. A scratchpad belongs to the **app** and not to a
project: it lives here beside `projects/` rather than inside one, `Pad` is not one of the states a
project switch closes, and a pad open in one project is the same pad in the next.

**A pad is filed under an id, and the id is never shown.** `PadId` is what the directory, the order
and the app's own table are keyed by; `Scratchpad::name` is what the reader calls it. The two are
separate so that a rename is a value changing and not a directory moving. That separation is what
buys everything below it: a name may be empty, hold spaces or be written in any alphabet, two pads
may be called the same thing, and the name box is an ordinary bound box with nothing to apply,
nothing to refuse and no gesture to discover. The id gets `ProjectId`'s treatment all the same: a
newtype whose `Deserialize` goes through the checked constructor, because it is interpolated into a
path *and* read back out of two files a user can edit, the order beside the pads and every pad's own
`Cargo.toml`, where it is what `[package] name` says. `check_name` is the one check, and the
crate-name rules it applies are strictly stronger than what a safe path component needs. It has no
`Display`, deliberately. That is also what gives the enumeration its rule: `Scratchpad::load_from`
answers `None` for a manifest whose crate name is not an id, so **a directory `load_from` answers
for is a pad and anything else is not**, repaired at the point of use and never on load.

**The name lives in the package, under `[package.metadata]`**, the one place cargo reserves for a
tool of its own and ignores itself. So "the package is the storage" still holds: nothing describes a
pad beside its own directory, and `load_from` is still the exact inverse of `write_to`. It is *not*
in the order file beside the ids, which is `recent_projects`' rule for a project's name: a copy
there would be a second one to keep in step with the one the reader edits. A new pad is made with
**no name at all**, an empty one being a real answer and not a missing one, and what stands in for
it on screen is the UI's to decide. Nothing is written into the package until the reader has said
something.

The crate name being the id rather than the name has a second payoff: **a rename does not move the
artifact**. `reopen_binary` keys on the path cargo named, so a pad renamed between builds writes the
same executable rather than leaving the last one open beside it.

**Which pad opens is an order, `recents.toml`'s shape again**, in `scratchpads/recents.toml`. It
sits beside the pads rather than at the top of the state directory, so it is not a second file to
tell apart from the projects' one, and it is a file where every sibling is a directory, so the
listing steps over it with no special case. `PadOrder` is `Recents` verbatim, over ids: the front is
what to open, `touch` answers whether anything *moved* (which is what keeps a startup that reopens
the front pad from writing a file), and nothing prunes itself on load. **`pads()` is the order's ids
then the pads it does not name**, in id order. Each row carries the name out of that pad's own
package, read at the moment the list is asked for, which is what lets the panel draw a pad nothing
has ever opened. That second half is the difference from `recent_projects`, which lists only the
projects a reader has opened: this is the list a reader picks a pad from, so a pad that fell off the
end of `MAX_PAD_RECENTS` or was made outside the app has to be reachable. A pad is remembered when
it is **opened**, and only if there is a directory for it, which keeps the "nothing is written until
there is something to say" rule: the pad a first run holds is in memory until something is typed
into it.

**A new pad is `new_pad`, and the claim is a `create_dir` that fails rather than opens**: the first
free `pad-N`, `ProjectId::anonymous`'s shape and bound, stepping over an id another copy of the app
already took. Unlike an anonymous project it **writes the package at once**: pressing New is a
deliberate act, and a claimed directory with no package in it is not a pad and the listing would
repair it away. The stem is `DEFAULT_ID`'s and deliberately without its number. The pad a first run
opens is `pad` and New makes `pad-1`, `pad-2`, so New can never hand out the id of the pad the app
is already holding. It could if that one were `pad-1`: a pad nobody has typed in has claimed no
directory for the `create_dir` to fail on.

**There is no rename operation**, and that is the point of the id: renaming writes
`Scratchpad::name` and the ordinary per-change save puts it on disk, exactly as typing in the source
does.

**A delete is the one thing here that destroys what the reader wrote**, so it is behind a question
(below) and the module's half of that is being narrow. `delete_pad` builds the path out of the id
alone, a checked crate name that can be neither `..`, nor a separator, nor absolute, and then
refuses a directory `load_from` no longer answers for -- so a `remove_dir_all` can only ever reach a
directory holding a manifest this module wrote. `symlink_metadata` is what makes that the directory
itself rather than whatever a link put in its place, and a pad with no directory is already deleted
and says so. What goes is the whole directory, cargo's `target/` included: the package is the
storage, so there is no part of a pad anywhere else. Nothing goes back to the order file --
`PadOrder::forget` is about the list on screen, and an id whose directory has gone is one `pads_in`
already steps over.

A dependency is a `(name, version)` row and the **version is required**. A `*` is refused with its
own reason, since a requirement whose answer changes with the day is the one thing a scratchpad must
not have. Rows are checked against two grammars (a possible crate name, a possible version
requirement) and never against crates.io: whether a crate exists is cargo's answer. Every bad row
comes back as `(index, Problem)` so the editor can mark all of them at once, a repeat of one crate
included, since `[dependencies]` is a table and the second row would otherwise silently win. A
scratchpad with a bad row **refuses to write** rather than generating a manifest that differs from
what is on screen. **Building is blocking and belongs on a worker thread**, exactly as `open_files`
is. Running cargo and reading what it said is `src/cargo.rs`, shared with the project's own build
(`agents/Sidebar.md`); `build_in` writes the package, calls it, and narrows what comes back to the
one binary a generated package has. The artifact path is what cargo *named*, never
`target/debug/<crate>` derived from the name and the profile, which a `CARGO_TARGET_DIR`, a config
above the directory or an executable suffix each make silently wrong. Turning that stream into an
answer is a pure function of cargo's stdout, stderr and exit status, which is what lets a failed
build be a test over a canned stream. Three answers, not two: the compiler said no (with cargo's own
stderr kept, since `no matching package named ... found` is said there and nowhere else), or nothing
was compiled at all. `Build` is still the pad's own type, since two of its answers -- a bad
dependency row and a package that would not write -- are about a generated package and mean nothing
to a workspace.

**Running is the artifact and not `cargo run`.** `run_in` spawns the executable `build_in` already
asked cargo to name, in the scratchpad's own directory with a null stdin. Re-entering cargo would
redo resolution to arrive back at that same path, or could arrive at a *different* one (the reader
has usually typed since, so what ran would not be what the diagnostics describe). It would
interleave cargo's progress into the stream the reader is reading as their program's output, and it
would make stopping meaningless, since killing a `cargo run` kills cargo and leaves its child with
nothing holding it. What the app is handed back is a `Running`, whose one job is `stop`, since
`Child`'s own `Drop` neither waits nor kills, so a run abandoned rather than stopped goes on running
with nothing that could ever find it again. `stop_all` is the same thing for every run at once, off
a `static`, because the window's close hook can read no state, `Saves`' reason exactly, and it sits
beside `flush` in `main.rs`.

**A run is a process group, so the stop reaches the grandchildren too.** A scratchpad is a buffer
someone is experimenting in and `Command::new` is an ordinary thing to experiment with. A stop that
killed only the process this app holds a handle for would leave the rest running with nothing that
could ever find them: the grandchild's pid was never anywhere but inside the program that is now
gone. `Group` is that one idea with two implementations and the same three moments: something before
the spawn, something taking hold of what was spawned, and a kill. It lives in `src/process.rs`,
having moved there when the language server needed the same thing for the same reason
(`agents/Lsp.md`). On Unix it is
`Command::process_group(0)`, std's own, so only the kill needs a crate, and
`libc::kill(-pgid, SIGKILL)`, the group being the child's own pid and the negative guarded, since
`-1` is every process this user may signal. On Windows it is a **kill-on-close job object**, created
and assigned right after the spawn, and closing the app's only handle to it is the kill. The sliver
between the spawn and the assignment is accepted rather than bought back with `CREATE_SUSPENDED` and
a `ResumeThread`, for a window a scratchpad's program does not use, and a job the system refuses
leaves the stop exactly what it was. The child's own kill stays, under the same lock and after the
group's, as what a refused job or a third platform still gets. The **reap is untouched**, since the
group changes who dies, not how the end is noticed, and every other way a run is stopped
(`stop_all`, the rebuild, the next run, the window closing) goes through the same `stop` and
inherits it. What is tested is the group being real: the pgid the kernel reports for a run is the
run's own pid, and not the one it would have inherited. What is inside the group once the program
starts forking is the same fact one step on, and is judged by hand.

**Output is streamed, not collected**, which is the whole difference from `build_in`'s
run-it-and-return-the-output shape: a program that prints and then loops for ever has said
something, and a value returned at exit would never say it. Two threads, one per pipe, hand each
line to a callback as it arrives; whichever finishes last reaps the process and emits the one
`Ended`. So a run is over when both pipes are at the end **and** the process is reaped. A program
that hands its output to a grandchild outliving it shows as still running, which is the honest
answer, since the output is still coming. The reap `try_wait`s on a poll rather than `wait`ing,
because holding the `Child` is exactly what would make a stop wait for the process it is killing.
**Three bounds, and each is a different failure.** `MAX_LINE` (4 KiB) cuts a line with no newline in
it, so a program writing megabytes in one line is still *delivered* rather than accumulated.
`MAX_OUTPUT_LINES` (5000) is what is kept, oldest first out, with `RunOutput::dropped` so the view
can say the story is missing its beginning; it is a line cap and not a byte cap, because the view is
a list of rows and a byte budget would make the row count depend on how long the lines happened to
be. And the app's own `RUN_EVENTS`-bounded channel is backpressure that reaches the program itself:
a full channel blocks the pipe thread, which fills the pipe, which blocks the writer.


## The Scratchpad view

**The Scratchpad page** (`Tab::Scratchpad`) is the pads there are down one side and the shown one
beside it: its source, its crates, its build and what the compiler said. It is a *view* for the
reason the settings page is: there is one of it, it resolves against no object, and neither code
pane could draw one. That there are many *pads* does not make it many views: the pad list is the
Scratchpad view's own side panel, because the content area's strip is deliberately not the place for
a second document list (a chip there is a *place in a binary*). What it **builds** needs no rule at
all: the executable goes through `open_files` and its functions are ordinary tabs.

**A delete is asked for, and a row is where it is asked from.** A right-click on a pad's row offers
one item, and the item deletes nothing: it writes `Pads::confirming`, and the popup that field draws
is the question -- `RescuedPopup`'s `Popup` in a second place, dimming what is under it and taking
Escape or a press outside as no. Not the × a dependency row has: a × there is one press away from a
list one row shorter, and this is one press away from the reader's own source being gone. The
question names the pad and the path its package is at, since if there were anything to get back that
is where it would be, and there is not. `confirming` sits beside `refused` at the root for the
reason that one does, and a delete that fails is what `refused` then says -- which is why it holds
the whole sentence now rather than a `Failure` the panel puts a word in front of.

**Letting go comes first and the disk second.** `request_delete_pad` takes the pad out of the table,
out of the order and out of the buffers, drops its save baseline, and only then queues
`PadJob::Delete`. That order is what makes an answer about a deleted pad harmless: the worker is one
ordered thread, so a build in flight finishes against a directory that is still there, and what it
answers arrives for a pad nothing holds and is dropped -- which is how a build the reader deleted
their way out of does not go on to open its artifact. A queued save of that pad is superseded by the
delete, having nothing left to write to. The pad's program is stopped, the directory it was started
in being about to go; a run still forking is stopped where it lands, its handle arriving for no pad
in the table. **There is always a pad to show**: the next in the order takes over, and when the last
one goes the table comes back to the pad a first run holds, opened like any other so that nothing is
written until something is typed into it. That `Open` is queued *behind* the delete, or it would
read the directory the delete is about to remove.

**That panel is a fixed width** (`PAD_LIST_WIDTH`) and not a `ResizableContainer`, unlike the two
splits in this app a reader can drag. A dock tab that is not the active one in its panel is
unmounted, and a `ResizablePanel` forgets its size on unmount, so a draggable width here would need
a number kept at the root the way `SplitRatio` is, for something nobody has asked to drag. Its rows
are a plain `ScrollView`, the History list's shape rather than the symbol list's, there being a
handful of one-label rows, and each draws the pad's **name**. **`pad_label` is the one place that
decides what that is**: the name the reader gave it, or, for a pad they have not named, the id in
angle brackets, `<pad-3>`. That is `<entry point>`'s device in a second place, and it is the whole
of why an id may be drawn at all: in brackets it is plainly the app's word and not a name someone
chose, where a bare id would be offering itself as one. A flat "Unnamed" was the alternative and is
worse: three fresh pads would be three identical rows. **The name box is an ordinary bound box**,
the project view's exactly: it writes into the shown pad's own `Scratchpad::name` on every keystroke
and the save effect writes the package, because nothing is filed under the name. Its placeholder is
that same label, so an empty box says what the pad is called in the list beside it, and typing
replaces it where a seeded name would have to be cleared first. The one thing the panel can be told
no about is New, whose failure sits under the list as `Pads::refused`. That is at the root and not
in the view, since an answer that landed while the reader was in another tab still has to be there
when they come back.

**Its editor is freya's own `CodeEditor`**, which the read-only source pane rejected
(`agents/Panes.md`). Its two objections were about painting and scrolling a listing from *outside*;
the first does not apply to a pane the reader is typing in, since the one line the editor
backgrounds is the caret's. The second still does. Its scroll lives in its own `CodeEditorData`,
`pub(crate)` and with no controller to hand in, so nothing outside that crate can move it. That cost
nothing for as long as the only thing moving in this pane was the reader's own typing, and it is
exactly what the diagnostic jump below cannot do. What comes with it is a cursor, a selection, an
undo history, the clipboard, IME preedit and an incremental tree-sitter re-parse per keystroke. Two
things stay ours. The colours are mapped onto the palette (`EditorTheme` beside the
`EditorSyntaxTheme` `Palette::syntax` already answers for). The font: the component takes **one**
family where everything else takes a chain, and the rest of the chain arrives by inheritance from
the box around it, since freya appends a parent's families behind an element's own. Its line height
is `code_row_height()` reached through the multiplier it wants, with half a pixel of slack because
it multiplies and floors. The editor's `SyntaxBlocks` is `HIGHLIGHTED`'s hazard in a second place:
colours resolved in at parse time, and `set_appearance`'s clear cannot reach inside a
`CodeEditorData`. So an effect keyed on the appearance re-sets its theme and re-parses.

**One worker thread owns every pad's directory.** Reading a scratchpad back, writing the package,
listing the pads, claiming a new one, moving a renamed one and `cargo build` are all blocking, so
all of them go to one `std::thread` fed an `async_channel`, `use_analysis`'s shape. It is one thread
and not several because the point is not only that the UI thread stays free but that a directory has
a single writer, so a save cannot land inside the build that is reading what it writes. **Saves
supersede, per pad, and builds never do**: a keystroke is a save, so the loop drains its queue while
what it holds is one, and whatever is behind it is either a newer save or a build that writes the
package itself. The **per pad** is a correctness rule and not a refinement. Keyed on nothing, a save
of one pad would be dropped in favour of a job for another, and that pad's package would quietly
fall behind what is on screen, since the pad that lost the write is the one nobody is looking at. So
`superseded` replaces a save only while the job behind it names the same pad, and hands a job for
another pad back to a hold-back queue rather than stepping over it. That a build of one pad delays
another's save is accepted: the reader types in one pad at a time. Two builds cannot start at once,
on the button (`enabled`) and in `request_build` both, because a build takes seconds and a second
job queued behind the first would compile bytes that have since changed.

**Everything the app holds about a scratchpad is per pad.** `Pads` is the table of them and which
one is shown; `PadState` is one pad's own, and every field it has (what was read, what is being
built, which run is going and what it has written) was already about one pad. A pad is in the table
from the moment it is first shown and never leaves, so `Pads::state()` is never absent and no call
site grows an `Option`. **Runs are per pad**: an event carries the pad beside the run number, so a
program started in one pad goes on running and goes on writing into *its own* list while another pad
is on screen, and its `Ended` stops the pad it belongs to rather than the one being looked at. What
stops a run is unchanged and per pad (its Stop, its pad's rebuild, its pad's next run), and the
window closing still stops every run everywhere, `stop_all` walking a `static` that never knew about
pads in the first place. **Buffers are per pad too**: `PadText` is a `CodeEditorData` each rather
than one replaced on every switch, so a pad comes back with the cursor, the selection and the undo
history it was left with, and a rename moves its buffer with it. The editor is mounted only for a
pad the table holds a buffer for, and that is *not* what makes its mapped `Writable` safe. Two
things are wrong with it. Every event of one press is emitted against the tree freya measured before
any of them ran, so the press that confirms a delete is followed, in that same batch, by the
editor's own global press -- still mounted, still indexing the table by the pad whose buffer has
just gone. And freya compares any two `Writable`s as equal, so a component holding one is never told
it now points elsewhere: the editor's rows keep the map they mounted with, and a switch to a pad
already read -- which has no gap to be unmounted in -- leaves them drawing the pad that was left.
Delete *that* pad and the rows draw a buffer with no lines in it, which panics inside freya rather
than here.

So there are two answers, one for each. **The index is total**: a pad with no buffer gets the
table's spare one, which is what the tail of an event batch writes into. **And the editor is keyed
by its pad**, so a pad change is a different element and the editor and every row of it are taken
down and built again against the pad on screen -- which is what keeps a render off the spare, and
what makes the reader's own switch draw the right text.

**Switching pads writes the one being left before it opens the next, and through the worker.** The
jobs are one ordered queue, so a save queued ahead of the arriving pad's read lands ahead of it. A
save left to the effect would not: the mirror into the model and the write out of it are two
effects, the second woken by the first, so a click landing between them would leave the last
keystroke unwritten. `save_if_changed` is the one comparison behind both callers, the effect for the
pad being typed into and `show_pad` for the pad being left, and the baseline it compares against
travels in `PadJobs`, since a switch has to reach it from outside the hook that owns the loop. A pad
already read is shown from what is held and is never read a second time.

**Nothing is written until the disk has been read.** `PadState::opened` is `Saves::written`'s rule
in a second place, and now per pad: the app boots holding `Scratchpad::default` and the reader's own
source arrives a thread later, so a save in between would put the default over a scratchpad someone
was keeping. The baseline is then seeded *by that answer*, so a run in which nothing is typed writes
nothing and a scratchpad nobody opened leaves no directory behind. Startup is one question above
that, `PadJob::List`, asked on mount, whose answer says which pad to open: the front of the order,
or, when there is no order at all, the pad the app booted holding, opened like any other so that
`opened_in` seeds its baseline without writing anything. `Scratchpad::write` refuses outright rather
than generating a manifest that differs from the rows, so a bad row stops the source being written
too, which the pane says over the rows, each of which says its own half. Every bad row is marked,
not the first: `Scratchpad::problems` answers with `(index, Problem)` for all of them, and
`Problem::half` says which of the row's two boxes to redden, because `Repeated` is a *name*
collision and nothing in its wording says so.

**A failed build points back at a row structurally, never by looking for a crate name in a
sentence.** A rejected build with no compiler diagnostics at all is cargo refusing before it
compiled anything, and `[dependencies]` is the only part of the generated package this pane can get
wrong. So cargo's own stderr, where `no matching package named ... found` is said and nowhere else,
is drawn under the rows. Once the compiler has spoken, the same stderr says only what the
diagnostics list already does and is dropped.

**A diagnostic's span is a target, and the target is the cursor.** rustc says where an error is
(`src/main.rs:9:17`, under the message) and the editor has a cursor that can be put there, so the
place is pressed rather than counted to. The conversion between the two is `Span::offset_in`, a pure
function of the source text and therefore unit-tested rather than eyeballed. rustc counts a line and
a column **from one** and counts a column in **characters**; an editor counts a cursor in **UTF-16
code units** from the start of the text; and lines are separated by `\n` and nothing else, which is
rustc's own rule since it normalises `\r\n` before it numbers anything. It is applied to the buffer
**as it is now** and not to what was compiled, since the reader has usually typed since, so it
clamps twice and for one reason: a column past the end of its line lands at the end of that line, a
line past the end of the text at the end of the text, and nothing here can be out of range or fail.
The press clears the selection first, because `TextSelection::move_to` moves only the far end of a
range, so a jump made with something selected would stretch the selection to the span instead of
going there.

**Only a span in the pad's own source is a target.** cargo names a file in a dependency as readily
as it names `src/main.rs`, and there is nowhere to put a cursor in one: the editor holds the pad's
source and this app opens no other file for editing. So a registry path keeps the plain label it
always had, cut down to the file's own name, with no wash under it, no pointer over it and no press,
where the pad's own file gets the relocation link's hover exactly, which is what says "this can be
pressed" everywhere else in this app. A target that did nothing when pressed was the other answer
and is worse: a hover is a promise, and one kept for `src/main.rs` and broken for everything else is
worse than never making it.

What the jump **cannot** do is scroll the editor to the line, for the reason the paragraph on the
editor gives: its scroll is private and there is no controller to hand in. So the jump *marks* the
line (the cursor's row takes the editor's own current-line background and its number lights in the
gutter) and a line already off screen stays off screen. The way to buy the scroll back would be to
give the editor its content's full height inside a `ScrollView` of ours, so that the one doing the
scrolling is the one we hold. That is a real technique and it was rejected, since it de-virtualises
the editor, and a pad someone pastes a long file into would then build every line of it on every
render. A scratchpad is a small file the reader has just been typing in, so the line a diagnostic is
about is usually on screen already; the honest thing is to say so here rather than to pay for the
exception in every keystroke.

**Wrap or scroll is decided by the list a line is in, and not by the line.** Both surfaces here draw
a tool's own output and both had it clipped at the pane's right edge, which is worst exactly where
it matters most: a diagnostic carrying a span is the widest line rustc writes, and
`--> src/main.rs:9:17`, the half that says *where*, is the half that went over the edge. The
diagnostics are a **plain `ScrollView` of wrapping paragraphs**: a build says dozens of things, so
there is nothing to virtualise away, and once nothing is virtual a block's height may be whatever
its text turns out to need. The run output **stays a `VirtualScrollView`** and takes a horizontal
scroll instead, because it is bounded at `MAX_OUTPUT_LINES` and nothing smaller, and a virtual list
steps by one `item_size`, so it has to know a row's height before it has built one, which is
precisely what a wrapped row cannot tell it. So the two are not a matter of taste: a wrapping row
and a virtual list are incompatible, and which surface can afford which follows from how much each
of them has to draw.

What wrapping costs the diagnostics is that a caret can land under the wrong character, which is why
the rendered block used to cut instead. The answer is which line pays: a line that fits is
untouched, so every diagnostic narrower than the pane is drawn exactly as it was, and the only line
that wraps is the one clipping would have thrown the end of away. A caret out of place is a worse
drawing of something still readable, where a cut is the answer not being there at all. What the
sideways scroll costs the output is that the width it can be moved over is the widest row the list
has *built*, so a wide line further down is not reachable until it has been scrolled to vertically.
A virtual list has no better answer, having never measured the rows it did not draw.

**Running does not sit on that worker, and stopping does not go near it.** `PadJob::Run` only starts
the program and comes straight back. It goes to the worker because it forks and because the
directory it hands the program is that thread's, not because it blocks. A run has no bound on how
long it takes (an accidental `loop {}` is the ordinary case in a buffer someone is experimenting
in), so a run queued like a build would freeze every save behind it and the reader could not edit
their way out. A stop is the same argument turned around: queued behind a build it would arrive
after the thing it was meant to interrupt, so it is a direct `Running::stop` from the handler.
`RunState` has four states because `Starting` is the one a `bool` loses: a fork is fast but not
instant, and a Stop pressed inside that window is remembered by leaving `Starting`, which is what
makes the arriving handle unwanted and stopped where it lands. `Over(Stopped)` is written by the
run's own `Ended` and never by the button, so the pane says "Stopped" when the process is gone
rather than when it was asked. **Events carry a run number**, which `use_analysis` was at pains not
to need. It can compare identities because an answer carries the `Symbol` it is about and that
symbol predates the request, whereas the process an event is about does not exist until after the
first bytes can be written. Stopping one program and starting another is one keypress, and untagged
the first one's last lines and its `Ended` would land in the second's output. **stdout and stderr
are told apart by colour and by nothing else**, and deliberately not by the red every invalid thing
wears: stderr is not an error, it is the other stream, so it takes the palette's one warm hue.
Between the two streams there is no order to preserve and none is claimed: two pipes read by two
threads, which is all a terminal has either.

**The list follows the newest line, and the reader takes it back by scrolling away.** Arriving lines
keep the pane pinned to the bottom while the reader is at the bottom; a wheel away from there
releases it and leaves them exactly where they are however much arrives after; coming back to the
bottom arms it again. Being at the bottom is judged **in rows against the viewport as it is now**,
`reveal_row`'s shape, and never as a row index written down earlier: past `MAX_OUTPUT_LINES` the
oldest rows drop off the front and every index shifts by one for each line that lands. The whole of
it is one effect, subscribed to the pane's own scroll, and **what it does depends on what woke it**.
A line arriving is deliberately not an occasion to re-judge: the row that has just been added is
below the viewport by definition, so a run that asked would find the pane scrolled away on the first
line of every run and follow nothing, ever. So arriving lines only *spend* the answer, and a scroll,
a resize, and the scroll the effect itself makes, are what write it. The two are told apart by the
output's identity, which is `OutputRows`' `PartialEq` again rather than its length: at the cap the
count stops changing while the rows go on being replaced. The one judgement it makes is that the
newest row is drawn *at all* rather than drawn entire, because a scroll offset is a whole number of
pixels where a list of rows is not, and a view clamped hard against its end stands a fraction of a
pixel short.

The pane is a component of its own, **keyed on the pad**, so that the scroll and the follow belong
to that pad's output instead of being one position dragged between pads by a switch. What the key
costs is that a pad comes back following again, having been remounted. The follow is what a pane
arrives armed with rather than something carried across a switch, and it is the pad being looked at
whose scrolling is worth keeping.

**What stops a run**: its Stop button, its pad's rebuild, its pad's next run, its pad being deleted,
and the window closing. The first four are per pad, since another pad's program is about another
executable, and the last is still app-wide. A **rebuild** stops it for three separate sufficient
reasons: cargo is about to write over the file the process *is*, `reopen_binary` is about to close
the objects describing those bytes, and one pad has one output pane. The **next run** stops it
because two generations of output arriving into one list is a pane with no answer to "what is this".
A **delete** stops it because the directory it was started in is about to go, and a program left
behind by that is one nothing could ever find again. An **edit**
stops nothing, deliberately: a run is of an executable and not of the buffer, and a keystroke that
killed the reader's program would make it impossible to take a note about what it printed. **Leaving
the pad** stops nothing either: the program goes on and its lines go on landing in that pad's own
list, which is what switching back shows. A **project switch** stops nothing either: `Pad` is not
one of the states in `ProjectStates` (above).

**A rebuild replaces rather than accumulates.** `reopen_binary` is `close_binary` followed by what
the toolbar's Open does, in one handler. A binary is a **path** throughout this app (that is what
`close_binary` closes by and what `project::binaries` derives the saved list from) and a rebuild
writes the same path with different bytes, so two generations of one file cannot both be in the
objects list. The cost is real and is the reader's: the tabs for that file's functions, their
viewing positions and the history entries into them go with it. Keeping them would be `Rebuilt`'s
resolve-by-name machinery pointed at a live state instead of at a session file.

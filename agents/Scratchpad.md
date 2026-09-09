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
reader's document. Each name is spelled once -- `cargo::MANIFEST` for the manifest, and
`scratchpad::SOURCE_FILE` for the source, which is also what says whether a diagnostic's span is
the pad's own and which language the editor colours. The manifest carries an empty `[workspace]`,
so a scratchpad is its own workspace root wherever the state directory turns out to be. A
scratchpad belongs to the **app** and not to a project: it lives here beside `projects/` rather
than inside one, `Pad` is not one of the states a project switch closes, and a pad open in one
project is the same pad in the next.

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
`Display`, deliberately. That is also what gives the enumeration its rule: the manifest read
answers `None` for a crate name that is not an id, so **a directory whose manifest parses, with a
source file beside it, is a pad and anything else is not**, repaired at the point of use and never
on load. `stated_in` is that sentence, and it is all the questions short of opening a pad ask: the
listing wants a name and a delete wants a yes or no, and neither is worth reading the reader's own
document. `load_from` is `stated_in` and then that document, so what one answers for the other
does -- bar the one case they part over on purpose. A `src/main.rs` that is there and is not text
lists as a pad and refuses to open, which is what this module says about any package it cannot
read; missing from the list, it would be a pad the reader cannot fix.

**The name lives in the package, under `[package.metadata]`**, the one place cargo reserves for a
tool of its own and ignores itself. So "the package is the storage" still holds: nothing describes a
pad beside its own directory, and `load_from` is still the exact inverse of `write_to`. It is *not*
in the order file beside the ids, which is `recent_projects`' rule for a project's name: a copy
there would be a second one to keep in step with the one the reader edits. A new pad is made with
**no name at all**, an empty one being a real answer and not a missing one, and what stands in for
it on screen is the UI's to decide. Nothing is written into the package until the reader has said
something.

The crate name being the id rather than the name has a second payoff: **a rename does not move the
artifact**. cargo names the same executable either way, so a pad renamed between builds rebuilds
over what it built last rather than leaving that file on the disk under the old name.

**Which pad opens is an order, `recents.toml`'s shape again**, in `scratchpads/recents.toml`. It
sits beside the pads rather than at the top of the state directory, so it is not a second file to
tell apart from the projects' one, and it is a file where every sibling is a directory, so the
listing steps over it with no special case. `PadOrder` **is** the projects' order -- both are
`order::Order<T>`, one type in `src/order.rs`: the front is what to open, `touch` answers whether
anything *moved* (which is what keeps a startup that reopens the front pad from writing a file),
and nothing prunes itself on load. **`MAX_ORDER` bounds the file and not the list**, `remember`
truncating what goes out rather than what a `touch` keeps. That is the pads' rule and the projects
inherited it: the list here is what the panel draws, and the panel is the only way to open a pad,
so an order that dropped its own tail would drop exactly the pads the listing below goes to the
trouble of appending. It is also loaded through `Store::read` like everything else, which is what
it was not: it parsed its own file and answered a default, so the next `remember` wrote over a pad
order the reader never heard was unreadable. **`pads()` is the order's ids then the pads it does
not name**, in id order. Each row carries the name out of that pad's own manifest, read at the
moment the list is asked for, which is what lets the panel draw a pad nothing has ever opened.
That second half is the
difference from `recent_projects`, which lists only the projects a reader has opened: this is the
list a reader picks a pad from, so a pad that fell off the end of `MAX_ORDER` or was made
outside the app has to be reachable. The manifest and no more: a name is all a row shows, so
listing N pads reads N small files and not the N sources with them. A pad is remembered when it is **opened**, and only if there is
a directory for it, which keeps the "nothing is written until there is something to say" rule: the
pad a first run holds is in memory until something is typed into it.

**A new pad is `new_pad`, and the claim is a `create_dir` that fails rather than opens**: the first
free `pad-N`, through `Store::claim` -- the one bounded loop an unsaved project and a rescued file
are claimed by too -- stepping over an id another copy of the app already took. Unlike an
anonymous project it **writes the package at once**: pressing New is a deliberate act, and a
claimed directory with no package in it is not a pad and the listing would repair it away. The
stem is `DEFAULT_ID`'s and deliberately without its number. The pad a first run opens is `pad` and
New makes `pad-1`, `pad-2`, so New can never hand out the id of the pad the app is already
holding. It could if that one were `pad-1`: a pad nobody has typed in has claimed no
directory for the `create_dir` to fail on.

**There is no rename operation**, and that is the point of the id: renaming writes
`Scratchpad::name` and the ordinary per-change save puts it on disk, exactly as typing in the source
does.

**A delete is the one thing here that destroys what the reader wrote**, so it is behind a question
(below) and the module's half of that is being narrow. `delete_pad` builds the path out of the id
alone, a checked crate name that can be neither `..`, nor a separator, nor absolute, and then
refuses a directory `stated_in` no longer answers for -- so a `remove_dir_all` can only ever reach a
directory holding a manifest this module wrote. `symlink_metadata` is what makes that the directory
itself rather than whatever a link put in its place, and a pad with no directory is already deleted
and says so. What goes is the whole directory, cargo's `target/` included: the package is the
storage, so there is no part of a pad anywhere else. Nothing goes back to the order file --
`Order::forget` is about the list on screen, and an id whose directory has gone is one `pads`
already steps over.

A dependency is a `(name, version)` row and the **version is required**. A `*` is refused with its
own reason, since a requirement whose answer changes with the day is the one thing a scratchpad must
not have. Rows are checked against two grammars (a possible crate name, a possible version
requirement) and never against crates.io: whether a crate exists is cargo's answer. Every bad row
comes back as `(RowId, Problem)` -- the row's own id, not where it is drawn -- so the editor can mark
all of them at once, a repeat of one crate included, since `[dependencies]` is a table and the second
row would otherwise silently win. A scratchpad with a bad row **refuses to write** rather than
generating a manifest that differs from what is on screen. **Building is blocking and belongs on a worker thread**, exactly as `open_files`
is. Running cargo and reading what it said is `src/cargo.rs`, shared with the project's own build
(`agents/Sidebar.md`); `build_in` writes the package, calls it, and narrows what comes back to the
one binary a generated package has. The artifact path is what cargo *named*, never
`target/debug/<crate>` derived from the name and the profile, which a `CARGO_TARGET_DIR`, a config
above the directory or an executable suffix each make silently wrong. Turning that stream into an
answer is a pure function of cargo's stdout, stderr and exit status, which is what lets a failed
build be a test over a canned stream. Three answers, not two: the compiler said no (with cargo's own
stderr kept, since `no matching package named ... found` is said there and nowhere else), or cargo
never ran.

**`Build` holds what cargo said rather than copying it.** `Build::Ran` is a `cargo::Run` and the one
binary a generated package has; the arm of its own is for the builds that never happened -- a bad
dependency row, a package that would not write, a cargo that would not start -- which is about a
generated package and means nothing to a workspace. What a pane says about a build is
`cargo::Run::verdict`, so the pad and the Project view cannot report the same build differently;
`PadState` and `Builds` each add only "Building..." and pass the rest on. It was two copies of the
summary, down to the wording, and the two had already drifted over a cargo that would not start.

**Running is the artifact and not `cargo run`.** `run_in` spawns the executable `build_in` already
asked cargo to name, in the scratchpad's own directory with a null stdin. Re-entering cargo would
redo resolution to arrive back at that same path, or could arrive at a *different* one (the reader
has usually typed since, so what ran would not be what the diagnostics describe). It would
interleave cargo's progress into the stream the reader is reading as their program's output, and it
would make stopping meaningless, since killing a `cargo run` kills cargo and leaves its child with
nothing holding it. What the app is handed back is a `process::Handle`, whose one job is to stop the
program and everything it forked; how a program is started, stopped and reaped, and why a stop is a
group's kill, is `agents/Process.md`.

**Output is streamed, not collected**, which is the whole difference from `build_in`'s
run-it-and-return-the-output shape: a program that prints and then loops for ever has said
something, and a value returned at exit would never say it. Two threads, one per pipe, hand each
line to a callback as it arrives; whichever finishes last reaps the process and emits the one
`Ended`. So a run is over when both pipes are at the end **and** the process is reaped. A program
that hands its output to a grandchild outliving it shows as still running, which is the honest
answer, since the output is still coming. The reap `try_wait`s on a poll rather than `wait`ing,
because holding the `Child` is exactly what would make a stop wait for the process it is killing.
**A reader that will not start is a reader that has finished**, and the two bounds on what a
program writes are `agents/Process.md`'s along with the rest of the reading. The third bound is the
app's own: a `RUN_EVENTS`-bounded channel, which is backpressure that reaches the program itself,
since a full channel blocks the pipe thread, which fills the pipe, which blocks the writer.


## The Scratchpad view

**The Scratchpad page** (`Tab::Scratchpad`) is the pads there are down one side and the shown one
beside it: its source, its crates, its build and what the compiler said. It is a *view* for the
reason the settings page is: there is one of it, it resolves against no object, and neither code
pane could draw one. That there are many *pads* does not make it many views: the pad list is the
Scratchpad view's own side panel, because the content area's strip is deliberately not the place for
a second document list (a chip there is a *place in a binary*). What it **builds** is the pad's own
and not the project's: the program is held in the pad's state, drawn by the pad's pane, and is in
neither the Objects panel nor the paths a project saves.

**`ScratchpadTab` is a skeleton, and every piece of the pane reads the slice of `Pads` it draws.**
The pad list, the heading with Build and Run, the name and package rows, the dependency rows, the
diagnostics and the delete question are each a component; the tab itself reads only what the editor
and the listing beside it are drawn of -- which pad is shown, the program it last built, and where
its run got to. It was one function drawing all of them, and it copied the shown `PadState` out
whole first: the source, every dependency and every diagnostic cloned on every keystroke. **The
split buys no renders.** freya subscribes a scope to the whole of a state it read, so a keystroke in
the name box still wakes every piece that read `Pads`. What it takes away is that clone, and a
function nobody could read a piece of without scrolling past the rest.

**What a build made is written into the package, so a pad opens on its program.** Nothing the
app holds about a build survives a restart, and the artifact's path may never be derived --
`target/debug/<id>` is silently wrong beneath a `CARGO_TARGET_DIR`, a config above the directory,
or an executable suffix -- so what cargo *named* is kept, under `[package.metadata]` beside the
pad's own name. That keeps "the package is the storage": nothing describes a pad outside its
directory and `load_from` is still `write_to`'s inverse. Beside the path goes the **digest** of
what the build was of, not the source itself: the source is already in the package a line away,
and the only question is whether the two are still the same. Sixteen lowercase hex digits,
`analysis::FileDigest`'s written form, compared as text -- text this app did not write is simply
not equal, which reads as "changed", the rule the session's own digests follow. So a pad edited
between the build and the restart still says it is out of date, and `Program::built_from` is that
digest rather than the value, since a program read back in a later run has to answer the same
question the same way. A path whose file has gone reads back as nothing, which is the same answer
as never having built.

**The program is read on the scratchpad worker, in the same answer as the build -- and in the
same answer as the open.** That thread
already owns the pad's directory -- `target/` is inside it -- so the single writer of what cargo
wrote is also its single reader, and `built` and the program it describes can never disagree: there
is no pass in which the pad has an executable it has not read. A job of its own would be a second
thing to supersede, to arrive out of order and to answer for a deleted pad; the parse is
milliseconds against a build's seconds and sits behind the same `building` flag, so it delays
nothing the build was not delaying already. `PadJob::Open` reads it back the same way, off the
package it has just loaded, so a pad that was built in an earlier run is shown its program by the
answer that opens it. It streams nothing, deliberately: `open_binaries`'
streaming shape is for a 331 MB file or a 196-member archive, and this is one small file nothing
draws until it is whole.

**An answer that arrives for a pad nothing holds is dropped, and so is one for a pad that asked no
build.** The first is `request_delete_pad`'s order -- the state is out of the table before the
delete is queued -- and it is now the whole story: there is no `Objects` entry to undo, no `Loads`,
no tab, no saved path, and the parsed program is dropped with the answer value. The second is the
`building` flag, read as well as written: `Pads::forget` comes back to the default pad's id when
the last pad goes, so an answer can arrive for a *different pad under the same id*, and a pad that
asked for no build is not building.

**The pane draws it as an object's whole code, beside the editor.** The two are a source-driven
tab's two panes with a source side the reader types in, in a `ResizableContainer` of the pad's own
(`PadSplit`, `PadSplits`) rather than the document split's -- two containers sharing one context
would carry the handle across a switch between a document and this page, and the pad's drag would
be written into the project's session, where a pad has no business being. The editor leads, as a
driven side does and because the keyboard goes to the first box a tab registers. The listing is put
away by the same `PaneToggle` a document's bar carries, in the heading row beside Build and Run:
that row is the pad's own strip of controls and the one thing here that is always up, the editor
having no bar. There is no `SymbolBar` over it, whose section and toggle are both filed under a
`DocId` the pad has not got.

**It opens on the pad's own code**, the lowest *placed* address the pad's `src/main.rs` produced
(`compiled::lowest_placed`) -- lowest and not first, since the crate answers in raw address order
and two code sections would put the wrong one first. Without it the pane would open at the top of a
linked Rust program, which is the runtime's code and not the reader's, and the pair the cursor
lights only reaches stretches that have decoded. It is written as a `Planting` and not as a place
in `Places::code_at`: the listing keeps no place of its own, and an entry there would hold that program's
bytes with nothing that would ever forget them, where a planting is taken once by the pane and put
back to `None`. `PadAssembly` is keyed by the program, so a rebuild takes it and its listing down
and builds them against the new one.

**The listing follows the editor's cursor, and nothing comes back.** The line the cursor is on is
written as the source run (`mark_line`, the same door a click on a source row goes through), so
the listing lights the instructions compiled from it and owes it a scroll -- a source-driven tab's
two panes, with a source side the reader types in. The other direction is not written down because
it cannot be: freya's editor can neither light a set of lines nor be scrolled from outside, so an
instruction that named a line could do nothing with it (`notes/upstream/freya.md`). What the drive
compares against is **the run on screen** and never a line remembered beside it: `use_land` puts
`Marks::default()` back on every change of the active entry, and the page becoming the tab on
screen is one, so a drive that remembered would be wiped a beat after the reader arrived and would
never say it again. That comparison is also what makes typing along one line write nothing.

**An edit since the build says so over the listing**, the Source pane's checksum row in a second
place -- and exact where that one is a guess, since the app wrote the source this program was
built from and kept it beside the program. What a build was *of* is the source and the dependency
rows (`Compiled`), and deliberately **not** the name: it lives in `[package.metadata]`, which cargo
compiles nothing from, so a rename must not make a listing out of date. A value and not a counter,
so a reader who types a character and takes it back is building the same program and is told so.
`built_from` is taken from the scratchpad the **job** carried, never from what is on screen when
the answer lands, so a build the reader typed during says it is out of date the moment it arrives.

**Which file is the pad's own is asked of the program and never constructed.** rustc records
`src/main.rs` as it was handed it and the name a reader of the debug info gets back is that joined
onto the unit's `DW_AT_comp_dir` -- the directory rustc ran in, which is where the pad's directory
*resolved* to and not how this app spells it. So the pad asks the object what files it has code
from (`Object::source_files`) and takes the one ending in `src/main.rs`
(`scratchpad::own_source`). A program that names none -- no debug info, or paths remapped -- opens
nowhere, and that is an answer rather than a failure.

**A delete is asked for, and a row is where it is asked from.** A right-click on a pad's row offers
one item, and the item deletes nothing: it writes `Pads::confirming`, and the popup that field draws
is the question -- `RescuedPopup`'s `Popup` in a second place, dimming what is under it and taking
Escape or a press outside as no. Not the × a dependency row has: a × there is one press away from a
list one row shorter, and this is one press away from the reader's own source being gone. The
question names the pad and the path its package is at, since if there were anything to get back that
is where it would be, and there is not. `confirming` sits beside `refused` at the root for the
reason that one does, and a delete that fails is what `refused` then says -- which is why it holds
the whole sentence now rather than a `Failure` the panel puts a word in front of.

**Letting go comes first and the disk second.** `request_delete_pad` takes the pad out of the
table -- its save baseline with it, the baseline being a field of the state the table holds -- out
of the order and out of the buffers, and only then queues `PadJob::Delete`. That order is what
makes an answer about a deleted pad harmless: the worker is one ordered thread, so a build in
flight finishes against a directory that is still there, and what it answers arrives for a pad
nothing holds and is dropped -- which is how a build the reader deleted their way out of does not
go on to open its artifact. A queued save of that pad is superseded by the delete, having nothing
left to write to. The pad's program is stopped, the directory it was started in being about to go;
a run still forking is stopped where it lands, its handle arriving for no pad in the table.
**There is always a pad to show**: the next in the order takes over, and when the last one goes
the table comes back to the pad a first run holds, opened like any other so that nothing is
written until something is typed into it. That `Open` is queued *behind* the delete, or it would
read the directory the delete is about to remove.

**That panel is a fixed width** (`PAD_LIST_WIDTH`) and not a `ResizableContainer`, unlike the two
splits in this app a reader can drag. A tab that is not the one on screen is
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
all of them go to one `use_worker` (`agents/Worker.md`), the shape every worker here has. It is one thread
and not several because the point is not only that the UI thread stays free but that a directory has
a single writer, so a save cannot land inside the build that is reading what it writes. **Saves
supersede, per pad, and builds never do**: a keystroke is a save, so the loop drains its queue while
what it holds is one, and whatever is behind it is a newer save, a build that writes the package
itself, or a delete that takes it away. Which jobs may stand in for a save is a correctness rule and
not a refinement. Taking whatever was next, a save would be dropped in favour of a job that writes
nothing -- another pad's, or a run of this one queued behind the save the same keystroke made -- and
the package would quietly fall behind what is on screen. The baseline has already moved to the
dropped save, so nothing would ever write that edit again. So `superseded` replaces a save only with
a job that names the same pad *and* writes or removes its package, and hands anything else back to a
hold-back queue rather than stepping over it. That a build of one pad delays another's save is
accepted: the reader types in one pad at a time. Two builds cannot start at once, on the button
(`enabled`) and in `request_build` both, because a build takes seconds and a second job queued
behind the first would compile bytes that have since changed. **Nor does anything run while a build
does**, on the button and in `request_run` both: cargo is writing over the very executable a run
would start, and a run begun mid-build is not one that build's own `stop_run` took down. **The
keys are the buttons and not a second copy of them.** Ctrl+B, F5, Shift+F5 and Ctrl+N call
`request_build`, `request_run`, `stop_run` and `request_new_pad`, and ask nothing of the state
themselves: every refusal above is a property of the request, so a key that went round it to read
`building` for itself would be the second reading to drift. They are answered on `ScratchpadTab`'s
own rect, which `page_body` mounts for the tab on screen alone, so they work while the reader is
looking at the pad and nowhere else -- a build begun from a document tab shows no sign of itself,
neither the button saying "Building..." nor the diagnostics it ends in being on screen. A build
that comes back also **forgets what the panes have read of the pad's package**, which is written to the same
`src/main.rs` every time and would otherwise be drawn as it was first read for the life of the
process (`forget_source_under`, `agents/Panes.md`).

**Everything the app holds about a scratchpad is per pad.** `Pads` is the table of them and which
one is shown; `PadState` is one pad's own, and every field it has (what was read, what is being
built, which run is going and what it has written) was already about one pad. A pad is in the table
from the moment it is first shown and never leaves, so `Pads::state()` is never absent and no call
site grows an `Option`. **Runs are per pad**: an event carries the pad beside the run number, so a
program started in one pad goes on running and goes on writing into *its own* list while another pad
is on screen, and its `Ended` stops the pad it belongs to rather than the one being looked at. What
stops a run is unchanged and per pad (its Stop, its pad's rebuild, its pad's next run), and the
window closing still stops every run everywhere, `process::stop_all` walking a list that never knew
about pads in the first place. **Buffers are per pad too**: `PadText` is a `CodeEditorData` each rather
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
keystroke unwritten. `Pads::unsaved_change` is the one comparison behind both callers, the effect
for the pad being typed into and `show_pad` for the pad being left. A pad already read is shown
from what is held and is never read a second time. **That is the answer's rule as well as the
question's** (`Pads::opened`): a pad shown, left and shown again before its first answer arrives
is asked for twice, `show_pad` going by `PadState::opened` and the answer being what seeds the
baseline behind it, so an answer for a pad that is already open is dropped -- and the buffer is
made only where it says it took one. Taking it would put back what the disk held before the read
-- older than anything typed since -- and make that the baseline, leaving the disk ahead of the
screen with no save owing until the next keystroke wrote the older text back over it.

**The baseline is a field of the pad, and the comparison is asked under `peek`.** `PadState::disk`
is what the worker last read or was last handed, and `PadState::opened` is that field being there:
one fact in one place, where a `bool` beside a map held elsewhere was an invariant nothing
checked. The cost is that the save effect now writes the state it reads to subscribe, so
`save_if_changed` asks `PadState::unsaved` under `peek` first and takes a guard only where there
is something to send. A write notifies whether or not it changed anything, and an effect is a loop
that runs and then waits to be notified, so the effect's own write makes that wait return at once
and the task never yields. A guard taken whatever the answer said costs not a render but the
window: every test that mounts the scratchpad hangs rather than fails, which is why there is no
headless test of this and a unit test of `Pads::unsaved_change` instead.

**Nothing is written until the disk has been read.** `PadState::opened` is `Saves::written`'s rule
in a second place, and now per pad: the app boots holding `Scratchpad::default` and the reader's
own source arrives a thread later, so a save in between would put the default over a scratchpad
someone was keeping. There is nothing to compare against until that answer seeds the baseline,
which is the whole of what an absent one means, so a run in which nothing is typed writes nothing
and a scratchpad nobody opened leaves no directory behind. Startup is one question above that,
`PadJob::List`, asked on mount, whose answer says which pad to open: the front of the order, or,
when there is no order at all, the pad the app booted holding, opened like any other so that
`opened_in` seeds its baseline without writing anything. `Scratchpad::write` refuses outright
rather than generating a manifest that differs from the rows, so a bad row stops the source being
written too, which the pane says over the rows, each of which says its own half. Every bad row is
marked, not the first: `Scratchpad::problems` answers with `(RowId, Problem)` for all of them, and
`Problem::half` says which of the row's two boxes to redden, because `Repeated` is a *name*
collision and nothing in its wording says so.

**A dependency row is named by an id, and the two boxes it is drawn as write back through that id.**
Mapping one by position runs into both of the things the buffers ran into above. Every event of one
press is emitted against the tree freya measured before any of them ran, so the press on a row's × is
followed, in that same batch, by handlers on rows the next render takes down, reading through a map
into a list that is already shorter. And freya compares any two `Writable`s as equal, so a row
holding one is never told it now points elsewhere: keyed by position, the boxes under a deleted row
keep the positions they mounted with, and the caret is left in the row that has moved up into its
place -- the reordering under an edit a list of text boxes must not do. So the rows are keyed by
their ids, and `Scratchpad::dependency_mut` is total: an id the list no longer has gets the pad's
spare row, `PadBuffers::gone`'s device a level up. Ids come from a counter on the pad and are never
handed out twice, and they say nothing about what a row *asks for* -- two rows are equal when they
name the same crate at the same version -- so neither an id nor the counter nor the spare can make a
program out of date or the disk copy look stale.

**A package that will not load is refused rather than replaced.** `load_from` answers `None` both
for a directory with nothing in it and for one holding a package this module cannot read back -- a
dependency written by hand as a table, the ordinary way to ask for a feature, is enough. `opened_in`
tells the two apart on whether either file is there, and answers `Failure::Unreadable` for the
second. The pad is then left **unopened**: no buffer is made for it, its baseline is never seeded,
and `save_if_changed` steps over a pad that is not open, so the reader's own `Cargo.toml` and
`src/main.rs` stay as they are and the pane says why. Answering what was handed in, as an empty
directory does, would seed the pad from the default the app boots holding, and the first keystroke
would write that over both files. **Building is the same write**, `build_in` putting the package on
disk before it compiles it, so `request_build` refuses a pad that is not open, on the button
(`enabled`) and in the request both -- or the loss the keystroke cannot bring about would be one
deliberate press away. Such a pad does not move to the front of the order either: a restart may not
come back to one that will not open.

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

**The target itself is drawn by `PlaceTarget`** (`src/ui/place_target.rs`), which the Project view's
own diagnostics use too -- the same hover, the same colours, the same stopped press. The two go to
different places, so **the press is handed in** as an `EventHandler`: the contexts it needs are
consumed by the pane while *that* renders, `use_consume` being a hook. An `EventHandler` never
compares equal, so a target re-renders whenever its pane does; a label and a hover flag is the whole
cost.

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

**A line arriving is written into the table and not over it.** The task takes everything already
queued in one go, so a batch is one render however many lines it holds, and it writes through the
state's own guard. `Pads` holds every pad's source, dependencies, diagnostics and output, so
replacing the table to push one line would copy all of it -- and copy the deque of lines a second
time inside `Arc::make_mut`, the `Arc` over them having just been cloned with the table. What is
left is the one copy the pane's own hold on those lines forces. The guard is taken only when the
batch holds something for a pad that is still there and still on the run it names, a write notifying
whether or not it changed anything.

**The list follows the newest line, and the reader takes it back by scrolling away.** Arriving lines
keep the pane pinned to the bottom while the reader is at the bottom; a wheel away from there
releases it and leaves them exactly where they are however much arrives after; coming back to the
bottom arms it again. Being at the bottom is judged **in rows against the viewport as it is now**,
`reveal_row`'s shape, and never as a row index written down earlier: past `MAX_OUTPUT_LINES` the
oldest rows drop off the front and every index shifts by one for each line that lands. The whole of
it is one effect, subscribed to the pane's own scroll, and **what it does depends on what woke it**
(`tail_move`, the arithmetic apart from the effect).
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
executable, and the last is still app-wide. A **rebuild** stops it for two separate sufficient
reasons: cargo is about to write over the file the process *is*, and one pad has one output pane.
The **next run** stops it
because two generations of output arriving into one list is a pane with no answer to "what is this".
A **delete** stops it because the directory it was started in is about to go, and a program left
behind by that is one nothing could ever find again. An **edit**
stops nothing, deliberately: a run is of an executable and not of the buffer, and a keystroke that
killed the reader's program would make it impossible to take a note about what it printed. **Leaving
the pad** stops nothing either: the program goes on and its lines go on landing in that pad's own
list, which is what switching back shows. A **project switch** stops nothing either: `Pad` is not
one of the states in `ProjectStates` (above).

**A pad's program is the pad's own, and a rebuild costs the reader nothing.** It used to go into
`Objects` like any other binary, and a rebuild had to close it first -- a binary is a **path**
throughout this app, and a rebuild writes the same path with different bytes, so two generations of
one file could not both be in the list. That close took the tabs for that file's functions, their
viewing positions and the history entries into them with it, every time. None of it is left: a pad
is not part of a project, so its program is in no list, nothing can open a tab into it, and a
rebuild is a value in the pad's own state being replaced.

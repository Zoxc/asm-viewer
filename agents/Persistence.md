# Persistence

What the app writes to disk and when: the projects, each one's two files, the session restore, the
recents order and the user's own settings. Scratchpads have their own storage and are in
`agents/Scratchpad.md`.

The code is `src/project/`, split the way this note reads: `files.rs` is the two schemas and the
id, `restore.rs` is live state into a session and back, `recents.rs` is the order, `saves.rs` is
when a write happens, `trust.rs` is the agreement to run a language server. `project.rs` over
them is the lifecycle -- entering a project, leaving it, putting it somewhere else, taking it
away -- since each of those touches more than one.

There is **no published version of this app yet**, so persisted formats need no backward
compatibility: a schema change is just a schema change, a stale file is ignored rather than
migrated, and `#[serde(default)]` is added only when it earns its place on its own merits.

**Ignored, but not lost** (`Store::read`, `src/store.rs`). Every one of these files is read back
into a default when it will not parse, and the next write puts a good file over it -- so the one
thing that rule costs is the reader's own file, taken away without a word. `Store::read` is the
one read the store has, and **every load on the way to a write goes through it**: it reads the
**bytes** (a file that is not UTF-8 will not parse either, and is lost the same way), and on a
failure copies them to `incompatible/` under the path the file had, removes the original, and
answers `None` -- which is what every one of these loads already meant by "not there", so no caller
changed shape. It being the store's only read is the point: the pad order used to parse its file
itself and so let the next write destroy it, which is exactly the drift a second reader of the
rule invites. A file the system will not hand over at all is left alone, since nothing can be
salvaged from it, and **`Store::write` refuses it** until a read of it succeeds. It used to be
answered as absent and nothing more, on the grounds that nothing was about to write over it --
but `remember` writes `recents.toml` straight after reading it, and a rename needs no permission
on the file it replaces, so one the reader could not read was replaced by a one-entry list. A file
that will not parse and cannot be moved aside (`incompatible/` cannot be made, the disk is full) is
refused the same way, since it is still the only copy; a copy cut short is removed. **Only the app's own files are read through
it, wherever they sit**: the session beside a project the reader gave a place is outside the store
and is the app's all the same, and the next flush would replace it as surely as any other. It used
to be read and left where it was, a path outside the store taken for somebody else's, so a session
a newer build wrote was overwritten with the default and never named. The reader's own file, the
project file, is read another way (below). The mirror (`incompatible/projects/1.avproj.session`, or
`incompatible/outside/home/me/app/app.avproj.session` for one outside the store, so it cannot be
taken for one of the store's own) is so that a moved
file keeps the shape of the path it had rather than being flattened into one heap, and the
destination is claimed through `Store::claim` -- `settings.toml`, then `2-settings.toml` -- which
is `unsaved_project`'s "a create that fails rather than opens", and for its reason: nothing there
is ever overwritten, not by an earlier rescue and not by a second copy of the app moving the same
file at this moment. The original is **removed** rather than copied, since nothing writes over
`settings.toml` until a setting changes and a file left in place would be rescued again on every
launch.

**A project file is never moved aside, and that is now the rule and not an exception.** It may be
the reader's own file, sitting in their tree beside the code, and the app has no business taking one
away; so `Project::load_from`, the plain read `recent_projects` has always drawn its rows with,
is what `load_project` opens one with too. A project file that will not parse therefore does not
open at all -- and since nothing opens, nothing writes over what could not be read, which is the
whole of what the rescue was protecting. The session beside it *is* the app's own and still goes
through `Store::read`. The project file is read by `source::read_text_in`'s rule: a symlink is
followed, but a fifo or a device is refused and the read stops past `source::MAX_SIZE`. The recent
list reads every project file on the UI thread, and a stranger's tree could otherwise name a fifo
there and stop the app for good.

**So what is left is saying why.** `load_project` answers a `Failure` -- the path that was asked
for, and a `Reason` -- rather than a bare `None`: telling the reader is the whole of what happens
to a project that will not open, and "it is not there, or it is not a project file the app can
read" was the window declining to say what the load already knew. Four answers and not one:
nothing at that path, a file the system would not hand over, bytes that are not text, and TOML
that will not parse. The last two are why `load_from` reads **bytes** here as well -- `read_to_string`
folds them into one `io::Error` -- and it is `Store::read`'s reason a level up. A parse failure is
taken apart rather than printed: `message()` and `span()`, the span counted over the text into a
line and a column, since the error's own `Display` is a three-line diagram with a caret under the
column, which lines up only in a fixed-width font and only while nothing wraps, and a window this
wide can promise neither. `Reason`'s `Display` is then one whole sentence per variant -- what the
reader is shown -- and `UnopenedPopup` draws it with the path under it. `Reason::NoStore` is the
one a caller supplies rather than the load: nowhere to keep anything is not a fact about the file,
but it is still why the project did not open. The two ways in that report it -- a startup given a
file, and a switch -- both ask `store_for` (`src/ui/session.rs`) for the store, so the failure is
built in one place.

**And the reader is told**, which is the half that makes it a rescue at all: a file moved somewhere
nobody hears about is a file lost politely. Each destination is sent on a channel the store owns
and its clones share (`Store::moved`), and a task `roots` spawns over the run's store
(`name_moved`, `src/ui/session.rs`) adds it to `Rescued` as it arrives. `RescuedPopup`
(`src/ui/rescued_view.rs`) names them over freya's `Popup`, which is shown exactly when it has
children, so the list being empty *is* the window not being there. The store says it and the UI
does not ask: the loads run on the UI thread and on the pad worker, at startup, on a switch, on a
new project and whenever a pad is opened, and the earlier version, which asked at startup and on a
switch, left the pad worker's moves unnamed until the next switch. A channel per store and not one
static, so each headless test hears its own store's moves and no other test's. The task **adds** to
the list rather than setting it: a window still naming what one load moved does not lose it when
the next runs, and a load that moved nothing leaves a closed window closed. It is drawn through `notice` (`src/ui/parts.rs`), the shell all four of
the app's asking windows share, which is `Popup` itself and neither `PopupTitle` nor
`PopupContent`: both of those set a font size of their own, which would draw the window in a size
the reader never chose.

**That directory is a `Store`** (`src/store.rs`), and it is the whole of the storage layer:
where a file goes, how one is written, how one is read back when it may be bad, and how a free
name under it is claimed. `Store::open` answers where -- `ASSEMBLY_VIEWER_STATE` where that names
a directory, and `dirs::state_dir()` (falling back to `data_local_dir()`) + `assembly-viewer/`
where it does not. The variable is there because **more than one copy of this app otherwise
shares one directory** -- two checkouts, or a build somebody is trying something in beside the
window the reader actually uses -- and they do not merely take turns: one writing a file the
other's build cannot parse is one moving the reader's file aside as unreadable, since that is what
every load on the way to a write does. Unset and empty are one answer, so a script that meant to
set it and did not cannot put a reader's projects in the working directory.

**One store is opened per run**, in `app()` where the settings are loaded, and handed down: no
module looks the place up for itself, and a path given to the store is relative to it unless it is
absolute, which is what lets a project file the reader gave a place go through the same writer as
the app's own. That rule is why **every path a project is named by is absolute in memory**: a
relative one would be read where the app was started and written under the store. The two ways
one can come in relative are made absolute where they enter, lexically (`std::path::absolute`):
the project named on the command line, in `main`, and a relative `ASSEMBLY_VIEWER_STATE`, whose
paths would otherwise come back through `Store::path` with the base joined on twice. The
project's path also has its `..` taken out (`project::absolute`): the recent list compares
by spelling, so `../app/app.avproj` would sit there beside the same file's plain path.
The project's directory is a third, typed into a box that takes any spelling. It is made
absolute where the text becomes a path (`OpenProject::workspace`), with its `..` taken out by
text (`cargo::lexical`). Kept relative, the files joined to it never matched the debug info's,
which are absolute, so a finished build forgot none of the source tabs the debug info had opened;
and the project file saved it as typed, to be read back against the file's own directory.
Taking `..` out by text is cargo's rule for a `[package] workspace` path; after a symlink it
names another directory than the kernel would, the cost of one spelling for the whole project.
`Store::relative` is the same rule the other way -- where an absolute path sits
under the store, or `None` for one outside it -- so the directory stays a private field and
nobody strips a prefix by hand. `Saves` keeps the one it was pointed at when the project was
opened, so the periodic flush and the close hook -- neither of them in the component tree -- have
one without being handed one. `Store::at` is the other constructor and is the tests', declared
with them in `store/tests.rs`: a store under a directory of a test's own, which is what the
`x()`/`x_in(base)` twin of every stored operation used to be for.

Each file under it is written atomically via `.tmp` + rename (`Store::write`, over the one
`write_atomically`; the free function is there because `cargo.rs` edits a manifest that is not the
app's at all). **Each write has a temporary of its own**, `<file>.<pid>.<n>.tmp` made with
`create_new`, and one a failed write leaves is removed: two apps on one store write `recents.toml`
together, and a temporary they shared was one file both wrote into, so the rename put a splice of
the two in place. The temporary is **synced before the rename**, because a rename is
atomic against a crash of the process and not against a power loss: the directory entry can reach
the disk ahead of the data, and the file the next launch reads is then zero bytes or a truncated
tail -- one that will not parse, so the rescue moves the reader's project or session aside and
hands back a default, which is the very loss the dance exists to prevent. The directory entry is
left unsynced: losing the rename costs the last save, where losing the data costs the file. **A
symlink is written through**: the rename lands on the file the link names, with that file's
permissions, since a rename over the link made it a plain file and a project file the reader linked
in from a repository of theirs never saw another save. One
fsync per save: the session's at most one every 30 s, and a box typed in -- the Project page's
three, the font family -- writes once the typing stops and not per keystroke.

**A project is its project file's path.** `ProjectId` is not where a project is but *which* one it
is: sixty-four random bits in the file, written as sixteen hex digits, and carried by every file the
app keeps beside it. That is what a session is checked against -- the session is found by the
project file's *name*, which says nothing about whether that file still holds the project it held,
so one whose id does not match is dropped whole rather than opened over a project it was never
written for. An **absent** id counts as another id, since a session that cannot say which project it
belongs to is not this one's. Random and not a counter, because a counter is only unique to the
machine that kept it and two projects meet the moment one is checked in.

An **unsaved** project is one whose file is under the app's own `projects/`, and nothing else
distinguishes it from one the reader gave a place. Its file is `projects/<n>.avproj`, claimed by a
`File::create_new` that **fails rather than opens**, so the claim is one atomic operation rather
than a listing followed by a race; `project::label` turns the stem back into `Unsaved project 3`.
The number carries no meaning: giving a project a place later does not change which project it is,
the id having said that all along. **Nothing makes a project but the reader asking for one**
(`start_new`, reached from the menu): with none open, `record` and `flush` write nothing and claim
nothing. A project file used to be claimed by the first write that had anything to say, which read
well until the app could sit with no project open at all -- then arranging the window, or opening
Settings, was a project appearing on disk behind the reader's back. It is still claimed **empty**,
which is why `Project::binaries` needs its `serde(default)` like every other field: between the
claim and the first write the file holds no keys at all and has to read as the empty project it
is.

**How the window was arranged is the session's `[ui]`**: the sidebar's width, the document
split's, and the sidebar's own arrangement. It is what the app *noticed* the reader doing to
their own window, which is the line the two files are drawn along, and it never travels --
how somebody arranged their sidebar is not a thing to check in. Every field is an `Option`
and the section is absent until something is dragged, so a window nobody has touched writes
nothing and a build without one of them reads the rest.

The arrangement is a **mirror** of freya's `DockNode`, which derives no serde -- and a mirror
is what keeps `project/files.rs` framework-free besides. Panels are written as **strings**
(`Panel::stored`) for `SavedTab`'s reason: an unknown name is a parse error where a string is
one panel this build does not have. `DockArea::restored` therefore treats what comes back as
a reader's arrangement and not a promise: a name this build lacks is dropped, one named twice
lands once, an empty group is dropped and the split closes up around it, and **every panel
this build has that the file never named is put in the first group** -- otherwise a release
that adds a panel would hide it from everyone who had ever arranged their sidebar. The
sidebar's width is a `SidebarSplit` -- a number and the `ResizableContext` it is read back out
of, as the document's split is (`Split`, `agents/UI.md`): a
`ResizablePanel` registers at its `initial_size` and forgets on unmount, and the window's body
is rebuilt whenever a project arrives or goes. A switch goes from one project to the next with
no render between, so that rebuild is keyed by the `Stay`, which every project left moves on,
and not by whether a project is open. What **cannot** be saved is how tall a group inside the
dock was dragged; freya recomputes those on every render and hands out no controller
(`notes/upstream/freya.md`).

**Startup opens what the app was given, or what it was last in.** `app()` takes the project
named on the command line and `use_restore_on_startup` prefers it (`project::open_at`, which
is `switch` without the flush, there being nothing to flush yet). Both answer with the project
and its session and nothing else: no path under them is canonicalised or reduced, so a caller
keeps the one it already holds, and `reopen`, which held none, names the one it took out of the
list. `main` answers for a path that is not a project file *before* `launch`, on the command
line it came from: a windowed program that starts and says nothing has said nothing. A path
that **is** one and still will not open is the app's to answer, and it says so in a window
(`Unopened`, which carries the `Failure` and not just the path), which is the whole of what
is left to do -- the file is never
moved aside and nothing is written over it. `reopen` is the one open nobody asked for, and so
the one that keeps quiet: a recent list naming a file that has **gone** is `None`, nothing to
reopen rather than a failure, since the list never prunes itself and that is what an ordinary
startup after a deleted project looks like. A file that is there and will not open is reported
like any other.

**Where a project is kept is `put_in`, and it serialises afresh rather than copying bytes.**
A path in a project file is relative to that file's own directory, so the same bytes in
another directory would be a claim about *that* tree; the project is therefore written out
through `Project::save_to`, and the session -- absolute throughout -- carried across. What is
pending is flushed **first**, while `Saves` still points at the old place, and what travels
is then what `Saves` holds: `written` and `stored` are the two files as they now stand, so
neither is read back. That saves two reads and a parse under the lock, on the UI thread, and
drops a failure a Save has no business having -- a project file deleted or mangled underneath
a run holding it perfectly well used to make Save write nothing. The baselines are the truer
answer besides: a project just started whose id the flush could not write has an empty file,
and a re-read would hand it to its new place with no id, and so with no session either.
`Put::Copy` is Save as:
a copy under a **new id**, because there are two projects afterwards and one id across both
would mean each matched the other's session. `Put::Move` is an unsaved project's Save: the id
stays, and the two files it came from go -- unless they are the two it just wrote, which a Save
dialog pointed at the project's own file makes them. That is asked of the canonical paths as well
as the spellings, so `projects/../projects/3.avproj` is the same file too. `Saves::moved_to` then moves the id and nothing
else -- only *where* the project is has changed, so every other baseline still describes what
the app is holding. A session write that fails does not fail the put, the project being already
where the reader asked; the session is then owed, as a failed flush's is, and a move keeps the old
session file rather than take away the only copy on disk.

**What travels comes out of one accessor**, `Saves::to_put`: the file the project is in now,
and the two files as they stand under the id the put gives them. `put_in` stayed in
`project.rs` -- it deletes the reader's files and rewrites the recent order, which is the
lifecycle's work and not "when a write happens" -- so without it the baselines it reads had
to be open to the parent module. `delete` reads the same way, through `writing_into`, and no
baseline is public.

**`close` and `delete` both end with `Saves::closed`**, which is every baseline back to what
the app boots into: the caller is about to empty the app, and a baseline still describing the
project just left would read that emptying as a change and write it back into it -- the same
ordering `switch` has. `close` flushes first and `delete` does not, the project being about
to go. `delete` refuses any path that is not under `projects/`: a project the reader gave a
place is their own file, and this app has no business deleting one whatever asked.

**A path in the project file is relative where it can be** (`Project::against`, turned one way
by `load_from` and the other by `save_to`). It is what makes a project worth checking in: a
`binaries` naming `target/debug/viewer` is a claim about the tree the file sits in, where the
absolute spelling is a claim about one machine. For a symlinked project file that directory is
the target's (`store::through_links`): the bytes are there and a save lands there, so a read of
the link and of the target mean the same tree. Only paths **under the project file's own
directory** are turned, everything else having nothing to be relative to; and it is the project
file alone -- the session beside it never travels and its digests are keyed by the paths the app
is holding. In memory a project is always absolute, so `Saves`' baselines and the app's binaries
compare as they always did; the relative spelling exists for the length of one `save_to`. What
is turned is `directory`, `binaries`, and each bookmark's *binary* path -- a `SavedDocument::Source`
is what the debug information said rather than something this filesystem was asked about.

**The agreement to run a language server is in neither file** (`agents/Lsp.md`). The reader
chose it, so by the line above it would be the project file's, but both files can be checked in
together, id and all, and an agreement travelling with them would run a language server over a
stranger's tree without asking. It is kept in the store instead, as an order of the directories
agreed to (`agreed.toml`, `src/project/trust.rs`). It still rides in `Session::trusted`, skipped
by serde, because that is the path between the app and the policy: `load_project` fills it from
the store, `Session::from_state` takes it back, and `Saves::agreement` writes the store at once
when it changes, taking the agreement off the old directory where the project moved. It grants
only a `trusted` over the pair it already held. The UI clears `trusted` when the directory or the
program is typed in, but after the record that sees the change; granting there put the new pair
in the store until the next record took it back, and an app ending between the two left the
reader agreed to a directory they were never asked about. `Saves::opened` seeds it into the
session baseline beside the id: like the directory and the bookmarks, it is restored
*synchronously*, so a baseline without it would read the state the app boots into as a change.

**Each project is two files, and the line between them is the one the save policy already drew.**
`<project>.avproj` is what the user *said* (`name`, `directory`, `binaries`, `bookmarks`) and is
written **at once**, because a binaries change is what `Saves` writes immediately; a detail
typed in follows half a second after the typing stops.
`<project>.avproj.session` beside it is what the app *noticed* (`digests`, `active`, `active_page`,
`tabs` with their trails, `history`, the record of visits) and is the file rewritten every thirty
seconds. The session takes the project file's whole name and not its stem, so the two sort together
and one ignore rule reaches both -- which is the point of the naming, a project file being something
a reader may check in.

**The id is stamped by the policy and not by the caller, once.** `Saves::id` is which project
is open; `Saves::record` puts it on the session before comparing it against the baseline, so the
stamp cannot read as a change, and builds the `Project` with it. The writes take both halves as it
handed them back, and `saves` is held under one lock from the decision to the write, so a second
stamp could only put back what the first one wrote. Ids are minted in two places. `Saves::to_put`
gives a copy its own. `Saves::opened` gives one to a file that has none -- written by hand, or
claimed empty by `start_new` -- and `written.id` stays `None`, since that is what the file holds, so
the id is **owed to the next flush** like a detail typed in. It used to wait for the first write the
reader caused, and a project the reader only read -- no binaries, nothing changed -- never had one:
every session went out under an id the file did not hold, and every load dropped it. Nothing in the
UI knows the id, which is why nothing in the UI can get it wrong.

**Building puts a `[cargo]` section in each file, and which file each half goes in is that same
line.** The profile is what the reader chose, so it is the project file's and is written the moment
they choose it, a rename's own timing. The paths the last build produced are what the app noticed,
so they are the session's. They are saved at all for one reason: a build replaces the artifacts
of the build *before* it, and the build before it may have been in another run of the app -- without
them a restart would leave the reader's binaries behind with nothing that could refresh them. Each
section is a **table of its own and is absent when it has nothing to say**, so a project nobody has
chosen a profile in and a session nothing was built in each write no section at all. A table also
leaves room for what the next build setting needs, which a loose key would not.

**Field order in these structs decides nothing.** `toml` 1.x serializes into a value tree and
writes it out plain values first, then tables, then arrays of tables, at every level and in
whatever order the struct declares them; there is no `ValueAfterTable` error left to hit. The rule
this file once followed -- every plain value before the first sub-table, on pain of a *runtime*
failure -- was `toml` 0.5's and is gone. A struct is declared in whatever order reads best, and
the round-trip tests pin what the file says rather than where a key sits in it.

So the file a user might keep, copy or hand-edit is exactly the one that changes only when they do
something. Three things follow, and they are why it
is two files rather than two tables. A session that will not parse loses a scroll position
and not the list of binaries. The file *is* the project, so a run killed between the `create_new`
and the first write reopens as the empty project it is rather than being orphaned. And a binaries
change writes **both**, so a session can never name a tab into a binary the project file has
already let go of.

`recents.toml` sits above `projects/`, beside `settings.toml`: the project files, most recently
opened first.
**Which project to reopen is the first entry and not a field of its own**; a `last` beside the list
would be a second answer the order already gives. It is an *order* and not an index of what exists
(the files are that), which is why `MAX_ORDER` (50) is safe and why nothing prunes a path
whose file has gone. Safe because the list puts back what the file did not name:
`recent_projects` follows the order with every unsaved project it left out, most recently written
first, since the list is the only way to reach one. A project with a place that falls off is the
reader's own file and opens again by it. Nothing prunes because
repairing it on load would write a file on a startup where the reader did nothing. A path under the store is written **relative to it** and every other path absolutely, so
moving the state directory does not lose every unsaved project at once; in memory they are all
absolute, the relative spelling belonging to the file and nowhere else (`write_recents`).
The **cap is the store's own**: `Store::save_order` is the one writer of an order file and cuts to
`MAX_ORDER` on the way out, so a module that keeps an order hands over what it holds and no caller
has to remember the number. Under it is `Store::save`, the log-and-swallow an order and
`settings.toml` share: these are the files the app carries on without, where a project's own write
hands its error back.
`Order::touch` answers whether anything moved, so reopening the project already at the front
writes nothing. The order itself is `order::Order<T>`, the newest-first list every list of places
in the app is: the projects', the scratchpads', a tab's trail and the record of visits. The
recent-projects view reads each row's name out of that project's own file, never out of this file:
a copy in here would be a second copy to keep in step with the one the user edits.

**Bookmarks are the project file's** (`src/bookmarks.rs`; the panel over them is
`agents/Sidebar.md`'s). A bookmark is a place the reader chose to be able to come back to, which is
the deliberate side of the split, so the list is written at once. It is a
`SavedDocument` with the **name it was made under** beside it, because a saved symbol carries only
its mangled name and a bookmark whose binary is closed has nothing else to be drawn by. A place
that can spell its own name keeps none: a symbol the *app* named rather than the file is saved as
which name it is and an address, and `Bookmark::label` renders it afresh, so the one thing that
would tie a reader's bookmarks to a spelling is not in the file. The list holds no `Arc`: a
bookmark *outlives* the binary it points into, since a reader's own list must not
shrink behind their back, where the history's rule is to drop. So whether one is live is a question
asked of the objects loaded now, wherever it is drawn, and never a fact the list keeps. So is
whether a document *is* bookmarked (`Bookmarks::matching`), since a rebuild moves a symbol while its
entry keeps the address it was made at, and the two saved forms would never agree again about a
bookmark the panel is drawing live. Nothing about it is in the session: `clear_project` leaves
the state alone and the incoming project sets it the way it sets the directory, and `close_binary`
has nothing to forget.

Inside those files, identity is **path + object name + symbol name + address** for a place in a
binary and **the path itself** for a source file, never pointers. The symbol name is a `SavedName`:
the file's own spelling, or — for one of the names the app made up (`agents/Analysis.md`) — which
of them it is (`SavedMadeUp`), the address beside it being the rest of what such a name says. The
two are separate types because neither carries the other's half: a made-up name has no string to
save and the file's own has no `MadeUp` to render, so nothing has to answer for a name that is
neither. Those are rendered again on the way back, by whatever `MadeUp` spells them now, which is
what lets the app rename them without dropping the places saved on them; a saved string would
quietly stop matching. That mapping lives in exactly two places, `SavedDocument::from_document`
and `::resolve`. A source file's path is written as its text where it is UTF-8 and as its bytes
where it is not (`any_path`), which TOML writes as an array of numbers. serde's own `PathBuf`
refuses such a path, and that refusal stops the whole file being written, so one tab or bookmark
on a file with such a name would have ended every save until it fell off the record. On Windows
the bytes are not the platform's, so such a path is written lossily and names no file on the
way back.

**One `tabs` list of every kind, not a `tabs` and a `sources` beside it**, because there is one
bar. An object's whole code is saved by its object's path and name exactly as the object's own tab
is, and joins the same list: **one `SavedDocument::Object` with a `shown` saying which of the
two**. The path and the name say both, so `shown` is the whole of what tells them apart. The
reader's own interleaved order is what comes back, and the one document that was on screen is
`active` whichever kind it is. It is written out in full
rather than as an index, since a tab that no longer resolves is *dropped* (which would shift the
index) while the active one *degrades*. **A page is a tab in that same list**, a row with a `page`
and no trail, so the bar comes back as it stood rather than with Project, Settings and the
Scratchpad swept to one end; `active_page` beside `active` is the page that was on screen, and the
two are exclusive by construction, the bar having one tab on screen. The name written is
`Page::stored` and never `Page::title`: a title is what the reader sees and may be reworded, where a
stored name changing would empty every saved bar. It is a **string** and not a serde enum, because
an unknown variant is a parse error and a session that will not parse is moved aside whole
(`Store::read`): a page this build lacks costs that one tab, where an error would cost every tab,
every trail and the record of visits. **A document `tabs` entry is a whole trail**: `temporal` +
`cursor` + `entries`, every place the tab has shown newest first with the cursor on the one it
showed, so that Back works across a restart. Reopening after a rebuild is this app's daily
loop, and a trail lost on every restart would be worth little; the cost is a file a few entries
longer per tab, capped at `history::MAX_ENTRIES` (50) per trail. Each place carries **the rows both
of its sides were left at**: an entry is a `SavedEntry` (`asm_row` + `asm_into` + `src_row` +
`src_into` + `line` + `asm_address` + `code_address` + `src_line` + `document`), rather than the
tab having arrays of rows beside its trail. Each `_into` is how far into its row the side was
left, in 65536ths of it, and absent for none (`TopRow`): a whole row alone came back snapped to
the row's top, which is nearly always, a wheel's step being no whole number of rows. A whole row
and a part rather than one float, so the row is exact however far down a listing goes.
`asm_address` is where an object's **code** tab was *scrolled* to, as a placed address, and is
absent for every other kind: that listing's rows are counted afresh as it is decoded, so a row
there is no place to come back to and an address is (`agents/UI.md`, `Places::code_at`). It is a claim about a layout, so
a rebuilt binary takes it with the rows. For such a tab `asm_row` and `asm_into` are how many rows
past the address's own row it was and how far into the last (`Spot`): a stretch's rule, header,
labels and first instruction all sit at one address. The rows travel with their place because a restore
drops the places that no longer resolve, which would shift every later row of a parallel array onto
the wrong place. They are rows and not pixel offsets so that a font change does not move every saved
position, and they are hints and not facts: `#[serde(default)]`, and clamped to what the tab holds
*now* by `Positions::row`. `line` is which line a **source-driven** tab's assembly side was driven
from and is absent for every other kind. It is what makes such a tab's `asm_row` mean anything:
without it the listing that row is a row of is not there to come back to. Nothing resolves it, being
a number and not a place, so a rebuilt binary takes the two rows with it and leaves the line, which
is simply asked again out of what is loaded now. `src_line` is the other line and not the same one:
it is which line of the file the **place** is, where the place is one in a file, and what Back comes
back to. The two part company the moment the reader clicks elsewhere in the file, which is why one
cannot be spelled with the other; the place's own is what the drive falls back to when nothing was
clicked (`agents/Panes.md`). It is no more a claim about a layout than a file is, so a rebuilt
binary keeps it. `code_address` is that pair on the other side: which placed address the **place**
is, where the place is one in an object's code, against `asm_address`'s scroll. The two part
company the moment the reader scrolls, and spelling one with the other brings a listing opened at
no place in particular back as the address it was scrolled to. It is a claim about a layout as the
scroll is, so a rebuilt binary takes both, and the place comes back as the whole listing. It
is also the address of an instruction of a symbol, the one place inside a symbol (a call it makes
to itself, `Stop::in_symbol`), there the symbol's own address and not a placed one. **Which of the
two spaces a saved number is in is the document it was saved with**, which is the one thing the
file cannot state, so `document::Address::in_document` is where it comes back; past that
the two are a `PlacedAddress` and a `SectionAddress` and cannot be swapped.
The file states the halves apart and so can state a pairing that means nothing --
a line of an object's code, an address in a file, or either address in the other's space -- which
a `history::Stop` cannot hold. So
`RestoredEntry::stop` puts them back through `Stop::paired`, which is where that rule lives: the
document says which half is its own, and a half that does not belong to it is the whole document
rather than a guess. A door's landing states them apart too and is the other caller
(`agents/Panes.md`), so the pairing is written once for both. This is the last place the two are
seen apart; nothing past it carries them. A restore answers with a `RestoredTab` per tab, a
page or a document, rather than a tuple, since the rows and the line no longer survive the same things: the live trail,
`History::rebuilt` over the places that resolved with the saved cursor carried past the ones that
did not, and a `RestoredEntry` per surviving place. A tab with nothing left on its trail is dropped
whole, and so is a page this build does not have.

`Session::digests` is the digest each binary had when the session was saved, keyed by path. It is in
the *other* file from `binaries` and not a field beside them, because `binaries` is the list to
*open* and a digest is what to *believe* afterwards. A mismatch is not an error, a dialog or a
refusal. `Loaded::of` collects the paths whose digest no longer matches, and under one of those the
**name is the identity and the address is only a tie-breaker**: a symbol that merely moved resolves,
where an unchanged file drops it, and a name that names two symbols and no longer names an address
resolves to neither, since a stale address is exactly what lands a reader on the wrong function. The
saved **row is dropped**, being a claim about a listing this build no longer has. A path with *no*
saved digest is a third state, not a mismatch: it behaves as everything did before digests existed.

`Loaded` is what a saved place is resolved against: the objects loaded now, **indexed** by the
file and member name a saved place names one by, and that set of changed paths. Indexed because a
restore resolves every entry of every tab, up to 200 visits and the active document against the
one list, and a scan per place is a component-wise `Path` compare against every member of an
archive that can hold thousands. A place resolved **on its own** scans instead
(`Loaded::scanning`, which is what `resolve_by_name` and so every bookmark uses): one index over
the whole list costs more than the one scan it saves. `project::by_file` needs neither: it is
the first object out of each file, in the order the files were opened, with how many objects
came out of it, which is what `binaries`, `binary_counts` and `digests` are each a reading of.
It groups by the run a file's objects make, the rule the Objects list follows too
(`crate::tree`), because the loader keeps one file's objects in one run. Where the list holds
two objects a saved place cannot tell apart, the first is the one that answers.

Coming back, the **active document degrades** (symbol -> its object -> nothing, since there is one
of it and the app must open somewhere) while **a trail's places and the visits are dropped** (a list
of places the reader cannot get back to is worse than a short list). A source-driven entry resolves
against nothing, so it neither degrades nor drops: a deleted file comes back as a tab over the
pane's own "Source file not found". `History::rebuilt` is the one walk both a restore and a
file-close go through for each trail, carrying the cursor to the newest survivor at or older than
it. `History::restored` also collapses duplicates and trims to the newest `MAX_ENTRIES` (50, per
tab), and `Visits::restored` does the same for the record, at its own `MAX_VISITS` (200) -- both
through `Order::restored_within`, which is where collapsing duplicates onto their newest
occurrence and the cut after it live. `Order::touch_within` is the same pair on the way in,
which is what `Visits::record` and `History::push` are.

**When** a save happens is `Saves` (`project/saves.rs`), a `static Mutex` rather than UI state
because two of the three things driving it sit outside the component tree. `Saves` decides and
holds the baselines; the `record` and `flush` the app calls are `project.rs`'s, and they put on
disk what was decided.
`record(&details, &binaries, loading, &bookmarks, session)` is called on every state change and
compares each against its baseline -- with one exception, which is how often. Where a pane is
scrolled to and where a handle is dragged are written on every scroll row and every pointer move,
and each record builds a whole `Session` and deep-compares it, so `use_save_on_change`
(`src/ui/session.rs`) is two observers: everything else records at once, and those five states
(the three `Positions` maps, the two splits) record at most once per 250 ms, from peeks, of the
state the burst has come to. What they change is session-only and waits for the flush anyway; a
close inside that window loses one scroll position, which a restore treats as a hint. By reference, all but the session: on the ordinary run nothing
about `project.toml` has changed and everything handed in is dropped, so only the write path clones.
**`Details` is the four user-given fields and `Project` holds it whole**, under `#[serde(flatten)]`,
so they are keys of `project.toml` as before and a fifth is added in one place. The baseline is
compared with `==` and the write hands the same value back, neither field by field. The one cost:
serde reads a flattened field out of a buffer, which `toml` can put no span on, so a hand-edited
`directory = 7` is reported at line 1 rather than where it is, where `id = 7` beside it still names
its place. The session is the exception -- it has to be built to be compared, and it
is kept when it differs. A change to the `binaries` writes **both files immediately**.
A change to the `bookmarks` writes **the project file alone**, since it lets go of no binary and
so cannot leave the two files disagreeing. A change to the user-given `details` (the directory,
the server, the files, the profile) is the project file alone for the same reason, but it is
**owed** rather than written: three of the four are boxes, and a box changes them once per
keystroke, each write an fsync on the UI thread. `Saves::owed_project` holds it and `flush`
writes it, so every flush -- the close hook's, `switch`'s, `put_in`'s -- takes it along; the
save observer also calls `flush_project` once `Proj` has been still for `SETTLE` (`Settle`,
`src/ui/session.rs`). A write that does go at once, for the binaries or the bookmarks, carries
the details the app holds, so it clears what is owed, and details changed back owe nothing.
The owed write is policy-side and not a debounce in front of `record` so that no flush can miss
it: a Save pressed straight after typing keeps what was typed.
A change to only the session marks it **pending**: a tab is
expressed against the binaries rather than the other way round, costs one click to remake, and
arrives on every navigation, since `open_document` pushes onto a trail or opens a tab on the way
to each change of document. Nothing in `record` has to *say* which is which: which file a field
lives in is what decides it, and the `Option<Session>` beside the `Project` in the `Recorded` it
hands back is how it says which half it decided. `flush()` writes what is owed, then the pending
session, on a 30s
timer and from the window's close hook, which is the one exit hook freya 0.4 has
(`WindowConfig::with_on_close`, a `Send` callback that cannot read any `State`, which is exactly
why the policy is a static).

**A baseline is what the file holds, so it moves with the write and not with the decision to write
it.** `record` and `take_owing` only hand back what to write; the caller moves the baselines
afterwards, through `wrote_project` and `wrote_session` and only where `write_or_warn` answered that
the file was written. A failure leaves the change for the next `record` to see again, and hands
what was being written back to be owed to the next `flush`: the session to `owes_session`, the
project file to `owes_project`, with the `binaries_changed` its write needs. A session carried by a
binaries change whose project write failed is owed with it rather than written, and `flush` holds
the session back while that project file is still owed, so the session never names a tab into a
binary the project file does not list. `take_owing` empties pending rather than copying it for
that reason: what it hands out is either written or handed back.
Advancing first meant that a disk full for one tick left the app believing a
file held a session that never reached it: nothing marked the session pending again, so the close
hook's flush found nothing to do and the reader kept the one from before, for one warning in a log a
windowed app never shows. Which baselines a project-file write moves is the
`binaries_changed` beside it -- `Saves::written` becomes the project just written whatever
happened, and the app's own list of binaries moves only when the change was to them, since any
other write puts back the list the file already held.

**Every baseline is the state the app boots into**, which is why the binaries and the session start
empty and `Saves::written` does not. The binaries and the session are restored *asynchronously*:
the app boots holding nothing and fills in when the parse lands. So seeding them from the loaded
project would make the first comparison see the still-empty boot state as a change and write an
empty project over a good one. `Saves::written` -- `project.toml` as the file holds it, which is
one baseline for the directory, the server, the profile and the bookmarks together -- *is* seeded
whole by `reopen`, because the directory and the bookmarks are restored *synchronously*, into
`Proj` and `Bookmarked`, before a single effect has run. An effect's first run is a later pass than
the render whose `use_hook` set them (`agents/Headless.md`), so registering the save observer
before the restore is not what keeps them apart and does not have to be. Until the project view
held them (`Proj`), `Saves` **carried** the details across the calls instead of comparing them
against a baseline; a change to one now arrives through `record` like everything else and that
special case is gone. The binaries in `Saves::written` are the one part of it nothing is compared
against: they are what the project file currently *says*, and a write that is not about the
binaries writes them back rather than the app's own list. Otherwise a change during the startup
parse, or after a restore that opened none of them, would forget a file through a change that had
nothing to do with it.
The same holds for a binary the load produced nothing for -- deleted, being relinked, never built
on this machine -- while others did load: the reader did not remove it. `Saves::unheld` is the
file's binaries the app has not held since the project was opened, and a write about the binaries
puts them back where they were in the list. One leaves the set only by being held, so closing it
after that is a removal like any other. The cost is that one gone for good stays in the file, and
nothing in the app shows it or takes it out.

**`Saves::stored` is the session file itself**, beside the empty baseline rather than instead
of it. The baseline answers "has this changed", which needs the boot state; `put_in` asks
"what does the file hold", which needs the session the project was opened on. The two are the
same the moment anything has been written, so `wrote_session` moves both; before that they
differ, for as long as a load holds every session back.

**A list still being read is not the app's list**, which is the `loading` flag. The objects arrive
one at a time, so while a load is in flight `record` neither compares the binaries nor writes them:
the first to land would otherwise put a project naming only itself on disk. The session the app
holds until `restore_project` has resolved its tabs is held back the same way, and for a further
reason: a session is only ever marked pending, so marking that tabless one is already enough to
lose the tabs -- the next flush writes whatever is pending, and the close hook, `switch`, `close`
and the 30-second timer all flush. A session left pending *before* the load began describes a real
state and is left alone. The boot state is never one: `restore_project` registers its load before
it spawns the task that reads it, since the save observer's first run is a task queued ahead of
that one, and a record there would mark the tabless boot session pending. A build's reopen does
the same for the same reason (`agents/Sidebar.md`). Both baselines stay behind the streamed list,
so the record that follows the load is the one that sees the change and writes both files -- the
save observer reads `Loads` as well, which is what re-runs it when the load ends. The cost is that a binary opened or closed
while another is being read waits for that same record instead of reaching the disk at once.
A binaries write that failed before the load does not wait: every record in the window tries it
again. Left to that record, a details-only record in between replaced it with a write naming none
of them, and the next flush let the session out ahead of the project file.

**Which project is open is `Saves`' too**, and changing it at runtime is `switch(id)` or
`start_new()`. Both `flush` the project being left while the policy still points at it, `remember`
the one being entered at the front of `recents.toml`, and re-point every baseline through
`Saves::opened`, to empty, because the app is about to be emptied. Emptying it is the caller's half
and stays in `ui/session.rs`, the states being the UI's. `recent_projects(&store)` is the
list the views draw, read once per project into the root's `Recents`: `recents.toml`'s order and
then the unsaved projects it does not name, each row described by reading *that project's own* file,
with an id whose directory has gone dropped here. The list never prunes itself on load, and this is
the point of use where the repair is free.

Startup reopens the **last project**: `project::reopen`, the front of `recents.toml`, both halves
of it. `use_restore_on_startup` knows nothing about where they came from, which is what keeps a
project picker out of it. The binaries stream in the way any other open does, so the sidebar fills
in behind them, but the **session waits for the whole load**: an object or symbol tab, a selection
or a history entry is resolved against the objects by name, and resolving one against a half-filled
list would drop the tabs whose object had not landed yet. A project left during that wait restores
nothing: leaving ends the load, and the session waiting on it is not the open project's. The test
is `Loads::left`, a load `clear` stopped, and not whether the load is still running, since closing
its files ends it too; nor which project file is open, since leaving one and opening it again starts
a restore of its own. The **pages go back before any of that
and synchronously**, at the places they had in the bar and with the one that was on screen raised:
a page resolves against no object, so a session whose only tab was Settings has nothing to wait
for. The documents follow, in `restore_documents`: after the load where there are binaries, and
**at once where there are none**. A source place resolves against no object as a page does, so a
project with no binaries -- one opened by its directory and read in the Files view, or one whose
binaries have all been deleted -- still comes back with the files the reader had open. Whatever the
objects list holds, the tabs naming an object that is not there are dropped and the rest put back.
**The visits, the tabs and the active document are one question and are answered as one**, by
`Session::restore`: it builds one `Loaded` -- the objects indexed, the saved digests walked once
against them -- and resolves all three under it, so a tab and the active document cannot be read
against two different answers about which binaries have changed, and a caller cannot take one and
forget the others. `Session::pages`
and `shown_page` stay outside it, being the pages' half and going back first either way.
`restore_documents` sets the visits, then for each restored tab opens its trail whole
(`Docs::open_trail`, temporal flag and all), calls `place_entries` and puts the tab in the bar at
the place it had -- counted over what survived, so the tabs that resolved keep their order around
the pages already there -- and then raises the tab already showing the active document, or, for
one that degraded, opens it with `Reach::NewTab`. It raises rather than opens because opening a
place a tab already shows promotes that tab, and the tab on screen is often the temporal one. A
session left on a page has no active document, and that page is raised again instead: every tab put
in the bar is shown as it goes in. Two orderings are load-bearing.
The **rows go into the `Positions` maps, and the driven line into `Driven`, per entry and before
the tab is shown**: those maps are the one thing the restore writes directly, which is why the
writes have a name of their own (`place_entries`), and a pane puts its view back when it notices
the place it is showing has changed, so a row arriving after the tab is on screen arrives after the
only moment anything looks at it. And tabs go before the active document, because the active
document is looked for among them, and one opened first would go beside whichever tab was on screen
instead of in place. The saved order is stated outright rather than reproduced by opening each tab
beside the one before it. A place that no longer resolves is **dropped** off its trail, like a
visit, and a tab left with none is dropped. A source-driven place is never resolved at all, so a
file that has been deleted comes back as a tab over the pane's own "Source file not found" rather
than silently vanishing.

**The settings are a file of their own, above the projects** (`src/settings.rs`, `settings.toml` at
the top of the state directory beside `recents.toml`, since a setting is the user's and not any one
project's; same atomic `.tmp` + rename, same "a missing, unreadable or corrupt file is simply the
default"). The split is the one `notes/specs/Storage.md` states. `Settings` is the theme choice
(`Theme`: light, dark or follow the desktop) and a `FontSetting`, a family and a size, for each of
the interface and fixed-width fonts. **Every field is an `Option` and `None` is a real third
state**: "the user has not said, ask the desktop", which is neither an empty string nor the
desktop's current answer copied into the file. An unspecified field is therefore a key that is
*absent* from the TOML (`skip_serializing_if`, since TOML has no null anyway), so nothing can later
mistake an inherited value for a chosen one, and the settings page can show the difference. Sizes
are stored in **points**, the unit the desktops answer in, so an override and the value it overrides
are comparable; `fonts.rs` converts once at the end. There is **no `Saves`-shaped policy and
no autosave timer**, only the font family box's reason for waiting: a change is **owed**
(`Settings::owe`, a static holding the newest settings and their store) and `settings::flush`
writes it, once the changes settle (`Settle`) and from the close hook. **Resolving
`Theme::Desktop` is deliberately not this module's job**: "which theme does the desktop prefer" is a
question for whatever owns the window, so `settings.rs` holds only the choice and stays
framework-free, and `ui/palette.rs` puts the two together (`resolve_appearance`). It once spawned a
subprocess per platform to answer it and no longer does: the windowing system already knows, it
answers on every platform this runs on, and its answer is live rather than a value baked in at
startup. `fonts::resolve` merges the settings over the desktop's answer **field by field**, pure and
tested, and `fonts::inherited` is that same merge of *nothing*: what an unspecified field is falling
through to, which is what the settings page draws in an empty box. Everything in `fonts.rs` is in
**points** up to one conversion at `Font::size`, the app's own defaults included (9pt and 10.5pt),
because an override and the value it overrides have to be the same kind of number for the page to
put them beside each other. The desktop's answer is cached per process (`desktop_answer`), since the
page re-resolves on every change and a lookup is a subprocess.

**A panic is written down beside everything else the app stores** (`src/panics.rs`,
`Store::panics()`). This is a windowed program: the default hook writes a line to a stderr
nobody is looking at, and the work is done on threads of its own, so a panicking worker left a
pane waiting for an answer that was never coming and no trace anywhere. The hook writes the
thread, the location, the message and a `Backtrace::force_capture` -- forced, so a backtrace does
not depend on `RUST_BACKTRACE` in whatever environment the app was launched from -- **appending**
one record per panic to one file per launch, where every other file the app stores is replaced
whole by `write_atomically`. A file per launch keeps a run's panics in one place and in order.
**Guarded panics are capped at twenty a run** (`MAX_GUARDED`), and one past the cap is not even
captured: the demangler is guarded per name, so a file that upsets it raises a panic per symbol,
and each capture symbolizes a backtrace under a lock the whole process shares. Uncapped, one such
file stalled the demangler pool and wrote hundreds of megabytes of records. That file and whether the app is already on its way down are
what a run remembers across its panics, and they are one `Run`: the installed hook keeps one
static of it and hands it to the rule, so a test can have a run of its own, the tests sharing
one process.

Three things the hook's own position decides. It runs **before the unwind**, so it can ask
`analysis::guard::guarded()` whether the panic is one the crate catches on purpose: those are
written down and nothing else happens, since nothing has gone wrong with the app. It runs on the
panicking thread while that thread still holds whatever it held, and `std::sync::Mutex` is not
reentrant, so the shutdown -- `shutdown::before_exit`, the project, the settings and the
scratchpads flushed and then every child the app started stopped -- goes on **a thread of its own** and reaches the lock
only once the unwind has let it go. **The main thread waits for that thread**, for five seconds at
most: it is the UI thread, nothing between it and `main` catches an unwind (neither freya nor
winit on Linux), and an unwind out of `main` ends the process with the save half written and
rust-analyzer left running. The bound is for a lock the main thread itself holds, which only its
unwind lets go. Any other thread returns into its unwind at once, since the main thread keeps the
process alive. **A later panic on the main thread waits too**, for the one shutdown the run has:
its unwind ends the process just as the first one's would. If the first panic's box is still open,
that panic starts the shutdown itself rather than wait for the reader, and the box goes with the
process: rfd shows a worker's box on macOS by handing it to the main thread, so waiting for it
there would never end. **A later panic on any other thread starts nothing**: workers fail
together, from one cause or on the channel the first one held, and a second worker's panic used to
start the shutdown and end the process under the first one's box. So the shutdown starts when
that box is closed or the main thread panics, whichever is first. And it is installed from
`ui::app`'s first render rather than from `main`, which is freya's doing (`notes/upstream/freya.md`): a hook set before `launch` is the inner one,
and freya's box would be up and the process gone before ours ran. The app's workers are named
(`thread::Builder::name`) for the one reason that the box then says which of them died.

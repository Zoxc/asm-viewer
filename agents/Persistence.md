# Persistence

What the app writes to disk and when: the projects, each one's two files, the session restore, the
recents order and the user's own settings. Scratchpads have their own storage and are in
`agents/Scratchpad.md`.

There is **no published version of this app yet**, so persisted formats need no backward
compatibility: a schema change is just a schema change, a stale file is ignored rather than
migrated, and `#[serde(default)]` is added only when it earns its place on its own merits.

**Ignored, but not lost** (`src/rescue.rs`). Every one of these files is read back into a default
when it will not parse, and the next write puts a good file over it -- so the one thing that rule
costs is the reader's own file, taken away without a word. `rescue::parse` is what every load that
is on the way to a *write* goes through: it reads the **bytes** (a file that is not UTF-8 will not
parse either, and is lost the same way), and on a failure copies them to `incompatible/` under the
path the file had, removes the original, and answers `None` -- which is what all four of these loads
already meant by "not there", so no caller changed shape. A file the system will not hand over at
all is left alone: nothing can be salvaged from it, and nothing is about to write over it either.
The mirror (`incompatible/projects/<id>/session.toml`) is so that a project's files keep the shape
they had rather than being flattened into one heap of `session.toml`s, and the destination is
claimed by `File::create_new` -- `settings.toml`, then `2-settings.toml` -- which is
`ProjectId::anonymous`'s "a create that fails rather than opens", and for its reason: nothing there
is ever overwritten, not by an earlier rescue and not by a second copy of the app moving the same
file at this moment. The original is **removed** rather than copied, since nothing writes over
`settings.toml` until a setting changes and a file left in place would be rescued again on every
launch.

The one load that does **not** go through it is `Project::load_from` as `recent_projects_in` calls
it, which reads every listed project's file to describe its row. Drawing a row is a reading of a
project and not a claim on it: nothing is about to write over a project nobody has entered, so its
file stays where it is until it is opened. `load_project`, which *is* the open, rescues both halves.

**And the reader is told**, which is the half that makes it a rescue at all: a file moved somewhere
nobody hears about is a file lost politely. `rescue::moved()` hands over the destinations recorded
since it was last asked -- a `static Mutex<Vec<PathBuf>>`, because what fills it is a load and not a
component -- and `RescuedPopup` (`src/ui/rescued_view.rs`) names them over freya's `Popup`, which is
shown exactly when it has children, so the list being empty *is* the window not being there. It is
asked twice, at the two points a run loads any of these files. `app()` asks **after**
`use_restore_on_startup`, the three loads a startup makes (`Settings::load`, the same again behind
`fonts()`, and the project reopened) all being synchronous and all landing before that line;
`switch_project` asks again and **adds** to the list rather than setting it, so a window still naming
what the startup moved does not lose it when a project is switched. Neither `PopupTitle` nor
`PopupContent` is used: both set a font size of their own, which would draw this in a size the reader
never chose.

Everything is written under `dirs::state_dir()` (falling back to `data_local_dir()`) +
`assembly-viewer/`, atomically via `.tmp` + rename (one `write_atomically`, used by every file
`project.rs` owns). The temporary is **synced before the rename**, because a rename is atomic
against a crash of the process and not against a power loss: the directory entry can reach the disk
ahead of the data, and the file the next launch reads is then zero bytes or a truncated tail -- one
that will not parse, so the rescue moves the reader's project or session aside and hands back a
default, which is the very loss the dance exists to prevent. The directory entry is left unsynced:
losing the rename costs the last save, where losing the data costs the file. One fsync per save, at
most one every 30 s.

**A project is a directory, and its id is that directory's name.** More than one exists; each is
`projects/<id>/`. `ProjectId` is a validated single path component (ASCII alphanumerics, `-` and
`_`, first character alphanumeric), because it is interpolated into a path and is read back out of a
file a user can edit. An **anonymous** project is one whose `name` key is simply *absent*, the way
an unspecified font is in `settings.rs`. Its id is the first free `project-N`, claimed by a
`create_dir` that **fails rather than opens**, so the claim is one atomic operation rather than a
listing followed by a race. It survives a restart because it is a directory, and it costs the user
no decision. The `project-N` spelling carries no meaning: naming a project later does not move it. A
project directory is created by the **first write that has something to say** (`open_project`,
reached only from `record`/`flush`), so a run in which nothing was ever opened leaves nothing
behind.

**Each project is two files, and the line between them is the one the save policy already drew.**
`project.toml` is what the user *said* (`name`, `directory`, `binaries`, `bookmarks`) and is written
**at once**, because a binaries change is what `Saves` writes immediately. `session.toml` is what
the app *noticed* (`digests`, `active`, `active_page`, `tabs` with their trails, `history`, the
record of visits) and is the file rewritten every thirty seconds.

**Building puts a `[cargo]` section in each file, and which file each half goes in is that same
line.** The profile is what the reader chose, so it is `project.toml`'s and is written the moment
they choose it, a rename's own timing. The paths the last build produced are what the app noticed,
so they are `session.toml`'s. They are saved at all for one reason: a build replaces the artifacts
of the build *before* it, and the build before it may have been in another run of the app -- without
them a restart would leave the reader's binaries behind with nothing that could refresh them. Each
section is a **table of its own and is absent when it has nothing to say**, so a project nobody has
chosen a profile in and a session nothing was built in each write no section at all. A table also
leaves room for what the next build setting needs, which a loose key would not.

Both are placed after the plain values of their struct, which is the field-order rule the file has
followed since `toml` 0.x. Note that the rule is now belt and braces: `toml` 1.x's serializer emits
every plain value before every table whatever order the struct declares them in, so the round-trip
tests can no longer fail on a misplaced field. The declaration order is kept anyway, because it is
what a reader of the struct is entitled to assume and because nothing here wants to depend on that
serializer detail. So the file a user might keep, copy or hand-edit is
exactly the one that changes only when they do something. Three things follow, and they are why it
is two files rather than two tables. A `session.toml` that will not parse loses a scroll position
and not the list of binaries. The directory *is* the project, so a run killed between `create_dir`
and the first write reopens as the empty project it is rather than being orphaned. And a binaries
change writes **both**, so `session.toml` can never name a tab into a binary `project.toml` has
already let go of.

`recents.toml` sits above `projects/`, beside `settings.toml`: the ids, most recently opened first.
**Which project to reopen is the first entry and not a field of its own**; a `last` beside the list
would be a second answer the order already gives. It is an *order* and not an index of what exists
(the directories are that), which is why `MAX_RECENTS` (50) is safe and why nothing prunes an id
whose directory has gone: repairing it on load would write a file on a startup where the reader did
nothing. `Recents::touch` answers whether anything moved, so reopening the project already at the
front writes nothing. The recent-projects view reads each row's name out of that project's own
`project.toml`, never out of this file: a name copied in here would be a second copy to keep in step
with the one the user edits.

**Bookmarks are `project.toml`'s** (`src/bookmarks.rs`; the panel over them is
`agents/Sidebar.md`'s). A bookmark is a place the reader chose to be able to come back to, which is
the deliberate side of the split, so the list is written at once like a rename. It is a
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
bookmark the panel is drawing live. Nothing about it is in `session.toml`: `clear_project` leaves
the state alone and the incoming project sets it the way it sets the name, and `close_binary` has
nothing to forget.

Inside those files, identity is **path + object name + symbol name + address** for a place in a
binary and **the path itself** for a source file, never pointers. The symbol name is a `SavedName`:
the file's own spelling, or — for one of the names the app made up (`agents/Analysis.md`) — which
of them it is, the address beside it being the rest of what such a name says. Those are rendered
again on the way back, by whatever `MadeUp` spells them now, which is what lets the app rename them
without dropping the places saved on them; a saved string would quietly stop matching. That mapping
lives in exactly two places, `SavedDocument::from_document` and `::resolve`. A source file's path
is a `String`, since it is what the debug info said rather than something this filesystem was
asked about.

**One `tabs` list of every kind, not a `tabs` and a `sources` beside it**, because there is one
bar. `SavedDocument::Code`, an object's whole code, is saved by its object's path and name exactly
as the object is and joins the same list. The reader's own interleaved order is what comes back, and
the one document that was on screen is `active` whichever kind it is. It is written out in full
rather than as an index, since a tab that no longer resolves is *dropped* (which would shift the
index) while the active one *degrades*. **A page is a tab in that same list**, a row with a `page`
and no trail, so the bar comes back as it stood rather than with Project, Settings and the
Scratchpad swept to one end; `active_page` beside `active` is the page that was on screen, and the
two are exclusive by construction, the bar having one tab on screen. The name written is
`Page::stored` and never `Page::title`: a title is what the reader sees and may be reworded, where a
stored name changing would empty every saved bar. It is a **string** and not a serde enum, because
an unknown variant is a parse error and a session that will not parse is moved aside whole
(`rescue`): a page this build does not have costs that one tab, where an error would cost every tab,
every trail and the record of visits. **A document `tabs` entry is a whole trail**: `temporal` +
`cursor` + `entries`, every place the tab has shown oldest first with the cursor on the one it
showed, so that Back works across a restart. Reopening after a rebuild is this app's daily
loop, and a trail lost on every restart would be worth little; the cost is a file a few entries
longer per tab, capped at `history::MAX_ENTRIES` (50) per trail. Each place carries **the rows both
of its sides were left at**: an entry is a `SavedEntry` (`asm_row` + `src_row` + `line` +
`asm_address` + `src_line` + `document`), rather than the tab having arrays of rows beside its
trail.
`asm_address` is where an object's **code** tab was left, as a placed address, and is absent for
every other kind: that listing's rows are counted afresh as it is decoded, so a row there is no
place to come back to and an address is (`agents/UI.md`, `CodeAt`). It is a claim about a layout, so
a rebuilt binary takes it with the rows. How many rows past the address the tab was is not saved, a
label being a fine place to come back to. The rows travel with their place because `resolve_tabs`
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
binary keeps it. `resolve_tabs` answers with a `RestoredTab`, a page or a document,
rather than a tuple, since the rows and the line no longer survive the same things: the live trail,
`History::rebuilt` over the places that resolved with the saved cursor carried past the ones that
did not, and a `RestoredEntry` per surviving place. A tab with nothing left on its trail is dropped
whole, and so is a page this build does not have. **Field order within these structs is load-bearing**: TOML emits plain values before tables,
so `binaries` sits beside the name only because every other field of `Project` is a plain value and
`bookmarks`, the one array of tables in that file, comes last; `SavedTab`'s `temporal` and `cursor`
must precede its `entries`, a `SavedEntry`'s rows its `document`, a `SavedDocument::Symbol`'s
`address` its `symbol_name` (a `SavedName` is written as a table where it holds the file's own
name), and a `Bookmark`'s `name` its `document` (`SavedHistory` has no plain field at all).
Getting it wrong fails at *runtime*, not at compile time, and a round trip through real TOML per
struct is what holds it.

`Session::digests` is the digest each binary had when the session was saved, keyed by path. It is in
the *other* file from `binaries` and not a field beside them, because `binaries` is the list to
*open* and a digest is what to *believe* afterwards. A mismatch is not an error, a dialog or a
refusal. `Rebuilt` collects the paths whose digest no longer matches, and under one of those the
**name is the identity and the address is only a tie-breaker**: a symbol that merely moved resolves,
where an unchanged file drops it, and a name that names two symbols and no longer names an address
resolves to neither, since a stale address is exactly what lands a reader on the wrong function. The
saved **row is dropped**, being a claim about a listing this build no longer has. A path with *no*
saved digest is a third state, not a mismatch: it behaves as everything did before digests existed.

Coming back, the **active document degrades** (symbol -> its object -> nothing, since there is one
of it and the app must open somewhere) while **a trail's places and the visits are dropped** (a list
of places the reader cannot get back to is worse than a short list). A source-driven entry resolves
against nothing, so it neither degrades nor drops: a deleted file comes back as a tab over the
pane's own "Source file not found". `History::rebuilt` is the one walk both a restore and a
file-close go through for each trail, carrying the cursor to the last survivor at or before it.
`History::restored` also collapses duplicates and trims to the newest `MAX_ENTRIES` (50, per tab),
and `Visits::restored` does the same for the record, at its own `MAX_VISITS` (200).

**When** a save happens is `Saves` in `project.rs`, a `static Mutex` rather than UI state because
two of the three things driving it sit outside the component tree.
`record(details, binaries, loading, bookmarks, session)` is called on every state change and
compares each against its baseline. A change to the `binaries` writes **both files immediately**. A
change to the user-given `details` (the name and the directory) or to the `bookmarks` writes
**`project.toml` alone**, since neither lets go of a binary and so neither can leave the two files
disagreeing. A change to only the session marks it **pending**: a tab is expressed against the
binaries rather than the other way round, costs one click to remake, and arrives on every
navigation, since `open_document` pushes onto a trail or opens a tab on the way to each change of
document. Nothing in `record` has to *say* which is which: which file a field lives in is what
decides it, and the `Option<Session>` beside the `Project` in the `Recorded` it hands back is how it
says which half it decided. `flush()` writes the pending session, on a 30s timer and from the
window's close hook, which is the one exit hook freya 0.4 has (`WindowConfig::with_on_close`, a
`Send` callback that cannot read any `State`, which is exactly why the policy is a static).

**A baseline is what the file holds, so it moves with the write and not with the decision to write
it.** `record` and `owing` only hand back what to write; the caller moves the baselines afterwards,
through `wrote_project` and `wrote_session` and only where `write_or_warn` answered that the file
was written. A failure leaves the change for the next `record` to see again and the session pending
for the next `flush`. Advancing first meant that a disk full for one tick left the app believing a
file held a session that never reached it: nothing marked the session pending again, so the close
hook's flush found nothing to do and the reader kept the one from before, for one warning in a log a
windowed app never shows. Which baselines a `project.toml` write moves is the
`binaries_changed` beside it -- the details and the bookmarks always, the binaries only when the
change was to them, since any other write puts back the list the file already held.

**Every baseline is the state the app boots into**, which is why two of them start empty and two do
not. The binaries and the session are restored *asynchronously*: the app boots holding nothing and
fills in when the parse lands. So seeding them from the loaded project would make the first
comparison see the still-empty boot state as a change and write an empty project over a good one.
`Saves::given` and `Saves::bookmarks` *are* seeded by `reopen`, because the name, the directory and
the bookmarks are restored *synchronously*, into `Proj` and `Bookmarked`, before a single effect has
run. An effect's first run is a later pass than the render whose `use_hook` set them
(`agents/Headless.md`), so registering the save observer before the restore is not what keeps them
apart and does not have to be. Until the project view held a name (`Proj`), `Saves` **carried** the
name across the calls instead of comparing it against a baseline; a rename now arrives through
`record` like everything else and that special case is gone. `Saves::listed` is the one piece of
bookkeeping that grew out of it: it is what `project.toml` currently *says* the binaries are, and a
write that is not about the binaries writes that back rather than the app's own list. Otherwise a
rename during the startup parse, or after a restore that opened none of them, would forget a file
through a change that had nothing to do with it.

**A list still being read is not the app's list**, which is the `loading` flag. The objects arrive
one at a time, so while a load is in flight `record` neither compares the binaries nor writes them:
the first to land would otherwise put a project naming only itself on disk, and beside it the empty
session the app holds until `restore_project` has resolved its tabs. Both baselines stay behind the
streamed list, so the record that follows the load is the one that sees the change and writes both
files -- the save observer reads `Loads` as well, which is what re-runs it when the load ends. The
cost is that a binary opened or closed while another is being read waits for that same record
instead of reaching the disk at once.

**Which project is open is `Saves`' too**, and changing it at runtime is `switch(id)` or
`start_new()`. Both `flush` the project being left while the policy still points at it, `remember`
the one being entered at the front of `recents.toml`, and re-point every baseline through
`Saves::opened`, to empty, because the app is about to be emptied. Emptying it is the caller's half
and stays in `ui/project_view.rs`, the states being the UI's. `recent_projects()` is the list a view
draws: `recents.toml`'s order, each row described by reading *that project's own* `project.toml`,
with an id whose directory has gone dropped here. The list never prunes itself on load, and this is
the point of use where the repair is free.

Startup reopens the **last project**: `project::reopen`, the front of `recents.toml`, both halves of
it. `use_restore_on_startup` knows nothing about where they came from, which is what keeps a project
picker out of it. The binaries stream in the way any other open does, so the sidebar fills in behind
them, but the **session waits for the whole load**: a tab, a selection or a history entry is
resolved against the objects by name, and resolving one against a half-filled list would drop the
tabs whose object had not landed yet. The **pages go back before any of that and synchronously**, at the
places they had in the bar and with the one that was on screen raised: a page resolves against no
object, so a session whose only tab was Settings has nothing to wait for and a project with no
binaries at all still comes back as it was left. The documents are then restored. `restore_project`
sets the visits, then for each saved tab opens its trail whole (`Docs::open_trail`, temporal flag
and all), writes its rows and puts it in the bar at the place it had -- counted over what survived,
so the tabs that resolved keep their order around the pages already there -- and then opens the
active document with `Reach::NewTab`, which raises the tab already showing it and, for one that
degraded, opens a tab. Two orderings are load-bearing. The **rows go
into the `Positions` maps, and the driven line into `Driven`, per entry and before the tab is
shown**: those maps are the one thing the restore writes directly, and a pane puts its view back
when it notices the place it is showing has changed, so a row arriving after the tab is on screen
arrives after the only moment anything looks at it. And tabs go before the active document, because
`open_document` opens what it cannot find and would otherwise put it beside whichever tab was on
screen instead of finding it in place. The saved order is stated outright rather than
reproduced by opening each tab beside the one before it. A place that no longer
resolves is **dropped** off its trail, like a visit, and a tab left with none is dropped. A
source-driven place is never resolved at all, so a file that has been deleted comes back as a tab
over the pane's own "Source file not found" rather than silently vanishing.

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
are comparable; `fonts.rs` converts once at the end. Field order is load-bearing here too (`theme`
is a plain value and the two fonts are tables), and the round-trip test is what holds it. There is
**no `Saves`-shaped policy and deliberately no second autosave timer**: a settings change is already
as rare as a deliberate action, so `Settings::save` is public and writes at once. **Resolving
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

**A panic is written down beside everything else the app stores** (`src/panics.rs`, `panics/`
under `project::base`). This is a windowed program: the default hook writes a line to a stderr
nobody is looking at, and the work is done on threads of its own, so a panicking worker left a
pane waiting for an answer that was never coming and no trace anywhere. The hook writes the
thread, the location, the message and a `Backtrace::force_capture` -- forced, so a backtrace does
not depend on `RUST_BACKTRACE` in whatever environment the app was launched from -- **appending**
one record per panic to one file per launch, where every other file the app stores is replaced
whole by `write_atomically`. A file per launch is what keeps a bad input honest: a guarded panic
fires once per name, so a file that upsets the demangler is twenty thousand records in one file
rather than twenty thousand files.

Three things the hook's own position decides. It runs **before the unwind**, so it can ask
`analysis::guard::guarded()` whether the panic is one the crate catches on purpose: those are
written down and nothing else happens, since nothing has gone wrong with the app. It runs on the
panicking thread while that thread still holds whatever it held, and `std::sync::Mutex` is not
reentrant, so the shutdown -- `project::flush`, then `scratchpad::stop_all` -- goes on **a thread
of its own** and reaches the lock only once the unwind has let it go. And it is installed from
`ui::app`'s first render rather than from `main`, which is freya's doing (`notes/upstream/freya.md`):
a hook set before `launch` is the inner one, and freya's box would be up and the process gone
before ours ran. The app's workers are named (`thread::Builder::name`) for the one reason that
the box then says which of them died.

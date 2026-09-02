# Persistence

What the app writes to disk and when: the projects, each one's two files, the session restore,
the recents order and the user's own settings. Scratchpads have their own storage and are in
`agents/Scratchpad.md`.

There is **no published version of this app yet**, so persisted formats need no backward
compatibility: a schema change is just a schema change, a stale file is ignored rather than
migrated, and `#[serde(default)]` is added only when it earns its place on its own merits.

Everything is written under `dirs::state_dir()` (falling back to `data_local_dir()`) +
`assembly-viewer/`, atomically via `.tmp` + rename (one `write_atomically`, used by every file
`project.rs` owns).

**A project is a directory, and its id is that directory's name.** More than one exists;
each is `projects/<id>/`, and `ProjectId` is a validated single path component — ASCII
alphanumerics, `-` and `_`, first character alphanumeric — because it is interpolated into a
path and is read back out of a file a user can edit. An **anonymous** project is one whose
`name` key is simply *absent*, the way an unspecified font is in `settings.rs`; its id is the
first free `project-N`, claimed by a `create_dir` that **fails rather than opens**, so the claim
is one atomic operation rather than a listing followed by a race. It survives a restart because
it is a directory, and it costs the user no decision. The `project-N` spelling carries no
meaning: naming a project later does not move it. A project directory is created by the **first
write that has something to say** (`open_project`, reached only from `record`/`flush`), so a run
in which nothing was ever opened leaves nothing behind.

**Each project is two files, and the line between them is the one the save policy already
drew.** `project.toml` is what the user *said* — `name`, `directory`, `binaries` — and is written
**at once**, because a binaries change is what `Saves` writes immediately. `session.toml` is what
the app *noticed* — `shown`, `digests`, `selection`, `tabs`, `sources`, `history` — and is the
file rewritten every thirty seconds. So the file a user might keep, copy or hand-edit is exactly
the one that changes only when they do something. Three things follow, and they are why it is two
files rather than two tables: a `session.toml` that will not parse loses a scroll position and
not the list of binaries; the directory *is* the project, so a run killed between `create_dir` and
the first write reopens as the empty project it is rather than being orphaned; and a binaries
change writes **both**, so `session.toml` can never name a tab into a binary `project.toml` has
already let go of.

`recents.toml` sits above `projects/`, beside `settings.toml`: the ids, most recently opened
first. **Which project to reopen is the first entry and not a field of its own** — a `last` beside
the list would be a second answer the order already gives. It is an *order* and not an index of
what exists (the directories are that), which is why `MAX_RECENTS` (50) is safe and why nothing
prunes an id whose directory has gone: repairing it on load would write a file on a startup where
the reader did nothing. `Recents::touch` answers whether anything moved, so reopening the project
already at the front writes nothing. The recent-projects view reads each row's
name out of that project's own `project.toml`, never out of this file: a name copied in here would
be a second copy to keep in step with the one the user edits.

Inside those files, identity is **path + object name + symbol name + address** for a place in a
binary and **the path itself** for a source file, never pointers; that mapping lives in exactly two
places, `SavedDocument::from_document` and `::resolve`. A source file's path is a `String`, since
it is what the debug info said rather than something this filesystem was asked about.

**One `tabs` list of every kind, not a `tabs` and a `sources` beside it**, because there is one
strip — `SavedDocument::Code`, an object's whole code, is saved by its object's path and name
exactly as the object is and joins the same list: the reader's own interleaved order is what comes back, and the one document that was on
screen is `active` whichever kind it is — written out in full rather than as an index, since a tab
that no longer resolves is *dropped* (which would shift the index) while the active one *degrades*.
Each entry carries **the rows both of its sides were left at**: a `tabs` entry is a `SavedTab`
(`asm_row` + `src_row` + `line` + `asm_address` + `document`), rather than the list having arrays
of rows beside it. `asm_address` is where an object's **code** tab was left, as a placed address,
and is absent for every other kind: that listing's rows are counted afresh as it is decoded, so a
row there is no place to come back to and an address is (`agents/UI.md`, `CodeAt`). It is a claim
about a layout, so a rebuilt binary takes it with the rows; how many rows past the address the
tab was is not saved, a label being a fine place to come back to. The
rows travel with their tab because `resolve_tabs` drops the tabs that no longer resolve, which
would shift every later row of a parallel array onto the wrong tab. They are rows and not pixel
offsets so that the row height following the fonts (Step 9c) does not move every saved position,
and they are hints and not facts — `#[serde(default)]`, and clamped to what the tab holds *now* by
`Positions::row`. `line` is which line a **source-driven** tab's assembly side was driven from and
is absent for every other kind. It is what makes such a tab's `asm_row` mean anything: without it
the listing that row is a row of is not there to come back to. Nothing resolves it, being a number
and not a place, so a rebuilt binary takes the two rows with it and leaves the line, which is
simply asked again out of what is loaded now. `resolve_tabs` answers with a named `RestoredTab`
rather than a tuple, the rows and the line no longer surviving the same things. **Field order within these structs is load-bearing**: TOML emits plain values
before tables, so every field of `Project` being a plain value is what lets `binaries` sit beside
the name, `SavedTab`'s two rows must precede its `document`, and `SavedHistory::cursor` its
`entries`. Getting it wrong fails at *runtime*, not at compile time, and a round trip through real
TOML per struct is what holds it.

`Session::digests` is the digest each binary had when the session was saved, keyed by path — in
the *other* file from `binaries` and not a field beside them, because `binaries` is the list to *open* and a digest is
what to *believe* afterwards. A mismatch is not an error, a dialog or a refusal: `Rebuilt` collects
the paths whose digest no longer matches, and under one of those the **name is the identity and the
address is only a tie-breaker** (a symbol that merely moved resolves, where an unchanged file drops
it; a name that names two symbols and no longer names an address resolves to neither, since a stale
address is exactly what lands a reader on the wrong function), and the saved **row is dropped**,
being a claim about a listing this build no longer has. A path with *no* saved digest is a third
state, not a mismatch: it behaves as everything did before digests existed.

Coming back, the **active document degrades** (symbol -> its object -> nothing, since there is one
of it and the app must open somewhere) while **history entries are dropped** (a list of places the
reader cannot get back to is worse than a short list). A source-driven entry resolves against
nothing, so it neither degrades nor drops: a deleted file comes back as a tab over the pane's own
"Source file not found". `History::rebuilt` is the one walk both a restore
and a file-close go through, carrying the cursor to the last survivor at or before it.
`History::restored` also collapses duplicates and trims to the newest `MAX_ENTRIES` (200).

**When** a save happens is `Saves` in `project.rs`, a `static Mutex` rather than UI state because
two of the three things driving it sit outside the component tree. `record(details, binaries,
session)` is called on every state change and compares each against its baseline: a change to the
`binaries` writes **both files immediately**; a change to the user-given `details` — the name and
the directory — writes **`project.toml` alone**, since a rename lets go of no binary and so cannot
leave the two files disagreeing; a change to only the session marks it **pending** — a tab because
it is expressed against the binaries rather than the other way round, costs one click to remake,
and arrives on every navigation, `activate` opening one on the way to each change of document.
Nothing in `record` has to *say* which is which: which file a field lives in is what decides it,
and `Written` is how it says which half it decided. `flush()` writes the pending
session — on a 30s timer and from the window's close hook, which is the one exit hook freya 0.4
offers (`WindowConfig::with_on_close`, a `Send` callback that cannot read any `State`, which is
exactly why the policy is a static).

**Every baseline is the state the app boots into**, which is why two of them start empty and one
does not. The binaries and the session are restored *asynchronously* — the app boots holding
nothing and fills in when the parse lands — so seeding them from the loaded project would make the
first comparison see the still-empty boot state as a change and write an empty project over a good
one. `Saves::given` *is* seeded by `reopen`, because the name and directory are restored
*synchronously*, into the state the project view renders, before a single effect has run. Until 8e
that field was a value `Saves` **carried** across the calls rather than a baseline, for want of
anything on screen holding a name; the project view holds one now (`Proj`), so a rename arrives
through `record` like everything else and the special case is gone. `Saves::listed` is the one
piece of bookkeeping that grew out of it: it is what `project.toml` currently *says* the binaries
are, and a write that is not about the binaries writes that back rather than the app's own list —
otherwise a rename during the startup parse, or after a restore that opened none of them, would
forget a file through a change that had nothing to do with it.

**Which project is open is `Saves`' too**, and changing it at runtime is `switch(id)` or
`start_new()`: both `flush` the project being left while the policy still points at it, `remember`
the one being entered at the front of `recents.toml`, and re-point every baseline through
`Saves::opened` — empty, because the app is about to be emptied. Emptying it is the caller's half
and stays in `ui/project_view.rs`, the states being the UI's. `recent_projects()` is the list a view draws:
`recents.toml`'s order, each row described by reading *that project's own* `project.toml`, with an
id whose directory has gone dropped here — the list never prunes itself on load, and this is the
point of use where the repair is free.

Startup reopens the **last project** — `project::reopen`, the front of `recents.toml`, both halves
of it — and `use_restore_on_startup` knows nothing about where they came from, which is what keeps
a project picker out of it. The binaries stream in the way any other open does, so the sidebar
fills in behind them, but the **session waits for the whole load**: a tab, a selection or a history
entry is resolved against the objects by name, and resolving one against a half-filled list would
drop the tabs whose object had not landed yet. The strip is then restored, and **through the two
functions that hold the invariants** rather than by writing the list: `use_restore_on_startup`
sets the history, then `activate`s each tab and then the active one. Two orderings are
load-bearing. The **rows go into the two `Positions` maps, and the driven line into `Driven`,
before the tabs are opened** — those three maps are the one thing the restore writes directly, and
a pane puts its view back when it notices the tab it is showing has changed, so a row arriving
after the `activate` arrives after the only moment anything looks at it. And tabs before the active document, because `activate` opens what it
cannot find and would otherwise append it at the end of the strip instead of finding it in place
(the other direction is safe: a document that degraded to its object while the strip holds the
symbol simply opens a tab). An assembly-driven tab that no longer resolves is **dropped**, like a
history entry; a source-driven one is never resolved at all, so a file that has been deleted comes
back as a tab over the pane's own "Source file not found" rather than silently vanishing.

**The settings are a file of their own, above the projects** (`src/settings.rs`, `settings.toml`
at the top of the state directory beside `recents.toml`, since a setting is the user's and not any
one project's; same atomic `.tmp` + rename, same "a missing, unreadable or corrupt file is simply
the default"). This was the first slice of the storage split `notes/Goals.md` asks for under
*Projects*, and the same cut runs through each project: what the app **noticed** changes on every
click, what the user **said** changes when they say so, so they have different rates, different
save policies and different consequences when one of them will not parse. `Settings` is the theme choice (`Theme`: light, dark or follow the desktop) and a
`FontSetting` — a family and a size — for each of the interface and fixed-width fonts. **Every
field is an `Option` and `None` is a real third state**: "the user has not said, ask the desktop",
which is neither an empty string nor the desktop's current answer copied into the file. An
unspecified field is therefore a key that is *absent* from the TOML (`skip_serializing_if`, since
TOML has no null anyway), so nothing can later mistake an inherited value for a chosen one, and the
settings page can show the difference. Sizes are stored in **points**, the unit the desktops answer
in, so an override and the value it overrides are comparable; `fonts.rs` converts once at the end.
Field order is load-bearing here too — `theme` is a plain value and the two fonts are tables — and
the round-trip test is what holds it. There is **no `Saves`-shaped policy and deliberately no second
autosave timer**: a settings change is already as rare as a deliberate action, so `Settings::save`
is public and writes at once. **Resolving `Theme::Desktop` is deliberately not this module's job**:
"which theme does the desktop prefer" is a question for whatever owns the window, so `settings.rs`
holds only the choice and stays framework-free, and `ui/palette.rs` puts the two together
(`resolve_appearance`). It once spawned a subprocess per platform to answer it and no longer does —
the windowing system already knows, it answers on every platform this runs on, and its answer is
live rather than a value baked in at startup. `fonts::resolve` merges the settings over the
desktop's answer **field by field**, pure and tested, and `fonts::inherited` is that same merge of
*nothing*: what an unspecified field is falling through to, which is what the settings page draws in
an empty box. Everything in `fonts.rs` is in **points** up to one conversion at `Font::size`, the
app's own defaults included (9pt and 10.5pt, which are the 12 and 14 logical pixels the floem
version drew at), because an override and the value it overrides have to be the same kind of number
for the page to put them beside each other. The desktop's answer is cached per process
(`desktop_answer`), since the page re-resolves on every change and a lookup is a subprocess.


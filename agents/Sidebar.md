# The sidebar and the project view

The filtered lists, the Objects tree and its rows for files still being read, closing a binary,
the Files view over the project's directory, the Search panel over its text, the Project view and a
project switch.

**The sidebar lists filter themselves.** `FilterBar` is one component behind every box in the
sidebar, the Search panel's included. The `Filter` is a `use_state` in the owning tab rather than a
root context: a filter is a view of a list, never part of the session. `filter.rs` compiles every filter to one `regex::Regex`, plain
patterns included, because the three toggles *are* three regex constructs: a `RegexBuilder` flag for
case (so a pattern's own `(?i)` still wins for the part it covers), `\b(?:…)\b` for whole word (the
non-capturing group is load-bearing), and escaping on the way in for the third. That is also the
faster answer: 3 ms against `str::contains`'s 3.7 ms over 151k names. A pattern that does not
compile is `Matcher::Invalid`, a third answer that matches nothing *and* prints the reason, because
matching everything hides a half-typed `(`. The toggles call `prevent_default` on their press, or an
`Input` gives up its keyboard focus mid-word. Only the Symbols list needs a memo (`Filtered`,
holding indices, and `None` for the unfiltered case so it costs what it did before there was a
filter); Objects, History and Bookmarks filter where their rows are built. A History row draws the
shortened name (`entry_text`) and is filtered on the whole one (`entry_name`), so a generic argument
the row has no room for can still be searched for.

**A filtered list is ranked, an unfiltered one is not.** `Filtered` orders its indices by
`filter::Rank`, which sits beside the matcher because it is the same regex asked a second question:
where its first match *starts*. A match at the name's first character beats one at a word boundary
— regex's `\b`, which is the Word toggle's own notion, so the two agree that `::` bounds and `_`
does not — which beats one inside a word. Between two of a kind the shorter name wins, being the
one the pattern says most of, and the list's own order breaks the last tie, so the sort is total
and `sort_unstable` is safe. Ranking is `Regex::find` in place of `is_match`, one pass and not two,
and costs almost nothing: over the app's own 154k names `find` takes 10–18 ms against `is_match`'s
11–14 ms, since a name that does not match is scanned whole either way. Nothing typed is still
`None`: no pass, no sort, the list in its own order. The Locations panel builds the same memo and
so ranks the same way, which is wanted, since one line can answer with thousands of instantiations.

**Ctrl+F puts the caret in the box over the list it is pressed in.** The binding is on the rows of
`use_filter_pane` and not on the root, so it reaches the box of the list the reader is in and
nothing else: the Objects box from the Objects list, and no box at all from a code pane, which
keeps its own keys. `is_find_chord` is exact — Ctrl or Meta, and neither Shift nor Alt — which is
what leaves Ctrl+Shift+F to the Search panel. The rows are
focusable and a press on one focuses them, the code panes' own shape (`a11y_id`,
`a11y_focusable`, `on_pointer_down`): without it no list could hold the keyboard and the chord
would have nothing to fire from. The cost is that a press on a row takes the keyboard off the code
pane, so a copy there wants the pane clicked first.

The handler sits on the rows themselves and not over both halves of the pane because **a key event
is emitted only for a focused node that listens for it** — an ancestor's handler is reached by
bubbling afterwards, and a focused node with no handler of its own emits nothing to bubble
(`notes/upstream/freya.md`). In the box the chord does nothing, the box being where it leads, but
the bar still has to decline it: an `Input` inserts a character it has no chord of its own for, so
Ctrl+F would type an `f` into the pattern. The `on_pre_key_down` returns `false` for the chord,
before the edit, and otherwise repeats freya's default, which the hook replaces wholesale. The
Project, Settings and scratchpad boxes are left as they are: they filter no list, and five copies
of freya's default is not worth it. Two ids are minted in the pane, the rows' and the box's, the
pane being what holds them both. The headless tests pin the whole door — the chord ignored with
nothing focused, answered from a pressed row, and not typed into the box it reaches.

**Pressing an object opens all of its code** as one listing (`Document::Code`, `agents/UI.md`). That
is the one thing an object has to show that a symbol does not; the file's own facts belong to the
file-tab goal. **The Objects list is a tree** (`src/tree.rs`). `ObjectTree::new` groups objects by
*consecutive runs* of equal `Object::path` and flattens the result into `TreeRow`s. Keeping the run
is the writer's job: `take_load` puts each object after the last one of its own file and not at the
end, because two loads reading at once interleave their batches and the order is permanent -- a file
split over two runs would be drawn as two rows with the same name, each with its own fold and a part
of the count, for the rest of the session. The tree is a
shape in the data, never in the element tree, because a `VirtualScrollView` is told a length and
asked for row *n*. A file contributing exactly **one** object is its own row and gets no parent.
Filter and folds interact by one rule: a file row is never hidden while a row under it is visible.
So a file shows when its own name matches *or* a member's does, and those are different answers. A
file match keeps the reader's fold; a member match forces the row open (`Expansion::Forced`, a third
state, drawing no disclosure triangle), since a search that folds its results away has answered
nothing. Each row has a text tag (`ELF`/`PE`/`COFF`/`MACH`/`AR`) rather than an icon, because
nothing in Lucide's 1640 icons names an object format.

**A tree row is four columns and one of them is elastic.** The triangle and the format tag are fixed
widths every row keeps whether or not it has one, so the tags and the names line up down the list.
The triangle itself is `parts::chevron`, one glyph, one width and one colour for every list in the
app that folds.
An archive's member count is a column of its own, its digits and a `COUNT_GUTTER` beside them. The
name is the row's single `flex` child, which torin only works out under `Content::Flex`. The order
that follows is the whole of `Goals.md`'s "the count should survive a narrow sidebar": the fixed
columns and the count are measured whole and the name gets what is left, so a sidebar dragged narrow
takes from the name, which ellipsises, and never from the count. Without the flex the name takes the
remainder *before* the count is placed and the digits land past the row's edge, where its own
`Overflow::Clip` eats them. Without the gutter the name grows right up against them and the ellipsis
looks like part of the number. The headless test compares a 150px pane against a 300px one rather
than against an absolute width, because text is really shaped under the runner and a digit measures
whatever fonts the machine has.

**The triangle is an icon**, the Lucide chevron `disclosure` (`src/ui/parts.rs`) draws, which the
Files tree, the Search and Locations panels and the symbol bar all take from the same place. It was
`\u{25b8}`/`\u{25be}` in the interface font, so its shape, its size and its baseline were the
desktop's rather than the app's -- the last small mark here drawn as text. It follows the font
anyway, and so does its column: `chevron_size` is half a list row and `chevron_width` is that and a
pixel either side, which at the app's own interface font is the 14 the column was fixed at. A
`const` would be a mark a reader at 21pt has to hunt for, `field_label_width`'s reasoning. What an
icon cannot say is which one it is -- an `SvgViewer` rasterises to an image -- so open or shut is
carried as accessibility's own `expanded` on the column, which is both what a screen reader reads
and what the headless tests find a triangle by (`disclosures`, `src/ui/tests.rs`).

**A file being read is a row before it has an object**, which is `notes/specs/Sidebar.md`'s file
still being read. The state is on the **file**, not on an object, because an object that has not
been parsed does not exist: the unit part-way through is the one the reader opened, the one
`close_binary` closes, and the one that already has a row. `Loads` (in `tree.rs`, the state behind
the `Loading` context) is the files being read, one entry per (load, path). The tree draws a
`TreeRow::Pending` for each that has produced nothing yet and marks the ones that have `loading`.
Three rules come with it. A file still being read is **always** a file row even at one object, since
"one object is its own row" needs to know the one is all there will be, and a row that promoted
itself to a parent as the second member landed would move the list under a reader already reading
it. A row with nothing behind it is **a variant of its own**, not a `File` with its group, its count
and its expansion set to sentinels: it has no members, so there is nothing to fold and it draws no
triangle, and the path is the whole of its identity, which is what keys it. And it shows `…` rather
than a format tag, since a file's format is not known until it has been parsed; the name beside it
is dimmed to `address_fg`. Those are two static cues rather than a spinner, because a sidebar row is
one of hundreds and none of the others move.

**A project file's row offers one thing more**: "Open as project", appended in the same
handler as the reveal so the Objects rows -- which share `close_menu` -- keep the one item
they had. It is the one kind of file this view knows something about beyond "it is a file",
and it is still a file, so what any row offers is offered too.

**Adding a binary is a control over the Objects panel** (`AddBinaries`, `src/ui/sidebar.rs`),
which is what the top bar's Open button was. It moved because this is the list it adds to:
the bar is about the *project* -- which one is open, and what becomes of it -- and a binary is
one of the things a project holds. With no project there is no sidebar and so no control,
which is right: the way in from there is the menu's "Open a file as a project", and that
makes a project to put the binary in.

**A file row is also how a binary is closed.** Right-click opens a `ContextMenu` (which needs the
`ContextMenuViewer` mounted at the root of `app()`; opening one without it panics) with a single
"Close file". A member row has nothing: the unit that closes is the file. `close_binary` is composed
of rules from the modules that own them, `Selection::in_file`, `tabs::landing`,
`Docs::retain_entries` and `Visits::retaining`, plus one for a file that is still arriving
(`Loads::cancel`, without which the objects still coming out of the worker would put the file back
one member at a time). Three decisions inside it matter. A tab **showing** a place in the file
closes with it, and the selection follows the tabs rather than degrading (a file takes its objects
and their symbols together, so there is nothing to fall back to), while every other tab keeps its
slot and loses the places in the file off its trail, with the cursor carried. The record of visits
**drops** them the same way, both being one `Order` (`src/order.rs`), so the two cannot drift. And
the unit is the **path**, so one file opened twice closes once.

**The Bookmarks list is the reader's own** (`src/ui/bookmarks_view.rs` over `src/bookmarks.rs`;
`agents/Persistence.md` for where it is saved), where the History is the app's record of where they
went: a row is added on purpose and stays until it is removed on purpose. Three things follow. A row
is **live or dead by resolution**: the tab reads `Objects` and `Bookmarked` together and asks
`SavedDocument::resolve_by_name` for each entry where the rows are built, so a binary opening or
closing re-resolves every row. That is the whole mechanism: `close_binary` forgets nothing here,
since the list holds no `Arc` to forget, and reopening the binary brings a bookmark back without the
list having changed. A **dead row is drawn, dimmed, and does nothing**: `tree_name`'s dimming, the
loading-file-row idiom, with no press or hover handler at all, the way a history button with nowhere
to go has none. Dropping it would be the history's rule, and a reader's own list must not shrink
behind their back. And a live row's press is `open_document` like a Symbols row's, a `Preview` into
the temporal tab and a `NewTab` with Ctrl (`reach`, the one rule every row outside the panes reads,
in `ui/documents.rs` beside `Reach` and not with any one list), never `navigate`, since a bookmark
is a place and not a position on a trail. The row draws
`Bookmark::label` even when live, `short_name` of it for a symbol with the whole in the tooltip and
under the filter, as a History row does, so a row does not change its spelling when its binary
goes. That is the **stored name**, except for a symbol the app named rather than the file, which
stores no name and spells itself from what was saved (`agents/Persistence.md`).
Right-click offers **Remove bookmark**, by index rather than by place, because a dead row is exactly
the one that resolves to no place and the one it is most wanted on. **A bookmark is made wherever
the thing it is about is under the pointer**, through one `bookmark_item` (`src/ui/menus.rs`): a
right-click on a Symbols row or a History row (`bookmark_menu`, that item alone), on a document's
tab (`agents/UI.md`), or on an instruction row in either assembly listing (`agents/Panes.md`). It
says "Add bookmark" or "Remove bookmark" by `Bookmarks::matching` at the press, so a symbol that
moved under a rebuild still shows as bookmarked. On an instruction row it says "Bookmark symbol",
since the row is not the symbol and has to say what it would bookmark. The name a new one gets is
the whole `entry_name`, what the row's tooltip says. The rows *consume* `Bookmarked` and `Objects`
and peek them in the handler; a `read` would subscribe 115k symbol rows to every bookmark made.
Nothing on the symbol bar does this yet (`notes/Goals.md`).

**The Files view is the project's directory, read one level per unfold** (`src/ui/files_view.rs`
over `src/files.rs`). A project directory is arbitrarily large, so nothing walks it: `FileTree::new`
reads the root's own entries, and `toggle` reads one directory's when it is unfolded and forgets
them when it is folded. So a refold is a re-read, which is the whole refresh story and why there is
no file-watcher dependency. **The tree is the fold state**: a directory is unfolded exactly when its
children have been read, so there is no expansion set beside it to keep in step. The tree is a
`use_state` in the tab, a view of a list and never part of the session, rebuilt by an effect
whenever `Proj`'s directory string changes; a keystroke in the Project view's box costs one
`read_dir` of a half-typed path, which fails cheaply. The root is a row like any other, named after
the directory's last component, so refolding it is how the top level is refreshed. A root that
cannot be read is a placeholder's job to say, as is a project with no directory at all, which points
at the Project view and never puts the working directory in its place. The read is on the UI thread,
one `read_dir` of one level per fold, the `pads_in` precedent: nothing is *analysed*, and a listing
is what a file dialog does; a worker is the upgrade if a network mount ever makes a fold slow. Rows
are directories first and then files, each sorted by `walk::by_name`, the comparator the walk sorts
by too, and hidden entries are shown; `.git` and `target` fold away with one click. **A symlink is
not a row**, whatever it points at: the kind is the one `read_dir` hands back and is never followed,
which is what the walk does and what `source::showable` answers (`agents/Finding.md`), so a row here
is a row a press opens. There is no filter bar: a filter over a lazily read tree can only see what
is unfolded, and the search stories are the Symbols filter and `notes/Goals.md`'s source search.

**The Search panel is the Files panel's other half** (`src/ui/search_view.rs` over
`src/search.rs`): the Files view answers *what is here*, this one answers *where is that*. A panel
of its own and not a box on the Files view, because the two disagree about almost everything -- one
is a lazily read tree of every entry, the other a flat streamed answer that skips what git ignores
-- and because a filter bar narrows rows already in hand, where this asks a question and waits.

**Enter asks; typing does not.** A filter bar edits live because its list is already in memory; a
search reads every file under the project directory, so a pattern is asked for once it is finished.
The box is `FilterBar` with two more props -- a placeholder, and a `State<u64>` Enter bumps -- and
`use_search_pane` beside `use_filter_pane` over one builder, so the toggles are the same three. A **counter and not a callback**: freya's `Callback` is never equal to another, so a bar
holding one would re-render on every render of the panel, which for a streaming answer is every
batch. The pattern itself is `filter::Filter`, and the expression it compiles to is
`Filter::expression`, factored out of `Filter::matcher` so the two searches cannot disagree about
what a toggle means -- `grep-regex` has a `word` option of its own and it is deliberately looser
than `\b`. Each crate answers for its own half: the box shows `regex`'s error, which is what
`is_askable` asks for, and the search builds its pattern once with `grep-regex`, where nothing
typed and a pattern it refuses are both no search.

**The walk is ripgrep's** (`ignore`, `grep-searcher`, `grep-regex`), which is where the ignore
rules, the binary detection and the line-at-a-time reading come from rather than being written here.
Five decisions are the app's. `require_git(false)`, since a project directory is usually not a git
working tree and the crate's default would then walk `target/` whole. The sort puts a directory's
own files before the directories under it, which costs a `symlink_metadata` per comparison and buys
the one thing a reader watching a list grow needs: it only ever grows at its end. The name half is
`walk::by_name`, the Files view's comparator as well, and it allocates nothing: the two names are
lowercased a character at a time as they are compared, where each side used to be copied twice.
`max_filesize` is `source::MAX_SIZE`, so the search reads only what the source pane could show and
every hit can be opened. `follow_links(false)` is the crate's default written out, being the app's
rule and not the crate's: a symlink is not a project file anywhere (`agents/Finding.md`). And a
hit's line is decoded, **then** matched, **then** trimmed and cut, with the spans moved afterwards:
matching a trimmed line changes what `^` and `\b` answer, and match offsets taken from raw bytes are
wrong the moment a lossy decode replaces one.

**A search is a thread and a channel of its own, and cancelling one is letting the receiver go.**
`start_search` writes state and nothing else -- the id bumped, the hits emptied -- and an effect in
`use_search_with` is what starts the walk, `use_analysis_with`'s shape. The effect reads a **memo**
of the question and not the state itself: every hit is a write to that state, and an effect reading
it would start a fresh search for each batch of its own answer. Hits come back through
`take_hits`, which is `take_load`'s loop -- a batch per wake, since a write is a render -- and which
returns the moment the search is no longer the one being asked for. Returning drops the receiver,
the walk's next send fails, and it breaks where it stands. That one rule covers a second search, a
project switched away from (`clear_project` empties the state), and the app closing. Whether a batch
is this search's is `Searched::take`'s to say, and it says so **before taking any of it** and not at
the end of the loop, or the old walk's last batch lands under the new question. It is the one taker
that writes through the guard rather than through `write_if`: what is held is up to `MAX_HITS`
hits, and a clone per batch would copy every one of them to add the few that just arrived. `clear_project` **bumps** the id rather than setting it back to nothing: a walk of
the project being left is parked in its receiver and learns nothing until its next batch, so a
counter that restarted at zero would hand the new project the very numbers that walk still answers
to, and its hits -- files outside the new directory -- would land under the new question. `Loads`
keeps its `next` across a `clear` for the same reason. The work is an argument to the hook for the reason the analysis worker's is: a
walk that answers as fast as it is asked can say nothing about superseding.

**The references panel is the Locations panel, and one model holds both answers**
(`src/references.rs`, `src/grouped.rs`, `ui::locations`). A grouped list is items under the
file each is in with a fold per file, flattened into rows shared under one `Arc` so handing
them to a scroll view is a pointer compare -- because it is the same drawing problem. That
spares the handing over and not the building: the rows are made again from scratch every
time a search's hits grow, which is once a batch. So **each item is under an `Arc` of its
own from the moment it is pushed** and building an item row is a pointer bump. Copying a hit -- a path, up to
300 characters of the line and a vector of spans -- into every rebuild is work that squares
over one search, on the UI thread, for rows whose contents never change once pushed: over a
capped 10,000-hit search the rebuilding measured 70 ms against 19 ms. What
differs is only how a list is built and what an item is: a search appends as it walks, and a
server's answer, which lands whole, is grouped when it does -- files by path and references
by line, so a reader can find a file, where a search keeps the order its walk found them in
and only ever grows at the end. The lines are read off the disk there, since the server says
nothing about the text: each file once, on the language worker with the ask -- a read blocks,
and that is the thread that may block; a file that will not read leaves its references the
number they already have. The panels draw one row too (`ui::place_row`), down to the line's
text with the name marked in it (`search::drawn` cuts a long line for both, `found_line`
washes what matched in both): a list of line numbers says where a name is used and not how.
`Folding` is what is left of the difference -- which state a press on a file row writes its
fold to, whether a press on a place may refuse the path, and which panel's pick the row is
drawn against. The filter matches the file's path, applied where the rows are built rather than
through `Filtered`'s memo -- that is for the thousands a line's symbols can be, and a name's
references are tens.

**A place row opens through `open_source_place`** (`agents/Panes.md`), the arrival every door
into a place in a source file makes, so a hit opens exactly as a reference does, down to the tab's
assembly side being driven from the line it landed on. A hit's press is guarded by
`source::showable` in front of that call, so a row cannot open a tab the pane would refuse: that
path came off a walk of the directory, where the guard is the Files row's own. A reference's is
not, the other doors into that arrival taking a path a server or the debug info named, for which
opening the file and letting the pane say what is wrong with it is the honest answer. It lands
on the **match** and not just its line: a `Landing` carries the columns to select, and `line_pick`
makes them the row's `CharSelection` where a door naming no columns leaves a caret at column 0. So
Ctrl+C there copies the match, `copy_text` preferring characters to rows. The columns are the
*file's* line in UTF-16 units, counted before the line is trimmed for drawing and counted in units
rather than bytes, or a multi-byte character ahead of the match would move it. What matched is drawn
as a **wash behind it**, `match_bg`'s desaturated green -- the paragraph's own highlight, since a
span carries a colour and a weight and no fill of its own. It was a bold orange on the characters,
which made a match a thing of its own rather than a place in a line and cut the row's text into
three pieces to carry it; the wash leaves the text one piece and says the same thing over a
picked-out row as over a plain one. **A filtered list marks the same way**: `Matcher::marks` hands
back where the pattern is in a name, the panel compiles the filter once per render and shares it
with the rows it builds (`Marking`), and each row washes what matched in the name it draws -- which
is what says why a row is in a list that has been narrowed. The marks are byte ranges everywhere
outside the text engine and UTF-16 units inside it, `marked_units` being the one place the two meet.

**Ctrl+Shift+F is one more line in the key handler the root already has** (`root_key_down`), never a
second one: an element keeps one handler per event name, so a second `on_global_key_down` would
replace the first and take the modifier tracking -- and with it Ctrl-click and Shift-click -- away
silently. It is a *global* handler, so it answers from wherever the keyboard is, including nowhere;
what could still swallow it is the filter boxes' `prevent_default`, so they decline this chord as
they decline Ctrl+F. The chord cannot focus the box itself, a panel that is not on top being unmounted:
it raises the panel and leaves a flag the panel's effect spends once it has a node to focus, the
`Landing` pattern.

**A click opens a file as source; opening it as a binary is its menu, and the parser's call.** What
a file *is* is not judged here: not by extension (this project's own binaries have none) and not by
reading its head, which was tried and taken out because it made the view a second opinion about what
an object is, and the parser already has the one that counts. So a press opens anything the source
cache would read (`source::showable`: a regular file within `source::MAX_SIZE`, asked of the
metadata and never of the bytes), as `open_document` on a `Document::Source` spelled as **the
project directory joined with each entry's own name, never canonicalised**. That gate is not a
copy of the cache's rule but the call the cache itself makes before it reads, so it cannot fall
behind: a refusal added to the reader is one every press already obeys. Both of those rules
live in `open_source_file` (`ui/documents.rs`) and are written nowhere else: the finder's
Enter and its rows are the same door (`agents/Finding.md`).
`compiled_from` matches a file on the exact string `addr2line` renders, `DW_AT_comp_dir` joined
with the file entry, so a tree-opened file matches the debug info's, and
shares a tab with a companion-opened one, exactly when the project directory is the directory the
build ran in. Opening a binary is a deliberate act, so it is every file row's right-click, whose one
item is **Open file**: `open_binaries` on that path, the toolbar's call, where `object` decides
whether anything parses and a file that does not leaves nothing behind but a `…` row that goes when
the load ends. When the path is already loaded or loading the item is instead the Objects row's own
**Close file**, since opening a path twice puts a second copy of each of its objects in the list.
That item spawns with `spawn_forever` and not `spawn`, because a task belongs to the scope that
spawned it and the menu's button is unmounted by the press that chose it (`AGENTS.md`'s gotchas).
Under whichever of the two is drawn sits **Show in file manager**, the same `reveal_item` a
document's tab carries (`agents/UI.md`), on the row's own path: appended to the menu here rather
than built into either of them, so that the Objects rows, which share `close_menu`, keep the one
item they had. **A directory's menu is that item alone**: a folder is as showable as a file, and
there is no object inside one to open.

**The Project view** (`Tab::Project`) is what a project's `name` and `directory` are finally set
from. It is **one view and not two**, where `notes/Goals.md` asks for a project view and a
recent-projects view separately. They are one question, which project am I in and what else is
there: the recent list is how a reader *leaves* the project the rest of the pane describes, and a
tab of its own would be empty in every session where a project was reopened, which is all of them
after the first. The list leaves the open project *out*, since the pane above it is a better and
fresher description of that one than a row read off a file could be.

`OpenProject` is the value `Proj` holds, and its two editable fields are `String`s where `Details`
has `Option`s: they are what is in two text boxes, and a text box has no third state. An empty box
*is* how a reader says "I have not said". `OpenProject::details` is the one place the two spellings
meet, and it trims, so a box of spaces is a box of nothing rather than a project named `" "`. Each
box writes straight into `Proj`, so a keystroke is a state change the save observer sees like any
other and `record` writes `project.toml` at once. That is a few hundred atomically-written bytes per
keystroke of something typed once a project. The binaries it lists come from `Objects` through
`project::binaries`, which is what the saved list is *derived from*, so what the pane draws is what
the next write will say.

**The pane is five sections and not one render.** The project's own fields, the binaries, the
cargo build, the language server and the recent projects are each a component reading the contexts
it draws, so a change redraws the sections that read it and no others: an answer from the language
server leaves the artifact rows and the diagnostics standing, and a binary opening touches nothing
but the list of them. A keystroke still redraws the four sections that read `Proj`, the boxes
writing straight into it, but not the binaries -- and this is the pane the reader types into.

**The Project view is also where the project is built** (`src/ui/building.rs` over
`src/cargo.rs`), under a heading naming the tool rather than the act, since the pane has
several other things a reader could mean by "build". The manifest found is **named** in the
section: what cargo is run over is otherwise a rule the reader has to know. Four decisions.

**The state is a root context, not the tab's.** A tab that is not on screen is unmounted, so a build held
in `ProjectTab` would be lost the moment the reader looked at something else while it ran. `Builds`
is therefore provided at the root beside `Pad`, and it is in `ProjectStates`, because what one
project built says nothing about the next: a switch clears it, or the first build over there would
replace binaries opened over here.

**One worker thread, for the scratchpad's reason.** The work blocks, and it is one thread rather
than several so the project's directory has a single writer -- the debug-lines edit cannot land
inside the build that is reading the same manifest. Nothing supersedes: a build is asked for by a
press and takes seconds, and the two manifest jobs are cheap. Two builds cannot start at once, on
the button's `enabled` and in `start_build` both. The work function is handed in, so a headless test
drives the whole mechanism with no cargo on the machine.

**Artifacts are what cargo named, and the workspace's own are found by `manifest_path`.** cargo
reports a `compiler-artifact` for every crate in the graph -- 449 of them for this app's own
workspace, of which two are its own -- so the list is filtered to the artifacts whose manifest is
under the directory being built, matched by path component. cargo is handed a working directory
rather than a path, and derives every manifest path from its own `env::current_dir()`, so the
directory is first spelled the way cargo will spell it -- and the two platforms spell it
differently. On Unix that is `getcwd(2)`, which the kernel answers with symlinks resolved and `..`
gone, so only `fs::canonicalize` meets it: a `..` or a symlink in what the reader typed would
otherwise match nothing. On Windows it is `GetCurrentDirectoryW`, which keeps the plain prefix,
collapses `..` by spelling and leaves a junction alone -- `path::absolute` exactly.
`fs::canonicalize` answers verbatim there (`\\?\C:\work\app`), which `Path` reads as a different
prefix from cargo's plain one, and every build listed no artifact at all. Both sides are still
reduced from the verbatim form before the comparison, since a reader can type one into the project
box and `path::absolute` hands a verbatim path back as given. A target contributes its `executable`
where it has one and its `filenames` otherwise, which is what puts a library's `.rlib` in the list:
an archive this app opens like any other, and the most interesting thing a workspace produces for
it. The `.rmeta` beside it is dropped, the one place here a file is judged by its name, because
it holds no code and a row for it could only ever fail to parse.

**A build replaces the artifacts of the build before it, and nothing else.** A binary is a path, so
two generations of one file cannot both be in the objects list -- but narrowed: a file the reader
opened by hand is theirs even where a build has just written the same path. So the set replaced is
the *previous* build's list intersected with what is open, closed one by one and reopened in a
single `open_binaries` rather than a close and a spawn each. That list is saved with the session, which is what makes the
rule survive a restart (`agents/Persistence.md`). A finished build also forgets everything read of
the sources under the project's directory, which nothing else in the app ever re-reads
(`forget_source_under`, `agents/Panes.md`).

**Which of a build's places can be opened is worked out beside the build.** cargo spells a
file relative to where it ran, so a diagnostic's place is the project's directory joined
with it, and it is drawn as a target where that file is under the directory and the source
cache would read it; a place in a dependency stays a plain label, a target that did nothing
when pressed being worse than never offering one. Both questions are the worker's
(`openable`), asked once per distinct file and carried back beside the run as
`Builds::sources`. Asked at the row instead they were a `stat` per diagnostic per frame,
for as long as the section was on screen, and a build says two hundred things as readily as
two.

The **debug-lines offer** is why the profile and the manifest are read together. Release is the
default profile, since a reader inspecting a binary is usually asking what the optimiser did, and
cargo's own default for release is *no* debug information -- which is a binary with no source side,
the app's whole other half. So the view says so where the profile is chosen and offers to fix it,
writing `debug = "line-tables-only"` into that profile: exactly what the source side reads, and the
cheapest to build. The write is `toml_edit` and not `toml`, since it is the reader's own manifest
and a round trip through a value would take every comment and blank line with it.

**The manifest that is read and written is the workspace root's**, which is the project's own only
when the project is not a member of a workspace. cargo takes `[profile.*]` from the root alone and
ignores a member's own table, so asking the directory's manifest answers about a file the build pays
no attention to: a root that already asks for debug information goes unseen and the offer is made
for ever, and taking that offer writes a table cargo ignores while the view says the lines are
there. `profile_manifest` is that rule -- a manifest with a `[workspace]` table is a root, a package
may name its root outright, and failing both it is the nearest ancestor with one -- and it is a walk
up the directories rather than `cargo locate-project`, so it costs no process and is a unit test
over some files rather than over a toolchain. Whether the root's `members` really cover the
directory is not checked: cargo refuses to build a package its ancestor workspace does not claim, so
there is no build there to ask about. When that file is not the project's own, the view **names**
it, beside the manifest cargo is run over: the offer edits a file outside the project, and a write
the reader was not told about is the one thing it must not be.

**A project switch is a close and a restore, through the same functions.** `switch_project` is
`project::switch` (flush, re-point, remember), then `clear_project`, then `enter_project`.
`clear_project` is a `close_binary` per path and then a `close_tab` for whatever is left, never a
write to the list, so a project is left in a state the reader could have reached by hand. Its one
extra line is `Loads::clear`, which cannot go through the per-path walk: a file that has been asked
for and has produced nothing yet is not in the objects list for that walk to reach, and its objects
would otherwise arrive into the project that comes next. `enter_project` is the body the startup
restore was, extracted so the ways in cannot drift: it sets which project is open and its bookmarks
-- synchronously, before the save observer can see them -- and then calls `restore_project`. The
source-driven tabs go in that second walk, where
a closing *binary* deliberately leaves them standing: a file tab outlives the binary that led the
reader to it because the text stands on its own, but it does not outlive the project whose session
recorded that it was open. The ordering is what makes it safe: `project::switch` empties the
baselines *before* the app is emptied, and freya wakes an effect by a notify rather than at the
write, so the save observer runs once after the whole handler and sees a settled state that matches
the baseline exactly. `new_project` goes the same way, with a default project and an empty session:
there is nothing saved to put back, so the restore does nothing, and the way in is still the one way
in.

**What the Project view starts, it starts with `spawn_forever`.** A task belongs to the scope that
spawned it, and this view is drawn only while its tab is the one on screen, so a `spawn` here is
dropped by the next raise. `restore_project` is the worse case: on a switch the handler runs under
the scope of the pressed row, and that row is gone by the next render, the recent list leaving the
open project out; the runner renders before it polls a new task, so a `spawn` there was dropped
before it read a byte and the project came up with no binaries and no tabs -- which the save
observer then wrote into its session. The startup restore never saw it: its scope is the root's.
An **artifact row**'s load goes the same way, and what it leaves is worse than nothing: the load is
registered before a byte is read, so a row stays in the Objects list loading for ever, and only
closing the file clears one. So does the **folder dialog** behind "Choose...": through the xdg
portal it is not modal to the window, the app keeps taking input while it is up, and a reader who
raised another tab meanwhile got the directory they had before and nothing to say why.

**Every list row is one frame.** `list_row` (`src/ui/parts.rs`) is the chrome the ten sidebar-style
rows open with: the height the scroll view over them uses as its `item_size`, the padding and the
spacing their columns are laid on, and the three-way background -- picked out, under the pointer, or
nothing. A row appends its own press, its menu and its children, and keeps its own hover state,
since a hook may not run in a plain function and there is no `.hover()` pseudo-state to read
instead. The ten copies had drifted into two paddings, two spacings and two hover colours, and
folding them together settled each. The two washes looked like one per pane ground -- one over
`pane_bg`, another over the cream four of the panels sat on -- but the Locations panel drew its
reference rows in one and its location rows in the other on the same ground, which is what makes it
drift and not a rule. So one wash lights a row wherever it is drawn. The grounds went next: **every
panel is on `pane_bg`** and the cream is out of the palette, and no caller names a surface any more
-- `use_filter_pane` and `use_search_pane` took one as an argument, which is what let the three tabs
of one sidebar disagree. `dead_list_row` is the same frame with no hover, for the bookmark whose
place does not resolve.

**What a list lights is its own pick.** A lit row used to be one fact -- the row *is* what the tab
on screen shows -- so four lists lit one row each, four lit none, and there was no way to point at a
row without opening it. **Alt+press picks a row out and opens nothing**, the same Alt that says a
press on a link in a code row is not a door, and that needs a selection the list owns: nothing about
the tabs has moved for one to be derived from. `src/ui/picks.rs` is the whole of it. A list draws
its own pick where it has one and the row the tab shows where it has not; a press in the list is the
only thing that moves that list's pick, so a tab switched to or a link followed leaves it where the
reader put it. The cost is stated rather than hidden: a list that has been pressed in no longer
follows the tabs. A `Pick` is one enum over what the eight lists each call a row -- an object, a
symbol, a visit, a bookmark, a path, a place in a file -- compared by `Arc` pointer identity where
there is an `Arc` behind it, and the table is one per `Panel`, held at the root because a panel that
is not its dock tab's is unmounted. **The keyboard picks the colour**: the list holding it draws its
pick in `text_select_bg`, the code panes' own selection, and every other list draws its in
`selected_bg`'s grey, so what the next key would act on is the one thing in blue. What answers for a
list is the focusable rows box its pane already mints for Ctrl+F, handed down as `RowsBox`; the
Files tree, which has no filter bar to mint one, grew one of its own. The hover is a grey too, and
the palette test holds it fainter than either -- the pointer passing over a row must not read as the
reader having chosen it. The file finder is not a panel and keeps its own pick, its keyboard row
being one already: an Alt+press there moves the arrows' row and leaves the panel up. **The pick is
the list's cursor**, which is what makes a list something the keyboard can be used in at all: Up and
Down move it, the list scrolling to keep it in view, and Enter opens it the way pressing its row
would. What each list answers with is a `ListKeys` -- how many rows, which row is at a place, and
what pressing the row at a place does -- built by the panel, since the panel is the only thing that
knows its rows, and answered once on the box they are drawn in. Two closures and not a `Vec<Pick>`:
the Symbols list is 115k rows. The place a pick was made at is remembered with it for the same
reason, finding one again being a walk of the whole list per keystroke; a place goes stale when the
list moves under it, which the keys notice and start again from the end the arrow came from.
**Opening a tab hands it the keyboard**, however the row was opened: a reader who has put a listing
on screen is reading it, so the next key is answered there and not in the list behind it, and the
pick left behind goes grey saying so. Which is why acting on a row answers with a `Pressed`: a row
that only folded a group opened nothing and keeps the keyboard, as does a press with Alt held, which
opens nothing at all -- and those are what leave a list holding the keyboard for its arrows and its
Enter to be used in.

**Tooltips** are how a cut row is read, so `cut_tooltip` mounts nothing where the name fitted: a
tooltip repeating what is already on screen is noise the pointer drags down the list. What decides
it is the row's own answer to whether it fit -- `Fitted`, which the name paragraph writes and the
row reads as it renders (`ui/parts.rs`). It is instant, `CUT_TOOLTIP_DELAY`; freya's 500ms makes
sweeping down a list useless. `extra_tooltip` is the other kind, always there and keeping that half
second: a file row's path where the row draws its name, the filter toggles' words for `\b`. The
code rows have neither.

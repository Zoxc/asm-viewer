# The sidebar and the project view

The four filtered lists, the Objects tree and its rows for files still being read, closing a binary,
the Files view over the project's directory, the Project view and a project switch.

**The four sidebar lists filter themselves.** `FilterBar` is one component with four uses. The
`Filter` is a `use_state` in the owning tab rather than a root context: a filter is a view of a
list, never part of the session. `filter.rs` compiles every filter to one `regex::Regex`, plain
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

**Pressing an object opens all of its code** as one listing (`Document::Code`, `agents/UI.md`). That
is the one thing an object has to show that a symbol does not; the file's own facts belong to the
file-tab goal. **The Objects list is a tree** (`src/tree.rs`). `ObjectTree::new` groups objects by
*consecutive runs* of equal `Object::path` and flattens the result into `TreeRow`s. The tree is a
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

**A file being read is a row before it has an object**, which is `notes/specs/Sidebar.md`'s file
still being read. The state is on the **file**, not on an object, because an object that has not
been parsed does not exist: the unit part-way through is the one the reader opened, the one
`close_binary` closes, and the one that already has a row. `Loads` (in `tree.rs`, the state behind
the `Loading` context) is the files being read, one entry per (load, path). The tree draws a
`TreeRow::File` for each that has produced nothing yet and marks the ones that have `loading`. Three
rules come with it. A file still being read is **always** a file row even at one object, since "one
object is its own row" needs to know the one is all there will be, and a row that promoted itself to
a parent as the second member landed would move the list under a reader already reading it. A row
with nothing behind it has **no group** (`group: Option<usize>`, the group being the first object's
pointer), which is exactly the row that can never be folded, so it draws no triangle. And it shows
`…` rather than a format tag, since a file's format is not known until it has been parsed; the name
beside it is dimmed to `address_fg`. Those are two static cues rather than a spinner, because a
sidebar row is one of hundreds and none of the others move.

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
**drops** them through the same walk a restore uses, so the two cannot drift. And the unit is the
**path**, so one file opened twice closes once.

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
the temporal tab and a `NewTab` with Ctrl (`reach`, the one rule every sidebar row reads), never
`navigate`, since a bookmark is a place and not a position on a trail. The row draws the **stored
name** even when live, `short_name` of it for a symbol with the whole in the tooltip and under the
filter, as a History row does, so a row does not change its spelling when its binary goes.
Right-click offers **Remove bookmark**, by index rather than by place, because a dead row is exactly
the one that resolves to no place and the one it is most wanted on. **A bookmark is made wherever
the thing it is about is under the pointer**, through one `bookmark_item` (`bookmarks_view.rs`): a
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
are directories first and then files, each sorted by name without regard to case, and hidden entries
are shown; `.git` and `target` fold away with one click. There is no filter bar: a filter over a
lazily read tree can only see what is unfolded, and the search stories are the Symbols filter and
`notes/Goals.md`'s source search.

**A click opens a file as source; opening it as a binary is its menu, and the parser's call.** What
a file *is* is not judged here: not by extension (this project's own binaries have none) and not by
reading its head, which was tried and taken out because it made the view a second opinion about what
an object is, and the parser already has the one that counts. So a press opens anything the source
cache would read (`files::shows_as_source`: a regular file within `source::MAX_SIZE`, asked of the
metadata, the one bound so a press cannot open a tab the pane would refuse), as `open_document` on a
`Document::Source` spelled as **the project directory joined with each entry's own name, never
canonicalised**. `compiled_from` matches a file on the exact string `addr2line` renders,
`DW_AT_comp_dir` joined with the file entry, so a tree-opened file matches the debug info's, and
shares a tab with a companion-opened one, exactly when the project directory is the directory the
build ran in. Opening a binary is a deliberate act, so it is every file row's right-click, whose one
item is **Open file**: `open_binaries` on that path, the toolbar's call, where `object` decides
whether anything parses and a file that does not leaves nothing behind but a `…` row that goes when
the load ends. When the path is already loaded or loading the item is instead the Objects row's own
**Close file**, since opening a path twice puts a second copy of each of its objects in the list.
That item spawns with `spawn_forever` and not `spawn`, because a task belongs to the scope that
spawned it and the menu's button is unmounted by the press that chose it (`AGENTS.md`'s gotchas). A
directory has no menu.

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

**A project switch is a close and a restore, through the same functions.** `switch_project` is
`project::switch` (flush, re-point, remember), then `clear_project`, then `restore_project`.
`clear_project` is a `close_binary` per path and then a `close_tab` for whatever is left, never a
write to the list, so a project is left in a state the reader could have reached by hand. Its one
extra line is `Loads::clear`, which cannot go through the per-path walk: a file that has been asked
for and has produced nothing yet is not in the objects list for that walk to reach, and its objects
would otherwise arrive into the project that comes next. `restore_project` is the body the startup
restore was, extracted so the two cannot drift. The source-driven tabs go in that second walk, where
a closing *binary* deliberately leaves them standing: a file tab outlives the binary that led the
reader to it because the text stands on its own, but it does not outlive the project whose session
recorded that it was open. The ordering is what makes it safe: `project::switch` empties the
baselines *before* the app is emptied, and freya wakes an effect by a notify rather than at the
write, so the save observer runs once after the whole handler and sees a settled state that matches
the baseline exactly. `new_project` is the same thing with nothing to restore.

**Tooltips** are how a truncated row is read, so `row_tooltip` sets the delay to `Duration::ZERO`;
freya's 500ms default makes sweeping down a list useless. The filter toggles keep the default (their
tooltip explains what `\b` means), and the code rows have none.

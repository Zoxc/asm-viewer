# The sidebar and the project view

The three filtered lists, the Objects tree and its rows for files still being read, closing a
binary, the Project view and a project switch.

**The three sidebar lists filter themselves.** `FilterBar` is one component with three uses, and
the `Filter` is a `use_state` in the owning tab rather than a root context — a filter is a view of a
list, never part of the session. `filter.rs` compiles every filter to one `regex::Regex`, plain
patterns included, because the three toggles *are* three regex constructs: a `RegexBuilder` flag
for case (so a pattern's own `(?i)` still wins for the part it covers), `\b(?:…)\b` for whole word
(the non-capturing group is load-bearing), and escaping on the way in for the third. That is also
the faster answer — 3 ms against `str::contains`'s 3.7 ms over 151k names. An uncompilable pattern
is `Matcher::Invalid`, a third answer that matches nothing *and* prints the reason, because
matching everything hides a half-typed `(`. The toggles call `prevent_default` on their press, or
an `Input` gives up its keyboard focus mid-word. Only the Symbols list needs a memo (`Filtered`,
holding indices, and `None` for the unfiltered case so it costs what it did before there was a
filter); Objects and History filter where their rows are built. A History row draws the shortened
name (`entry_text`) and is filtered on the whole one (`entry_name`), so a generic argument the row
has no room for is still something a reader can search for.

**Pressing an object opens all of its code** as one listing (`Document::Code`, `agents/UI.md`),
which is the one thing an object has to show that a symbol does not; the file's own facts are the
file-tab goal's. **The Objects list is a tree** (`src/tree.rs`). `ObjectTree::new` groups objects by *consecutive
runs* of equal `Object::path` and flattens the result into `TreeRow`s — the tree is a shape in the
data, never in the element tree, because a `VirtualScrollView` is told a length and asked for row
*n*. A file contributing exactly **one** object is its own row and grows no parent. Filter and
folds interact by one rule: a file row is never hidden while a row under it is visible, so a file
shows when its own name matches *or* a member's does — and those are different answers. File
matched keeps the reader's fold; members matched forces the row open (`Expansion::Forced`, a third
state, drawing no disclosure triangle) since a search that folds its results away has answered
nothing. Each row wears a text tag (`ELF`/`PE`/`COFF`/`MACH`/`AR`) rather than an icon, because
nothing in Lucide's 1640 icons names an object format.

**A tree row is four columns and one of them is elastic.** The triangle and the format tag are
fixed widths every row keeps whether or not it has one, so the tags and the names line up down
the list; an archive's member count is a column of its own, its digits and a `COUNT_GUTTER`
beside them; and the name is the row's single `flex` child, which torin only works out under
`Content::Flex`. The order that follows is the whole of `Goals.md`'s "the count should survive a
narrow sidebar": the fixed columns and the count are measured whole and the name is handed what
is left, so a sidebar dragged narrow is taken out of the name — which ellipsises — and never out
of the count. Without the flex the name takes the remainder *before* the count is placed and the
digits land past the row's edge, where its own `Overflow::Clip` eats them; without the gutter the
name grows right up against them and the ellipsis reads as part of the number. Both halves are
one headless test, which compares a 150px pane against a 300px one rather than against a width of
its own, text being really shaped under the runner and a digit therefore measuring whatever fonts
the machine has.

**A file being read is a row before it has an object**, which is `notes/Goals.md`'s "an indicator
for an object still being processed" — and the state is on the **file**, not on an object, because
an object that has not been parsed does not exist: the unit part-way through is the one the reader
opened, the one `close_binary` closes, and the one that already has a row. `Loads` (in `tree.rs`,
the state behind the `Loading` context) is the files being read, one entry per (load, path); the
tree draws a `TreeRow::File` for each that has produced nothing yet and marks the ones that have
`loading`. Three rules come with it. A file still being read is **always** a file row even at one
object, since "one object is its own row" needs to know the one is all there will be and a row that
promoted itself to a parent as the second member landed would move the list under a reader already
reading it. A row with nothing behind it has **no group** (`group: Option<usize>`, the group being
the first object's pointer), which is exactly the row that can never be folded, so it draws no
triangle. And it wears `…` rather than a format tag, since which format a file is is not known
until it has been parsed; the name is dimmed to `address_fg` beside it, two static cues rather than
a spinner, because a sidebar row is one of hundreds and none of the others move.

**A file row is also how a binary is closed** — right-click opens a `ContextMenu` (which needs the
`ContextMenuViewer` mounted at the root of `app()`; opening one without it panics) on a single
"Close file". A member row offers nothing: the unit that closes is the file. `close_binary` is
composed of three rules from the modules that own them — `Selection::in_file`, `Tabs::close_all`,
`History::retaining` — plus a fourth for a file that is still arriving (`Loads::cancel`, without
which the objects still coming out of the worker would put the file back one member at a time), and
three decisions inside it matter: the selection **follows the tabs**
rather than degrading (a file takes its objects and their symbols together, so there is nothing to
fall back to); the history **drops** through the same `History::rebuilt` walk a restore uses, so
the two cannot drift; and the unit is the **path**, so one file opened twice closes once.

**The Project view** (`Tab::Project`) is what a project's `name` and `directory` are finally set
from — two fields that round-tripped since 8d with nothing to write them. It is **one view and not
two**, where `notes/Goals.md` asks for a project view and a recent-projects view separately: they
are one question — which project am I in, and what else is there — the recent list is how a reader
*leaves* the project the rest of the pane describes, and a tab of its own would be empty in every
session where a project was reopened, which is all of them after the first. The goal's "if none was
open" case is the pane answering for itself. The list leaves the open project *out*, the pane above
it being a better and fresher description of that one than a row read off a file could be.

`OpenProject` is the value `Proj` holds, and its two editable fields are `String`s where
`Details` has `Option`s: this is what is in two text boxes, and a text box has no third state —
an empty box *is* how a reader says "I have not said". `OpenProject::details` is the one place the
two spellings meet, and it trims, so a box of spaces is a box of nothing rather than a project
named `" "`. Each box writes straight into `Proj`, so a keystroke is a state change the save
observer sees like any other and `record` writes `project.toml` at once — `Goals.md`'s "user
project changes save immediately" taken literally, at a few hundred atomically-written bytes per
keystroke of something typed once a project. The binaries it lists come from `Objects` through
`project::binaries`, which is what the saved list is *derived from*, so what the pane draws is what
the next write will say.

**A project switch is a close and a restore, through the same functions.** `switch_project`
is `project::switch` (flush, re-point, remember), then `clear_project`, then `restore_project` —
and `clear_project` is a `close_binary` per path and then a `close_tab` for whatever is left,
never a write to the list, so a project is left in a state the reader could have reached by hand.
Its one extra line is `Loads::clear`, which cannot go through the per-path walk: a file that has
been asked for and has produced nothing yet is not in the objects list for that walk to reach, and
its objects would otherwise arrive into the project that comes next. `restore_project`
is the body the startup restore was, extracted so the two cannot drift. The source-driven tabs go
in that second walk, where a closing *binary* deliberately leaves them standing: a file tab
outlives the binary that led the reader to it because the text stands on its own, but it does not
outlive the project whose session recorded that it was open. The ordering is what makes it safe — `project::switch` empties the
baselines *before* the app is emptied, and freya wakes an effect by a notify rather than at the
write, so the save observer runs once after the whole handler and sees a settled state that matches
the baseline exactly. `new_project` is the same thing with nothing to restore.

**Tooltips** are how a truncated row is read, so `row_tooltip` sets the delay to `Duration::ZERO` —
freya's 500ms default makes sweeping down a list useless. The filter toggles keep the default
(their tooltip explains what `\b` means), and the code rows have none.


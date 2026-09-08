# The file finder

Ctrl+P, the overlay it opens, the matcher behind the box, and the walk both readers of a
project's directory share.

**One walk, in `walk.rs`.** The `ignore` builder used to sit inside `search.rs`. It is out here
because a second reader of the same directory arrived: a file the Search panel finds a hit in but
the finder will not offer, or the other way round, is the app telling a reader two things about
one project. `require_git(false)`, the source pane's size bound, symlinks left unfollowed and the
order a directory's entries come back in are settled once, and so is which entries count as
files: `walk::files` is one iterator of them, which `search::search` reads and `walk::walk_files`
builds the finder's rows from. `Found` — the path, the path written from the project's directory
with `/` separators, and where the name starts in it — is built on the walking thread, because it is what every keystroke
is matched against and taking a path apart per file per character is work the match should not
be doing. It never leaves that side: the finder's worker holds the walk, and only the rows a
query picked out cross to the UI.

**A symlink is not a project file.** `source::fits` asks `symlink_metadata` and refuses one, the
walk does not follow one, and the Files view drops one from a level it reads (`agents/Sidebar.md`).
One rule in three places rather than three rules: a symlinked source file used to be invisible to
the finder and to Search and openable from a Files row, which is the app saying two things about
one file. The project's root is still resolved, so a project reached through a symlinked directory
is walked whole; it is the entries under it that are taken as they are found.

**Characters in order, not a regex.** `fuzzy.rs` is its own module and not a fourth toggle on
`filter.rs`: a filter bar asks whether a name *contains* a pattern and compiles to one
`regex::Regex`, and no regex a reader would type says "these characters, in this order, gaps
allowed". A path is placed **twice** and the better placement kept — once reading forward, each
character as early as it fits, and once walking back from the end of that first whole match, each
as late as it fits. Neither wins everywhere: reading forward keeps `ab`'s match on the first
character of a name where walking back would start it inside a word, and walking back pulls `ui`
together into the `src/ui/` it names where reading forward is already right but `sv` is not.
Walking back from the end of the *path* rather than from the first whole match is the version
that looks clever and is wrong: it takes `ui`'s `i` from `files_view`, four words past the
directory the reader was typing. `Score` compares in the order the spec ranks them — the file's
own name, then runs, then a word's start, then the shorter path — and is a plain `Ord` struct,
`filter::Rank`'s shape, so the order is in the field order and nowhere else. The pass back is
skipped where reading forward already scored the best a path can — in the name, one run, at a
word's start — because nothing can beat that and a tie keeps the first placement anyway.

**The box is prepared once, not once per path.** What was typed is a `Query`, its characters
lower-cased on the way in, so a keystroke folds them once rather than once per walked file; a
comparison then folds only the path's side. That answers what folding both sides answered
because a character a fold produced folds to itself — a fact about Unicode's tables, pinned in
`fuzzy/tests.rs` over every character there is. A character that folds to more than one (`İ`
folds to an `i` and a combining dot) is kept as the whole fold, so it asks for what it asked
for before.

**The walked files never reach the UI thread.** They are the worker's, and what crosses is the
rows it picked out for a query. Both of the things that used to be done with the list here were
paid for a frame at a time. Appending a batch of a walk to a shared `Arc` copies the whole list,
so the first Ctrl+P of a run copied tens of thousands of paths once per batch -- and a real walk
delivers in handfuls, not in the channel's whole 512, so it is thousands of copies of a growing
list, on the thread that draws. That is the freeze a reader met while moving the pointer over the
list, and why the second open was the better one. Matching the box against the list was the other:
one pass over every walked path per keystroke.

**One worker, told of the walk and of the box on one channel.** Two would mean a thread that
polls or a runtime to select on them, where one lets it block. Every message carries the walk it
belongs to and the worker drops the rest, `Searched`'s own rule. It drains what is waiting before
it answers, so a burst of a walk is one ranking and not one per file, and while a walk is still
streaming it answers at most every `WALK_REFRESH`: a rank of everything found so far is worth no
more per file than it is per tenth of a second. What stops a walk nobody is waiting for is an
`AtomicU64` the next open bumps, not a full channel -- the channel has to be unbounded, because
the UI sends the box into it and a UI thread parked in a send is the freeze this exists to
prevent.

**The list is kept between opens.** A walk of a project's directory costs the same every time and
answers almost the same thing, so a finder that walked afresh on each Ctrl+P would make a reader
wait for what it already knew. What keeps it is the worker, which lives as long as the app: an
open walks again behind what the worker already has, and a reader who types before that walk ends
is answered from it. Only the **first** walk, with nothing to show, goes into the list as it
finds; a later one accumulates and swaps at the end, or rows would move under a reader already
typing against them.

**The list lags the box, by an answer.** About 12 ms over 20,000 paths in a release build, which
is shorter than the gap between two keystrokes, but it is a lag and the rows say which query they
were picked out for. Two things follow. The panel says *No files match* only about a query that
has been answered; under one that has not, it draws the rows it has and, having none, only its
box. And Enter opens the row the panel **drew** -- the row the reader is looking at -- rather than
waiting for the answer to the box, which would drop the keystroke of a reader who typed and
pressed Enter in one movement.

**Not freya's `Popup`.** `RescuedPopup` gets its overlay layer, its press-outside and its
Escape from `Popup` for free. The finder cannot: `PopupBackground` `.center()`s its content down
the window and offers no way to pin it to the top, which is where an editor's quick-open is and
where a reader typing a path is looking. So the layout is hand-rolled, and Escape and the press
outside come with it. This is `DocumentMenuButton` giving up `ContextMenu` for the same kind of
reason.

Three things about that layout are load-bearing, and each of them was a finder nothing could be
clicked in:

- **Two rects, not one.** `PopupBackground`'s own shape: the press outside is taken by a rect of
  its own with nothing in it, and the panel sits in a second over it. Nest the panel inside the
  rect that takes the press and the press never arrives.
- **The layer and the global position go on the one rect over everything**, not on the two under
  it. On the children instead, nothing in the overlay takes a press at all — the rows included.
- **Nothing in the panel may be `expanded`.** The panel is as tall as what is in it, so a body
  that fills its parent makes the panel the height of the window; it then covers the rect that
  takes the press outside, and every press the finder answers goes into it. `placeholder` is
  `expanded`, which is why the finder draws its own lines instead.

**A row is one line drawn in two colours and washed in a third.** The name first and the directories
above it after it, which is not the order a path is written in: the name is what a reader looks for
down a list, and a column of names all starting `src/ui/` says nothing. What the query matched is
washed rather than recoloured (`match_bg`), so the marks have to be moved into the order the line is
*drawn* in -- the hits in the name are the line's own, and the hits in the directories are shifted
along by the name and the gap after it (`row_line`). The wash is the paragraph's own highlight, in
UTF-16 units, where everything upstream of it is bytes.

**The app behind it is not dimmed.** A reader choosing a file is reading the window under the
finder, so nothing there is taken away; the panel's shadow is the whole of what says the finder
is over it, which is why it is a soft blur and not a hairline.

**The selection remembers its query.** `Finder` holds the row the keyboard is on *and* what was in
the box when it was moved there, and the row is read by comparing them. The obvious version — an
effect that resets the row when the box changes — is wrong in a way only a headless test catches: a
deps effect runs a render late, so a Down pressed in the same pass as the typing is undone by the
reset arriving after it. Nothing here needs an effect at all once the row carries the query it
belongs to. The row is **clamped where it is moved**, not only where it is drawn: counting on past
the last row left it above the list, and the reader who held Down then spent an Up per overshoot
before the highlight moved at all. The clamp is `Listed::clamp`, written once: the panel drawing a
row, Enter opening one and an arrow moving one all have to land on the same row. What it clamps
against is the drawn list -- the key handler is handed the list the memo holds, peeked, so a press
reads what the panel is showing rather than working a list out for itself. That row is also the
finder's **pick**, in the sense every list in the app now has one (`agents/Sidebar.md`): an
Alt+press moves the keyboard to the row under the pointer and opens nothing, where a plain press
opens the file and closes the panel. It is drawn in the selection while the box holds the keyboard,
which it does from the moment the chord opens the finder, and in the grey a list not being typed in
draws its pick with -- so the finder needs no rule of its own for either, only the box handed down
as `RowsBox`.

**The list follows that row.** The panel is `FINDER_ROWS` tall and the arrows walk past it, so the
list is given a `ScrollController` and each move ends in `reveal_caret` -- the code panes' own
rule, which takes the row height because a list row and a code row are measured in different
fonts. Without it the highlight went under the panel's edge at the thirteenth press while Enter
went on opening the row it was on: a file the reader never saw named.

**The empty box is the UI's own.** What it lists is the source files visited most recently, which
is not the walk's answer at all -- there are as many of them as the reader has been places, and a
file opened before the walk finished is listed. It is worked out in the memo, on the branch that
lists it: reading the visits is what subscribes the memo to them, and a memo subscribed to them
while the box had text would be woken by every file the reader opens.

**What the list is drawn from is a memo of its own**, `Asking`, between `Finder` and the list. A
subscription is to a whole state and not to a field of one, and the row the keyboard is on lives
in `Finder` beside the box, so a memo reading that state was woken by every arrow press: while
the ranking was still here, a held Down ranked the walk at the keyboard's repeat rate and the
overlay froze. `Asking` carries only the four things the list depends on, so it does run per
press and hands back what it handed back before, and `set_if_modified` stops there.

**A file picked out of the finder opens in a tab that stays**, `NewTab` rather than the `Preview`
every sidebar row uses. A sidebar row is browsing -- walking a list to see what each one is, which
is what the preview tab is for -- but typing a path out and picking it off the list is choosing
that file, and the next row clicked would take a preview tab back. Ctrl says nothing here that a
plain press does not, so the finder no longer reads it. Otherwise it is the Files row's door
exactly, `open_source_file` (`ui/documents.rs`): the same guard on what the source pane would
refuse, and the same uncanonicalised spelling of the path.

**The chord is answered at the root**, in `root_key_down`, which stays the window's one
`on_global_key_down` — a second one would replace it and take the modifier tracking with it,
silently. Every text box has to **decline** Ctrl+P in its `on_pre_key_down`: the `_` arm there
calls `prevent_default`, which cancels the global key event beside it, so a box that does not
decline the chord both types a `p` and stops the finder opening. The decline is one call,
`box_keys` (`ui/chords.rs`). The finder's own box names Escape, the arrows and Enter in it too —
they belong to the panel's handler, not to the box.

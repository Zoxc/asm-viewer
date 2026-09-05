# The file finder

Ctrl+P, the overlay it opens, the matcher behind the box, and the walk both readers of a
project's directory share.

**One walk, in `walk.rs`.** The `ignore` builder used to sit inside `search.rs`. It is out here
because a second reader of the same directory arrived: a file the Search panel finds a hit in but
the finder will not offer, or the other way round, is the app telling a reader two things about
one project. `require_git(false)`, the source pane's size bound and the order a directory's
entries come back in are settled once, and `search::search` and `walk::walk_files` both take
them. `Found` — the path, the path written from the project's directory with `/` separators, and
where the name starts in it — is built on the walking thread, because it is what every keystroke
is matched against and taking a path apart per file per character is the one cost worth moving
off the UI thread here.

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
`filter::Rank`'s shape, so the order is in the field order and nowhere else.

**The list is kept between opens.** A walk of a project's directory costs the same every time and
answers almost the same thing, so a finder that walked afresh on each Ctrl+P would make a reader
wait for what it already knew. `Finder` lives at the root, not in the overlay, and the walk is
started from `app()` for the same reason: it has to go on after the overlay it was opened from is
closed, since the list it is filling is what the next open draws. An open shows what it has at
once and walks again behind it. Only the **first** walk, with nothing to show, streams into the
list as it goes; a later one accumulates and swaps at the end, or rows would move under a reader
already typing against them.

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

**The app behind it is not dimmed.** A reader choosing a file is reading the window under the
finder, so nothing there is taken away; the panel's shadow is the whole of what says the finder
is over it, which is why it is a soft blur and not a hairline.

**The selection remembers its query.** `Finder` holds the row the keyboard is on *and* what was
in the box when it was moved there, and the row is read by comparing them. The obvious version —
an effect that resets the row when the box changes — is wrong in a way only a headless test
catches: a deps effect runs a render late, so a Down pressed in the same pass as the typing is
undone by the reset arriving after it. Nothing here needs an effect at all once the row carries
the query it belongs to. The row is **clamped where it is moved**, not only where it is drawn:
counting on past the last row left it above the list, and the reader who held Down then spent an
Up per overshoot before the highlight moved at all. `moved` works the list out for itself, which
is the pass the memo already makes per keystroke made once more per press.

**The list follows that row.** The panel is `FINDER_ROWS` tall and the arrows walk past it, so the
list is given a `ScrollController` and each move ends in `reveal_caret` -- the code panes' own
rule, which takes the row height because a list row and a code row are measured in different
fonts. Without it the highlight went under the panel's edge at the thirteenth press while Enter
went on opening the row it was on: a file the reader never saw named.

**Ranking is on the UI thread**, in a memo over the typed text and the walked files, as a sidebar
list's own filter is. It is one pass over a string per file with no allocation for the paths that
do not match. If a directory ever turns up where that shows, the ranking moves onto the worker
beside the walk; nothing else would have to change, the memo being the only reader.

**The chord is answered at the root**, in `root_key_down`, which stays the window's one
`on_global_key_down` — a second one would replace it and take the modifier tracking with it,
silently. Every text box has to **decline** Ctrl+P in its `on_pre_key_down`: the `_` arm there
calls `prevent_default`, which cancels the global key event beside it, so a box that does not
decline the chord both types a `p` and stops the finder opening. The finder's own box declines
Escape, the arrows and Enter for the same reason — they belong to the panel's handler, not to
the box.

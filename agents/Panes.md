# The two panes of a document

The Source pane and the Assembly pane: which of the two is on the left, which file the source side
draws, who writes a source-driven tab's line, how each pane's picked-out run lights the other, how
a click from outside lands, the arrow gutter, and what a run copies.

**The side a tab is driven from is the left-hand pane.** An assembly-driven tab reads
assembly-then-source and a source-driven one source-then-assembly, so in both the leading pane is
the one the reader came here to read and the trailing one is what it resolves to. Only
`DocumentBody` knows this; neither pane is told which side it was put on, so the swap is the order
of two panels and nothing else, and everything the two share is keyed by which pane it is rather
than by where the pane sits. The split's one remembered width is the exception and is kept by place
on purpose (`agents/UI.md`): the handle stays where the reader left it across a switch of kind
rather than jumping across the window.

**The Source pane draws the active tab's source side**, and `source_side` is the one place either
pane decides which file that is — so the pane and the effect that drops its picked-out rows cannot
disagree about which listing is up. A **subject** is a source-driven tab's own file; a
**companion** is the file the drawn symbol was compiled from, which comes out of `SymbolLines`
inside `Studied` and not out of `Active`, because the analysis arrives from a worker thread and
anything reading the two separately sees them disagree for as long as the work takes. Only the
symbol's *own* file is drawn, never the rest of `LineInfo::files`, since a Rust function inlines
dozens -- with one exception: when the source pane's picked-out run is in another file the
listing's line info names, the companion is *that* file. A `Landing` (a click from outside both
panes) plants the run in the file the line is in, and a symbol whose prologue was inlined from
elsewhere would otherwise open on that elsewhere, the line asked for sitting in a file that is
not up and the reveal with nowhere to go. A run picked out inside the pane is in the file already
shown, and a click on an inlined instruction picks nothing out on this side, so neither changes
which file is up. **In an object's code the companion is the file of the pressed instruction**
and nothing before a press -- the listing draws no symbol of its own, and an instruction row's
run is a run of the file the row was compiled from (`Picked::file`) -- and the tab opens on that
row's line; the pane says "Click an instruction" until then. The other way round, a click on a
source row beside it owes the listing a scroll to the instruction compiled from that line, which
`use_kept_place`'s reveal pays out of whichever held stretch has one and leaves owed while none
does -- the stretch may not be decoded yet, and the answer that decodes it wakes the effect
again.

**A tab opens its source side on the symbol's own lines**, which is what selecting a symbol
asked to see: a function a hundred lines into its file was otherwise read from the top of the
file for as long as it took to scroll. `SymbolLines` carries the **line** the symbol opens at
beside the file it opens in, both taken from **one** line-info row — the row the first
instruction was compiled from, else the first row naming a file at all — so the line can never
be a line of some other file; both are worked out on the worker, beside the info they come from.
`opening_row` turns that into the row `use_kept_position` opens a tab it has never shown at,
backed off by the `CONTEXT_ROWS` a reveal keeps above the row it scrolls to. A row remembered
for the tab wins over it, so this is the *first* open and not every one, and everything with
nothing to say falls back to the top of the file as it always did: an object with no line info,
a prologue DWARF places on no line, a source-driven tab (whose subject is a **file** the reader
opened, and files open at the top), and a companion that is not the symbol's own file — the last
being a landing's doing, which comes with a reveal of its own.

A companion wears a **header naming its file**, which a subject does not: the strip already names
a subject, and nothing else in the window would name a companion now that the Source pane has no
strip of its own. Pressing that header opens the file as a source-driven tab, as pressing a source
file's row in the Files view does (`agents/Sidebar.md`); until the source search lands those are
the two doors into one. The **assembly** side of a
source-driven tab draws the symbol the tab's driven line was compiled into, which is an ordinary
`Analyzed::showing` like any other tab's; before a line has been clicked in it there is no
question, so it says so.

**The Source pane checks the file it opened against the checksum the debug info recorded**, where
it recorded one. A PDB carries a hash per source file (MD5 unless the producer was told
otherwise; `LineInfo::hash_of`, `SourceHash`), and `source::load` takes all three digests of a
file's bytes as it reads them (`SourceDigests`, once per file, cached with it) — so the pane
compares two arrays per render and says, in one row over the source rows, that *this file differs
from the one the binary was built from* when they disagree. The file is still shown, being the
best thing there is to show; what the row says is that its line numbers are the compiler's and not
necessarily this file's. The recorded hash is looked up **by the name the pane is showing** in the
drawn symbol's line info (`SymbolLines::hash_for`), for a subject and a companion alike, since that
is the one place a checksum comes from; no hash, or a match, says nothing, and DWARF as read
carries none yet (`notes/Goals.md`). Verify, not locate: a hash that disagrees does not send the
pane looking for another file with that name — that is the path-mapping goal's, and the hash is
what will pick among its candidates.

**The Assembly pane wears a bar naming what it is drawing**, in both spellings: the demangled name
over the mangled original, `src/ui/symbol_bar.rs`. Until it there was one place a symbol was named
-- its tab, where `short_name` has cut it to a `module::fn_name` and `elide` to forty characters --
and the mangled spelling appeared nowhere in the window at all. It names **the drawn symbol and
never the selected one**, worked out from the same `Analyzed::showing` the listing under it is
built from, which is the rule the pane already obeyed and the Info pane it replaces did not: the
analysis and the selection disagree for as long as the worker takes, and a bar naming a function
the rows below it are not of is worse than no bar. A tab that is a whole *object*, and a tab that is
an object's *code*, are the two selections no listing is ever worked out for -- `ask` answers
`None` for both -- so there the bar falls back to the document and names the object; everything
else gets no bar rather than an empty one. Because the four answers the pane can give are each a `return`, and a header cannot be drawn
above a return, they moved into `AssemblyPane::body` and the pane became a `Content::Flex` column,
the way the Source pane has been since its companion header: the bar takes its own height, the
listing takes the rest, and the listing's five pixels of inset went onto the listing, a header
running the full width of the pane.

Each name is **one line however long the name is** -- ellipsised, never wrapped. This repo's own
samples reach 1038 bytes mangled, which is fifteen wrapped lines at this pane's width, and a bar
tall enough for the worst name is a bar that is that tall for every other one. So the tooltip and
the clipboard are between them the whole of how a reader gets at the rest: **pressing a name
copies it**, failing silently the way the listing's own Ctrl+C does, since a platform whose display
handle gave freya-winit no clipboard has none and a header has nowhere to say so. The row lights
with `toggle_hover_bg`, the grey a chrome control takes under the pointer, and deliberately not the
relocation label's `link_hover_bg`: that one is a translucent white, and over the header's own grey
it moves the surface six levels and says nothing -- which is what the wash test now holds, beside a
contrast floor for `address_fg` on `header_bg`, the mangled row's own colour. The names sit in a
box of their own inside the `flex` child rather than being it, `tree_name`'s trick: a flex child is
measured from its content first, so a label placed there takes the width of its whole name and the
ellipsis never happens.

**The bar is the collapsed state of a section**, opened by a disclosure triangle in a column of its
own -- the Objects tree's idiom down to the two glyphs, a triangle being the toggle where a name is
a copy. What it opens is the rest of what is known: for a symbol its section, address, declared
size, extent and the object it came from; for an object the format, the symbol count and the path.
That is what the Info view answered, plus the address and the object, which were shown nowhere at
all. Each fact is a `field_row` cut to one line for the reason the names are.

**Open or shut is the tab's and not the pane's**, which is `Expanded` at the root: both panes are
mounted afresh for every document, so a `use_state` here would shut the section every time the
reader looked at another tab, and a setting that undoes itself reads as a bug.
It is keyed by **`DocId` alone and not by `Entry`**, unlike `AsmAt`, `SrcAt` and `Drives` beside
it, and that is what makes it free: a `DocId` is `Copy + Hash` and holds no `Arc<Object>`, where an
entry's document does and would have to be forgotten in all three of `close_tab`, `close_others`
and `close_binary` or a closed binary's bytes would be held for as long as the app ran. Ids are
never handed out twice, so an entry a closed tab left behind is four bytes of dead weight and can
never be taken for another tab's -- a reopened tab correctly comes up shut. It follows that the
section stays open or shut along the whole of a tab's trail, a fact about the tab and not about any
one place on it. It is not persisted: a view of a tab, like a filter. The pane takes its tab's id
as a prop from `DocumentBody`; a headless harness mounting one with no tab behind it hands it a
stray id (`DocId::stray`), under which nothing is ever kept.

**A click in a source-driven tab's own file is the only writer of `Driven` inside the panes.**
A click in a companion file picks the line out and nothing more, and a click in the *assembly*
pane never reaches that handler at all — which is what stops a listing from re-driving itself. Nothing else changes: the
active document does not, so nothing is pushed onto the tab's trail, the tab already being where
the reader is. A line is kept per tab *and* place -- an `Entry`, the key the positions use -- rather
than one for the window or one per file, so a file reached twice along one trail is driven from one
line and two tabs on one file from two; it is a `u32` and holds no `Arc<Object>`, so it survives
its binary being closed and the next ask simply answers out of what is left. A right-click on a source row is neither a selection nor a drive: it opens `locate_menu` --
the line's locations and, inside a function as the file's parse says, the function's instances
-- both answered in the Locations view (`agents/Worker.md`), whose rows are what choose.

The rows are the app's own (`SourceRow`, a `VirtualScrollView`), **not** freya's `CodeEditor`,
which paints a line background only for the cursor's row and keeps its scroll state private —
which is to say it cannot do the two things this pane exists to do, highlight the *set* of lines
an instruction maps to and be scrolled to one from the other pane. Neither objection survives a
pane the reader is typing in, so the Scratchpad's editor *is* that component (`agents/Scratchpad.md`). What
`freya-code-editor` does offer is its tree-sitter pipeline, public on its own: `SyntaxHighlighter` +
`SyntaxBlocks` + an `EditorSyntaxTheme` turn a `Rope` into one list of `(Color, TextNode)` spans per
line. The theme is the app's own (`Palette::syntax`), the grammars are ours — Rust, C and C++, and
the TOML and JSON a project directory is full of, which the Files view opens and which get no
function pass, a configuration file defining none — and an unknown
extension degrades to one plain span per line. A file is parsed when loaded and cached in a
`static` in `ui/highlight.rs` — parsing is stateful across lines, so it cannot be per row — and
parsed **twice**: `SyntaxHighlighter` keeps its tree private, and the function spans the source
row's menu needs (`src/functions.rs`, "the function this line is inside") are read off a second
parse with the same grammar for C and C++, and for Rust off a scanner of our own
(`functions/rust.rs`), the grammar losing whole files of the standard library to `const impl`
(`notes/upstream/tree-sitter-rust.md`); either way a few hundred bytes are kept against a
tree that would be most of the file again. Two things about
`SyntaxBlocks` bite: `get_line` unwraps rather than answering `None`, and it holds one block per
`Rope::len_lines()`, which counts a phantom line after a trailing newline (hence `Highlighted::lines`).

**The two panes point at each other through their picked-out runs**, and through nothing the
pointer does. `Marked` holds one run per pane (`Marks`, two `Picked`s, `ui/marks.rs`), and what
a pane draws in green is the *pair*: the rows of it that are the same place as the other pane's
run -- the instructions a picked-out line was compiled from, the line a picked-out instruction
came from, every one of them and not the first. A run is a `RowSelection` of listing rows plus
the file it is a run of, so the assembly side pairs a row by asking the row's own
`AsmData::position` against the run's file and lines, and the source side pairs a line by
turning the run's rows into positions -- `Studied::places` for a symbol's listing, `code_places`
over the held stretches for an object's code, through the rows the section view shares as
`CodeRows` -- and keeping the lines of its own file. `row_background` is four colours -- the
pair's green, the selection's blue-grey for a row picked out whole, that colour faded for the
caret's row, and a deeper green for a row that is both (`Wash`) -- and a run of
paired rows wears a rule a step deeper along its top and its bottom (`pair_border`, on the
rows at either end, which each list works out from its neighbours: `Edges::of`), so a block of
them is told from the pane where the wash alone is faint. Nothing else lights a row: no hover,
no pin.

**Every row of a listing is as wide as the pane or as the widest row the listing has drawn,
whichever is more** (`Widest`, `src/ui/width.rs`), which is what lets both panes scroll
sideways; the chrome every row shares is `code_row` (`src/ui/code_row.rs`), below. A `VirtualScrollView` already scrolls on x -- the wheel's `delta_x`, Shift and the
wheel, a horizontal scrollbar -- but only as far as the widest row it has *built*, and a row
filling the pane leaves it nothing. A row measured to its content would give it that and lose
the wash, which is the row's own background. So the rows follow freya's own `CodeEditor`, which
gives every line one width: one `State` per list holds the widest content any row has reported
under the listing's key, every row takes `max(pane, widest)` as its width through a `Size::Fn`,
and reports its own content -- `on_sized`'s `inner_sizes`, the children plus padding, never
the laid-out width, or narrowing the pane would leave a sideways scroll over nothing. Three
things are load-bearing. **It is a width and not a minimum**: torin sizes an auto-width node
from its minimum *plus* its children (`notes/upstream/freya.md`), so `width(auto)` under a
`min_width` of the pane came out the pane and the content again; the cost of the width is one
layout, a row wider than anything drawn being cut until it has reported. **The separator never
reports**: its rule fills the row, so it would report the row plus its gutter and the widest
would grow by a gutter every layout without end. **The key is the identity of what outlives
the rows** -- the disassembly, the highlighted file, the *object* for the section view, whose
`Built` is rebuilt every generation -- hashed with the mono font's size; a key that no longer
matches is a floor of nothing, which is the reset, made without an effect: the old listing's
rows drop to the pane's width on the new one's first render and report again. The extent is
therefore the widest row **drawn so far**, as the scratchpad's run output's is: a wide row
further down is reached once the reader has scrolled to it. The editor's own estimate -- the
most characters on a line times a `W` -- was not taken: it needs Skia on the UI thread, is
wrong for tabs and double-width glyphs, and an object's code cannot know its widest line
before the worker has decoded it. Four tests pin it, one per mechanism: the wheel moving a
wide instruction and its gutter together, the wheel moving a wide source line, a picked-out
row's wash reaching the widest row and no further than the pane where nothing overflows, and
a listing's extent not outliving it.
There was a pin, and a hover; both went in one step (`notes/Goals.md`), because the hover said
what the pointer already says and a pin was a second selection under another name, and one
selection for the window meant picking out an instruction lost the run on the source side. Three
things are load-bearing: **a position is a file and a line** (`LinePos`), since an inlined
header's line 42 is not line 42 of the open file — the one `Arc` in the UI compared by
*contents*; **a run is dropped only when its listing goes, or kept with its place** — the
assembly's when the same place asks another question (a click on a line of a source-driven tab)
or the rows are counted afresh, the source's when the pane moves *off the run's file* within one
place (`use_clear_marks`) and not whenever the file changes, since a landing plants a run in the
file the pane is about to show and the switch it causes must not be what drops it; a change of
the active *entry* is neither effect's to answer, being a switch of place, which `use_land` owns
whole (below); and **the scroll
is a request, answered once** — `Picked::owed` says which panes have yet to scroll to the run,
`owed_reveal` only *looks*, `reveal_made` is what clears a pane's flag, and `reveal_row` does
nothing when the row is already on screen. A click in one pane owes the other; a landing owes
both. The split is not tidiness: in a source-driven tab the click that picks a line out is the
click that asks for the listing, so the run it wakes is still holding the previous one, in which
no row matches — a single take would spend the request there and the listing that can answer it
would arrive to nothing owed. A request nothing matches stays owed until the next click replaces
it or the run is dropped with its listing. **And the ask is the run** for a source-driven tab:
`use_land` plants the driven line as the source pane's run whenever the tab arrives with none
kept for it, with nothing owed, or coming back to one would show a listing with nothing lit and
no reason given. None of this is a navigation: the selection does not change and nothing is pushed onto
any trail. `open_document` remains the only path for anything that does, and `navigate` the only
one that moves a cursor. **Both rows do the
left button and the right in one `pointer_down`**: freya's `on_secondary_down` is
`on_pointer_down` under another name and an element keeps one handler per event, so the row
that set both kept the menu and never started its run (`secondary`, `notes/upstream/freya.md`).

**A click from outside both panes owes both a scroll, and lands through the change of document
it makes.** A row in the Locations panel opens its symbol *and* picks the line out, so
`Picked::owed` is a pair of flags (`Owed`) rather than an `Option<Pane>`: a click in one pane
asks the other, a click in neither asks both, and each pane pays its own half -- the source pane
to its own run, the assembly pane to the pair. Opening is an `open_document` -- a `Preview` from
a row, as every sidebar row's is, or a `NewTab` with Ctrl held (`agents/UI.md` for the three
reaches) -- and the change of document that makes is exactly what `use_land` answers by giving the
arriving place its own runs -- so the row does not pick anything out; it leaves a `Landing`
(`Land`, at the root) naming the document, the line, and -- where the click was a door out of a
listing -- the instruction's address, and that effect turns the line into the source pane's run
when the document it names arrives, **over whatever the place had kept** (below): a click from
outside named a place, and the run it makes is the only run in either pane, or the assembly pane
would light its old run beside the pair of the new. Whichever document arrives spends it, the
one it named or another, since a landing left lying would pick a line out in a document opened
for some other reason later. The instruction is the half of a landing the change of document
cannot answer, and goes on as a `Planting` (the paragraph after the doors). A row
whose symbol is already on top picks the line out at once (`documents::land`), the opening then
changing nothing and no effect running. A row answering a question asked from a source-driven tab
chooses for that tab instead -- `Located::subject` carries the tab's id beside its file, and the
choice is written under that entry while the tab still shows the file -- and `land_on` raises the
tab with the line left as a landing, a move and not a visit, the tab being open already. **Two doors
join the two views** and both go through the same functions. A **Ctrl**-press on a label in an
object's code opens the symbol's own tab, a `NewTab` as Ctrl opens one everywhere; a plain press
picks the row out like any other, since a label is a row of the listing first, though a label's
row is a row of no file. Ctrl is watched at the root exactly as Shift is (`Ctrl` beside `Shift`,
both kept by `ModifierKeys`), a freya pointer event carrying no modifiers, and the label lights as
a link only while it is held. A Caps Lock the desktop has made into Ctrl names itself Caps Lock in
every event, so it is learnt from its first release (`ModifierKeys`' doc,
`notes/upstream/freya.md`). **The third door is the address an instruction goes to when
nothing names it** -- a call into the middle of a function, a call to a function a stripped
image has no symbol for, a jump out of the symbol in a listing with no row for it
(`Instruction::target`, `agents/Analysis.md`). The number is drawn as a `TargetLabel`
(`Link::Target`, the third of `split`'s links), inline in the row's paragraph as the other two
are, and a Ctrl-press on it is `show_in_code` with the **placed** address and no line: the
object's code in a tab of its own, landed on the row **at or below** the address, the view and
the caret both. That rounding is `section::Rows::row_for`'s and holds for every kind of row --
the instruction holding the byte, the row of bytes covering it, the guessed row of a stretch
nobody has decoded -- and `use_kept_place` finishes it: a move has arrived when the view's top
row is the row the place names, by row and not by spot, since a spot derived from the offset
never spells an address inside a row; and when the rows are rebuilt under a view that was at
the map's place as well as the old rows could tell, the map's place is re-applied and not the
derived one, so a target in a stretch the worker had not reached lands on its own instruction
once the stretch is decoded, not on the row its guess was nearest (`agents/UI.md`). A plain press on the
number is a press on the row's text, and the label lights as a link -- and the row shows the
hand over it (`Text::door`) -- only while Ctrl is held. Both doors into the object's code, this
one and the menu's, take `AsmData::placed`, the section's bias added to the row's own address:
the bias the *listing* draws is nothing in a symbol's own tab, and the code tab's rows are
placed. Undefined imports and relocations against a section symbol stay plain text; the crate
says why. **A relocation link and the companion header are clicks inside the
tab**, and are followed **in place**: pushed onto the tab's trail so the function left is one Back
away, the way a browser follows a link, and in a tab of their own beside it with Ctrl.
An instruction's menu offers to show it among its neighbours -- "Show in unified view", offered in
a symbol's listing and not in the code listing it would open -- which is `show_in_code`, a tab of
its own: `land` with the instruction's placed address and its line where it has one, and then the
code tab's place written to `CodeAt` under the entry the open handed back -- in the same handler
and before any render, so the pane's first run finds it, and after the open only because the
entry names the tab and a new tab has no id until it is opened; when the code tab is already on
top that write is what moves the view, the place-keeping hook reading the map for exactly this.
The address therefore travels twice, as the tab's place, which is where the view goes, and in the
landing, which is where the caret goes. The same menu in the code listing offers the door the
other way, "Open as symbol" (`open_as_symbol`): the symbol's own tab, the caret on the row's
instruction -- by the symbol's **own** address, the space that listing draws, where the code
tab's doors take the placed one -- and landed on the row's line where it has one, so a label is
not the only way back. Both listings' menus end
with "Bookmark symbol" -- the symbol the row is code of, which in the code listing is the stretch's
own -- through the same `bookmark_item` the sidebar rows and a tab's header use
(`agents/Sidebar.md`), worded for a row that is an instruction and not the symbol; with it the menu
always has something to offer, so a row opens one whether or not it has a line or a door.

**A door that names an instruction puts the caret on it, and does so when the listing is
drawn and not when the document arrives.** A line is a row of a file, which has the same rows
every time, so `use_land` plants it as the document arrives; an instruction is a row of a listing
that comes *after* the document -- a symbol's from the worker, an object's code's as the skeleton
comes and again as the stretch decodes -- and a caret planted before the rows exist would be
planted in nothing. So the address half of a `Landing` goes on as a `Planting` (`Plant`, at the
root) naming the document, left by `use_land` in the same run that plants the line -- never
before it, which is what makes the order safe: `use_land` resets both panes' runs as a place
arrives, and a caret planted ahead of that would be reset with them -- or by `land` itself for a
tab already on top, where no document changes and `use_land` never runs. Two listings spend it,
each reading the state so a door opened over the tab on top wakes it. **In an object's code**
`use_kept_place` plants it in the first run that has rows and finds a planting naming its
document, over the kept run: on the row **holding the byte** (`Rows::body_row_for`, `row_for`
past a stretch's header and labels, since the view is better shown the label over a function and
a caret is not), and with `Owed::default()` -- the tab's place is the same address, written by the
same door, and it is what scrolls the view there; a reveal beside it would cancel the place's
move (`use_kept_place` returns after a reveal) and put the row three rows down instead of at the
top. The place is authoritative in that pane, and the line's run, where the door left one, stops
owing this pane the pair for the same reason (`land_row`). The planted address is what is kept
for the caret's row (`Kept::spots`): a guessed row's own place is its share of an undecoded
stretch, and carrying the caret by that once the stretch decoded put it on the row nearest the
guess, one row off the instruction. So a place already kept for a row of the run **stays** for as
long as it still names the row on screen (`row_of`), the exact address a planting gave and a
derived one alike, and the carry across a recount goes through the kept place before the derived
one. **In a symbol's listing** `InstructionList`'s planting effect, keyed on the entry the drawn
answer is of -- not the tab's document, since the pane draws the listing being left until the
worker answers -- plants it on the row of the instruction at or below the address and owes the
pane the reveal, `Owed::by(Assembly)`, which `use_kept_position` pays first and over the kept row
as it pays any reveal; there the reveal is authoritative, a symbol's tab having no place by
address. Either listing spends the planting whether or not it could answer it -- an address in no
stretch, or before the first instruction, is dropped, not left owed -- and `use_land` drops it on
every arrival besides, so a listing that never came leaves no caret for a document opened later.
Four tests pin it: `show_in_unified_view_puts_the_caret_on_the_instruction_once_it_has_a_row`
(the wait for rows, the guessed row, the exact re-place), `open_as_symbol_puts_the_caret_on_the_instruction_once_the_listing_is_drawn`
(the wait for the answer, the pair's scroll handed over),
`the_code_opened_at_a_target_lands_on_the_row_at_or_below_it` (the door over the tab on top) and
`a_landings_instruction_is_spent_by_whichever_document_arrives`.

**Navigating brings back each pane's caret and selection with the place.** Back, Forward, a
switch of tab and a place a tab has been at before put back, in **both** panes, what the reader had
picked out there when the place was last shown -- the companion's run in an assembly-driven tab,
the listing's in a source-driven one -- the way `use_kept_position` puts the scroll rows back.
The runs are kept per tab and place, `MarksAt` at the root: a `Positions<Entry, Kept>` beside
`AsmAt`/`SrcAt`, `Positions` generalised to a `Clone` value for it, forgotten in the same three
closers for the same reason -- an `Entry` holds the `Arc<Object>` its document points into -- and
never saved: a run is a view of a tab. `use_land` is the whole of it, and is the one effect that
touches the marks on a change of the active entry: it holds the entry the runs on screen belong to
in an `Rc<RefCell>`, as `use_kept_position` holds its tab, saves `Marked` under that entry on the
way out -- **settled**, no gesture and nothing owed, and only while the entry is still on its
trail, since the run after a close is still holding the place that has gone and would put its
binary straight back -- and then gives the arriving place its own. Three rules settle what wins: a
pending `Landing` over what was kept, in both panes; a kept run over the driven line, being the
more specific, the driven line planted where the kept source run is none; and **a restored run
owes no scroll**, the kept rows being what put each side back, and a reveal beside them would fight
them. Writing on the way out and not on every change of `Marked` is deliberate: a sweep writes on
every pointer move, and the entry those writes belong to is a memo a beat behind them. What made
this subtle is that `use_clear_marks`'s two effects are woken by the same change of entry as
`use_land`, in an order nothing guarantees -- a drop made there *for the switch* could land after
the restore and take the restored run with it, which is what
`navigating_brings_back_each_panes_caret_and_selection` saw before each of them learnt to keep
the entry it last ran for and hand a change of entry off to `use_land` untouched. **An object's
code is the one listing whose rows are not its rows next time**: the reading is reset when the
tab is left (`use_reading_of`) and comes back as guesses, so a run kept by rows would land rows
away. Its assembly run is kept with the **place each of its rows stood for** (`Kept::spots`,
stamped with the reading generation), written by `use_kept_place` whenever the run or the rows
change -- never on the run that switches tab, when the marks on screen are still the last tab's --
and carried through them (`Kept::carry`) when the rows are built for the first time since the
reset, which is a pass after `use_land` has put the kept run back; until then the pane's run is
none, never a run of rows that are gone. `use_land` does that carry itself in the one case the
rows on screen are already the object's at another generation, a second tab on the same code,
where the section view rebuilds nothing. A restored run is never out of range for what is drawn:
a symbol's listing and a file have the same rows every time, a code run any row of which has no
place any more is dropped rather than guessed, and every reader of a run already answers a row
past the end with nothing. `a_run_in_an_objects_code_comes_back_by_the_places_its_rows_stood_for`
pins the carry, `a_landing_on_arrival_wins_over_the_kept_runs` the precedence and
`closing_a_tab_and_a_binary_forget_the_kept_runs` the forgetting.

**The arrow gutter** draws every branch staying inside the symbol, with the layout in `src/lanes.rs`
because a `VirtualScrollView` builds row *n* knowing nothing but *n* — a row has to be *told* which
lines pass through it. `Lanes::new` is called on the worker, inside `Studied::new` and beside the
disassembly it is derived from, so a lane layout can never arrive a beat after the rows it is drawn
over. Lanes are assigned **greedily,
shortest span first**, which makes nesting a consequence rather than a rule; two branches sharing
only a row still take two lanes, or a top half and a bottom half in one lane would read as a line
passing through; and the gutter is capped at `MAX_LANES` (5) with the outermost lane **shared**
past that, since the corner and the arrowhead survive sharing and only the joining line goes
ambiguous. It is drawn with **rects**, not `canvas()`, whose `RenderCallback` has a `PartialEq`
returning `true` unconditionally — exactly wrong for a row a scroll view recycles. `InstructionRow`
therefore pads horizontally only: a line must reach the row's top and bottom edges or the column
comes out dashed. Picking rows out draws their own branches darker (`branch_lit_fg`), which is
the pane's own run and not the pair: a source position is many rows. The run is **listing
rows** and not instruction indices, so that one state can serve a listing of many symbols; the
list converts it back through `Lanes::instructions_in` and asks `Lanes::touching_any` once for
the run, one pass over the edges rather than one per row, and a run that is one separator
lights nothing. In an object's code that is done per held stretch, each stretch's lanes
speaking its own instructions.

**Every stroke in it is put on the device pixel grid by its edges.** freya lays a window out in
logical pixels and multiplies the whole tree by the window's scale factor on the way to Skia,
rounding nothing afterwards, so a hairline placed by its *centre* comes out spread over two device
pixels and drawn as two grey ones — blurred beside the crisp text next to it. `Grid`
(`src/pixels.rs`) rounds the edges instead: a stroke is asked for by the line it runs along and
the ink it should have, and comes back as the run of whole device pixels nearest that, never
thinner than one. The scale factor reaches it through `pixel_grid()` in `ui/metrics.rs`, off
freya's own `Platform::scale_factor` — a root context the renderer writes and `freya-testing`
takes from `with_scale_factor` — so reading it subscribes the row the way asking for a colour or
a font does. What was actually wrong at 1× was the horizontal run and the cut at
`code_row_height() / 2.0`, whole numbers whenever the row height is even and so half-pixels once
the stroke was centred on them; the lanes' own columns already landed right, `lane_x` being half a
pixel off a multiple and the stroke half a pixel wide. At a fractional scale everything was. Two
things are deliberately left off the grid: the row's own top and bottom, which a lane's line must
reach exactly or the column comes out dashed, and the arrowhead's two diagonals, which no
placement can align — at 30° a line crosses into a new row of pixels wherever it is put — and
which are drawn **half a device pixel wider** instead, so the two rows the antialiasing spreads
them over stop reading lighter than the run they point along. Only their pivot is snapped, and it
is the run's own end. A corner's half-stroke now ends at the *far* edge of that run rather than on
its centre line, so the joint is filled to the pixel instead of stopping inside the run behind an
antialiased edge. All of it is relative to the gutter's own origin, which nothing inside a row can
see -- and **that origin is put on the grid by the list** (`Nudge`, `src/ui/code_row.rs`): the
box around a listing's rows learns where it was laid out from its own `on_sized` and pads its
top by the rest of the device pixel, so whatever fraction the bars, tabs and fonts above it add
up to, its rows start on a pixel edge and, their height being whole pixels, stay on one. What
that buys is the washes: two rows' backgrounds meeting on a fraction each fade into the other
over the pixel they share, and a translucent wash fading twice reads as a light seam between
every pair of selected rows. The scroll offset is whole logical pixels, so at 1× and 2× the
rows stay on the grid as they scroll; at 1.5× they do not, and nothing here pretends otherwise.
The caret is a stroke of the row's own on the same grid (`caret_x` off the laid-out paragraph,
`Grid::span` from the column rightward, two logical pixels wide as most editors draw theirs,
`caret_fg`), where the engine's own would sit on the glyph's fractional edge, and **so is the
highlight**: a rect from the first
column's x to the last's, the row's whole height, `Grid::span`, painted before the paragraph
in the tree so the text sits over it -- the engine's own highlight is the glyphs' tight box,
which is shorter than the row by whatever the line's fonts and the link's placeholder add to
it, and left a seam between one row's and the next's. Both are drawn from the render after the
paragraph's first layout, which is when the holder can say where a column is; an empty row
inside a run shows a stub a quarter of a row wide, or the run would read as broken there.
Both marks are `interactive(false)`, and **both slots are always there**, empty rects when
there is nothing to mark: freya matches siblings by position (`notes/upstream/freya.md`), so a
highlight appearing before the paragraph on the press would move the paragraph along one and
remount it, link and all, between the down and the up, and the press meant for the link would
never fire -- which `a_link_in_the_text_is_one_unit_and_still_opens_its_symbol` caught. The gutter is a child of the row, so a sideways scroll carries it with the addresses
-- by a whole number of pixels, the scroll offset being an `i32` -- and the strokes stay on the
grid. `the_gutter_puts_its_strokes_on_whole_device_pixels` pins the axis-aligned ones on a
26-pixel row, that being the even height the old placement was worst on. The rule a separator
row draws goes on the grid the same way and from the same answer — `Grid::stroke` over the
middle of a row — so a rule and a horizontal run crossing one row sit in the same device
pixels rather than half a pixel apart; `a_block_rule_lands_on_whole_device_pixels` measures the
second off the first rather than working it out twice.

**A row a branch lands on starts a block**, and the listing says so with a `SeparatorRow` above
it — a **row of its own**, not a border on the row below, so a block reads as separated from the
one before rather than as underlined by it. The set is the gutter's own — `RowLanes::arrow`,
worked out in `Lanes::new` beside the disassembly — and not `edges` asked a second time, so the
separator and the arrowhead below it cannot disagree. Never above the first row: a boundary over
the top of a symbol says nothing and would open the listing with a gap. Only the targets, too:
the row after a `ret` or an unconditional `jmp` also begins a block, but nothing below the
disassembler says which instructions end a fall-through, and that is crate work this did not
need. **Every separator is keyed**, by the address of the row it opens and in a key space of its
own (`(true, address)` against the instruction rows' `(false, address)`): freya matches siblings
by key alone, so separators left on the type's default key were one row to it, and a scroll of
a separator's distance -- one wheel notch is two or three rows -- put an instruction row's props
into a separator's scope and panicked inside freya (`notes/upstream/freya.md`;
`scrolling_past_a_separator_keeps_every_row_its_own`).

A `VirtualScrollView` is given one `item_size` for the whole listing, so the separator is
`code_row_height()` like every other row and the rule is drawn *inside* it, across its middle.
What that costs is **two index spaces**, and `Lanes` is the only thing allowed to convert between
them: `listing_rows`, `row_of` and `instruction_at`. An **instruction index** is what
`AsmData::position`, the gutter, `Lanes::touching` and the branch edges speak; a **listing row**
is what the scroll (`reveal_row`, `use_kept_position`) and the picked-out run (`Marked`,
`on_listing_key`) speak. `InstructionRow` carries both and never mixes them. A row is also told
three things about the listing it is in, through `AsmData`, so that the same row serves a
listing that is not one symbol's: `base`, the listing row the symbol's first instruction row is
drawn at, added to every row `Lanes` answers (a branch label's target is `base + row_of`);
`bias`, added to every address the row draws or copies (`Section::bias`,
what tells two functions of a relocatable object apart when both are at 0); and `width`, the
gutter's lane count, the symbol's own when it is read alone. On its own, a symbol's listing
hands in 0, 0 and its lanes' width, and nothing about it changed. The separator draws
the lanes that cross it — `Lanes::boundary`, the row below's `top` strokes run full height — so a
branch's line is unbroken where the listing opens the gap under it, and it carries neither stub
nor arrowhead, both of which belong to the row landed on. It takes the mark handlers too, so a
sweep down the listing is not cut in half at every boundary, and it copies as the blank line it
looks like. **It takes the instruction rows' own three pixels of horizontal padding**, which is
not cosmetic: without it every lane steps three pixels sideways at every block it crosses and each
branch line comes out kinked — a fault the model cannot show, since the `RowLanes` handed to the
rows were right the whole time, so `the_gutter_runs_straight_through_a_separator` asserts on the
laid-out strokes. The rule is a rect of its own rather than a border — a border is drawn on an edge of
the box it is given and the box here is a whole row — and it starts after the gutter rather than
crossing it: the gutter is a column of unbroken branch lines and a rule struck through them reads
as one of them breaking. It is placed by the grid and no longer centred by `cross_align`, which
was the whole of what put it on a fraction: half of an even row height is a whole number and a
one-pixel rect centred on one straddles the two pixels either side. The offset is a padding
rather than an absolute position, so the rule still takes the width the row's flex leaves it. Its colour is
`block_rule`, held quieter against the pane than `branch_fg` — it runs the width of the listing
where the gutter's stroke is a few pixels long (`agents/Appearance.md`).

**A listing of a whole object's code is rows before it is instructions** (`src/section.rs`).
A `VirtualScrollView` has to be told its length up front, and x86 being variable-length, the
instruction rows of a section cannot be counted without decoding it -- which is the one thing the
section view must not do eagerly. So `Rows` counts from the skeleton: a header row where a placed
section starts, a label row per symbol at each stretch's address, and under them either the rows
the stretch's decoded body takes -- the symbol's own instruction rows and separators, straight
out of its `Lanes`, then its gap as rows of sixteen bytes -- or, for a stretch nobody has decoded,
a **guess**: its bytes over four, x86's mean instruction length, and never fewer than one. The
listing therefore has its whole length from the first frame, the scrollbar means something, and
what the reader scrolls over is empty space that fills in as the worker reaches it; the length
starts estimated and settles. What makes that bearable is that **every row has an address and
every address a row** -- `address_of` and `row_for`, placed addresses both, an empty row's being
its share of the stretch's bytes rounded so the two agree -- since an address is the one name for
a row that survives the rows around it changing. The view keeps the reader's place as an address
for exactly that reason -- plus how many rows past `row_for` it was, since a stretch's header, its
labels and its first instruction all sit at one address and `row_for` answers the first of them. Nothing here is a fourth answer to the two index spaces above: a decoded
stretch's rows *are* its `Lanes`' rows, at `body_start` into the listing, which is the `base` an
`InstructionRow` is handed.

**The section view reads an object's code in windows and keeps its place by address**
(`src/ui/section_view.rs`). The rows above are drawn into one `VirtualScrollView`, the
instruction rows being `InstructionRow` told its `base`, `bias` and a gutter `MAX_LANES` wide,
the separators `SeparatorRow`, and the header, label, empty and gap rows four small rows of the
view's own -- all of them keyed in a key space per kind over the placed address they stand for,
the separators' lesson in six places. Two effects do the rest. `use_kept_place` keeps the reader's
place (`agents/UI.md`, `CodeAt`), plants a door's caret once there are rows to plant it in (the
planting paragraph above), and rebuilds the rows whenever the reading's generation changes,
in the one run that also moves the controller to where the place now is -- and what it produces
is the rows **and the reading they were counted from**, as one `Built`, because the effect runs a
pass after the answer and for that pass the reading the pane can read is newer than the rows on
screen: a stretch the answer let go of, drawn from the old rows against the new reading, found no
bytes, and every one of its rows fell back to one key, which freya's diff panics on. The list draws
from the pair and never from the two apart; a gap row is keyed by its own address besides. `use_window` reads the
controller, the viewport, the rows **and the reading** -- the pane mounts a beat before the
reading is its own, `Active` being a memo, and a run that found the reading about something
else must be woken when it catches up, or the tab stays empty until the pane is resized -- and asks -- through `Window`, which the worker's sender
reads -- for the stretches within `BUFFER` (3) screens above and below the viewport that are not
held, nearest the middle of the viewport first and at most `WINDOW` (64) of them; the worker
answers a chunk, the rows change, the effect wakes on them and asks for the rest, so the buffer
fills from the viewport outwards and a page up or down lands on rows already decoded. Before
there is a skeleton it asks for that, with nothing decoded. What a row copies is what it draws:
`section .text` for a header, `<name>:` after its address for a label, the instruction's own line
for an instruction, and for a gap row a data directive -- `dq` for a row that divides into
quadwords, down to `db` for one that does not, the values little-endian as x86 reads them --
followed by the same bytes as characters between bars: a hex dump's shape, which is how a row of
data is told from a row of assembly, in its shape and not in a colour. Nothing for an empty row or
a separator.

**A branch's displacement is the other way to follow it**, drawn as a `BranchLabel` exactly where
a call's resolved target is drawn as a `RelocationLabel` — `Instruction::branch_span` says which
span to lift out, and the row is the same three children either way. Only where
`Assembly::edge_from` finds an edge, which is the set the gutter has an arrow for: a tail call
keeps its plain operand, having no row here to be pointed at. Pressing it is `reveal_row` on the
edge's target **and the run a press on that row would have made** — `mark_row`, the row landed on
alone, of the target's file (`position(edge.to)`), with the Source pane owed the scroll and the
Assembly pane not, since it has just been given one. It is still **not a navigation**: the
document does not change and nothing is pushed onto the trail, so a Back that undid reading
further down one function would be answering a question nobody asked. It is a selection, though,
and in both of a selection's senses: the row picked out, which holds for an object with no line
info at all, and the pair lit on the other side where the target has a line. Arriving at a target
and then having to click it to light it up made the reader say twice where they had gone, and
both panes would meanwhile be lit at the place the reader had just left. The press is still
stopped from bubbling — the row under it would otherwise keep the row the instruction being
*left* is in picked out, which is the one answer the click is not asking for. The listing's own
`ScrollController` and its measured height are handed down to each row for it: both are the
list's handles and neither changes while it lives.

**A run of rows can be picked out and copied** in both panes (press, sweep or shift-click, Ctrl+C;
Ctrl+A takes the listing, Escape drops it). The state is `Marked`, holding one
`Picked` **per pane** (`Marks`) — independent, each the other pane's pair and the scroll it owes —
and Ctrl+C copies the run of the pane whose box has the keyboard. The press is `pointer_down` (a
press event arrives only once the button is back up), in the same handler as the right button's
menu (`secondary`), and the sweep is `pointer_move` -- not `pointer_over`, which freya fires once
on entry whatever its doc string says (`notes/upstream/freya.md`). Shift is watched globally at the root, because a freya
pointer event carries no modifiers at all. The key handlers are on each pane's own focusable box
and deliberately not global, or a Ctrl+C meant for a filter box would come back as a page of
disassembly. Runs are dropped by `use_clear_marks` at the root, not by an effect inside each list —
`AsmData` carries an `Arc<Lanes>` rebuilt every render, so that effect would wipe the run the press
just started. What is copied is what the row draws: `asm_line` (address plus the instruction with
the target's name in its operand), and the rope's own line for source, tabs and all -- and, in an
object's code, each kind of row as it draws (`row_line`), a separator and an empty row as the blank
line they are. That listing's run **survives its rows being counted afresh under it**, though a run
is listing rows: the section view's own rebuild (`use_kept_place`, which produces the new `Built`
in the one run that moves the controller) carries it across through `carry_assembly`, each row of
it -- the rows' two ends and the caret's -- put through the address it stood for in the old rows
(`spot_at`) and back to a row of the new (`row_for`), the way the reader's place is kept across the
same recount; a run any end of which has no row any more goes. Across a *switch* the old rows
are gone with the reading, and the run comes back through the places kept for it instead
(`Kept::spots`, the paragraph on navigating above). It used to be dropped on every
answer that landed, and with up to 64 stretches asked for and 8 answered a chunk, answers kept
landing after the reader had clicked: the caret vanished and the companion file with it.

**A sweep along a row's text picks characters out**, beside the rows and not instead of them
(`src/chars.rs`; `Picked::chars`). Every row of the three listings is drawn by one `code_row`
(`src/ui/code_row.rs`) -- the shared width, wash and handlers, with what comes before the text
(the arrow gutter, the address, a line number) and the text itself handed in -- and the text is
**one `paragraph()`**, the relocation or branch link placed inside it as an inline child: freya
reserves a placeholder sized from the child and moves the child's layout node to it, so the link
keeps its hover, its cursor and its press, and to the text engine it is one unit of the row
(`Piece::Inline`), which copies as the whole name. The model is the app's own and framework-free:
a `Caret` is a row and a column in **UTF-16 units**, the unit skia answers a pointer in and takes
a highlight in, and a `Line` is the row's text in pieces so a column into what is drawn is a
column into what is copied -- `instruction_line`, `source_line` and `code_line` are built from the
same splits the rows draw from, and `asm_line` is the address plus `instruction_line`. freya
supplies exactly the two primitives a paragraph has anyway: the hit-test behind its
`ParagraphHolder` (`caret_col`, `word_at`, in `ui/marks.rs`; `None` before layout where freya's
own code would unwrap) and the highlight paint (`highlights`, `text_select_bg`,
`CursorMode::Expanded` so it fills the row). No `use_editable`, no rope of the listing: the
editor's model wants one rope and a line per row, and an object's code is estimated rows that are
counted afresh with every answer. **Gutter against text**: the arrow gutter, the address column,
the line number, a separator and an empty row are gutter, and a press there puts the caret at the
row's start and makes the sweep go **by rows** (`Picked::by_rows`, `CharSelection::by_rows`):
whole ones from the anchor's row to the pointer's, as a sweep down an editor's line numbers goes,
and back on the anchor's own row the caret the press left; a press on the text anchors the caret
at the column, and the sweep moves both leads -- the rows' to the row under the pointer, the
characters' to the column, which is 0 left of the text and the end right of it. Every pick is
therefore a caret and a selection, and `Picked::chars` is not optional. The column is the pointer's row-relative x less the paragraph's x within
the row, both taken from `on_sized` into cells (scroll-invariant, and no font's advance assumed).
**A sweep carries on beyond the rows** -- outside the listing's box, the pane, the window --
because the platform keeps reporting a held button's pointer wherever it goes and freya sends
its global move to every listener (`notes/upstream/freya.md`): each list's box listens with
`on_sweep_beyond`, which asks `beyond` (`src/chars.rs`, pure, tested) where the sweep reaches
-- nothing while the pointer is over a row, which answers for itself; else the row on screen
nearest it (`Reach`: the first above, the last below, the one level with the pointer beside)
at **the column under the pointer's x clamped into the box**, which the list asks of the row
through the paragraph the row lent it (`Listing::texts`, written by every render of a row with
text): past the left edge that is the first column in sight and past the right the last, and not
the row's start or end, which is what the sweep used to jump to. Held past any edge of the box,
the sweep **scrolls the view**: a task the handler starts moves it every `AUTOSCROLL_TICK` towards
the pointer -- a row up or down, a row's height sideways, the sideways extent being the widest row
(`Widest::extent`) -- and reaches the run out to what came in, for as long as the button is down
and the pointer stays past an edge, the pointer's last place kept in a cell since nothing arrives
from a pointer that is not moving (`use_sweep_beyond`, a hook so the cells outlive the handler a
render remakes; one task at a time). The release is the root's
`on_capture_global_pointer_press` and not the plain global press, which freya's scrollbar
thumb cancels. **A control the sweep passes over does not answer the pointer**: the companion header and the
symbol bar's names are `interactive(false)` while a sweep is under way (`sweeping`), since freya's
tooltip arms on the hover alone and a pointer dragging a selection up past them armed and showed
theirs (`notes/upstream/freya.md`). **What re-renders a row when the caret moves** is its list's
data: the three lists hand their rows `chars` through `new_with_data`, and the section view's
`SectionRows`, comparing by hand, once compared everything but it -- so a move along a row, which
changes no row of the run, rebuilt nothing and the caret stayed drawn where it was, while a
vertical move redrew: Left, Right, Home and End read as dead in the unified view alone
(`the_keys_move_the_caret_along_a_row_the_worker_decoded`). **Nothing inside a row may listen
to `pointer_down`**: a bubbling event is measured against the
deepest listener and every ancestor gets the same data, so a child listening would hand the row a
location relative to itself; the links listen to the press and to `over`/`out`. While the
characters are non-empty their rows draw the highlight and no wash (the pair's green
still shows); a plain press leaves an empty run and the caret's row washed, plus a **caret** at
the pressed column -- drawn over a selection too, at its lead, since that is where the next key
moves from. The paragraph takes the row's whole
height (`height(fill)`, as `CodeEditor`'s lines do) so the highlight, which `Expanded` mode
stretches to the paragraph's box, runs from one row into the next without a gap. The pointer's
icon is the row's to set, in its move handler and nowhere else -- an I-beam over the text and
to the right of it, the hand over a link (whose box in the paragraph says when it is under the
pointer; the two labels' own `CursorArea`s went, or they would fight it), the arrow over the
gutter and on leaving the row -- and only when it changes, through one cell for the thread:
a set is a message to the platform, and a row's own memory of the icon would be wrong the
moment its neighbour set another. **The only row wash is the caret's** (`wash_of`, `Wash`): the
row the lead is on while nothing is selected, in the selection's colour faded (`cursor_row_bg`);
a selection washes no row, the highlight being the selection, and a run picked out whole -- from
the gutter, by Ctrl+A (first row's start to last row's end), from outside the panes (a caret at
the line's start, or at the start of the instruction a door named) -- is a selection like any
other. The whole-row wash that preceded it made a
gutter click look like a different kind of thing from a text click, which it is not. freya counts presses
(`EventsCombos::pressed`, root state, 500 ms and 5 px): two on a word take the word as skia
divides them (`get_word_boundary`), three the row's text, and a sweep after either goes on by
character. Ctrl+C copies the characters where any are selected and otherwise the rows -- the
caret's row as its own line, address and all, as an editor copies the line under a caret with
nothing selected (`copy_text`, pure, so the rule is tested without a clipboard); Escape collapses
the selection to its caret, the rows to the caret's row with it, and drops the run on a second
press (`peel`); everything that drops a run drops its caret with it. Each row is handed its own `highlight` by its list
(`highlight_of`, unclamped at the end so a row's prop changes only when an end moves on it), the
reason `selected` is a row prop. Five tests pin it, the ends of a row's text being what is
pressed on so none measures a font: the sweep and the wash giving way, the address as gutter,
copy and Escape's order, the link as one unit that still opens its symbol (the placeholder's
one unit is skia's, which the registry cannot show, so the test is what says so), and the
double press.

**The keyboard moves the caret** (`Motion`, `CharSelection::moved`, `src/chars.rs`; `move_caret`
in `ui/marks.rs`): the arrows by character and, with Ctrl, by word; Home and End to the row's
ends and, with Ctrl, the listing's; Page Up and Page Down by a screen of rows; Shift reaches the
run out from its anchor and a key without it collapses the run to the new caret. The motions are
framework-free and tested against a five-row listing: a step is over a *character*, never a
UTF-16 unit, so a two-unit character is one step and a column left inside one by a sweep rounds
outward as `slice` does; an inline element is one step and one word; a word is a run of one
kind -- alphanumerics and underscores, or punctuation -- with whitespace passed over first, the
rule an editor's Ctrl+arrow follows, and skia's `get_word_boundary` is **not** used for it (it
is what the double press takes, but it is a hit-test on a laid-out paragraph and a key move
has no row on screen to ask). Left at a row's start goes to the row above's end and Right at its
end to the row below's start; the listing's ends clamp rather than wrap. A vertical move keeps a
**goal column** -- the column the lead had before the first of a run of them -- so moving down
through a short row and on comes back to it; it lives in `CharSelection` and not `Picked`, since
everything that puts the lead at a column of its own (a press, a sweep, a sideways key) clears
it, and those are all `CharSelection`'s own constructors. The lead is clamped to the listing and
the row's text first, because a sweep beyond the rows leaves it at `END`. The UI half decodes the
key on the pane's own box, as Ctrl+C is, and does three things the model cannot: **the rows follow
the caret** -- a one-row run at its row, or with Shift the run reached out to it, `dragging`
false -- because the rows are the place the panes point at each other through and a caret on
row 12 with the pair lit for row 3 would be two places at once; **no scroll is owed** to the other
pane (`Owed::default()`), since a held key repeats and every repeat would yank the other pane
about while the reader walks this one; and the pane reveals the caret's row through
`reveal_caret`, handed in as a closure by each list -- **not** the `reveal_row` a click uses, whose
`CONTEXT_ROWS` of margin would scroll the view while the caret was still on screen and walk the
rows away from under the reader on every repeat of a held key: the caret's reveal moves the view
only when the caret has left it, a row above coming to the top and one below to the bottom. A
caret walked **past the pane's edge** brings the pane sideways to it: the row that draws the caret
knows its x, the list's box (`Listing`, a context each list provides its rows, with its scroll and
its bounds -- freya reports a row's own `visible_area` unclipped) says where the pane ends, and a
task scrolls the list by the difference, from a task and not the render since a scroll is a
write; a listing with no run does nothing with the key. A page is `viewport /
code_row_height()`, floored, and the motion makes one of none. Nothing edits: no Backspace, no
typing. Five headless tests pin the mechanism, each on
the ends of a row's text and a row's y so none measures a font: the caret moving with Right,
Down, End, Home and a Left that crosses to the row above; Shift reaching out and a plain key
collapsing; Ctrl+End scrolling a short pane to the last row and Ctrl+Home back; and an arrow
key placing a caret on a gutter run, then doing nothing once Escape has dropped it.


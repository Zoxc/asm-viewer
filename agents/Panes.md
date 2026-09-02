# The two panes of a document

The Source pane and the Assembly pane: which of the two is on the left, which file the source side
draws, who writes a source-driven tab's line, how the panes point at each other, how a click from
outside lands, the arrow gutter, and the run of rows a reader copies.

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
dozens -- with one exception: under a pin marked `landed` (made by a `Landing`, a click from
outside both panes) whose file the listing's line info names, the companion is *that* file. A
Locations row opens a symbol on a line, and a symbol whose prologue was inlined from elsewhere
would otherwise open on that elsewhere, the line asked for sitting in a file that is not up and
the reveal with nowhere to go. A pin made inside the panes changes no file, so clicking an
inlined instruction leaves the symbol's own file on screen as it always did.

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
being a landed pin's doing, which comes with a reveal of its own.

A companion wears a **header naming its file**, which a subject does not: the strip already names
a subject, and nothing else in the window would name a companion now that the Source pane has no
strip of its own. Pressing that header opens the file as a source-driven tab, and until the project
explorer and the source search land it is the only door into one. The **assembly** side of a
source-driven tab draws the symbol the tab's driven line was compiled into, which is an ordinary
`Analyzed::showing` like any other tab's; before a line has been clicked in it there is no
question, so it says so.

**A click in a source-driven tab's own file is the only writer of `Driven`.** A click in a
companion file is a pin and nothing more, and a click in the *assembly* pane never reaches that
handler at all — which is what stops a listing from re-driving itself. Nothing else changes: the
active document does not, so nothing is pushed onto the history, the tab already being where the
reader is. A line is kept per tab rather than one for the window, and it is a `u32` and holds no
`Arc<Object>`, so it survives its binary being closed and the next ask simply answers out of what
is left.

The rows are the app's own (`SourceRow`, a `VirtualScrollView`), **not** freya's `CodeEditor`,
which paints a line background only for the cursor's row and keeps its scroll state private —
which is to say it cannot do the two things this pane exists to do, highlight the *set* of lines
an instruction maps to and be scrolled to one from the other pane. Neither objection survives a
pane the reader is typing in, so the Scratchpad's editor *is* that component (`agents/Scratchpad.md`). What
`freya-code-editor` does offer is its tree-sitter pipeline, public on its own: `SyntaxHighlighter` +
`SyntaxBlocks` + an `EditorSyntaxTheme` turn a `Rope` into one list of `(Color, TextNode)` spans per
line. The theme is the app's own (`Palette::syntax`), the grammars are ours, and an unknown
extension degrades to one plain span per line. A file is parsed once when loaded and cached in a
`static` in `ui/highlight.rs` — parsing is stateful across lines, so it cannot be per row. Two things about
`SyntaxBlocks` bite: `get_line` unwraps rather than answering `None`, and it holds one block per
`Rope::len_lines()`, which counts a phantom line after a trailing newline (hence `Highlighted::lines`).

**The two panes point at each other** through two root contexts that are inputs, not derivations.
`Focused` is where the *pointer* is; `Pinned` is where a *click* fixed them, which outlives the
pointer moving on. Two states and two shades, because a pin a hover can overwrite is a pin a hover
silently undoes; `row_background` composites the translucent colours with `blend`. Three things are
load-bearing: **a position is a file and a line** (`LinePos`), since an inlined header's line 42 is
not line 42 of the open file — the one `Arc` in the UI compared by *contents*; **a row cannot clear
the focus unconditionally**, because `EventName::cmp` leaves the order of the leaving and entering
rows' handlers undefined, so `release_focus` clears only what this row put there and `LineFocus`
carries a `FocusOrigin`; and **the scroll is a request, answered once** — `owed_reveal` only
*looks*, `reveal_made` is what clears it, and `reveal_row` does nothing when the row is already on
screen. The split is not tidiness: in a source-driven tab the click that pins is the click that
asks for the listing, so the run it wakes is still holding the previous one, in which no row
matches — a single take would spend the request there and the listing that can answer it would
arrive to nothing owed. A request nothing matches stays owed until the next click replaces it or
the tab changes. **And the ask is the pin** for a source-driven tab: `use_clear_focus` drops the
pin with the tab, so both panes fall back to the line the tab is driven from, or coming back to one
would show a listing with nothing lit and no reason given. None of this is a navigation: the
selection does not change and nothing is pushed onto the
history. `navigate` remains the only path for anything that does.

**A click from outside both panes owes both a scroll, and lands through the change of document
it makes.** A row in the Locations panel opens its symbol *and* pins the line, so `Pin::reveal`
is a pair of flags (`Owed`) rather than an `Option<Pane>`: a click in one pane asks the other,
a click in neither asks both, and each pane pays its own half. Opening is an `activate`, and the
change of document that makes is exactly what `use_clear_focus` answers by dropping the pin --
so the row does not pin; it leaves a `Landing` (`Land`, at the root) naming the document and
the line, and that effect turns it into the pin when the document it names arrives.
Whichever document arrives spends it, the one it named or another, since a landing left lying
would pin a line in a document opened for some other reason later. A row whose symbol is
already on top pins at once (`documents::land`), `activate` then changing nothing and no effect
running.

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
comes out dashed. Hovering a row draws its own branches darker, which needs a row *index* in
`InstructionList` rather than `Focused` — a source position is many rows.

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
see: at 1× and 2× every offset above it is a whole number, and at 1.5× the pane's own padding
decides. `the_gutter_puts_its_strokes_on_whole_device_pixels` pins the axis-aligned ones on a
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
need.

A `VirtualScrollView` is given one `item_size` for the whole listing, so the separator is
`code_row_height()` like every other row and the rule is drawn *inside* it, across its middle.
What that costs is **two index spaces**, and `Lanes` is the only thing allowed to convert between
them: `listing_rows`, `row_of` and `instruction_at`. An **instruction index** is what
`AsmData::position`, the gutter, `Lanes::touching` and the branch edges speak; a **listing row**
is what the scroll (`reveal_row`, `use_kept_position`) and the picked-out run (`Marked`,
`on_listing_key`) speak. `InstructionRow` carries both and never mixes them. The separator draws
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

**A branch's displacement is the other way to follow it**, drawn as a `BranchLabel` exactly where
a call's resolved target is drawn as a `RelocationLabel` — `Instruction::branch_span` says which
span to lift out, and the row is the same three children either way. Only where
`Assembly::edge_from` finds an edge, which is the set the gutter has an arrow for: a tail call
keeps its plain operand, having no row here to be pointed at. Pressing it is `reveal_row` on the
edge's target **and the pin a press on that row would have made** — `position(edge.to)`, with the
Source pane owed the scroll and the Assembly pane not, since it has just been given one. It is
still **not a navigation**: the document does not change and nothing is pushed onto the history, so
a Back that undid reading further down one function would be answering a question nobody asked. It
is a selection, though, and **in both senses**. `mark_row` picks out the row landed on — replacing
the row the press started on, which `pointer_down` has already marked, that being the one handler a
stopped press does not undo — and this is the half that holds for an object with no line info at
all. The pin is the other half, the cross-pane one, and a target the debug info places nowhere pins
nothing rather than clearing what is pinned, which is the rule the row itself obeys. Arriving at a
target and then having to click it to light it up made the reader say twice where they had gone,
and both panes would meanwhile be lit at the place the reader had just left. The press is still stopped from bubbling — the
row under it would otherwise pin the line the instruction being *left* came from, which is the one
answer the click is not asking for. The
listing's own `ScrollController` and its measured height are handed down to each row for it, the
way the hovered index already is: both are the list's handles and neither changes while it lives.

**A run of rows can be picked out and copied** in both panes (press, sweep or shift-click, Ctrl+C;
Ctrl+A takes the listing, Escape drops it). Character selection is deliberately absent: freya's
selection is char offsets into a rope wanting one `paragraph()` per line, and an instruction row is
a gutter of rects, an address label and up to three elements. The state is `Marked`, holding a
`RowSelection` **and its pane** — one selection for the window, because Ctrl+C must have one
answer. The press is `pointer_down` (a press event arrives only once the button is back up) and the
sweep is the existing `pointer_over`. Shift is watched globally at the root, because a freya
pointer event carries no modifiers at all. The key handlers are on each pane's own focusable box
and deliberately not global, or a Ctrl+C meant for a filter box would come back as a page of
disassembly. Runs are dropped by `use_clear_marks` at the root, not by an effect inside each list —
`AsmData` carries an `Arc<Lanes>` rebuilt every render, so that effect would wipe the run the press
just started. What is copied is what the row draws: `asm_line` (address plus the instruction with
the target's name in its operand), and the rope's own line for source, tabs and all.


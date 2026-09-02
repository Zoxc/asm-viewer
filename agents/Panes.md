# The two panes of a document

The Source pane and the Assembly pane: which file the source side draws, who writes a
source-driven tab's line, how the panes point at each other, how a click from outside lands, the
arrow gutter, and the run of rows a reader copies.

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

**A row a branch lands on starts a block**, and says so with a hairline across its own top edge,
so the listing reads as the basic blocks it is rather than as one unbroken run. The set is the
gutter's own — `RowLanes::arrow`, worked out in `Lanes::new` beside the disassembly — and not
`edges` asked a second time from the row, so the rule and the arrowhead it sits beside cannot
disagree. It is a **border and not a gap**: a `VirtualScrollView` is given one `item_size` and
every row must equal it, so a real gap means variable row heights or a spacer row of its own in
the list, while a border is paint alone — the layout knows nothing about one — and is drawn
inside the height the row already has. On the *top* edge, because a block starts at its target
and the mark belongs to the row it starts rather than to the one above, which the scroll view may
not have built at all. Only the targets: the row after a `ret` or an unconditional `jmp` also
begins a block, but nothing below the disassembler says which instructions end a fall-through and
that is crate work this mark did not need. The colour is `block_rule`, held quieter against the
pane than `branch_fg` — it runs the whole width of the listing where the gutter's stroke is a few
pixels long (`agents/Appearance.md`).

**A branch's displacement is the other way to follow it**, drawn as a `BranchLabel` exactly where
a call's resolved target is drawn as a `RelocationLabel` — `Instruction::branch_span` says which
span to lift out, and the row is the same three children either way. Only where
`Assembly::edge_from` finds an edge, which is the set the gutter has an arrow for: a tail call
keeps its plain operand, having no row here to be pointed at. Pressing it is `reveal_row` on the
edge's target **and the pin a press on that row would have made** — `position(edge.to)`, with the
Source pane owed the scroll and the Assembly pane not, since it has just been given one. It is
still **not a navigation**: the document does not change and nothing is pushed onto the history, so
a Back that undid reading further down one function would be answering a question nobody asked. It
is a selection, though. Arriving at a target and then having to click it to light it up made the
reader say twice where they had gone, and the two panes would meanwhile be lit at the place the
reader had just left. A target the debug info places nowhere pins nothing rather than clearing what
is pinned, which is the rule the row itself obeys. The press is still stopped from bubbling — the
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


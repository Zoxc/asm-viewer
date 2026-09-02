# Goals

Inspecting and comparing binaries.

`- [ ]` is a goal that is not done yet, `- [x]` one that is, `- [?]` one that is only a maybe —
not decided on yet — and `- [D]` one that is deferred, with the reason why on the item.

Only check an item off when all of it is done. A goal that is only partly done gets split into
one item per part, so the unfinished half stays visible.

## Source / assembly split view

- [x] Have a source view and an assembly view side by side. This is the default layout of the
  content area: the source a symbol was compiled from beside its assembly.
- [x] Map between the two views — an instruction knows its source line and both panes show
  it: hovering either side lights up what it maps to on the other.
- [x] Hovering one side highlights the other side. One source line is many instructions and
  every one of them lights up, not just the first.
- [x] Selecting one side highlights the other side. A click pins the position it points at
  and both panes keep it lit, in a stronger shade than the hover, until another click or
  another symbol.
- [ ] Drop the assembly row's own hover wash and spend that colour on the pair instead.
  There are two washes under the pointer today — `code_row_hover_bg`, which says only "the
  pointer is here", and `line_focus_bg`, which says the far more useful "this row and that
  source line are the same place". The first says what the pointer already says and competes
  with the second for the reader's eye, so the row a listing is hovered on should draw
  nothing of its own and the pair's colour should be the one that was being spent on it.
  Both panes, since the mapping is symmetric. The contrast tests re-judge whatever is left
  once one of the two washes is gone, and the question to settle first is whether the pair
  wants the hover's colour outright or its own turned up to where the hover was.
- [x] Have a function to find all source / assembly locations that match, producing a list on the
  other side. "Find all locations" on a source row or an instruction row asks the analysis
  worker for every symbol the line was compiled into over every open object, and the Locations
  view — a sidebar tab beside History — lists them one row per symbol, the object after the
  name, under a heading naming the line. A row is a symbol and not a range: the crate answers
  symbols by design, and one line answers with 9 374 of them on this app's own binary, so
  a range per hit would be seconds of DWARF walking behind every click. Pressing a row opens the
  symbol and pins the line in it with both panes owed the scroll, which is where the range is.
- [x] A function to pick the generic instance of a source function. Same query, different
  presentation — "all symbols for this function, pick one" against "all locations for this line,
  list them" — so it is the Locations view under another heading ("N instances of `foo`"),
  asked for by a second entry in a source row's context menu, offered where a function encloses
  the row. The function's lines are the source's (`src/functions.rs`: a scanner of our own for
  Rust, the grammar losing whole files to `const impl`; the tree-sitter parse for C and C++),
  not DWARF's; every symbol holding code from them is listed, an
  inlined caller included, in the crate's order, and the panel's filter narrows to a name. A row
  chooses as a location row does (`Driven::choice`, from the row the menu was opened on), and
  the row lit is the symbol drawn, so the choice shows in a source-driven tab.
- [x] An active navigation function where selection on one side moves the other side to the
  matching place — within one symbol. Clicking a source line scrolls the assembly to the
  first instruction it produced, clicking an instruction scrolls the source to its line, and
  neither is a navigation: the selection does not change and nothing is pushed onto the
  history.
- [x] The same across symbols, preferring recent history when a source line maps into several.
  It lands **only where source is driving**, which is the whole of where it lands: a
  source-driven tab's line click resolves across every open object and puts the assembly side on
  the symbol it picks, while an assembly-driven tab's source side keeps the within-symbol reveal
  above exactly — selection unchanged, nothing pushed onto the history. The tie-break is
  `compiled::pick`: most recently visited, with the symbol on screen at the head of that list.
- [x] Syntax highlighting for both sides. Assembly is coloured by span kind; source is
  tree-sitter, through the highlighter `freya-code-editor` exposes publicly.
- [x] Use the assembly colour palette for the source side's syntax highlighting, so the two
  panes read as one view rather than as two themes. `EditorSyntaxTheme` is built from the
  app's own `Palette` now: keywords and types take the mnemonic's purple, variables and
  fields the register's olive, literals the immediate's blue, function and module names the
  relocation target's near-black, punctuation the rest's grey, and comments and strings are
  two new entries of the palette's own.
- [x] Pull attributes, types and functions apart in the source side's highlighting. Three
  of `syntax()`'s entries had been mapped onto colours the assembly side already had, and a
  Rust file read as two: `attribute` and `type_` were both `keyword_fg`, `function` and
  `function_method` were `name_fg`, which is also the plain text. Three palette entries of
  their own now, light-first and turned through the background for dark — `attribute_fg` a
  plain grey that recedes, since `#[derive(..)]` is scaffolding around the code rather than
  code; `type_fg` a dim red, so a type is told from the keyword introducing it;
  `function_fg` a blue with none of the address column's greyness, so a call site is told
  from the text around it. `function_macro` went with the other two though nothing asked
  for it: `resolve_capture_color` walks a child left on the text colour up to its parent, so
  it would have been painted `function_fg` silently anyway. The assembly side keeps the five
  colours it had — that was the open question, and the answer is that none of the three has
  anything to name in a listing: `SpanKind` is a mnemonic, a prefix, a register, a number, an
  address and glue, and the one name there is a relocation target, as often data as a
  function and already told apart by `name_fg`/`name_hover_fg` and its underline. The
  contrast test holds the three on `pane_bg` alone, beside the strings and comments, and
  `attribute_fg` to a relationship as well: quieter than the keyword it left, the punctuation
  beside it and the plain text.
- [D] Grammars beyond Rust / C / C++ for the source side. Any other extension renders plain;
  each language is a `tree-sitter-<lang>` dependency and an arm in `language()`. Deferred, and
  deferred *per language* rather than as a whole: a grammar is a parser generator's worth of
  generated C compiled into the binary, so wiring a list of them up front pays for parsers for
  languages nobody here is reading, while adding the one language a reader turns out to want is
  two lines and one dependency at the moment it is wanted. Undeferred by someone opening a
  binary built from something else; `notes/Plan.md` 5d says exactly what to write.
- [D] Weight and not only colour for the source side's keywords. Assembly mnemonics are drawn
  bold and source keywords are not, so the two panes now agree about hue and still disagree
  about emphasis. The spans `SyntaxBlocks` hands back carry a `Color` and nothing else, so a
  row cannot ask whether a span is a keyword without comparing its colour to
  `palette().keyword_fg` — a kind recovered from a colour, which is exactly backwards. It needs
  the capture kind carried through beside the colour, or the bold dropped from the assembly
  side instead. Deferred at the Goals → Steps split: either fix is real plumbing for an
  emphasis nit, and neither is wanted yet.
- [ ] Read and highlight source files on a background thread. Everything a *binary* costs is off
  the UI thread already; the source side is not. `source_text` reads the file off disk
  (`source::load`) and runs the whole tree-sitter parse (`Highlighted::new`, twice: the
  highlighter's and the one the function spans are read off) inside a render, so
  the frame that first shows a file pays for both, and a large file pays for them visibly. Two
  caches keep it to once per file — `source.rs`'s own and `HIGHLIGHTED` — but that once is a
  frame, and one of them is emptied deliberately: the spans carry the palette's colours baked
  into them, so `set_appearance` clears `HIGHLIGHTED` and a theme switch re-parses every file on
  screen at the moment the window is repainting. `use_analysis`'s shape is what this wants — one
  worker for the app's lifetime, a channel, and a pane that goes on drawing what it has until the
  answer lands. Two things to check before starting, since neither is a move: whether what
  crosses is `Send` at all (a `Highlighted` is a `Rope`, a `SyntaxBlocks` and the function
  spans, and the highlighter
  is `freya-code-editor`'s), and which palette a parse off the thread resolves its colours
  against, the answer having to be the one the rows are drawn in when it arrives.

## Navigation

- [x] Clicking on functions in assembly should navigate to them.
- [ ] Clicking on functions in source should navigate to them. A click on a source line moves
  the assembly pane to that line's instructions; a click on a *call* in the source still does
  nothing, since nothing maps a source identifier to the symbol it names. Sequenced after
  the LSP item under *Projects*: rust-analyzer is what should answer which symbol an
  identifier names, rather than building name matching over demangled strings here.
- [x] Navigating in assembly should also navigate source, within a symbol: clicking an
  instruction scrolls the source pane to the line it was compiled from.
- [x] Selecting another symbol puts the source pane on that symbol's own lines. The pane
  never inherited the offset the *previous* symbol left, and since the one strip landed a
  tab remembers a row for *each of its two sides*, keyed by the document, so two symbols
  compiled from one file do not share a position. The first open was the half left: a tab
  seen for the first time opened at the top of the file. `SymbolLines` now carries the
  **line** the symbol opens at beside the file it opens in -- both off one line-info row,
  worked out on the worker -- and `use_kept_position` takes an opening row a tab nothing is
  remembered for lands at, three rows above that line so the signature is not flush against
  the top. A remembered row still wins, and a symbol whose debug info places it nowhere
  opens at the top of the file as before.
- [x] Mouse buttons can navigate history so you can go back and forth.
- [ ] Back and forward should be **per tab**, the way a browser's are: each tab keeps its own
  trail, the mouse buttons walk the trail of the tab on screen, and going back in one tab does
  not move another. With it, two rules that only make sense together: a click that navigates —
  a relocation target in the assembly, a row in a sidebar list — **stays inside the current
  tab**, replacing what it shows, and **Ctrl+click opens a new tab** instead; and the History
  panel stays **global**, one record of everywhere the reader has been across every tab, since
  that is what makes it a way of finding somewhere you were rather than a second copy of a
  tab's own trail.
  What is in the way is that today **a tab *is* its document**. `Docs` maps a tab's id to one
  `Document`, the active document is that table read through the panel's active tab, and both
  viewing-position maps are keyed by the document. A tab that shows several documents in turn
  means the table holds a *trail and a cursor* per tab, with the document becoming the trail's
  current entry. The tab id itself survives that unchanged, which is the one piece already
  built: a dock tab has been a handle rather than a document since documents moved into the
  dock.
  Four things to decide before starting, none of them obvious. What the two `Positions` maps
  are keyed by once one tab shows many documents — per tab, or per tab *and* document, which is
  the difference between a tab remembering one scroll position and remembering one per place it
  has been. What `session.toml` saves per tab: the current entry alone, which is what it holds
  now, or the whole trail, which makes back work across a restart and makes the file bigger by
  a factor of however far the reader wandered. What `close_binary` does with a tab whose trail
  is *partly* in the closing file, where today the tab simply goes with its document. And what
  a click in the History panel means once tabs have trails — go there in the current tab, or
  raise the tab that already shows it.
  It also settles the temporal-tab item under *Panels and tabs* from the other direction: that
  one exists because walking a symbol list leaves a tab behind per click, and a click that
  navigates in place leaves none. Decide the two together rather than building both.
- [x] Add `<`, `>` navigation buttons to the top bar. Two chevrons at the left of the toolbar,
  driving the same `navigate` the mouse's side buttons do, so every rule about tabs, selection
  and history holds without a second spelling of it. Each names where it would land in its
  tooltip, and each is **dimmed rather than hidden** when there is nothing that way: the pair
  keeps its place under the pointer, and a reader who has been nowhere yet can still see it is
  there. They read `Hist`, which is what repaints them as entries appear and as the cursor
  moves.

## Assembly viewer

- [x] Bar under the Assembly tab with the full demangled + mangled symbol name. A header inside
  the Assembly pane, the demangled name over the mangled original, each on one line. It names
  **what the pane is drawing** and not what is selected — the same `Analyzed::showing` the rows
  below it come from, where the Info pane it is taking over from read the selection and so said a
  different thing for as long as the worker took. A tab that is a whole object has no listing
  ever worked out for it, so the bar names the object there instead. The names are ellipsised
  rather than wrapped, this repo's samples reaching 1038 bytes mangled, and **pressing one copies
  it**, which with the tooltip is the whole of how the rest of a long name is got at.
- [x] Name the Assembly tab after the function — just `namespace/module::fn_name`, without the
  extra generics, mangling, etc. (for Rust / C++). There is no Assembly tab any more: a document's
  tab *is* named after its function, and `src/naming.rs` cuts that name down to its last two path
  segments. Generic arguments go however deep they nest, `<Vec<T> as IntoIterator>::into_iter` is
  `Vec::into_iter`, a C++ argument list and the `const` after it go, `operator<<` is a name rather
  than a bracket, and rustc's legacy `::h<hash>` suffix goes with them; the closure a symbol *is*
  is kept, and only the innermost one. Over this app's own binary that is 151 characters of
  demangled name down to 21. The History rows take the same name through `entry_text`, which the
  two share; the whole name is still what a tooltip says and what the History filter matches, and
  the 40-character elision is still the last word for a name that is long anyway.
- [x] An expanding section under the Assembly tab to show more symbol info, replacing the Info
  tab. The bar above is its collapsed state, opened by a disclosure triangle in a column of its
  own — the Objects tree's idiom down to the two glyphs, a triangle being the toggle where a name
  is a copy. It answers what the Info view answered, plus the address and the object a symbol came
  from, neither of which was shown anywhere: section, address, declared size and extent for a
  symbol; format, symbol count and path for an object, which is the other case the view had. Open
  or shut is kept **per tab** and never saved, since both panes are mounted afresh for every
  document and a flag in the pane would shut the section every time the reader looked elsewhere.
  It is keyed by `DocId` and not by `Document`, so it holds no closed binary's bytes and needs
  forgetting in none of the three closing paths. The view is then gone — eight dockable views
  become seven — and nothing had to be migrated with it, the dock layout not being persisted.
- [x] Keep the `rip+` visible in a relocated rip-relative operand — `mov dword ptr [rip+<target>], 7`
  rather than `mov dword ptr [<target>], 7` — when you can navigate to the target.
- [x] Don't zero-pad the target of a jump or a call. `jle short 000000000000004Bh` spent the
  width of a 64-bit address on a number nowhere near one; it now reads `jle short 4Bh`.
  Displacements and immediates are a separate `iced-x86` option and are left as they were.
- [x] Allow selection, of rows. Both panes: click a row, shift-click or drag to reach out
  to another, Ctrl+C copies the run as text, Ctrl+A takes the whole listing and Escape
  drops it. One selection for the window rather than one per pane, so Ctrl+C has one
  answer. The assembly copies what the row draws — the address column and the instruction
  with the relocation target's name in its operand — and the source copies the file's own
  lines.
- [ ] Text selection, of characters, in both the assembly and the source pane: drag across
  part of a line or across lines and Ctrl+C copies the text, the way an editor does, beside
  the row selection above, which stays for whole runs. Was deferred, with the cost read out
  of the freya sources (`notes/Plan.md`, 7c), and the cost stands: a scroll view is *not*
  what stops it — freya's selection is a range of char offsets into a rope held by the
  editor, and its own `CodeEditor` selects across the rows of a `VirtualScrollView` happily —
  but the model wants one rope, one line per row and **one `paragraph()` per line**, and an
  assembly row is a gutter of rects, an address label and up to three separate elements, the
  middle one being the clickable relocation target, which could only survive as an inline
  child whose placeholder character is not a character of any rope. So it is a rewrite of
  the relocation link and the arrow gutter on the assembly side; the source side could have
  it cheaply and is wanted in the same step, so the two panes keep behaving alike.
- [ ] Ctrl+C copies whatever is selected, wherever it is: the run of rows in either pane
  today, the characters once the goal above lands, and the other places text is picked out
  -- a filter box, the scratchpad's editor, its diagnostics and its output -- so the one
  binding has one meaning and never comes back with a page of disassembly for a word picked
  out somewhere else. The key handlers are per pane on purpose (`agents/Panes.md`); this is
  the rule that decides which of them answers, not a global handler.
- [x] A separator before a row something jumps to, so the listing reads as the basic blocks
  it is rather than as one run of instructions. A **row of its own** and not a border on the
  row below: a block reads as separated from the one above rather than as underlined by it.
  `VirtualScrollView` is given one `item_size` and every row must equal it, so the separator
  is `code_row_height()` like the rest and the rule is drawn centred inside it, clear of the
  gutter — what it costs is a second index space, listing rows against instruction indices,
  which `Lanes` converts between and nothing else may. It carries the lanes crossing it, and
  the padding the instruction rows take, so a branch's line runs over the gap unbroken and
  unkinked; and it takes the mark handlers so a sweep is not cut in half at every boundary. The
  targets were already known and are not worked out again — `RowLanes::arrow`, the same set
  7b's gutter draws an arrowhead on, carried to the row beside the lanes it already gets. Its
  colour is `block_rule`, recessive by construction: it reads against the pane and stays
  quieter than the branch line beside it in both palettes, which the palette test holds. The
  row after a `ret` or an unconditional `jmp` starts a block too, but nothing below the
  disassembler says which instructions end a fall-through, so that half is left.
- [x] A unified **section** view of code: the whole `.text` as one endless scroll, with the
  symbols drawn as labels *inside* the listing where they start — what `objdump -d` reads like.
  *Done*: a third kind of document, `Document::Code`, one per object, opened by pressing the
  object in the Objects list and drawn by `src/ui/section_view.rs` — every code section placed in
  the parse's layout, a label per symbol, the rows estimated from the bytes before a stretch is
  decoded and empty until it is, decoded in windows of a few screens around the reader through
  the one worker, the place kept as an address, the pinned line's file beside it, and two doors
  between it and the symbol view (`agents/Panes.md`, `agents/Worker.md`, `agents/UI.md`).
  It is how you see what sits between two functions, what the padding is, and code no symbol
  claims at all.
  It is a **separate viewing mode beside the function/symbol assembly view, not a replacement
  for it**. Both stay: reading one function is the common case and is what the panes, the
  history and the saved session are all built around, while the section view is for the times
  you need the surroundings. That is the deciding constraint for everything below — the
  symbol-keyed machinery is not migrated, it stays exactly as it is, and the section mode brings
  its own address-keyed answers alongside.
  Three things in the way, all of them real and worth knowing before starting. **Length**: a
  `VirtualScrollView` is told a row count up front, and x86 is variable-length, so instruction
  *n* of a section cannot be found without decoding from a known start — the sorted symbol
  addresses in `Section::symbols` are the sync points that make this tractable. *Decided, in
  the crate* (`Listing`, `agents/Analysis.md`): the section is one stretch per symbol address,
  each the symbol's own listing plus the bytes from its extent to the next label as a gap, and
  a stretch is decoded when asked for and counted then; the skeleton is the addresses alone and
  costs nothing. A gap is shown as bytes and never decoded — bytes no symbol claims are not
  known to be code. And it is *all* the object's code, not one section's: every code section
  placed in the layout the parse gives them (`CodeListing`), which is what makes one function
  per section — rustc's shape — read as one listing rather than as many of one function. **Identity**: `Assembly::edges`, `Lanes`, the
  per-tab viewing row and the copy-a-run selection are all indices into *one symbol's*
  instructions. Being a separate mode is what makes that affordable — none of it has to change
  — but the section mode needs the same four answers keyed by address instead, so the question
  is whether those are generalised over what indexes them or written twice. Note that indices
  were deliberately chosen over addresses in the first place, so that a symbol's edges are
  independent of where it sits; a section listing has no such need. *Decided*: written twice.
  The crate's half is on the row — every instruction has its `address` and a branch has the
  address it names (`Instruction::branch`), judged by nothing, since whether the target has a
  row is only known once the stretch it is in has been decoded; the other three are the view's
  to write against those. **Scale**: the app's own binary's `.text` is far past what
  decoding eagerly on the analysis worker would answer in one go, so this wants the worker to
  answer for a *window* of the section rather than for a whole symbol.
  Note what it would make easy in return: the "gap or line before a jump target" item below, and
  showing a symbol in the context of its neighbours rather than as an island.

- [x] Have arrows for jumps. A gutter left of the addresses draws every branch that stays
  inside the symbol as a line from its row to its target's, with an arrowhead where it lands
  and shorter branches nested inside longer ones. At most five lanes wide, and only as wide
  as the symbol needs; past five, the outermost lane is shared. Hovering a row draws its own
  branches darker, all the way to where they go.
- [x] Pixel-align the gutter's strokes. Every stroke in `gutter` is now placed by its **edges**
  through `Grid` (`src/pixels.rs`), which rounds them to the device pixel grid rather than
  putting a centre on a fraction and letting a one-pixel line come out as two grey ones. The
  scale factor was there to be had: freya keeps it on `Platform`, a root context the renderer
  writes and `freya-testing` takes from `with_scale_factor`, so `pixel_grid()` in
  `ui/metrics.rs` reads it the way `fonts()` and `palette()` are read and subscribes the row
  that asked. What was really blurred at 1× was the horizontal run and the cut at
  `code_row_height() / 2.0` — whole numbers whenever the row height is even, and so half-pixels
  once a stroke was centred on them; the lanes' own columns already landed right, `lane_x`
  being half a pixel off a multiple and the stroke half a pixel wide. At a fractional scale all
  of it was off. The arrowhead's diagonals are **not** aligned and never could be — at 30° a
  line crosses into a new row of pixels wherever it is put — so they are weighted instead of
  aligned: half a device pixel more ink, so the two rows the antialiasing spreads them over stop
  reading lighter than the crisp run they point along. Only their pivot is snapped, and it is
  that run's own end. The row's own top and bottom stay exact, a lane's line having to reach
  them or the column comes out dashed, and a corner's half-stroke now ends at the far edge of
  the run instead of inside it. The snapping is relative to the gutter's origin, which nothing
  inside a row can see: at 1× and 2× every offset above it is whole, at 1.5× the pane's padding
  decides. `pixels::tests` pins the geometry and
  `the_gutter_puts_its_strokes_on_whole_device_pixels` pins the laid-out strokes on a 26-pixel
  row, the even height the old placement was worst on. The separator row's rule went the same
  way and from the same answer, `Grid::stroke` over the middle of a row: it was centred by
  `cross_align` and so sat at 12.5 in a 26-pixel row, and a rule half a pixel off the gutter's
  horizontal run reads as a step where a branch lands on a boundary.
  `a_block_rule_lands_on_whole_device_pixels` measures the rule against the run rather than
  working the answer out a second time.
- [x] Follow a jump by clicking its target offset, the way a call's relocation target is
  clicked. A branch's displacement (`jle 4Bh`) is a link where the listing holds the row it
  lands on — the same set the gutter draws an arrow for — and pressing it puts that row on
  screen. `Instruction::branch_span` is `relocation_span`'s twin, the span the displacement
  was printed into, and the two are exclusive: a branch whose displacement is a relocation
  placeholder names no address of its own. `Assembly::edge_from` pairs the span with the edge
  that says where it goes. It is a scroll within one symbol and **not** a navigation: the
  document does not change and nothing is pushed onto the history. It does move the
  selection, the way clicking the target row would: the row landed on becomes the picked-out
  one, and its line is pinned with the source side owed the scroll — the first half holding
  for an object with no line info, the second lighting both panes where the reader has
  arrived rather than where they left. A branch out of the symbol — a tail call — keeps its
  plain operand; making that one navigate like a call target is an item of its own.
- [ ] Show the current symbol as a breadcrumb in the unified view. The bar over the pane names
  the object for a code tab, and nothing on screen says which function the rows under the
  pointer, or at the top of the pane, belong to once its label has scrolled off — a reader
  three screens into a long function is reading unnamed code. The stretch the top row is in is
  already what the kept place is worked out from (`section::Rows::row`), so the bar could name
  that symbol beside the object, object › symbol, and pressing it would be the "Open as symbol"
  door in one more place. Whether it follows the top row or the pointer is the one decision:
  the top row is stable and the pointer is what a hover already lights.
- [ ] Let a call target with no symbol be opened. `Code::relocation` answers a `Relocated`
  whose `target` is `None` whenever the relocation points at something the object has no text
  symbol for — a section, a data symbol, an undefined import — and the operand is then drawn
  as the placeholder the linker will overwrite, with nothing to click. A reader who can see
  where a call goes should be able to go there, so the question is what a document is when
  there is no symbol to name it: a place in a section at an address, which is the same
  question the unified section view above asks, and probably the same answer.

## UI

- [x] Migrate the app's hand-rolled panes to freya's own panel components (`ResizableContainer` /
  `ResizablePanel` / `ResizableHandle`), which also makes the split user-resizable.
- [x] Docking panels: a `DockingArea` inside each half of the split, with Objects, Symbols, Info
  and Assembly as tabs that can be dragged between the two areas, stacked into one panel as real
  tabs, or split further. Two of those four are no longer views: an assembly listing is half of a
  *document* rather than a pane of its own, and the Info view has since been folded into the bar
  over that listing.
- [x] A dark mode, in the same palette rather than a second one — the light colours carried
  over at dark-mode lightness, so the two themes are recognisably one design. Every relationship
  in the light palette is preserved through the dark one; the translucent washes were re-judged
  as what they *composite to* rather than inverted, since the same alpha over a dark ground is a
  fraction of the step it was over white. Asking for a colour is what subscribes a component to
  the theme, so no call site changed and a switch repaints exactly the scopes that draw
  something coloured. The highlighted-source cache is cleared inside the one function that can
  change the appearance, so it cannot be routed around. freya's own components get its
  `light_theme()`/`dark_theme()` alongside, a white text box on a dark pane not being a theme
  switch. The light theme is byte-for-byte what it was. "Follow the desktop" is answered by the
  windowing system rather than by asking a desktop tool: freya surfaces winit's `Window::theme()`
  as a reactive `Platform::preferred_theme`, so no process is spawned and the app repaints when
  the desktop switches theme while it is running, which a one-shot query could never do.
- [x] Use freya's icon libraries where they suit, panel titles at least — an icon beside
  Objects / Symbols / Info / History / Assembly / Source in the tab bar. `freya`'s
  `icons-lucide` feature is on and `Tab::icon` names one glyph per view (`package`,
  `square-function`, `history`, `binary`, `file-code`; `info` went with the Info view), drawn at the interface
  font's size times 1.25 and in the palette's new `icon_fg`. The two places that had
  settled for text were weighed again with the dependency in and both keep it: Lucide does
  carry `case-sensitive` / `whole-word` / `regex`, but rendered at the toggles' 22px they
  say less than `Aa` / `\b` / `.*`, which are the regex the toggles turn on; and nothing in
  the 1640-icon set names an object file format, so the file-type tags would all become one
  generic page. See `notes/Plan.md` 6e.
- [D] Bring back the floem-style thicker scrollbars. Deferred: freya 0.4 hardcodes the scrollbar
  sizes (its `ScrollBar` theme declares a `size` field that is never read, and `ScrollView` /
  `VirtualScrollView` always pass `theme: None` with the override fields `pub(crate)`), so the only
  way is vendoring the whole scrollview module (~1350 lines) out of `freya-components` — too much to
  carry for a cosmetic change. Revisit if freya makes it themeable.
- [ ] Keep every expensive operation off the UI thread. A standing rule rather than a task that
  finishes, since each new one arrives with whatever feature needed it. freya's executor *is* the
  UI thread, so a `spawn` is not the answer: what is expensive goes onto a `std::thread` fed an
  `async_channel`, the shape `use_analysis`, `open_binaries` and the scratchpad's worker already
  share — one thread for the app's lifetime, a queue drained to its newest entry where requests
  supersede, and a pane that goes on drawing what it has until an answer lands. Three things are
  across already: binary inspection, reading and parsing a binary, and the scratchpad's build and
  run. What is known not to be — `project::flush` writes both TOML files from a timer task on the
  executor every thirty seconds, and again from the window's close hook; `fonts::resolve` spawns
  `kreadconfig`/`gsettings` subprocesses, on the first frame and again on a settings change (the
  answer is cached per process, so it is the first call that costs); startup reads
  `settings.toml`, `recents.toml` and the open project's two files synchronously inside `app()`;
  and reading and highlighting a source file, which is its own item under *Source / assembly
  split view*. None of those four is measured, which is where this starts: the rule is worth
  keeping, and an atomic write of a few hundred bytes may still be cheaper than the channel it
  would take to move it.
- [ ] Make the line-number gutter gray, in the scratchpad's editor and in the Source pane alike,
  so the numbers read as a margin beside the code and not as a column of it.

## Panels and tabs

- [x] History panel on the bottom left with recent functions.
- [x] The history panel also lists recent source files. It falls out of the history recording
  *documents*: a visited file is an entry like any function, spelt by its own name and wearing
  the same kind icon its tab does. The recording rule changed with it — the push moved into
  `activate`, which is told why it is being called, so opening a document or going to one is a
  visit and switching to a tab that is already open is not.
- [x] Don't insert duplicate history entries, bump existing ones instead.
- [ ] A tab kind for a file, so an object or an archive can be opened and read about. Today
  a row in the Objects list can only be expanded or closed, and everything the parse learnt
  about the file — its format, architecture, sections, symbol counts, what its members are,
  how long each took to read — is either shown nowhere or squeezed into the section under the
  Assembly pane's symbol bar, which is about a *symbol*. Opening a file row would give it a document of its own: the file's own
  facts up top, its members listed for an archive, and the timings the read already measures
  beside them, so "why was this slow" is a question the app can answer about itself. The
  decisions are what document kind a file is (`Document` names a place in a binary or a file
  today, and a whole binary is neither), and whether the timings are always collected or
  gathered only when something asks.
- [x] Tree view for objects, with an indicator per row for the file type. A file that
  contributed one object is one row; an archive is a parent row its members fold under,
  and the type is a short tag (`ELF`, `PE`, `COFF`, `MACH`, `AR`) rather than a picture —
  freya's icon set is a dependency behind a feature and has no notion of an object format.
- [x] An indicator for an object still being processed. The state belongs to the *file*, not to
  an object — an object that has not been parsed does not exist, while the file is the thing the
  reader opened and the thing that already has a row. A file being read wears `…` where its
  format tag will go, since the format is not known yet, and its name is dimmed. Two static cues
  rather than a spinner: a sidebar row is one of hundreds.
- [x] Filter bar under objects / symbols / history, with icons for caps / full word / regex.
  One `FilterBar` in three places, each list keeping its own filter; the toggles are written
  as the regex they turn on (`Aa`, `\b`, `.*`) and a pattern that will not compile says so.
- [x] Tooltip for items in panels. The objects, symbols and history rows all show their
  own text in full, which is what a name cut off at the pane's edge needs; the assembly
  and source rows deliberately have none, being code the pointer sweeps across rather than
  a name that could not fit.
- [ ] Show a row's tooltip only when the row is actually cut off. A tooltip repeating a name
  that is already fully on screen is noise the pointer drags behind it across a whole list —
  and with the zero delay above it arrives the instant the pointer crosses a row. What it
  needs is a comparison the app cannot presently make: freya reports a laid-out box but not
  the width the text *wanted*, so "does it fit" has to come from somewhere — a measured
  `longest_line()` on a row allowed its natural width, or the ellipsis being detectable, or
  the row reporting its own overflow. Settling that is the goal; the rule once it is settled
  is one line per list, and the same question decides it for the tab bar, whose tabs elide
  too.
- [x] Instant tooltip delay for list items. A truncated name in a list is read by sweeping
  the pointer down it, and freya's 500ms default made that useless, so every list row
  passes `TooltipContainer::delay` a zero. The filter toggles keep the default: theirs
  explains what `\b` means rather than finishing a word, and a pointer crossing the bar
  should not light three of them. What is left is freya's own 150ms fade-in, which is
  inside the component and not reachable from outside — measured, not guessed: at 200ms
  the tooltip is up, at 48ms it is not.
- [x] Context menu on a file in the objects panel to close it. Right-clicking a file row —
  an archive, or a file that contributed the one object it is named after — offers "Close
  file", and closing drops every object of that path, an archive's members with it. What
  pointed at them: the open tabs in it are closed and the selection moves to the
  neighbouring tab the way closing one tab by hand does (no selection at all only when that
  was the last), the history entries are *dropped* rather than degraded, through the same
  walk a restore uses for binaries that have changed, and `Project::binaries` follows the
  objects and is written to disk at once. A member row offers nothing: the file is the unit
  that closes, so the row above it is the one that closes it.
- [x] Bookmarks panel for pinned symbols / functions — a list the reader adds to deliberately
  and that outlives the session, unlike the history, which records everywhere they went and
  drops the oldest. A sidebar panel beside Objects / Symbols / History, saved with the
  project (`project.toml`, being what the user said; `agents/Persistence.md`). Added and
  removed from a right-click on a Symbols row or a History row, on a document's tab, or on an
  instruction row in the assembly view ("Bookmark symbol"); a bookmark whose binary is
  closed is kept and drawn dimmed rather than dropped, and comes back when the binary does
  (`agents/Sidebar.md`). The name clash this warned about is cleared: the context that was
  `Pinned` — the source position a click fixed the two panes on, a transient, one-at-a-time
  gesture and nothing like a bookmark — is `Anchored` now (`Anchor` the value), so nothing in
  the code says "pinned" of a symbol.
- [ ] A bookmark control on the symbol bar, for the symbol being read, beside the two menus
  above.
- [x] Left panel to explore project directory / files. The Files view: the project's directory
  as a tree, read one level per unfold and re-read on a refold, which is the refresh; the tree
  is the fold state. A file's row opens it as a source-driven tab, spelled the way the debug
  info spells it so the two meet in one tab; its right-click offers "Open file", which is the
  parser's to judge, or "Close file" once it is loaded. Nothing here judges what a file is
  (`agents/Sidebar.md`).
- [ ] Only close the assembly view by default in a source-driven tab if its file is not in a
  compiled language: a `Cargo.toml` or a `.json` opens with the source side alone, a `.rs` or
  `.c` with both, as now.
- [x] Left panel for symbol search — the Symbols panel filters every loaded object's symbols
  by substring, whole word or regex, on the demangled name the row shows.
- [ ] Make that search reachable and ranked: no keyboard shortcut puts the caret in the filter
  box, and matches come back in the list's own name order rather than by how well they match.
- [ ] Left panel for project directory / source search.
- [x] Tabs for assembly functions / source files. One tab per open document — a function, an
  object or a source file. Clicking a tab switches; the × closes it and moves to the neighbour;
  closing the last one goes back to the placeholder. The list is saved with the session and comes
  back on a rerun, in the order the reader left it.
- [x] Open documents are tabs in the dock, beside Project / Settings / Scratchpad, rather than a
  strip of the app's own over it. This reverses the earlier decision, whose argument was that the
  dock tree is the layout and a layout must survive documents opening and closing. Two thirds of
  that are answered by **designating** one panel: it is exempt from the folding sweep, so closing
  the last document folds nothing away, and it gives one answer to "which document is active" —
  its own active tab, from which `Active` is now *derived* rather than kept beside the list. The
  remaining third is a real cost and is accepted: the layout and the list of open documents are no
  longer separable, so the arrangement survives a close because a rule says so rather than because
  the shape makes it impossible to break. What it buys is that a reader arranges documents the way
  they already arrange the views — tabbed together, split, or dragged aside — and that there is one
  kind of tab header to change instead of two. A document may only ever live in that panel, since
  one visible document is what lets the analysis, the picked-out rows and the two panes' focus each
  hold one answer for the window; a view may go anywhere, that panel included. The Source pane
  stops being independently dockable in return, being inside a document rather than beside one.
  A view being the tab on top means there is *no* active document, which is what keeps the
  derivation honest and is the one visible edge: the analysis clears and the session records
  nothing active until a document is on top again.
- [x] Two kinds of tab, assembly-driven and source-driven, told apart by an icon. The two
  independent strips (one for functions, one for files, each with its own notion of what is
  open) are one strip of `Document`s, each of which is a place in a binary or a file, and the
  variant says which of the tab's two sides it is *about* and therefore which drives the
  other. So opening a file and opening a function produce the same kind of thing. The doctrine
  changed with it: "a document is a place in a binary" became "a place in a binary *or* a
  file". A source-driven tab is opened from the Source pane's companion header, which is the
  only door into one until the project explorer and the source search land.
- [x] The assembly side of a source-driven tab: clicking a line in the file shows the assembly
  compiled from it. The tab is driven from a line, and the click in its own file is the only
  thing that writes one — a click in the assembly pane never does, so a listing cannot re-drive
  itself. The question goes to the one analysis worker, which now takes a question rather than a
  symbol; which symbol wins is the most recently visited, **with the symbol already on screen at
  the head of that list**, since nothing is pushed onto the history between two clicks in one
  function and reading down a generic function would otherwise walk across its instantiations.
  Below the tie-break the order is the crate's own and is arbitrary; picking deliberately is the
  generic-instance item above. A line no object holds code from leaves the listing that is up and
  loses only the pin's highlight, which is what says the click landed nowhere. Two things came out
  of it that were not in the ask: an answer can now outlive the document that named it, a
  source-driven tab surviving a binary close, so the analysis lets go of a closed binary; and the
  reveal a click asks for is now looked at before it is taken, the listing that can answer it not
  being the one that is up when the click is made.
- [x] The source side of a source-driven tab is on the left and its assembly on the right. The
  side a tab is driven from leads in both kinds, so the pane the reader came here to read sits
  where the assembly does in an assembly-driven tab and the side it resolves to sits beside it.
  `DocumentBody` is the only thing that knows which way round a document goes — neither pane is
  handed a side — so the swap is the order of two `.panel(..)` calls, and everything the two
  panes share was already keyed by which pane it is rather than by where it sits: the focus, the
  pin and the scroll each owes it, the picked-out run, and the row each side was left on. The
  split's one remembered width was the one thing keyed by position, and was left that way
  deliberately: it is now the *leading* pane's width rather than the assembly pane's, so
  switching between the two kinds of tab leaves the handle where the reader dragged it instead of
  throwing the two widths across the window. Panel 0 is the leading pane either way round, which
  is what keeps the container's own drag arithmetic — positional too — agreeing with it.
- [ ] Refactor the tabs away from freya's dock panels and onto components of the app's own,
  with a fixed panel for the tabs rather than one the reader can fold, split or drag documents
  out of.
- [ ] Let the views close — Project, Settings, the Scratchpad and the rest — and add a menu at
  the top left of the window to reopen them. Today a view has no × because there is no way back
  once it is closed; the menu is that way back, so the × can come.
- [ ] Selecting a symbol in a view opens a "temporal" tab when the symbol is not already open
  in a tab: one preview tab reused by the next such selection, so walking down a list does
  not leave a tab behind per click. What promotes it into a tab that stays is a design
  decision for the step that builds it.
- [x] Close the other tabs from a tab's context menu. A right-click on a document's tab offers
  "Close other tabs", which keeps the tab it was opened on and closes every other document in
  the panel; the panel lands on the kept tab when what was on screen is among the ones closing,
  and stays where it is otherwise. A view sharing the document panel is left alone — it is not a
  document, and having no × of its own is the same argument. A tab that is the only one open
  offers no menu rather than a menu whose one row would do nothing.
- [x] Reach a tab that has scrolled off the end of the bar. Documents are opened by the dozen
  and the bar scrolls, so a tab past the right-hand edge used to be reachable only by scrolling
  to it — and a reader had no way of seeing what was out there. A control at the right of the
  document bar lists **every** open document, with the one on screen marked, and picking one
  activates it. All of them rather than only the hidden ones: which tabs are off-screen means
  measuring the bar's content against its viewport, and a list that changes length as the bar is
  dragged is worse to use than a complete one. The popup is positioned by the app rather than
  through `ContextMenu`, which pins a menu to the pointer and clamps to nothing — from a button
  at the right-hand edge that would draw off the side of the window.
- [ ] A tab should open immediately even while its binary is still being read, with a loading
  message inside it rather than nothing. Today a tab can only exist once the object it names
  does: a document is resolved against the objects list by path, object name and symbol name, so
  the startup restore deliberately waits for the *whole* load before opening any tab — resolving
  against a half-filled list would drop the tabs whose object had not landed yet. The objects
  themselves already stream into the sidebar as they parse, so a reader watches a file's symbols
  appear while the strip above them stays empty, and on a 331 MB binary that is seconds of a
  window with nothing open in it. What it needs is a tab that can be *unresolved*: opened from
  the saved document, drawn as itself with "Loading…" in its panes, and resolved when the object
  it names arrives — which also means deciding what such a tab does if the load finishes and the
  object never comes, where the answer is probably the same drop the restore does now, only
  later and visibly.
- [x] A larger close target on a tab, with a highlight under the pointer. The × on a document's
  tab is a control of its own now: a square centred on the glyph with four pixels of air on every
  side, capped at the row so it never decides how tall the bar is, so what a reader hits is the
  padding as much as the × itself. The glyph is drawn a third larger than the interface font
  beside it, being a mark rather than a letter — at the text's own size the multiplication sign
  reads as a scratch — and the target is written as the glyph plus its air, so growing one grows
  the other. Under the pointer it takes a wash of its
  own and the glyph comes up to the interface text, while the tab under it stays lit — the two
  are told apart by the close being the deeper step, in both palettes, which the visible-step
  test holds along with the glyph's contrast over the composited wash. A component and not
  another line of the chip, because freya has no hover pseudo-state and the state has to be the
  ×'s own.
- [ ] A shortcuts panel, listing every key and every mouse gesture the app answers to. There
  is no way to find out what the app does today short of reading the source: Ctrl+C, Ctrl+A and
  Escape in the two code panes, the mouse's side buttons going back and forward, shift-click
  reaching a selection out, right-click's menus, and whatever the keyboard goal below adds.
  A view like Settings, listing them by the view each belongs to. The one decision is whether
  the list is written out by hand — honest, and wrong the first time someone adds a binding
  without touching it — or generated from the handlers, which would mean bindings become data
  the handlers read rather than matches they are written as. That refactor is the real content
  of this item, and it is worth doing only if the keyboard goal below wants it too.
- [ ] Reach the panels from the keyboard. Only the two code panes have key handlers at all: the
  tab chips are pointer targets, the sidebar lists have no cursor, and there is no way in to a
  filter box — which is the ranked-search item above seen from the other side. Note what it
  needs deciding first: what "the focused pane" means when either dock area can hold any view.
- [x] The archive row's object count survives a narrow sidebar. The count is a column of its
  own — the digits and a gutter beside them — measured whole before the name is handed what
  the columns leave, so dragging the split left is taken out of the name, which ellipsises, and
  never out of the count. Half of it was already there and unrecorded: the row is laid out under
  `Content::Flex` with the name as its one flex child, which is what keeps the count inside the
  row's clip instead of past its edge. What was missing was the gutter, without which the name
  grew right up to the digits and the ellipsis read as part of the number. A headless test pins
  both halves, comparing a 150px pane against a 300px one rather than against a width of its
  own: text is really shaped there, so a digit measures whatever fonts the machine has.

## Projects

- [x] Minimal project support: the previous session's binaries and selected symbol reopen when
  the app is rerun, for easier testing.
- [x] Have a project concept. A project is a directory under `projects/<id>/`, and the
  directory *is* the identity — not its files, not its given name, not its associated
  directory. `ProjectId` is a validated single path component, checked on the way in from
  disk too, so a hand-edited recents list cannot name `..`.
- [x] Anonymous projects — opening files without an explicit project — should be saved too, next
  to the user / global settings. There can be multiple such anonymous projects. Anonymous means
  the `name` key is *absent*, the same real third state an unspecified font setting is. The id
  is the first free `project-N`, claimed with a `create_dir` that fails rather than opens, so it
  cannot collide with a directory already there or with a second copy of the app racing for it;
  the spelling carries no meaning, and naming a project later does not move it. The directory is
  created by the first write that has something to say, so a run in which nothing was opened
  leaves nothing behind.
- [x] Each project can have multiple binaries loaded.
- [x] Has an associated directory, set from the project view — a text box and a folder picker.
  Editing it writes `project.toml` at once, the way opening a binary does: a rename or a
  re-association is a deliberate user action, and it lets go of no binary, so it writes that
  file alone and leaves the session pending.
- [x] Can have multiple tabs with different function assemblies / source files open. Within
  a session: one strip holds them all, each tab being a place in a binary or a file. They are
  carried across a restart by the "saves the open tabs" item below.
- [x] Saves the open tabs. One ordered list in `session.toml`, of the same `SavedDocument` the
  history and the active document already use, so the reader's own interleaving of functions
  and files survives a restart. Coming back goes through the functions that hold the tab
  invariants rather than writing the list, and the ordering is load-bearing — the tabs are
  opened before the active one, or `activate` appends it at the end of the strip instead of
  finding it in place. An assembly-driven tab that no longer resolves is dropped, the way a
  history entry is; a source file that is no longer on disk still comes back, because the
  pane's own "Source file not found" is the right answer and dropping it would silently lose
  a file the reader had open.
- [x] Saves a viewing position per tab. Each open tab carries the row *each of its two sides*
  was left at, in memory and in `session.toml`, so switching to a tab puts both panes back
  where they were and a tab seen for the first time opens at the top. A row rather than a pixel offset, so a later
  change to the row height does not move every saved position, and a hint rather than a fact:
  it is clamped to what the tab holds now, so a rebuilt binary or a shortened file cannot come
  back past the end.
- [x] Saves hashes of the binaries, so a restore can tell the same file from one that has
  been rebuilt underneath it. An xxHash64 of the whole file, taken on the parse worker while
  the bytes are already in hand — 31 ms on the 331 MB sample, 2% of its open — and one hash per
  *file*, so an archive's 196 members share it. Size + mtime was rejected on correctness rather
  than cost: mtime is not a property of the bytes, so a deterministic rebuild reads as changed
  and a `cp -p` or a checkout reads as unchanged, which is the exact case this exists to catch.
  Under a binary whose digest no longer matches, the **name becomes the identity and the address
  only a tie-breaker** — a symbol that merely moved still resolves, but a name that names two
  symbols and no longer names an address resolves to neither, a stale address being precisely
  what lands a reader on a different function — and the saved viewing row is dropped, being a
  claim about a listing this build no longer has. No dialog and no refusal. A binary with no
  saved digest is a third state rather than a mismatch, so existing sessions load unchanged.
- [ ] Can have an LSP server (like rust-analyzer) / cargo integration so it can build for you and find binaries.
- [?] Maybe store LSP output in a more compact index given we expect source to not be modified?
- [x] A main view where you can see all project info: the project's name and directory, both
  editable, the id it is stored as, and every open binary with how many objects it contributed.
  It is a dockable view rather than a chip in the content strip, for the reason now written into
  `agents/UI.md`: a document there is a *place in a binary*, and a project is not one.
- [?] Snapshots of projects where binaries and source can be embedded (compressed?) and different versions of projects can be compared.
- [x] Split project storage into toml? for user given settings and another file for opened tabs /
  cached binary inspection data. Three files now: `settings.toml` for the user's own preferences,
  `projects/<id>/project.toml` for what the user *said* about a project (its name, its directory,
  its binaries) and `projects/<id>/session.toml` for what the app *noticed* (tabs, sources, the
  shown file, the selection, the history, the per-tab rows, the binary digests). The line is the
  one the save policy already drew: the first is written the moment the user acts, the second on
  the thirty-second flush — so the file worth keeping or hand-editing is exactly the one that
  changes only when they do something. A corrupt session then costs a scroll position rather
  than the list of binaries, and a binaries change writes both, so a session can never name a
  tab into a binary the project has let go of.
- [x] Save the navigation history.
- [x] Opening binary files saves immediately.
- [x] User project changes should save immediately. `project::record` writes at once when
  `binaries` differs and leaves everything else pending, so both changes the user can
  currently make to a project — opening a binary and closing one — are on disk before the
  next click. Anything Step 8's project model adds (a directory, a name, binary hashes) has
  to go through the same comparison to keep this true.
- [x] Periodically save if anything has changed. History, open files and view positions do not
  need to save immediately.

## Startup

- [x] Reopen previous open project on startup. `recents.toml` is an *order*, most recent first,
  and the project to reopen is its first entry rather than a field of its own — the order already
  answers that, and a second answer would be one to keep in step. The directories are what say
  which projects exist, so the list needs no pruning.
- [x] Have a view of recent projects if none was open — a section of the project view rather
  than a view of its own, since the recent list is how you *leave* the project the rest of the
  pane describes, and a separate tab would be empty in every session where a project was
  reopened. Each row is named from its own `project.toml` rather than from a name copied into
  the list, ids whose directory is gone are dropped, and the open project is left out because
  the pane above already describes it more freshly. Clicking a row switches project at runtime:
  the old one is flushed while the save policy still points at it, every binary and then every
  remaining tab is closed through the functions that hold the tab invariants, and the new one
  is restored through the same body the startup restore uses.

## Fonts and settings

- [x] Match system fonts / coding fonts by default, by runtime lookup. KDE / Gnome / Windows.
  `XDG_CURRENT_DESKTOP` only sorts the two Linux tools rather than choosing one — a tool that
  is not installed is already "the desktop said nothing" — so `kreadconfig` and `gsettings`
  are both tried, and Gnome's `text-scaling-factor` is applied to the point size because it
  is where Gnome puts "make text bigger". Windows goes through
  `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)` over a target-gated `windows-sys`, with no
  external process; its fixed-width half stays `Consolas`, Windows storing no desktop-wide
  monospace font to look up. Compile-checked for the Windows target, but only the decoding is
  tested — nothing here has been run on Windows.
- [x] Have a settings page where you can override (with a default being unspecified with clear
  visual distinction). A dockable view with the theme (light / dark / follow the desktop, naming
  which the desktop currently prefers) and, per font, a family box and a size stepper with a
  preview in the resolved font. An override is told from an inherited value three ways: the
  field's own label changes colour, the value is real text rather than a dim placeholder, and a
  Clear button exists only where there is something to clear. What the placeholder shows is
  `resolve(&Settings::default())`, so it is *by construction* the value that would be used
  rather than a second guess at it. Sizes are a stepper and not a text box deliberately: with
  settings applying live, a box means typing `1` on the way to `12` and getting a 1pt window,
  and a third "not a number" state that nothing else here has.
  Fonts became reactive the way colours did — asking for a font subscribes you to it — so a row's
  height now follows the font it is drawn in instead of being a constant. There are two heights,
  not one: the sidebar's rows are interface-font rows and the code panes' are fixed-width ones,
  and no row mixes the two, so a single height over the larger of the fonts meant raising the
  assembly font silently padded the sidebar. Each `item_size` comes from the height its own rows
  draw at, which is what keeps a scroll view and its rows from disagreeing; saved viewing
  positions are rows, so they survive a font change naming the same instruction.
- [ ] Move a settings file that will not parse aside instead of ignoring it, and say where it
  went. Today a stale or hand-broken file is silently ignored and the next write overwrites it,
  so the reader loses whatever was in it without ever being told — which is the one place
  "persisted formats need no backward compatibility" costs something real. The file goes into an
  `incompatible/` directory that **mirrors the structure it came from**, so a project's own
  files keep the shape they had rather than being flattened into one heap of `session.toml`s.
  Nothing there is ever overwritten: a name already taken gets a number prefix, and the claim
  has to be made the way an anonymous project's directory is — a create that fails rather than
  opens — since a second copy of the app may be moving the same file at the same moment. Then a
  popup naming every path written, because a rescue the reader never hears about is the same as
  no rescue. The decisions are which files this covers (`settings.toml` alone, or every
  persisted file — `project.toml`, `session.toml`, the recents list), and where the directory
  sits relative to each of them.

## Scratchpad

- [x] A scratchpad function which allows creating single file rust projects where you can build
  with cargo and view assembly output. The generated cargo package *is* the storage, so there is
  no second format to disagree with what cargo is handed, and the editor is freya's own
  `CodeEditor` — rejected for the read-only source pane because it paints a line background only
  for the cursor's row and keeps its scroll state private, both of which that pane needs and an
  editor you type in does not. Building runs off the UI thread and what it produces goes through
  the ordinary open path, so the scratchpad's functions appear in the content strip like any
  other binary's. One scratchpad for now, with no picker: the model holds many, but a picker is a
  second document list.
- [x] Let a scratchpad depend on crates.io crates, as a **list of crates edited in the UI** —
  name and required version per row — rather than as a convention inside the source. Every bad
  row is marked in place, on the half of the row that is wrong, with the reason under it; a
  wildcard is refused, a version being required so a scratchpad builds the same way twice. A
  build that cargo rejects *before compiling anything* is shown against the rows rather than
  searched for a crate name: the dependency list is the only part of the generated package this
  pane can get wrong, so no compiler diagnostics means the rows are where the answer is.
- [x] Allow these files to run with output viewable. The built artifact is spawned directly
  rather than through `cargo run`: the build already asked cargo where it put the executable, and
  re-entering cargo would redo resolution, interleave its own progress lines into what the reader
  is reading as their program's output, and leave a killed `cargo run` with a child nobody holds.
  Output streams line by line while the program is going, stdout and stderr told apart by colour,
  and Stop is a real kill rather than a dropped handle — `Child`'s `Drop` neither waits nor
  kills. A run is also ended by a rebuild (cargo is about to overwrite the file the process *is*)
  and by closing the window; an edit ends nothing, a run being of an executable rather than of
  the buffer. Bounded three ways, each a different failure: a line with no newline is cut rather
  than accumulated, the oldest lines are dropped past a cap and the pane says how many, and the
  channel is bounded so backpressure reaches the program itself.
- [x] Follow the newest line of a running scratchpad's output. Arriving lines keep the pane
  pinned to the bottom while the reader is at the bottom, a wheel away from there releases the
  follow and leaves them where they are, and coming back to the bottom arms it again. Being at
  the bottom is judged in rows against the viewport as it is now — `reveal_row`'s shape — and
  never as a remembered row: past the output cap the oldest rows drop off the front and every
  index shifts. A line arriving is not an occasion to re-judge, the newest row being below the
  viewport by definition; the pane is keyed on the pad, so one pad's scrolling never breaks
  another's follow.
- [?] Use freya's tty for the scratchpad's output, in place of the list of coloured rows the
  run pane draws: a terminal would carry a program's own colours, cursor movement and
  progress bars, where the rows keep only which stream a line came from.
- [x] Wrap or scroll the compiler output, wrap or scroll being decided by the list a line is in
  rather than by the line. The diagnostics and cargo's own stderr are a plain `ScrollView` of
  wrapping paragraphs — a build says dozens of things, so there is nothing to virtualise away, and
  once nothing is virtual a block may be as tall as its text needs; the `--> src/main.rs:9:17` a
  span carries, which was the widest line rustc writes and so the first thing the pane's right
  edge took, now wraps. The run output stays a `VirtualScrollView` stepping by one `item_size`,
  being bounded at nothing smaller than `MAX_OUTPUT_LINES`, and takes a sideways scroll instead:
  a virtual list has to know a row's height before it has built one, which is exactly what a
  wrapped row cannot tell it. Each costs one honest thing — a wrapped caret can sit under the
  wrong character, though only on a line clipping would have cut outright, and the output scrolls
  only as wide as the widest row it has drawn.
- [x] Click a diagnostic to reach the code it is about. The place under the message is a target
  now, and pressing it puts the editor's cursor on the line and the column rustc named. The
  conversion between the two ways of counting is one pure function of the source text,
  `Span::offset_in`, and is unit-tested rather than eyeballed: rustc counts from one and counts a
  column in characters, a cursor is UTF-16 code units from the start of the text, and lines are
  `\n` and nothing else. It is applied to the buffer as it is *now* rather than to what was
  compiled, the reader having usually typed since, so a column past its line's end lands at that
  end and a line past the file's end at the file's end — clamped, never out of range. Only a span
  in the pad's own `src/main.rs` is a target: cargo names a dependency's file just as readily and
  there is nowhere to put a cursor in one, so those keep the plain label, no hover and no press,
  a promised press that did nothing being the worse answer. The cue is the relocation link's own
  — its wash and its pointer — and the colour a place already wears. What it does is put the
  cursor there; bringing the line *on screen* is the half below.
- [D] Scroll the editor to the line a pressed diagnostic names. The cursor lands on it and the
  row is marked — the editor's own current-line background, and its gutter number lit — but a
  line that was off screen stays off screen, so a reader with an error below the fold is told
  where it is without being shown it. Deferred on freya rather than on taste: 0.4.3's
  `CodeEditor` keeps its scroll in `CodeEditorData.scrolls`, `pub(crate)`, with no
  `new_controlled` and no controller to hand in, and it scrolls to its own cursor nowhere in the
  crate — so nothing outside it can move that view. This is the same objection already written
  down for not using `CodeEditor` in the read-only source pane, and the paragraph there claiming
  "nothing here wants to scroll it from elsewhere" is what this invalidated. The one way to buy
  it is to give the editor its content's full height inside a `ScrollView` of ours, which
  de-virtualises it — every line of a pasted file built on every render — and that is too much
  for a reveal. Undeferred by freya exposing the scroll, or by the editor scrolling to its own
  cursor.
- [ ] Draw the compiler's error once, not twice. A diagnostic block is a header — the level
  word, the place, and `Diagnostic::message`, the one sentence rustc wrote — followed by
  `Diagnostic::rendered`, which is cargo's own block and *opens with that same sentence and that
  same place*. So every error in the pane says itself twice, and the taller the block the more
  obviously. Which copy goes is the decision. Keeping the header and dropping `rendered` loses
  the caret, the source excerpt and the `help:` lines, which are most of what makes a rustc
  error readable; keeping `rendered` and dropping the header loses the level's colour, the
  wrapping the header does, and — the one that is not cosmetic — the press target, since
  `SpanTarget` hangs off the header's place and the `-->` inside the rendered block is text in a
  paragraph rather than an element of its own. A third answer is to draw neither and build the
  block from the JSON's own `spans` and `children`, which is the only one that ends with the app
  deciding what an error looks like rather than reprinting what cargo decided.
  And can it be coloured? Two ways, neither free. `Scratchpad::build_in` passes
  `--color=never` today; `--color=always` would make `rendered` carry ANSI SGR escapes, which
  means a parser turning them into spans and a mapping from rustc's eight colours onto the
  palette's — the app's own colours being the rule everywhere else, and a terminal's red on a
  themed pane not being one of them. Rendering from the JSON instead makes the colour the app's
  by construction and costs the caret line, which would have to be drawn rather than reprinted.
  Sequenced after this, since what is coloured depends on which copy survives.
- [x] Scratchpads are a concept of their own, disjoint from projects, and there are many of them
  saved. Each is its own directory under `scratchpads/` the way each project is one under
  `projects/`, and it is filed under an **id the reader never sees** — the directory, the crate
  and the order file all say the same `PadId`, checked the way a `ProjectId` is because it is
  interpolated into a path and read back out of files a user can edit. What the reader deals in
  is a **name**, a value in the pad's own package under `[package.metadata]`, so it may be
  empty, hold spaces or repeat another pad's, and renaming is a keystroke the ordinary save
  writes out rather than a directory move that has to claim a target and refuse a collision. A pad
  nobody has named has an empty one, and the pane calls it `<pad-3>` — the id in the brackets that
  already mark `<entry point>` as the app's word rather than the file's.
  Which pad opens is the front of an order beside the pads, `recents.toml`'s rules exactly: an
  order and not an index of what exists, `touch` answering whether anything moved, nothing
  pruning itself on load. The *list* is the union of that order and the pad directories it does
  not name, each row's name read from that pad's own package, since a reader picks a pad from it
  and one that fell off the end has to stay reachable. The list lives in the **Scratchpad view's
  own side panel**, the content area's strip deliberately not being the place for a second
  document list. A new pad is the first free `pad-N`, claimed by a `create_dir` that fails rather
  than opens and written at once. **Runs are per pad**: several can be going at once,
  leaving a pad stops nothing, and switching only switches which output is on screen. So is the
  editor buffer, so a pad comes back with the cursor and the undo history it was left with.
  Deleting a pad is deliberately absent — it is the one operation here that destroys a reader's
  source, so it waits until it is asked for.
- [x] Stop a run's grandchildren with it. A run is started in a process group of its own, so Stop
  kills the program *and* everything it forked rather than only the process the app holds a handle
  for — a grandchild's pid was never anywhere but inside the program that is now gone. One idea
  with two implementations: `process_group(0)` before the spawn and `kill(-pgid, SIGKILL)` in the
  stop on Unix, and a kill-on-close job object the child is assigned to on Windows, where closing
  the app's only handle is the kill. The child's own kill stays behind the group's, as what a job
  the system refuses still gets. Nothing else moved: the reap is the same `try_wait` on a poll —
  the group changes who dies, not how the end is noticed — and the rebuild, the next run,
  `stop_all` and the window closing all go through the same stop and inherit it.

## Binary inspection design

- [x] Can explore while binary is processed to find all functions. Objects reach the sidebar as
  they parse rather than all at once at the end, so a 196-member archive offers its first member
  at 102 ms where the window used to show nothing for 685 ms, and everything already parsed is
  explorable while the rest arrives. A single-object file gains no time — there is one answer and
  it arrives when it arrives — but its row is on screen from the start rather than nothing being
  there.
- [x] Rely on declared functions in binaries. Don't assume things can be code — only declared
  text symbols are disassembled, nothing is scanned for.
- [x] Rely on debug info: DWARF line info is read (lazily, per section).
- [x] Rely on debug info for function extents too, rather than estimating from the next symbol's
  address (`estimate_size`), since declared sizes are often 0 in COFF/ELF. `SymbolData::extent`
  takes a `DW_TAG_subprogram`'s `DW_AT_low_pc`/`DW_AT_high_pc` where there is one, and the
  *smaller* of it and the estimate: the estimate over-reaches into padding, but `high_pc`
  describes the function, so an alias or a split cold part inside one subprogram would otherwise
  swallow the next function. It stays lazy, behind the same cache, bias and `catch_unwind` the
  line rows go through. On the `analysis` crate's own rlib 3 704 of 3 705 symbols answer from DWARF;
  on the app's own binary half do, the rest being C++ dependencies built without it.
- [x] Take entry points and exported DLL / dylib functions as symbols too. `declared_code`
  reads `dynamic_symbols`, `exports` and `entry` for a **linked image** — all declared, so
  nothing is guessed at or scanned for — and a prebuilt LLVM DLL goes from zero functions to
  22 918. One symbol per address, earliest source winning, since a repeated address makes the
  sorted list `estimate_size` searches answer 0; the section is found by looking the address up
  in the kept text sections, which is also what keeps exported *data* out. A relocatable object
  is skipped, 0 being a real function's first byte there rather than "no entry point".
- [?] A multi-threaded, **deterministic** analysis pass that finds all the code: labels,
  functions, jump targets, entry points, exports and the rest. Deterministic in the strong
  sense — the same binary gives the same set of code locations, in the same order, whatever the
  thread scheduling did, so a result can be cached, compared between runs and saved into project
  info without ever being subtly different from the one before. Undecided because it sits
  against the standing rule above ("rely on declared functions, don't assume things can be
  code"): a jump-target sweep *discovers* code rather than reading a declaration, which is the
  thing that rule exists to forbid. Deciding it means deciding how far that rule bends — a
  target reached from an already-declared function is arguably still declared, transitively,
  while scanning a section for instruction-looking bytes is not. The pieces that are already
  declarations (entry points, exports, unwind targets in `.pdata`) do not need this and are
  their own items.

- [ ] Find unwind targets. Now also the honest fix for the other half of the above: a stripped
  PE's export table is sparse, so an exported function's extent is derived across every
  unexported function between it and the next export — megabytes, in nine cases in the DLL, and
  3.7 MB in the worst. `estimate_size` is capped at 1 MiB to stop that costing seconds per
  redraw, but an x86-64 image carries a `RUNTIME_FUNCTION` in `.pdata` stating both ends of
  every function with unwind info, which would make the gaps the cap exists for stop existing.
- [ ] PDB line info, through the `pdb` crate: a linked PE names its `.pdb` in the debug
  directory's CodeView record, so an `.exe`/`.dll` built with debug info gets a source view.
  DWARF is read today and the PE sample has no debug sections at all.
- [?] CodeView embedded in COFF (`.debug$S`/`.debug$T`), which is what a rustc `.rlib` member
  carries — a different container from a `.pdb` file and likely hand parsing, so it stays
  undecided on its own.
- [x] Binary inspection is off the UI thread. Disassembly, line info and the arrow gutter's
  lane layout moved together onto one worker for the app's lifetime, which drains its queue to
  the newest request so the clicks a reader passed are dropped before being started rather than
  pushed through the most expensive call in the crate. An answer carries the symbol it is about
  and is kept only if that symbol is still selected, so a stale answer is discarded by a
  comparison rather than by a counter. The first symbol clicked on the app's own binary cost 589 ms
  on the UI thread and now costs a channel send.
- [x] Binary inspection uses more than one core, which is the whole of what demangling had left
  to give. `demangle::batch` hands one object's names to a process-wide pool of threads with
  64 MiB stacks — `available_parallelism` capped at 8, started on the first batch that needs one
  and then kept — and they take the batch in grains of 256 names off one atomic cursor. A grain
  is handed back with the index it started at and written there, so the answer is the batch's
  own order whatever the scheduling did and two runs over one file agree. Grains are pulled as a
  thread frees rather than dealt out up front, because a name's cost is superlinear in its
  length and where the long ones sit is the file's business. Two costs went at once: the
  demangling itself, and the 64 MiB thread that used to be created and joined once per object —
  237 of them for the crate's own rlib. Release, best of 3, on a machine with five other agents
  building on it: the app's own debug binary 1 701 → 1 414 ms, the rlib 383 → 151 ms. Streaming
  is untouched, objects still reaching the sidebar as they parse and in file order. No
  dependency was added for it: `rayon` is in the lock already, under `image`, but its threads
  would still have to be a pool of this crate's own to get the stack size — which is the whole
  of what is hard here — and what is left over is a queue and a cursor.
- [ ] Parse an archive's members on more than one core as well. The second of the two levels,
  and the one still sequential: `open_files_streaming` walks a file's members in order on one
  thread, so the read, the section decompression and the symbol-table pass of the rlib's 237
  members happen one after another even though the demangling inside each no longer does. What
  it needs beyond the pool that now exists is a reorder buffer, since members have to be emitted
  in file order for the Objects tree to be the same tree twice, and a rule for what a `Break`
  from the caller means once members are in flight.
- [ ] Hand an object to the UI as soon as its names are demangled, and build the source →
  assembly mapping behind it. The two halves of an open are not wanted at the same moment: the
  symbol list is what the reader is waiting for, while `SourceIndex` — a file and a line out to
  the symbols compiled from them — is what the first "find all locations" and the first click
  in a source-driven tab ask for. It is built on the first ask today (`OnceLock::get_or_init` in
  `line.rs`, deliberately never at parse time), so that click pays 2.2 s on this app's own
  binary, of which 2.0 s is the extent pass. That is on the worker and not on the UI thread, so
  nothing is blocked — what is wrong is that it is seconds late every first time, and the wait
  is spent after the reader has asked rather than while they were reading the symbol list.
  Four things to decide before starting. Whether it runs on the demangling pool now that there
  is one or on a queue of its own — it is one long job per object rather than a batch of short
  ones, so it wants a thread it can be left running on rather than a grain. What an ask arriving
  mid-build does, where `OnceLock` makes the caller wait for the whole thing and the honest
  alternatives are answering the slow way or answering "not yet". Whether every open object
  gets one or only the ones the reader has touched, since the index is 10 MB and holding the
  line programs its walk parsed takes the process from 756 MB to 1.23 GB — which is the whole
  argument for its being lazy today, and the reason this is a change of *when* rather than of
  *whether*. And what a binary closed mid-build does with the build still running over it.
- [D] Cache the demangled names between runs. **Built, measured and then thrown away** — it is
  not in this repo's history, deliberately, so this item is the whole record of it. The gain did
  not justify what it cost to carry. What it bought, per file, release, cold open
  against warm: the app's own binary 1 589 → 1 174 ms (-26%), the `analysis` rlib 111 → 31 ms
  (-72%), the LLVM DLL 542 → 479 ms (-12%), a 3.5 MB CodeView rlib 5 → 1 ms (too
  small to mean anything). **An average of roughly 37% across the three samples big enough to
  measure**, and about 25% weighted by the time actually spent rather than by file.
  Against that: a fourth place on disk outside both the project and the settings, an eviction
  budget, a hand-rolled binary format with its own version and checksum (TOML cannot express an
  `Option<String>`), and two defects the format's own corruption sweep found — a short document
  silently truncating the symbol table, and a well-formed one able to hold a wrong name. That is
  a lot of machinery, and a wrong function name on screen is a bad failure, for a second and a
  half on the largest sample here.
  Undeferred by the open getting slower or the saving getting larger — parallel demangling above
  was the thing to try first and has since taken that cost down without persisting anything, so
  what is left to buy here is smaller than the numbers above.
- [D] Cache inspection results in the project info. Neither `assembly()` nor `line_info()`
  memoizes, so leaving a listing and coming back re-derives it — 4–8 ms on the app's own binary,
  which is cheap enough that a keyed cache was deliberately not added on the way past: it would
  be an unbounded pile of `Assembly`s for listings the reader has left. Deferred at the
  Goals → Steps split: a persisted cache would carry everything that killed the
  demangled-names cache below — a format with a checksum, an eviction budget, a corruption
  sweep — to save milliseconds per click where that cache saved 26–72% of open time. If
  anything is ever cached here it is small derived metadata (a file → symbol index,
  subprogram extents), never listings.
- [x] Binary inspection should be designed to be portable, allowing different disassembly
  libraries to be used. `disasm.rs` holds `Assembly`/`Instruction`/`SpanKind`/`BranchEdge` and
  names no backend; `disasm/x86.rs` is the only `iced-x86` in the crate. The trait is one call
  wide and shaped by what `Assembly` needs — bytes, an address, and one question per
  instruction about whether a relocation covers it — so a second backend is an impl and not a
  rewrite. The four x86 spellings that have no cross-architecture answer live behind it: the
  `SymbolResolver` substitution, the per-instruction `rip+` flip, the flow-control judgement
  and the `FormatterTextKind` mapping. Only iced-x86 sits behind the seam today, which is the
  point rather than a gap.
- [x] Make the architecture-specific analysis **generic rather than dynamically dispatched**.
  `disassembler(architecture) -> Option<Box<dyn Disassembler>>` is gone: `Assembly::decode` is
  the one `match` over the architecture, each arm naming a concrete backend, and
  `Assembly::decoded` — the backend's call and the branch resolution together, which is the
  whole decode path — is generic over it and compiled once per backend. Nothing is boxed and
  no signature says `dyn`, so every type here stays nameable by a caller. What it buys is not
  the virtual call — one per symbol is nothing — but the allocation going away and the
  backend's formatting and span-mapping becoming inlinable into the decode loop, which *is*
  per instruction. The trait stays as the shape a backend is written to; the trade taken is
  that a new architecture is a new arm rather than a new impl behind a registry, the set being
  closed at compile time either way. Behaviour is unchanged, held by the existing tests.
- [x] Decode as the architecture the object declares, rather than as x86-64 whatever it is.
  `Object::architecture` comes from the file, and the bitness comes from the architecture and
  not from `is_64()` — x32 is 64-bit code with 32-bit pointers, the one case the file's class
  gets backwards. An architecture no backend claims is now a third answer (`Assembly::
  undecodable`) rather than a confident page of nonsense.
- [x] Say so in the UI when an architecture cannot be decoded. The assembly pane reads
  `Assembly::undecodable` and says "No disassembler for aarch64" rather than drawing an empty
  pane the reader has to guess about — an empty listing being indistinguishable from an empty
  function.
- [x] Should never panic on any file input. Errors doing analysis should allow inspecting
  functions without errors. Searched for rather than asserted: a seeded, bounded mutation sweep
  (`tests/mutations.rs`) truncates every corpus file at every length, poisons every count, offset
  and size in the ELF/PE headers and tables, and splats bytes, then asks each result everything
  the app asks. It found a stack overflow in the demanglers that **aborted** the process — which
  `catch_unwind` cannot catch — and one corrupt symbol address that cost its neighbour its
  listing. Both fixed, each with a fixture of its own, since the sweep is the searcher and the
  fixture is the regression test. Two `addr2line` arithmetic bugs found with them: one is now
  declined before the call, one stays wrapped.
- [D] Don't run by default, make that opt-in as needed. Measured and declined, with the numbers
  in `agents/UI.md`: everything expensive is *already* deferred — line info and the DWARF context to
  the first query, subprogram extents behind the same cache, disassembly to selection. What is
  left at open time is reading the bytes and walking the symbol table, which is precisely what
  the Objects and Symbols lists *are*, so there is nothing to opt out of without opting out of
  the app. The only remaining lever is deferring demangling (21% of the app's own binary, 47% of the
  `analysis` rlib in release), and it only defers to the first click on that object while costing a second
  async stage feeding the symbol list. Undeferred by a future eager pass — the code-discovery
  goal above would be one.
- [D] Prefer memory mapped files and minimal memory footprint, store locations into the mapped
  file? Deferred: a mapped file rebuilt underneath the map is a `SIGBUS` on a page fault, and a
  rebuild under the app is the scratchpad's ordinary cycle — the reason the digests exist — so
  a sound design needs that answered before it saves a byte; the decompressed and relocated
  sections cannot be mapped regardless; and nothing measured says memory is a problem yet.
- [?] How to design an index to allow source files / assembly to map, without large memory footprint.
- [x] Have this in its own crate. Use it for test cases.

---

Maintain this file when a feature is requested.

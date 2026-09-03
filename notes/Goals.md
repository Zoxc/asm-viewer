# Goals

Inspecting and comparing binaries.

`- [ ]` is a goal that is not done yet, `- [x]` one that is, `- [?]` one that is only a maybe —
not decided on yet — and `- [D]` one that is deferred, with the reason why on the item.

Only check an item off when all of it is done. A goal that is only partly done gets split into
one item per part, so the unfinished half stays visible.

A goal that is done can be moved into `notes/specs/`, written there as the rule it settled; it
leaves this list when it is. That is a move made on request, like everything else about specs
(`notes/specs/README.md`).

## Source / assembly split view

- [ ] Mark, in the Source pane's line-number gutter, the lines that have assembly: a reader
  scanning a file should be able to tell at a glance which lines the symbol on the other side
  was compiled from and which produced nothing, without hovering each one. The answer is the
  symbol's own line info, which `SymbolLines` already holds and `AsmData::position` already
  reads the other way round, so the gutter needs the set of lines the rows name and a mark
  beside each, in a colour the palette gets a field for.
- [ ] The same the other way: mark, in the Assembly pane, the instructions that have a source
  line, so a reader can tell the rows the debug info places somewhere from the ones it places
  nowhere — a prologue, padding, an inlined stretch from a file the pane is not showing — without
  hovering each. `AsmData::position` is already the question per row; what is missing is a mark
  in the row for a `Some`, in the same colour as the source gutter's, and a decision about
  where in an instruction row it goes, the gutter there being the branch arrows' already.
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

- [ ] Clicking on functions in source should navigate to them. A click on a source line moves
  the assembly pane to that line's instructions; a click on a *call* in the source still does
  nothing, since nothing maps a source identifier to the symbol it names. Sequenced after
  the LSP item under *Projects*: rust-analyzer is what should answer which symbol an
  identifier names, rather than building name matching over demangled strings here.
- [ ] A file search dialog on Ctrl+P: type part of a path and open the file it names, the
  way an editor's quick-open does.

## Assembly viewer

- [ ] Fix the `<entry point>` tab name: the shortening in `naming.rs` reads a name as a
  demangled path with generic arguments, and the angle brackets the app itself puts around
  its two made-up names — `<entry point>`, and `<function 0x…>` for a function only an
  unwind entry declares — are not that. A tab for either should say the name as it is.
- [ ] Ctrl+C copies whatever is selected, wherever it is: the run of rows in either pane
  today, the characters once the goal above lands, and the other places text is picked out
  -- a filter box, the scratchpad's editor, its diagnostics and its output -- so the one
  binding has one meaning and never comes back with a page of disassembly for a word picked
  out somewhere else. The key handlers are per pane on purpose (`agents/Panes.md`); this is
  the rule that decides which of them answers, not a global handler.

- [ ] Drop the `<` `>` around a symbol's label in the unified view. A stretch's label is
  drawn `<name>:` (`src/ui/section_view.rs`, in the rows and in what is copied), objdump's
  spelling, and the brackets say nothing the colon and the row's own colour do not; a demangled
  Rust or C++ name is full of angle brackets of its own, so a pair more around it reads as
  part of the name. Two things to keep straight: the two made-up names the app itself puts in
  angle brackets, `<entry point>` and `<function 0x…>`, must still read as made up once the
  label's own pair is gone, and the headless tests find labels by their drawn text.
- [ ] Show the current symbol as a breadcrumb in the unified view. The bar over the pane names
  the object for a code tab, and nothing on screen says which function the rows under the
  pointer, or at the top of the pane, belong to once its label has scrolled off — a reader
  three screens into a long function is reading unnamed code. The stretch the top row is in is
  already what the kept place is worked out from (`section::Rows::row`), so the bar could name
  that symbol beside the object, object › symbol, and pressing it would be the "Open as symbol"
  door in one more place. Whether it follows the top row or the pointer is the one decision:
  the top row is stable and the pointer lights nothing.
- [ ] Navigate to an address in the unified view: type one and the listing goes to the row
  at that address, in the object whose code is on screen. Today the only way to a place in the
  section view is a label, a symbol's "Show in unified view" or the kept place a tab came back
  to, so an address read off a crash, a linker map or a debugger has no door. The place the
  view keeps is already an address (`CodeAt`, `Spot`), so landing is the write that hook
  answers; what is undecided is where the address is typed -- a box in the bar over the pane,
  or a Ctrl+G dialog -- whether it is the placed address the listing draws or the object's
  own, and what happens to one that falls between rows or outside every section.
- [ ] Let a call target with no symbol be opened, where a relocation names it as a section and
  an addend. `Code::relocation` answers a `Relocated` whose `target` is `None` whenever the
  relocation points at something the object has no text symbol for — a section symbol with an
  addend (`.text+0x40`), a data symbol, an undefined import — and the operand is then drawn as
  the placeholder the linker will overwrite, with nothing to click. The section-plus-addend
  case is an address the object could compute, but the parse keeps no section symbols and
  reads no relocation kind, both of which the sum depends on (`S + A` against a PC-relative
  `P`); with those read it would be one more `target` and the door above would serve it. An
  undefined import has no address at all and stays plain text whatever is read.

## UI

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
- [ ] Show a row's tooltip only when the row is actually cut off. A tooltip repeating a name
  that is already fully on screen is noise the pointer drags behind it across a whole list —
  and with the zero delay above it arrives the instant the pointer crosses a row. What it
  needs is a comparison the app cannot presently make: freya reports a laid-out box but not
  the width the text *wanted*, so "does it fit" has to come from somewhere — a measured
  `longest_line()` on a row allowed its natural width, or the ellipsis being detectable, or
  the row reporting its own overflow. Settling that is the goal; the rule once it is settled
  is one line per list, and the same question decides it for the tab bar, whose tabs elide
  too.
- [ ] A bookmark control on the symbol bar, for the symbol being read, beside the two menus
  above.
- [ ] Only close the assembly view by default in a source-driven tab if its file is not in a
  compiled language: a `Cargo.toml` or a `.json` opens with the source side alone, a `.rs` or
  `.c` with both, as now.
- [ ] Make that search reachable and ranked: no keyboard shortcut puts the caret in the filter
  box, and matches come back in the list's own name order rather than by how well they match.
- [ ] Left panel for project directory / source search.
- [ ] Refactor the tabs away from freya's dock panels and onto components of the app's own,
  with a fixed panel for the tabs rather than one the reader can fold, split or drag documents
  out of.
- [ ] Let the views close — Project, Settings, the Scratchpad and the rest — and add a menu at
  the top left of the window to reopen them. Today a view has no × because there is no way back
  once it is closed; the menu is that way back, so the × can come.
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

## Projects

- [ ] Can have an LSP server (like rust-analyzer) / cargo integration so it can build for you and find binaries.
- [?] Maybe store LSP output in a more compact index given we expect source to not be modified?
- [?] Snapshots of projects where binaries and source can be embedded (compressed?) and different versions of projects can be compared.

## Startup


## Fonts and settings

- [ ] A syntax-highlighting sample on the settings page, with the colours chosen from it. A
  block of source and a few lines of assembly drawn in the current palette -- a keyword, a
  type, a call, a string, a comment, an attribute; a mnemonic, a register, an immediate, an
  address, a relocation name -- beside the theme choice, so the reader sees what a theme
  does before reading in it. Each span is a colour the reader can change: pressing one opens
  a picker for that palette field (the `Palette` fields `Palette::syntax` and `kind_color`
  map the categories onto), and the choice is an override per theme in `settings.toml`, told
  from the palette's own value and cleared the way a font override is
  (`agents/Appearance.md`). What it costs: the source pane's colours are baked into a
  `SyntaxBlocks` when a file is loaded, so a colour change has to re-parse every cached file
  as a theme switch already does; and the contrast test that holds every foreground to a
  floor cannot hold an override, so the picker wants to say when a colour falls under it.
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

- [?] Use freya's tty for the scratchpad's output, in place of the list of coloured rows the
  run pane draws: a terminal would carry a program's own colours, cursor movement and
  progress bars, where the rows keep only which stream a line came from.
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
- [ ] Delete a scratchpad, from its row in the side panel, behind a confirmation: it is the one
  operation here that destroys a reader's source, so it is asked for and not one click.

## Binary inspection design

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

- [?] Name a **funclet** after its parent. LLVM's and MSVC's word for an outlined `catch`,
  cleanup, `filter` or `finally` body: a separate body with a prologue of its own, and so a
  primary unwind entry of its own, which looks like a small function to the table and is a
  `<function 0x…>` today — 99 851 cleanup funclets and 774 catch funclets in `rustc_driver`,
  nearly half of its entries and most of its 118 209 nameless symbols, since its PDB does not
  name Rust's cleanup funclets either. The entry itself cannot say: LLVM emits no handler for
  a cleanup funclet at all (a `FIXME` in `WinException::beginFunclet`), and a catch funclet's
  only tell is the handler data it shares with its parent. The parent's handler data does
  list them all — `__CxxFrameHandler3`'s `FuncInfo` (identifiable by its `0x1993052x` magic;
  its unwind map's actions are the cleanup funclets, its try blocks' handlers the catch
  funclets), `__C_specific_handler`'s scope table — and every one of the 100 625 it lists in
  `rustc_driver` is already an entry begin, so this is classification, not discovery:
  `<cleanup of 0x…>`, `<catch of 0x…>`, and the parent's symbol bar listing them. It is the
  C runtime's format rather than the image's, one per handler and versioned
  (`__CxxFrameHandler4` compresses it), and the handler is local CRT code in both sample
  DLLs, so it cannot be told by an import name; the magic would be the gate. Undecided
  whether reading a runtime's private tables is within the "nothing is scanned for" rule,
  or worth it for a name that is mostly `<cleanup of …>`.
- [ ] Take a nonzero `st_size` before the DWARF extent walk. The `.eh_frame` measurement
  says `st_size` was exact for every one of `librustc_driver.so`'s 197 375 functions, while
  `SymbolData::extent` never reads it: an ELF with a symbol table and no `.eh_frame` (built
  `-fno-asynchronous-unwind-tables`, or a Mach-O) still pays the DIE walk for an answer its
  symbol table states. The estimate's clamp to the next symbol would apply as it does to an
  unwind entry's end; what has to be settled is a COFF or assembler `st_size` that is wrong
  rather than 0, which is what the "declared sizes are frequently 0" rule never had to face.
- [?] Exception handlers as targets, both formats. The unwind data names what runs on an
  exception: on ELF the CIE's personality (`rust_eh_personality`, through a `DW.ref.` GOT
  slot, in 45 008 of `librustc_driver.so`'s 172 169 FDEs; `__gxx_personality_v0` for C++)
  and each FDE's LSDA in `.gcc_except_table` (2.4 MB here), whose call-site table says, per
  call, where control lands — a **landing pad inside the same function**, not a funclet,
  since the Itanium ABI has none — and whose action and type tables say what is caught; on
  PE the handler and its `FuncInfo`, the funclet goal above. The viewer's use would be the
  arrow gutter: a call's landing pad drawn as an edge the way a branch is, and a row marked
  as one, which is the one thing about a function's control flow the listing cannot show
  today. The LSDA is the C++ ABI's format rather than the file's, shared by GCC, Clang and
  rustc and stable, where PE's is one C runtime's; whether reading either sits inside the
  "nothing is scanned for" rule is the same question as the funclet goal's, and the two
  should be decided together.
- [ ] Make PDB private symbols informative only. The module streams' `S_GPROC32`/`S_LPROC32`
  records — the PDB's *private* symbols, as against the linker's publics — are today a source
  of symbols in their own right, fifth in `declared_code`'s order, and their names are the
  compiler's display names (`core::ptr::drop_in_place<T>`), which no demangler claims and
  `naming.rs` then shortens as if they were demangled output. The publics carry the linker's
  decorated names, which the demangler reads as it reads an ELF's. So: the Symbols list is
  built from what the image and the publics name, and a procedure only *informs* the symbol at
  its address — its length as the declared size and extent (`debug_extent` already), its
  display name shown in the symbol bar's info section beside the demangled one — rather than
  standing as a symbol itself.
- [ ] Map a PDB's source paths onto this machine: the names come out verbatim (`C:\...` from
  MSVC, `/rustc/<hash>\library\...` from rustc), so the Source pane cannot open them where the
  build was elsewhere. A root-to-root mapping saved with the project, with the recorded
  checksum (`LineInfo::hash_of`) deciding among candidates rather than the name alone.
- [ ] Check the source hash for DWARF too: carry DWARF 5's `DW_LNCT_MD5` the way the PDB's
  checksum is carried (`LineInfo::hash_of`), so the Source pane's "this file differs from the
  one the binary was built from" applies to an ELF or Mach-O as it does to a PE. clang records
  the MD5, gcc does not; `addr2line` 0.21 renders a file name without handing its entry back,
  so this means rendering the name from `gimli`'s own `FileEntry` the way `addr2line` does.
- [?] CodeView embedded in COFF (`.debug$S`/`.debug$T`), which is what a rustc `.rlib` member
  carries — a different container from a `.pdb` file and likely hand parsing, so it stays
  undecided on its own.
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
  `line.rs`, deliberately never at parse time), so that click pays 0.43 s on this app's own
  binary — 2.2 s before its `.eh_frame` stated the extents, 2.0 s of which was the extent
  pass. That is on the worker and not on the UI thread, so
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
- [ ] Catch a panic in any optional pass of the parse, so the object still loads without it.
  `parse_object` is the symbol table plus what is declared elsewhere, and each of those extras
  is a dependency's parser over file-controlled bytes: the PDB opened at parse for its
  procedures and publics (`pdb2`), the unwind table (`.pdata` by hand, `.eh_frame` through
  `gimli`'s CFI reader), the demangling batch. The line-info seam already wraps every backend
  call in `without_panicking`, and the demanglers run under `catch_unwind` on stacks of their
  own; the unwind reader and the PDB's eager open do not, so a panic in `gimli` or `pdb2` there
  takes the whole `Object` with it — and the answer for the reader is the file not appearing
  at all, where it could appear as its symbol table alone. One guard per pass, each declining
  to its "nothing declared" answer, under the rule that stands: checked arithmetic first and
  the guard for a dependency's bug, never for ours; a stack overflow still aborts and is
  bounded before the call, as the demanglers' is. Whether a caught panic should be *said* —
  an object shown with a note that its PDB or unwind table was not read — is the UI's half.
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

---

Maintain this file when a feature is requested.

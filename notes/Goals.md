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
- [ ] Have a function to find all source / assembly locations that match, producing a list on the
  other side. Needs cross-symbol search and a panel to put the result in, which is Step 6's
  tabs and left panels.
- [ ] A function to pick the generic instance of a source function. Same prerequisite: one
  source function is many symbols, and nothing yet lists them.
- [x] An active navigation function where selection on one side moves the other side to the
  matching place — within one symbol. Clicking a source line scrolls the assembly to the
  first instruction it produced, clicking an instruction scrolls the source to its line, and
  neither is a navigation: the selection does not change and nothing is pushed onto the
  history.
- [ ] The same across symbols, preferring recent history when a source line maps into several.
  Needs the cross-symbol search above.
- [x] Syntax highlighting for both sides. Assembly is coloured by span kind; source is
  tree-sitter, through the highlighter `freya-code-editor` exposes publicly.
- [x] Use the assembly colour palette for the source side's syntax highlighting, so the two
  panes read as one view rather than as two themes. `EditorSyntaxTheme` is built from the
  app's own `Palette` now: keywords and types take the mnemonic's purple, variables and
  fields the register's olive, literals the immediate's blue, function and module names the
  relocation target's near-black, punctuation the rest's grey, and comments and strings are
  two new entries of the palette's own.
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

## Navigation

- [x] Clicking on functions in assembly should navigate to them.
- [ ] Clicking on functions in source should navigate to them. A click on a source line moves
  the assembly pane to that line's instructions; a click on a *call* in the source still does
  nothing, since nothing maps a source identifier to the symbol it names. Sequenced after
  the LSP item under *Projects*: rust-analyzer is what should answer which symbol an
  identifier names, rather than building name matching over demangled strings here.
- [x] Navigating in assembly should also navigate source, within a symbol: clicking an
  instruction scrolls the source pane to the line it was compiled from.
- [ ] Selecting another symbol should put the source pane on that symbol's own lines. Most of
  the way there: the pane no longer inherits the offset the *previous* symbol left, and since
  the one strip landed a tab remembers a row for *each of its two sides*, keyed by the
  document, so two symbols compiled from one file no longer share a position. What is left is
  the first open: a tab seen for the first time opens its source side at the top of the file
  rather than at the symbol's own lines.
- [x] Mouse buttons can navigate history so you can go back and forth.
- [ ] Add `<`, `>` navigation buttons to the top bar.

## Assembly viewer

- [ ] Bar under the Assembly tab with the full demangled + mangled symbol name.
- [ ] Name the Assembly tab after the function — just `namespace/module::fn_name`, without the
  extra generics, mangling, etc. (for Rust / C++). Half answered from the other side: the
  content area's tab strip names every open function, so the dock tab itself stays
  "Assembly". What is missing either way is the shortening — a chip shows the whole
  demangled name cut at 40 characters, not `module::fn_name`.
- [ ] An expanding section under the Assembly tab to show more symbol info, replacing the Info
  tab.
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
- [D] Allow selection, of characters — deferred, with the reason read out of the freya
  sources (`notes/Plan.md`, 7c). A scroll view is *not* what stops it: freya's selection is
  a range of char offsets into a rope held by the editor, and its own `CodeEditor` selects
  across the rows of a `VirtualScrollView` happily. What stops it is that the model wants
  one rope, one line per row and **one `paragraph()` per line**, and an assembly row is a
  gutter of rects, an address label and up to three separate elements — the middle one
  being the clickable relocation target, which could only survive as an inline child whose
  placeholder character is not a character of any rope. Character selection in the assembly
  pane is a rewrite of the relocation link and the arrow gutter; the source pane could have
  it cheaply but would then behave unlike the pane beside it.
- [ ] A gap or a line before a row something jumps to, so the listing reads as the basic
  blocks it is rather than as one run of instructions. The targets are already known —
  `Assembly::edges` names them, and 7b's gutter draws an arrowhead on each. Note the
  constraint before choosing between the two: `VirtualScrollView` is given one `item_size`
  and `ROW_HEIGHT` must equal it or scrolling misaligns, so a real *gap* means variable row
  heights (or a spacer row of its own in the list), while a hairline drawn inside the row's
  own top edge costs nothing and cannot desynchronise anything.
- [ ] A unified **section** view of code: the whole `.text` as one endless scroll, with the
  symbols drawn as labels *inside* the listing where they start — what `objdump -d` reads like.
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
  addresses in `Section::symbols` are the sync points that make this tractable, so the section
  is probably the concatenation of its symbols' listings plus the gaps between them, decoded
  lazily per symbol and counted as it goes. **Identity**: `Assembly::edges`, `Lanes`, the
  per-tab viewing row and the copy-a-run selection are all indices into *one symbol's*
  instructions. Being a separate mode is what makes that affordable — none of it has to change
  — but the section mode needs the same four answers keyed by address instead, so the question
  is whether those are generalised over what indexes them or written twice. Note that indices
  were deliberately chosen over addresses in the first place, so that a symbol's edges are
  independent of where it sits; a section listing has no such need. **Scale**: `viewer-sample`'s `.text` is far past what
  decoding eagerly on the analysis worker would answer in one go, so this wants the worker to
  answer for a *window* of the section rather than for a whole symbol.
  Note what it would make easy in return: the "gap or line before a jump target" item below, and
  showing a symbol in the context of its neighbours rather than as an island.

- [x] Have arrows for jumps. A gutter left of the addresses draws every branch that stays
  inside the symbol as a line from its row to its target's, with an arrowhead where it lands
  and shorter branches nested inside longer ones. At most five lanes wide, and only as wide
  as the symbol needs; past five, the outermost lane is shared. Hovering a row draws its own
  branches darker, all the way to where they go.
- [ ] Follow a jump by clicking its target offset, the way a call's relocation target is
  clicked. A branch's displacement (`jle 4Bh`) is drawn as plain text where a call's resolved
  target is already a clickable label, so the place a jump goes is still reached by scrolling
  to it. The analysis is already there — `Assembly::edges` names both ends as row indices and
  `reveal_row` already puts a pane on a row — and the crate's half is only recording which
  span the target landed in, `relocation_span`'s twin. Like clicking across the two panes, it
  is a scroll within one symbol rather than a navigation: the selection does not change and
  nothing is pushed onto the history.

## UI

- [x] Migrate the app's hand-rolled panes to freya's own panel components (`ResizableContainer` /
  `ResizablePanel` / `ResizableHandle`), which also makes the split user-resizable.
- [x] Docking panels: a `DockingArea` inside each half of the split, with Objects, Symbols, Info
  and Assembly as tabs that can be dragged between the two areas, stacked into one panel as real
  tabs, or split further.
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
  `square-function`, `info`, `history`, `binary`, `file-code`), drawn at the interface
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
- [ ] One background under both code panes. The assembly pane draws on `asm_pane_bg` and the
  source pane on `pane_bg`, so there is a faint seam down the middle of the default layout. It
  predates the shared palette and is a surface rather than a syntax colour, so 5e left it
  alone; it is one line to unify, and the only real question is which of the two both panes
  should take.

## Panels and tabs

- [x] History panel on the bottom left with recent functions.
- [x] The history panel also lists recent source files. It falls out of the history recording
  *documents*: a visited file is an entry like any function, spelt by its own name and wearing
  the same kind icon its tab does. The recording rule changed with it — the push moved into
  `activate`, which is told why it is being called, so opening a document or going to one is a
  visit and switching to a tab that is already open is not.
- [x] Don't insert duplicate history entries, bump existing ones instead.
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
- [ ] Bookmarks panel for pinned symbols / functions — a list the reader adds to deliberately
  and that outlives the session, unlike the history, which records everywhere they went and
  drops the oldest. A sidebar panel beside Objects / Symbols / History, saved with the
  project. Note the name clash before implementing: `Pinned` in `ui.rs` already means the
  source position a click fixed the two panes on (a transient, one-at-a-time gesture), which
  is a different thing from a bookmark — one of the two wants renaming.
- [ ] Left panel to explore project directory / files.
- [x] Left panel for symbol search — the Symbols panel filters every loaded object's symbols
  by substring, whole word or regex, on the demangled name the row shows.
- [ ] Make that search reachable and ranked: no keyboard shortcut puts the caret in the filter
  box, and matches come back in the list's own name order rather than by how well they match.
- [ ] Left panel for project directory / source search.
- [x] Tabs for assembly functions / source files. One strip over the content area, one tab
  per open document — a function, an object or a source file. Clicking a tab switches; the ×
  closes it and moves to the neighbour; closing the last one goes back to the placeholder.
  They are their own strip rather than dock tabs deliberately — the dock tree is the layout,
  and a layout must survive documents opening and closing. The strip is saved with the session
  and comes back on a rerun, in the order the reader left it.
- [x] Two kinds of tab, assembly-driven and source-driven, told apart by an icon. The two
  independent strips (one for functions, one for files, each with its own notion of what is
  open) are one strip of `Document`s, each of which is a place in a binary or a file, and the
  variant says which of the tab's two sides it is *about* and therefore which drives the
  other. So opening a file and opening a function produce the same kind of thing. The doctrine
  changed with it: "a document is a place in a binary" became "a place in a binary *or* a
  file". A source-driven tab is opened from the Source pane's companion header, which is the
  only door into one until the project explorer and the source search land.
- [ ] The assembly side of a source-driven tab: clicking a line in the file shows the assembly
  compiled from it. Needs the reverse mapping — source line to the symbols compiled from it —
  which line info cannot answer today, and a rule for which symbol wins when a line maps into
  several (recent history is the tie-break). It is the same prerequisite the two items under
  *Source / assembly split view* wait on.
- [ ] Selecting a symbol in a view opens a "temporal" tab when the symbol is not already open
  in a tab: one preview tab reused by the next such selection, so walking down a list does
  not leave a tab behind per click. What promotes it into a tab that stays is a design
  decision for the step that builds it.
- [ ] A larger close icon on a tab, with a highlight under the pointer. The × on a chip is
  small enough to be a target you aim at rather than one you hit, and nothing distinguishes
  the pointer being over the × from being over the chip — so the only feedback that you are
  about to close a tab rather than switch to it arrives after the tab is gone.
- [ ] Reach the panels from the keyboard. Only the two code panes have key handlers at all: the
  tab chips are pointer targets, the sidebar lists have no cursor, and there is no way in to a
  filter box — which is the ranked-search item above seen from the other side. Note what it
  needs deciding first: what "the focused pane" means when either dock area can hold any view.
- [ ] The archive row's object count should survive a narrow sidebar. A parent row ellipsises
  its name before it drops the count column, so dragging the split far enough left eats the
  count instead of the name. It reads correctly at the 300px the sidebar starts at.

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
  `AGENTS.md`: a document there is a *place in a binary*, and a project is not one.
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
- [ ] Follow the newest line of a running scratchpad's output. It has to be scrolled by hand
  today, so a long run scrolls away from the reader. Needs the viewport height and a "the reader
  has scrolled away, leave them there" rule — the same shape `reveal_row` already has.
- [ ] Wrap or scroll the compiler output. Diagnostics and cargo's own stderr clip at the pane's
  right edge, and a diagnostic carrying a span is exactly the line too wide to fit, so the part
  that says where the error is is the part that gets cut.
- [ ] Click a diagnostic to reach the code it is about. The span is drawn under the message but
  is not a target, so a reader with an error still finds the line by counting. Both halves
  already exist: cargo's JSON carries the span's line and column, and the editor has a cursor
  that can be put on one.
- [ ] Scratchpads are a concept of their own, disjoint from projects, and there are many of them
  saved. Today there is exactly one and it is implicit: the app opens `Scratchpad::default`,
  fills it from whatever is on disk and never asks which pad it is looking at. Two halves of
  the separation are already right and are what the rest should be built on — the pads live in
  `scratchpads/` at the top of the state directory beside `projects/` and `settings.toml`,
  never inside a project, and `Pad` is deliberately not one of the states a project switch
  closes, because a scratchpad belongs to the app and not to whatever binary is being read.
  What is missing is the rest of the concept: many pads kept side by side, each its own
  directory the way each project is, each with a name the user can give it, a list of them that
  outlives the session, and a way to move between them — a pad open in one project is the same
  pad in the next, since nothing about it is about a binary. Two decisions to make before
  building it. Where the list lives: a scratchpad list is a second document list, and the
  content area's strip deliberately is not the place for one (a chip there is a *place in a
  binary*), so it is the scratchpad view's own or a sidebar panel of its own. And which pad
  opens at startup, which is the question `recents.toml` already answers for projects — an
  order, most recent first, rather than a field saying "last".
- [ ] Stop a run's grandchildren with it. `Running::stop` kills the process the app spawned, so a
  scratchpad that spawns a child of its own leaves it running with nothing that could ever find
  it again. The fix is starting the run in a process group of its own and killing the group —
  a `libc` call on Unix and a job object on Windows, neither of which this crate carries today.

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
  line rows go through. On `libanalysis-sample.rlib` 3 704 of 3 705 symbols answer from DWARF;
  on `viewer-sample` half do, the rest being C++ dependencies built without it.
- [x] Take entry points and exported DLL / dylib functions as symbols too. `declared_code`
  reads `dynamic_symbols`, `exports` and `entry` for a **linked image** — all declared, so
  nothing is guessed at or scanned for — and `LLVM-24-rust-dev.dll` goes from zero functions to
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
  comparison rather than by a counter. The first symbol clicked on `viewer-sample` cost 589 ms
  on the UI thread and now costs a channel send.
- [ ] Binary inspection should be multi threaded — in the sense of using more than one core,
  which it does not. Everything above is *off* the UI thread but still sequential: `demangled`
  is one `map` per object and `open_files_streaming` walks objects in order, so an archive's 196
  members demangle one after another. Both levels are embarrassingly parallel, and demangling is
  the whole of what is left to parallelise (281 ms of `viewer-sample`'s 1 437 ms open). Note the
  constraint before starting: long names are demangled on a thread with a 64 MiB stack because
  the demanglers recurse per pointer, so a parallel version wants a bounded pool of big-stack
  threads rather than one per object.
- [D] Cache the demangled names between runs. **Built, measured and then thrown away** — it is
  not in this repo's history, deliberately, so this item is the whole record of it. The gain did
  not justify what it cost to carry. What it bought, per file, release, cold open
  against warm: `viewer-sample` 1 589 → 1 174 ms (-26%), `libanalysis-sample.rlib` 111 → 31 ms
  (-72%), `LLVM-24-rust-dev.dll` 542 → 479 ms (-12%), `librustc_data_structures` 5 → 1 ms (too
  small to mean anything). **An average of roughly 37% across the three samples big enough to
  measure**, and about 25% weighted by the time actually spent rather than by file.
  Against that: a fourth place on disk outside both the project and the settings, an eviction
  budget, a hand-rolled binary format with its own version and checksum (TOML cannot express an
  `Option<String>`), and two defects the format's own corruption sweep found — a short document
  silently truncating the symbol table, and a well-formed one able to hold a wrong name. That is
  a lot of machinery, and a wrong function name on screen is a bad failure, for a second and a
  half on the largest sample here.
  Undeferred by the open getting slower or the saving getting larger — parallel demangling above
  attacks the same cost without persisting anything, and is the thing to try first.
- [D] Cache inspection results in the project info. Neither `assembly()` nor `line_info()`
  memoizes, so leaving a listing and coming back re-derives it — 4–8 ms on `viewer-sample`,
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
- [ ] Make the architecture-specific analysis **generic rather than dynamically dispatched**.
  The seam landed as `disassembler(architecture) -> Option<Box<dyn Disassembler>>`, so every
  request boxes a zero-sized backend and calls it through a vtable. The set of architectures is
  closed at compile time, so the dispatch wants to happen *once*, in a `match` that hands each
  arm a concrete type, with the generic code monomorphised per backend. What that buys is not
  the virtual call — the trait is one call wide per symbol, so a vtable lookup there is
  nothing — but the allocation going away and the backend's formatting and span-mapping
  becoming inlinable into the decode loop, which *is* per instruction. It also keeps the
  crate's types nameable: a `dyn` in a public signature is a type the caller cannot spell.
  Note the trade before doing it: `dyn` is what makes a backend list open-ended, and generics
  mean every new architecture is a new arm rather than a new impl behind a registry. That is
  the right trade here, the set being fixed at compile time either way.
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
  in `AGENTS.md`: everything expensive is *already* deferred — line info and the DWARF context to
  the first query, subprogram extents behind the same cache, disassembly to selection. What is
  left at open time is reading the bytes and walking the symbol table, which is precisely what
  the Objects and Symbols lists *are*, so there is nothing to opt out of without opting out of
  the app. The only remaining lever is deferring demangling (21% of `viewer-sample`, 47% of the
  rlib in release), and it only defers to the first click on that object while costing a second
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

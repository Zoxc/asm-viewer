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
- [ ] Grammars beyond Rust / C / C++ for the source side. Any other extension renders plain;
  each language is a `tree-sitter-<lang>` dependency and an arm in `language()`.

## Navigation

- [x] Clicking on functions in assembly should navigate to them.
- [ ] Clicking on functions in source should navigate to them. A click on a source line moves
  the assembly pane to that line's instructions; a click on a *call* in the source still does
  nothing, since nothing maps a source identifier to the symbol it names.
- [x] Navigating in assembly should also navigate source, within a symbol: clicking an
  instruction scrolls the source pane to the line it was compiled from.
- [ ] Selecting another symbol should put the source pane on that symbol's own lines. It opens
  the right file, but keeps whatever scroll position the symbol before it left behind.
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
- [ ] Allow selection. Neither side has it: the source pane is hand-rolled rows rather than
  freya's `CodeEditor` (which does have selection, but can only highlight the cursor's own line
  and cannot be scrolled from outside — see `notes/Plan.md`, 5a).
- [ ] A gap or a line before a row something jumps to, so the listing reads as the basic
  blocks it is rather than as one run of instructions. The targets are already known —
  `Assembly::edges` names them, and 7b's gutter draws an arrowhead on each. Note the
  constraint before choosing between the two: `VirtualScrollView` is given one `item_size`
  and `ROW_HEIGHT` must equal it or scrolling misaligns, so a real *gap* means variable row
  heights (or a spacer row of its own in the list), while a hairline drawn inside the row's
  own top edge costs nothing and cannot desynchronise anything.
- [x] Have arrows for jumps. A gutter left of the addresses draws every branch that stays
  inside the symbol as a line from its row to its target's, with an arrowhead where it lands
  and shorter branches nested inside longer ones. At most five lanes wide, and only as wide
  as the symbol needs; past five, the outermost lane is shared. Hovering a row draws its own
  branches darker, all the way to where they go.

## UI

- [x] Migrate the app's hand-rolled panes to freya's own panel components (`ResizableContainer` /
  `ResizablePanel` / `ResizableHandle`), which also makes the split user-resizable.
- [x] Docking panels: a `DockingArea` inside each half of the split, with Objects, Symbols, Info
  and Assembly as tabs that can be dragged between the two areas, stacked into one panel as real
  tabs, or split further.
- [ ] A dark mode, in the same palette rather than a second one — the light colours carried
  over at dark-mode lightness (inverted?), so the two themes are recognisably one design.
  The groundwork is in: every colour is a field of one `Palette` in `ui.rs` and every call
  site reads it through `palette()`, so what is left is a second `const`, something that
  re-renders when the choice changes, and clearing the highlighted-source cache on a switch.
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

## Panels and tabs

- [x] History panel on the bottom left with recent functions.
- [ ] The history panel also lists recent source files. The Source pane has its own tab
  strip now, so the open files are a list the history could draw from — but nothing records
  when one was *visited*, only that it is open.
- [x] Don't insert duplicate history entries, bump existing ones instead.
- [x] Tree view for objects, with an indicator per row for the file type. A file that
  contributed one object is one row; an archive is a parent row its members fold under,
  and the type is a short tag (`ELF`, `PE`, `COFF`, `MACH`, `AR`) rather than a picture —
  freya's icon set is a dependency behind a feature and has no notion of an object format.
- [ ] An indicator for an object still being processed. Nothing can be in that state yet:
  `open_files` parses every object on its worker thread before the UI hears about any of
  them, so this waits for "Can explore while binary is processed" under *Binary inspection
  design*.
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
  neighbouring tab the way closing one tab by hand does (`Selection::None` only when that
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
- [x] Tabs for assembly functions / source files. A strip of chips over the content area,
  one per open function or object, and a second strip over the Source pane, one per open
  file. Clicking a chip switches; the × closes it and moves to the neighbour; closing the
  last one goes back to the placeholder. They are chips rather than dock tabs deliberately
  — the dock tree is the layout, and a layout must survive documents opening and closing
  (`notes/Plan.md`, 6c). Not persisted yet: that is *Projects*' "saves with tabs".
- [ ] Two kinds of tab, assembly-driven and source-driven, told apart by an icon. The
  left-most pane is the one the tab is *about*, and it drives what the right-hand pane
  shows: an assembly-driven tab has the function on the left and the source it came from on
  the right, a source-driven tab has the file on the left and the assembly for the line on
  the right. Reading of the request, to confirm before building: this replaces the two
  independent strips 6c produced (one for functions, one for files, each with its own
  notion of what is open) with one strip whose chips each say which side is in charge — so
  opening a file from a directory panel and opening a function from the symbol list produce
  the same kind of thing, differing only in which way the mapping runs.
- [ ] A larger close icon on a tab, with a highlight under the pointer. The × on a chip is
  small enough to be a target you aim at rather than one you hit, and nothing distinguishes
  the pointer being over the × from being over the chip — so the only feedback that you are
  about to close a tab rather than switch to it arrives after the tab is gone.

## Projects

- [x] Minimal project support: the previous session's binaries and selected symbol reopen when
  the app is rerun, for easier testing.
- [ ] Have a project concept.
- [ ] Anonymous projects — opening files without an explicit project — should be saved too, next
  to the user / global settings. There can be multiple such anonymous projects.
- [ ] Each project can have multiple binaries loaded.
- [ ] Has an associated directory.
- [x] Can have multiple tabs with different function assemblies / source files open. Within
  a session: the content area's strip holds the open functions and objects, the Source
  pane's holds the open files. Carrying them across a restart is the "saves with tabs" item
  below.
- [ ] Saves with tabs / hashes of binaries, open tabs and viewing positions.
- [ ] Can have an LSP server (like rust-analyzer) / cargo integration so it can build for you and find binaries.
- [?] Maybe store LSP output in a more compact index given we expect source to not be modified?
- [ ] A main view where you can see all project info.
- [?] Snapshots of projects where binaries and source can be embedded (compressed?) and different versions of projects can be compared.
- [ ] Split project storage into toml? for user given settings and another file for opened tabs / cached binary inspection data
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

- [ ] Reopen previous open project on startup.
- [ ] Have a view of recent projects if none was open.

## Fonts and settings

- [ ] Match system fonts / coding fonts by default, by runtime lookup. KDE / Gnome / Windows.
- [ ] Have a settings page where you can override (with a default being unspecified with clear visual distinction).

## Scratchpad

- [ ] A scratchpad function which allows creating single file rust projects where you can build with cargo and view assembly output.
- [ ] Allow `#[crates(version = "3.4.6") extern crate dfsh;` to use specific versions from crates.io (require a specific version).
- [ ] Allow these files to run with output viewable

## Binary inspection design

- [ ] Can explore while binary is processed to find all functions.
- [x] Rely on declared functions in binaries. Don't assume things can be code — only declared
  text symbols are disassembled, nothing is scanned for.
- [x] Rely on debug info: DWARF line info is read (lazily, per section).
- [ ] Rely on debug info for function extents too, rather than estimating from the next symbol's
  address (`estimate_size`), since declared sizes are often 0 in COFF/ELF.
- [ ] Take entry points and exported DLL / dylib functions as symbols too. Only the symbol
  table's `SymbolKind::Text` entries are kept today, so a stripped shared library is a file
  with nothing in it — `LLVM-24-rust-dev.dll` has no COFF symbol table at all and lists zero
  functions, which is a whole sample the app cannot open in any useful sense. The image
  declares its code in two other places the `object` crate already reads: the entry point
  (`Object::entry`) and the export table (`Object::exports`, plus `dynamic_symbols` for an
  ELF `.so`), and both are *declared* functions in the sense the goal above means — nothing
  is being guessed at or scanned for. Both need a size, which they do not carry, so
  `estimate_size` has to derive it the way it does for a declared size of 0.
- [ ] Find unwind targets.
- [?] PDB / CodeView line info. DWARF is read today; the PE sample has no debug sections at all
  and a rustc `.rlib` member is COFF with CodeView (`.debug$S`/`.debug$T`), so on Windows output
  there is no source view without this.
- [ ] Binary inspection should be light weight and multi threaded. Result saves in project info.
- [ ] Binary inspection should be designed to be portable, allowing different disassembly libraries to be used.
- [ ] Should never panic on any file input. Errors doing analysis should allow inspecting functions without errors.
- [ ] Don't run by default, make that opt-in as needed.
- [?] Prefer memory mapped files and minimal memory footprint, store locations into the mapped file?
- [?] How to design an index to allow source files / assembly to map, without large memory footprint.
- [x] Have this in its own crate. Use it for test cases.
- [ ] Add a minimal test case every time we find something wrong with binary inspection.

---

Maintain this file when a feature is requested.

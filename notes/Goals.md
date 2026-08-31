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
- [ ] Selecting one side highlights the other side. Hovering does; there is nothing to
  select on either side yet — a click navigates nowhere within a symbol until 5c.
- [ ] Have a function to find all source / assembly locations that match, producing a list on the other side.
- [ ] A function to pick the generic instance of a source function.
- [ ] An active navigation function where selection on one side navigates to the relevant section on the other side, preferring recent history on duplicates.
- [x] Syntax highlighting for both sides. Assembly is coloured by span kind; source is
  tree-sitter, through the highlighter `freya-code-editor` exposes publicly.
- [ ] Grammars beyond Rust / C / C++ for the source side. Any other extension renders plain;
  each language is a `tree-sitter-<lang>` dependency and an arm in `language()`.

## Navigation

- [x] Clicking on functions in assembly should navigate to them.
- [ ] Clicking on functions in source should navigate to them.
- [ ] Navigating in assembly should also navigate source.
- [x] Mouse buttons can navigate history so you can go back and forth.
- [ ] Add `<`, `>` navigation buttons to the top bar.

## Assembly viewer

- [ ] Bar under the Assembly tab with the full demangled + mangled symbol name.
- [ ] Name the Assembly tab after the function — just `namespace/module::fn_name`, without the
  extra generics, mangling, etc. (for Rust / C++).
- [ ] An expanding section under the Assembly tab to show more symbol info, replacing the Info
  tab.
- [x] Keep the `rip+` visible in a relocated rip-relative operand — `mov dword ptr [rip+<target>], 7`
  rather than `mov dword ptr [<target>], 7` — when you can navigate to the target.
- [ ] Allow selection. Neither side has it: the source pane is hand-rolled rows rather than
  freya's `CodeEditor` (which does have selection, but can only highlight the cursor's own line
  and cannot be scrolled from outside — see `notes/Plan.md`, 5a).
- [ ] Have arrows for jumps.

## UI

- [x] Migrate the app's hand-rolled panes to freya's own panel components (`ResizableContainer` /
  `ResizablePanel` / `ResizableHandle`), which also makes the split user-resizable.
- [x] Docking panels: a `DockingArea` inside each half of the split, with Objects, Symbols, Info
  and Assembly as tabs that can be dragged between the two areas, stacked into one panel as real
  tabs, or split further.
- [D] Bring back the floem-style thicker scrollbars. Deferred: freya 0.4 hardcodes the scrollbar
  sizes (its `ScrollBar` theme declares a `size` field that is never read, and `ScrollView` /
  `VirtualScrollView` always pass `theme: None` with the override fields `pub(crate)`), so the only
  way is vendoring the whole scrollview module (~1350 lines) out of `freya-components` — too much to
  carry for a cosmetic change. Revisit if freya makes it themeable.

## Panels and tabs

- [x] History panel on the bottom left with recent functions.
- [ ] The history panel also lists recent source files.
- [x] Don't insert duplicate history entries, bump existing ones instead.
- [ ] Tree view for objects, with icon indicators for processing / file type.
- [ ] Filter bar under objects / symbols / history, with icons for caps / full word / regex.
- [ ] Tooltip for items in panels — missing from objects now.
- [ ] Left panel to explore project directory / files.
- [ ] Left panel for symbol search.
- [ ] Left panel for project directory / source search.
- [ ] Tabs for assembly functions / source files.

## Projects

- [x] Minimal project support: the previous session's binaries and selected symbol reopen when
  the app is rerun, for easier testing.
- [ ] Have a project concept.
- [ ] Anonymous projects — opening files without an explicit project — should be saved too, next
  to the user / global settings. There can be multiple such anonymous projects.
- [ ] Each project can have multiple binaries loaded.
- [ ] Has an associated directory.
- [ ] Can have multiple tabs with different function assemblies / source files open.
- [ ] Saves with tabs / hashes of binaries, open tabs and viewing positions.
- [ ] Can have an LSP server (like rust-analyzer) / cargo integration so it can build for you and find binaries.
- [?] Maybe store LSP output in a more compact index given we expect source to not be modified?
- [ ] A main view where you can see all project info.
- [?] Snapshots of projects where binaries and source can be embedded (compressed?) and different versions of projects can be compared.
- [ ] Split project storage into toml? for user given settings and another file for opened tabs / cached binary inspection data
- [x] Save the navigation history.
- [x] Opening binary files saves immediately.
- [ ] User project changes should save immediately.
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

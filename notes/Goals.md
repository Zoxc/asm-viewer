# Goals

Inspecting and comparing binaries.

`- [ ]` is a goal that is not done yet, `- [x]` one that is, and `- [?]` one that is only a maybe — not decided on yet.

## Source / assembly split view

- [ ] Have a source view and an assembly view, mapping between them. This is the default split view that should generally be used.
- [ ] Selecting one side (or hovering) highlights the other side.
- [ ] Have a function to find all source / assembly locations that match, producing a list on the other side.
- [ ] An active navigation function where selection on one side navigates to the relevant section on the other side, preferring recent history on duplicates.
- [ ] Syntax highlighting for both sides.

## Navigation

- [ ] Clicking on functions in assembly should navigate to them (same in source).
- [ ] Navigating in assembly should also navigate source.
- [ ] Mouse buttons can navigate history so you can go back and forth.

## Assembly viewer

- [ ] Allow selection.
- [ ] Have arrows for jumps.

## Panels and tabs

- [ ] History panel on the bottom left with recent functions / source files.
- [ ] Left panel to explore project directory / files.
- [ ] Left panel for symbol search.
- [ ] Left panel for project directory / source search.
- [ ] Tabs for assembly functions / source files.

## Projects

- [ ] Have a project concept.
- [ ] Each project can have multiple binaries loaded.
- [ ] Has an associated directory.
- [ ] Can have multiple tabs with different function assemblies / source files open.
- [ ] Saves with tabs / hashes of binaries, open tabs and viewing positions.
- [ ] Can have an LSP server (like rust-analyzer) / cargo integration so it can build for you and find binaries.
- [?] Maybe store LSP output in a more compact index given we expect source to not be modified?
- [ ] A main view where you can see all project info.
- [?] Snapshots of projects where binaries and source can be embedded (compressed?) and different versions of projects can be compared.

## Startup

- [ ] Reopen previous open project on startup.
- [ ] Have a view of recent projects if none was open.

## Fonts and settings

- [ ] Match system fonts / coding fonts by default, by runtime lookup. KDE / Gnome / Windows.
- [ ] Have a settings page where you can override (with a default being unspecified with clear visual distinction).

## Scratchpad

- [ ] A scratchpad function which allows creating single file rust projects where you can build with cargo and view assembly output.
- [ ] Allow `#[crates(version = "3.4.6") extern crate dfsh;` to use specific versions from crates.io (require a specific version).

## Binary inspection design

- [ ] Can explore while binary is processed to find all functions.
- [ ] Rely on debug info and declared functions in binaries. Don't assume things can be code.
- [ ] Binary inspection should be light weight and multi threaded. Result saves in project info.
- [ ] Binary inspection should be designed to be portable, allowing different disassembly libraries to be used.
- [ ] Should never panic on any file input. Errors doing analysis should allow inspecting functions without errors.
- [ ] Don't run by default, make that opt-in as needed.
- [?] Prefer memory mapped files and minimal memory footprint, store locations into the mapped file?
- [?] How to design an index to allow source files / assembly to map, without large memory footprint.
- [ ] Have this in its own crate. Use it for test cases.
- [ ] Add a minimal test case every time we find something wrong with binary inspection.

---

Maintain this file when a feature is requested.

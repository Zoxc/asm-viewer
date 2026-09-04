## User rules

Standing instructions from the user.

- Committing. Whatever is uncommitted when you start stays uncommitted.
- Run rustfmt over every file you modified, before committing it. Format **only those files**,
  with `rustfmt --edition 2021 <paths>`, and not the workspace: a bare `cargo fmt` reformats
  everything, and much of this repo predates anyone running it, so it drags unrelated reflow into
  a diff that then has to be picked apart by hand. The `--edition 2021` is load-bearing; plain
  `rustfmt` parses as 2015 and will mangle what it cannot read. Nothing here needs a
  `rustfmt.toml`: the defaults are what the formatted files already follow.
- Commit messages. Keep the title brief -- a line, not a paragraph -- and put what needs
  saying under it, ideally in one short paragraph. What changed is in the diff; the message
  is for what it is and why.
- Don't reference an uncommitted file from a committed one.
- Keep `notes/Goals.md` current. It is the checklist of planned features.
- `notes/specs/` is what a finished feature does, one file per area with a section per feature,
  moved there from `notes/Goals.md`. **Never write or change a spec without asking the user
  first**; read the spec files to gauge the writing style and keep development history and
  implementation detail out (`notes/specs/README.md`).
- Before presenting edits to the user, make one final pass over every text written (prose, a
  code comment, a doc comment, markdown): cut what can go, and prefer simple English, short
  direct sentences and plain words. Neither may lose precision. Only then, not after every
  edit.
- **Adding a goal is not a request to do it.** "Add a goal: …" means write the item down and
  stop there; the checklist is where work is planned, not where it is started. Implement one
  only when asked for the thing itself.
- Prefer TOML for files, not JSON.
- Add a minimal test case every time something is found wrong with binary inspection.
- Answer a question about the UI with a headless test rather than by launching the app. A
  throwaway one, deleted once it has answered, is fine. See `agents/Headless.md`.

## What this is

A desktop GUI ("Assembly Viewer") for inspecting object files: open an ELF/PE/Mach-O object or a
static archive, browse its text symbols, and read a symbol's disassembly, with relocation targets
resolved to clickable symbol names, beside the source it was compiled from. Rust, cargo
workspace, [freya](https://freyaui.dev) 0.4 for the UI.

## Build

`cargo run --features devtools` starts freya's devtools server alongside the app (`[::1]:7354`,
opt-in so it never reaches a release build). The viewer is a separate `cargo install
freya-devtools-app`, only one devtools-enabled freya app can run at a time, and there is no in-app
shortcut. See `notes/DevTools.md`.

The first build compiles Skia (via `freya-engine` -> `freya-skia-safe`) and takes a long time. On
Fedora it needs `freetype-devel fontconfig-devel libglvnd-devel wayland-devel` to link, and `mold`
is not a supported linker.

Dependency versions are pinned by compatibility, not taste. The `tree-sitter-*` grammars must sit
on the `tree-sitter-language` ABI of the one `tree-sitter` the app and `freya-code-editor` share
(`cargo tree -p viewer -d`). `addr2line` 0.21 / `gimli` 0.28 / `object` 0.32 must stay one copy
each (check with `cargo tree -p analysis -d` after touching any of them), as must
`fallible-iterator` 0.3, which `gimli` and `pdb2` share, and `digest` 0.10, which the three hash
crates share. The reasoning is in the `Cargo.toml` comments; keep them current.

### Test fixtures

Almost every fixture is built **in memory** with the `object` and `gimli` writers
(`crates/analysis/tests/common/mod.rs`), so the suite needs nothing on disk and is green in a fresh
checkout. The exception is `crates/analysis/tests/fixtures/`: one small C file and, **committed**
from it, the two objects `gcc` produced (so the crate is pinned against DWARF a real toolchain
emits); a stripped shared object `gcc` and `ld` produced with its functions hidden
(`line_fixture_hidden.so`: no symbol table, nothing in `.dynsym`, so the crate is pinned against
an `.eh_frame` as a real linker lays it out, the only thing naming its functions); and three DLL
plus `.pdb` pairs that `clang-cl` and rustup's `rust-lld` produced (so it is pinned against a real
linker's PE debug directory and PDB, which nothing in memory can synthesize). Of the three, one
exports its three functions; `line_fixture_noexport` exports nothing, so every name it shows is
the PDB's, and alone it lists the three `<function 0x…>`s its `.pdata` states; and
`line_fixture_public` is that object linked with a one-function C++ file (`public_fixture.cpp`)
compiled without `/Z7`, so the PDB's only name for that function is a decorated public symbol.
`tests/real_object.rs` and `tests/pdb.rs` read them and fail loudly rather than skipping when they
are missing; the build commands are in `line_fixture.c`'s header, `pdb.rs`'s and `unwind.rs`'s.

The measurements quoted in `agents/` were taken on two inputs `cargo build` produces: the app's
own debug binary (~331 MB, one linked ELF, ~115k text symbols, ~267 MB of DWARF) and the `analysis`
crate's own rlib (~20 MB, 196 archive members, DWARF per member). Session state is restored on
launch, so `cargo run` reopens the last binaries on the last symbol and a visual check costs one
command.

## Layout

- `crates/analysis/src/lib.rs` — object parsing, disassembly, relocation resolution.
- `crates/analysis/src/line.rs` — line info, lazy: an address range in, source rows out. The
  seam: the two questions every backend answers, dispatched by `match`, and the one collector
  that makes every answer's rows hold `LineInfo`'s invariants. Names no debug format.
- `crates/analysis/src/line/dwarf.rs` — the DWARF backend, and the only part that knows
  DWARF's debug sections and `addr2line`.
- `crates/analysis/src/line/pdb.rs` — the PDB backend: a PE's `.pdb` found by its CodeView
  record, matched by GUID and age, read a page at a time; the only part that knows `pdb2`. Also
  the one eager path through the seam: opened at parse for the procedures and publics it names,
  which `parse_object` takes as symbols.
- `crates/analysis/src/line/source.rs` — the same line info the other way: a file and a line,
  out to the symbols compiled from them, built on the seam and not on a backend.
- `crates/analysis/src/unwind.rs` — the unwind tables a linked image states its functions'
  bounds in (an x86-64 PE's `.pdata`, an ELF's `.eh_frame`), read for the ranges they declare;
  the only part that reads call-frame information.
- `crates/analysis/src/disasm.rs` — the disassembler seam; `disasm/x86.rs` is the only `iced-x86`.
- `crates/analysis/src/guard.rs` — the calls whose panics are caught on purpose, and the flag
  that lets a panic hook tell one of those from a panic that has broken the app.
- `src/cargo.rs` — running cargo and reading what it said: the artifacts it names, the
  diagnostics it reports, and the profile's debug information in the manifest being built.
- `src/project.rs` — projects: their identity, the two files each is stored in, the save policy,
  and which language server each is read with.
- `src/rescue.rs` — a stored file that will not parse, moved aside before a write replaces it.
- `src/settings.rs` — the user's own settings (`settings.toml`): the font overrides and the theme.
- `src/source.rs` — source files read off disk and cached by path, failures included.
- `src/scratchpad.rs` — a scratchpad: its id, its name, the cargo package generated around one
  source file, its build, and the pads there are in the order they were last opened.
- `src/filter.rs` — what a filter bar is asking for and the matcher it compiles to.
- `src/search.rs` — the project's directory searched for a pattern: the walk, the match, and
  the hits grouped under the file each is in.
- `src/tree.rs` — the Objects list's tree shape, and which files are still being read into it.
- `src/files.rs` — the project's directory as a tree: read one level per unfold, forgotten on
  the fold, and flattened into the rows the Files view draws.
- `src/lanes.rs` — where each branch is drawn in the assembly view's arrow gutter.
- `src/lsp.rs` — the language server: the program the project names started over its
  directory, the messages spoken to it, the project's own `.vscode/settings.json` read into
  what it is told, and the process a stop kills.
- `src/process.rs` — the process group a child is started in, so a stop reaches what it
  started; a scratchpad's run and the language server both have one.
- `src/pixels.rs` — the device pixel grid, and a stroke put on it by its edges.
- `src/rows.rs` — the run of rows a reader selects to copy.
- `src/chars.rs` — the run of characters a sweep over a row's text selects: a place is a row
  and a column in UTF-16 units, a row's text is pieces, and what each row draws and copies.
- `src/section.rs` — the rows a listing of an object's whole code is made of: estimated before
  a stretch is decoded, the symbol's own after, and an address for every one.
- `src/docs.rs` — `Docs`, the table mapping a dock tab's `DocId` to the trail behind it: every
  place the tab has shown with a cursor on the one it shows, and which tab is the temporal one.
- `src/compiled.rs` — the symbols a source line was compiled into, and which of them a tab follows.
- `src/tabs.rs` — `landing`, the rule a close obeys; `Positions`, where each tab was left;
  `Driven`, which line a source-driven tab's assembly side follows and which symbol was chosen.
- `src/history.rs` — one tab's back/forward trail.
- `src/visits.rs` — everywhere the reader has been, across every tab: what the History panel
  lists.
- `src/bookmarks.rs` — the reader's bookmarks: a saved place and the name it was made under,
  in the order they were added; saved in `project.toml`, live only against what is loaded.
- `src/naming.rs` — a demangled name cut down to the `module::fn_name` a tab is called by.
- `src/panics.rs` — a panic on any thread: the record, the file a run appends it to, the box
  the reader is shown, and the shutdown after it.
- `src/fonts.rs` — the desktop's font settings (KDE, Gnome, Win32) merged under the user's own.
- `src/functions.rs` — the functions a source file defines, by the lines they span, and which
  one a line is inside; `functions/rust.rs` is the scanner that finds Rust's without the grammar.
- `src/ui.rs` — the freya UI's root: its prelude, the list of its files, `toolbar` with its two
  history buttons, and `app`.
- `src/ui/metrics.rs` — every measurement no component owns, and the fonts they follow.
- `src/ui/palette.rs` — every colour, the theme it is resolved from, and the compositing rules.
- `src/ui/state.rs` — the contexts provided once at the root and read with `use_consume`.
- `src/ui/analyzed.rs` — the worker's question, its answer, and the supersession rule.
- `src/ui/focus.rs` — a place in a file, the landing a click from outside the panes makes, and
  where each side of a tab was left.
- `src/ui/marks.rs` — the run of rows selected in each pane, the pair it lights on the other
  side, the scroll it owes, and what Ctrl+C copies.
- `src/ui/highlight.rs` — a source file parsed when loaded, its spans and its functions, and the
  cache holding it.
- `src/ui/locations.rs` — every symbol a line, or the function around it, was compiled into: the
  question, the answer, the panel.
- `src/ui/reading.rs` — what the worker has decoded of an object's code for the section view,
  and the window of it the view asks for next.
- `src/ui/rescued_view.rs` — the window naming the stored files that would not parse and where
  each was moved to.
- `src/ui/search_view.rs` — the Search panel: what was searched for, the hits as they arrive,
  and the one worker that finds them.
- `src/ui/section_view.rs` — the section view: an object's code as one listing, its rows, the
  place it keeps as an address, and the window it asks for.
- `src/ui/filter_bar.rs` — one filter bar, its three toggles, and the Symbols list's memo.
- `src/ui/language.rs` — whether a language server is running, what the project's own
  settings said, the worker that talks to it, and the control in the top bar that starts and
  stops it.
- `src/ui/documents.rs` — what opening, closing and moving between documents means.
- `src/ui/sidebar.rs` — the three lists a binary is browsed with, and the rows each is built of.
- `src/ui/building.rs` — building the project's own workspace: what is held about it, the one
  worker thread, and which binaries a finished build replaces.
- `src/ui/bookmarks_view.rs` — the Bookmarks list: one row per bookmark, live against what is
  loaded and kept dimmed when it is not.
- `src/ui/files_view.rs` — the Files view: the project's directory as a tree, a file's row
  opening it as source and its menu offering it as a binary.
- `src/ui/code_row.rs` — one row of a code listing as all three listings draw theirs: the
  shared width, wash and pointer handlers, and the one paragraph a row's text is.
- `src/ui/assembly.rs` — the assembly side of a document: the rows, the gutter, the pane.
- `src/ui/symbol_bar.rs` — the bar over that pane naming what it is drawing, and its section.
- `src/ui/source_view.rs` — the source side of one, and which file it is showing.
- `src/ui/dock.rs` — the dock, its two-kinded tab, and the document panel's own tab bar.
- `src/ui/project_view.rs` — which project is open: the pane, the switch, the save observers,
  and what the language server has to say for itself.
- `src/ui/settings_view.rs` — the settings page, and the three hooks behind the theme and fonts.
- `src/ui/pad.rs` — the scratchpads the app holds, which is shown, and their one worker thread.
- `src/ui/pad_view.rs` — the scratchpad's pane: pad list, editor, crates, diagnostics, output.
- `src/ui/parts.rs` — the small stateless pieces of drawing shared by unrelated panes.
- `src/ui/width.rs` — the widest row a code listing has drawn, and the width every row of
  it takes from that: what lets the code panes scroll sideways with their wash whole.

Ten `ui/` names avoid shadowing a crate module the prelude brings in (`source_view`,
`project_view`, `filter_bar`, `bookmarks_view`, `files_view`, `pad`, `analyzed`, `building`,
`rescued_view`, `language`); the rest is in `agents/UI.md`.

Everything except the UI is framework-free and unit-tested rather than eyeballed. **A module's
tests are a file of their own**: `src/<module>/tests.rs`, declared `#[cfg(test)] mod tests;` at
the foot of `src/<module>.rs`, so the module a reader opens is the module and not the module plus
half again of what it is asserted to do. The path a test is named by (`project::tests::…`) is
unchanged, which is the point: it is where the file sits and not what the module tree looks like.

## Design notes

The reasoning behind the code (what was decided, what it cost, and what was measured) is in
`agents/`. **Read the note for the area before changing it, and rewrite the paragraph a change
invalidates in the same commit**: these are the record of why things are the way they are.

- `agents/Analysis.md` — the crate: parse pipeline, data model, demangling, line info both ways,
  the disassembler seam, relocations, branch edges, and the never-panic testing.
- `agents/Persistence.md` — projects and their two files, the session restore, `Saves`, recents,
  and `settings.toml`.
- `agents/Scratchpad.md` — a scratchpad as a generated cargo package, its id and name, building,
  running, and the view with its one worker thread.
- `agents/UI.md` — freya 0.4, the root contexts, documents and the dock, per-tab positions,
  opening a binary, and how the UI is tested.
- `agents/Worker.md` — the one analysis worker: asks, locates, supersession, what shows meanwhile.
- `agents/Panes.md` — the Source and Assembly panes: companion files, `Driven`, the two runs and
  the pair, landing, the arrow gutter, copying rows.
- `agents/Sidebar.md` — the filtered lists, the Objects tree, closing a binary, the Project view.
- `agents/Appearance.md` — the palette, theme switching, fonts, row heights, the Settings page.
- `agents/Lsp.md` — the language server: why it is a control, the hand-rolled protocol and
  what rust-analyzer needs of it, the process, what an answer is about, and what a project's
  own settings file is read into.
- `agents/Headless.md` — `freya-testing` as it actually behaves, checked against its sources.

What the app *does*, the rules a finished feature follows, is in `notes/specs/`, one file per
area with a section per feature, moved there from `notes/Goals.md` once the goal is done. A spec
says what and an `agents/` note says why; neither repeats the other, and a spec is only written
or changed on the user's say-so (`notes/specs/README.md`).

Bugs and gaps in dependencies are in `notes/upstream/`, one file per crate: what was hit, what
it cost here, and whether it was reported. Add the note with the workaround. Each file may end
with a `## Wanted` section for features the crate lacks and the app substitutes for: add the
feature there with the substitute, so a release that brings it is noticed.

## Rules that hold everywhere

- **Never panic on any file input.** Checked arithmetic in preference to a wider `catch_unwind`;
  the guard is for a dependency's bug, never for ours. A stack overflow aborts and cannot be
  caught, so anything recursing over file-controlled input is bounded before the call.
- **Nothing is analysed on the UI thread**, and nothing is cached in the UI: the worker's answer
  is held, not memoized.
- **A document is a place in a binary or a file; everything else is a view.** `open_document`,
  `raise`, `navigate`, `close_tab`, `close_others` and `close_binary` are the only six functions
  that change what is open or what a tab shows.
- **Identity in the UI is `Arc` pointer identity**, never names or indices: list keys are
  `Arc::as_ptr(..).addr()` and prop `PartialEq`s are hand-written with `Arc::ptr_eq`.
- **Asking for a colour or a font is what subscribes a scope to it** (`palette()`, `fonts()`);
  `set_appearance` and `set_fonts` are the only writers. Never write a literal colour or row height.
- **Persisted formats need no backward compatibility** yet: a stale file is ignored, not migrated.
  Field order in the serde structs is load-bearing, since TOML puts plain values before tables.

## Gotchas before editing the UI

- A `State`'s `peek`/`read` hands back a guard, and an `if let` holds its scrutinee's temporary
  until the end of its **body**, so `if let Some(x) = *state.peek() { state.set(..) }` compiles and
  panics the moment it runs. `let ... else` and `match` end theirs with the statement. Bind the read
  to a `let` of its own before any write. That class of bug is invisible to every other test in the repo; the headless tests in `src/ui/tests.rs` catch it.
- There is no `.hover()` pseudo-state. A hoverable row is a `Component` with `use_state(|| false)`
  plus `on_pointer_over`/`on_pointer_out` (`over`/`out`, not `enter`/`leave`, so hovering a child
  keeps the highlight).
- `VirtualScrollView`'s builder closure is never compared across renders, so anything the rows
  depend on must go through `new_with_data`, not be captured.
- A row's height must equal the `item_size` given to the `VirtualScrollView` over it, or scrolling
  misaligns. There are **two** of them, `list_row_height()` for rows in the interface font and
  `code_row_height()` for rows in the fixed-width one, so a view and its rows have to agree about
  *which*, as well as about the number. Both are functions of the fonts and no longer a `const`, so
  never write a literal row height anywhere; the two halves are safe only because both are read in
  the same render pass. This is also why variable-height rows are not free.
- `Size` has no `From<f32>`; write `Size::px(300.)`. But `.padding`, `.spacing`, `.margin` and
  `.corner_radius` do take plain `f32`.
- `label()` and `paragraph()` do not implement `StyleExt`, so they have no `.background()` /
  `.border()`; wrap them in a `rect()`.
- `spawn` ties a task to the scope it was called in, and a task whose scope is unmounted is
  dropped, before its first poll if that comes first. A handler on something the handler
  itself takes down (a context menu's item, which the press closes) has to `spawn_forever`.
- A `VirtualScrollView` scrolls sideways only as far as the widest row it has built, so a
  row that should be reachable sideways is never `width(Size::fill())`. It is not
  `width(Size::auto())` with a `min_width` either: torin sizes an auto-width node from its
  minimum *plus* its children. The code panes' rows take `Widest::row_width`
  (`src/ui/width.rs`) and report their content through `on_sized`'s `inner_sizes`.
- A bubbling pointer event (`pointer_down`, `press`) is measured once against the deepest
  listener and every ancestor's handler gets the same data, so `element_location()` in an
  ancestor is relative to that child. Nothing inside a code row listens to `pointer_down`
  for this reason (`src/ui/code_row.rs`). And `pointer_over` fires on entry only, whatever
  its doc string says; a sweep that follows the pointer is `pointer_move`.

## Testing the UI

`freya-testing` runs the whole app headless on the test's own thread. The binary's suite runs in
under two seconds, so a test written to settle one point costs less than a `cargo run` and a
look. It can be asked about any control, drag, scroll, keyboard binding, laid-out size, worker
answer, or which component re-rendered; it cannot say how anything *looks*, measure text, or
observe the platform. Keep the tests that pin a mechanism and delete the ones that only proved
the code just written does what it says. A headless test has to be made to fail first on the
mechanism it claims to test. The rest is in `agents/UI.md` and `agents/Headless.md`.

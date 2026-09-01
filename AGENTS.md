## User rules

Standing instructions from the user.

- Committing. Whatever is uncommitted when you start stays uncommitted.
- Don't reference an uncommitted file from a committed one.
- Keep `notes/Goals.md` current. It is the checklist of planned features.
- Prefer TOML for files, not JSON.

## What this is

A desktop GUI ("Assembly Viewer") for inspecting object files: open an ELF/PE/Mach-O object or a
static archive, browse its text symbols, and read a symbol's disassembly — with relocation targets
resolved to clickable symbol names — beside the source it was compiled from. Rust, cargo
workspace, [freya](https://freyaui.dev) 0.4 for the UI.

## Build

`cargo run --features devtools` starts freya's devtools server alongside the app (`[::1]:7354`,
opt-in so it never reaches a release build); the viewer is a separate `cargo install
freya-devtools-app`, only one devtools-enabled freya app can run at a time, and there is no in-app
shortcut. See `notes/DevTools.md`.

The first build compiles Skia (via `freya-engine` -> `freya-skia-safe`) and takes a long time; on
Fedora it needs `freetype-devel fontconfig-devel libglvnd-devel wayland-devel` to link, and `mold`
is not a supported linker.

Dependency versions are pinned by compatibility, not taste: the `tree-sitter-*` grammars must sit
on the `tree-sitter-language` ABI that `freya-code-editor`'s `tree-sitter` uses, and `addr2line`
0.21 / `gimli` 0.28 / `object` 0.32 must stay one copy each — check with `cargo tree -p analysis
-d` after touching any of them. The reasoning is in the `Cargo.toml` comments; keep them current.

### Test fixtures and sample inputs

Almost every fixture is built **in memory** with the `object` and `gimli` writers
(`crates/analysis/tests/common/mod.rs`), so the suite needs nothing on disk and is green in a fresh
checkout. The exception is `crates/analysis/tests/fixtures/`: one small C file and the two objects
`gcc` produced from it, **committed** so the crate is pinned against DWARF a real toolchain emits.
`tests/real_object.rs` reads them and fails loudly rather than skipping when they are missing.

Four files in the repo root are untracked **sample inputs** to open in the app, not build outputs.
Nothing committed may depend on them:

| file | what it is | debug info |
|---|---|---|
| `viewer-sample` | this app's own debug binary, ~331 MB, one linked ELF, ~115k text symbols | DWARF, ~267 MB of it |
| `libanalysis-sample.rlib` | the `analysis` crate's rlib, ~20 MB, 196 archive members | DWARF, per member |
| `LLVM-24-rust-dev.dll` | a prebuilt LLVM, ~137 MB | none — no debug sections, no COFF symbol table, so no text symbols either |
| `librustc_data_structures-*.rlib` | an old rustc rlib, ~3.5 MB, 2 members | CodeView (`.debug$S`/`.debug$T`), not DWARF |

The first two are regenerated with `cargo build && cp target/debug/viewer viewer-sample && cp
target/debug/libanalysis.rlib libanalysis-sample.rlib`. Session state is restored on launch, so
`cargo run` reopens the last binaries on the last symbol and a visual check costs one command.

## Layout

- `crates/analysis/src/lib.rs` — object parsing, disassembly, relocation resolution.
- `crates/analysis/src/line.rs` — DWARF line info, lazy. The only part that knows `gimli`/`addr2line`.
- `src/project.rs` — the persisted session (`project.toml`) and the save policy.
- `src/settings.rs` — the user's own settings (`settings.toml`): the font overrides and the theme choice.
- `src/source.rs` — source files read off disk and cached by path, failures included.
- `src/filter.rs` — what a filter bar is asking for and the matcher it compiles to.
- `src/tree.rs` — the Objects list's tree shape: which objects came from which file.
- `src/lanes.rs` — where each branch is drawn in the assembly view's arrow gutter.
- `src/rows.rs` — the run of rows a reader picks out to copy.
- `src/tabs.rs` — `Tabs<T>`, the open-tab list, with no cursor of its own.
- `src/history.rs` — back/forward navigation history.
- `src/fonts.rs` — the desktop's font settings, asked of KDE, Gnome or the Win32 API.
- `src/ui.rs` — the entire freya UI (~4400 lines, in commented sections).

Everything except `ui.rs` is framework-free and unit-tested rather than eyeballed.

## Analysis

**Parse pipeline** (`open_files` -> `parse_object`): each selected file is first tried as an
`ArchiveFile` and every member parsed as a separate `Object`, then the file itself is *also* parsed
as a plain object — so a non-archive contributes one `Object` and an archive one per member.
Failures are swallowed (`.ok()`), so a file that will not parse just never appears. Reading and
parsing run on a `std::thread` and come back over an `async_channel`, so a large binary does not
freeze the UI.

**Data model** — built once at open time, shared via `Arc`. Only `SymbolKind::Text` symbols are
kept — plus, for a **linked image only**, the code it declares elsewhere: `dynamic_symbols`,
`exports` and `entry` (`declared_code`). All three are *declared*, so this keeps the "nothing is
scanned for" rule; `LLVM-24-rust-dev.dll` has no COFF symbol table at all and goes from zero
functions to 22 918 on the strength of it. One symbol per address, earliest source winning
(symbol table > dynamic symbol > export > entry point), because `Section::symbols` is the sorted
list `estimate_size` searches and a repeated address makes it answer 0. The section comes from
looking the address up in the kept **text** sections, which doubles as the filter keeping exported
*data* out. A relocatable object is skipped entirely: `entry()` answers 0 for a `.o`, and 0 there
is a real function's first byte. The entry point has no name and is called `<entry point>` — angle
brackets because no assembler, linker or mangling scheme emits them, so it cannot collide with a
real one. `Object` holds `symbols: HashMap<SymbolIndex, Arc<SymbolData>>` (for relocation-target
lookup) and `symbols_sorted` (name-sorted, for the UI list). `Object::data` is an `ObjectData` —
an `Arc<[u8]>` of the whole file plus a `Range` — kept for the object's lifetime, because parsing
keeps decompressed bytes only for sections holding text symbols and the lazy line-info pass needs
the rest; every object from one file shares that one allocation, so an archive costs its bytes
once. `Section` owns decompressed bytes, relocations keyed by address, and a sorted list of its
text symbols' addresses. `SymbolData::estimate_size` derives a symbol's extent from the *next*
address in that list — **clipped to the section's own bytes**, since that list is numbers out of
the file and one wild `st_value` in it would otherwise cost the symbol *above* it its listing
rather than only itself. Declared sizes are frequently 0 in ELF/COFF, which is why the
derivation exists at all; the declared size is kept separately and only displayed. `SymbolData::extent` is the answer that is actually used, and
prefers DWARF — a `DW_TAG_subprogram`'s `DW_AT_low_pc`/`DW_AT_high_pc` — where the object has any,
taking the **smaller** of the two: the estimate over-reaches into padding, but `high_pc` describes
the *function*, so a second symbol inside one subprogram (an alias, an assembler label, a split
cold part) would otherwise swallow the next function. The derivation is capped at
`MAX_DERIVED_SIZE` (1 MiB) — not a claim about how long a function can be, but the point past
which it is certainly describing something else: a stripped PE's export table is sparse, so nine
of the DLL's exports derived megabytes and one derived 3.7 MB, which is 772 302 instructions
decoded *per render*. `.pdata`/`RUNTIME_FUNCTION` is the real fix and is its own Goals item.

**Names are demangled in one batch per object, on a stack sized for them** (`demangled`). A
mangled name is bytes out of a string table, and it is the *file* that chooses how deep the
demangler reading it recurses: `msvc-demangler` 0.11 has no recursion limit at all (`P` → pointee
→ type → pointee, one byte per level) and `cpp_demangle`'s is deep enough that reaching it is
megabytes of stack, so a 209-byte name overflows the 2 MiB a `std::thread` gets — and a stack
overflow is an **abort**, which no `catch_unwind` turns back into "this symbol has no demangled
name". Two bounds together: a name over `MAX_MANGLED_NAME` (2048 bytes, against a longest of 1038
across every sample in the repo) is not demangled at all, and the rest are demangled on a thread
with `DEMANGLE_STACK` (64 MiB, a reservation and not a cost) — except where every name in the
object is under `SHORT_MANGLED_NAME` (64), which is every fixture in the test suite and is the
caller's own stack's business. A name no demangler will take is displayed exactly as the file
wrote it, which is what an unrecognised name already did.

**Line info** (`line.rs`) is lazy and never touched at parse time. It answers two questions under
one set of rules — the rows covering a range, and a subprogram's extent (`Object::subprogram_extent`,
one DIE walk per unit visited, cached by unit offset) — so everything below holds for both. `Object::dwarf` is a
`DwarfCache` caching *both* answers — the built `addr2line::Context` and the fact that there is
none — so an object without debug info costs one section-table scan ever. `None` from `line_info`
means "no line info" for every reason at once: no DWARF, foreign debug info (CodeView), DWARF that
will not parse, or DWARF that says nothing about the range asked about. Four design points are
load-bearing:

- **No self-borrow.** Readers are `gimli::EndianArcSlice` — an `Arc<[u8]>` per DWARF section, built
  by copying, decompressing and relocating — so the context owns its data and is `'static` rather
  than making `Object` self-referential. Cost: one allocation per debug section on first query.
- **`Sync` via a `Mutex`.** `addr2line::Context` caches parsed line programs in an `UnsafeCell`
  behind `&self`, so it is `Send` but not `Sync`. `lib.rs` holds a `const _` assertion that
  `Object: Send + Sync` so this cannot regress silently.
- **One query per symbol, not per instruction.** `SymbolData::line_info(&object)` returns an
  `Arc<LineInfo>` for the whole extent; the UI answers each instruction locally with
  `LineInfo::row_at`. Rows are ascending, non-overlapping, clipped to the range, and coalesced —
  but *not* contiguous. `line`/`column` are `Option`, because DWARF line 0 means "no line" and
  column 0 means "left edge". Non-overlapping is an invariant *made* to hold in `line_info_inner`
  by clipping after the sort, not a property of DWARF.
- **An address alone is not a key in a relocatable object.** Sections there have no address until
  linked and rustc emits one `.text.<name>` per function, so every function lands on 0 and the line
  programs pile up (52 229 of 54 109 rows overlapped, measured on `libanalysis-sample.rlib`).
  `line.rs` does what a linker does: `section_biases` gives each **text** section of a
  **relocatable** object a place of its own, `relocate` adds the bias, and the query adds it and
  subtracts it from every row returned. Both limits matter — a linked image holds real addresses
  literally and must be left alone, and an absolute relocation in a debug section is often an
  offset into another `.debug_*` section rather than an address. Hence `Object::line_info` takes a
  `&Section`: a bare range is not a question the crate can answer.

`without_panicking` (a `catch_unwind`) wraps the context build and every query, for two known
reachable bugs, both unchecked arithmetic in `addr2line` 0.21 on numbers a `.debug_*` section
states and neither of them something this crate can validate without parsing the DWARF twice: a
row's length is `next.address - row.address`, so a line program that moves its address backwards
is a subtract-with-overflow panic on a file the user merely opened; and a unit's range is
`low_pc + high_pc`, which overflows for a unit whose length runs off the end of the address space
— that one while the context is being *built*, which is why the guard is around the build too.
What is *not* left to the guard is the third one: `find_units` asks about `probe + 1` unchecked,
so `subprogram_extent` declines `u64::MAX` outright rather than catching the panic afterwards.

**Disassembly** (`SymbolData::assembly`) is hardcoded to 64-bit x86 via
`Decoder::with_ip(64, ..)`, so non-x86 objects decode as garbage rather than erroring. Each
instruction is formatted into an `Instruction` implementing `iced_x86::FormatterOutput`, capturing
`(String, SpanKind)` spans for the UI to colour. `SpanKind` is the backend-independent stand-in for
`FormatterTextKind`; the app has no `iced-x86` or `object` dependency (`BinaryFormat` and
`SectionIndex` are re-exported from `analysis` for that reason). The decode loop's own arithmetic
is checked: the instruction pointer is the symbol's address plus what has been decoded, both of
them the file's numbers, so a section placed at the end of the address space wraps it and the
offset derived from it is a slice index. The listing stops at the wrap rather than indexing past
the symbol.

**Relocation handling** is the subtle part. A relocation whose address falls anywhere in the
instruction's byte range is resolved to an `Arc<SymbolData>`, and the target's name is printed *in
place of* the placeholder operand through iced-x86's `SymbolResolver` hook — not by suppressing the
number, which left the brackets the formatter had already opened empty (`call qword ptr []`).
Nothing maps a relocation back to an operand number, so the resolver is armed once per instruction
and the **first** operand asked takes it; a second numeric operand keeps its real value. A
rip-relative operand keeps its `rip+` only when a name is going into it: `assembly` flips
`rip_relative_addresses` **per instruction**, on exactly those with both a resolved relocation and
a rip-relative memory operand, because `[target]` would otherwise claim an absolute address the
encoding does not have. The option cannot be set per operand — `format_memory` reads the global
one. `Instruction::relocation_span` is the index of the span the name landed in, recorded by an
override of `write_symbol`; that is what lets `InstructionRow` render the run before it as one
`paragraph()`, the span as a `RelocationLabel`, and the run after it as a second `paragraph()`.

**Branch edges** (`Assembly::edges`) are the branches staying inside one symbol, for the arrow
gutter. Both ends are **indices into `instructions`**, not addresses, because that is what a row
can be asked about and it makes the answer independent of where the symbol sits. `from`/`to` are in
*execution* order (a backward branch has `from > to`); `first()`/`last()`/`is_backward()` sit on
top. A **call is not an edge** even when it lands inside the symbol, because control comes straight
back. Four things are dropped rather than drawn, each of which would be a line to a place it does
not point at: a branch out of the symbol, one landing mid-instruction, one whose displacement is a
relocation placeholder (tested on the *raw* relocation lookup, since a branch relocated against a
section carries no text symbol while its displacement is just as meaningless), and `jmp $`.

**"Never panic on any file input" is tested two ways, and they are different jobs.**
`tests/mutations.rs` is the **search**: it takes every fixture the suite builds — both committed
gcc objects, the synthesized DWARF one, the ELF `.so` and the PE DLL — and truncates it at every
length, writes poison values (`0`, `u32::MAX`, `u64::MAX`, the file's own length…) into every
numeric field of every header, section header, symbol and relocation, and splats pseudo-random
runs over it, running the whole pipeline over each result. It is sampled by an even stride and
seeded from a constant (never `rand`, never the clock), so which cases run is fixed and it stays
under two seconds. `tests/robustness.rs` is the **regression suite**: one named, minimal fixture
per defect that was actually found, because a sweep that goes green tells you nothing about which
bug it was that stopped happening. `common::parse_and_walk` is the one definition of "ask a parsed
object everything", shared by both. The rule that goes with them is `notes/Goals.md`'s: a minimal
test case every time something is found wrong, and **checked arithmetic in preference to a wider
`catch_unwind`** — the guard is for a dependency's bug, never for ours. Note also what *cannot* be
caught: a stack overflow aborts, so anything recursing over file-controlled input (the demanglers,
above) has to be bounded before the call rather than wrapped.

## Persistence

There is **no published version of this app yet**, so persisted formats need no backward
compatibility: a schema change is just a schema change, a stale file is ignored rather than
migrated, and `#[serde(default)]` is added only when it earns its place on its own merits.

The session is `project.toml` under `dirs::state_dir()` (falling back to `data_local_dir()`) +
`assembly-viewer/`, written atomically via `.tmp` + rename. `Project` holds the opened `binaries`,
the content area's open `tabs`, the `selection` and the `history`, all as **path + object name +
symbol name + address**, never pointers; that mapping lives in exactly two places,
`SavedSelection::from_selection` and `::resolve`. Beside them sit the Source pane's open
`sources` — `String`s, since they are what the debug info said rather than paths this filesystem
was asked about — and the `shown` index into them. Each open tab also carries **the row it was
left at**: a `tabs` entry is a `SavedTab` (`row` + `selection`) and a `sources` entry a
`SavedSource` (`row` + `path`), rather than either list having an array of rows beside it. The row
travels with its tab because `resolve_tabs` *drops* the tabs that no longer resolve, which would
shift every later row of a parallel array onto the wrong tab. It is a row and not a pixel offset
so that a later `ROW_HEIGHT` (Step 9's fonts) does not move every saved position, and it is a
hint and not a fact — `#[serde(default)]`, and clamped to what the tab holds *now* by
`Positions::row`. **Field order within these structs is load-bearing**: TOML emits plain values
before tables, so `binaries`/`shown` must precede `selection`/`tabs`/`sources`/`history`,
`SavedTab::row` must precede its `selection`, and `SavedHistory::cursor` its `entries`. Getting it
wrong fails at *runtime*, not at compile time.

Coming back, the **selection degrades** (symbol -> its object -> nothing, since there is one of it
and the app must open somewhere) while **history entries are dropped** (a list of places the reader
cannot get back to is worse than a short list). `History::rebuilt` is the one walk both a restore
and a file-close go through, carrying the cursor to the last survivor at or before it.
`History::restored` also collapses duplicates and trims to the newest `MAX_ENTRIES` (200).

**When** a save happens is `Saves` in `project.rs`, a `static Mutex` rather than UI state because
two of the three things driving it sit outside the component tree. `record(project)` is called on
every state change: a change to `binaries` writes **immediately**, carrying anything pending with
it; a change to only the selection, a tab or the history marks it **pending** — a tab because it
is expressed against the binaries rather than the other way round, costs one click to remake, and
arrives on every navigation, `activate` opening one on the way to each selection change.
`flush()` writes what is pending — on a 30s timer and from the window's close hook, which is the
one exit hook freya 0.4 offers (`WindowConfig::with_on_close`, a `Send` callback that cannot read
any `State`, which is exactly why the policy is a static). `Saves::written` starts as
`Project::default()`, **not** as the project loaded at startup, so nothing is pending before
something is actually opened — seeding it from the loaded project would write an empty project
over a good one.

Both tab strips are restored, and **through the five functions that hold the invariants** rather
than by writing either list: `use_restore_on_startup` sets the history, `activate`s each content
tab and then the selection, and `open_file`s each source file and then the shown one. Three
orderings are load-bearing. Each strip's **rows go into its `Positions` map before its tabs are
opened** — that map is the one thing the restore writes directly, and a pane puts its view back
when it notices the tab it is showing has changed, so a row arriving after the `activate` arrives
after the only moment anything looks at it. Tabs before the selection, because `activate` opens
what it cannot find and would otherwise append the restored selection at the end of the strip
instead of finding it in place (the other direction is safe: a selection that degraded to its object while the strip
holds the symbol simply opens a tab). And the shown file last, because `open_file` puts the pane
on whatever it opens. A content tab that no longer resolves is **dropped**, like a history entry;
a source file is never resolved at all, so one that has been deleted comes back as a tab over the
pane's own "Source file not found" rather than silently vanishing.

**The settings are a second file, not a section of the first** (`src/settings.rs`, `settings.toml`
beside `project.toml`, same directory, same atomic `.tmp` + rename, same "a missing, unreadable or
corrupt file is simply the default"). This is the *user-given settings* half of the storage split
`notes/Goals.md` asks for under *Projects*: the session is what the app **noticed** and changes on
every click, a setting is what the user **said** and changes when they say so, so they have
different rates, different save policies and different consequences when one of them will not
parse. `Settings` is the theme choice (`Theme`: light, dark or follow the desktop) and a
`FontSetting` — a family and a size — for each of the interface and fixed-width fonts. **Every
field is an `Option` and `None` is a real third state**: "the user has not said, ask the desktop",
which is neither an empty string nor the desktop's current answer copied into the file. An
unspecified field is therefore a key that is *absent* from the TOML (`skip_serializing_if`, since
TOML has no null anyway), so nothing can later mistake an inherited value for a chosen one, and the
settings page can show the difference. Sizes are stored in **points**, the unit the desktops answer
in, so an override and the value it overrides are comparable; `fonts.rs` converts once at the end.
Field order is load-bearing here too — `theme` is a plain value and the two fonts are tables — and
the round-trip test is what holds it. There is **no `Saves`-shaped policy and deliberately no second
autosave timer**: a settings change is already as rare as a deliberate action, so `Settings::save`
is public and writes at once. `Theme::appearance()` asks the desktop which it prefers, in the spirit
`fonts.rs` asks it for fonts (KDE's `Colors:Window/BackgroundNormal` luminance, then the scheme
*name* only when the name says so; Gnome's `color-scheme`, whose `default` is *not* an answer;
macOS's `AppleInterfaceStyle`; Windows is a named hole needing a `windows-sys` feature), and it is
deliberately uncached, since "follow the desktop" is a question and not a value. `fonts()` merges
the settings over the desktop's answer **field by field** (`fonts::resolve`, pure and tested), but
is still a `OnceLock`: the doc comment on it says exactly what the settings page has to change and
why a re-readable `fonts()` alone would not be the fix.

## UI (freya 0.4)

freya 0.4 is **not** Dioxus-based: no `rsx!`, no `#[component]`, no `use_signal`. It is a builder
API (`rect().width(Size::fill()).child(..)`) over its own `freya-core`. Most freya material online
describes the older API and does not apply.

**State** is a handful of `State`s provided at the root with `use_provide_context` and read with
`use_consume`: `Objects`, `Sel` (the active `Selection`), `Open`/`Files`/`Shown` (open tabs),
`AsmAt`/`SrcAt` (where each of those tabs was left), `Hist`, `Focused`, `Pinned`, `Marked`/`Shift`,
plus the memos `Symbols` and `Lines`.

**`Sel` is the active tab.** `Open(State<Tabs<Selection>>)` is the *list* of open tabs and holds no
cursor: the active one is whichever entry equals `Sel`, which is well defined because no two tabs
are equal. `Selection` is what the history records and the session saves, and a list with a second
answer to "what is on screen" would be two states to keep in step. `Files`/`Shown` mirror this for
the Source pane. Both invariants — the selection is one of the open tabs (or `None`, which is
exactly "nothing open"), and a file is shown exactly when one is open — are held by five functions
and nothing else: `activate`, `close_tab`, `open_file`, `close_file`, `close_binary`. **Every** site
that would set the selection calls `activate`, `navigate` included, because the history keeps an
entry long after its tab was closed.

**Layout** is a toolbar over a `ResizableContainer`: a `PanelSize::px(300.)` sidebar and a
`PanelSize::percent(100.)` content pane, mixing the two sizing modes deliberately so the sidebar
keeps a fixed width and the content takes the rest, with freya's 4px `ResizableHandle` between
them. `ResizableContainer` renders itself `.expanded()`, so it needs a parent already sized —
`Size::flex(..)` only works under a parent with `.content(Content::Flex)`. The content panel is a
flex column of `TabStrip` over the `DockingArea`, so the open-document chips sit above **both**
panes: the active tab is what the assembly *and* the source show, and a strip inside one of them
would follow that pane wherever it was docked.

Inside each panel is a `DockingArea` over a `DockArea` model; the six views are dockable tabs
draggable between the two areas (both use `Tab` as the payload, and `use_drag` keeps one
`DockDrag<Tab>` at the root). The outer split stays a `ResizableContainer` because docking cannot
express a literal 300px. A drag carries only the tab, so the area receiving a drop evicts it from
the other through a wired-up `other: Option<State<DockArea>>`. `root()` never returns `None` — an
area losing its last tab collapses to a single *empty* panel (`DockArea::tidy`) so tabs can be
dragged back in. A tab is a **persistent view**, not a slot the selection drives: each of the six
is a unit `Component` that consumes context and renders off the current `Selection` itself, so a
selection change re-renders only the tabs that read it and never the root.

**Tab strips are not dock tabs.** The six `Tab`s are *views*; the chips in a strip are the
*documents* open in them. Open functions are not dock tabs because the dock tree is the *layout*
(closing the last one would fold the split away), because a per-panel active tab gives two answers
to "which function is active", and because the list would then be inseparable from a layout nothing
persists. A chip's name is elided **by character count in Rust**, where every other truncation is a
width: a `maximum_width` anywhere inside a chip makes it shrinkable, and a horizontal scroll view
measures children against the space *left*, so chips past the edge get no width and draw as a bare
×. Do not "fix" that back into a width.

**Each tab remembers where it was left.** A pane has one `ScrollController` and shows one tab at a
time, so left alone it hands the tab arriving whatever offset the one leaving had. `AsmAt`/`SrcAt`
are two root `Positions` maps beside `Open`/`Files`, keyed by the very values those lists hold — so
an entry means "this tab" for exactly as long as the tab is open — and `use_kept_position` is the
whole of the behaviour, called once by `InstructionList` (keyed by the `Selection`) and once by
`SourceList` (keyed by the file it is showing). What is kept is a **row**, clamped to what the tab
holds *now*, so a rebuilt binary or a shortened file cannot come back past the end. Three things are
load-bearing. Reading the controller's position (`<(i32, i32)>::from`) is a `State::read`, which is
what **subscribes the effect to the pane's own scroll**: every position is written down as it
happens rather than on the way out, which is what survives the window merely being closed. The tab
the controller is *holding* is tracked in the hook — an `Rc<RefCell>`, not a `State`, since nothing
renders from it — because it is not the tab the app is showing during the one run that has to move
the view, and every write goes under the held one. And a `Pin::reveal` **wins** over a remembered
position with nothing written to make it: the two are never owed at once, since this moves the view
only when the tab changes while a click asking for a reveal changes no tab (and a selection change,
which does, drops the pin), and when a reveal scrolls, the effect wakes on that scroll and records
where it landed. `close_tab`/`close_file`/`close_binary` forget a tab's position with the tab, which
is not tidiness: a `Selection` key holds the `Arc<Object>` it points into — and the hook is handed
the tab list precisely so that the run *after* a close, still holding the tab that has gone, cannot
put it straight back.

**The Source pane** shows one of its open files, chosen by `Shown`, and follows the selection:
`use_open_source_file` opens the file the active symbol was compiled from — only the symbol's *own*
file, never the rest of `LineInfo::files`, since a Rust function inlines dozens. Its other input is
`Lines(Memo<SymbolLines>)`, which carries **both** the `LineInfo` and the file to open on; that
pairing is load-bearing, because a `Memo` recomputes in a spawned task and an effect reading `Sel`
and `Lines` together sees them disagree for one beat.

The rows are the app's own (`SourceRow`, a `VirtualScrollView`), **not** freya's `CodeEditor`,
which paints a line background only for the cursor's row and keeps its scroll state private. What
`freya-code-editor` does offer is its tree-sitter pipeline, public on its own: `SyntaxHighlighter` +
`SyntaxBlocks` + an `EditorSyntaxTheme` turn a `Rope` into one list of `(Color, TextNode)` spans per
line. The theme is the app's own (`Palette::syntax`), the grammars are ours, and an unknown
extension degrades to one plain span per line. A file is parsed once when loaded and cached in a
`static` in `ui.rs` — parsing is stateful across lines, so it cannot be per row. Two things about
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
carries a `FocusOrigin`; and **the scroll is a request, taken once** — `take_reveal` *removes* it,
so a repeat click is a second request, and `reveal_row` does nothing when the row is already on
screen. None of this is a navigation: the selection does not change and nothing is pushed onto the
history. `navigate` remains the only path for anything that does.

**The arrow gutter** draws every branch staying inside the symbol, with the layout in `src/lanes.rs`
because a `VirtualScrollView` builds row *n* knowing nothing but *n* — a row has to be *told* which
lines pass through it. `Lanes::new` is called in `AssemblyView`, deliberately not in a `use_memo`,
which would land a beat after the disassembly it belongs to. Lanes are assigned **greedily,
shortest span first**, which makes nesting a consequence rather than a rule; two branches sharing
only a row still take two lanes, or a top half and a bottom half in one lane would read as a line
passing through; and the gutter is capped at `MAX_LANES` (5) with the outermost lane **shared**
past that, since the corner and the arrowhead survive sharing and only the joining line goes
ambiguous. It is drawn with **rects**, not `canvas()`, whose `RenderCallback` has a `PartialEq`
returning `true` unconditionally — exactly wrong for a row a scroll view recycles. `InstructionRow`
therefore pads horizontally only: a line must reach the row's top and bottom edges or the column
comes out dashed. Hovering a row draws its own branches darker, which needs a row *index* in
`InstructionList` rather than `Focused` — a source position is many rows.

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

**The three sidebar lists filter themselves.** `FilterBar` is one component with three uses, and
the `Filter` is a `use_state` in the owning tab rather than a root context — a filter is a view of a
list, never part of the session. `filter.rs` compiles every filter to one `regex::Regex`, plain
patterns included, because the three toggles *are* three regex constructs: a `RegexBuilder` flag
for case (so a pattern's own `(?i)` still wins for the part it covers), `\b(?:…)\b` for whole word
(the non-capturing group is load-bearing), and escaping on the way in for the third. That is also
the faster answer — 3 ms against `str::contains`'s 3.7 ms over 151k names. An uncompilable pattern
is `Matcher::Invalid`, a third answer that matches nothing *and* prints the reason, because
matching everything hides a half-typed `(`. The toggles call `prevent_default` on their press, or
an `Input` gives up its keyboard focus mid-word. Only the Symbols list needs a memo (`Filtered`,
holding indices, and `None` for the unfiltered case so it costs what it did before there was a
filter); Objects and History filter where their rows are built.

**The Objects list is a tree** (`src/tree.rs`). `ObjectTree::new` groups objects by *consecutive
runs* of equal `Object::path` and flattens the result into `TreeRow`s — the tree is a shape in the
data, never in the element tree, because a `VirtualScrollView` is told a length and asked for row
*n*. A file contributing exactly **one** object is its own row and grows no parent. Filter and
folds interact by one rule: a file row is never hidden while a row under it is visible, so a file
shows when its own name matches *or* a member's does — and those are different answers. File
matched keeps the reader's fold; members matched forces the row open (`Expansion::Forced`, a third
state, drawing no disclosure triangle) since a search that folds its results away has answered
nothing. Each row wears a text tag (`ELF`/`PE`/`COFF`/`MACH`/`AR`) rather than an icon, because
nothing in Lucide's 1640 icons names an object format.

**A file row is also how a binary is closed** — right-click opens a `ContextMenu` (which needs the
`ContextMenuViewer` mounted at the root of `app()`; opening one without it panics) on a single
"Close file". A member row offers nothing: the unit that closes is the file. `close_binary` is
composed of three rules from the modules that own them — `Selection::in_file`, `Tabs::close_all`,
`History::retaining` — and three decisions inside it matter: the selection **follows the tabs**
rather than degrading (a file takes its objects and their symbols together, so there is nothing to
fall back to); the history **drops** through the same `History::rebuilt` walk a restore uses, so
the two cannot drift; and the unit is the **path**, so one file opened twice closes once.

**Tooltips** are how a truncated row is read, so `row_tooltip` sets the delay to `Duration::ZERO` —
freya's 500ms default makes sweeping down a list useless. The filter toggles keep the default
(their tooltip explains what `\b` means), and the code rows have none.

**One palette, one place.** Every colour is a field of `Palette` in `ui.rs`, there is one instance
(`Palette::LIGHT`), and `palette()` is how anything reaches it — no call site names a colour. The
indirection is the point: a dark mode is asked for in *this* palette, so it is one more `const`,
something that re-renders on a switch, and a `HIGHLIGHTED.clear()` — that cache holds
`SyntaxBlocks` with colours already resolved into them, so its entries would not be stale but the
wrong theme. The code colours are named for what they mean, not for the pane they came from, and
`Palette::syntax` maps `freya-code-editor`'s ~33 capture fields onto them. Beware
`resolve_capture_color`: it treats a capture whose colour equals `text` as unmapped and walks *up*
the dotted name, so giving a child field the text colour while its parent holds another silently
paints the child in the parent's colour. This is deliberately **not** freya's own theming —
`ColorsSheet` names none of these roles, and the source pane's colours cannot be read from the
element tree at all, being baked into a `SyntaxBlocks` when a file is *loaded*.

**Fonts.** `fonts()` asks the desktop for its interface and fixed-width fonts and converts points
to pixels. **Which desktop to ask is a runtime question**, not a compile-time one — one Linux build
runs on both — so `XDG_CURRENT_DESKTOP` only *sorts* `kreadconfig6`/`kreadconfig5` (KDE's `font`
and `fixed`, a comma-separated spec) against `gsettings` (Gnome's `font-name` and
`monospace-font-name`, a quoted Pango `Family Size` whose family can hold spaces and trailing style
words), and the other is tried anyway: a tool that is not installed is already a `None` here.
Gnome's `text-scaling-factor` multiplies the point size, because it is *how* Gnome says "make text
bigger" — `font-name` keeps its nominal size and the accessibility slider moves this instead;
winit's own display scale is separate and multiplying both would compound. Windows is `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)`
for `lfMessageFont`, over a `windows-sys` pinned to the 0.61 the lock already holds transitively
so no fourth copy of it appears (`cargo tree -d`). Its `lfHeight` is divided by the screen DC's
`LOGPIXELSY` rather than by `SystemParametersInfoForDpi`, deliberately: that function and
`GetDpiForSystem` only exist from Windows 10 1607, and `windows-sys` links its imports statically,
so naming one would turn "no font setting" into a process that will not start — winit itself
`GetProcAddress`es that family for the same reason. The pairing is also what makes it *correct*:
both the metrics and the DC's DPI are virtualised into whatever DPI space the process is in, so
they agree without this file knowing which that is. Windows stores no desktop-wide monospace font
at all, so that half stays `Consolas`. Each font is then a *chain*:
the desktop's answer in front of the platform's own (`Segoe UI`/`Consolas`,
`.AppleSystemUIFont`/`Menlo`, else the generic `sans-serif`/`monospace` that skia resolves through
fontconfig). A family named with no usable size keeps the family and takes the app's default size.
The platform font must be named — freya's global fallbacks are all proportional, so a
chain resolving to nothing silently takes the assembly view out of a monospaced face — and must
equally not name *another* platform's families, which had a Windows box rendering in DejaVu. The
one font freya will not let an element set is the tooltip's, hardcoded in its theme, so `app()`
provides a `Theme` with `tooltip.font_size` at the interface size.

**Identity throughout the UI is `Arc` pointer identity**, not names or indices: list keys are
`Arc::as_ptr(..).addr()` and every prop `PartialEq` is hand-written in terms of `Arc::ptr_eq`. That
matters twice — duplicate symbol names across objects stay distinct, and `#[derive(PartialEq)]` on
an `Arc<T>` field would deep-compare on every parent render.

### Gotchas before editing `ui.rs`

- A `State`'s `peek`/`read` hands back a guard, and an `if let` holds its scrutinee's temporary
  until the end of its **body** — so `if let Some(x) = *state.peek() { state.set(..) }` compiles and
  panics the moment it runs. `let ... else` and `match` end theirs with the statement. Bind the read
  to a `let` of its own before any write. The single headless test in `ui.rs` (`freya-testing`, a
  dev-dependency for this and nothing else) exists because that class of bug is invisible to every
  other test in the repo.
- There is no `.hover()` pseudo-state. A hoverable row is a `Component` with `use_state(|| false)`
  plus `on_pointer_over`/`on_pointer_out` (`over`/`out`, not `enter`/`leave`, so hovering a child
  keeps the highlight).
- `VirtualScrollView`'s builder closure is never compared across renders, so anything the rows
  depend on must go through `new_with_data`, not be captured.
- `ROW_HEIGHT` must equal the `item_size` given to each `VirtualScrollView`, or scrolling
  misaligns. This is why variable-height rows are not free.
- `Size` has no `From<f32>` — write `Size::px(300.)`. But `.padding`, `.spacing`, `.margin` and
  `.corner_radius` do take plain `f32`.
- `label()` and `paragraph()` do not implement `StyleExt`, so they have no `.background()` /
  `.border()`; wrap them in a `rect()`.

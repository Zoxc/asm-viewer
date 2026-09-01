## User rules

Standing instructions from the user.

- Committing. Whatever is uncommitted when you start stays uncommitted.
- Run rustfmt over every file you modified, before committing it. Format **only those files** —
  `rustfmt --edition 2021 <paths>` — and not the workspace: a bare `cargo fmt` reformats
  everything, and much of this repo predates anyone running it, so it drags unrelated reflow into
  a diff that then has to be picked apart by hand. The `--edition 2021` is load-bearing; plain
  `rustfmt` parses as 2015 and will mangle what it cannot read. Nothing here needs a
  `rustfmt.toml`: the defaults are what the formatted files already follow.
- Don't reference an uncommitted file from a committed one.
- Keep `notes/Goals.md` current. It is the checklist of planned features.
- Prefer TOML for files, not JSON.
- Add a minimal test case every time something is found wrong with binary inspection.
- Answer a question about the UI with a headless test rather than by launching the app — a
  throwaway one, deleted once it has answered, is fine. See `agents/Headless.md`.

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
- `crates/analysis/src/disasm.rs` — the disassembler seam; `disasm/x86.rs` is the only `iced-x86`.
- `src/project.rs` — projects: their identity, the two files each is stored in, and the save policy.
- `src/settings.rs` — the user's own settings (`settings.toml`): the font overrides and the theme choice.
- `src/source.rs` — source files read off disk and cached by path, failures included.
- `src/scratchpad.rs` — a scratchpad: the cargo package generated around one source file, and its build.
- `src/filter.rs` — what a filter bar is asking for and the matcher it compiles to.
- `src/tree.rs` — the Objects list's tree shape: which objects came from which file, and
  which files are still being read into it.
- `src/lanes.rs` — where each branch is drawn in the assembly view's arrow gutter.
- `src/rows.rs` — the run of rows a reader picks out to copy.
- `src/docs.rs` — `Docs`, the table mapping a dock tab's `DocId` to the document it stands for.
- `src/tabs.rs` — `landing`, the rule a close obeys, and `Positions`, where each tab was left.
- `src/history.rs` — back/forward navigation history.
- `src/fonts.rs` — the desktop's font settings, asked of KDE, Gnome or the Win32 API, merged
  under the user's own; in points until one conversion at the end.
- `src/ui.rs` — the freya UI's root: its prelude, the list of its files, `toolbar` and `app`.

The UI is a directory, and each of its files is a **cut out of what was one 8 700-line
`ui.rs`** rather than a boundary designed from scratch — what each holds is what that file's
section banners and this document already said belonged together. Two mechanical points
carry across all of them and are written out in `src/ui.rs`'s own `//!` header: the imports
there are `pub(crate) use` and every file begins `use super::*;`, so each keeps the set of
names it had as a section; and each `mod x;` is followed by a `pub(crate) use x::*;`, so a
name means what it always meant wherever it is written. Visibility is what the compiler
asked for and no more, so the annotations *are* the list of what crosses a boundary.

- `src/ui/metrics.rs` — every measurement no component owns, and the fonts they follow.
- `src/ui/palette.rs` — every colour, the theme it is resolved from, and the two compositing
  rules that turn a wash and a span kind into one.
- `src/ui/state.rs` — the contexts provided once at the root and read with `use_consume`.
- `src/ui/analyzed.rs` — the analysis worker, what it answers with, and the supersession rule.
- `src/ui/focus.rs` — the two panes pointing at each other, and where each side of each tab
  was left.
- `src/ui/marks.rs` — the run of rows a reader picks out, and what Ctrl+C copies.
- `src/ui/highlight.rs` — a source file parsed once when loaded, and the cache holding it.
- `src/ui/filter_bar.rs` — one filter bar, its three toggles, and the Symbols list's memo.
- `src/ui/documents.rs` — what opening, closing and moving between documents means.
- `src/ui/sidebar.rs` — the three lists a binary is browsed with, the Info pane, and their rows.
- `src/ui/assembly.rs` — the assembly side of a document: the rows, the gutter, the pane.
- `src/ui/source_view.rs` — the source side of one, and which file it is showing.
- `src/ui/dock.rs` — the dock, its two-kinded tab, and the strip that is its document panel's
  own tab bar.
- `src/ui/project_view.rs` — which project is open: the pane, the switch and the save policy's
  observers.
- `src/ui/settings_view.rs` — the settings page, and the three hooks behind the theme and fonts.
- `src/ui/pad.rs` — the scratchpad and the one worker thread that owns its directory.
- `src/ui/pad_view.rs` — the scratchpad's pane: the editor, the crates, the diagnostics, the output.
- `src/ui/parts.rs` — eleven small stateless pieces of drawing shared by panes with nothing
  else in common.

Five of the names are not the obvious one, and each avoids shadowing a crate module the
prelude has already brought in: `ui::source_view` (not `source`), `ui::project_view` (not
`project`), `ui::filter_bar` (not `filter`), `ui::pad` (not `scratchpad`) and `ui::analyzed`
(not `analysis`, which is the crate `ui/tests.rs` calls into). One name genuinely collides:
`freya::prelude` exports a `use_theme` of its own, so `ui/tests.rs` names ours explicitly —
an explicit import wins over a glob, and that line is the disambiguation rather than a
duplicate.

Everything except the UI is framework-free and unit-tested rather than eyeballed. **A module's tests
are a file of their own** — `src/<module>/tests.rs`, declared `#[cfg(test)] mod tests;` at the
foot of `src/<module>.rs` — so the module a reader opens is the module and not the module plus
half again of what it is asserted to do. The path a test is named by (`project::tests::…`) is
unchanged, which is the point: it is where the file sits and not what the module tree looks like.

## Analysis

**Parse pipeline** (`open_files_streaming` -> `parse_object`): each selected file is first tried as
an `ArchiveFile` and every member parsed as a separate `Object`, then the file itself is *also*
parsed as a plain object — so a non-archive contributes one `Object` and an archive one per member.
Failures are swallowed (`.ok()`), so a file that will not parse just never appears. Reading and
parsing run on a `std::thread` and come back over an `async_channel`, so a large binary does not
freeze the UI.

**Objects are handed over as they are parsed, not collected.** `open_files_streaming(paths, emit)`
calls `emit` with a `Progress` per event — `Parsed(object)` and, once per path *whatever* came of
it, `Finished(path)` — and there is deliberately no *start*: the caller supplied the paths and they
are walked in order, so the only thing it cannot already know is when one is done with. A callback
and not a channel or an iterator, because the crate stays framework-free: a channel would make it
pick one and pick bounded or unbounded, which is a backpressure policy belonging to whoever draws
the result, and an iterator would mean self-borrowing the file's bytes across a yield. `emit`
answers a `ControlFlow`, which is how a walk nobody is waiting for stops where it stands — a closed
331 MB file is not parsed to the end into a value that will be dropped. Its one honest limit: a
single answer cannot say "skip the rest of *this* file but go on to the next", so a multi-file
request in which one file is closed goes on parsing that file and drops the rest at the caller.
`open_files` is that same callback closing over a `Vec`, for the tests and anything with nowhere to
put objects one at a time. The digest stays **one pass per file** — `ObjectData::whole_file` is
built once at the top of each path and every member is cut from it — which is the thing streaming
must not quietly turn into 196 hashes of the same 20 MB.

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
once. It also carries the file's `FileDigest` — xxHash64 of the *whole file*, taken once in
`ObjectData::whole_file` because the bytes are in hand there and an archive member is cut from that
same value, so 196 members cost one pass (1.5 ms against the 129 ms `open_files` takes on
`libanalysis-sample.rlib`; 32 ms against 1.6 s on the 331 MB `viewer-sample`). Nothing in the crate
reads it: it exists so a restore can tell the file it saved from one rebuilt underneath it.
`Section` owns decompressed bytes, relocations keyed by address, and a sorted list of its
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

**Demangling is single-threaded, and it is the last of the open-time cost.** After the lazy
line info, the lazy DWARF context, the lazy subprogram extents and the worker-thread
disassembly, it is the only expensive thing left at open time that is not simply reading the
file: 281 ms of `viewer-sample`'s 1 437 ms, 78 ms of `libanalysis-sample.rlib`'s 181 ms
(release; debug numbers overstate every one of these and are not worth quoting). What is left
beside it — the read and the walk of the symbol table — *is* the objects and symbols lists.

`demangled` is one sequential `map` per object and `open_files_streaming` walks the objects in
order, so an archive's 196 members demangle one after another on one core. Both levels are
embarrassingly parallel and neither is exploited; that is `notes/Goals.md`'s open
"multi threaded" item, and the constraint on it is the 64 MiB stack long names need, so a
parallel version wants a bounded pool of big-stack threads rather than one per object.

**Persisting the demangled names was built and thrown away** — it is not in the history, and the
`[D]` item in `notes/Goals.md` is the whole record of it. It worked and it was measured (a 37% average open, best on archives, worst on
the export-heavy DLL), but it cost a fourth place on disk, an eviction budget and a hand-rolled
binary format with its own checksum, and its corruption sweep found two ways for it to be wrong
including a plausible wrong function name on screen. Parallel demangling attacks the same cost
without persisting anything, and is the thing to try first.
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
  `Object: Send + Sync` so this cannot regress silently — beside three more, `Symbol`,
  `Assembly` and `LineInfo`, which are what crosses into the app's analysis worker and back
  (`ui/analyzed.rs`, `use_analysis`). A field that stops being shared-safe is then a compile error in
  the crate rather than a borrow error in the UI, where the cheap fix would be to go back on
  the UI thread.
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

**Disassembly** (`SymbolData::assembly`) goes through a seam — `disasm.rs` defines everything a
caller sees and names no backend, `disasm/x86.rs` is the only module in the crate that mentions
`iced-x86`. **Which decoder is used is a property of the file, not of this crate**:
`Object::architecture` comes out of the header and `disasm::disassembler` maps it to a backend —
`I386` at 32 bits, `X86_64`/`X86_64_X32` at 64. Bitness comes from the architecture and not from
`is_64()`, because the x32 ABI is 64-bit *code* with 32-bit pointers and is the one case the file's
class gets backwards. An architecture no backend claims is a **third answer**, not an empty listing:
`Assembly::undecodable` carries the architecture's name. It has to be *said*, because there is no
byte sequence a decoder could refuse on — the same bytes that are an aarch64 function are a
confident page of x86 nonsense, which is what this used to print, and an empty listing on its own
reads as a symbol that holds no code. `assembly` still answers `None` for one thing only: a symbol
with no bytes at all. Nothing in the UI reads `undecodable` yet, so such an object currently draws
an empty pane rather than the reason.

The trait is **one call wide** (`Disassembler::disassemble(&Code) -> Decoded`) and is shaped by what
`Assembly` needs rather than by what a disassembler library offers. `Code` is the bytes, the address
they sit at, and one question — `Code::relocation`, asked per instruction, because a relocation
names a byte range and never an operand number. `Decoded` is the rows plus each branch's *address*;
turning those into row indices is `Assembly::decode`'s binary search, so `edges`' drop rules hold
for every backend rather than once per backend. What stays *behind* the seam is everything x86
spells its own way: the `SymbolResolver` substitution, the per-instruction `rip_relative_addresses`
flip, `branch_target`'s flow-control judgement and `FormatterTextKind -> SpanKind`. Each instruction
is formatted into an `Instruction` implementing `iced_x86::FormatterOutput`, capturing `(String,
SpanKind)` spans for the UI to colour. `SpanKind` is the backend-independent stand-in for
`FormatterTextKind`; the app has no `iced-x86` or `object` dependency (`BinaryFormat`, `Architecture`
and `SectionIndex` are re-exported from `analysis` for that reason). The decode loop's own arithmetic
is checked: the instruction pointer is the symbol's address plus what has been decoded, both of
them the file's numbers, so a section placed at the end of the address space wraps it and the
offset derived from it is a slice index. The listing stops at the wrap rather than indexing past
the symbol.

**Relocation handling** is the subtle part, and all of it is x86's (`disasm/x86.rs`). A relocation
whose address falls anywhere in the instruction's byte range is resolved to an `Arc<SymbolData>`, and the target's name is printed *in
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
can be asked about and it makes the answer independent of where the symbol sits. A backend hands
back the *address* each branch names — a forward branch names one no instruction has been decoded
at yet — and `Assembly::decode` resolves them, which is what keeps the four rules below one
decision rather than one per backend. `from`/`to` are in
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
object everything", shared by both. The rule that goes with them is the user rule above: a minimal
test case every time something is found wrong, and **checked arithmetic in preference to a wider
`catch_unwind`** — the guard is for a dependency's bug, never for ours. Note also what *cannot* be
caught: a stack overflow aborts, so anything recursing over file-controlled input (the demanglers,
above) has to be bounded before the call rather than wrapped.

## Persistence

There is **no published version of this app yet**, so persisted formats need no backward
compatibility: a schema change is just a schema change, a stale file is ignored rather than
migrated, and `#[serde(default)]` is added only when it earns its place on its own merits.

Everything is written under `dirs::state_dir()` (falling back to `data_local_dir()`) +
`assembly-viewer/`, atomically via `.tmp` + rename (one `write_atomically`, used by every file
`project.rs` owns).

**A project is a directory, and its id is that directory's name.** More than one exists;
each is `projects/<id>/`, and `ProjectId` is a validated single path component — ASCII
alphanumerics, `-` and `_`, first character alphanumeric — because it is interpolated into a
path and is read back out of a file a user can edit. An **anonymous** project is one whose
`name` key is simply *absent*, the way an unspecified font is in `settings.rs`; its id is the
first free `project-N`, claimed by a `create_dir` that **fails rather than opens**, so the claim
is one atomic operation rather than a listing followed by a race. It survives a restart because
it is a directory, and it costs the user no decision. The `project-N` spelling carries no
meaning: naming a project later does not move it. A project directory is created by the **first
write that has something to say** (`open_project`, reached only from `record`/`flush`), so a run
in which nothing was ever opened leaves nothing behind.

**Each project is two files, and the line between them is the one the save policy already
drew.** `project.toml` is what the user *said* — `name`, `directory`, `binaries` — and is written
**at once**, because a binaries change is what `Saves` writes immediately. `session.toml` is what
the app *noticed* — `shown`, `digests`, `selection`, `tabs`, `sources`, `history` — and is the
file rewritten every thirty seconds. So the file a user might keep, copy or hand-edit is exactly
the one that changes only when they do something. Three things follow, and they are why it is two
files rather than two tables: a `session.toml` that will not parse loses a scroll position and
not the list of binaries; the directory *is* the project, so a run killed between `create_dir` and
the first write reopens as the empty project it is rather than being orphaned; and a binaries
change writes **both**, so `session.toml` can never name a tab into a binary `project.toml` has
already let go of.

`recents.toml` sits above `projects/`, beside `settings.toml`: the ids, most recently opened
first. **Which project to reopen is the first entry and not a field of its own** — a `last` beside
the list would be a second answer the order already gives. It is an *order* and not an index of
what exists (the directories are that), which is why `MAX_RECENTS` (50) is safe and why nothing
prunes an id whose directory has gone: repairing it on load would write a file on a startup where
the reader did nothing. `Recents::touch` answers whether anything moved, so reopening the project
already at the front writes nothing. The recent-projects view reads each row's
name out of that project's own `project.toml`, never out of this file: a name copied in here would
be a second copy to keep in step with the one the user edits.

Inside those files, identity is **path + object name + symbol name + address** for a place in a
binary and **the path itself** for a source file, never pointers; that mapping lives in exactly two
places, `SavedDocument::from_document` and `::resolve`. A source file's path is a `String`, since
it is what the debug info said rather than something this filesystem was asked about.

**One `tabs` list of both kinds, not a `tabs` and a `sources` beside it**, because there is one
strip: the reader's own interleaved order is what comes back, and the one document that was on
screen is `active` whichever kind it is — written out in full rather than as an index, since a tab
that no longer resolves is *dropped* (which would shift the index) while the active one *degrades*.
Each entry carries **the rows both of its sides were left at**: a `tabs` entry is a `SavedTab`
(`asm_row` + `src_row` + `document`), rather than the list having arrays of rows beside it. The
rows travel with their tab because `resolve_tabs` drops the tabs that no longer resolve, which
would shift every later row of a parallel array onto the wrong tab. They are rows and not pixel
offsets so that the row height following the fonts (Step 9c) does not move every saved position,
and they are hints and not facts — `#[serde(default)]`, and clamped to what the tab holds *now* by
`Positions::row`. **Field order within these structs is load-bearing**: TOML emits plain values
before tables, so every field of `Project` being a plain value is what lets `binaries` sit beside
the name, `SavedTab`'s two rows must precede its `document`, and `SavedHistory::cursor` its
`entries`. Getting it wrong fails at *runtime*, not at compile time, and a round trip through real
TOML per struct is what holds it.

`Session::digests` is the digest each binary had when the session was saved, keyed by path — in
the *other* file from `binaries` and not a field beside them, because `binaries` is the list to *open* and a digest is
what to *believe* afterwards. A mismatch is not an error, a dialog or a refusal: `Rebuilt` collects
the paths whose digest no longer matches, and under one of those the **name is the identity and the
address is only a tie-breaker** (a symbol that merely moved resolves, where an unchanged file drops
it; a name that names two symbols and no longer names an address resolves to neither, since a stale
address is exactly what lands a reader on the wrong function), and the saved **row is dropped**,
being a claim about a listing this build no longer has. A path with *no* saved digest is a third
state, not a mismatch: it behaves as everything did before digests existed.

Coming back, the **active document degrades** (symbol -> its object -> nothing, since there is one
of it and the app must open somewhere) while **history entries are dropped** (a list of places the
reader cannot get back to is worse than a short list). A source-driven entry resolves against
nothing, so it neither degrades nor drops: a deleted file comes back as a tab over the pane's own
"Source file not found". `History::rebuilt` is the one walk both a restore
and a file-close go through, carrying the cursor to the last survivor at or before it.
`History::restored` also collapses duplicates and trims to the newest `MAX_ENTRIES` (200).

**When** a save happens is `Saves` in `project.rs`, a `static Mutex` rather than UI state because
two of the three things driving it sit outside the component tree. `record(details, binaries,
session)` is called on every state change and compares each against its baseline: a change to the
`binaries` writes **both files immediately**; a change to the user-given `details` — the name and
the directory — writes **`project.toml` alone**, since a rename lets go of no binary and so cannot
leave the two files disagreeing; a change to only the session marks it **pending** — a tab because
it is expressed against the binaries rather than the other way round, costs one click to remake,
and arrives on every navigation, `activate` opening one on the way to each change of document.
Nothing in `record` has to *say* which is which: which file a field lives in is what decides it,
and `Written` is how it says which half it decided. `flush()` writes the pending
session — on a 30s timer and from the window's close hook, which is the one exit hook freya 0.4
offers (`WindowConfig::with_on_close`, a `Send` callback that cannot read any `State`, which is
exactly why the policy is a static).

**Every baseline is the state the app boots into**, which is why two of them start empty and one
does not. The binaries and the session are restored *asynchronously* — the app boots holding
nothing and fills in when the parse lands — so seeding them from the loaded project would make the
first comparison see the still-empty boot state as a change and write an empty project over a good
one. `Saves::given` *is* seeded by `reopen`, because the name and directory are restored
*synchronously*, into the state the project view renders, before a single effect has run. Until 8e
that field was a value `Saves` **carried** across the calls rather than a baseline, for want of
anything on screen holding a name; the project view holds one now (`Proj`), so a rename arrives
through `record` like everything else and the special case is gone. `Saves::listed` is the one
piece of bookkeeping that grew out of it: it is what `project.toml` currently *says* the binaries
are, and a write that is not about the binaries writes that back rather than the app's own list —
otherwise a rename during the startup parse, or after a restore that opened none of them, would
forget a file through a change that had nothing to do with it.

**Which project is open is `Saves`' too**, and changing it at runtime is `switch(id)` or
`start_new()`: both `flush` the project being left while the policy still points at it, `remember`
the one being entered at the front of `recents.toml`, and re-point every baseline through
`Saves::opened` — empty, because the app is about to be emptied. Emptying it is the caller's half
and stays in `ui/project_view.rs`, the states being the UI's. `recent_projects()` is the list a view draws:
`recents.toml`'s order, each row described by reading *that project's own* `project.toml`, with an
id whose directory has gone dropped here — the list never prunes itself on load, and this is the
point of use where the repair is free.

Startup reopens the **last project** — `project::reopen`, the front of `recents.toml`, both halves
of it — and `use_restore_on_startup` knows nothing about where they came from, which is what keeps
a project picker out of it. The binaries stream in the way any other open does, so the sidebar
fills in behind them, but the **session waits for the whole load**: a tab, a selection or a history
entry is resolved against the objects by name, and resolving one against a half-filled list would
drop the tabs whose object had not landed yet. The strip is then restored, and **through the two
functions that hold the invariants** rather than by writing the list: `use_restore_on_startup`
sets the history, then `activate`s each tab and then the active one. Two orderings are
load-bearing. The **rows go into the two `Positions` maps before the tabs are opened** — those
maps are the one thing the restore writes directly, and a pane puts its view back when it notices
the tab it is showing has changed, so a row arriving after the `activate` arrives after the only
moment anything looks at it. And tabs before the active document, because `activate` opens what it
cannot find and would otherwise append it at the end of the strip instead of finding it in place
(the other direction is safe: a document that degraded to its object while the strip holds the
symbol simply opens a tab). An assembly-driven tab that no longer resolves is **dropped**, like a
history entry; a source-driven one is never resolved at all, so a file that has been deleted comes
back as a tab over the pane's own "Source file not found" rather than silently vanishing.

**The settings are a file of their own, above the projects** (`src/settings.rs`, `settings.toml`
at the top of the state directory beside `recents.toml`, since a setting is the user's and not any
one project's; same atomic `.tmp` + rename, same "a missing, unreadable or corrupt file is simply
the default"). This was the first slice of the storage split `notes/Goals.md` asks for under
*Projects*, and the same cut runs through each project: what the app **noticed** changes on every
click, what the user **said** changes when they say so, so they have different rates, different
save policies and different consequences when one of them will not parse. `Settings` is the theme choice (`Theme`: light, dark or follow the desktop) and a
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
is public and writes at once. **Resolving `Theme::Desktop` is deliberately not this module's job**:
"which theme does the desktop prefer" is a question for whatever owns the window, so `settings.rs`
holds only the choice and stays framework-free, and `ui/palette.rs` puts the two together
(`resolve_appearance`). It once spawned a subprocess per platform to answer it and no longer does —
the windowing system already knows, it answers on every platform this runs on, and its answer is
live rather than a value baked in at startup. `fonts::resolve` merges the settings over the
desktop's answer **field by field**, pure and tested, and `fonts::inherited` is that same merge of
*nothing*: what an unspecified field is falling through to, which is what the settings page draws in
an empty box. Everything in `fonts.rs` is in **points** up to one conversion at `Font::size`, the
app's own defaults included (9pt and 10.5pt, which are the 12 and 14 logical pixels the floem
version drew at), because an override and the value it overrides have to be the same kind of number
for the page to put them beside each other. The desktop's answer is cached per process
(`desktop_answer`), since the page re-resolves on every change and a lookup is a subprocess.

**A scratchpad is a generated cargo package, and the package is the storage**
(`src/scratchpad.rs`). One directory per scratchpad under the same base the other two files use
plus `scratchpads/`, holding exactly what cargo needs: a `Cargo.toml` naming the crate, its
pinned `edition` and its `[dependencies]`, and `src/main.rs`. Nothing describes a scratchpad
*beside* that — every field of the model is already a field of the package — so `load_from` is
the exact inverse of `write_to` rather than a second format that could disagree with what cargo
is handed, and both files go down through the same `.tmp` + rename, which the source earns:
`src/main.rs` is the reader's document. The manifest carries an empty `[workspace]`, so a
scratchpad is its own workspace root wherever the state directory turns out to be.

A dependency is a `(name, version)` row and the **version is required** — a `*` is refused with
its own reason, since a requirement whose answer changes with the day is the one thing a
scratchpad must not have. Rows are checked against two grammars (a possible crate name, a
possible version requirement) and never against crates.io: whether a crate exists is cargo's
answer. Every bad row comes back as `(index, Problem)` so the editor can mark all of them at
once, a repeat of one crate included — `[dependencies]` is a table, so the second row would
otherwise silently win — and a scratchpad with a bad row **refuses to write** rather than
generating a manifest that differs from what is on screen. **Building is blocking and belongs on
a worker thread**, exactly as `open_files` is: `build_in` writes the package, runs `cargo build
--message-format=json --color=never` with a null stdin, and hands back a value. The artifact path
is what cargo *named*, never `target/debug/<crate>` derived from the name and the profile, which
a `CARGO_TARGET_DIR`, a config above the directory or an executable suffix each make silently
wrong. Turning that stream into a `Build` is a pure function of cargo's stdout, stderr and exit
status, which is what lets a failed build be a test over a canned stream. Three answers, not two:
the compiler said no (with cargo's own stderr kept, since `no matching package named ... found`
is said there and nowhere else), or nothing was compiled at all.

**Running is the artifact and not `cargo run`.** `run_in` spawns the executable `build_in` already
asked cargo to name, in the scratchpad's own directory with a null stdin. Re-entering cargo would
redo resolution to arrive back at that same path, could arrive at a *different* one (the reader has
usually typed since, so what ran would not be what the diagnostics describe), would interleave
cargo's progress into the stream the reader is reading as their program's output, and would make
stopping meaningless — killing a `cargo run` kills cargo and leaves its child with nothing holding
it. What the app is handed back is a `Running`, whose one job is `stop`: `Child::kill`, since
`Child`'s own `Drop` neither waits nor kills, so a run abandoned rather than stopped goes on running
with nothing that could ever find it again. A **grandchild is out of reach**, which would need the
run in a process group of its own and a `libc` this crate does not carry. `stop_all` is the same
thing for every run at once, off a `static`, because the window's close hook can read no state —
`Saves`' reason exactly, and it sits beside `flush` in `main.rs`.

**Output is streamed, not collected**, which is the whole difference from `build_in`'s
run-it-and-return-the-output shape: a program that prints and then loops for ever has said
something, and a value returned at exit would never say it. Two threads, one per pipe, hand each
line to a callback as it arrives; whichever finishes last reaps the process and emits the one
`Ended`. So a run is over when both pipes are at the end **and** the process is reaped — a program
that hands its output to a grandchild outliving it reads as still running, which is the honest
answer, since the output is still coming. The reap `try_wait`s on a poll rather than `wait`ing,
because holding the `Child` is exactly what would make a stop wait for the process it is killing.
**Three bounds, and each is a different failure**: `MAX_LINE` (4 KiB) cuts a line with no newline in
it, so a program writing megabytes in one line is still *delivered* rather than accumulated;
`MAX_OUTPUT_LINES` (5000) is what is kept, oldest first out, with `RunOutput::dropped` so the view
can say the story is missing its beginning (a line cap and not a byte cap, because the view is a
list of rows and a byte budget would make the row count depend on how long the lines happened to
be); and the app's own `RUN_EVENTS`-bounded channel is backpressure that reaches the program itself
— a full channel blocks the pipe thread, which fills the pipe, which blocks the writer.

## UI (freya 0.4)

freya 0.4 is **not** Dioxus-based: no `rsx!`, no `#[component]`, no `use_signal`. It is a builder
API (`rect().width(Size::fill()).child(..)`) over its own `freya-core`. Most freya material online
describes the older API and does not apply.

**State** is a handful of `State`s provided at the root with `use_provide_context` and read with
`use_consume`: `Objects`, `Active` (the active `Document`), `Open` (the open tabs),
`AsmAt`/`SrcAt` (where each *side* of each of those tabs was left), `Hist`, `Proj` (which project
all of that belongs to), `Loading` (the files on their way into `Objects`), `Focused`, `Pinned`,
`Marked`/`Shift`, `Analysis` (what the worker has to say about
the selected symbol), `Pad`/`PadText` (the scratchpad, and the buffer being typed into it),
`SplitRatio`/`Splits` (how wide a document's assembly side is), plus the memos `Symbols` and
`Active`. The seven that a project *owns* travel together as a `ProjectStates`, since a project
switch closes all of them and reopens all of them.

**One strip, two kinds of tab.** A `Document` (`project.rs`) is **a place in a binary or a file**:
`Document::Assembly(Selection)` — an object or a function — or `Document::Source(Arc<str>)`, the
string the debug info said and never a path this filesystem was asked about. A tab has two sides,
assembly and source, and the variant says which side the tab is *about* and therefore which drives
the other. So opening a file from a directory panel and opening a function from the symbol list
produce the same kind of thing, differing only in which way the mapping runs. Each tab wears the
one glyph that tells the two apart — the same pair the Assembly and Source views wore before those
two became a document's left and right halves. Until Step 1 this was two strips with two notions of what was open — `Open`/`Sel` for
functions and `Files`/`Shown` for files — and the merge is what made the history able to record a
visited file and the session able to keep the strip's interleaved order.

**`Active` is a derivation, not a state.** What is open is `Open { dock, docs }`: the content
area's dock, whose **document panel**'s `tabs` vec *is* the list of open documents in the reader's
own order, and `Docs`, the table saying which `Document` each tab's `DocId` stands for. There is no
second list and no cursor — the active document is that panel's active tab read through the table,
which is the whole of `active_document`, and `open_documents` is the same walk for the list. `Docs`
holds no order at all; membership is the one thing the two share, and it is an invariant the three
functions keep and a test asserts: a document's tab and its table entry are made together and
closed together.

`Active` is a `Memo` over the two, because the dock notifies on every layout change and a reader
dragging a split must not re-render every pane that draws a document — `Memo` writes with
`set_if_modified`, so a drag that changed no document wakes nothing. It is therefore **a beat
behind**, a memo being recomputed by a task woken on a notify. That is right for anything that
*renders* and wrong for anything that must be true inside one event handler, so `activate`,
`close_tab`, `close_binary` and the save observer call `active_document` on the states directly and
never read the memo. `use_kept_position` asks `Docs` for the same reason: it decides whether to
write a row down for a tab that may have just been closed, and a memo could still be reporting it
open during exactly that run.

`Active` being `None` means two things and deliberately does not distinguish them: nothing is open,
or **the tab on top of the document panel is a view**. Making Settings the active tab therefore
means there is no active document — the analysis clears, `session.toml` writes `active = None`, and
a restart with a view on top restores every tab and shows none of them. That is the price of the
derivation, and it was taken over the alternative, which is remembering the last document that was
active there: memory rather than a reading of the dock, and the second source of truth back again.

The invariant — the active document is one of the open tabs, or `None` — is held by three functions
and nothing else: `activate`, `close_tab`, `close_binary`. **Every** site that would *open* a
document calls `activate`, `navigate` included, because the history keeps an entry long after its
tab was closed; pressing a tab needs none of them, freya's own header wrapper setting the panel's
active tab, which *is* the change. `Selection` itself has **no "nothing" variant**: having none open
is an absent one, which is the only spelling that stays honest once a selection is something a tab
can hold.

**Layout** is a toolbar over a `ResizableContainer`: a `PanelSize::px(300.)` sidebar and a
`PanelSize::percent(100.)` content pane, mixing the two sizing modes deliberately so the sidebar
keeps a fixed width and the content takes the rest, with freya's 4px `ResizableHandle` between
them. `ResizableContainer` renders itself `.expanded()`, so it needs a parent already sized —
`Size::flex(..)` only works under a parent with `.content(Content::Flex)`. The content panel holds
the `DockingArea` and nothing else: the open documents are tabs *in* it, so the bar over them is
the document panel's own tab bar rather than a strip of the app's.

Inside each panel is a `DockingArea` over a `DockArea` model. A `Tab` is two-kinded —
`Tab::View(View)` for one of the seven views, `Tab::Document(DocId)` for an open document — because
`DockingModel::TabId` is `Copy + PartialEq + Hash` and a `Document` is none of the three. Both areas
use `Tab` as the payload and `use_drag` keeps one `DockDrag<Tab>` at the root. The outer split stays
a `ResizableContainer` because docking cannot express a literal 300px. A drag carries only the tab,
so the area receiving a drop evicts it from the other through a wired-up
`other: Option<State<DockArea>>`. A view is a **persistent pane**, not a slot the selection drives:
each is a unit `Component` that consumes context and renders off the state it is about, so a
selection change re-renders only the panes that read it and never the root.

**One panel is designated, and the reason is the opening rather than the placeholder.** A click in
the symbol list opens a document, and that document has to land *somewhere*; a dock has many panels
and freya has no notion of "the panel documents belong to", so `DockArea::documents` names one.
Three rules follow. `on_drop` refuses a document into any other panel — one visible document is what
lets `Analysis`, `Marked`, `Focused` and `Pinned` each hold one answer for the window — and refuses
a `DocId` the table no longer knows, which is a drag that outlived its document and is the whole
payoff of **ids never being reused**. A **view**, by contrast, may go anywhere, that panel included:
Project, Settings and the Scratchpad start tabbed in it, to the left of the documents, where they
are always visible. And `tidy` exempts it from the folding sweep, so closing the last document
cannot fold the content area away.

`tidy` is freya's `close_empty_panels` **written out rather than called**, because that sweep retains
every non-empty child with no exemption and has to be replaced rather than followed — a panel
re-created after it would come back somewhere else in the tree. The two behaviours of freya's that
are kept: a split left with one child collapses into it, and a lone panel at the root is never
removed. Likewise a close never goes through `DockNode::remove_tab_except`, which sets a panel's
active tab to `tabs.first()`; landing on the **neighbour** is a rule of this app, so `close_tab`
removes the tab by hand and chooses with `tabs::landing`.

**Open documents *are* dock tabs.** This supersedes 6c's "the tab strip is not the dock's tabs",
which argued three things. Two are answered by the designated panel: there is one answer to "which
document is active" — that panel's active tab — and closing the last document folds nothing away,
the panel being exempt from `tidy`. The third stands and is the price: the layout and the list of
open documents are no longer separable, and the arrangement survives a close because a rule says so
rather than because the shape makes it impossible to break. What it buys is that a reader arranges
their documents the way they already arrange the views, and that Steps 9, 12, 19 and 33 each have
one kind of tab to change instead of two.

A document's header is `chip` — the same element the content area's own strip drew, hover state and
× included. **Nothing in it activates the tab**: freya wraps a header in a `DropZone` around a
`rect().on_press(set_active)` around a `DragZone`, so pressing it makes it the panel's active tab
and therefore the active document. That is also why the × must `stop_propagation`, or a close would
first switch to the tab it is closing. The × is drawn for documents only — the views are furniture,
one of a kind, with no way back once closed, where a document is always reachable again from the
symbol list or the history.

The document panel's tab bar is the horizontally scrolling one the strip used to be (`chip_strip`),
because documents are opened by the dozen; a view panel's stays a plain row, seven views always
fitting. Two things bite there. freya appends one child more than there are tabs — a
`rect().expanded()` drop zone for "past the last tab" — and `expanded()` is meaningless inside a
horizontal scroll view, so it is given a width of its own. And a tab's name is elided **by character
count in Rust**, where every other truncation is a width: a `maximum_width` anywhere inside one makes
it shrinkable, and a horizontal scroll view measures children against the space *left*, so tabs past
the edge get no width and draw as a bare ×. Do not "fix" that back into a width.

**A document's two sides live inside its tab.** `Tab::Document` renders `AssemblyPane` beside
`SourcePane` in a `ResizableContainer` — not a nested `DockingArea`, which is a great deal of
machinery for a two-way split. The cost is real and was taken deliberately: **the Source pane is no
longer independently dockable**, since it is inside a document rather than beside one. Each pane
takes its `Document` as a prop rather than reading `Active`, which is both synchronous and honest —
only the active tab's content is mounted, so a pane is only ever built for the tab it belongs to.

That unmounting is why the split ratio is held at the root (`SplitRatio`, with `Splits` the shared
`ResizableContext` it is read back out of). A `ResizablePanel` registers at its `initial_size` in a
`use_hook` and *removes* its entry in a `use_drop`, so even a shared context comes back holding the
initial sizes under new panel ids; what survives is a number the app keeps, fed in as `initial_size`
and written back out while the split is on screen. One number for the app and not one per document:
per-document would be a third `Positions`-shaped map to forget in `close_tab`, for a number nobody
asked to differ per document.

**A document is a place in a binary or a file; everything else is a view.** This is 8e's rule with
Step 1's amendment — it used to end at "in a binary" — and everything below it is unchanged, so
decide nothing about it again. The document panel holds `Document`s and never anything else, and
that is what lets five separate things work without a case each: the Assembly *and* Source panes both
render "the active tab", the history records it, `SavedDocument::from_document`/`::resolve` write
it down and find it again after a restart, `close_binary` knows which tabs a closing file takes
with it, and `entry_text` knows what to call it. A project view, the settings page and a
scratchpad's editor are none of that: they resolve against no object, they are no file on disk the
panes could open, there is one of each rather than many, and neither pane could draw one. So they
are **dockable views** — a `Tab` — which is the mechanism the app already has for "a pane with its
own state that the reader can put where they like", and which `InfoTab` was already an instance of.
A third `Document` variant was the alternative and buys a tab in a strip nothing else would put a
second entry in, at the price of five answers nobody wants: what `resolve` does with it after a
restart, what `Document::in_file` says when a binary closes, what the panes draw for it, what the
history means by a "place" that is not one, and what the session file spells it as. Persistence
follows from the same sentence: a `Tab` is layout, and the dock layout is deliberately not
persisted, so a view is **explicitly excluded** from the saved tabs and `SavedDocument` needs no
answer for it. What a scratchpad *builds* needs no rule at all — the artifact goes through
`open_files` like any other binary, and its functions are ordinary tabs.

**Each tab remembers where each of its sides was left.** A pane has one `ScrollController` and
shows one tab at a time, so left alone it hands the tab arriving whatever offset the one leaving
had. `AsmAt`/`SrcAt` are two root `Positions` maps beside `Open`, **both keyed by the `Document`**
— so an entry means "this side of this tab" for exactly as long as the tab is open — and
`use_kept_position` is the whole of the behaviour, called once by `InstructionList` and once by
`SourceList`. Keying the source side by the *file*, which is what the Source pane's own strip did,
made two functions compiled from one file share a position they have no reason to share. What is
kept is a **row**, clamped to what the tab holds *now*, so a rebuilt binary or a shortened file
cannot come back past the end. Three things are
load-bearing. Reading the controller's position (`<(i32, i32)>::from`) is a `State::read`, which is
what **subscribes the effect to the pane's own scroll**: every position is written down as it
happens rather than on the way out, which is what survives the window merely being closed. The tab
the controller is *holding* is tracked in the hook — an `Rc<RefCell>`, not a `State`, since nothing
renders from it — because it is not the tab the app is showing during the one run that has to move
the view, and every write goes under the held one. And a `Pin::reveal` **wins** over a remembered
position with nothing written to make it: the two are never owed at once, since this moves the view
only when the tab changes while a click asking for a reveal changes no tab (and a selection change,
which does, drops the pin), and when a reveal scrolls, the effect wakes on that scroll and records
where it landed. `close_tab`/`close_binary` forget both of a tab's positions with the tab, which is
not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points into — and the hook is
handed the tab list precisely so that the run *after* a close, still holding the tab that has gone,
cannot put it straight back.

**Opening a binary is the one path in, and it streams.** `open_binaries` is `close_binary`'s
opposite number and the only thing that ever adds to `Objects` — the toolbar's Open, a session
restore and a scratchpad's rebuild all go through it, so they cannot differ about what opening a
file means. A `std::thread` and an `async_channel`, `use_analysis`' shape, but the answers come back
one at a time: `Loads::begin` registers the paths **before a byte is read**, so the sidebar has a
row for the whole of the wait rather than from whenever the first answer lands, and `take_load`
writes each batch of objects in as it arrives. The channel is **unbounded and drained in batches** —
unbounded because backpressure is exactly wrong here (the worker is the thing that should run flat
out) and batched because a write per member is a re-render per member, which for an archive whose
members parse in a millisecond is a hundred renders nobody sees. **An object nobody asked for any
more is dropped, not prevented**, which is `use_analysis`' rule in a second place and has to be: the
worker is already parsing when the file is closed. It is checked against `Loads::holds` — the load
*and* the path, since a file closed and reopened while the first parse ran is two loads and only the
second one's objects belong on screen, which is the whole reason a load has an id where an analysis
answer needs none (an answer is about a `Symbol` that already existed; a load is about work that has
produced nothing to be identified by). `take_load` **returning** is what stops the worker: it drops
the receiver, the next send fails, and the walk breaks. What streaming buys is not uniform:
`libanalysis-sample.rlib`'s first member is offered at 102 ms against the 685 ms the whole file
takes (debug build), while `viewer-sample` is one object and gains no object earlier at all — there
the win is the row, on screen from the click instead of an empty list for six seconds. **Nothing
further is opt-in**, and measuring says why: of a file's parse, the whole of what is left after
line info, the DWARF context and the disassembly were already made lazy is reading the bytes and
walking the symbol table, which is what the Objects and Symbols lists *are*. On `viewer-sample`
(release) that is 1.38 s of which 766 ms is the read and 286 ms the demangling; deferring the
demangling is the only lever there is, and it defers work until the first click on the object.

**Nothing is analysed on the UI thread.** `SymbolData::assembly` decodes and formats the whole
symbol, and `SymbolData::line_info` builds the object's entire DWARF context on the first query
against it — 1.4 s together for the first symbol clicked in `viewer-sample` (debug build; 0.6 s in
release), and both of them used to run in `render`. `use_analysis` moves them together, because
they are asked for by the same click and the pane needs both: **one worker thread** for the app's
lifetime, fed an `async_channel` of `Symbol`s, answering with a `Studied` (the `Assembly`, its
`Lanes`, and the `SymbolLines`). One worker and not a thread per request or a pool, because
requests *supersede*: a reader going down the symbol list issues one per row and wants the last
one's answer, so the queue is drained to its newest entry each time round and the rest are dropped
*before* being started. A thread each would put a whole run of clicks through the most expensive
call in the crate at once for one useful answer, and `DwarfCache` is a `OnceLock`, so the losers
would block on the winner rather than race usefully. (The parallelism `notes/Goals.md` asks for is
about parsing many objects at once and is a different job.)

**A superseded answer is recognised, not prevented.** Every answer carries the `Symbol` it is
about and is kept only if that symbol is the one selected *now* — a comparison and not a generation
counter, since `Selection` already compares by `Arc` pointer identity, and since the answer for the
first A of an A → B → A is a perfectly good answer for the third selection. A dropped answer is
what clicking twice quickly *means*, so nothing logs or retries. **What the panes show meanwhile**
is the listing they already have: `Analyzed` holds `shown` (the symbol actually drawn, which is the
one selected *before* this one for as long as the worker takes), `pending` and `slow`. A listing is
replaced by the next listing and never by a blank, or every click would flash the pane empty for a
frame; only after `SLOW_ANALYSIS` (180 ms, started by the request and never polled) does the
message displace it, which is the order of the arms in `Analyzed::showing` — the one place either
pane decides what it is drawing, so the two cannot disagree. Two things follow
from `shown` being the drawn symbol rather than the selected one: `InstructionList` is mounted only
for a listing that exists, so `use_kept_position` cannot write a pending tab down at row 0 before a
row of it has been seen; and the Source pane's companion file comes out of `Analysis` rather than
out of `Active`, so it cannot name a file the previous symbol was compiled from, which is what
`Studied` carrying its `Symbol` and `SymbolLines` carrying its file are for.

**Nothing is cached in the UI, deliberately.** `SymbolData::assembly` does not memoize — it decodes
afresh and hands back a new `Arc<Assembly>` — and `Object::line_info` caches the DWARF context and
the subprogram extents but re-walks the covering units' line programs per call. What the `Analysis`
state gives is the one thing a re-render needed: the answer is *held*, so a hover, a theme change or
a resize costs nothing where the old shape re-decoded in `render`. A second, keyed cache would be an
unbounded pile of `Assembly`s for listings the reader has left, to save a few milliseconds on a
symbol they have already been shown.

**The Source pane draws the active tab's source side**, and `source_side` is the one place either
pane decides which file that is — so the pane and the effect that drops its picked-out rows cannot
disagree about which listing is up. A **subject** is a source-driven tab's own file; a
**companion** is the file the drawn symbol was compiled from, which comes out of `SymbolLines`
inside `Studied` and not out of `Active`, because the analysis arrives from a worker thread and
anything reading the two separately sees them disagree for as long as the work takes. Only the
symbol's *own* file is drawn, never the rest of `LineInfo::files`, since a Rust function inlines
dozens.

A companion wears a **header naming its file**, which a subject does not: the strip already names
a subject, and nothing else in the window would name a companion now that the Source pane has no
strip of its own. Pressing that header opens the file as a source-driven tab, and until the project
explorer and the source search land it is the only door into one. The **assembly** side of a
source-driven tab is blank: which symbols a source line compiled into is Step 2's index and picking
one of them is Step 1d, and until then the pane draws nothing rather than carrying over an answer
from a tab where "no symbol selected" is true.

The rows are the app's own (`SourceRow`, a `VirtualScrollView`), **not** freya's `CodeEditor`,
which paints a line background only for the cursor's row and keeps its scroll state private —
which is to say it cannot do the two things this pane exists to do, highlight the *set* of lines
an instruction maps to and be scrolled to one from the other pane. Neither objection survives a
pane the reader is typing in, so the Scratchpad's editor *is* that component (below). What
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
carries a `FocusOrigin`; and **the scroll is a request, taken once** — `take_reveal` *removes* it,
so a repeat click is a second request, and `reveal_row` does nothing when the row is already on
screen. None of this is a navigation: the selection does not change and nothing is pushed onto the
history. `navigate` remains the only path for anything that does.

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

**A file being read is a row before it has an object**, which is `notes/Goals.md`'s "an indicator
for an object still being processed" — and the state is on the **file**, not on an object, because
an object that has not been parsed does not exist: the unit part-way through is the one the reader
opened, the one `close_binary` closes, and the one that already has a row. `Loads` (in `tree.rs`,
the state behind the `Loading` context) is the files being read, one entry per (load, path); the
tree draws a `TreeRow::File` for each that has produced nothing yet and marks the ones that have
`loading`. Three rules come with it. A file still being read is **always** a file row even at one
object, since "one object is its own row" needs to know the one is all there will be and a row that
promoted itself to a parent as the second member landed would move the list under a reader already
reading it. A row with nothing behind it has **no group** (`group: Option<usize>`, the group being
the first object's pointer), which is exactly the row that can never be folded, so it draws no
triangle. And it wears `…` rather than a format tag, since which format a file is is not known
until it has been parsed; the name is dimmed to `address_fg` beside it, two static cues rather than
a spinner, because a sidebar row is one of hundreds and none of the others move.

**A file row is also how a binary is closed** — right-click opens a `ContextMenu` (which needs the
`ContextMenuViewer` mounted at the root of `app()`; opening one without it panics) on a single
"Close file". A member row offers nothing: the unit that closes is the file. `close_binary` is
composed of three rules from the modules that own them — `Selection::in_file`, `Tabs::close_all`,
`History::retaining` — plus a fourth for a file that is still arriving (`Loads::cancel`, without
which the objects still coming out of the worker would put the file back one member at a time), and
three decisions inside it matter: the selection **follows the tabs**
rather than degrading (a file takes its objects and their symbols together, so there is nothing to
fall back to); the history **drops** through the same `History::rebuilt` walk a restore uses, so
the two cannot drift; and the unit is the **path**, so one file opened twice closes once.

**The Project view** (`Tab::Project`) is what a project's `name` and `directory` are finally set
from — two fields that round-tripped since 8d with nothing to write them. It is **one view and not
two**, where `notes/Goals.md` asks for a project view and a recent-projects view separately: they
are one question — which project am I in, and what else is there — the recent list is how a reader
*leaves* the project the rest of the pane describes, and a tab of its own would be empty in every
session where a project was reopened, which is all of them after the first. The goal's "if none was
open" case is the pane answering for itself. The list leaves the open project *out*, the pane above
it being a better and fresher description of that one than a row read off a file could be.

`OpenProject` is the value `Proj` holds, and its two editable fields are `String`s where
`Details` has `Option`s: this is what is in two text boxes, and a text box has no third state —
an empty box *is* how a reader says "I have not said". `OpenProject::details` is the one place the
two spellings meet, and it trims, so a box of spaces is a box of nothing rather than a project
named `" "`. Each box writes straight into `Proj`, so a keystroke is a state change the save
observer sees like any other and `record` writes `project.toml` at once — `Goals.md`'s "user
project changes save immediately" taken literally, at a few hundred atomically-written bytes per
keystroke of something typed once a project. The binaries it lists come from `Objects` through
`project::binaries`, which is what the saved list is *derived from*, so what the pane draws is what
the next write will say.

**A project switch is a close and a restore, through the same functions.** `switch_project`
is `project::switch` (flush, re-point, remember), then `clear_project`, then `restore_project` —
and `clear_project` is a `close_binary` per path and then a `close_tab` for whatever is left,
never a write to the list, so a project is left in a state the reader could have reached by hand.
Its one extra line is `Loads::clear`, which cannot go through the per-path walk: a file that has
been asked for and has produced nothing yet is not in the objects list for that walk to reach, and
its objects would otherwise arrive into the project that comes next. `restore_project`
is the body the startup restore was, extracted so the two cannot drift. The source-driven tabs go
in that second walk, where a closing *binary* deliberately leaves them standing: a file tab
outlives the binary that led the reader to it because the text stands on its own, but it does not
outlive the project whose session recorded that it was open. The ordering is what makes it safe — `project::switch` empties the
baselines *before* the app is emptied, and freya wakes an effect by a notify rather than at the
write, so the save observer runs once after the whole handler and sees a settled state that matches
the baseline exactly. `new_project` is the same thing with nothing to restore.

**Tooltips** are how a truncated row is read, so `row_tooltip` sets the delay to `Duration::ZERO` —
freya's 500ms default makes sweeping down a list useless. The filter toggles keep the default
(their tooltip explains what `\b` means), and the code rows have none.

**One palette, one place.** Every colour is a field of `Palette` in `ui/palette.rs`, there are two
instances (`Palette::LIGHT` and `Palette::DARK`), and `palette()` is how anything reaches
whichever is current — no call site names a colour, and none of them changed when the second
palette arrived, which is what the indirection was for. The dark values are the light ones
**carried over**, not designed again: every relationship holds on both sides (the header a step
off the pane, the pin the focus at more alpha, each code colour keeping its hue and its place in
the ordering), and the only ones that could not be flipped literally are the translucent washes,
which `blend` composites over the pane — the same alpha over a dark ground is a fraction of the
step it was over white, so each was judged as what it *comes out as*. Two tests hold that: a
contrast floor for every foreground on the surface it is really drawn on (3.0, not WCAG's 4.5 —
the light palette's address column and its comments are meant to recede and sit between 3 and
3.5), and a
visible-step floor for every wash over the row under it, with the pin required to stay louder than
the focus. The code colours are named for what they mean, not for the pane they came from, and
`Palette::syntax` maps `freya-code-editor`'s ~33 capture fields onto them. Beware
`resolve_capture_color`: it treats a capture whose colour equals `text` as unmapped and walks *up*
the dotted name, so giving a child field the text colour while its parent holds another silently
paints the child in the parent's colour — a property of which fields *share* a value, so a second
palette can break it by landing two colours on each other, and `captures_do_not_walk_up` asserts it
for both. This is deliberately **not** freya's own theming — `ColorsSheet` names none of these
roles, and the source pane's colours cannot be read from the element tree at all, being baked into
a `SyntaxBlocks` when a file is *loaded*.

**A theme switch repaints by being asked for a colour.** `palette()` reads a thread-local
`State<Appearance>` and hands back a `&'static` to one of the two `const`s, so `State::read`
subscribes whichever scope is rendering: *asking for a colour is what subscribes a component to the
theme*, exactly once, wherever it sits and whatever built it. The two alternatives were weighed and
lost. Threading a context read through the call sites is freya's own idiom but impossible here — a
hook must run unconditionally in a component body, and `palette()` is called from free functions,
from `if` arms, from render callbacks and from `Highlighted::new`, which is not a component; it
would be a line in each of the twenty-one components with the free functions still on a static, and
a forgotten line would be a patch of the old theme. Re-rendering from the root does not work at
all: freya marks a child dirty only when its props change (`freya-core`'s `runner.rs`) and every
view here is a unit `Component`, so forcing it means a `key` that remounts the tree and throws away
the three filters, the objects tree's folds and every scroll controller. The cost of what was
chosen is that `palette()` is a thread-local lookup and a subscribe rather than a constant — tens
of nanoseconds against perhaps a thousand calls per full render. **`set_appearance` is the only way
to change it**, because the switch also has to `HIGHLIGHTED.clear()`: that cache holds
`SyntaxBlocks` with colours already resolved into them, so its entries are not stale but the wrong
theme, and nothing a re-render does would repaint them. The clear is inside the setter
(`set_if_modified_and_then`) rather than at a call site, so it cannot be routed around. The
appearance is resolved by `use_theme` at the root of `app()` from two inputs — the stored choice
(`settings.rs`, read once: it is a file) and `Platform::preferred_theme`, which freya keeps from
winit's `Window::theme()` and re-sets on the OS's `ThemeChanged` event — through the pure
`resolve_appearance`, where only `Theme::Desktop` is a question at all. **Not a `use_hook`**: the
preference is a `State`, so *reading* it subscribes the root and a desktop that goes dark while the
app is running repaints, which the subprocess this replaced could never do. It resolves in the
render body rather than in an effect, because an effect lands a frame late and a frame late on a
dark desktop is a white window flashing; the write is idempotent, so the frame it costs is the one
after an actual change, and the two-hop path (the platform wakes the root, the root's write wakes
everything that drew a colour) is what the headless test spells out. The control for the choice is
the settings page's three buttons, which write the choice and nothing else — `set_appearance` stays
the one writer. The one thing `text_fg` adds is the
interface text: set once on the root rect and *inherited*, since freya resolves an unset `color`
from the parent's, and it is `BLACK` in the light palette because that was already the default.

**Fonts.** `fonts.rs` asks the desktop for its interface and fixed-width fonts.
**Which desktop to ask is a runtime question**, not a compile-time one — one Linux build
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
one font freya will not let an element set is the tooltip's, hardcoded in its theme, so
`interface_theme` provides a `Theme` with `tooltip.font_size` at the interface size — on top of
freya's own `light_theme()`/`dark_theme()` sheet, chosen by the appearance, which is the one place
freya's theming is used for colour: the filter boxes, scrollbars, resizable handle, tooltips and
context menu read their colours from it and from nothing else, and a white text box on a dark pane
is not a theme switch.

**A font change repaints the same way a theme change does, and moves the rows with it.** `fonts()`
in `ui/metrics.rs` reads a thread-local `State<Arc<Fonts>>` exactly as `palette()` reads the appearance, so
*asking for a font is what subscribes a scope to it*; `set_fonts` is the one writer, and unlike
`set_appearance` it has nothing to invalidate beside it, a cached `SyntaxBlocks` carrying colours
and no font. The readers are the two row heights, `icon_size`,
`FontExt::assembly_font`, the root rect's own `.font(&fonts().ui)` and the
tooltip's `font_size` in the root's `Theme` — that last one is the only place a change has to be
*carried* rather than picked up, freya's theme sheet being a value, so the root's effect has the
interface size in its deps beside the appearance. `ROW_HEIGHT` went the same way and became a
function: one font's size plus `ROW_LEADING` (12, which is exactly what the old constant's 26 was
over the 14px fixed-width default). That was 9c's real
decision, and the alternative — a page offering a 20pt assembly font and drawing it clipped inside
a 26px row — was worse than the work. It is safe because the scroll view's `item_size` and its
rows' own height are read in the **same render pass**, so they cannot see different numbers, and
because the per-tab positions 8b saves are *rows* rather than pixel offsets. The floor
(`MIN_ROW_HEIGHT`) is against a hand-edited `settings.toml`, where a size of 0.1 is positive enough
to pass `FontSetting::size` and would make `item_size` a fraction of a pixel.

**And it is two functions, because no row mixes the two fonts.** `list_row_height` follows
`fonts().ui` and `code_row_height` follows `fonts().mono`; both are `row_height_for`, so they can
differ only in which font they ask about. It was one number — the *larger* of the two sizes — and
the `max` read as a constraint while being nothing of the kind: every row in the code panes sets
`assembly_font()` on itself and on each of its spans, every sidebar row sets nothing and inherits
the interface font from the root, and no row anywhere draws in both. So the one number was two
lists sharing an answer, and raising either font padded the rows drawn in the other — an 18pt
assembly font made the objects tree, the symbol list, the tab bars and the chips 36px tall for a
12px font. Which height a site takes is decided by **the font its rows are actually drawn in**, and
getting one wrong is a misalignment that reads as a rendering glitch: the code height goes to the
instruction and source rows, the editor's line height, a run's output rows and the `item_size` of
those views; the list height to everything else, the filter bar's own height, `toggle_size` and
`icon_size`'s cap included, since a filter bar sits over a sidebar list and there is no filter over a code pane.
`row_at`/`row_offset` are the **code** panes' conversion alone — `use_kept_position` and
`reveal_row` are called by `InstructionList` and `SourceList` and by nothing else — so the old
"one conversion for every pane" argument for a single height went with the `max`. One thing did
move: at the app's own defaults (9pt interface, 10.5pt fixed-width) a sidebar row is 24px where it
was 26, because 26 was the *mono* font's number and had never been anything else. No floor holds it
at 26; that would be the same coupling under another name.

**The Settings page** (`Tab::Settings`) is where the theme choice and the two font overrides are
edited. `Prefs` holds an `EditedSettings` — `OpenProject`'s shape, and for its reason: a family is
a `String` here and an `Option<String>` in the file, an empty box **is** how a reader says "I have
not said", and `EditedSettings::settings` is the one place the two spellings meet. A *size* gets no
such treatment: it is a stepper and not a text box, so there is no half-typed state and no third
answer for text that is not a number — which also keeps a reader from spending a keystroke at 1pt
on the way to typing 12. `use_settings` is the whole of the wiring, and the write it makes is
compared against **what the file currently says** rather than against what was loaded, `Saves`'
rule: a fixed baseline would leave the file holding the middle answer when a reader changes a
setting and changes it back, and comparing at all is what stops a run that never opened the page
from creating `settings.toml`. `use_settings_with` takes the write as an argument, since the real
one edits the settings of whoever runs the tests.

**An override is drawn differently from the value it would replace**, which is the goal's own
words and the reason `settings.rs` keeps `None` as a real third state. Three cues, deliberately
more than one: the field's *name* is interface text when the reader set it and `address_fg` when
they did not; the *value* is real text in the box against a placeholder showing what is being
inherited (`fonts::inherited`, so what is shown is by construction what would be used, the
platform's own family and the app's own size included); and the **Clear** button is there only when
there is something to clear, which is also the only way back to unspecified — a family box can be
emptied, a stepper cannot.

**The Scratchpad page** (`Tab::Scratchpad`) is the source, the crates, the build and what the
compiler said, and it is a *view* for the reason the settings page is: there is one of it, it
resolves against no object, and neither code pane could draw one. What it **builds** needs no rule
at all — the executable goes through `open_files` and its functions are ordinary tabs.

**Its editor is freya's own `CodeEditor`**, which the read-only source pane deliberately rejected.
That is not a reversal: both of the pane's objections were about painting and scrolling a listing
from *outside*, and neither survives a pane the reader is typing in — the one line it backgrounds
is the caret's, which is the only current line an editor has, and nothing here wants to scroll it
from elsewhere. What comes with it is a cursor, a selection, an undo history, the clipboard, IME
preedit and an incremental tree-sitter re-parse per keystroke. Two things stay ours: the colours,
mapped onto the palette (`EditorTheme` beside the `EditorSyntaxTheme` `Palette::syntax` already
answers for), and the font — the component takes **one** family where everything else takes a
chain, and the rest of the chain arrives by inheritance from the box around it, since freya appends
a parent's families behind an element's own. Its line height is `code_row_height()` reached through the
multiplier it wants, with half a pixel of slack because it multiplies and floors. The editor's
`SyntaxBlocks` is `HIGHLIGHTED`'s hazard in a second place — colours resolved in at parse time, and
`set_appearance`'s clear cannot reach inside a `CodeEditorData` — so an effect keyed on the
appearance re-sets its theme and re-parses.

**One worker thread owns the scratchpad's directory.** Reading a scratchpad back, writing the
package and `cargo build` are all documented in `scratchpad.rs` as blocking, so all three go to one
`std::thread` fed an `async_channel`, `use_analysis`'s shape — one thread and not three, because
the point is not only that the UI thread stays free but that the directory has a single writer, so
a save cannot land inside the build that is reading what it writes. **Saves supersede and builds
never do**: a keystroke is a save, so the loop drains its queue while what it holds is one, and
whatever is behind it is either a newer save or a build that writes the package itself. A build is
what the reader asked for and its answer is the point. Two builds cannot start at once, on the
button (`enabled`) and in `request_build` both, because a build takes seconds and a second job
queued behind the first would compile bytes that have since changed.

**Nothing is written until the disk has been read.** `PadState::opened` is `Saves::written`'s rule
in a second place: the app boots holding `Scratchpad::default` and the reader's own source arrives a
thread later, so a save in between would put the default over a scratchpad someone was keeping. The
baseline is then seeded *by that answer*, so a run in which nothing is typed writes nothing and a
scratchpad nobody opened leaves no directory behind. `Scratchpad::write` refuses outright rather
than generating a manifest that differs from the rows, so a bad row stops the source being written
too — which the pane says over the rows, each of which says its own half. Every bad row is marked,
not the first: `Scratchpad::problems` answers with `(index, Problem)` for all of them, and
`Problem::half` says which of the row's two boxes to redden, because `Repeated` is a *name*
collision and nothing in its wording says so.

**A failed build points back at a row structurally, never by looking for a crate name in a
sentence.** A rejected build with no compiler diagnostics at all is cargo refusing before it
compiled anything, and `[dependencies]` is the only part of the generated package this pane can get
wrong — so cargo's own stderr, where `no matching package named ... found` is said and nowhere
else, is drawn under the rows. Once the compiler has spoken, the same stderr says only what the
diagnostics list already does and is dropped.

**Running does not sit on that worker, and stopping does not go near it.** `PadJob::Run` only
starts the program and comes straight back — it goes to the worker because it forks and because the
directory it hands the program is that thread's, not because it blocks. A run has no bound on how
long it takes (an accidental `loop {}` is the ordinary case in a buffer someone is experimenting
in), so a run queued like a build would freeze every save behind it and the reader could not edit
their way out. A stop is the same argument turned around: queued behind a build it would arrive
after the thing it was meant to interrupt, so it is a direct `Running::stop` from the handler.
`RunState` has four states because `Starting` is the one a `bool` loses — a fork is fast but not
instant, and a Stop pressed inside that window is remembered by leaving `Starting`, which is what
makes the arriving handle unwanted and stopped where it lands. `Over(Stopped)` is written by the
run's own `Ended` and never by the button, so the pane says "Stopped" when the process is gone
rather than when it was asked. **Events carry a run number**, which `use_analysis` was at pains not
to need: it can compare identities because an answer carries the `Symbol` it is about and that
symbol predates the request, whereas the process an event is about does not exist until after the
first bytes can be written. Stopping one program and starting another is one keypress, and untagged
the first one's last lines and its `Ended` would land in the second's output. **stdout and stderr
are told apart by colour and by nothing else**, and deliberately not by the red every invalid thing
wears: stderr is not an error, it is the other stream, so it takes the palette's one warm hue.
Between the two streams there is no order to preserve and none is claimed — two pipes read by two
threads, which is all a terminal has either. Recorded, not built: the list does not follow the
newest line, so a long run has to be scrolled; auto-follow needs the viewport height and a "the
reader has scrolled away" rule, which is `reveal_row`'s shape and its own piece of work.

**What stops a run**: the Stop button, a rebuild, the next run, and the window closing. A **rebuild**
stops it for three separate sufficient reasons — cargo is about to write over the file the process
*is*, `reopen_binary` is about to close the objects describing those bytes, and one scratchpad has
one output pane. The **next run** stops it because two generations of output arriving into one list
is a pane with no answer to "what is this". An **edit** stops nothing, deliberately: a run is of an
executable and not of the buffer, and a keystroke that killed the reader's program would make it
impossible to take a note about what it printed. A **project switch** stops nothing either, and for
a reason that is already settled — `Pad` is not one of the states in `ProjectStates`, because
there is one scratchpad and it belongs to the app rather than to a project, so a switch never
touches it.

**A rebuild replaces rather than accumulates.** `reopen_binary` is `close_binary` followed by what
the toolbar's Open does, in one handler: a binary is a **path** throughout this app — that is what
`close_binary` closes by and what `project::binaries` derives the saved list from — and a rebuild
writes the same path with different bytes, so two generations of one file cannot both be in the
objects list. The cost is real and is the reader's: the tabs for that file's functions, their
viewing positions and the history entries into them go with it. Keeping them would be `Rebuilt`'s
resolve-by-name machinery pointed at a live state instead of at a session file.

**Identity throughout the UI is `Arc` pointer identity**, not names or indices: list keys are
`Arc::as_ptr(..).addr()` and every prop `PartialEq` is hand-written in terms of `Arc::ptr_eq`. That
matters twice — duplicate symbol names across objects stay distinct, and `#[derive(PartialEq)]` on
an `Arc<T>` field would deep-compare on every parent render.

### Testing the UI

`freya-testing` runs the whole app — components, hooks, effects, layout, events — with no window,
no GPU and no event loop, on the test's own thread. It is a dev-dependency for `src/ui/tests.rs`
and for nothing else, and the binary's whole suite runs in under two seconds, so
a test written to settle one point costs less than a `cargo run` and a look and is worth writing
even when it is going to be deleted again. Keep the ones that pin a mechanism — that a component
with no props re-rendered because it read `palette()`, that a superseded answer was dropped — and
delete the ones that only proved the code just written does what it says.

What it can be asked: **any control that can be pressed** and the state it changes, **a drag,
including a drop into the dock**, **a scroll and where it lands**, **a keyboard binding** with the
modifiers built by hand (`press_key` hardcodes `Modifiers::default()` and cannot express Ctrl),
**a row height, width or position as laid out** rather than as requested, **anything a worker
thread answers**, and **which component re-rendered** — that last has no other test shape at all.
What it cannot: whether it *looks* right; any text width, which is really shaped by the machine's
own fonts and so is an assertion about the machine; the platform's own behaviour, which is stubbed
(the cursor icon, the desktop's real theme, the clipboard, the file dialog); and anything timed,
there being no virtual clock, so a timing test costs its own duration in real seconds. Those are
what launching the app is for.

`agents/Headless.md` is the full account, checked against `freya-testing` 0.4.3's sources rather
than its README: the runner's whole surface, the rule that a pass is *either* a render *or* a
round of task polling (which is why a change needs somewhere between *n* and *2n*
`sync_and_update`s and why the tests here loop rather than count), and the house rule that a
headless test has to be made to fail first on the *mechanism* it claims to test.

### Gotchas before editing the UI

- A `State`'s `peek`/`read` hands back a guard, and an `if let` holds its scrutinee's temporary
  until the end of its **body** — so `if let Some(x) = *state.peek() { state.set(..) }` compiles and
  panics the moment it runs. `let ... else` and `match` end theirs with the statement. Bind the read
  to a `let` of its own before any write. That class of bug is invisible to every other test in
  the repo, and is what the headless tests in `src/ui/tests.rs` were first written for.
- There is no `.hover()` pseudo-state. A hoverable row is a `Component` with `use_state(|| false)`
  plus `on_pointer_over`/`on_pointer_out` (`over`/`out`, not `enter`/`leave`, so hovering a child
  keeps the highlight).
- `VirtualScrollView`'s builder closure is never compared across renders, so anything the rows
  depend on must go through `new_with_data`, not be captured.
- A row's height must equal the `item_size` given to the `VirtualScrollView` over it, or scrolling
  misaligns. There are **two** of them — `list_row_height()` for rows in the interface font and
  `code_row_height()` for rows in the fixed-width one — so a view and its rows have to agree about
  *which*, as well as about the number. Both are functions of the fonts and no longer a `const`, so
  never write a literal row height anywhere; the two halves are safe only because both are read in
  the same render pass. This is also why variable-height rows are not free.
- `Size` has no `From<f32>` — write `Size::px(300.)`. But `.padding`, `.spacing`, `.margin` and
  `.corner_radius` do take plain `f32`.
- `label()` and `paragraph()` do not implement `StyleExt`, so they have no `.background()` /
  `.border()`; wrap them in a `rect()`.

# The analysis crate

`crates/analysis` is framework-free: object parsing, the data model, demangling, lazy DWARF line
info in both directions, the disassembler seam and the robustness suite. Nothing here knows freya.

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
scanned for" rule; a prebuilt LLVM DLL with no COFF symbol table at all goes from zero
functions to 22 918 on the strength of it. One symbol per address, earliest source winning
(symbol table > dynamic symbol > export > entry point), since an export is very often the
symbol table's own function under a second name. The symbol table itself may hold two names for
one address (an alias, an assembler label) and both are kept, but `Section::symbols` — the
sorted list `estimate_size` binary-searches — holds each address **once**: a repeated entry
made the search land on either twin and answer 0 for an aliased symbol, which in an object
without DWARF was a function with no listing at all. The section comes from
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
the 196-member rlib; 32 ms against 1.6 s on the 331 MB binary). Nothing in the crate
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
of the LLVM DLL's exports derived megabytes and one derived 3.7 MB, which is 772 302 instructions
decoded *per render*. `.pdata`/`RUNTIME_FUNCTION` is the real fix and is its own Goals item.

**Names are demangled in one batch per object, on stacks sized for them** (`demangle.rs`). A
mangled name is bytes out of a string table, and it is the *file* that chooses how deep the
demangler reading it recurses: `msvc-demangler` 0.11 has no recursion limit at all (`P` → pointee
→ type → pointee, one byte per level) and `cpp_demangle`'s is deep enough that reaching it is
megabytes of stack, so a 209-byte name overflows the 2 MiB a `std::thread` gets — and a stack
overflow is an **abort**, which no `catch_unwind` turns back into "this symbol has no demangled
name". Two bounds together: a name over `MAX_MANGLED_NAME` (2048 bytes, against a longest of 1038
across every input tried) is not demangled at all, and the rest are demangled on a thread
with `DEMANGLE_STACK` (64 MiB, a reservation and not a cost) — except where every name in the
object is under `SHORT_MANGLED_NAME` (64) and there are no more of them than one grain, which is
every fixture in the test suite and is the caller's own stack's business. A name no demangler
will take is displayed exactly as the file wrote it, which is what an unrecognised name already
did.

**Demangling is the last of the open-time cost, and it is what this crate parallelises.** After
the lazy line info, the lazy DWARF context, the lazy subprogram extents and the worker-thread
disassembly, it was the only expensive thing left at open time that is not simply reading the
file: 281 ms of the 331 MB binary's 1 437 ms, 78 ms of the 196-member rlib's 181 ms
(release; debug numbers overstate every one of these and are not worth quoting). What is left
beside it — the read and the walk of the symbol table — *is* the objects and symbols lists.

`demangle::batch` spreads one object's names across a **process-wide pool of big-stack threads**:
`available_parallelism` capped at `MAX_THREADS` (8), each with `DEMANGLE_STACK`, started on the
first batch that needs one (`OnceLock`) and kept for the life of the process. A pool and not a
thread per object because 64 MiB is not a thread to create 237 times; capped at 8 because
demangling is a fraction of an open and each thread keeps its deepest recursion committed once it
has taken one. The jobs pull the batch in grains of `GRAIN` (256) names off one shared cursor
rather than being dealt equal shares, since a name's cost is superlinear in its length and where
the long ones sit is the file's business — an even split hands one thread the object's whole C++
section. **The answer does not depend on the scheduling**: a grain is handed back over a channel
with the index it started at and written there, so the vector is the batch's own order and two
runs over one file agree. The names are *moved* into the shared batch (`demangle::Names`, an
`Arc<Vec<Option<String>>>`) and moved back out of it in `parse_object`, because a job outlives the
frame that submitted it and cannot borrow one; 115k names is not a copy worth making for that. A
job never submits a job, which is why a bounded pool cannot deadlock itself however many opens are
in flight, and a batch whose grain never comes back — a pool thread killed under it — leaves
those names as the file wrote them rather than waiting. A pool that would not start at all falls
back to the one big-stack thread this used to be.

No dependency was added for it: `rayon` is in the lock already (transitively, under `image`), but
its threads would still have to be a pool of this crate's own to get the stack size, which is the
whole of what is hard here, and what is left over is a queue and a cursor.

Measured release, best of 3, on a loaded machine (five other agents building): the app's own debug
binary 1 701 → 1 414 ms and the crate's own rlib, now 237 members, 383 → 151 ms. The archive's
share is larger than its demangling alone, because the old path also created and joined a 64 MiB
thread per object. **Only demangling is parallel.** `open_files_streaming` still walks the paths
and an archive's members in order on one thread, so objects reach the caller as they parse and in
file order; parallelising that level is its own open item in `notes/Goals.md` and wants a reorder
buffer to keep that order.

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
  programs pile up (52 229 of 54 109 rows overlapped, measured on the 196-member rlib).
  `line.rs` does what a linker does: `section_biases` gives each **text** section of a
  **relocatable** object a place of its own, `relocate` adds the bias, and the query adds it and
  subtracts it from every row returned. Both limits matter — a linked image holds real addresses
  literally and must be left alone, and an absolute relocation in a debug section is often an
  offset into another `.debug_*` section rather than an address. Hence `Object::line_info` takes a
  `&Section`: a bare range is not a question the crate can answer.

The bias moves exactly what `relocate` moves, and a unit's declared ranges need not be among them.
A line program's `DW_LNE_set_address` is always relocated in a relocatable object, so a sequence
always follows its section; a unit's range list usually does too, since DWARF 4 states a range as a
pair of addresses and DWARF 5 has `DW_RLE_start_length` — but neither obliges a producer to. A
range can also be two offsets from a base the unit gives as a `DW_AT_low_pc` of 0 it never
relocates (`DW_RLE_offset_pair`), and offsets follow nothing. The unit is then left declaring a
range its own code is no longer in, and `addr2line` will not look inside a unit whose ranges miss
the probe: a silent "no line info" for every section the bias moved, which is worse than a wrong
answer because nothing about it looks wrong. So `stale_range_lists` reads each unit's root DIE
after the first load and ends any `DW_AT_ranges` list that carries no relocation of its own where
it begins — zeroed in place rather than removed, so the offsets units hold into the section are
still in bounds and read there as a list of no ranges. `addr2line` then takes that unit's ranges
from its line program's sequences, which did move. A list is judged by its **first entry**, which
is where the relocation lands in every form that states an address, `DW_RLE_base_address` included:
the offset pairs rustc writes for an inlined subroutine's ranges are offsets from a base that *is*
relocated and are correctly left alone. Only a unit's own list is examined and only that one is
dropped; the lists its children hold are read by nobody here and are not ours to rewrite. An object
with relocations in `.debug_addr` is declined rather than judged, since DWARF 5's `DW_RLE_startx_*`
state their addresses there. Measured on the 196-member rlib the rule fires on nothing, and the
root-DIE pass costs 1.5% of a sweep of every symbol's line info and extent (200 ms -> 203 ms).

**The reverse mapping is an index, and a whole-object one** (`line/source.rs`). "Which functions
was this line compiled into" is not a question about one symbol, so it is not a query but a table:
built on the **first source question against an object** and never before one, behind a `OnceLock`
beside the two `Mutex`es, and empty rather than absent if building it panics. It is what Step 1d's
source-driven tab, Step 4's find-all and Step 5's instance picker each need, and it is the whole of
what the crate owes them — *where inside* a symbol the line's code sits is the forward direction's
question and is already answered, so a caller walks index → symbol → `line_info` → rows and there is
one definition of "this line's rows" rather than two that can drift.

The build is one pass and its **order is load-bearing**: every symbol's extent is taken *first*,
before the context's lock is held, because `SymbolData::extent` reaches `Dwarf::extent_inner`, which
takes that same lock, and a `Mutex` is not reentrant — computing an extent inside the row loop
deadlocks the first object anyone asks. Then one `find_location_range(0, u64::MAX)` over the whole
address space (safe where `subprogram_extent` had to decline `u64::MAX`: that unchecked `probe + 1`
is in `find_units`, and this goes through `find_units_range`), with each row attributed to the
symbols its addresses fall in. Addresses stay **biased** throughout, so the section bias that tells
two functions at address 0 apart is applied once and never undone. The extent used is
`SymbolData::extent` and not the next-symbol estimate: the index and `SymbolData::line_info` then
cannot disagree about what a symbol covers, which is the invariant a caller walking index → symbol →
rows depends on and the one a test asserts over every fixture, the two committed gcc objects
included. **A file is matched exactly**, on the string `addr2line` renders — which is by
construction the string `LineInfo::files` spells, so a name out of the forward direction can be
handed straight back. Nothing here normalises a path or asks the filesystem about one; two objects
whose `DW_AT_comp_dir` disagree do not join, and that is a cross-object question for whoever asks
one.

Measured, release, first ask: the 331 MB binary 2.2 s — **2.0 s of it the extent pass** and 0.23 s the
line-program walk — for 2 096 files and 624 544 `(line, symbol)` pairs, 10 MB of them, taking the
process from 756 MB to 1.23 GB with the parsed line programs held; the 196-member rlib 94 ms
for all 196 objects, 862 files, 25 870 pairs. Every ask afterwards is two binary searches (5 µs).
The extent pass being nine tenths of it is the price of that agreement, and it is a DIE walk of the
whole object — the cheap alternative, attributing by `estimate_size`, is one binary search per row
and lets the index name a symbol whose own line info does not name the line back. One line into many
symbols is not theoretical: `core/src/ptr/mod.rs:848`, `drop_in_place`, answers with **9 374** of
the 331 MB binary's symbols.

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
`Object::architecture` comes out of the header and `Assembly::decode` matches on it — `I386` at 32
bits, `X86_64`/`X86_64_X32` at 64. Bitness comes from the architecture and not from
`is_64()`, because the x32 ABI is 64-bit *code* with 32-bit pointers and is the one case the file's
class gets backwards. An architecture no backend claims is a **third answer**, not an empty listing:
`Assembly::undecodable` carries the architecture's name. It has to be *said*, because there is no
byte sequence a decoder could refuse on — the same bytes that are an aarch64 function are a
confident page of x86 nonsense, which is what this used to print, and an empty listing on its own
reads as a symbol that holds no code. `assembly` still answers `None` for one thing only: a symbol
with no bytes at all. Nothing in the UI reads `undecodable` yet, so such an object currently draws
an empty pane rather than the reason.

The trait is **one call wide** (`Disassembler::disassemble(&Code) -> Decoded`) and is shaped by what
`Assembly` needs rather than by what a disassembler library offers. **The dispatch through it is
generic and not dynamic**: `Assembly::decode` is the one `match` over the architecture, each arm
naming a concrete backend, and `Assembly::decoded` — the whole decode path, the backend's call and
the branch resolution together — is generic over the backend and compiled once per one. Nothing is
boxed and no signature says `dyn`, which also keeps every type here nameable by a caller. What that
buys is not the virtual call, one per symbol being nothing, but the allocation going away and the
backend's formatting and span-mapping becoming inlinable into the per-instruction loop. The trade is
that a new architecture is a new arm rather than a new impl behind a registry, which is what a set
closed at compile time wants anyway. `Code` is the bytes, the address
they sit at, and one question — `Code::relocation`, asked per instruction, because a relocation
names a byte range and never an operand number. `Decoded` is the rows plus each branch's *address*;
turning those into row indices is `Assembly::decoded`'s binary search, so `edges`' drop rules hold
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
`branch_span` is its **twin** and is recorded by an override of `write_number`, which is how a
branch target reaches the output: it is the span an instruction's *own* displacement was printed
into. The two are exclusive by construction, since a branch covered by a relocation is not one
that named an address, so a row has at most one link and the same three-child split serves both.

**Branch edges** (`Assembly::edges`) are the branches staying inside one symbol, for the arrow
gutter. Both ends are **indices into `instructions`**, not addresses, because that is what a row
can be asked about and it makes the answer independent of where the symbol sits. A backend hands
back the *address* each branch names — a forward branch names one no instruction has been decoded
at yet — and `Assembly::decoded` resolves them, which is what keeps the four rules below one
decision rather than one per backend. `from`/`to` are in
*execution* order (a backward branch has `from > to`); `first()`/`last()`/`is_backward()` sit on
top. A **call is not an edge** even when it lands inside the symbol, because control comes straight
back. Four things are dropped rather than drawn, each of which would be a line to a place it does
not point at: a branch out of the symbol, one landing mid-instruction, one whose displacement is a
relocation placeholder (tested on the *raw* relocation lookup, since a branch relocated against a
section carries no text symbol while its displacement is just as meaningless), and `jmp $`.
Those four keep their `branch_span` all the same — the number is where the number is — so
**the span and the edge are separate answers** and a caller that wants to *follow* a branch needs
both. `Assembly::edge_from` is the pairing, and it is a binary search rather than a scan: an
instruction names at most one target and a backend decodes from the front, so `from` ascends
strictly across `edges`.

**"Never panic on any file input" is tested two ways, and they are different jobs.**
`tests/mutations.rs` is the **search**: it takes every fixture the suite builds — both committed
gcc objects, the synthesized DWARF one, the ELF `.so` and the PE DLL — and truncates it at every
length, writes poison values (`0`, `u32::MAX`, `u64::MAX`, the file's own length…) into every
numeric field of every header, section header, symbol and relocation, and splats pseudo-random
runs over it, running the whole pipeline over each result. It is sampled by an even stride and
seeded from a constant (never `rand`, never the clock), so which cases run is fixed and it stays
around two seconds — 2.1, of which the reverse index costs the last tenth. `tests/robustness.rs` is the **regression suite**: one named, minimal fixture
per defect that was actually found, because a sweep that goes green tells you nothing about which
bug it was that stopped happening. `common::parse_and_walk` is the one definition of "ask a parsed
object everything", shared by both. The rule that goes with them is the user rule in `AGENTS.md`: a minimal
test case every time something is found wrong, and **checked arithmetic in preference to a wider
`catch_unwind`** — the guard is for a dependency's bug, never for ours. Note also what *cannot* be
caught: a stack overflow aborts, so anything recursing over file-controlled input (the demanglers,
above) has to be bounded before the call rather than wrapped. `demangle/tests.rs` pins both halves
of that for the pool — that a split batch answers in its own order, and that a 1000-level name in
one lands on a pool thread and not on the submitter's stack, which is a test that does not fail
but *aborts* when it is wrong.


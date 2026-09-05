# The analysis crate

`crates/analysis` is framework-free: object parsing, the data model, demangling, lazy line info in
both directions (from DWARF or a PDB), the disassembler seam and the robustness suite. Nothing here
knows freya.

**Parse pipeline** (`open_files_streaming` -> `parse_object`): each selected file is first tried as
an `ArchiveFile`, with every member parsed as a separate `Object`, and then the file itself is
*also* parsed as a plain object. So a non-archive contributes one `Object` and an archive one per
member. Failures are swallowed (`.ok()`), so a file that will not parse just never appears. Reading
and parsing run on a `std::thread` and come back over an `async_channel`, so a large binary does not
freeze the UI.

**Objects are handed over as they are parsed, not collected.** `open_files_streaming(paths, emit)`
calls `emit` with a `Progress` per event: `Parsed(object)` and, once per path *whatever* came of it,
`Finished(path)`. There is deliberately no *start* event: the caller supplied the paths and they are
walked in order, so the only thing it cannot already know is when one is done with. It is a callback
and not a channel or an iterator because the crate stays framework-free. A channel would make it
pick one, and pick bounded or unbounded, which is a backpressure policy belonging to whoever draws
the result; an iterator would mean self-borrowing the file's bytes across a yield. `emit` answers a
`ControlFlow`, which is how a walk nobody is waiting for stops where it stands: a closed 331 MB file
is not parsed to the end into a value that will be dropped. Its one honest limit: a single answer
cannot say "skip the rest of *this* file but go on to the next", so a multi-file request in which
one file is closed goes on parsing that file and drops the rest at the caller. `open_files` is that
same callback closing over a `Vec`, for the tests and anything with nowhere to put objects one at a
time. The digest stays **one pass per file**: `ObjectData::whole_file` is built once at the top of
each path and every member is cut from it. That is the thing streaming must not quietly turn into
196 hashes of the same 20 MB.

**Data model**, built once at open time and shared via `Arc`. Only `SymbolKind::Text` symbols are
kept, plus, for a **linked image only**, the code it declares elsewhere (`declared_code`):
`dynamic_symbols`, `exports` and `entry`; for a PE whose `.pdb` is found beside it and matches, the
**procedures** that PDB records (`S_GPROC32`/`S_LPROC32` with a nonzero length; `Pdb::procedures`,
below) and then its **publics** (`S_PUB32` flagged as code or a function; `Pdb::publics`); and the
**unwind entries** the image's own table states. For an x86-64 PE that table is its exception
directory (`.pdata`: one `RUNTIME_FUNCTION` per function with unwind info, a begin and an end and no
name, plus one byte of the `UNWIND_INFO` it names for the chained flag). For an ELF it is its
`.eh_frame` (one FDE per function with any, a start and a length; the same format on every
architecture, and on x86-64 every function has one by default, leaves included; no fragment flag, so
a `.cold` part is a function of its own). `unwind::entries` is the one part that reads call-frame
information, and only for a linked image, since a relocatable object's FDEs are written before their
addresses are and, read as they lie, fall inside `.text`. All of them are *declared*, the PDB's by a
file matched to the image by GUID and age and the unwind table's by the image to its own loader, so
this keeps the "nothing is scanned for" rule. A prebuilt LLVM DLL with no COFF symbol table at all
goes from zero functions to 22 918 on the strength of the exports; `rustc_driver.dll` from its 15
241 exports to 70 728 symbols on the strength of its PDB's procedures and 115 861 with its publics;
a stripped `rustc.exe` from `<entry point>` alone to 412 and 415. There is one symbol per address,
earliest source winning (symbol table > dynamic symbol > export > entry point > PDB procedure > PDB
public > unwind entry), since an export is very often the symbol table's own function under a second
name. The PDB comes last so a name the image itself states is never displaced by the debug file's
spelling of it, and its 82 933 procedures collapse to the 55 487 the image did not already name,
folded functions sharing an address being one place. Its publics come after its procedures because a
procedure carries a display name and a length and a public only a decorated name and an address. But
the publics are the linker's table of every externally visible symbol, so they survive a
**stripped** PDB (`/PDBSTRIPPED` keeps them and drops the module streams) and name what no module's
symbols do: a module that shipped without debug info (2250 of `rustc_driver`'s 2907 have no stream),
thunks, assembler code. 82 900 of its 141 498 publics are flagged as functions (the other 58 598 are
data, and `rust-lld` never sets the `code` flag alone), and 45 133 of those are addresses nothing
else named. The unwind entries come last of all because they carry no name. One at an address
anything else named adds nothing; in the three committed DLLs every entry's begin is an export's or
a procedure's. One nothing named is called `<function 0x140001000>` by its address, so that 20 000
of them in one list are told apart, with the entry's stated length as its declared size. It is
`<fragment 0x140001000>` instead where the entry's unwind info is **chained**
(`UNW_FLAG_CHAININFO`): a cold part, or the piece after a mid-body stack adjustment, of a function
with a primary entry elsewhere, which Microsoft calls a *function fragment* (195 of the LLVM DLL's
68 507 entries, 481 of `rustc_driver`'s 218 434). An unwind info that cannot be read, or is of
another version, leaves the entry a function, its range being stated either way. Not to be confused
with a *funclet*, an outlined `catch` or cleanup body, which has a primary entry of its own and is
told from a function only by the handler's private data (Goals). Parsed with no `.pdb` beside it,
the no-export DLL fixture goes from no symbols to its three. Measured, release: the LLVM DLL goes
from its 22 918 exports to 73 793 symbols on its 68 507 entries (50 875 of them functions nothing
named; every entry's begin is covered), `rustc_driver.dll` from 115 861 to 234 070 on 218 434
entries (118 209 nameless), and the C-API `LLVM-C.dll` from its 1 221 exports to 59 407. On ELF the
table adds no symbol while the symbol table is there (every one of `librustc_driver.so`'s 172 169
FDEs begins at a `.symtab` function, `st_size` equal to the FDE's length) and names everything once
it is gone: a stripped copy goes from the 16 728 functions its `.dynsym` names to 172 169. Reading
the table costs a tenth to a fifth more at open and pays back at the first extent: every covered
symbol's is stated (115 096 of `viewer-sample`'s 115 577, 197 375 of the `.so`'s 197 381) where it
was a DWARF walk, which is what the reverse index's first ask below is made of. A procedure's name
is the compiler's display name (`add`, `core::ptr::drop_in_place<T>`), which goes through the same
demangling batch as an export's and comes out untouched. A public's is the decorated name as the
linker saw it (`?add@@YAHHH@Z`, `_ZN4core3ptr…`, a plain `add` for C), and goes through the same
batch to come out demangled, the raw spelling kept as `name`. A procedure's length and an unwind
entry's stated length are the symbol's *declared* size where an export's and a public's is 0; the
extent used is still `SymbolData::extent`. The symbol table itself may hold two names for one
address (an alias, an assembler label) and both are kept, but `Section::symbols`, the sorted list
`estimate_size` binary-searches, holds each address **once**: a repeated entry made the search land
on either twin and answer 0 for an aliased symbol, which in an object without DWARF was a function
with no listing at all. A symbol whose name will not read out of the string table is a place in the
file all the same and is kept: its address goes into `Section::symbols`, so the symbol below it is
bounded by it, but **not** into `known`, so an export, a PDB procedure or public, or an unwind entry
can still claim that address and give it a real name. Only where none does is the symbol listed at
all, under `<function 0x…>`, which is why the walk that builds the symbols runs after
`declared_code`. The section comes from looking the address up in the kept **text** sections, which
doubles as the filter keeping exported *data* out. A relocatable object is skipped entirely:
`entry()` answers 0 for a `.o`, and 0 there is a real function's first byte. The two nameless
declarations, the entry point and an unwind entry, are called `<entry point>` and `<function 0x…>`
or `<fragment 0x…>`, in angle brackets because no assembler, linker or mangling scheme emits them,
so none can collide with a real one. The three are one type, `made_up::MadeUp`, whose `Display` is
the only place they are spelled and whose `of` reads one back — by rendering the three candidates
for an address and comparing, so there is no second spelling to keep in step. What a symbol holds
is still the `String` it builds. The type itself travels as far as the app's *saved* places, which
write which name it is and the symbol's address rather than the spelling, so a bookmark on one
outlives a change of spelling (`agents/Persistence.md`); `made_up/tests.rs` pins today's three all
the same. And it carries the one thing every made-up name shares: it is not the file's own, so no
demangler is ever offered one.
`Object` holds `symbols: HashMap<SymbolIndex, Arc<SymbolData>>`
(for relocation-target lookup), `symbols_sorted` (name-sorted, for the UI list) and `by_address`
(placed-address-sorted, built on the first disassembly, for a call target's name; below).
`Object::data` is an `ObjectData`, an `Arc<[u8]>` of the whole file plus a `Range`, kept for the
object's lifetime, because parsing keeps decompressed bytes only for sections holding text symbols
and the lazy line-info pass needs the rest. Every object from one file shares that one allocation,
so an archive costs its bytes once. It also carries the file's `FileDigest`: xxHash64 of the *whole
file*, taken once in `ObjectData::whole_file` because the bytes are in hand there, and an archive
member is cut from that same value, so 196 members cost one pass (32 ms against the 1.6 s the open
takes on the 331 MB binary). Nothing in the crate reads it: it exists so a restore can tell the file
it saved from one rebuilt underneath it. `Section` owns decompressed bytes, relocations keyed by the
address the bytes they patch sit at, a sorted list of its text symbols' addresses, and the ranges
the file's unwind table states for its functions (`unwind`, sorted by start, each start once, ends
clamped to the section's bytes; empty for a file with no table read). That key is the parse's own
doing: `object` hands back what the format states, an address in ELF and COFF but an offset from the
start of the section in Mach-O, which lays its sections out one after another. So `parse_object`
adds a Mach-O section's address as it builds the map, and `Code::relocation` can ask by address
whatever the file is. The debug sections are relocated straight from `object`'s iterator
(`line/dwarf.rs`'s `relocate`) and want the offset as it comes, since it indexes the bytes being
patched. `SymbolData::estimate_size` derives a symbol's extent from the *next* address in the symbol
list, **clipped to the section's own bytes**, since that list is numbers out of the file and one
wild `st_value` in it would otherwise cost the symbol *above* it its listing rather than only
itself. Declared sizes are frequently 0 in ELF/COFF, which is why the derivation exists at all.
`SymbolData::extent` is the answer that is actually used, and has three
answers in order. First, **the end the unwind table states**, where an entry covers the address,
whatever named the symbol. That is the image's own statement, to its loader, of the very bytes the
unwinder covers, so neither the estimate nor its cap bounds it and the debug info is not asked. Only
the next symbol clamps it, because a listing is one stretch per symbol decoded as that symbol's
extent, and an extent reaching past the next label would draw its rows twice; every entry's begin
being a symbol, that is also what stops a parent at the chained entry of its cold part. Then, **the
size the file declares for the symbol**, clamped the same way. Only an ELF `st_size` counts as one:
it is the ABI's own statement of how many bytes the symbol is, every mainstream toolchain fills it
in, and on the `.so` above it equals the FDE's length for all 172 169 functions the `.eh_frame`
covers. No other format's nonzero size means that. A COFF function symbol's is the `TotalSize` of an
auxiliary function-definition record, written for COFF's line-number data rather than to measure
code; XCOFF's is a csect's length, and one csect can hold several functions; Mach-O states no size
at all. A size that is *wrong* rather than 0 would be taken as fact, so the trusted set is an
allowlist of one and a format joins it on evidence. The clamp catches an over-reaching declaration —
hand-written assembly with a `.size` past the next label — while one that is too small stands, as an
unwind entry's stated end and a `DW_AT_high_pc` already do. What this is for is the ELF with a
symbol table and no `.eh_frame`, built `-fno-asynchronous-unwind-tables`: every function of it used
to cost a DIE walk for an answer its symbol table had already stated. **Else the smaller** of the
extent the debug info declares (a `DW_TAG_subprogram`'s `DW_AT_low_pc`/`DW_AT_high_pc`, or a PDB
procedure's length) and the estimate. The estimate over-reaches into padding, but the debug info's
extent describes the *function*, so a second symbol inside one function (an alias, an assembler
label, a split cold part) would otherwise swallow the next function. The derivation is capped at
`MAX_DERIVED_SIZE` (1 MiB). That is not a claim about how long a function can be, but the point past
which it is certainly describing something else: a stripped PE's export table is sparse, so nine of
the LLVM DLL's exports derived megabytes and one derived 3.7 MB, which was 772 302 instructions
decoded *per render*. The unwind table is the fix for that. Where there is one (an x86-64 PE, an ELF
with an `.eh_frame`) every entry's begin is a symbol and every covered symbol's end is stated, so
the cap reaches only a symbol no entry covers (a leaf without unwind info on a PE, hand-written
assembly, a mutated table) and, on an image without one, everything that declares no size either —
an ARM64 PE or a Mach-O, where it stays. Measured, release: none of the LLVM DLL's 73 793 extents
reaches the cap now; the extent pass over all of them is 4.6 ms, where `rustc_driver.dll`'s 234 070
take 756 ms because the 15 636 no entry covers each go to the PDB.

**Names are demangled in one batch per object, on stacks sized for them** (`demangle.rs`). A mangled
name is bytes out of a string table, and it is the *file* that chooses how deep the demangler
reading it recurses. `msvc-demangler` 0.11 has no recursion limit at all (`P` → pointee → type →
pointee, one byte per level) and `cpp_demangle`'s is deep enough that reaching it is megabytes of
stack, so a 209-byte name overflows the 2 MiB a `std::thread` gets. A stack overflow is an
**abort**, which no `catch_unwind` turns back into "this symbol has no demangled name". Two bounds
together: a name over `MAX_MANGLED_NAME` (2048 bytes, against a longest of 1038 across every input
tried) is not demangled at all, and the rest are demangled on a thread with `DEMANGLE_STACK` (64
MiB, a reservation and not a cost). The exception is an object where every name is under
`SHORT_MANGLED_NAME` (64) and there are no more of them than one grain, which is every fixture in
the test suite and is the caller's own stack's business. A name no demangler will take is displayed
exactly as the file wrote it, which is what an unrecognised name already did.

**Demangling is the last of the open-time cost, and it is what this crate parallelises.** After the
lazy line info, the lazy debug-info backend, the lazy function extents and the worker-thread
disassembly, it was the only expensive thing left at open time that is not simply reading the file:
281 ms of the 331 MB binary's 1 437 ms, 78 ms of the 196-member rlib's 181 ms (release). What is
left beside it, the read and the walk of the symbol table, *is* the objects and symbols lists.

`demangle::batch` spreads one object's names across a **process-wide pool of big-stack threads**:
`available_parallelism` capped at `MAX_THREADS` (8), each with `DEMANGLE_STACK`, started on the
first batch that needs one (`OnceLock`) and kept for the life of the process. It is a pool and not a
thread per object because 64 MiB is not a thread to create 237 times, and capped at 8 because
demangling is a fraction of an open and each thread keeps its deepest recursion committed once it
has taken one. The jobs pull the batch in grains of `GRAIN` (256) names off one shared cursor rather
than being dealt equal shares, since a name's cost is superlinear in its length and where the long
ones sit is the file's business; an even split hands one thread the object's whole C++ section.
**The answer does not depend on the scheduling**: a grain is handed back over a channel with the
index it started at and written there, so the vector is the batch's own order and two runs over one
file agree. The names are *moved* into the shared batch (`demangle::Names`, an
`Arc<Vec<Option<String>>>`) and moved back out of it in `parse_object`, because a job outlives the
frame that submitted it and cannot borrow one; 115k names is not a copy worth making for that. A job
never submits a job, which is why a bounded pool cannot deadlock itself however many opens are in
flight, and a batch whose grain never comes back (a pool thread killed under it) leaves those names
as the file wrote them rather than waiting. A pool that would not start at all falls back to the one
big-stack thread this used to be.

No dependency was added for it: `rayon` is in the lock already (transitively, under `image`), but
its threads would still have to be a pool of this crate's own to get the stack size, which is the
whole of what is hard here, and what is left over is a queue and a cursor.

Measured, release: the app's own debug binary 1 701 → 1 414 ms and the crate's own rlib, now 237
members, 383 → 151 ms. The archive's share is larger than its demangling alone, because the old path
also created and joined a 64 MiB thread per object. **Only demangling is parallel.**
`open_files_streaming` still walks the paths and an archive's members in order on one thread, so
objects reach the caller as they parse and in file order; parallelising that level is its own open
item in `notes/Goals.md` and wants a reorder buffer to keep that order.

**Persisting the demangled names was built and thrown away.** It is not in the history, and the
`[D]` item in `notes/Goals.md` is the whole record of it. It worked and it was measured (a 37%
average open, best on archives, worst on the export-heavy DLL), but it cost a fourth place on disk,
an eviction budget and a hand-rolled binary format with its own checksum, and its corruption sweep
found two ways for it to be wrong, including a plausible wrong function name on screen. Parallel
demangling attacks the same cost without persisting anything, and is the thing to try first.

**Line info** (`line.rs`) is lazy and not touched at parse time, with one exception: a PE whose
matching `.pdb` is opened at parse for the procedures it names (`DebugInfo::pdb`, below), and even
there only the PDB's tables are read then; its line programs wait for the first question. It answers
two questions under one set of rules, the rows covering a range and a function's declared extent
(`Object::function_extent`), so everything below holds for both. `line.rs` is a **seam** that names
no debug format: `DebugInfo` holds one `Backend`, a closed enum dispatched by `match` the way
`Assembly::decode` is, and `line/dwarf.rs` is the first backend and the only module that knows
DWARF's debug sections and `addr2line` (a DIE walk per unit visited for the extent, cached by unit
offset; `gimli`'s call-frame reader is `unwind.rs`'s, which is not a backend). Every backend's rows
go through one `RowCollector`, whose `finish` is where `LineInfo`'s invariants are *made* (below),
so they hold whoever produced the rows. `Object::debug_info` is a `DebugInfoCache` caching *both*
answers, the built backend and the fact that there is none, so an object without debug info costs
one section-table scan ever. `None` from `line_info` means "no line info" for every reason at once:
no debug info, debug info in a format no backend reads (CodeView), debug info that will not parse,
or debug info that says nothing about the range asked about. Four design points are load-bearing:

- **No self-borrow.** Readers are `gimli::EndianArcSlice`, an `Arc<[u8]>` per DWARF section, built
  by copying, decompressing and relocating, so the context owns its data and is `'static` rather
  than making `Object` self-referential. Cost: one allocation per debug section on first query.
- **`Sync` via a `Mutex`.** `addr2line::Context` caches parsed line programs in an `UnsafeCell`
  behind `&self`, so it is `Send` but not `Sync`; a backend's lock is its own, taken and released
  per question. `lib.rs` holds a `const _` assertion that `Object: Send + Sync` so this cannot
  regress silently, beside three more, `Symbol`, `Assembly` and `LineInfo`, which are what crosses
  into the app's analysis worker and back (`ui/analyzed.rs`, `use_analysis`). A field that stops
  being shared-safe is then a compile error in the crate rather than a borrow error in the UI, where
  the cheap fix would be to go back on the UI thread.
- **One query per symbol, not per instruction.** `SymbolData::line_info(&object)` returns an
  `Arc<LineInfo>` for the whole extent; the UI answers each instruction locally with
  `LineInfo::row_at`. Rows are ascending, non-overlapping, clipped to the range, and coalesced, but
  *not* contiguous. `line`/`column` are `Option`, because DWARF line 0 means "no line" and column 0
  means "left edge". Non-overlapping is an invariant *made* to hold in `RowCollector::finish` by
  clipping after the sort, not a property of any debug format.
- **An address alone is not a key in a relocatable object.** Sections there have no address until
  linked and rustc emits one `.text.<name>` per function, so every function lands on 0 and the line
  programs pile up (52 229 of 54 109 rows overlapped, measured on the 196-member rlib). The parse
  does what a linker does: `section_biases` (`lib.rs`) gives each **text** section of a
  **relocatable** object a place of its own, recorded on the section as `Section::bias` beside
  `Section::code`; `relocate` adds the bias, and the query adds it and subtracts it from every row
  returned. `line/dwarf.rs` asks `section_biases` again rather than reading the biases back off
  the sections the parse kept: the rule is the layout, and a text section whose bytes would not
  read is dropped from the parse but still has to be placed, or the rows relocated against it land
  on 0 where the first section already sits. The layout starts above the highest address the file
  states — a Mach-O `.o` states one per section — so nothing is moved *down* and a bias is never a
  wrapped value: `relocate`'s wrapping add and a query's checked one mean the same thing. It is
  decided at parse and not in `line.rs` because the listing of an object's whole code is laid out
  by the same rule, and one layout read twice cannot disagree with itself. Both limits matter: a
  linked image holds real addresses literally and must be left alone, and an absolute relocation
  in a debug section is often an offset into another `.debug_*` section rather than an address.
  Hence `Object::line_info` takes a `&Section`: a bare range is not a question the crate can
  answer. `relocate` itself runs for a relocatable object only, for the same reason the bias does:
  a linked image's debug sections hold what the linker resolved, and one linked with
  `--emit-relocs` keeps the relocations that resolved them, which `object` attaches to their
  target section whatever the file kind. Applying one again adds the symbol's address a second
  time wherever the addend sits in the bytes rather than in the relocation (ELF `REL`, so i386 and
  ARM32), moving every address the debug info states.

The bias moves exactly what `relocate` moves (`line/dwarf.rs`), and a unit's declared ranges need
not be among them. A line program's `DW_LNE_set_address` is always relocated in a relocatable
object, so a sequence always follows its section. A unit's range list usually does too, since DWARF
4 states a range as a pair of addresses and DWARF 5 has `DW_RLE_start_length`, but neither obliges a
producer to. A range can also be two offsets from a base the unit gives as a `DW_AT_low_pc` of 0 it
never relocates (`DW_RLE_offset_pair`), and offsets follow nothing. The unit is then left declaring
a range its own code is no longer in, and `addr2line` will not look inside a unit whose ranges miss
the probe: a silent "no line info" for every section the bias moved, which is worse than a wrong
answer because nothing about it looks wrong. So `stale_range_lists` reads each unit's root DIE after
the first load and ends any `DW_AT_ranges` list that carries no relocation of its own where it
begins. It is zeroed in place rather than removed, so the offsets units hold into the section are
still in bounds and read there as a list of no ranges. `addr2line` then takes that unit's ranges
from its line program's sequences, which did move. A list is judged by its **first entry**, which is
where the relocation lands in every form that states an address, `DW_RLE_base_address` included: the
offset pairs rustc writes for an inlined subroutine's ranges are offsets from a base that *is*
relocated and are correctly left alone. Only a unit's own list is examined and only that one is
dropped; the lists its children hold are read by nobody here and are not ours to rewrite. An object
with relocations in `.debug_addr` is declined rather than judged, since DWARF 5's `DW_RLE_startx_*`
state their addresses there. Measured on the 196-member rlib the rule fires on nothing, and the
root-DIE pass costs 1.5% of a sweep of every symbol's line info and extent (200 ms -> 203 ms).

**The PDB backend** (`line/pdb.rs`) reads the other debug format a linked PE comes with: not
sections in the image but a **second file**, so it is the one backend that touches the filesystem.
`DebugInfo::load` tries DWARF first (a MinGW or clang PE can carry `.debug_*`) and a `.pdb` only for
an object with none. **Finding it**: the debug directory's CodeView record, which `object` already
reads (`pdb_info()`), names the `.pdb` by path, GUID and age. The path is the build machine's, so
three candidates are tried in order: the recorded file name beside the binary, the binary's own name
with `.pdb` beside it, and last the recorded path itself where it is absolute. A **UNC or device
path is never tried** (`\\host\share\x.pdb`, `\\?\C:\x.pdb`, anything beginning with two separators,
judged as a string since a Windows linker wrote it whatever this is running on): the recorded path
is bytes the binary chose, read at parse for every PE the reader opens, and opening a UNC path logs
the machine in to `host` over SMB with the reader's credentials before a byte comes back. Every
candidate is stat'd before it is opened, as `source.rs` does, so one naming a fifo does not block
the parse thread until a writer appears. **Matching it**: GUID *and* age both, the GUID naming the
build and the age the relink. An incremental relink keeps the GUID and bumps the age, and its `.pdb`
then describes code the image no longer has, which is worse than none.
The age compared is the DBI's, which the linker wrote; the info stream's own age is bumped by tools
that rewrite a PDB afterwards (source indexing) and may legitimately exceed the image's.
**Addresses**: a PDB states `section:offset`. Every one goes through the PDB's own `AddressMap` to
an RVA, which is also where an OMAP-rearranged image is undone (a path no fixture exercises beyond
its identity form), and onto the image base with checked arithmetic. So answers are in the virtual
address space a linked image's symbols already are, and a linked image has no section bias. **Per
module, on demand**: line info in a PDB is per module (one object the linker took in), found from an
address through the DBI's section contributions, a sorted table with a running `max_end`,
`source.rs`'s `SymbolRange` shape, built at load. A module is decoded whole the first time an
address in it is asked about (its rows through `RowCollector::finish` into one `LineInfo`, its
`S_GPROC32`/`S_LPROC32` lengths into an extent table) and kept, the way the DWARF backend keeps a
unit's subprogram extents. A row with no length, one whose successor sits below it, which only
assemblers emit, is dropped rather than given an end. Line 0 and column 0 are `None` as in DWARF.
`each_row` walks every module, so a first source question decodes the whole PDB, as the DWARF one
parses every line program. The DBI module list is a chain of variable-length records with no index,
so an index is reached only by parsing every record before it and **one walk serves every module a
question wants** (`Pdb::walk`, stopped after the last one asked for). A walk per module would cost
the square of a count the file states, and a module list of a few hundred honest megabytes declares
millions: the first source question would then hang the analysis worker, which no guard can catch.
**Two things a PDB has that DWARF-as-read does not**: a checksum per source file (`SourceHash`: MD5
from clang-cl and rustc, SHA-256 from MSVC since 2022, as the samples' CRT objects show), carried on
`LineInfo` beside the file name so a reader can tell the file they have from the one the compiler
read (`SourceDigests::of` takes all three digests of a file's bytes at once, so a file read once
answers any kind); and file names in the producer's spelling (`C:\...` from MSVC,
`/rustc/<hash>\library\...` from rustc), handed out verbatim as DWARF's are.
**The PDB is also a source of symbols**, the one debug format that is. A `/DEBUG` image has no COFF
symbol table, so what the image names is its exports and entry point (its `.pdata` states where its
functions are, not what they are called), and what the PDB knows is every function.
`Pdb::procedures` walks every module's symbol stream once for its `S_GPROC32`/`S_LPROC32` records
(name, `section:offset` through the address map onto the base, length), and `Pdb::publics` walks the
symbol records stream once for its `S_PUB32` records flagged as code or a function (decorated name,
`section:offset` the same way, no length). `parse_object` takes them as the last two *named* sources
in `declared_code`, in that order, before the nameless unwind entries (Data model, above). That
makes it the **one eager path through the seam**: `DebugInfo::pdb(file, path)` finds and matches the
`.pdb` as `load` would, declines a PE carrying DWARF of its own (the same "DWARF first" rule, asked
of `Dwarf::present` without building a context), walks the procedures and then the publics, and
hands back the backend it built. `parse_object` seeds that into the object's `DebugInfoCache`
(`preloaded`) so the first line question finds it there rather than opening the file again; an
object parsed without it keeps the lazy path unchanged. The walks hold nothing of the streams they
read but what they hand back. A module asked about later is read again for its lines, which is
exactly the first-question cost the lazy path had before, and holding every module's procedure table
from the walk would only duplicate what the symbols now carry as their declared size while the
stream still had to be read for its lines. The symbol records stream (`pdb2`'s `global_symbols`, the
one stream the publics are in, 229 318 records in `rustc_driver`'s) is read whole through
`BoundedFile` and dropped with the walk, since nothing later asks it anything. The whole of it
(open, match, both walks) is under the seam's `without_panicking`, so a `pdb2` panic anywhere in it
is "no PDB at parse" and the lazy path is left to try. **What is not read**: `/DEBUG:FASTLINK` PDBs,
which match and then answer nothing. A stripped PDB now answers its publics and nothing else (no
procedures, no lines, no extents), which is the shape the third committed pair stands in for
(below). The file stays open, read a page at a time through `BoundedFile`, never whole:
`rustc_driver`'s PDB is 268 MB. **Measured**, release, on the samples: `rustc.exe` (110 KB, a 3.7 MB
PDB) opens in 4.5–6 ms for 415 symbols. `rustc_driver.dll` (194 MB, 15 241 exports, the 268 MB PDB)
opens in 1.28–1.39 s for 115 861 symbols at 465 MB, against 738 ms for its exports alone: the extra
is one read of every module stream (2907 modules, of which 2250 have none) and 100 620 new names
through the demangling batch, the publics walk itself being 55 ms over 229 318 symbol records. Its
first line question is 0.5 ms, the PDB being open and matched already, and the first source
question, every module decoded, 1.3–1.5 s to 525 MB. Against the DWARF side's 2.2 s and +470 MB on a
331 MB binary that is the same shape at half the cost, one walk of the module list serving all 2907
of them. The open-time cost is the module streams' bytes: `pdb2` reads a module's stream whole,
lines and all, where the symbols are its first substream, so a `Source` that read only that far
would be the saving if it is ever worth it. Rust's legacy-mangled publics run up to 4059 bytes
(past `MAX_MANGLED_NAME`, so shown as written).

**The reverse mapping is an index, and a whole-object one** (`line/source.rs`). "Which functions was
this line compiled into" is not a question about one symbol, so it is not a query but a table. It is
built on the **first source question against an object** and never before one, behind a `OnceLock`
on `DebugInfo` beside the backend, not inside one, because it is built from what every backend
answers and not from any one's internals; and it is empty rather than absent if building it panics.
It is what the source-driven tab, find-all and the instance picker each need, and it is the whole of
what the crate owes them. *Where inside* a symbol the line's code sits is the forward direction's
question and is already answered, so a caller walks index → symbol → `line_info` → rows and there is
one definition of "this line's rows" rather than two that can drift.

The build is one pass and its **order is load-bearing**: every symbol's extent is taken *first*,
before the backend's lock is held. `SymbolData::extent` reaches `DebugInfo::extent`, which takes
that lock; `DebugInfo::each_row`, the one thing the index asks a backend for, holds that same lock
for its whole walk; and a `Mutex` is not reentrant, so computing an extent inside the visitor
deadlocks the first object anyone asks. The DWARF backend's `each_row` is one
`find_location_range(0, u64::MAX)` over the whole address space (safe where `extent` had to decline
`u64::MAX`: that unchecked `probe + 1` is in `find_units`, and this goes through
`find_units_range`), with each row attributed to the symbols its addresses fall in. Addresses stay
**biased** throughout, so the section bias that tells two functions at address 0 apart is applied
once and never undone. The extent used is `SymbolData::extent` and not the next-symbol estimate: the
index and `SymbolData::line_info` then cannot disagree about what a symbol covers, which is the
invariant a caller walking index → symbol → rows depends on and the one a test asserts over every
fixture, the two committed gcc objects included. **A file is matched exactly**, on the string the
backend renders, which is by construction the string `LineInfo::files` spells, so a name out of the
forward direction can be handed straight back. Nothing here normalises a path or asks the filesystem
about one; two objects whose `DW_AT_comp_dir` disagree do not join, and that is a cross-object
question for whoever asks one.

Measured, release, first ask: the 331 MB binary **0.43 s** (2.2 s before `.eh_frame` stated 115 096
of its 115 577 extents, 2.0 s of that the extent pass). The 0.23 s line-program walk is now most of
it, for 2 096 files and 624 544 `(line, symbol)` pairs, 10 MB of them, taking the process from 756
MB to 1.23 GB with the parsed line programs held. The 196-member rlib takes 94 ms for all 196
objects, 862 files, 25 870 pairs. Every ask afterwards is two binary searches (5 µs). The DIE walk
is still what a symbol without an unwind entry pays, deliberately: the cheap alternative,
attributing by `estimate_size`, is one binary search per row and lets the index name a symbol whose
own line info does not name the line back. One line into many symbols is not theoretical:
`core/src/ptr/mod.rs:848`, `drop_in_place`, answers with **9 374** of the 331 MB binary's symbols.

**The build has a budget**, since neither factor of the index's size is the app's to choose: it
holds one pair per row per symbol covering that row, a symbol table may name one address any number
of times, and nothing folds those — `SymbolData::extent` answers each alias its own declared size.
100 000 symbols at one address under a line program of 100 000 rows is a few megabytes of file
asking for 10^10 pairs, and Rust aborts on an allocation failure, the one failure the
`catch_unwind` around the build does not see. So the walk counts the pairs it pushes against 64 a
row, never fewer than 64 Ki and never more than 64 Mi, and past that hands back an **empty** index
— the "says nothing" answer a panicking build already gives, and empty rather than partial because
an index missing the rows the walk skipped would be wrong, where an empty one only says nothing. The rate is what a
crafted file inflates; the ceiling is what a file with rows enough would get around the rate with.
Measured, a 451 MB build of the app's own binary pushes 1 964 064 pairs over 2 112 859 rows: 0.93 a
row against the 64 allowed.

`without_panicking` (a `catch_unwind`) is `DebugInfo`'s and wraps the backend build and every
question at the seam, one net whichever backend is under it, for known reachable bugs in the
dependencies behind it. All are unchecked arithmetic on numbers a debug section states, and none is
something this crate can validate without parsing the debug info twice. In `addr2line` 0.21: a row's
length is `next.address - row.address`, so a line program that moves its address backwards is a
subtract-with-overflow panic on a file the user merely opened; and a unit's range is
`low_pc + high_pc`, which overflows for a unit whose length runs off the end of the address space.
That one panics while the context is being *built*, which is why the guard is around the build too.
What is *not* left to the guard is the third one: `find_units` asks about `probe + 1` unchecked, so
the DWARF backend's `extent` declines `u64::MAX` outright rather than catching the panic afterwards.
`pdb2` 0.10 has four of the same kind (a module's line data sliced at `start + size` unchecked, a
line block's size less its header, `section:offset + length` as a plain `+`, a string-table name at
a declared offset), all under the same net (`notes/upstream/pdb2.md`); and one that no guard
catches, a stream directory's declared length allocated before a byte is read, answered the way
`section_data` answers a lying compressed size: `BoundedFile` weighs every declared slice and their
total against the file's length first.

The guard is not the whole answer to the first of those. Overflow checks are on in a test and a
debug build and off in a release one, so where a debug build panics inside `addr2line` and answers
"no line info", a release build wraps: the backwards row's length becomes huge and the rows after
it come back lying below the query. So `line_info` clips a row to the query and drops one with
nothing left *before* taking the bias off (`clipped`). Subtracting first made a row below the
section's placement one that ran to the end of the address space, and `row_at` then answered with
it across the rest of the function — a confident wrong source line where there should have been
none. It is the one piece of the backend with a unit test of its own, no fixture being able to
produce that row in a build with the checks on.

**Disassembly** (`SymbolData::assembly`) goes through a seam: `disasm.rs` defines everything a
caller sees and names no backend, and `disasm/x86.rs` is the only module in the crate that mentions
`iced-x86`. **Which decoder is used is a property of the file, not of this crate**:
`Object::architecture` comes out of the header and `Assembly::decode` matches on it, `I386` at 32
bits, `X86_64`/`X86_64_X32` at 64. Bitness comes from the architecture and not from `is_64()`,
because the x32 ABI is 64-bit *code* with 32-bit pointers and is the one case the file's class gets
backwards. An architecture no backend claims is a **third answer**, not an empty listing:
`Assembly::undecodable` carries the architecture's name. It has to be *said*, because there is no
byte sequence a decoder could refuse on. The same bytes that are an aarch64 function are a confident
page of x86 nonsense, which is what this used to print, and an empty listing on its own looks like a
symbol that holds no code. `assembly` still answers `None` for one thing only: a symbol with no
bytes at all. The symbol's pane reads `undecodable` and names the architecture instead of a
listing; the section view, where a symbol is one stretch among many, draws the stretch's bytes as a
hex dump (`src/section.rs`, `agents/Panes.md`).

The trait is **one call wide** (`Disassembler::disassemble(&Code) -> Vec<Instruction>`) and is
shaped by what `Assembly` needs rather than by what a disassembler library offers. **The dispatch
through it is generic and not dynamic**: `Assembly::decode` is the one `match` over the
architecture, each arm naming a concrete backend, and `Assembly::decoded`, the whole decode path,
the backend's call and the branch resolution together, is generic over the backend and compiled once
per one. Nothing is boxed and no signature says `dyn`, which also keeps every type here nameable by
a caller. What that buys is not the virtual call, one per symbol being nothing, but the allocation
going away and the backend's formatting and span-mapping becoming inlinable into the per-instruction
loop. The trade is that a new architecture is a new arm rather than a new impl behind a registry,
which is what a set closed at compile time wants anyway. `Code` is the bytes, the address they sit
at, the object they belong to, and two questions asked per instruction: `Code::relocation`, because
a relocation names a byte range and never an operand number, and `Code::symbol_at`, for the address
an unrelocated call names. A row carries the *address* its own branch names (`Instruction::branch`);
turning those into row indices is `Assembly::decoded`'s binary search, so `edges`' drop rules hold
for every backend rather than once per backend. What stays *behind* the seam is everything x86
spells its own way: the `SymbolResolver` substitution, the per-instruction `rip_relative_addresses`
flip, `branch_target`'s flow-control judgement and `FormatterTextKind -> SpanKind`. Each instruction
is formatted into an `Instruction` implementing `iced_x86::FormatterOutput`, capturing
`(String, SpanKind)` spans for the UI to colour. `SpanKind` is the backend-independent stand-in for
`FormatterTextKind`; the app has no `iced-x86` or `object` dependency (`BinaryFormat`,
`Architecture` and `SectionIndex` are re-exported from `analysis` for that reason). The decode
loop's own arithmetic is checked: the instruction pointer is the symbol's address plus what has been
decoded, both of them the file's numbers, so a section placed at the end of the address space wraps
it and the offset derived from it is a slice index. The listing stops at the wrap rather than
indexing past the symbol.

**Relocation handling** is the subtle part, and all of it is x86's (`disasm/x86.rs`). A relocation
whose address falls anywhere in the instruction's byte range is resolved to an `Arc<SymbolData>`,
and the target's name is printed *in place of* the placeholder operand through iced-x86's
`SymbolResolver` hook, not by suppressing the number, which left the brackets the formatter had
already opened empty (`call qword ptr []`). Nothing maps a relocation back to an operand number, so
the resolver is armed once per instruction and the **first** operand asked takes it; a second
numeric operand keeps its real value. A rip-relative operand keeps its `rip+` wherever a relocation
covers it: `assembly` flips `rip_relative_addresses` **per instruction**, on exactly those with both
a relocation -- resolved to a name or not -- and a rip-relative memory operand, because
`format_memory` would otherwise fold the displacement into an absolute address the encoding does not
have. The displacement a relocation covers is a placeholder either way, so a resolved one would
print `[target]` and an unresolved one a number that names nothing. The option cannot be set per
operand, since `format_memory` reads the global one. The placeholder itself is still printed when
it is not zero -- `[rip-4]`, `[rip+8]`, as `objdump` prints it -- because those bytes are in the
encoding whatever they stand for; a zero one iced leaves out. A name, though, replaces the whole
number, whichever operand takes it -- a displacement, an immediate, a branch's own rel32 -- so an
addend a format stores in the operand rather than in the relocation entry (COFF, Mach-O) is not
printed beside the name. A near branch has no rip-like form to fall back on either: iced prints its
target as the address the displacement works out to, so a relocated one reads as an address it does
not go to. `Instruction::relocation_span` is the index of the span the name landed in, recorded by
an override of `write_symbol`. That is what lets `InstructionRow` render
the run before it as one `paragraph()`, the span as a `RelocationLabel`, and the run after it as a
second `paragraph()`. `branch_span` is its **twin** and is recorded by an override of
`write_number`, which is how a branch target reaches the output: it is the span an instruction's
*own* displacement was printed into. The two are exclusive by construction, since a branch covered
by a relocation is not one that named an address, so a row has at most one link and the same
three-child split serves both.

**A linked image's calls resolve by address**, since the linker consumed the relocations that named
their targets and left the displacement as the answer. Where no relocation covers an instruction and
it is a direct near `call`, the backend asks `Code::symbol_at` for the text symbol that **starts
exactly** at the address the encoding names, and hands it out through the same
`relocation`/`relocation_span` pair a relocated call uses: the resolver substitutes the name for the
operand, `write_symbol` records the span, and the UI's `RelocationLabel` draws it with no change of
its own. Three limits, each deliberate. *Exact start only*: a call into the middle of a function
stays the number it is, and a target no symbol starts at (a PLT stub, a stripped static) stays plain
text. *Same section*: the index is by placed address (`Section::bias` added), which makes a
relocatable object's all-at-0 code sections distinct places, but a displacement past a section's end
still lands in the placed space on some other section's function, so the hit has to be in the
instruction's own section; `tests/linked_call.rs` pins a two-section object whose call would
otherwise name the other's. *Calls only*: an unconditional `jmp` out of the symbol is a tail call
and could be named the same way, but its displacement is `branch_span`'s and the two spans are
exclusive, so making it a link to a function is the item of its own that `notes/Goals.md` says it
is. The relocation still wins where there is one: a relocated call whose target is a section symbol
keeps `None`, since its displacement is a placeholder whatever address it happens to spell. The
index is `Object::by_address`, a `Vec<(u64, Arc<SymbolData>)>` sorted by placed address with each
address once (two names for one address keep the first by name, the order `symbols_sorted` has, so
the answer is stable), behind a `OnceLock` like the debug info and derived from `symbols_sorted`, so
an `Object` built by hand writes `Default::default()` and cannot disagree with it. It is lazy rather
than built at parse because an archive's members are parsed all at once and read one at a time.
Building it for the sample's 115,577 symbols measured 67 ms in an unoptimized test build, once per
object on the first disassembly; a lookup after that is a binary search, 5 µs. A sort per click
would have been the same 67 ms on every symbol opened, which is why it is not rebuilt per
disassembly.

**Where an instruction goes is kept beside what it is named**: `Instruction::target` and
`target_span`. `target` is the address a direct near `call` or branch names in its own encoding
where no relocation covers its bytes and no symbol has named it: a call into the middle of a
function, a call to a function a stripped image has no symbol for, a jump out of the symbol. It is
in the section's own address space, as `address` is, and `Some` exactly when `target_span` is, the
span the number was printed into, which is `write_number`'s mark. `branch_span` is now *derived*
from it, the same span for exactly the rows whose `branch` is set, so the two twins became three
spans that are still exclusive: a call the resolver named has `relocation_span` and no `target` (its
address is the symbol's own), a branch has `branch_span` and the same index in `target_span`, an
unnamed call has `target_span` alone, and a row still has at most one link. Nothing is judged here
either; what the UI does with it is a **Ctrl** door into the object's code at that address
(`agents/Panes.md`). What it does not cover is a relocation against a section symbol with an addend
(`Relocated { target: None }`, a call into `.text+0x40` in a relocatable object), whose target would
be the section's address plus the addend, adjusted by the relocation's kind. The parse keeps no
section symbols and the relocation's kind is not read, so that operand stays the placeholder it is
and the item stays in `notes/Goals.md`.

**Branch edges** (`Assembly::edges`) are the branches staying inside one symbol, for the arrow
gutter. Both ends are **indices into `instructions`**, not addresses, because that is what a row can
be asked about and it makes the answer independent of where the symbol sits. A backend leaves the
*address* each branch names on its row, `Instruction::branch` (a forward branch names one no
instruction has been decoded at yet), and `Assembly::decoded` resolves them, which is what keeps the
four rules below one decision rather than one per backend. The address stays on the row afterwards,
judged by nothing: it is the address-keyed answer a listing of a whole section wants, where whether
the target has a row is only known once the stretch it is in has been decoded. `from`/`to` are in
*execution* order (a backward branch has `from > to`); `first()`/`last()`/`is_backward()` sit on
top. A **call is not an edge** even when it lands inside the symbol, because control comes straight
back. Four things are dropped rather than drawn, each of which would be a line to a place it does
not point at: a branch out of the symbol, one landing mid-instruction, one whose displacement is a
relocation placeholder (tested on the *raw* relocation lookup, since a branch relocated against a
section carries no text symbol while its displacement is just as meaningless), and `jmp $`. Those
four keep their `branch_span` all the same, the number being where the number is, so **the span and
the edge are separate answers** and a caller that wants to *follow* a branch needs both.
`Assembly::edge_from` is the pairing, and it is a binary search rather than a scan: an instruction
names at most one target and a backend decodes from the front, so `from` ascends strictly across
`edges`.

**The section listing** (`listing.rs`) is the crate's half of the unified section view: a whole
section as one address-keyed listing, beside the symbol view and not instead of it. Nothing
index-keyed changed for it. `Listing::new` is the **skeleton**, and it decodes nothing: one
`Stretch` per distinct symbol address inside the section's bytes, its range running to the next
address or the section's end, plus a leading stretch with no label when the first symbol is not at
the start (or there is no symbol at all). It is built from `Object::symbols` by pointer identity on
the section (two sections of a relocatable object share address 0) and ordered by
`(address, SymbolIndex)`, so two names at one address are one stretch with two labels in the file's
order rather than the hash seed's. A symbol placed outside the section's bytes is left out. That is
free, a scan of the object's symbols and a sort of the section's own, and it is what gives a view a
stable structure to scroll while instructions arrive. **A stretch is decoded on demand**
(`Listing::decode`), and that is when its symbol's extent is asked for: the code is literally
`SymbolData::assembly`'s answer, so the section and the symbol view cannot disagree, and the bytes
from `SymbolData::extent` to the next label are the stretch's `Gap`. The gaps are deliberately not
known up front: the extent is a DWARF walk that cost 2.0 s over the 331 MB binary before its
`.eh_frame` was read (the reverse index's measurement), and the skeleton has to cost nothing. **A
gap is never decoded.** Bytes no symbol claims are not known to be code (alignment padding, a jump
table MSVC put after the function, a stripped local), and decoding them would print the confident
page of nonsense `undecodable` exists to prevent. So a gap is *said*: its range and a `GapKind`, and
whoever draws it slices `Section::data`. The bytes are not copied, since the tail of a stripped PE's
export can be megabytes. There are two kinds only. `Bytes` is the ordinary one. `Cut` is the rest of
a stretch whose derived extent hit `MAX_DERIVED_SIZE`, said apart because it is very likely the
function going on past the cap rather than anything between two functions, and it starts wherever
the cap fell rather than at an instruction. Where the file has an unwind table only a symbol no
entry covers can get one, the rest having their ends stated. An architecture no backend decodes
gives the symbol stretch the `undecodable` `Assembly` the symbol view gets, and its gaps are `Bytes`
like any other. The section's end is saturating, where `estimate_size`'s is `None`: a listing has to
end somewhere. `tests/listing.rs` holds one invariant test over every fixture shape, both committed
gcc objects and gcc's stripped `.so` (the stretches partition the section exactly, every symbol
inside it is at one label, each stretch's code is the symbol's own row for row, and the gap starts
exactly where the extent stops), and a test per decision.

**All of an object's code is one listing** (`CodeListing`), because the function view's unit, a
symbol, is not the section's: rustc and `-ffunction-sections` put every function in a section of its
own, all at address 0, and a per-section listing of those is a listing of one function each. A
`CodeListing` is every `Section::code` section with bytes, each with its own `Listing`, **placed**
at `Section::bias` past its address and ordered by where it landed. That layout is the parse's
(`section_biases`), the same one the line info is read at, so a placed address means one thing to
both. A linked image's sections have real, distinct addresses and no bias, so there a placed address
*is* the address; a relocatable object's code sections each get a place of their own. The air the
layout leaves between two sections is nobody's bytes (`at` answers `None` there), and a section
boundary is a label for the view to draw, not a gap. Two things are left out rather than listed: a
code section with no bytes (gcc leaves an empty `.text` beside the split ones; it has a place in the
layout, so the biases the tests pin skip a grain for it) and a section whose placed range overlaps
the one before it, which a header can claim and nothing can draw. It is built in one pass over the
object's symbols, bucketed by section, rather than one scan per section, since a large crate's CGU
has thousands of both. Branches compose with it for free: in a linked image a branch's address is
unique across sections, so `Instruction::branch` is already the placed key; in a relocatable object
a jump to another function is a relocation, `branch` is `None`, and the relocation target is a
symbol, which its section's listing places. The unit stays the object, since an archive's members
share no addresses, so "all the code" of an archive is a list of objects' code listings, and that is
the sidebar's question.

**"Never panic on any file input" is tested two ways, and they are different jobs.**
`tests/mutations.rs` is the **search**. It takes every fixture the suite builds (both committed gcc
objects and gcc's stripped `.so`, the synthesized DWARF one, the ELF `.so`, the PE DLL and the same
DLL naming a `.pdb` that is nowhere) and the six that are files on disk, the linker's three DLLs
each parsed **beside its PDB** and those PDBs themselves. It truncates each at every length; writes
poison values (`0`, `u32::MAX`, `u64::MAX`, the file's own length…) into every numeric field of
every header, section header, symbol and relocation, and for the PDB of the MSF superblock and
stream directory, for the DLL of its debug directory and CodeView record, its exception directory
and every `RUNTIME_FUNCTION`'s three words, and for an ELF of every `.eh_frame` record's length, CIE
pointer, address and range; and splats pseudo-random runs over it, running the whole pipeline over
each result. A `.pdb` being a second file found beside its binary, a mutated PDB is written beside
its pristine DLL before that is parsed, under a directory per test in the target directory. A sanity
check first asks each intact pair for line info, so the sweep is known to reach the backend and not
a search that comes back empty; for the pairs whose image declares nothing, having a symbol at all
is the procedure walk having run, and the third pair's fourth function is the publics walk having
run. It is sampled by an even stride and seeded from a constant (never `rand`, never the clock), so
which cases run is fixed and it stays in single-digit seconds (about 5 s, of which the reverse index
costs a tenth and every section's listing, its first four stretches decoded, four tenths).
`tests/robustness.rs` is the **regression suite**: one named, minimal fixture per defect that was
actually found, because a sweep that goes green tells you nothing about which bug it was that
stopped happening. `common::parse_and_walk_at` is the one definition of "ask a parsed object
everything", shared by both: every symbol's extent, listing and line info, every section's line
info, the reverse index, and every section's `Listing` with its first `MAX_LISTING_STRETCHES` (4)
stretches decoded, since a decode is the symbol's disassembly over again and a section of any kind
has a listing. The PDB sweep found nothing the seam's guard did not already catch, and it cannot
find the one defect that is not a panic, the declared stream length `pdb2` would allocate before
reading, which `BoundedFile` answers by construction (`notes/upstream/pdb2.md`). Nor can either
sweep write a zstd frame, so what `object` would inflate one to, whatever its header declared, is
pinned by a hand-built fixture in `robustness.rs` (`notes/upstream/object.md`). The rule that goes
with them is the user rule in `AGENTS.md`: a minimal test case every time something is found wrong,
and **checked arithmetic in preference to a wider `catch_unwind`**; the guard is for a dependency's
bug, never for ours. Note also what *cannot* be caught: a stack overflow aborts, so anything
recursing over file-controlled input (the demanglers, above) has to be bounded before the call
rather than wrapped. `demangle/tests.rs` pins both halves of that for the pool: that a split batch
answers in its own order, and that a 1000-level name in one lands on a pool thread and not on the
submitter's stack, which is a test that does not fail but *aborts* when it is wrong.

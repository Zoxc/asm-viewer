# pdb2 0.10.2

The PDB reader behind `crates/analysis/src/line/pdb.rs`. A maintained fork of `pdb` 0.8
with the same API; see the `Cargo.toml` comment for why it and not `pdb` or `pdb-addr2line`.

**Unchecked arithmetic on numbers the file states**, the same class as `addr2line`'s
(`agents/Analysis.md`). Found by reading the source before the sweep reached them, and all
four reachable from a `.pdb` a user merely opened:

- `ModuleInfo::lines_data` slices its stream at `start..start + size` with no bounds check
  (`modi/mod.rs`), for a `size` the DBI's module record declares.
- A C13 line block's payload size is `block_size - size_of::<Header>()`, an underflow when a
  block declares fewer bytes than its own header (`modi/c13.rs`), and the block iterator
  `split_at`s the declared sizes unchecked.
- `PdbInternalSectionOffset + u32` is a plain `+=` (`common.rs`), reached from a line entry's
  offset plus its length and from a block's `code_size`.
- `PDBInformation::stream_names` indexes the names buffer at a declared offset unchecked
  (`pdbi.rs`), which `PDB::string_table` reaches through.

**Debug assertions on what the file states.** Two more panics, in a debug build only:

- `LineInfo::set_end` has `debug_assert!(self.offset <= end_offset)` (`modi/mod.rs:200`). A
  section offset compares only within one section, so it fails whenever a line's successor
  is in another section or at a lower offset. The line iterator calls it for every line.
- A C13 line block's parse has `debug_assert!(remainder.is_empty())` (`modi/c13.rs:568`),
  for a block that declares more data than its lines and columns take.
- Parsing a symbol record (`symbol/mod.rs`): `S_INLINEES` asserts its count equals the
  entries it holds (`:2612`), `S_CALLEES`/`S_CALLERS` that it has no more counts than functions
  (`:2581`), the `S_DEFRANGE*` family subtracts the header size from a record shorter than it
  (`:2072` and the five like it, an overflow), and a numeric leaf with an unknown prefix, as in
  an `S_CONSTANT`, is `unreachable!()` in a debug build (`common.rs:899`). The crate parsed
  every record it walked to find the procedures and the publics; it now parses only those
  (below), and neither has any of these.

No real linker output reaches the first two either, so each is left to the seam's net.

Told apart from our own mistakes by the panic location; none of them is something the crate
can validate without parsing the stream itself first. **What it cost**: nothing new — the
seam's one `without_panicking` already wraps the build and every question, whichever backend
answers, and `DebugInfo::load` is under it too because the string table is read at load.

**An OMAP translates in 32 bits, unchecked.** `OMAPRecord::translate` (`omap.rs:58`) is
`(address - source) + target`, and both numbers are the file's, so a target near the top of the
space overflows. It is reached from every `section:offset` a PDB with an OMAP is read at:
`PdbInternalSectionOffset::to_rva` for one address, and `AddressMap::rva_ranges` for a range.
**What it cost**: nothing of our own. No linker writes such a target, so the seam's net is left
to catch it, and the names or the PDB the walk was reading go with it. A release build does not
panic: the sum wraps, and the address that comes out is dropped only where it falls in no section.

**A symbol record's stated count is allocated before it is checked.** `FunctionListSymbol`, the
parse of an `S_CALLEES` or an `S_CALLERS` (`symbol/mod.rs:2573`), is
`vec![buf.parse()?; count as usize]`: the count is the record's first field and nothing weighs it
against the bytes the record holds, so a record of a few bytes stating `u32::MAX` asks for 16 GiB,
and `resize`s the invocations to match. Never a panic, so no guard catches it: an abort, or the
machine's memory, reached from every module's symbol walk at parse and again when the module is
decoded. **What it cost**: the walks read a record's kind (`Symbol::raw_kind`, two bytes, no
parse) and hand `pdb2` only the procedure and public kinds to parse (`PROCEDURES` and `PUBLICS`,
`line/pdb.rs`; `pdb2` keeps its own constants private, so the numbers are spelled there). That
also keeps every debug assertion above out of reach, and any bug yet to be found in a kind the
crate does not use. Pinned by `pdb.rs`' `a_symbol_record_the_walks_do_not_use_is_not_parsed`,
on the committed PDB with one record rewritten as an `S_CALLEES` stating `u32::MAX` and again as a
miscounted `S_INLINEES`. The test binary's allocator refuses any one request past 1 GiB, so a
regression is an abort that fails the run and not a machine out of memory.

**A module's line walk ends at the first block that will not read.** `LineIterator` walks every
lines subsection of a module as one (`modi/c13.rs`), and a block whose stated size runs past its
subsection is an error that ends it: the rows of every subsection after it were lost. The
subsections were framed one by one when the line program was read, so they could be walked one by
one, but the iterator neither goes on to the next nor says which it was in. **What it cost**:
the rest of that module's rows, and the module is counted. No linker writes such a block, so
nothing reads the module again. Pinned by `pdb.rs`' `a_line_block_that_does_not_read_is_counted`.

**The symbol walk does not say where a record too short to hold a kind ends.** `SymbolIter::next`
(`symbol/mod.rs`) returns `SymbolTooShort` for a record whose stated length is 0 or 1, after
reading the length and before stepping past the record. Going on from there is right for a
length of 0 and one byte short for a length of 1, and the error does not say which. **What it
cost**: nothing yet. `Pdb::procedures_in` stops there and keeps what it read, the module's
publics still naming its functions; stepping past would take reading the module's stream a
second time to frame the records ourselves.

**A declared stream length is allocated before a byte is read.** The blanket `Source` for a
`Read + Seek` sizes its `Vec` from the stream directory's page list, so a directory that lies
asks for gigabytes — never a panic, so no guard catches it. **What it cost**: `BoundedFile`
in `line/pdb.rs`, a `Source` of our own (~40 lines) that refuses any slice past the file's end
and any total past the file's length before allocating, the answer `section_data` already
gives a lying compressed size.

Not reported: the fork is one person's, the arithmetic is pervasive, and the guard was
already there for `addr2line`.

## Wanted

**A line walk per subsection**, or one that goes on to the next subsection after an error.

**Where a record too short to hold a kind ends, in `SymbolTooShort`**, so a walk could step past it.

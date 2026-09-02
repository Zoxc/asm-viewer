# pdb2 0.10.2

The PDB reader behind `crates/analysis/src/line/pdb.rs`. A maintained fork of `pdb` 0.8
with the same API; see the `Cargo.toml` comment for why it and not `pdb` or `pdb-addr2line`.

**Unchecked arithmetic on numbers the file states**, the same class as `addr2line` 0.21's
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

Told apart from our own mistakes by the panic location; none of them is something the crate
can validate without parsing the stream itself first. **What it cost**: nothing new — the
seam's one `without_panicking` already wraps the build and every question, whichever backend
answers, and `DebugInfo::load` is under it too because the string table is read at load.

**A declared stream length is allocated before a byte is read.** The blanket `Source` for a
`Read + Seek` sizes its `Vec` from the stream directory's page list, so a directory that lies
asks for gigabytes — never a panic, so no guard catches it. **What it cost**: `BoundedFile`
in `line/pdb.rs`, a `Source` of our own (~40 lines) that refuses any slice past the file's end
and any total past the file's length before allocating, the answer `section_data` already
gives a lying compressed size.

Not reported: the fork is one person's, the arithmetic is pervasive, and the guard was
already there for `addr2line`.

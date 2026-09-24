# object 0.40.0

The object-file reader the whole crate is built on.

**A zstd section's declared size is a hint, not a bound.** `CompressedData::decompress`
(`read/mod.rs`) reserves the size the compression header declares, decodes the whole of the
frame into that vector — `ruzstd`'s `decode_all_to_vec` grows it — and only then compares the
length and rejects a frame that disagreed. The rejection is after the allocation, so an ELF
with `SHF_COMPRESSED`, `ELFCOMPRESS_ZSTD`, `ch_size = 1` and a 64 KiB frame of RLE blocks
still allocates about 2 GiB before being turned down. The zlib path of the same function does
bound it: `flate2`'s `decompress_vec` never grows the vector it is given. Never a panic, so no
guard catches it; an allocation failure is an abort.

**What it cost**: `zstd_data` in `crates/analysis/src/parse.rs`, ten lines that inflate the
frame with `ruzstd` directly and read it through a `take` one byte past the declared size, so
a frame producing any other number of bytes is dropped like a declared size the ratio bound
rejects. `ruzstd` is named in `Cargo.toml` for it and compiles nothing new, `object` already
building that version. Pinned by `robustness.rs`'
`a_zstd_frame_producing_more_than_its_header_declares_is_dropped`, which the mutation sweep
cannot reach: it writes poison values into headers and does not synthesize a zstd frame.

Not reported, and unchanged from 0.32 through 0.40. One difference to keep in mind: 0.40
decodes *every* frame in the section, where `zstd_data` reads one, `ruzstd`'s
`StreamingDecoder` still ending at the first frame's end. A section written as several frames
therefore decodes short here, and short is dropped.

**A Mach-O `LC_MAIN` entry point is a file offset, not an address.** `MachOFile::entry`
(`read/macho/file.rs`) takes the first `LC_MAIN`, or `LC_UNIXTHREAD` whose PC it can read,
that it finds. For `LC_UNIXTHREAD` it answers that PC, an address. For `LC_MAIN`, which every
executable linked for macOS 10.8 or later has, it answers `entryoff` unchanged, and that is
the offset of `main` in the file. ELF and PE answer addresses. The trait's doc says only "the
virtual address of the entry point", so callers take it as one.

**What it cost**: with `__TEXT` at `0x100000000` the offset is in no section, so every
Mach-O executable lost its `<entry point>`. With `__TEXT` low, as in an i386 image, the offset
could land inside another function and put `<entry point>` in its middle, cutting its extent
short. The fix is `macho_entry` in `crates/analysis/src/parse.rs`, which does not call
`entry()` for a Mach-O at all. It walks the load commands in the same order. For `LC_MAIN` it
finds the segment whose file range holds the offset and adds that segment's `vmaddr`. For
`LC_UNIXTHREAD` it reads the PC itself (`thread_pc`, the same CPU table and offsets), and goes
on to the next command when it cannot, as `object` does; calling `entry()` there would have
handed back a later `LC_MAIN`'s raw offset. Pinned by `declared_code.rs`' three
`a_macho_…` tests.

Not reported.

**Some formats' function addresses are handed over as the file states them, not as code
addresses.** On 32-bit ARM ELF, `ElfSymbol::address` and `ElfFile::entry` answer `st_value` and
`e_entry` with bit 0 still set on a Thumb function, and so do the ELF exports; on MIPS ELF, the
same on a MIPS16 or microMIPS one. On an ARMNT PE and an armv7 Mach-O, the exports and the entry
point (`AddressOfEntryPoint`; `LC_MAIN`, `LC_UNIXTHREAD`) have bit 0 set on Thumb code. On PPC64
ELFv1, an `STT_FUNC`'s value and `e_entry` are the address of a function descriptor in `.opd`. On
XCOFF, `entry()` answers `o_entry`, which is also a descriptor's address. The trait's docs say only
"address" and "the virtual address of the entry point", and nothing marks these as different.

**What it cost**: a Thumb, MIPS16 or microMIPS function and its entry point sat one byte into the
code, and each exported one was listed twice, once a byte in. A PPC64 ELFv1 function's symbol and
every ELFv1 or XCOFF entry point landed in no code section and were dropped without a word. The fix
is `CodeAddresses` in `crates/analysis/src/parse.rs`, which clears the mode bit (`ModeBit`) and
reads a descriptor's first word, through its relocation in a relocatable object. Pinned by
`tests/code_addresses.rs`.

Not reported. `object` 0.40 has no helper for any of it. The one piece it offers is
`FileFlags::ppc64_abi`, which reads the ABI version.

**A Mach-O export trie's walk never ends past a bad node.** `ExportsTrieIterator` (and so
`MachOExportIterator`) hands back an error for a node it cannot read and does not move past it:
where an edge's name has no terminating NUL, `read_string` fails without advancing and without
counting the child off, so every later `next` answers the same error. A caller that skips errors,
as a per-entry iterator invites, loops forever.

**What it cost**: a hang on file input. `declared_code` skipped a bad export and read on, which is
right for a PE, whose iterator moves on first; an armv7 Mach-O fixture with a splatted trie hung
the mutations sweep. On a Mach-O the first error now ends the export walk. Pinned by
`declared_code.rs`' `a_macho_export_trie_cut_inside_an_edge_ends_the_export_walk`.

Not reported.

## Wanted

**An XCOFF image writer.** `write::Object` writes an XCOFF relocatable object and nothing
else: no auxiliary header, so no `o_entry`, and it picks every section's address itself,
packing them from 0. There is no lower-level XCOFF writer as there is for ELF and PE. What
the tests do instead: `xcoff_image` (`crates/analysis/tests/common/mod.rs`) writes the file
header, the auxiliary header and the section headers byte by byte.

**A PE export table.** `write::pe::Writer` lays out the headers, the sections and the data
directories, and leaves every directory's contents to the caller. What the tests do
instead: `armnt_dll` and `pe_image` (`crates/analysis/tests/common/mod.rs`) write the export
directory, its three arrays and its names by hand.

**A Mach-O executable writer.** `write::Object` writes `MH_OBJECT` only, with no segments,
no `LC_MAIN` and no export trie, and there is no lower-level Mach-O writer, only
`write::macho::Encoder`. What the tests do instead: `macho_executable` and
`macho_arm_executable` (`crates/analysis/tests/common/mod.rs`) put the load commands
together through the encoder, lay out the file by hand, and write the export trie byte by
byte.

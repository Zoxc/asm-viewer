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

# object 0.32.2

The object-file reader the whole crate is built on; see `crates/analysis/Cargo.toml` for why
0.32 and not a newer one.

**A zstd section's declared size is a hint, not a bound.** `CompressedData::decompress`
(`read/mod.rs`) reserves the size the compression header declares and then hands the frame to
`ruzstd`'s `read_to_end`, which grows the vector to whatever the frame produces. The zlib path
of the same function does bound it — `flate2`'s `decompress_vec` never grows the vector it is
given — so a check on the declared size, which is what `section_data` had, holds for one
format and not the other. An ELF with `SHF_COMPRESSED`, `ELFCOMPRESS_ZSTD`, `ch_size = 1` and
a 64 KiB frame of RLE blocks decompresses to about 2 GiB. Never a panic, so no guard catches
it; an allocation failure is an abort.

**What it cost**: `zstd_data` in `crates/analysis/src/lib.rs`, ten lines that inflate the
frame with `ruzstd` directly and read it through a `take` one byte past the declared size, so
a frame producing any other number of bytes is dropped like a declared size the ratio bound
rejects. `ruzstd` is named in `Cargo.toml` for it and compiles nothing new, `object` already
building that version. Pinned by `robustness.rs`'
`a_zstd_frame_producing_more_than_its_header_declares_is_dropped`, which the mutation sweep
cannot reach: it writes poison values into headers and does not synthesize a zstd frame.

Not reported, and not fixed by moving: 0.36 through 0.39 compare the length afterwards and
reject a frame that disagreed, but still read the whole of it into the vector first, which is
the allocation. Like 0.32, they also read one frame per section, which is what `zstd_data`
does; a section written as several frames would decode short, and short is dropped.

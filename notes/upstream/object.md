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

**What it cost**: `zstd_data` in `crates/analysis/src/lib.rs`, ten lines that inflate the
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

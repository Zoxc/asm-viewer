# addr2line 0.27.1

The DWARF line-table reader behind `crates/analysis/src/line/dwarf.rs`.

**A row's length is `next.address - row.address`, unchecked.** `LineLocationRangeIter::next`
(`src/line.rs`) takes the length of a row from the address of the row after it, and nothing
stops a line program from moving its address backwards — `DW_LNS_advance_pc` takes an
unsigned operand, but `DW_LNE_set_address` sets whatever it is given. So a file the reader
merely opened is a subtract-with-overflow panic. Not something this crate can check without
reading the line program a second time, so it is caught instead, by `without_panicking`
(`crates/analysis/src/line.rs`), and pinned by `robustness.rs`'
`a_line_program_that_runs_backwards_does_not_panic`.

**What it cost**: `clipped` in `line/dwarf.rs`. Overflow checks are off in a release build, so
there the panic is a wrap: the backwards row's length becomes huge and the rows after it come
back below the query. Every row is therefore clipped to the query, and rejected where nothing
is left, before the section's bias comes off it — subtracting first made a row reaching the end
of the address space, which the pane then showed as one confident wrong source line across the
function. Pinned by `line/dwarf/tests.rs`, a unit test and not a fixture because no fixture can
produce that row in a build with the checks on.

**`Context::find_units` asks its range index about `probe + 1`, unchecked** (`src/unit.rs`),
so the very last address in the space panics. Declined rather than caught: `Dwarf::extent`
returns nothing for `u64::MAX` outright.

**A unit's extent is the debug info's word and no more.** `for_each_range` (`src/lib.rs`) adds
`low_pc` and a `DW_AT_high_pc` length with `wrapping_add` and drops the range when the sum
comes out below the start, so nothing panics; but a subprogram's own `DW_AT_high_pc` is handed
back as it was written. A function at the top of the address space claiming a length that runs
off the end is therefore a range that does not exist, which `Symbol::extent`
(`crates/analysis/src/lib.rs`) declines. Pinned by `robustness.rs`'
`a_function_at_the_end_of_the_address_space_does_not_panic`.

Nothing here is reported: the two unchecked sums are in the class the crate has been fixing
release by release.

## Wanted

**A file entry handed back with the name it renders.** `render_file` (`src/line.rs`) joins the
compilation directory, the directory and the file name into a `String` and keeps nothing of
the `gimli::FileEntry` behind it, so the MD5 a DWARF 5 producer records per file cannot be read
off an answer. The app carries a `SourceHash` for exactly this — telling the file on disk apart
from the one the binary was built from — and fills it in from a PDB and not from DWARF
(`crates/analysis/src/line/dwarf.rs`, where the file is pushed with `None`). Substituting for
it means rendering the name from `gimli`'s own `FileEntry` the way `addr2line` does, which is
`notes/Goals.md`'s item, not this crate's bug.

# addr2line 0.21.0

The DWARF line-table reader behind `crates/analysis/src/line/dwarf.rs`; see
`crates/analysis/Cargo.toml` for why 0.21 and not a newer one.

**A query starting between two of a unit's sequences is answered with nothing.**
`LocationRangeUnitIter::new` (`src/lib.rs`) finds the sequence holding `probe_low` with a
three-way binary search, then reads a miss two ways: `Err(0)`, the probe below every sequence,
starts at the first, and every other miss becomes `sequences.len()`, which ends the walk before
it begins. A probe in the gap after sequence *i* and before *i+1* is `Err(i+1)`, so the unit
answers nothing even where a later sequence of it overlaps the range asked about. Still there in
0.22; 0.24 maps every miss to the sequence after it (`src/line.rs:187`).

It needs a unit whose declared range spans several sequences and a symbol beginning in a gap
between them. One sequence per section per unit — what gcc, clang and rustc emit — lines the
ranges up with the sequences and never makes that shape. When it does happen the answer is the
worst kind: `Object::line_info` says "no line info" for the whole symbol, while the reverse
index, which walks each unit from 0 and so never misses, names that symbol for the very lines
its pane will not show.

**What it cost**: nothing, deliberately. There is no workaround worth having — `Context` says
nowhere where a unit's sequences begin, so a query cannot be restarted from the right place
without reading the line program a second time — so the behaviour is pinned instead, by
`line_info.rs`'s `a_query_starting_between_two_sequences_of_a_unit_is_answered_with_nothing`.
It fails the day the crate moves, and this note goes with it. Not reported: later releases
have it right.

**Two panics on numbers a debug section states**, both under the seam's guard
(`crates/analysis/src/line.rs`) rather than validated, since neither can be checked without
reading the debug info twice. A row's length is `next.address - row.address`, so a line program
that moves its address backwards is a subtract-with-overflow panic on a file the user merely
opened; still so in 0.24.2. And a unit's range is `low_pc + high_pc`, which overflows for a unit
declaring a length that runs off the end of the address space — that one while the context is
being built, which is why the guard is around the build too. A third is not left to the guard:
`Context::find_units` asks its range index about `probe + 1` unchecked, so `Dwarf::extent`
declines `u64::MAX` outright rather than catching the panic afterwards.

**What the first of those cost**: `clipped` in `line/dwarf.rs`. Overflow checks are off in a
release build, so there the panic is a wrap: the backwards row's length becomes huge and the
rows after it come back below the query. Every row is therefore clipped to the query, and
rejected where nothing is left, before the section's bias comes off it — subtracting first made
a row reaching the end of the address space, which the pane then showed as one confident wrong
source line across the function. Pinned by `line/dwarf/tests.rs`, a unit test and not a fixture
because no fixture can produce that row in a build with the checks on.

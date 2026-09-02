# tree-sitter-rust 0.24

**The grammar does not know `const impl`, `const trait` or `[const]` bounds, and its error
recovery does not contain the damage.** One such item and the whole file parses as a single
`ERROR` node with a handful of `function_item`s recovered inside it. Measured on this
machine's nightly `library/core` (2026-09): 98 of 289 files fail to parse and about 1 200
function definitions go missing; blanking those keywords out byte-for-byte before the parse
still leaves 48 files and about 620 definitions missing, the remaining causes being other
syntax the grammar is behind on (`#[rustc_intrinsic]` bodiless fns, and more). Across
`core`, `std` and `alloc` plus this repo, 152 files of 1 104 fail.

Not reported. It is not a bug so much as a grammar that cannot keep up with a nightly
compiler, and it will recur with every new keyword.

**What it cost.** The instance picker (`src/functions.rs`) needs "the function around this
line", and a grammar that loses whole files cannot be the source of it for the library the
reader is most likely to be reading. Rust functions are found by a scanner of our own
(`src/functions/rust.rs`): the `fn` keyword, the name after it, and the block after the
signature, decided by tokens alone -- comments, strings, character literals, brackets.
Checked against the grammar on the 952 files of `core`/`std`/`alloc`/this repo that *do*
parse cleanly: identical on 901, and every difference on the other 51 is a `fn` inside a
`macro_rules!` body, which the grammar treats as token soup and the compiler as code whose
line info points into it -- listed on purpose. The highlighter (`ui/highlight.rs`) still
uses the grammar: colouring is per token and survives an `ERROR` node; a span does not.

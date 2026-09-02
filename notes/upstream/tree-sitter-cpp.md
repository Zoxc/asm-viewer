# tree-sitter-cpp 0.23

**A pointer-to-member declarator is a parse error.** `int (S::*member())(int)` leaves an
`ERROR` node holding the `S::` beside a `pointer_declarator` inside the
`parenthesized_declarator`, the grammar not knowing `S::*`. Seen when pinning the C++
cases of `src/functions.rs`.

Not reported. **What it cost:** `innermost_declarator` prefers a child whose kind ends in
`_declarator` and never takes an `ERROR` child, so the method is still named through the
declarator that did parse (`cpp_methods_in_a_class_body_and_out_of_it_are_functions`).

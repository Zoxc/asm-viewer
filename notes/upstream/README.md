# Upstream

Bugs and gaps found in dependencies while building this app, one file per crate: what was
hit, how it was told apart from our own mistake, what it cost here (the workaround, and
where it lives), and whether it has been reported. A note is added when the workaround is
written and rewritten when the dependency moves, so that a workaround whose reason has gone
can be taken out. The reasoning that belongs to *our* design is in `agents/`; this folder
is only for what is not ours to fix.

- [`freya.md`](freya.md) -- the UI framework, 0.4.3.
- [`tree-sitter-rust.md`](tree-sitter-rust.md) -- the Rust grammar, 0.24.
- [`tree-sitter-cpp.md`](tree-sitter-cpp.md) -- the C++ grammar, 0.23.

# rust-analyzer

Not a crate but the program a Rust project is read with (`src/lsp.rs`), so what it does and
does not answer is the same kind of fact as a crate's bugs.

**A question asked before the project is loaded is answered `null`, which is also the answer
for a name the server cannot place.** Nothing in the answer tells the two apart, so a click in
the first seconds after the server starts opens nothing and reports nothing, and the same
click a moment later works.

Seen against 2026-09's release, over a two-file cargo package: after `initialize` and
`initialized` and a 12-second wait, three of four `textDocument/definition` questions came
back `null`, one of them about a plain function call in the file itself; with a 25-second wait
and a retry, all of them answered, each with one location. So the `null` was "not yet" and not
"not there".

Told apart from our own mistake by asking the server directly, outside the app, with the
capabilities the app declares.

Not reported. The protocol has codes for this -- `ContentModified` and `ServerCancelled`,
which the app already reads as "ask again" -- but a server is not obliged to use them, and
answering nothing while there is nothing to answer with is defensible.

**What it cost.** Nothing yet: the app takes the empty answer at its word and opens nothing,
which is what it does for a name that is genuinely not there. Holding such a question and
asking it again once the server is ready is a goal (`notes/Goals.md`); until then this is the
explanation for a door that looks dead just after the server is started.

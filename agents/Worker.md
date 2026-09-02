# The analysis worker

The one worker thread the panes ask: what a question is, how requests supersede one another,
how an answer is judged when it lands, and what is drawn meanwhile.

**Nothing is analysed on the UI thread.** `SymbolData::assembly` decodes and formats the whole
symbol, and `SymbolData::line_info` builds the object's entire DWARF context on the first query
against it — 1.4 s together for the first symbol clicked in the 331 MB binary (debug build; 0.6 s in
release), and both of them used to run in `render`. `use_analysis` moves them together, because
they are asked for by the same click and the pane needs both: **one worker thread** for the app's
lifetime, fed an `async_channel` of `Question`s, answering with a `Studied` (the `Assembly`, its
`Lanes`, and the `SymbolLines`). One worker and not a thread per request or a pool, because
requests *supersede*: a reader going down the symbol list issues one per row and wants the last
one's answer, so the queue is drained to its newest entry each time round and the rest are dropped
*before* being started. A thread each would put a whole run of clicks through the most expensive
call in the crate at once for one useful answer, and `DwarfCache` is a `OnceLock`, so the losers
would block on the winner rather than race usefully. (The parallelism `notes/Goals.md` asks for is
about parsing many objects at once and is a different job.)

**A third kind of question is a window of an object's code** (`src/ui/reading.rs`), for the
section view: `Question::Code(CodeAsk)`, an object, its skeleton once the view has one, and the
stretches wanted by flat index, **nearest the reader first**. The skeleton (`CodeListing`,
free) is built on the worker with the first ask and answered with it; a stretch is decoded
through the crate's own `CodeListing::decode` and then `Studied::with_assembly`, the rest of what
`Studied::new` does over a listing already in hand, so the section view and the symbol's own
tab decode a function once and identically. The worker decodes **at most `CHUNK` (8) of the ask**
and answers, because the queue below is drained to its newest question only *between* jobs: a
window decoded whole would hold a symbol click behind every function on a screen and three
screens of buffer, where a chunk holds it behind a few, and the view asks for the rest once the
chunk has landed. The answers land in `Reading` (`Sections` at the root) and never in `Analyzed`,
which is one symbol's shape and read by everything that draws a symbol. **A decoded stretch is a
pure function of the object and the stretch and is never stale** -- unlike a listing, which is
stale the moment the ask moves on -- so an answer is taken whenever it is about the object and
the skeleton on screen, whichever window asked for it, and only `pending` is judged against the
ask; what a scroll superseded is exactly what the next window asks for again. Two things bound
it: a stretch farther than `KEEP` (512) from the last window is let go as the answer lands, and
the whole reading is dropped when the active document stops being that object's code or the
object closes under it (`use_reading_of`, an effect reading `Active` and `Objects` -- an effect
and not `close_binary`, the skeleton holding every section's bytes, so a rebuild and a project
switch drop it by the same line). The window is a state of its own, `Window`, and not a field of
the reading, because the effect working out the next window reads what is held and would wake
itself on writing beside it.

**The worker is asked a question, not handed a symbol.** An `Ask` is either the symbol an
assembly-driven tab names outright or the source line a source-driven tab is driven from, where
the symbol is whatever that line was compiled into — `compiled::compiled_from` over every open
object, `compiled::pick` choosing among the many one line answers with. Beside the line the ask
carries the tab's **choice** (`Driven::choice`, a row pressed in the Locations panel), at the
head of the ranking `pick` sees: it wins where the line compiled into it and the pick falls back
as if none were made where it did not, and it is part of the ask because a different choice is
a different question, or the listing already up would answer it. The choice is a `Symbol` and
so holds its file's bytes, which the line beside it does not; `close_binary` releases every
choice into the closing file (`Driven::release`) and leaves the lines, and nothing persists a
choice — a restart falls back to the ranking. `ask(active, driven)` is
the whole derivation and is a pure function; `Asked` is the pair of states it reads, and
deliberately **not a `Memo`**, `Active` being one already and a memo over a memo being two beats
behind — which matters because `peek_ask` is what decides whether an answer that has landed is
still wanted. `asked_of(ask)` is the one definition of which tab an answer belongs to (the file's
own tab for a source question, never the resolved symbol's, which is very likely not open), used
by the assembly pane's kept position, by the run of rows a listing change drops, and by the rule
below. Both kinds go to the one worker: a resolve builds the object's source index under the same
non-reentrant mutex `line_info` and `extent` take, so a second thread would block in `get_or_init`
rather than race, and two producers writing one `Analyzed` would break the single `shown`/`pending`
the panes read.

**A source question is asked of every open object, and that is the new worst case.** Each object's
index is built on the first ask — 94 ms for all 196 members of the 20 MB rlib, **2.2 s
and about half a gigabyte for the one object in the 331 MB binary** — and every ask afterwards is two
binary searches. One click on a source row can therefore cost more than any click before it, it is
not superseded once started, and every symbol click behind it waits. That is what `Analysing…` past
`SLOW_ANALYSIS` is for.

**A locate is the same query kept whole, answered into a state of its own** (`ui/locations.rs`).
"Find all locations" on a source row or an instruction row asks `Question::Locate` of the same
worker, and the answer -- `compiled_from`'s `Vec<Symbol>`, every symbol the line was compiled
into over every open object -- lands in `Located`, not in `Analyzed`: it stands until the next
ask whatever the reader opens meanwhile, so it is not a reading of the active document. The
question is a `Query`: the row it was asked from and a `Scope`, one line or **the whole of the
function around it** -- Step 5's instance picker, which is the same panel under another heading
("N instances of `foo`"), asked for by a second entry in the source row's menu. The function's
lines come from the source and not from DWARF (`src/functions.rs`: a scanner of our own for
Rust, since the grammar loses whole files to syntax it does not know, and the tree-sitter parse
for C and C++): "the function the reader is on" is a source notion, `DW_AT_decl_line` is one line and disagrees with the line program by the prologue, and a
symbol's own rows over-cover wherever something was inlined into it. Every symbol holding code
from those lines is listed, an inlined caller included, in the crate's order; the panel's
filter is how a name is narrowed to. A row of it is a **symbol and not a range inside one**, because the crate answers symbols by design
and finding each hit's ranges would be a line-program walk per symbol under the context mutex,
seconds for a line that answers with thousands and every symbol click waiting behind it;
landing on the line inside the symbol is the pin's job (`agents/Panes.md`). Three rules travel with it.
**The queue is drained to the newest question of each kind** (`newest`), not the newest
overall, since a locate is not a newer version of the listing question and drained to one a
symbol click would silently cancel the locations, or the other way round; the listing is
worked first, being what is on screen, then the window, then the locate -- and a window the
reader scrolled past is the one question here that *should* go, the next one asking for whatever
of it still matters. The answer is kept only while its line is the one
`asked` now -- the listing's comparison rule again, and there is no `pending` field, a line
being pending exactly while `asked` and `found` disagree. And **a closed binary takes its
locations with it** (`Found::retain_open`, in the effect reading `Objects` and on the answer
landing), `Shown::still_open`'s rule in a second place: a `Symbol` holds the file's bytes and
this list can hold thousands of them. Its one stated limit is the other direction: the answer is
about the objects that were open when it was asked, so a file opened afterwards is not searched
until the line is asked again -- which asking for the same line does, by dropping the stale
answer. Asking also brings the Locations view to the top of whichever panel holds it, looked
for through the content dock and then the area beside it; on the ask and never on the answer,
so a reader who moved on is not pulled back. **What a row does depends on where the line was
asked from** (`Located::subject`): from the file a source-driven tab is about — its own rows, or
the assembly side that listing belongs to — a row is *chosen for that tab*, which stays where it
is, is driven from the line and has its assembly side follow the symbol; from an assembly-driven
tab, or once the asking tab has closed, a row opens the symbol as a tab of its own. Both go
through `documents::land`, which takes the target document, so the pin and the landing are one
rule for either. An instance row is chosen the same way, from the row the menu was opened on,
so the tab is driven from that line; where the instance holds no code from it, `pick` falls
back as it does for any choice. **The row lit in the panel is the symbol drawn** (`Analysis`),
not the active document: for a source-driven tab the active document is a file, and the lit row
is the one thing on screen that says which instance its assembly side is on.

**`compiled::pick` ranks by where the reader has been, newest first, with the symbol on screen at
its head.** The head is the load-bearing part: nothing is pushed onto the history between two
clicks in one function, so without it reading down the lines of a generic function would walk
across its instantiations. Below the tie-break the order is the crate's own — the lowest-addressed
symbol of the first object that answered — which is arbitrary and is said to be arbitrary; the
instance picker above is where a reader says which instance they meant.

**An answer can now outlive the document that named it, and one rule stops it.** A symbol question
is a tab into one object and that tab closes with its file, so before this nothing in the analysis
needed a rule here at all. A source-driven tab survives `close_binary` by doctrine, so its answer
would go on being drawn — and a `Studied` holds a `Symbol` holds the `Arc<Object>` holds the whole
file's bytes, which is `Positions::forget`'s leak in a second place. `Shown::still_open` is asked in
the two places an answer is judged: by the effect, so a closed binary is a question asked again out
of what is left, and by the task taking answers, so the one already in flight when the file closed
is not taken either. It lives in `use_analysis_with` rather than in `close_binary` so that a close,
a rebuild and a project switch are one line instead of three, and because no handler can reach the
answer in flight. The effect therefore **reads** `Objects` where it only peeks the history: a
question asked of a different set of objects is a different question, while the ranking is an input
to an answer and a visit must not re-ask one. What is deliberately *not* covered: an answer is
about the objects that were open when it was asked, so a line clicked while a file is still being
read can answer with nothing a later object would have answered. It costs one more click, against a
generation counter for a case only a restore-time race reaches.

**A superseded answer is recognised, not prevented.** Every answer carries the `Ask` it is about
and is kept only if that is the question being asked *now* — a comparison and not a generation
counter, since an `Ask` already compares by identity, and since the answer for the first A of an
A → B → A is a perfectly good answer for the third. It is identity of two kinds: `Ask::Symbol` by
the `Arc` pointers `Symbol` compares, `Ask::Source` by `LinePos`, which is the one `Arc` in the UI
compared by its text — so two allocations of one path are one question and a tab switch does not
re-resolve. `Shown::answers` widens it in the one direction that is free: a source question that
resolved to a symbol has already answered a later ask for that symbol outright, and the listing is
retagged rather than worked out again. A dropped answer is
what clicking twice quickly *means*, so nothing logs or retries. **What the panes show meanwhile**
is the listing they already have: `Analyzed` holds `shown` (the listing actually drawn and the
question it answers, which is the one asked *before* this one for as long as the worker takes),
`answered`, `pending` and `slow`. A listing is
replaced by the next listing and never by a blank, or every click would flash the pane empty for a
frame; only after `SLOW_ANALYSIS` (180 ms, started by the request and never polled) does the
message displace it, which is the order of the arms in `Analyzed::showing` — the one place either
pane decides what it is drawing, so the two cannot disagree. `showing` takes the **document** and
not a word from its caller, which is what keeps that true: it spells "Click a source line" where a
symbol tab says "No symbol selected". `answered` is the last question answered *whatever it
answered with*, which is the one thing a listing cannot say for itself — a source line no object
holds code from leaves the listing that is up, losing only the pin's highlight, which is what says
the click landed nowhere, and it is kept only while that listing is `asked_of` the same tab. Two things follow
from `shown` being the drawn symbol rather than the selected one: `InstructionList` is mounted only
for a listing that exists, so `use_kept_position` cannot write a pending tab down at row 0 before a
row of it has been seen; and the Source pane's companion file comes out of `Analysis` rather than
out of `Active`, so it cannot name a file the previous symbol was compiled from, which is what
`Studied` carrying its `Symbol` and `SymbolLines` carrying its file are for.

**Nothing is cached in the UI, deliberately.** `SymbolData::assembly` does not memoize — it decodes
afresh and hands back a new `Arc<Assembly>` — and `Object::line_info` caches the DWARF context and
the subprogram extents but re-walks the covering units' line programs per call. What the `Analysis`
state gives is the one thing a re-render needed: the answer is *held*, so a hover, a theme change or
a resize costs nothing where the old shape re-decoded in `render`. A second, keyed cache would be an
unbounded pile of `Assembly`s for listings the reader has left, to save a few milliseconds on a
symbol they have already been shown. `Reading::held` is not that cache: it is the section view's
one answer, a listing being read in windows rather than whole, bounded by `KEEP` and dropped with
the tab.


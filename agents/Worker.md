# The analysis worker

The one worker thread the panes ask: what a question is, how requests supersede one another, how an
answer is judged when it lands, and what is drawn meanwhile.

**Nothing is analysed on the UI thread.** `SymbolData::assembly` decodes and formats the whole
symbol. `SymbolData::line_info` builds the object's entire DWARF context on the first query against
it. Together they take 1.4 s for the first symbol clicked in the 331 MB binary (debug build; 0.6 s
in release), and both used to run in `render`. `use_analysis` moves them off together, because one
click asks for both and the pane needs both. There is **one worker thread** for the app's lifetime.
It is fed an `async_channel` of `Question`s and answers each with a `Studied`: the `Assembly`, its
`Lanes`, and the `SymbolLines`. It is one worker and not a thread per request or a pool because
requests *supersede* each other. A reader going down the symbol list issues one request per row and
wants only the last one's answer, so each time round the queue is drained to its newest entry and
the rest are dropped *before* they are started. A thread per request would put a whole run of clicks
through the most expensive call in the crate at once for one useful answer, and `DebugInfoCache` is
a `OnceLock`, so the losers would block on the winner instead of running in parallel. (The
parallelism `notes/Goals.md` asks for is parsing many objects at once, a different job.)

**A third kind of question is a window of an object's code** (`src/ui/reading.rs`), for the section
view. `Question::Code(CodeAsk)` carries an object, its skeleton once the view has one, and the
stretches wanted by flat index, **nearest the reader first**. The skeleton (`CodeListing`, free to
build) is built on the worker with the first ask and answered with it. A stretch is decoded through
the crate's own `CodeListing::decode` and then `Studied::with_assembly`, which does the rest of what
`Studied::new` does over a listing already in hand; so the section view and the symbol's own tab
decode a function once and identically. The worker decodes **at most `CHUNK` (8) of the ask** and
then answers, because the queue is drained to its newest question only *between* jobs. A window
decoded whole would hold a symbol click behind every function on a screen and three screens of
buffer; a chunk holds it behind a few, and the view asks for the rest once the chunk has landed. The
answers land in `Reading` (`Sections` at the root) and never in `Analyzed`, which is one symbol's
shape and is read by everything that draws a symbol. **A decoded stretch is a pure function of the
object and the stretch and is never stale**, unlike a listing, which is stale the moment the ask
moves on. So an answer is taken whenever it is about the object and the skeleton on screen,
whichever window asked for it, and only `pending` is judged against the ask; what a scroll
superseded is exactly what the next window asks for again. Two things bound it. A stretch farther
than `KEEP` (512) from the last window is dropped as the answer lands. The whole reading is dropped
when the active document stops being that object's code or the object closes under it
(`use_reading_of`, an effect reading `Active` and `Objects`). It is an effect and not part of
`close_binary` because the skeleton holds every section's bytes, and the effect makes a rebuild and
a project switch drop it by the same line. The window is a state of its own, `Window`, and not a
field of the reading, because the effect that works out the next window reads what is held and would
wake itself if it wrote beside it.

**The worker is asked a question, not handed a symbol.** An `Ask` is either the symbol an
assembly-driven tab names outright, or the source line a source-driven tab is driven from. For a
line, the symbol is whatever the line was compiled into: `compiled::compiled_from` over every open
object, with `compiled::pick` choosing when one line answers with many. Beside the line the ask
carries the tab's **choice** (`Driven::choice`, a row pressed in the Locations panel). `pick` sees
it at the head of its ranking: it wins where the line compiled into it, and where it did not the
pick falls back as if none were made. It is part of the ask because a different choice is a
different question; otherwise the listing already up would answer it. The choice is a `Symbol` and
so holds its file's bytes, which the line beside it does not. `close_binary` releases every choice
into the closing file (`Driven::release`) and leaves the lines. Nothing persists a choice; a restart
falls back to the ranking. `ask(active, driven)` is the whole derivation and is a pure function.
`Asked` is the pair of states it reads. It is deliberately **not a `Memo`**: `Active` is one
already, and a memo over a memo is two beats behind, which matters because `peek_ask` decides
whether an answer that has landed is still wanted. `asked_of(ask)` is the one definition of which
tab an answer belongs to: the file's own tab for a source question, never the resolved symbol's,
which is very likely not open. It is used by the assembly pane's kept position, by the run of rows a
listing change drops, and by the rule below. Both kinds go to the one worker. A resolve builds the
object's source index under the same non-reentrant mutex `line_info` and `extent` take, so a second
thread would block in `get_or_init` rather than race, and two producers writing one `Analyzed` would
break the single `shown`/`pending` the panes read.

**A source question is asked of every open object, and that is the new worst case.** Each object's
index is built on the first ask: 94 ms for all 196 members of the 20 MB rlib, **2.2 s and about half
a gigabyte for the one object in the 331 MB binary**. Every ask after that is two binary searches.
So one click on a source row can cost more than any click before it, it is not superseded once
started, and every symbol click behind it waits. That is what `Analysing…` past `SLOW_ANALYSIS` is
for.

**A locate is the same query kept whole, answered into a state of its own** (`ui/locations.rs`).
"Find all locations" on a source row or an instruction row asks `Question::Locate` of the same
worker. The answer is `compiled_from`'s `Vec<Symbol>`, every symbol the line was compiled into over
every open object. It lands in `Located`, not in `Analyzed`: it stands until the next ask whatever
the reader opens meanwhile, so it is not a reading of the active document. The question is a
`Query`: the row it was asked from and a `Scope`, either one line or **the whole of the function
around it**. The latter is the instance picker, the same panel under another heading ("N instances
of `foo`"), asked for by a second entry in the source row's menu. The function's lines come from the
source and not from DWARF (`src/functions.rs`: a scanner of our own for Rust, since the grammar
loses whole files to syntax it does not know, and the tree-sitter parse for C and C++). This is
because "the function the reader is on" is a source notion, `DW_AT_decl_line` is one line and
disagrees with the line program by the prologue, and a symbol's own rows over-cover wherever
something was inlined into it. Every symbol holding code from those lines is listed, an inlined
caller included, in the crate's order; the panel's filter narrows the list to a name. A row is a
**symbol and not a range inside one**, because the crate answers symbols by design, and finding each
hit's ranges would be a line-program walk per symbol under the context mutex: seconds for a line
that answers with thousands, with every symbol click waiting behind it. Landing on the line inside
the symbol is the selected run's job (`agents/Panes.md`). **The queue is drained to the newest
question of each kind** (`newest`), not the newest overall: a locate is not a newer version of the
listing question, and drained to one, a symbol click would silently cancel the locations, or the
other way round. The listing is worked first, being what is on screen, then the window, then the
locate. A window the reader scrolled past is the one question here that *should* go; the next one
asks for whatever of it still matters. The answer is kept only while its line is the one `asked`
now, the listing's comparison rule again. There is no `pending` field: a line is pending exactly
while `asked` and `found` disagree. And **a closed binary takes its locations with it**
(`Found::retain_open`, in the effect reading `Objects` and when the answer lands). This is
`Shown::still_open`'s rule in a second place: a `Symbol` holds the file's bytes and this list can
hold thousands of them. Its one stated limit is the other direction. The answer is about the objects
that were open when it was asked, so a file opened afterwards is not searched until the line is
asked again; asking for the same line does that, by dropping the stale answer. Asking also brings
the Locations view to the top of whichever panel holds it, looked for through the content dock and
then the area beside it. That happens on the ask and never on the answer, so a reader who moved on
is not pulled back. **What a row does depends on where the line was asked from**
(`Located::subject`). Asked from the file a source-driven tab is about (its own rows, or the
assembly side that listing belongs to), a row is *chosen for that tab*: the tab stays where it is,
is driven from the line, and has its assembly side follow the symbol. Asked from an assembly-driven
tab, or once the asking tab has closed, a row opens the symbol as a tab of its own. Both go through
`documents::land`, which takes the target document, so the selected line and the landing are one
rule for either. An instance row is chosen the same way, from the row the menu was opened on, so the
tab is driven from that line; where the instance holds no code from it, `pick` falls back as it does
for any choice. **The row lit in the panel is the symbol drawn** (`Analysis`), not the active
document. For a source-driven tab the active document is a file, and the lit row is the one thing on
screen that says which instance its assembly side is on.

**A fourth kind is the Source gutter's marks** (`Question::Marks`, answered into `Coded` in
`ui/source_view.rs`). The question is a file and the answer is the lines of it any open object has
code from — `Object::lines_from_source`, which reads the same `SourceIndex` a locate does and hands
back bare line numbers rather than symbols. It is a whole file at a time and not a query per row,
and the Source pane asks it by writing the file it is drawing into `Coded::wanted`, the way the
section view asks for a window by writing it into `Window`: a view cannot reach the request channel,
so a state it writes and an effect here reads is how a pane asks. Worked **last** of the four, being
the answer whose absence costs the reader least while they wait. It is judged on landing by the
file the pane is showing *now*, the listing's comparison rule once more. What keeps it true is
different from the locate's, though: a set of line numbers has nothing in it to sweep for a binary
that has closed, and a state holding the objects to notice would be the state stopping them from
closing — so `Coded` records which objects the answer was worked out over, by pointer
(`object_ids`), and the effect asks again whenever those differ from what is open. A load finishing
is such a difference, which is what puts marks in a gutter drawn before its binary had been read.

**`compiled::pick` ranks by where the reader has been, newest first, with the symbol on screen at
its head.** The head is the load-bearing part: nothing is recorded between two clicks in one
function, so without it reading down the lines of a generic function would walk across its
instantiations. Below the tie-break the order is the crate's own, the lowest-addressed symbol of the
first object that answered. That is arbitrary and is said to be arbitrary; the instance picker above
is where a reader says which instance they meant.

**An answer can now outlive the document that named it, and one rule stops it.** A symbol question
is a tab into one object and closes with its file. A source-driven tab survives `close_binary` by
doctrine, so its answer would go on being drawn. A `Studied` holds a `Symbol`, which holds the
`Arc<Object>`, which holds the whole file's bytes: `Positions::forget`'s leak in a second place.
`Shown::still_open` is asked in the two places an answer is judged: by the effect, so a closed
binary means the question is asked again of what is left, and by the task taking answers, so an
answer already in flight when the file closed is not taken either. It lives in `use_analysis_with`
rather than in `close_binary` so that a close, a rebuild and a project switch are one line instead
of three, and because no handler can reach the answer in flight. The effect therefore **reads**
`Objects` where it only peeks the visits: a question asked of a different set of objects is a
different question, while the ranking is an input to an answer and a visit must not re-ask one. What
is deliberately *not* covered: an answer is about the objects that were open when it was asked, so a
line clicked while a file is still being read can answer with nothing where a later object would
have answered. That costs one more click, against a generation counter for a case only a
restore-time race reaches.

**A superseded answer is recognised, not prevented.** Every answer carries the `Ask` it is about and
is kept only if that is the question being asked *now*. This is a comparison and not a generation
counter: an `Ask` already compares by identity, and the answer for the first A of an A → B → A is a
perfectly good answer for the third. The identity is of two kinds: `Ask::Symbol` by the `Arc`
pointers `Symbol` compares, `Ask::Source` by `LinePos`, the one `Arc` in the UI compared by its
text, so two allocations of one path are one question and a tab switch does not re-resolve.
`Shown::answers` widens it in the one direction that is free: a source question that resolved to a
symbol has already answered a later ask for that symbol outright, and the listing is retagged rather
than worked out again. A dropped answer is what clicking twice quickly *means*, so nothing logs or
retries. **What the panes show meanwhile** is the listing they already have. `Analyzed` holds
`shown` (the listing actually drawn and the question it answers, which is the one asked *before*
this one for as long as the worker takes), `answered`, `pending` and `slow`. A listing is replaced
by the next listing and never by a blank, or every click would flash the pane empty for a frame.
Only after `SLOW_ANALYSIS` (180 ms, started by the request and never polled) does the message
displace it. That is the order of the arms in `Analyzed::showing`, the one place either pane decides
what it is drawing, so the two cannot disagree. `showing` takes the **document** and not a word from
its caller, which is what keeps that true: it says "Click a source line" where a symbol tab says "No
symbol selected". `answered` is the last question answered *whatever it answered with*, the one
thing a listing cannot say for itself. A source line no object holds code from leaves the listing
that is up and lights no pair in it, which is what says the click landed nowhere; it is kept only
while that listing is `asked_of` the same tab. Two things follow from `shown` being the drawn symbol
rather than the selected one. `InstructionList` is mounted only for a listing that exists, so
`use_kept_position` cannot write a pending tab down at row 0 before a row of it has been seen. And
the Source pane's companion file comes out of `Analysis` rather than out of `Active`, so it cannot
name a file the previous symbol was compiled from; that is what `Studied` carrying its `Symbol` and
`SymbolLines` carrying its file are for.

**Nothing is cached in the UI, deliberately.** `SymbolData::assembly` does not memoize: it decodes
afresh and hands back a new `Arc<Assembly>`. `Object::line_info` caches the DWARF context and the
subprogram extents but re-walks the covering units' line programs per call. The `Analysis` state
gives the one thing a re-render needed: the answer is *held*, so a selection, a theme change or a
resize costs nothing where the old shape re-decoded in `render`. A second, keyed cache would be an
unbounded pile of `Assembly`s for listings the reader has left, to save a few milliseconds on a
symbol they have already been shown. `Reading::held` is not that cache: it is the section view's one
answer, a listing read in windows rather than whole, bounded by `KEEP` and dropped with the tab.

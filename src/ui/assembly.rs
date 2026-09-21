//! The assembly half of a document, from the row up: what a row is drawn out of, the
//! branch gutter, the three operands a row can make a link of -- a relocation target's
//! name, a branch's own displacement where the listing has its row, and the address an
//! unnamed call or branch goes to, a door into the object's code that opens with Ctrl --
//! the virtual list of instructions and the pane holding it.
//!
//! The gutter is drawn with **rects and not `canvas()`**, whose `RenderCallback` compares
//! equal unconditionally -- exactly wrong for a row a scroll view recycles. And a row's
//! height must equal the [`code_row_height`] the view over it was given, or scrolling
//! misaligns -- which is why the separator starting a basic block is a hairline inside the
//! row's own top edge and not a gap above it.

use super::*;
use crate::counter;
use std::fmt;

/// The address column, as every row of every listing draws it and copies it: sixteen
/// upper-case hex digits and the space after them.
///
/// One spelling, so a change to the column's width -- or to how a 32-bit object's addresses
/// are shown -- is one edit and cannot leave the drawn column and the copied one disagreeing.
/// How wide the column is *drawn* is [`ADDRESS_WIDTH`], a floor the digits sit inside.
pub(crate) fn address_column(address: impl Drawable) -> String {
    format!("{address:016X} ")
}

/// What an address column may be handed: the two spaces, and the one that has not
/// committed to either. Sealed to those three so that the one place the app writes an
/// address for the reader cannot be given a length, a row index or a bare number -- which
/// `impl fmt::UpperHex` alone would take.
pub(crate) trait Drawable: fmt::UpperHex {}
impl Drawable for PlacedAddress {}
impl Drawable for SectionAddress {}
impl Drawable for Address {}

/// One instruction as one line of text, which is what a copy of the row has to be: the
/// address column, then [`instruction_line`]'s own text. The gutter is left out, being a
/// picture of the branches. `address` is what the listing draws the row at
/// ([`AsmData::drawn_address`]), which is the one decision about *which space* a row's
/// address is in.
pub(crate) fn asm_line(instruction: &Instruction, address: Address) -> String {
    format!("{}{}", address_column(address), text_of(instruction).0)
}

/// What one piece of an instruction's text is: one of the formatter's spans, or the link.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Piece {
    Span(SpanKind),
    Link,
}

/// An instruction's text after its address, piece by piece: the formatter's spans, with
/// the link in place of the one it replaced. There is at most one link because the crate
/// says so: an [`Operand`] is one case and carries the one span it has. A relocation's
/// link says the target's own name -- what [`SymbolData::display`] says, the rule the
/// disassembler substituted the operand by -- and a branch's or a call's the number the
/// formatter printed.
///
/// A name the formatter offered no operand for is a link all the same, appended after
/// every span behind a space.
///
/// **The one walk both halves of a row are built from**: the line it copies
/// ([`text_of`]) and the spans it draws ([`instruction_text`]), so a column into one is a
/// column into the other.
fn pieces(instruction: &Instruction) -> Vec<(&str, Piece)> {
    fn spans(run: &[(String, SpanKind)]) -> Vec<(&str, Piece)> {
        run.iter()
            .map(|(text, kind)| (text.as_str(), Piece::Span(*kind)))
            .collect()
    }
    let format = &instruction.format;
    // Where the link goes and what it says. The crate records a span into `format`'s own
    // length, so the bounds only guard a listing built by hand.
    let (at, text) = match &instruction.operand {
        Some(Operand::SymbolName { symbol, span }) => {
            (span.filter(|&i| i < format.len()), symbol.display())
        }
        Some(Operand::Branch { span, .. } | Operand::Call { span, .. }) => {
            let Some((text, _)) = format.get(*span) else {
                return spans(format);
            };
            (Some(*span), text.as_str())
        }
        Some(Operand::Placeholder) | None => return spans(format),
    };
    match at {
        Some(i) => {
            let mut pieces = spans(&format[..i]);
            pieces.push((text, Piece::Link));
            pieces.extend(spans(&format[i + 1..]));
            pieces
        }
        None => {
            let mut pieces = spans(format);
            pieces.push((" ", Piece::Span(SpanKind::Other)));
            pieces.push((text, Piece::Link));
            pieces
        }
    }
}

/// An instruction's text as the line a row copies, and the columns of its link in it.
///
/// The formatter's padding after the last span is not text, so it is trimmed; a link that
/// ended in whitespace, or said nothing but it, loses what was trimmed. The file names
/// the symbol, so either can happen, and the columns never run past the line.
fn text_of(instruction: &Instruction) -> (Line, Option<Range<usize>>) {
    let mut text = String::new();
    let mut link = None;
    for (piece, kind) in pieces(instruction) {
        let start = text.len();
        text.push_str(piece);
        if kind == Piece::Link {
            link = Some(start..text.len());
        }
    }
    text.truncate(text.trim_end().len());
    let len = text.len();
    let link = link
        .map(|link: Range<usize>| link.start.min(len)..link.end.min(len))
        .filter(|link| !link.is_empty());
    (Line::text(text), link)
}

/// The text instruction `index`'s row draws after its address, as the clipboard sees it,
/// and [`asm_line`] is this behind an address column.
///
/// Total: an index past the last instruction answers an empty line, which is what a row
/// asking about its neighbour below wants of the row after the last (see
/// [`Studied::position`]).
pub(crate) fn instruction_line(assembly: &Assembly, index: usize) -> Line {
    assembly
        .instructions
        .get(index)
        .map_or_else(Line::default, |instruction| text_of(instruction).0)
}

/// How many lanes the gutter is drawn with in a listing of a whole object's code: one
/// width for every symbol in it, so the addresses start at one x. The one place that
/// number is named.
pub(crate) const CODE_LANES: usize = lanes::MAX_LANES;

/// Which of the two listings a symbol's rows are drawn in, and where in it they sit.
///
/// The two are not a pair of independent settings: a symbol read alone starts at row 0 at
/// the file's own addresses with its own gutter, and a stretch of an object's code starts
/// where the stretch does at the addresses the layout placed it at with one gutter width
/// for every symbol. Saying which listing it is says all of that, so nothing can hand a
/// subject to the code listing or a bias to a symbol's own.
#[derive(Clone, PartialEq)]
pub(crate) enum In {
    /// A symbol's own listing, read on its own.
    Alone {
        /// The source-driven tab this listing is the assembly side of, or `None` for an
        /// assembly-driven tab's own listing.
        subject: Option<Subject>,
    },
    /// A stretch of an object's code, drawn among its neighbours.
    Code {
        /// The listing row this symbol's first instruction row is drawn at. What `lanes`
        /// answers in rows is relative to the symbol, and this is what the scroll and the
        /// picked-out run -- which speak the listing's rows -- have it added.
        base: usize,
        /// What is added to every address drawn or copied: the section's place in the
        /// object's layout (`Section::bias`), where two functions of a relocatable object
        /// are both at 0 and have to be told apart.
        bias: Bias,
    },
}

/// A disassembled symbol, where its branches are drawn and what says where its
/// instructions came from, compared by pointer.
///
/// The analysis itself is the worker's own [`Studied`], held whole rather than copied
/// apart, so a field added there reaches the rows without a builder to thread it through.
/// The rest is what the *listing* adds: which of the two listings this is, and where in it
/// the symbol sits.
#[derive(Clone, PartialEq)]
pub(crate) struct AsmData {
    /// What the worker made of the symbol: the listing, its gutter layout and its lines.
    pub(crate) studied: Studied,
    /// Which listing the rows are drawn in, and everything that follows from it.
    pub(crate) listing: In,
}

impl AsmData {
    /// `studied` drawn in `listing`. The one way one of these is made, so the two listings
    /// cannot differ in what they hand their rows.
    ///
    /// [`None`] for a symbol with nothing to decode, which draws no rows at all. That
    /// check is here and nowhere else, and it is what leaves [`AsmData::assembly`] an
    /// answer rather than a question.
    pub(crate) fn of(studied: Studied, listing: In) -> Option<AsmData> {
        studied.assembly.as_ref()?;
        Some(AsmData { studied, listing })
    }

    /// The source-driven tab this listing is the assembly side of: a location found from
    /// it is chosen for it. Never the code listing's, which is a tab of its own.
    pub(crate) fn subject(&self) -> Option<&Subject> {
        match &self.listing {
            In::Alone { subject } => subject.as_ref(),
            In::Code { .. } => None,
        }
    }

    /// The listing row this symbol's first instruction row is drawn at: 0 in a listing
    /// that is one symbol.
    pub(crate) fn base(&self) -> usize {
        match self.listing {
            In::Alone { .. } => 0,
            In::Code { base, .. } => base,
        }
    }

    /// The address the row of instruction `index` draws and copies, and **which space it
    /// is in**: the symbol's own where this listing is that symbol alone, and placed by
    /// the section's bias among its neighbours. The one place that is decided, so no row
    /// has to say which of the two it wanted.
    pub(crate) fn drawn_address(&self, index: usize) -> Address {
        let address = self.assembly().instructions[index].address;
        match self.listing {
            In::Alone { .. } => Address::Local(address),
            In::Code { bias, .. } => Address::Placed(address.placed(bias)),
        }
    }

    /// How many lanes the gutter is drawn with: the symbol's own on its own, and
    /// [`CODE_LANES`] among its neighbours.
    pub(crate) fn width(&self) -> usize {
        match self.listing {
            In::Alone { .. } => self.studied.lanes.width,
            In::Code { .. } => CODE_LANES,
        }
    }

    /// Whether this listing is the object's code already, where a row has no neighbours
    /// to be shown among.
    pub(crate) fn code_tab(&self) -> bool {
        matches!(self.listing, In::Code { .. })
    }

    /// The instructions the rows are drawn from: the worker's own, asked of it rather
    /// than kept beside it, so the two cannot differ. There is always one, [`AsmData::of`]
    /// making none of these for a symbol that has none.
    pub(crate) fn assembly(&self) -> &Arc<Assembly> {
        self.studied
            .assembly
            .as_ref()
            .expect("a listing is made only for a symbol with an assembly")
    }

    /// The object the listing was read out of.
    pub(crate) fn object(&self) -> &Arc<Object> {
        &self.studied.symbol.object
    }

    /// The symbol the listing is of, for a row to name the tab it can be opened alone in.
    pub(crate) fn symbol(&self) -> &Arc<SymbolData> {
        &self.studied.symbol.data
    }

    /// The gutter layout for this symbol's branches, derived from the assembly on the
    /// worker so it can never be a beat behind the rows it is drawn over.
    pub(crate) fn lanes(&self) -> &Arc<Lanes> {
        &self.studied.lanes
    }

    /// This listing as the find bar searches it. Built here and not at each of its two
    /// callers -- what the bar claims, and what a chord hands it -- so the two cannot
    /// name different listings: an answer is judged by `Searchable::id`, the assembly's
    /// pointer, and one about another listing is dropped.
    pub(crate) fn searchable(&self) -> Searchable {
        Searchable::Symbol {
            assembly: self.assembly().clone(),
            lanes: self.lanes().clone(),
        }
    }

    /// `address`, one of this listing's own, in the object's one address space: the
    /// section's place in the layout added (`SymbolData::placed`), which is what a door
    /// into the object's code takes. Not [`drawn_address`](Self::drawn_address), which is
    /// what this listing *draws*, and is a symbol's own where the listing is that symbol.
    pub(crate) fn placed(&self, address: SectionAddress) -> PlacedAddress {
        self.symbol().placed(address)
    }

    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    pub(crate) fn position(&self, index: usize) -> Option<LinePos> {
        self.studied.position(index)
    }

    /// Whether the instruction at `index` is the same place as a line of the source pane's
    /// picked-out run `pair`, which is [`Studied::paired`]; never, where there is no run.
    pub(crate) fn paired(&self, index: usize, pair: Option<&Picked>) -> bool {
        pair.is_some_and(|pair| self.studied.paired(index, pair))
    }
}

/// What the instruction rows are built from: the disassembly, the source pane's run --
/// whose pair the rows light -- and this pane's own, with the branches of the rows in
/// it. Kept apart from `AsmData` so that a selection cannot re-run anything the
/// disassembly drives.
#[derive(Clone, PartialEq)]
struct AsmRows {
    data: AsmData,
    /// What a row's menu writes, consumed once by the list: see [`RowStates`].
    asking: RowStates,
    /// The source pane's picked-out run, or `None` when there is none.
    pair: Option<Picked>,
    /// The edges starting or ending at a picked-out row, which every row the gutter
    /// draws them through has to know about. Worked out once here rather than per row.
    touching: Vec<PlacedEdge>,
    /// The run picked out here -- the caret, the characters, and so the rows -- for each
    /// row to draw its part of, or `None` when there is none.
    chars: Option<CharSelection>,
    /// What the find bar is looking for, compiled once for the list: every row wears the
    /// same one, and a new one is what makes them all draw again.
    marking: Option<Marking>,
}

/// What one row draws in the gutter: its own lanes, and how much of it belongs to a branch
/// of a picked-out row.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RowArrows {
    pub(crate) lanes: RowLanes,
    pub(crate) lit: Lit,
}

/// Where a press on a link in a code row goes, and whether it is a link at all just now
/// ([`Door::open_now`]). The three an instruction can offer are one run of its text with
/// three presses: the hover, the chrome, the Alt rule and the drawn text are the same for
/// each, and this is what differs. The fourth is not an operand: it is the label row in an
/// object's own listing (`section_view`), which asks the same rule so that every link in
/// the app is lit by one answer.
#[derive(Clone)]
pub(crate) enum Door {
    /// The name of a relocation target, in place of the meaningless numeric operand.
    ///
    /// Where the listing is the object's own code, the target's rows are rows of this
    /// very listing, so a plain press moves to them and the reader goes on reading where
    /// they were; Ctrl opens the symbol alone, in a tab of its own as Ctrl does
    /// everywhere. In a symbol's own listing there is nowhere to move to and a plain
    /// press follows the link in place, the way a browser follows one.
    Symbol {
        /// The target, in the object this listing is of.
        symbol: Symbol,
        /// Whether the listing this row is in is the object's code and not one symbol's.
        code_tab: bool,
    },
    /// The address an unnamed call, or a branch this listing has no row for, goes to: a
    /// door into the object's code at that address, and a link on its own, as every other
    /// operand link is. A plain press follows it -- in the unified view the address is a
    /// row of this listing, so the press moves to it, as one on a named target does; in a
    /// symbol's own listing it opens the object's code in place -- and **Ctrl** opens that
    /// code in a tab of its own, which is all Ctrl means anywhere.
    ///
    /// Landed on the row at or below the address (`section::Rows::row_for`): a call into
    /// the middle of a function lands on the instruction holding the byte, a target in a
    /// data stretch on the row of bytes covering it. The line is left unknown, the
    /// target's row not being this row.
    Address {
        object: Arc<Object>,
        /// Where the instruction goes, placed: in the object's one address space.
        address: PlacedAddress,
    },
    /// The row a branch that lands inside this symbol lands on: pressing the
    /// displacement puts that row on screen and pins the line it came from.
    ///
    /// **Not** a navigation. The document does not change, so nothing is pushed onto the
    /// tab's trail -- following a jump is reading further down the same listing, and a
    /// Back button that undid it would be answering a question nobody asked. It *is* a
    /// selection, though: arriving at the target and then having to click it to light it
    /// up made the reader say twice where they had gone, so the press pins exactly what a
    /// press on the target row would, source pane owed the scroll and all.
    Row {
        /// The listing row the branch lands on -- the instruction's row and not its
        /// index, since the scroll and the picked-out run are both in listing space.
        to: usize,
        /// Where that row points on the source side, or `None` where the debug info
        /// places it nowhere. The target's own position and not this row's: the run is
        /// the one a click on the row being jumped to would have made, of that row's
        /// file.
        at: Option<LinePos>,
    },
    /// The symbol a label row names in an object's own listing, opened in a tab of its
    /// own by a **Ctrl**-press on the label.
    ///
    /// A door **only** with Ctrl, and the one link that is: the rows the symbol is
    /// compiled into are the rows under the label, so a plain press has nowhere to go and
    /// stays the row's own -- picking the row out, and beginning a sweep. Which is why the
    /// label is drawn as a link only while Ctrl is held: nothing offers a press it will
    /// not take.
    Label { symbol: Symbol },
}

/// Where a press on a link goes, once the door and the modifiers have been asked
/// ([`Door::opens`]): the **whole** decision, which [`Opens::go`] carries out and adds
/// nothing to. Which is why the two variants that open a document carry the [`Reach`]
/// they open with: read a second time in `go`, whether Ctrl makes a tab of its own would
/// be decided there, and a test of `opens` could not see it.
enum Opens {
    /// The target's own rows, further down the listing already on screen: moved to, which
    /// is a scroll and a caret and neither a tab nor a visit, since `land` plants an
    /// address in the tab that is already showing it. `placed` is the address in the space
    /// that listing draws.
    InCode {
        object: Arc<Object>,
        placed: PlacedAddress,
    },
    /// The target as a listing of its own: followed in place, the way a browser follows a
    /// link, so the function left is one Back away -- or, with Ctrl, in a tab of its own.
    Symbol(Symbol, Reach),
    /// The object's code at an address: moved to where this listing is that code already,
    /// opened in place from a symbol's own listing, and in a tab of its own with Ctrl.
    Code {
        object: Arc<Object>,
        address: PlacedAddress,
        reach: Reach,
    },
    /// A row of the listing on screen, with the place it names for the source pane.
    Row { to: usize, at: Option<LinePos> },
}

impl PartialEq for Opens {
    /// Pointer identity for the objects, as everywhere in the UI, and the rest as they
    /// compare themselves. Hand-written and not derived because an `Object` has no
    /// `PartialEq` of its own. Here so a test can hold a whole decision against what the
    /// press should have made of it.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Opens::InCode { object, placed },
                Opens::InCode {
                    object: other_object,
                    placed: other_placed,
                },
            ) => Arc::ptr_eq(object, other_object) && placed == other_placed,
            (Opens::Symbol(symbol, reach), Opens::Symbol(other_symbol, other_reach)) => {
                symbol == other_symbol && reach == other_reach
            }
            (
                Opens::Code {
                    object,
                    address,
                    reach,
                },
                Opens::Code {
                    object: other_object,
                    address: other_address,
                    reach: other_reach,
                },
            ) => {
                Arc::ptr_eq(object, other_object)
                    && address == other_address
                    && reach == other_reach
            }
            (
                Opens::Row { to, at },
                Opens::Row {
                    to: other_to,
                    at: other_at,
                },
            ) => to == other_to && at == other_at,
            _ => false,
        }
    }
}

impl Opens {
    /// Go there: the four ways of carrying the decision out, apart from making it. No
    /// modifier is read here -- what they said is in the decision.
    ///
    /// `listing` is the list's own scroll and box, read at the press rather than at the
    /// render that drew the link, so a row moved to is a row of the listing on screen
    /// now.
    fn go(self, doors: Doors, listing: &Listing) {
        match self {
            Opens::InCode { object, placed } => {
                show_in_code(doors, object, placed, None, Reach::InPlace);
            }
            // Landed with the caret on the first instruction, which the pane owes a
            // reveal: the target opens at its top even where this tab has been there
            // before and kept a row for it.
            Opens::Symbol(symbol, reach) => {
                let address = symbol.data.address;
                let tab = Document::Symbol(symbol);
                let landing = Landing {
                    tab,
                    at: None,
                    address: Some(Address::Local(address)),
                };
                land(doors, landing, reach);
            }
            // `show_in_code` leaves the move to `land` where this listing is that code
            // already.
            Opens::Code {
                object,
                address,
                reach,
            } => {
                show_in_code(doors, object, address, None, reach);
            }
            Opens::Row { to, at } => {
                // The row is reached by a press, so the pane is on screen and measured:
                // the height is peeked, a handler subscribing to nothing.
                let mut controller = listing.controller;
                let _ = reveal_row(&mut controller, listing.height(), listing.rows(), to);
                // The row landed on becomes the picked-out one; the press that followed
                // the link marked nothing of its own. The source pane owes the scroll to
                // the target's line, where it has one; this pane has just been given its
                // own, above.
                mark_row(doors.marked, at.map(|at| at.file), to);
            }
        }
    }
}

impl Door {
    /// **Whether a press on the link is a door now**, which is the one answer the light,
    /// the pointer's icon and the press are all picked by, so none of the three can offer
    /// what the others will not do. Always, where what the door opens does not turn on the
    /// reader; only with **Ctrl** held for a label, which without it has nowhere to go.
    ///
    /// Asked through the closure the row is handed ([`TextLinks`]), for the light, the
    /// pointer's icon and the press. Alt is not part of it: Alt says a press on a link is
    /// a selection this time, and shuts every door in every pane, so the row asks it of
    /// every link rather than any one of them.
    ///
    /// `ctrl` is asked only where the answer turns on it, so only the links it can change
    /// are drawn again as it goes down and up.
    pub(crate) fn open_now(&self, ctrl: impl FnOnce() -> bool) -> bool {
        match self {
            Door::Symbol { .. } | Door::Address { .. } | Door::Row { .. } => true,
            Door::Label { .. } => ctrl(),
        }
    }

    /// **What a press on the link opens**, with Ctrl as it was when the button went down.
    /// [`None`] is a press that is not a door: it goes on into the row, which picks the
    /// line out and opens nothing. That is a door [`open_now`](Self::open_now) says is not
    /// open, which for a label is Ctrl not being held.
    fn opens(&self, ctrl: bool) -> Option<Opens> {
        if !self.open_now(|| ctrl) {
            return None;
        }
        // Where a door that opens a document opens it: in place, or, with Ctrl, in a tab
        // of its own -- the rule every link inside a pane follows (`Reach::inside`).
        let reach = Reach::inside_with(ctrl);
        Some(match self {
            // In the unified view the target is further down this same listing: moved to,
            // at the address that listing draws it at, which is the placed one.
            Door::Symbol { symbol, code_tab } if *code_tab && !ctrl => Opens::InCode {
                object: symbol.object.clone(),
                placed: symbol.data.placed(symbol.data.address),
            },
            Door::Symbol { symbol, .. } => Opens::Symbol(symbol.clone(), reach),
            Door::Address { object, address } => Opens::Code {
                object: object.clone(),
                address: *address,
                reach,
            },
            // Only ever with Ctrl, so in a tab of its own.
            Door::Label { symbol } => Opens::Symbol(symbol.clone(), reach),
            Door::Row { to, at } => Opens::Row {
                to: *to,
                at: at.clone(),
            },
        })
    }

    /// The colour the link is drawn in at rest, and the one it takes while it is lit --
    /// which is the rule under it too.
    fn colours(&self) -> (Color, Color) {
        match self {
            Door::Symbol { .. } | Door::Label { .. } => {
                (palette().name_fg, palette().name_hover_fg)
            }
            Door::Address { .. } => (kind_color(SpanKind::Address), palette().name_hover_fg),
            Door::Row { .. } => (kind_color(SpanKind::Address), palette().branch_lit_fg),
        }
    }
}

/// What a press on a linked operand reaches for: Ctrl, which the door is asked about,
/// where the door leads, and the list's own scroll and box. Gathered by the row that
/// draws the link ([`use_link_states`]), a handler being no place to call a hook.
#[derive(Clone)]
pub(crate) struct LinkStates {
    /// Whether Ctrl is held, which is what makes a label a door and a target open in a
    /// tab of its own.
    ctrl: State<bool>,
    /// Where a press on the link goes.
    doors: Doors,
    /// The list's own scroll and its measured height, which `reveal_row` needs at the
    /// moment of the press rather than at the render that drew the row.
    listing: Listing,
}

/// What the links in a row reach for, from inside the row: the [`Listing`] is the one the
/// list's box provided. `doors` is handed in so an instruction row takes its
/// [`RowStates`]'s, and its menu and its links cannot disagree about where a door leads.
pub(crate) fn use_link_states(doors: Doors) -> LinkStates {
    LinkStates {
        ctrl: use_consume::<Ctrl>().0,
        doors,
        listing: use_consume::<Listing>(),
    }
}

impl LinkStates {
    /// The link at `columns` of a row, through `door`: lit and followed while the door is
    /// open, and followed by carrying out what it [`opens`](Door::opens). Ctrl is read in
    /// the closure the row asks from its render, so only a row the pointer is on is
    /// subscribed to it, and peeked at the press.
    ///
    /// The row's text is in no file, so it has no names a language server could be asked
    /// about: a question is put by file, line and column.
    pub(crate) fn link(&self, columns: Range<usize>, door: Door) -> TextLinks {
        let LinkStates {
            ctrl,
            doors,
            ref listing,
        } = *self;
        let listing = listing.clone();
        let lit_fg = door.colours().1;
        let asked = door.clone();
        TextLinks {
            columns: vec![columns],
            is_link: Rc::new(move || asked.open_now(|| ctrl())),
            follow: Rc::new(move |_| {
                if let Some(opens) = door.opens(*ctrl.peek()) {
                    opens.go(doors, &listing);
                }
            }),
            lit_fg,
            names: Vec::new(),
            on_hover: None,
        }
    }
}

/// The branch gutter for one row: a vertical line for every lane running through it, the
/// horizontal run out to the listing where a branch starts or ends here, and an arrowhead
/// where one lands. `width` is the whole symbol's lane count and not this row's, so that
/// the addresses start at the same x on every row of the listing.
///
/// Rects, and not `freya-components`' `canvas()`, whose `RenderCallback` compares equal to
/// every other one: a canvas whose *drawing* changed while its layout did not tells the
/// diff nothing, and a row recycled by a `VirtualScrollView` is exactly that.
///
/// The strokes are positioned absolutely, which is what lets the lanes sit at fixed
/// columns and the two halves of a corner meet in the middle of the row. It is also why
/// `InstructionRow` pads horizontally only: a line has to reach the row's own top and
/// bottom edges, or the gutter comes out dashed with one gap per row.
///
/// Every stroke here is put on the device pixel grid by its **edges** ([`Grid`]), never
/// by placing its centre on a fraction: a one-pixel line drawn across two device pixels
/// comes out as two grey ones, which beside the crisp text next to it reads as blurred.
/// The two exceptions are deliberate -- the row's own top and bottom, which a line must
/// reach exactly or the column is dashed, and the arrowhead's diagonals, which no
/// placement can align and which are drawn half a device pixel wider instead.
fn gutter(width: usize, arrows: RowArrows) -> impl IntoElement {
    let grid = pixel_grid();
    let height = code_row_height();
    debug_assert!(width > 0);
    // The row of device pixels the horizontal run is drawn in. It is also where the two
    // halves of a corner meet and where the arrowhead pivots, so all three are put on the
    // grid by this one answer rather than each rounding `height / 2.0` for itself.
    let run = grid.stroke(height / 2.0, BRANCH_STROKE);
    // Where an arrowhead points, and where a horizontal run ends. Lane 0 is the innermost,
    // so the lanes are laid out leftwards from here.
    let tip = grid.edge(width as f32 * LANE_WIDTH + ARROW_WIDTH);
    let column = move |lane: usize| {
        grid.stroke(
            (width - 1 - lane) as f32 * LANE_WIDTH + LANE_WIDTH / 2.0,
            BRANCH_STROKE,
        )
    };

    // The horizontal run and the arrowhead are the two ends of one gesture, so both are
    // lit exactly when a branch of a picked-out row has an end in this one.
    let lit = arrows.lit.corner;

    let stroke = move |left: f32, top: f32, wide: f32, tall: f32, lit: bool| {
        rect()
            .position(Position::new_absolute().left(left).top(top))
            .width(Size::px(wide))
            .height(Size::px(tall))
            .background(if lit {
                palette().branch_lit_fg
            } else {
                palette().branch_fg
            })
    };

    rect()
        .width(Size::px(grid.edge(tip + GUTTER_PAD)))
        .height(Size::px(height))
        .children((0..width).filter_map(move |lane| {
            let vertical = arrows.lanes.lanes[lane];
            // A half stroke ends at the far side of the horizontal run rather than at its
            // centre line, so the corner is filled to the pixel instead of ending inside
            // the run and leaving the notch that an antialiased end would draw there.
            let (top, tall) = match (vertical.top, vertical.bottom) {
                (true, true) => (0.0, height),
                (true, false) => (0.0, run.far()),
                (false, true) => (run.near, height - run.near),
                (false, false) => return None,
            };

            let column = column(lane);
            Some(
                stroke(column.near, top, column.thick, tall, arrows.lit.lanes[lane]).into_element(),
            )
        }))
        .maybe_child(arrows.lanes.stub.map(|lane| {
            // From the near edge of its lane's own stroke and not from the lane's centre:
            // the half pixel that adds is under that stroke, and starting on the grid is
            // what keeps the run's own left edge crisp where the lane has no stroke to
            // hide it.
            let across = grid.span(column(lane).near, tip);
            stroke(across.near, run.near, across.thick, run.thick, lit)
        }))
        // The two strokes of the arrowhead are one stroke turned about its right end,
        // which is the tip, once each way. A diagonal cannot be put on the grid at all, so
        // it is weighted instead of aligned -- `Grid::diagonal` -- and only its pivot is
        // snapped, which is the run's own end.
        .maybe(arrows.lanes.arrow, |el| {
            let barb = grid.diagonal(BRANCH_STROKE);
            el.children([ARROW_ANGLE, -ARROW_ANGLE].map(move |angle| {
                stroke(
                    tip - ARROW_STROKE,
                    run.centre() - barb / 2.0,
                    ARROW_STROKE,
                    barb,
                    lit,
                )
                .rotate(angle)
                .transform_origin(TransformOrigin::right())
                .into_element()
            }))
        })
}

/// How wide a gutter of `width` lanes is drawn, which is what a row giving the column up
/// takes ([`gutter_column`]).
fn gutter_width(width: usize) -> f32 {
    if width == 0 {
        return 0.0;
    }
    let grid = pixel_grid();
    grid.edge(grid.edge(width as f32 * LANE_WIDTH + ARROW_WIDTH) + GUTTER_PAD)
}

/// The gutter's column, as every row that takes one takes it: `arrows` for a row drawing
/// its own branches, and [`None`] for a row that draws none and only gives the column up,
/// so the address beside it starts where an instruction row's does. Nothing at all for a
/// listing of no lanes -- a symbol branching nowhere inside itself, which most do -- since
/// an empty column would still be a column, and that is why the gutter mark cannot sit
/// inside it.
pub(crate) fn gutter_column(width: usize, arrows: Option<RowArrows>) -> Option<Element> {
    if width == 0 {
        return None;
    }
    Some(match arrows {
        Some(arrows) => gutter(width, arrows).into_element(),
        None => rect().width(Size::px(gutter_width(width))).into_element(),
    })
}

/// The address column, as an instruction row and an object listing's text rows both draw
/// it: wide enough for a 64-bit address and the space after it, so every row's text starts
/// at one x. [`None`] for a row that has no address of its own and gives the column up all
/// the same.
pub(crate) fn address_label(address: Option<impl Drawable>) -> Element {
    label()
        .text(address.map(address_column).unwrap_or_default())
        .min_width(Size::px(ADDRESS_WIDTH))
        .color(palette().address_fg)
        .max_lines(1)
        .into_element()
}

/// The hairline a [`SeparatorRow`] draws across its middle, between the gutter and the
/// listing's right edge.
///
/// A rect of its own and not a border on the row: a border is drawn on an edge of the box
/// it is given, and the box here is a whole row. It starts after the gutter rather than
/// crossing it, because the gutter is a column of unbroken branch lines and a rule struck
/// through them reads as one of them breaking.
///
/// Put on the device pixel grid by its edges, exactly as the gutter's strokes are and
/// from the same answer ([`Grid::stroke`] over the middle of a row), so that a rule and a
/// horizontal run drawn on one row sit in the same device pixels. Centring it with
/// `cross_align` was what put it on a fraction: half of an even row height is a whole
/// number, and a one-pixel rect centred on one straddles the two pixels either side of
/// it. The offset is a padding rather than an absolute position so the rule still takes
/// the width the row's flex leaves it.
pub(crate) fn block_rule() -> impl IntoElement {
    let rule = pixel_grid().stroke(code_row_height() / 2.0, BLOCK_RULE_STROKE);

    rect()
        .width(Size::fill())
        .height(Size::px(code_row_height()))
        .padding(Gaps::new(rule.near, 0.0, 0.0, 0.0))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(rule.thick))
                .background(palette().block_rule),
        )
}

/// The row between two basic blocks: a full row of the listing, carrying the rule across
/// its middle and the gutter's crossing lanes down its left.
///
/// A row of its own and not a border on the row below, so that a block reads as separated
/// from the one above rather than as underlined by it. It is exactly `code_row_height()`,
/// like every other row -- the `VirtualScrollView`'s `item_size` is one number for the
/// whole listing -- which is what the second index space in [`Lanes`] is for.
///
/// **Keyed, uniquely, and apart from the instruction rows.** Unkeyed, every separator
/// would share the type's default key, and freya matches siblings by key alone: a
/// listing scrolled by a separator's distance puts a different separator in the same
/// slot, the diff calls it the same row unmoved, and the moves around it leave the scope
/// graph disagreeing with the element tree -- at which point `run_scope` hands an
/// `InstructionRow`'s props to a scope keeping a `SeparatorRow`'s render closure, and
/// the downcast inside freya unwraps `None` (`notes/upstream/freya.md`). The key is the
/// address of the instruction below, tagged so it can never equal an instruction row's.
#[derive(Clone, PartialEq)]
pub(crate) struct SeparatorRow {
    /// The listing row this is, for the picked-out run: a sweep that crosses a boundary
    /// must not stop tracking the pointer, and a copy takes the blank line it draws.
    pub(crate) row: usize,
    /// The wash of its pane's selection, if it is in it.
    pub(crate) wash: Wash,
    /// The gutter's width for the whole symbol, and the lanes crossing this boundary.
    pub(crate) width: usize,
    pub(crate) arrows: RowArrows,
    pub(crate) key: DiffKey,
}

keyed!(SeparatorRow);

impl SeparatorRow {
    /// The separator above instruction `below` of `data`, drawn at listing row `row`.
    ///
    /// **The one way either listing makes one**, as [`AsmData::of`] is the one way it makes
    /// what they are drawn from: the lanes crossing the boundary belong to the row below,
    /// and the separator lights with that row's branches but never draws their corner,
    /// which is the arrowhead's and belongs to the row landed on.
    pub(crate) fn over(
        data: &AsmData,
        below: usize,
        row: usize,
        chars: Option<CharSelection>,
        touching: &[PlacedEdge],
    ) -> SeparatorRow {
        let mut lit = lanes::lit(touching, below);
        lit.corner = false;
        SeparatorRow {
            row,
            wash: wash_of(chars, row),
            width: data.width(),
            arrows: RowArrows {
                lanes: data.lanes().boundary(below),
                lit,
            },
            key: DiffKey::None,
        }
    }
}

impl Component for SeparatorRow {
    fn render(&self) -> impl IntoElement {
        let width = self.width;

        // The rows' own chrome, **never measured**: what it holds is the gutter and the
        // rule, and the rule is as wide as the row, so a separator reporting itself would
        // report the row plus its gutter and the widest row would grow by a gutter's
        // width every layout, without end. It takes the mark handlers with the chrome,
        // so a sweep down the listing is not cut in half by every boundary it crosses;
        // a run started on a separator is a row of no file and no text.
        use_code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                measured: false,
            },
            std::iter::once(code_mark(false))
                .chain(gutter_column(width, Some(self.arrows)))
                .collect(),
            None,
            None,
        )
        .child(block_rule())
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct InstructionRow {
    pub(crate) data: AsmData,
    /// What this row's menu writes, told to it by the list: a handler may not run a hook.
    /// Compares equal always, so it costs the row no render ([`RowStates`]).
    pub(crate) asking: RowStates,
    /// Which instruction this row draws.
    pub(crate) index: usize,
    /// Which row of the listing it is drawn in, which is `index` plus every separator
    /// above it and plus [`AsmData::base`]. The picked-out run and the scroll speak this
    /// one; everything else -- the gutter, the line info, the branch edges -- speaks
    /// `index`. See [`Lanes`].
    pub(crate) row: usize,
    /// What this row draws in the gutter, worked out by the list for the reason `paired`
    /// is: the lanes lit in row 40 belong to a branch of row 12.
    pub(crate) arrows: RowArrows,
    /// Whether this instruction was compiled from a line of the source pane's picked-out
    /// run, and if so which of its edges end the run of such rows. Worked out by the list
    /// rather than read here, so that a run growing by a line leaves every row not on it
    /// untouched.
    pub(crate) paired: Option<Edges>,
    /// The wash of its pane's selection: a row of a run picked out whole, or the caret's
    /// row. Worked out by the list too, so a row re-renders on its own wash changing and
    /// not on every row a drag passes over.
    pub(crate) wash: Wash,
    /// The columns of this row inside the pane's character selection, worked out by the
    /// list for the reason `selected` is (`RowChars`).
    pub(crate) chars: RowChars,
    /// What the find bar is looking for, compiled once per render of the list and shared
    /// by every row of it; `None` where no bar is open (`find_bar.rs`).
    pub(crate) marking: Option<Marking>,
    pub(crate) key: DiffKey,
}

keyed!(InstructionRow);

impl InstructionRow {
    /// Instruction `index` of `data`, drawn at listing row `row`. The one way either
    /// listing makes one, so a field added above is filled in one place and the two
    /// cannot hand their rows different things.
    ///
    /// What the list alone knows is handed in: `paired`, which is about the rows either
    /// side of this one, and `touching`, the edges of the run picked out in the pane.
    /// The wash, the columns and the gutter follow from those and are worked out here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn at(
        data: AsmData,
        asking: RowStates,
        index: usize,
        row: usize,
        paired: Option<Edges>,
        chars: Option<CharSelection>,
        touching: &[PlacedEdge],
        marking: Option<Marking>,
    ) -> InstructionRow {
        InstructionRow {
            arrows: RowArrows {
                lanes: data.lanes().row(index),
                lit: lanes::lit(touching, index),
            },
            data,
            asking,
            index,
            row,
            paired,
            wash: wash_of(chars, row),
            chars: RowChars::of(chars, row),
            marking,
            key: DiffKey::None,
        }
    }
}

/// Where a press on instruction `index`'s link goes, picked by what its operand names: a
/// relocation target's symbol; a branch's own row where this listing has the row it lands
/// on, which is the same set the gutter draws an arrow for; and otherwise the address it
/// goes to, a door into the object's code there in either listing -- the unified view's
/// own rows included, where the target may be screens away.
fn door_of(data: &AsmData, index: usize) -> Option<Door> {
    let instruction = data.assembly().instructions.get(index)?;
    Some(match instruction.operand.as_ref()? {
        Operand::SymbolName { symbol, .. } => Door::Symbol {
            symbol: Symbol {
                object: data.object().clone(),
                data: symbol.clone(),
            },
            code_tab: data.code_tab(),
        },
        // The run a press on the row landed on would have made.
        Operand::Branch { address, .. } => match data.assembly().edge_from(index) {
            Some(edge) => Door::Row {
                to: data.base() + data.lanes().row_of(edge.to),
                at: data.position(edge.to),
            },
            None => Door::Address {
                object: data.object().clone(),
                address: data.placed(*address),
            },
        },
        Operand::Call { address, .. } => Door::Address {
            object: data.object().clone(),
            address: data.placed(*address),
        },
        Operand::Placeholder => return None,
    })
}

/// The text instruction `index`'s row draws after its address, and the line that row
/// copies, both out of one walk ([`pieces`]). The link keeps the operand's own position,
/// inside the brackets of a memory operand and after the `rip+` of a rip-relative one,
/// and is a span of the row's text like any other, so a sweep selects across it.
///
/// `states` is handed in and not reached for: it is the list's, consumed once where the
/// rows are built, and this is not a component.
fn instruction_text(
    data: &AsmData,
    index: usize,
    chars: RowChars,
    marking: Option<Marking>,
    states: &LinkStates,
) -> Text {
    let instruction = &data.assembly().instructions[index];
    let door = door_of(data, index);
    // The link's text in the colour its door is drawn in at rest.
    let rest = door
        .as_ref()
        .map_or(palette().name_fg, |door| door.colours().0);
    let spans = pieces(instruction)
        .into_iter()
        .map(|(text, piece)| {
            let (colour, weight) = match piece {
                Piece::Span(kind) => (
                    kind_color(kind),
                    match kind {
                        SpanKind::Mnemonic => FontWeight::BOLD,
                        _ => FontWeight::NORMAL,
                    },
                ),
                Piece::Link => (rest, FontWeight::NORMAL),
            };
            Span::new(text.to_owned())
                .color(colour)
                .font_weight(weight)
                .assembly_font()
        })
        .collect();
    let (line, columns) = text_of(instruction);

    Text {
        marking,
        line,
        spans,
        chars,
        links: door
            .zip(columns)
            .map(|(door, columns)| states.link(columns, door)),
    }
}

/// What the right button offers on instruction `index`'s row: the line's locations, where
/// the debug info gives the row a line; the row shown among its neighbours, where it is
/// not already; and the symbol bookmarked, always. `at` is where the row points on the
/// source side, worked out by the caller, which needs the same answer.
///
/// The states it writes are handed in whole ([`RowStates`]), because reaching for a
/// context is a hook and this handler runs long after the render that built it.
fn instruction_menu(
    asking: RowStates,
    data: &AsmData,
    index: usize,
    at: Option<LinePos>,
) -> RowMenu {
    let RowStates {
        doors,
        locating,
        bookmarked,
        objects,
    } = asking;
    let instruction = &data.assembly().instructions[index];
    // The source-driven tab this listing is the assembly side of, if it is one: a
    // location found from it is chosen for it.
    let subject = data.subject().cloned();
    // The symbol this row is code of, in either listing: what the door back opens, and
    // what the menu bookmarks. One symbol, so one value.
    let symbol = Symbol {
        object: data.object().clone(),
        data: data.symbol().clone(),
    };
    // The row's door into the object's code, unless this listing is that already -- and
    // from there, the door back to the symbol read alone. The door takes the placed
    // address, which in a symbol's own listing is not the one drawn.
    let neighbours =
        (!data.code_tab()).then(|| (data.object().clone(), data.placed(instruction.address)));
    // The door back takes the symbol's own address, the space its listing draws.
    // Wherever this listing is not the tab itself, which is an object's code and the
    // assembly side of a source-driven tab: in the second the symbol has no other door,
    // the Symbols list aside, since the tab is a file. An assembly-driven tab is the
    // symbol already and gets none.
    let alone = (data.code_tab() || data.subject().is_some())
        .then(|| (symbol.clone(), instruction.address));
    // The same symbol as a document, which is what the bookmark item takes.
    let symbol_document = Document::Symbol(symbol);

    // The column is the source pane's business: nothing in an instruction row is a name a
    // server could be asked about.
    Rc::new(move |e: Event<PressEventData>, _| {
        let menu = match &at {
            // No Alt+F12: the key asks about the caret in the **Source** pane, and this
            // listing draws no source (`caret_questions`).
            Some(at) => locate_menu(
                locating,
                at.clone(),
                subject.clone(),
                None,
                Vec::new(),
                None,
            ),
            None => Menu::new(),
        };
        let menu = menu.maybe_child(neighbours.clone().map(|(object, address)| {
            let at = at.clone();
            MenuButton::new()
                .on_press(move |_| {
                    show_in_code(doors, object.clone(), address, at.clone(), Reach::NewTab)
                })
                .child("Show in unified view")
        }));
        let menu = menu.maybe_child(alone.clone().map(|(symbol, address)| {
            let at = at.clone();
            MenuButton::new()
                .on_press(move |_| open_as_symbol(doors, symbol.clone(), address, at.clone()))
                .child("Open as symbol")
        }));
        // And no Ctrl+D: the key bookmarks the place the tab on screen is showing, which
        // is this row's symbol only when the tab is that symbol read alone.
        let menu = menu.child(bookmark_item(
            bookmarked,
            objects,
            symbol_document.clone(),
            "Bookmark symbol",
            None,
        ));
        ContextMenu::open_from_event(&e, menu);
    })
}

counter!(
    /// Test-only: how many times this thread has drawn an instruction row.
    pub(crate) fn instruction_rows_drawn() = INSTRUCTION_ROWS_DRAWN
);

impl Component for InstructionRow {
    fn render(&self) -> impl IntoElement {
        #[cfg(test)]
        INSTRUCTION_ROWS_DRAWN.set(INSTRUCTION_ROWS_DRAWN.get() + 1);
        let links = use_link_states(self.asking.doors);

        // Where this row points on the source side. Worked out once here rather than in
        // each of the handlers, which all need the same answer.
        let at = self.data.position(self.index);
        let address = self.data.drawn_address(self.index);

        // Before the text: the mark, saying whether the debug info places this
        // instruction anywhere at all; the arrow gutter; and the address, which is gutter
        // too, a press on it picking the row out and no characters.
        let before = std::iter::once(code_mark(at.is_some()))
            .chain(gutter_column(self.data.width(), Some(self.arrows)))
            .chain([address_label(Some(address))])
            .collect();

        // The run is a run of the file this row was compiled from, which is what the
        // source pane shows beside an object's code.
        use_code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: at.as_ref().map(|at| at.file.clone()),
                paired: self.paired,
                wash: self.wash,
                measured: true,
            },
            before,
            Some(instruction_text(
                &self.data,
                self.index,
                self.chars,
                self.marking.clone(),
                &links,
            )),
            Some(instruction_menu(self.asking, &self.data, self.index, at)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// The instruction rows themselves, a component of their own so that the pointer focus and
/// the picked-out run -- which change on every pointer move across a row boundary -- do not
/// re-render the pane above, which changes only when a symbol is analysed.
#[derive(Clone, PartialEq)]
struct InstructionList {
    /// The tab these rows are in.
    tab: DocId,
    /// The listing these rows draw, made by the pane -- which is what says there is one
    /// to draw. It carries the whole of what the worker made of the symbol and not just
    /// its object, because these rows draw a disassembly *and* answer to it: a relocation
    /// link navigates to a symbol in the same object.
    data: AsmData,
    /// The question this listing answers, and **not** the one being asked: while the
    /// worker catches up the pane is still drawing the listing being left. Two things
    /// come out of it -- [`asked_of`], the place on the tab's trail whose viewing
    /// position this is (the file for a source-driven one, never the resolved symbol,
    /// which is very likely on no trail at all), and the file a source-driven tab is
    /// about, which its rows' menus choose a location for.
    asked: Ask,
    /// The last question the worker answered, which says whether `asked` is the tab's
    /// question yet ([`use_drawn_place`]).
    answered: Option<Ask>,
}

/// The listing row the reveal `owing` asks for goes to, and [`None`] where this listing
/// cannot answer it -- which leaves the request owed rather than spent on a guess.
///
/// A run of the pane's own is a run of these rows, which [`Owing::row`] answers. The
/// other pane's run is answered here by the first instruction compiled from a line of it.
/// Nothing at all when those lines produced no instruction here -- ones the optimiser
/// folded away, or belonging to another function, or, in a source-driven tab, the listing
/// this very click is asking for not having arrived yet. Scrolling somewhere arbitrary
/// would be worse than not scrolling.
///
/// The own run is already in the listing's rows and the paired instruction is not, so
/// `lanes` is what makes a row of it: the separators above it are rows too.
fn owed_listing_row(
    owing: &Owing,
    lanes: &Lanes,
    paired: impl FnOnce(&Picked) -> Option<usize>,
) -> Option<usize> {
    owing.row(|pair| paired(pair).map(|index| lanes.row_of(index)))
}

impl Component for InstructionList {
    fn render(&self) -> impl IntoElement {
        // What the rows' menus write, consumed here and carried to them: a handler may not
        // run a hook.
        let asking = use_row_states();
        let doors = asking.doors;
        let marked = doors.marked;
        let chars = chars_of(marked, Pane::Assembly);
        // The source pane's run, whose pair these rows light.
        let pair = pair_of(marked, Pane::Assembly);
        // The listing these rows are of, which is this disassembly: what its widest row
        // and its kept position are held under.
        let listing = Widest::key(Arc::as_ptr(self.data.assembly()).addr());
        // The box the rows are drawn in, and the scroll and the measurement that come
        // with it.
        let list = use_list_box(Pane::Assembly, listing);
        // What the find bar over this pane is looking for, for every row to wash, and
        // what it searches, claimed for as long as these rows are drawn.
        let at = (Placing::Tab(self.tab), Pane::Assembly);
        let marking = use_marking(at);
        use_searching(at, Some(self.data.searchable()));
        let (controller, viewport) = (list.controller, list.viewport());

        let data = self.data.clone();
        // The listing's rows, which is the instructions plus a separator above every row a
        // branch lands on. Everything below that scrolls, picks out or counts rows is in
        // this space; `AsmData::position`, the gutter and the edges are in the
        // instructions'. `Lanes` converts, and is the only thing that may.
        let length = data.lanes().listing_rows();
        // Where this tab was left, put back when it is switched to and written down as it
        // is scrolled -- and the scroll this pane owes a run, which wins over it.
        let docs = doors.open.docs;
        // The place this listing is for, which for a source-driven tab is a line of the
        // file and not the file: two lines of one file reached along one trail are two
        // entries, each with its own scroll.
        let (entry, fresh) = use_drawn_place(
            docs,
            asking.doors.places.driven,
            self.tab,
            &asked_of(&self.asked),
            true,
            self.answered.as_ref(),
        );
        use_kept_position(
            asking.doors.places.asm_at,
            docs,
            Pane::Assembly,
            {
                let data = data.clone();
                move || {
                    // Asked before anything else: `owed_reveal` reads the marks, and that
                    // read is what wakes this on the next click. **The request it answers
                    // nothing for is left owed**, so the listing that can answer it still
                    // finds it.
                    let owing = owed_reveal(marked, Pane::Assembly)?;
                    owed_listing_row(&owing, data.lanes(), |pair| data.studied.first_paired(pair))
                }
            },
            // A landing's half for this pane is an address, and an address is a row of a
            // listing that arrives later than the document: it is left as a `Planting`
            // and spent below, never taken here.
            |_: &Landing| None,
            controller,
            viewport,
            &entry,
            length,
            listing,
            // Nothing to open at: a listing *is* the symbol, so its first row is its own
            // first line and the top is where it already is.
            None,
        );
        // The caret a door left to be planted on an instruction of this listing, once the
        // listing is the place the tab is at and the document the caret names: the pane
        // draws the listing being left until the worker answers. On the row of the instruction at or below the address, the symbol's
        // own; `take_planting` has spent the planting by then, so an address before the
        // first is dropped rather than left. The pane owes the caret its reveal, as it
        // owes a click from outside, and the reveal wins over the kept row in
        // `use_kept_position`, as a reveal does.
        let plant = doors.plant;
        // The listing is in the deps with the entry, and not captured: the callback is
        // built once, and this list is handed each symbol it draws without being mounted
        // again.
        use_side_effect_with_deps(
            &(entry.clone(), data.clone(), fresh),
            move |((_, stop), data, fresh): &(Entry, AsmData, bool)| {
                // Never into the listing being left.
                if !fresh {
                    return;
                }
                // The planting for a symbol's own tab, whose listing draws the
                // section's own addresses; a placed one is another listing's.
                let Some(address) = take_planting(plant, &stop.document).and_then(Address::local)
                else {
                    return;
                };
                // The instruction holding the planted byte. An address before the listing's
                // first is a planting dropped rather than left.
                let Some(index) = data.assembly().instruction_at(address) else {
                    return;
                };
                let file = data.position(index).map(|at| at.file);
                land_row(
                    marked,
                    file,
                    data.lanes().row_of(index),
                    Owed::by(Pane::Assembly),
                );
            },
        );
        // The picked-out run is listing rows and the edges speak instructions;
        // `Studied::touching` crosses between the two. Base 0: a symbol read alone is
        // drawn from the listing's first row. A run that is one separator lights nothing.
        let touching = chars
            .map(|run| data.studied.touching(run.rows(), 0))
            .unwrap_or_default();

        // The bar's chords, the step it asks for and the listing's own keys, all of it
        // wired once (`use_listing_keys`). A separator copies as the blank line it is
        // drawn as, so a run lifted out of the listing keeps the blocks apart on the way
        // to the clipboard.
        let on_key_down = use_listing_keys(
            at,
            marked,
            // An assembly run's file is the row's own, so a run of the whole listing is a
            // run of no one file.
            None,
            &list,
            length,
            Some(data.searchable()),
            ListingText {
                line: Rc::new({
                    let (data, lanes) = (data.clone(), data.lanes().clone());
                    move |row| {
                        lanes
                            .instruction_at(row)
                            .and_then(|index| {
                                let instruction = data.assembly().instructions.get(index)?;
                                Some(asm_line(instruction, data.drawn_address(index)))
                            })
                            .unwrap_or_default()
                    }
                }),
                text: Rc::new({
                    let (assembly, lanes) = (data.assembly().clone(), data.lanes().clone());
                    move |row| {
                        lanes
                            .instruction_at(row)
                            .map(|index| instruction_line(&assembly, index))
                            .unwrap_or_default()
                    }
                }),
            },
        );

        list.use_rows(
            marked,
            length,
            on_key_down,
            AsmRows {
                data,
                asking,
                pair,
                touching,
                chars,
                marking,
            },
            asm_row,
        )
    }
}

/// One row of the listing: an instruction, or the separator above a row a branch lands
/// on. Its own function rather than a closure, `new_with_data` never comparing one: what
/// the rows are built from travels in [`AsmRows`] and nothing is captured here.
fn asm_row(i: usize, rows: &AsmRows) -> Element {
    let Some(index) = rows.data.lanes().instruction_at(i) else {
        // A separator, which belongs to the instruction below it.
        let below = rows.data.lanes().instruction_at(i + 1).unwrap_or(0);
        // Keyed by the row it opens, in a key space of its own: see `SeparatorRow`.
        let address = rows.data.assembly().instructions[below].address;
        return SeparatorRow::over(&rows.data, below, i, rows.chars, &rows.touching)
            .key((true, address))
            .into();
    };

    // Paired, and if so whether the rows either side are too: the listing's rows,
    // a separator being nobody's pair.
    let paired_at = |row: usize| {
        rows.data
            .lanes()
            .instruction_at(row)
            .is_some_and(|index| rows.data.paired(index, rows.pair.as_ref()))
    };
    let paired = paired_at(i).then(|| Edges::of(i, paired_at));
    InstructionRow::at(
        rows.data.clone(),
        rows.asking,
        index,
        i,
        paired,
        rows.chars,
        &rows.touching,
        rows.marking.clone(),
    )
    // Tagged, for the separators' sake: an address alone could be any
    // separator's too.
    .key((false, rows.data.assembly().instructions[index].address))
    .into()
}

/// The Assembly pane: a bar naming what is drawn over a dispatch over the things
/// [`Analyzed`] can be saying, and no work of its own.
///
/// It reads the analysis and not the active document for everything it draws, which keeps
/// the listing and the rows in step: while the worker is catching up the two disagree, and
/// it is the analysis that says which symbol is actually in hand. The one thing it asks
/// the document is the word for having been asked nothing, which differs by the kind of
/// tab -- a source-driven one is waiting for a line to be clicked in it.
#[derive(Clone, PartialEq)]
pub(crate) struct AssemblyPane {
    /// The tab this pane is in: what its bar's open-or-shut and its rows' positions are
    /// filed under.
    pub(crate) tab: DocId,
    pub(crate) document: Document,
}

impl AssemblyPane {
    /// What the bar over this pane names, which is what the pane itself is drawing.
    ///
    /// A tab that is a whole object, and one that is an object's code, are asked of no
    /// worker ([`ask`] answers `None` for both, and the hook then resets [`Analyzed`]), so
    /// there is never an analysis of either and the bar has to fall back to the document
    /// to name the object. Everything else is the symbol the pane is drawing.
    fn heading(&self, analysis: &Analyzed) -> Option<Heading> {
        match analysis.showing(&self.document) {
            // The extent comes from the listing being drawn and not from the crate: the bar
            // prints it in a render, and nothing analyses anything there.
            Showing::Listing(shown) => Some(Heading::Symbol {
                symbol: shown.studied.symbol.clone(),
                extent: shown.studied.extent(),
            }),
            _ => match &self.document {
                Document::Object(object) | Document::Code(object) => {
                    Some(Heading::Object(object.clone()))
                }
                _ => None,
            },
        }
    }

    /// Everything under the bar: the listing, or the word for why there is none.
    ///
    /// Its own function and not the body of `render`, because each of these answers is a
    /// return and a header cannot be drawn above a return.
    fn body(&self, analysis: &Analyzed) -> Element {
        // An object's code is its own listing, read in windows, and asks the analysis
        // nothing: `src/ui/section_view.rs`.
        if let Document::Code(object) = &self.document {
            return listing_inset(SectionList {
                place: Placing::Tab(self.tab),
                object: object.clone(),
            })
            .into();
        }
        let shown = match analysis.showing(&self.document) {
            Showing::Listing(shown) => shown,
            Showing::Message(text) => return placeholder_on(palette().asm_pane_bg, text),
            Showing::Nothing => return blank_pane(palette().asm_pane_bg),
        };
        // A listing that is one symbol: its rows start at the top, its addresses are the
        // file's own, its gutter is as wide as it needs, and it is not the code. None at
        // all for a symbol with nothing to decode.
        let data = AsmData::of(
            shown.studied.clone(),
            In::Alone {
                subject: match &shown.ask {
                    Ask::Source { at, .. } => Some(Subject {
                        tab: self.tab,
                        file: at.file.clone(),
                    }),
                    Ask::Symbol(_) => None,
                },
            },
        );
        let Some(data) = data else {
            return placeholder_on(palette().asm_pane_bg, "Assembly unavailable");
        };
        // An architecture no backend claims is a *third* answer -- the one above is only
        // "this symbol has no bytes" -- and it has to be said, an empty listing being
        // indistinguishable from a function that holds no code.
        if let Some(architecture) = data.assembly().undecodable {
            return placeholder_on(
                palette().asm_pane_bg,
                format!("No disassembler for {architecture}"),
            );
        }

        listing_inset(InstructionList {
            tab: self.tab,
            data,
            // The question the *drawn* answer answers, never the one being asked.
            asked: shown.ask.clone(),
            answered: analysis.answered.clone(),
        })
        .into()
    }
}

counter!(
    /// Test-only: how many times this thread has drawn the Assembly pane, the one pane
    /// every tab has.
    pub(crate) fn panes_drawn() = PANES_DRAWN
);

impl Component for AssemblyPane {
    fn render(&self) -> impl IntoElement {
        #[cfg(test)]
        PANES_DRAWN.set(PANES_DRAWN.get() + 1);

        let tab = self.tab;
        let bar = find_bar_over((Placing::Tab(tab), Pane::Assembly));
        // Read through a guard held for the render and never copied: `Analyzed` is a
        // whole answer. Nothing here writes it, and the children render after this returns.
        let analysis = use_consume::<Analysis>().0;
        let analysis = analysis.read();

        rect()
            .expanded()
            // The bar takes its own height and the listing is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().asm_pane_bg)
            .maybe_child(self.heading(&analysis).map(|heading| {
                SymbolBar {
                    heading,
                    tab,
                    // This pane leads in every tab but a source-driven one.
                    leading: self.document.driven_from() == Pane::Assembly,
                }
                .into_element()
            }))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(self.body(&analysis)),
            )
            // Last, so the listing above is given what is left: the code makes room for
            // the bar rather than being covered by it.
            .child(bar)
    }
}

#[cfg(test)]
mod tests;

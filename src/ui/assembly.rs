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

/// One instruction as one line of text, which is what a copy of the row has to be: the
/// address column, then the formatted instruction with the relocation target's name
/// substituted into its operand -- or appended, where the formatter offered no operand to
/// put it in. The gutter is left out, being a picture of the branches. `bias` is what the
/// listing adds to every address it draws (see [`AsmData::bias`]).
pub(crate) fn asm_line(instruction: &Instruction, bias: u64) -> String {
    let mut text = format!("{:016X} ", instruction.address.wrapping_add(bias));
    text.extend(instruction.format.iter().map(|(span, _)| span.as_str()));

    if instruction.relocation_span.is_none() {
        if let Some(target) = &instruction.relocation {
            text.push(' ');
            text.push_str(target.display());
        }
    }

    text.truncate(text.trim_end().len());
    text
}

/// Which element an instruction row draws in place of one of the formatter's spans.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Link {
    /// The relocation target's name, in the operand the relocation applies to.
    Relocation,
    /// A branch's displacement, where the listing has the row it lands on.
    Branch,
    /// The address an unnamed call or a branch this listing has no row for goes to: a
    /// door into the object's code at that address, opened with Ctrl and plain text
    /// without it.
    Target,
}

/// How an instruction's text is drawn after its address: the formatter's spans up to the
/// link, the link, and the spans after it. `linked` says whether its branch, if it has
/// one, is drawn as a link -- which is the listing's to know, being whether it has the
/// row the branch lands on. The spans are exclusive -- a branch whose displacement is a
/// relocation placeholder names no address of its own, and a target's span is a branch's
/// own where the row is a branch -- so there is at most one link, and an instruction with
/// no span has an empty tail.
pub(crate) fn split(
    instruction: &Instruction,
    linked: bool,
) -> (&[(String, SpanKind)], Option<Link>, &[(String, SpanKind)]) {
    let link = match instruction.relocation_span {
        Some(i) if instruction.relocation.is_some() => Some((i, Link::Relocation)),
        _ => match instruction.branch_span.filter(|_| linked) {
            Some(i) => Some((i, Link::Branch)),
            None => instruction
                .target_span
                .filter(|_| instruction.target.is_some())
                .map(|i| (i, Link::Target)),
        },
    };
    match link {
        Some((i, link)) if i < instruction.format.len() => (
            &instruction.format[..i],
            Some(link),
            &instruction.format[i + 1..],
        ),
        _ => (&instruction.format[..], None, &[][..]),
    }
}

/// Whether instruction `index`'s branch is drawn as a link: where the listing has the row
/// it lands on, which is the same set the gutter draws an arrow for.
pub(crate) fn linked(assembly: &Assembly, index: usize) -> bool {
    assembly.instructions[index].branch_span.is_some() && assembly.edge_from(index).is_some()
}

/// The text instruction `index`'s row draws after its address, as the clipboard sees it:
/// [`asm_line`] without the address column, the link as one inline piece. What the row
/// draws is built from the same [`split`], so a column into one is a column into the other.
pub(crate) fn instruction_line(assembly: &Assembly, index: usize) -> Line {
    let instruction = &assembly.instructions[index];
    let (head, link, tail) = split(instruction, linked(assembly, index));
    let mut line = Line::default();
    let push = |line: &mut Line, run: &[(String, SpanKind)]| {
        for (text, _) in run {
            line.push_text(text.clone());
        }
    };
    push(&mut line, head);
    match link {
        Some(Link::Relocation) => {
            if let Some(target) = &instruction.relocation {
                line.push_inline(target.display().to_owned());
            }
        }
        Some(Link::Branch) => {
            if let Some((text, _)) = instruction
                .branch_span
                .and_then(|i| instruction.format.get(i))
            {
                line.push_inline(text.clone());
            }
        }
        Some(Link::Target) => {
            if let Some((text, _)) = instruction
                .target_span
                .and_then(|i| instruction.format.get(i))
            {
                line.push_inline(text.clone());
            }
        }
        // The formatter offered no operand to put the name in: appended, as `asm_line`
        // appends it.
        None => {
            if let Some(target) = &instruction.relocation {
                line.push_text(" ");
                line.push_inline(target.display().to_owned());
            }
        }
    }
    push(&mut line, tail);
    // As `asm_line` ends: the formatter's padding after the last span is not text.
    if let Some(crate::chars::Piece::Text(last)) = line.pieces.last_mut() {
        last.truncate(last.trim_end().len());
    }
    line.pieces
        .retain(|piece| !matches!(piece, crate::chars::Piece::Text(text) if text.is_empty()));
    line
}

/// A disassembled symbol, where its branches are drawn and what says where its
/// instructions came from, compared by pointer.
#[derive(Clone)]
pub(crate) struct AsmData {
    pub(crate) assembly: Arc<Assembly>,
    pub(crate) object: Arc<Object>,
    /// The symbol the listing is of, for a row to name the tab it can be opened alone in.
    pub(crate) symbol: Arc<SymbolData>,
    /// The gutter layout for this symbol's branches, derived from `assembly` on the worker
    /// so it can never be a beat behind the rows it is drawn over.
    pub(crate) lanes: Arc<Lanes>,
    pub(crate) lines: SymbolLines,
    /// The source-driven tab this listing is the assembly side of and the file it is
    /// showing, or `None` for an assembly-driven tab's own listing. The file is compared
    /// by text, as `LinePos` is.
    pub(crate) subject: Option<(DocId, Arc<str>)>,
    /// The listing row this symbol's first instruction row is drawn at: 0 in a listing
    /// that is one symbol, and where the symbol starts in a listing of a whole object's
    /// code. What `lanes` answers in rows is relative to the symbol, and this is what the
    /// scroll and the picked-out run -- which speak the listing's rows -- have it added.
    pub(crate) base: usize,
    /// What is added to every address drawn or copied: 0 for a symbol
    /// read on its own, and the section's place in the object's layout
    /// (`Section::bias`) in a listing of all its code, where two functions of a
    /// relocatable object are both at 0 and have to be told apart.
    pub(crate) bias: u64,
    /// How many lanes the gutter is drawn with: the symbol's own on its own, and one width
    /// for every symbol in a listing of many, so the addresses start at one x.
    pub(crate) width: usize,
    /// Whether this listing is the object's code already, where a row has no
    /// neighbours to be shown among.
    pub(crate) code_tab: bool,
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.object, &other.object)
            && Arc::ptr_eq(&self.symbol, &other.symbol)
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
            && self.subject == other.subject
            && self.base == other.base
            && self.bias == other.bias
            && self.width == other.width
            && self.code_tab == other.code_tab
    }
}

impl AsmData {
    /// `address`, one of this listing's own, in the object's one address space: the
    /// section's place in the layout added (`Section::bias`), which is what a door into
    /// the object's code takes. Not [`bias`](Self::bias), which is what this listing
    /// *draws* and is nothing in a symbol's own listing, whose addresses are the file's.
    pub(crate) fn placed(&self, address: u64) -> u64 {
        let bias = self
            .symbol
            .section
            .as_ref()
            .map_or(0, |section| section.bias);
        address.wrapping_add(bias)
    }

    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    pub(crate) fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.info.as_ref()?;
        // `get` and not an index: a row's neighbour below can be past the listing.
        let row = lines.row_at(self.assembly.instructions.get(index)?.address)?;
        Some(LinePos {
            file: lines.files().get(row.file?)?.clone(),
            line: row.line?,
        })
    }

    /// Whether the instruction at `index` is the same place as a line of the source
    /// pane's picked-out run `pair`: compiled from that file, on one of those lines. One
    /// source line is many instructions and every one of them is lit, so this asks each
    /// row's own position rather than looking for the first match. An instruction the
    /// debug info places nowhere is never paired.
    pub(crate) fn paired(&self, index: usize, pair: Option<&Picked>) -> bool {
        let Some(pair) = pair else {
            return false;
        };
        let Some(at) = self.position(index) else {
            return false;
        };
        pair.file.as_ref() == Some(&at.file)
            && (at.line as usize)
                .checked_sub(1)
                .is_some_and(|row| pair.rows.contains(row))
    }
}

/// What the instruction rows are built from: the disassembly, the source pane's run --
/// whose pair the rows light -- and this pane's own, with the branches of the rows in
/// it. Kept apart from `AsmData` so that a selection cannot re-run anything the
/// disassembly drives.
#[derive(Clone, PartialEq)]
struct AsmRows {
    data: AsmData,
    /// The source pane's picked-out run, or `None` when there is none.
    pair: Option<Picked>,
    /// The edges starting or ending at a picked-out row, which every row the gutter
    /// draws them through has to know about. Worked out once here rather than per row.
    touching: Vec<PlacedEdge>,
    /// The run of rows picked out here, or `None` when there is none.
    rows: Option<RowSelection>,
    /// The characters picked out here, for each row to draw its part of.
    chars: Option<CharSelection>,
}

/// What one row draws in the gutter: its own lanes, and how much of it belongs to a branch
/// of a picked-out row.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RowArrows {
    pub(crate) lanes: RowLanes,
    pub(crate) lit: Lit,
}

/// The clickable name of a relocation target, in place of the meaningless numeric operand.
///
/// Where the listing is the object's own code, the target's rows are rows of this very
/// listing, so a plain press moves to them and the reader goes on reading where they
/// were; Ctrl opens the symbol alone, in a tab of its own as Ctrl does everywhere. In a
/// symbol's own listing there is nowhere to move to and a plain press follows the link
/// in place, the way a browser follows one.
#[derive(Clone)]
struct RelocationLabel {
    object: Arc<Object>,
    target: Arc<SymbolData>,
    /// Whether the listing this row is in is the object's code and not one symbol's.
    code_tab: bool,
}

impl PartialEq for RelocationLabel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && Arc::ptr_eq(&self.target, &other.target)
            && self.code_tab == other.code_tab
    }
}

impl Component for RelocationLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let ctrl = use_consume::<Ctrl>().0;
        let alt = use_consume::<Alt>().0;
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let code_at = use_consume::<CodeAt>().0;
        let symbol = Symbol {
            object: self.object.clone(),
            data: self.target.clone(),
        };
        let text = self.target.display().to_owned();
        let code_tab = self.code_tab;
        let object = self.object.clone();
        // Where the target is drawn in the object's own listing: its address plus where
        // the layout put its section, which is the one address space that listing draws.
        let placed = self.target.address.wrapping_add(
            self.target
                .section
                .as_ref()
                .map_or(0, |section| section.bias),
        );

        rect()
            .maybe(hovering(), |rect| {
                rect.background(palette().link_hover_bg)
                    .corner_radius(6.0)
                    .border(
                        Border::new()
                            .fill(palette().name_hover_fg)
                            .width(BorderWidth {
                                top: 0.0,
                                right: 0.0,
                                bottom: 2.0,
                                left: 0.0,
                            }),
                    )
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(move |e: Event<PressEventData>| {
                // Alt says this press is not a door: it is left to the row, whose
                // `pointer_down` has already begun a selection over the link.
                if *alt.peek() {
                    return;
                }
                // Or the press bubbles into the row, which would pin the line the
                // instruction being left came from.
                e.stop_propagation();

                // In the unified view the target is further down this same listing:
                // moved to, which is a scroll and a caret and neither a tab nor a visit,
                // since `land` plants an address in the tab that is already showing it.
                if code_tab && !*ctrl.peek() {
                    show_in_code(
                        open,
                        visits,
                        marked,
                        landing,
                        plant,
                        code_at,
                        object.clone(),
                        placed,
                        None,
                    );
                    return;
                }

                // A link inside the tab: followed in place, the way a browser follows
                // one, so the function left is one Back away -- or, with Ctrl, in a tab
                // of its own beside this one.
                let reach = if *ctrl.peek() {
                    Reach::NewTab
                } else {
                    Reach::InPlace
                };
                open_document(
                    open,
                    visits,
                    Document::Assembly(Selection::Symbol(symbol.clone())),
                    reach,
                );
            })
            .child(label().text(text).max_lines(1).color(if hovering() {
                palette().name_hover_fg
            } else {
                palette().name_fg
            }))
    }
}

/// The address an unnamed call, or a branch this listing has no row for, goes to: a
/// door into the object's code, in the unified view at that address, that opens with
/// **Ctrl** as a label's does in that view (`TextRow`) and is the plain number without it,
/// a press on which picks the row out like a press on any of the row's text. Lit as a
/// link only while Ctrl is held, which is when a press is one -- except in the unified
/// view, where the address is a row of this listing and a plain press moves to it, as
/// one on a named target does.
///
/// A tab of its own, as `show_in_code` opens one from the instruction's menu, landing on
/// the row at or below the address (`section::Rows::row_for`): a call into the middle of
/// a function lands on the instruction holding the byte, a target in a data stretch on
/// the row of bytes covering it. The line is left unknown, the target's row not being
/// this row.
#[derive(Clone)]
struct TargetLabel {
    /// The operand as the disassembler printed it, which is what a reader is clicking.
    text: String,
    object: Arc<Object>,
    /// Where the instruction goes, placed: in the object's one address space.
    address: u64,
    /// Whether the listing this row is in is the object's code and not one symbol's.
    code_tab: bool,
}

impl PartialEq for TargetLabel {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && Arc::ptr_eq(&self.object, &other.object)
            && self.address == other.address
            && self.code_tab == other.code_tab
    }
}

impl Component for TargetLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let ctrl = use_consume::<Ctrl>().0;
        let alt = use_consume::<Alt>().0;
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let marked = use_consume::<Marked>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let code_at = use_consume::<CodeAt>().0;
        let object = self.object.clone();
        let address = self.address;
        let text = self.text.clone();
        let code_tab = self.code_tab;
        // A link while Ctrl is held, and always in the listing the address is a row of:
        // the cue is the name's colour and the box a relocation link wears, since that
        // is the door it is.
        let link = hovering() && (ctrl() || code_tab);

        rect()
            .maybe(link, |rect| {
                rect.background(palette().link_hover_bg)
                    .corner_radius(6.0)
                    .border(
                        Border::new()
                            .fill(palette().name_hover_fg)
                            .width(BorderWidth {
                                top: 0.0,
                                right: 0.0,
                                bottom: 2.0,
                                left: 0.0,
                            }),
                    )
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(move |e: Event<PressEventData>| {
                // A plain press is a plain press: it goes on into the row, which
                // `pointer_down` has already picked out, and opens nothing -- except in
                // the unified view, where the row it goes to is one of this listing's
                // and moving to it is what the press is for. Alt says the same of every
                // press: not a door this time, so the selection begun over the link
                // stands.
                if (!code_tab && !*ctrl.peek()) || *alt.peek() {
                    return;
                }
                // Or the press bubbles into the row, which would pin the line the
                // instruction being left came from.
                e.stop_propagation();
                show_in_code(
                    open,
                    visits,
                    marked,
                    landing,
                    plant,
                    code_at,
                    object.clone(),
                    address,
                    None,
                );
            })
            .child(label().text(text).max_lines(1).color(if link {
                palette().name_hover_fg
            } else {
                kind_color(SpanKind::Address)
            }))
    }
}

/// The clickable displacement of a branch that lands inside this symbol: pressing it puts
/// the row it names on screen and pins the line that row came from.
///
/// **Not** a navigation. The document does not change, so nothing is pushed onto the
/// tab's trail -- following a jump is reading further down the same listing, and a Back
/// button that undid it would be answering a question nobody asked. It *is* a selection,
/// though:
/// arriving at the target and then having to click it to light it up made the reader say
/// twice where they had gone, so the press pins exactly what a press on the target row
/// would, source pane owed the scroll and all.
#[derive(Clone, PartialEq)]
struct BranchLabel {
    /// The operand as the disassembler printed it, which is what a reader is clicking.
    text: String,
    /// The listing row the branch lands on -- the instruction's row and not its index,
    /// since the scroll and the picked-out run are both in listing space.
    to: usize,
    /// Where that row points on the source side, or `None` where the debug info places it
    /// nowhere. The target's own position and not this row's: the run is the one a click
    /// on the row being jumped to would have made, of that row's file.
    at: Option<LinePos>,
    /// The listing's own scroll, and how tall it is: `reveal_row` needs both, and needs
    /// them at the moment of the press rather than at the render that drew this label.
    controller: ScrollController,
    viewport: State<f32>,
}

impl Component for BranchLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let alt = use_consume::<Alt>().0;
        let marked = use_consume::<Marked>().0;
        let text = self.text.clone();
        let to = self.to;
        let at = self.at.clone();
        let mut controller = self.controller;
        let viewport = self.viewport;

        rect()
            .maybe(hovering(), |rect| {
                rect.background(palette().link_hover_bg)
                    .corner_radius(6.0)
                    .border(
                        Border::new()
                            .fill(palette().branch_lit_fg)
                            .width(BorderWidth {
                                top: 0.0,
                                right: 0.0,
                                bottom: 2.0,
                                left: 0.0,
                            }),
                    )
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(move |e: Event<PressEventData>| {
                // Alt says this press is not a door: left to the row, which is already
                // sweeping a selection out over the link.
                if *alt.peek() {
                    return;
                }
                // Or the press bubbles into the row, which would keep the row *this*
                // instruction is picked out, where the reader asked for the one it
                // jumps to.
                e.stop_propagation();
                // The row is reached by a press, so the pane is on screen and measured.
                let _ = reveal_row(&mut controller, *viewport.peek(), to);
                // The row landed on becomes the picked-out one, replacing the row the
                // press started on -- which `pointer_down` has already marked, that
                // being the one handler a stopped press does not undo. The source
                // pane owes the scroll to the target's line, where it has one; this
                // pane has just been given its own, above.
                mark_row(marked, at.as_ref().map(|at| at.file.clone()), to);
            })
            .child(label().text(text).max_lines(1).color(if hovering() {
                palette().branch_lit_fg
            } else {
                kind_color(SpanKind::Address)
            }))
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

/// How wide a gutter of `width` lanes is drawn, for a row that has no gutter and wants
/// its address column to start where an instruction row's does.
pub(crate) fn gutter_width(width: usize) -> f32 {
    if width == 0 {
        return 0.0;
    }
    let grid = pixel_grid();
    grid.edge(grid.edge(width as f32 * LANE_WIDTH + ARROW_WIDTH) + GUTTER_PAD)
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
    /// The listing's widest row and its key, as `InstructionRow` carries them: the rule
    /// runs the width of the widest row, so the listing does not read as ending early.
    pub(crate) widest: Widest,
    pub(crate) listing: u64,
    pub(crate) key: DiffKey,
}

impl KeyExt for SeparatorRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
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
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row: self.row,
                file: None,
                paired: None,
                wash: self.wash,
                widest: self.widest,
                listing: self.listing,
                measured: false,
            },
            std::iter::once(code_mark(false))
                .chain((width > 0).then(|| gutter(width, self.arrows).into_element()))
                .collect(),
            None,
            None,
        )
        .child(block_rule())
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
pub(crate) struct InstructionRow {
    pub(crate) data: AsmData,
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
    /// The listing's scroll and its height, for a branch operand to scroll to the row it
    /// names. Out of the `PartialEq` below: both are the list's own handles and neither
    /// changes while it lives.
    pub(crate) controller: ScrollController,
    pub(crate) viewport: State<f32>,
    /// The widest row the listing has drawn, for this row to be no narrower than. Out of
    /// the `PartialEq` for the same reason as the two above: a row asks the state itself.
    /// See `ui/width.rs`.
    pub(crate) widest: Widest,
    /// The listing's key in that state, which the row is floored under and reports
    /// itself under. **Compared**, unlike the handle: the key holds the fixed-width
    /// font's size, and a row left with the old one goes on asking for the width the
    /// larger font measured.
    pub(crate) listing: u64,
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
    pub(crate) key: DiffKey,
}

impl PartialEq for InstructionRow {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
            && self.index == other.index
            && self.row == other.row
            && self.paired == other.paired
            && self.wash == other.wash
            && self.chars == other.chars
            && self.arrows == other.arrows
            && self.listing == other.listing
    }
}

impl KeyExt for InstructionRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for InstructionRow {
    fn render(&self) -> impl IntoElement {
        // Consumed here, in the render, because the menu handler may not run a hook.
        let marked = use_consume::<Marked>().0;
        let located = use_consume::<Locations>().0;
        let dock = use_consume::<SidebarDock>().0;
        let open = use_open();
        let visits = use_consume::<Visited>().0;
        let landing = use_consume::<Land>().0;
        let plant = use_consume::<Plant>().0;
        let code_at = use_consume::<CodeAt>().0;
        let bookmarked = use_consume::<Bookmarked>().0;
        let objects = use_consume::<Objects>().0;
        // The source-driven tab this listing is the assembly side of, if it is one: a
        // location found from it is chosen for it.
        let subject = self.data.subject.clone();
        // The symbol this row is code of, in either listing: what the menu bookmarks.
        let symbol_document = Document::Assembly(Selection::Symbol(Symbol {
            object: self.data.object.clone(),
            data: self.data.symbol.clone(),
        }));
        let row = self.row;
        let width = self.data.width;
        let instruction = &self.data.assembly.instructions[self.index];
        // The address as the listing draws it: the symbol's own, plus where the listing
        // has placed the symbol's section.
        let address = instruction.address.wrapping_add(self.data.bias);
        // The row's door into the object's code, unless this listing is that already --
        // and from there, the door back to the symbol read alone. The door takes the
        // placed address, which in a symbol's own listing is not the one drawn.
        let neighbours = (!self.data.code_tab).then(|| {
            (
                self.data.object.clone(),
                self.data.placed(instruction.address),
            )
        });
        // The door back takes the symbol's own address, the space its listing draws.
        // Wherever this listing is not the tab itself, which is an object's code and the
        // assembly side of a source-driven tab: in the second the symbol has no other
        // door, the Symbols list aside, since the tab is a file. An assembly-driven tab
        // is the symbol already and gets none.
        let alone = (self.data.code_tab || self.data.subject.is_some()).then(|| {
            (
                Symbol {
                    object: self.data.object.clone(),
                    data: self.data.symbol.clone(),
                },
                instruction.address,
            )
        });

        // Where this row points on the source side. Worked out once here rather than in
        // each of the handlers, which all need the same answer.
        let at = self.data.position(self.index);

        // The disassembler says which span the link replaced, so the text is three
        // parts: the spans before that span, the link, and the spans after it. That
        // keeps the link in the operand's own position, inside the brackets of a memory
        // operand and after the `rip+` of a rip-relative one. The link is an inline child
        // of the row's one paragraph, so it is one unit of the row's text to the engine
        // and the row's columns are the clipboard's (`instruction_line`).
        let (head, link, tail) = split(instruction, linked(&self.data.assembly, self.index));
        let inline: Option<Element> = match link {
            Some(Link::Relocation) => instruction.relocation.as_ref().map(|target| {
                RelocationLabel {
                    object: self.data.object.clone(),
                    target: target.clone(),
                    code_tab: self.data.code_tab,
                }
                .into_element()
            }),
            // A branch's displacement is the other way to follow it: the row it lands
            // on, and the run a press on that row would have made.
            Some(Link::Branch) => {
                let edge = self.data.assembly.edge_from(self.index);
                let span = instruction
                    .branch_span
                    .and_then(|i| instruction.format.get(i));
                edge.zip(span).map(|(edge, (text, _))| {
                    BranchLabel {
                        text: text.clone(),
                        to: self.data.base + self.data.lanes.row_of(edge.to),
                        at: self.data.position(edge.to),
                        controller: self.controller,
                        viewport: self.viewport,
                    }
                    .into_element()
                })
            }
            // Where the instruction goes, with no name and no row here: the door into the
            // object's code at that address, in either listing -- the unified view's own
            // rows included, where the target may be screens away.
            Some(Link::Target) => {
                let span = instruction
                    .target_span
                    .and_then(|i| instruction.format.get(i));
                instruction.target.zip(span).map(|(target, (text, _))| {
                    TargetLabel {
                        text: text.clone(),
                        object: self.data.object.clone(),
                        address: self.data.placed(target),
                        code_tab: self.data.code_tab,
                    }
                    .into_element()
                })
            }
            // The formatter offered no operand to put the name in: appended.
            None => instruction.relocation.as_ref().map(|target| {
                RelocationLabel {
                    object: self.data.object.clone(),
                    target: target.clone(),
                    code_tab: self.data.code_tab,
                }
                .into_element()
            }),
        };
        let appended = link.is_none() && inline.is_some();

        // Whatever text runs up to the link ends in the formatter's padding to the
        // operand column, and Skia trims trailing whitespace when it measures a
        // paragraph — which would butt the name right up against the mnemonic. Make
        // that padding non-breaking to keep the column; one unit each way, so the
        // columns still agree with the plain text.
        let spans = |run: &[(String, SpanKind)], pad_end: bool| {
            let last = run.len().saturating_sub(1);
            run.iter()
                .enumerate()
                .map(|(i, (text, kind))| {
                    let text = if pad_end && i == last {
                        let kept = text.trim_end_matches(' ');
                        format!("{kept}{}", "\u{a0}".repeat(text.len() - kept.len()))
                    } else {
                        text.clone()
                    };

                    Span::new(text)
                        .color(kind_color(*kind))
                        .assembly_font()
                        .font_weight(if *kind == SpanKind::Mnemonic {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                })
                .collect::<Vec<_>>()
        };
        let mut head = spans(head, link.is_some() || appended);
        if appended {
            // The space `asm_line` puts before an appended name, non-breaking for the
            // reason above.
            head.push(
                Span::new("\u{a0}")
                    .color(kind_color(SpanKind::Other))
                    .assembly_font(),
            );
        }
        let text = Text {
            line: instruction_line(&self.data.assembly, self.index),
            head,
            inline,
            tail: spans(tail, false),
            chars: self.chars,
            door: link == Some(Link::Target),
            links: Vec::new(),
            on_link: None,
        };

        // The menu: the line's locations, where the debug info gives the row a line; the
        // row shown among its neighbours, where it is not already; and the symbol
        // bookmarked, always.
        let menu: Rc<dyn Fn(Event<PressEventData>, Option<usize>)> = Rc::new({
            let at = at.clone();
            // The column is the source pane's business: nothing in an instruction row is
            // a name a server could be asked about.
            move |e: Event<PressEventData>, _| {
                let menu = match &at {
                    Some(at) => {
                        locate_menu(located, dock, at.clone(), subject.clone(), None, Vec::new())
                    }
                    None => Menu::new(),
                };
                let menu = menu.maybe_child(neighbours.clone().map(|(object, address)| {
                    let at = at.clone();
                    MenuButton::new()
                        .on_press(move |_| {
                            show_in_code(
                                open,
                                visits,
                                marked,
                                landing,
                                plant,
                                code_at,
                                object.clone(),
                                address,
                                at.clone(),
                            )
                        })
                        .child("Show in unified view")
                }));
                let menu = menu.maybe_child(alone.clone().map(|(symbol, address)| {
                    let at = at.clone();
                    MenuButton::new()
                        .on_press(move |_| {
                            open_as_symbol(
                                open,
                                visits,
                                marked,
                                landing,
                                plant,
                                symbol.clone(),
                                address,
                                at.clone(),
                            )
                        })
                        .child("Open as symbol")
                }));
                let menu = menu.child(bookmark_item(
                    bookmarked,
                    objects,
                    symbol_document.clone(),
                    "Bookmark symbol",
                ));
                ContextMenu::open_from_event(&e, menu);
            }
        });

        // Before the text: the mark, saying whether the debug info places this
        // instruction anywhere at all; the gutter -- nothing at all for a symbol that
        // branches nowhere inside itself, which most do, since an empty column would
        // still be a column, and why the mark cannot sit inside it; and the address,
        // which is gutter too: a press on it picks the row out and no characters.
        let before = std::iter::once(code_mark(at.is_some()))
            .chain((width > 0).then(|| gutter(width, self.arrows).into_element()))
            .chain([label()
                .text(format!("{address:016X} "))
                .min_width(Size::px(200.0))
                .color(palette().address_fg)
                .max_lines(1)
                .into_element()])
            .collect();

        // The run is a run of the file this row was compiled from, which is what the
        // source pane shows beside an object's code.
        code_row(
            Chrome {
                pane: Pane::Assembly,
                row,
                file: at.as_ref().map(|at| at.file.clone()),
                paired: self.paired,
                wash: self.wash,
                widest: self.widest,
                listing: self.listing,
                measured: true,
            },
            before,
            Some(text),
            Some(menu),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The instruction rows themselves, a component of their own so that the pointer focus and
/// the picked-out run -- which change on every pointer move across a row boundary -- do not
/// re-render the pane above, which changes only when a symbol is analysed.
#[derive(Clone)]
struct InstructionList {
    /// The tab these rows are in.
    tab: DocId,
    assembly: Arc<Assembly>,
    /// The whole symbol and not just its object, because these rows draw a disassembly
    /// *and* answer to it -- a relocation label navigates to a symbol in the same object.
    symbol: Symbol,
    /// The question this listing answers, and **not** the one being asked: while the
    /// worker catches up the pane is still drawing the listing being left. Two things
    /// come out of it -- [`asked_of`], the place on the tab's trail whose viewing
    /// position this is (the file for a source-driven one, never the resolved symbol,
    /// which is very likely on no trail at all), and the file a source-driven tab is
    /// about, which its rows' menus choose a location for.
    asked: Ask,
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab
            && Arc::ptr_eq(&self.assembly, &other.assembly)
            && self.symbol == other.symbol
            && self.asked == other.asked
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Component for InstructionList {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Assembly);
        let chars = chars_of(marked, Pane::Assembly);
        // The source pane's run, whose pair these rows light.
        let pair = pair_of(marked, Pane::Assembly);
        // The box the keyboard reaches this pane through: a `pointer_down` anywhere inside
        // it bubbles to here and asks for focus, which is what makes Ctrl+C mean this
        // listing.
        let a11y = use_a11y();
        use_tab_keyboard(a11y);

        let controller = use_scroll_controller(ScrollConfig::default);
        // How tall the list is, which `reveal_row` needs to know whether the row it was
        // asked for is on screen already. `VirtualScrollView` measures itself but keeps
        // the answer, so the rect wrapping it is measured here instead.
        let mut viewport = use_state(|| 0.0f32);
        // The widest row drawn, under this disassembly's identity: what every row is at
        // least as wide as, so the list scrolls sideways over a stable extent.
        let widest = use_widest();
        let listing = Widest::key(Arc::as_ptr(&self.assembly).addr());

        let data = AsmData {
            assembly: self.assembly.clone(),
            object: self.symbol.object.clone(),
            symbol: self.symbol.data.clone(),
            lanes: self.lanes.clone(),
            lines: self.lines.clone(),
            subject: match &self.asked {
                Ask::Source { at, .. } => Some((self.tab, at.file.clone())),
                Ask::Symbol(_) => None,
            },
            // A listing that is one symbol: its rows start at the top, its addresses
            // are the file's own, and its gutter is as wide as it needs.
            base: 0,
            bias: 0,
            width: self.lanes.width,
            code_tab: false,
        };
        // The listing's rows, which is the instructions plus a separator above every row a
        // branch lands on. Everything below that scrolls, picks out or counts rows is in
        // this space; `AsmData::position`, the gutter and the edges are in the
        // instructions'. `Lanes` converts, and is the only thing that may.
        let length = data.lanes.listing_rows(data.assembly.instructions.len());
        // Where this tab was left, put back when it is switched to and written down as it
        // is scrolled -- and the scroll this pane owes a run, which wins over it.
        let docs = use_consume::<OpenDocs>().0;
        // The place the tab is at, which for a source-driven tab is a line of the file
        // and not the file: two lines of one file reached along one trail are two
        // entries, each with its own scroll. Read and not peeked, so a step between them
        // re-renders this pane.
        let entry = (
            self.tab,
            place_at(&docs.read(), self.tab, &asked_of(&self.asked)),
        );
        use_kept_position(
            use_consume::<AsmAt>().0,
            move |(tab, stop): &Entry| docs.peek().contains(*tab, stop),
            {
                let data = data.clone();
                move |controller: &mut ScrollController| {
                    let row = match owed_reveal(marked, Pane::Assembly) {
                        None => return false,
                        Some(Owing::Own(rows)) => *rows.rows().start(),
                        // The first instruction compiled from a line of the source
                        // pane's run. Nothing at all when the lines produced no
                        // instruction here -- ones the optimiser folded away, or
                        // belonging to another function, or, in a source-driven tab,
                        // the listing this very click is asking for not having arrived
                        // yet. Scrolling somewhere arbitrary would be worse than not
                        // scrolling, and **the request is left owed**, so the listing
                        // that can answer it still finds it.
                        Some(Owing::Pair(pair)) => {
                            let Some(index) = (0..data.assembly.instructions.len())
                                .find(|&index| data.paired(index, Some(&pair)))
                            else {
                                return false;
                            };
                            data.lanes.row_of(index)
                        }
                    };
                    if !reveal_row(controller, *viewport.read(), row) {
                        return false;
                    }
                    reveal_made(marked, Pane::Assembly);
                    true
                }
            },
            // A landing's half for this pane is an address, and an address is a row of a
            // listing that arrives later than the document: it is left as a `Planting`
            // and spent below, never taken here.
            |_: &Landing, _: &mut ScrollController| false,
            controller,
            &entry,
            length,
            listing,
            // The top: a listing *is* the symbol, so its first row is its own first line.
            0,
        );
        // The caret a door left to be planted on an instruction of this listing, once the
        // listing is the document it names -- which is the drawn answer's document and
        // not the tab's, since the pane draws the listing being left until the worker
        // answers. On the row of the instruction at or below the address, the symbol's
        // own; an address before the first is dropped, not left. The planting is read
        // and not peeked, being written a beat after the pane mounts: `use_land` leaves
        // it as the document arrives, and `land` leaves it for a tab already on top. The
        // pane owes the caret its reveal, as it owes a click from outside, and the reveal
        // wins over the kept row in `use_kept_position`, as a reveal does.
        let plant = use_consume::<Plant>().0;
        use_side_effect_with_deps(&entry, {
            let data = data.clone();
            let mut plant = plant;
            move |(_, stop): &Entry| {
                let planting = plant.read().clone();
                let Some(planting) = planting.filter(|planting| planting.tab == stop.document)
                else {
                    return;
                };
                plant.set(None);
                let after = data
                    .assembly
                    .instructions
                    .partition_point(|instruction| instruction.address <= planting.address);
                let Some(index) = after.checked_sub(1) else {
                    return;
                };
                let file = data.position(index).map(|at| at.file);
                land_row(
                    marked,
                    file,
                    data.lanes.row_of(index),
                    Owed::by(Pane::Assembly),
                );
            }
        });
        // The picked-out run is listing rows, and `touching` speaks instructions: a run
        // that is one separator lights nothing.
        let touching = rows
            .and_then(|run| data.lanes.instructions_in(run.rows()))
            .map(|indices| data.lanes.touching_any(indices))
            .unwrap_or_default();

        let on_key_down = {
            let assembly = self.assembly.clone();
            let lanes = self.lanes.clone();
            let (text_assembly, text_lanes) = (assembly.clone(), lanes.clone());
            // A separator copies as the blank line it is drawn as, so a run lifted out of
            // the listing keeps the blocks apart on the way to the clipboard.
            let mut controller = controller;
            on_listing_key(
                marked,
                Pane::Assembly,
                // An assembly run's file is the row's own, so a run of the whole
                // listing is a run of no one file.
                None,
                length,
                viewport,
                move |row| {
                    lanes
                        .instruction_at(row)
                        .and_then(|index| assembly.instructions.get(index))
                        .map(|instruction| asm_line(instruction, 0))
                        .unwrap_or_default()
                },
                move |row| {
                    text_lanes
                        .instruction_at(row)
                        .filter(|&index| index < text_assembly.instructions.len())
                        .map(|index| instruction_line(&text_assembly, index))
                        .unwrap_or_default()
                },
                // The caret's row, brought on screen after a key has moved it.
                move |row| reveal_caret(&mut controller, *viewport.peek(), code_row_height(), row),
            )
        };

        let nudge = use_nudge();
        let grid = pixel_grid();
        // The list as its rows and a sweep past its edge know it: its scroll, its box,
        // the paragraphs the rows lend it, and its widest row.
        let listing_ctx = use_provide_context(|| Listing::new(controller, widest));
        let bounds = listing_ctx.bounds.clone();

        rect()
            .expanded()
            .a11y_id(a11y)
            .a11y_focusable(true)
            .on_pointer_down(move |_| a11y.request_focus())
            .on_key_down(on_key_down)
            .on_sized({
                let bounds = bounds.clone();
                move |e: Event<SizedEventData>| {
                    viewport.set_if_modified(e.area.height());
                    nudge.measured(grid, e.area.min_y());
                    bounds.set(e.area);
                }
            })
            .on_global_pointer_move(use_sweep_beyond(
                marked,
                Pane::Assembly,
                listing_ctx.clone(),
                nudge,
                length,
                listing,
            ))
            // On the grid: see `Nudge`.
            .padding(nudge.padding())
            .child(
                VirtualScrollView::new_with_data_controlled(
                    AsmRows {
                        data,
                        pair,
                        touching,
                        rows,
                        chars,
                    },
                    move |i, rows: &AsmRows| {
                        let wash = wash_of(rows.chars, i);
                        let Some(index) = rows.data.lanes.instruction_at(i) else {
                            // A separator, which belongs to the instruction below it: the
                            // lanes it carries are that row's, and it lights with them
                            // but never draws their corner.
                            let below = rows.data.lanes.instruction_at(i + 1).unwrap_or(0);
                            let mut lit = lanes::lit(&rows.touching, below);
                            lit.corner = false;

                            // Keyed by the row it opens, in a key space of its own:
                            // see `SeparatorRow`.
                            let address = rows.data.assembly.instructions[below].address;
                            return SeparatorRow {
                                row: i,
                                wash,
                                width: rows.data.width,
                                arrows: RowArrows {
                                    lanes: rows.data.lanes.boundary(below),
                                    lit,
                                },
                                widest,
                                listing,
                                key: DiffKey::None,
                            }
                            .key((true, address))
                            .into();
                        };

                        // Paired, and if so whether the rows either side are too:
                        // the listing's rows, a separator being nobody's pair.
                        let paired_at = |row: usize| {
                            rows.data
                                .lanes
                                .instruction_at(row)
                                .is_some_and(|index| rows.data.paired(index, rows.pair.as_ref()))
                        };
                        let paired = paired_at(i).then(|| Edges::of(i, paired_at));
                        InstructionRow {
                            paired,
                            data: rows.data.clone(),
                            index,
                            row: i,
                            wash,
                            chars: RowChars::of(rows.chars, i),
                            arrows: RowArrows {
                                lanes: rows.data.lanes.row(index),
                                lit: lanes::lit(&rows.touching, index),
                            },
                            controller,
                            viewport,
                            widest,
                            listing,
                            key: DiffKey::None,
                        }
                        // Tagged, for the separators' sake: an address alone could be
                        // any separator's too.
                        .key((false, rows.data.assembly.instructions[index].address))
                        .into()
                    },
                    controller,
                )
                .length(length)
                .item_size(code_row_height()),
            )
    }
}

/// The Assembly pane: a bar naming what is drawn over a dispatch over the things
/// [`Analyzed`] can be saying, and no work of its own.
///
/// It reads the analysis and not the active document for everything it draws, which keeps
/// the listing and the rows in step: while the worker is catching up the two disagree, and
/// it is the analysis that says which symbol is actually in hand. The one thing it asks
/// the document is the word for having been asked nothing, which differs by the kind of
/// tab -- a source-driven one is waiting for a line to be clicked in it.
#[derive(Clone)]
pub(crate) struct AssemblyPane {
    /// The tab this pane is in: what its bar's open-or-shut and its rows' positions are
    /// filed under.
    pub(crate) tab: DocId,
    pub(crate) document: Document,
}

impl PartialEq for AssemblyPane {
    fn eq(&self, other: &Self) -> bool {
        self.tab == other.tab && self.document == other.document
    }
}

impl AssemblyPane {
    /// What the bar over this pane names, which is what the pane itself is drawing -- and
    /// for a tab that is a whole object, the object, that being the one selection no
    /// listing is ever worked out for.
    fn named(&self, analysis: &Analyzed) -> Option<Named> {
        match analysis.showing(&self.document) {
            Showing::Listing(shown) => Some(Named::Symbol(shown.studied.symbol.clone())),
            _ => match &self.document {
                Document::Assembly(Selection::Object(object)) | Document::Code(object) => {
                    Some(Named::Object(object.clone()))
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
            return rect()
                .expanded()
                .padding(5.0)
                .child(SectionList {
                    tab: self.tab,
                    document: self.document.clone(),
                    object: object.clone(),
                })
                .into();
        }
        let shown = match analysis.showing(&self.document) {
            Showing::Listing(shown) => shown,
            Showing::Message(text) => return placeholder(text),
            Showing::Nothing => return rect().expanded().background(palette().asm_pane_bg).into(),
        };
        let studied = &shown.studied;
        let Some(assembly) = studied.assembly.clone() else {
            return rect()
                .padding(5.0)
                .child(label().text("Assembly unavailable"))
                .into();
        };
        // An architecture no backend claims is a *third* answer -- the one above is only
        // "this symbol has no bytes" -- and it has to be said, an empty listing being
        // indistinguishable from a function that holds no code.
        if let Some(architecture) = assembly.undecodable {
            return placeholder(format!("No disassembler for {architecture}"));
        }

        rect()
            .expanded()
            // The listing's own inset, which is on it and not on the pane: the bar above
            // runs the full width of the pane the way a header does.
            .padding(5.0)
            .child(InstructionList {
                tab: self.tab,
                assembly,
                symbol: studied.symbol.clone(),
                // The question the *drawn* answer answers, never the one being asked.
                asked: shown.ask.clone(),
                lanes: studied.lanes.clone(),
                lines: studied.lines.clone(),
            })
            .into()
    }
}

impl Component for AssemblyPane {
    fn render(&self) -> impl IntoElement {
        let analysis = use_consume::<Analysis>().0.read().clone();
        let tab = self.tab;

        rect()
            .expanded()
            // The bar takes its own height and the listing is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().asm_pane_bg)
            .maybe_child(self.named(&analysis).map(|named| {
                SymbolBar {
                    named,
                    tab,
                    // This pane leads in every tab but a source-driven one.
                    leading: !matches!(self.document, Document::Source(_)),
                }
                .into_element()
            }))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .child(self.body(&analysis)),
            )
    }
}

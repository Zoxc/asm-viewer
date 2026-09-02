//! The assembly half of a document, from the row up: what a row is drawn out of, the
//! branch gutter, the two operands a row can make a link of -- a relocation target's name
//! and a branch's own displacement -- the virtual list of instructions and the pane
//! holding it.
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
/// put it in. The gutter is left out, being a picture of the branches.
fn asm_line(instruction: &Instruction) -> String {
    let mut text = format!("{:016X} ", instruction.address);
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

/// A disassembled symbol, where its branches are drawn and what says where its
/// instructions came from, compared by pointer.
#[derive(Clone)]
struct AsmData {
    assembly: Arc<Assembly>,
    object: Arc<Object>,
    /// The gutter layout for this symbol's branches, derived from `assembly` on the worker
    /// so it can never be a beat behind the rows it is drawn over.
    lanes: Arc<Lanes>,
    lines: SymbolLines,
    /// The file of the source-driven tab this listing is the assembly side of, or `None`
    /// for an assembly-driven tab's own listing. Compared by text, as `LinePos` is.
    subject: Option<Arc<str>>,
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.object, &other.object)
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
            && self.subject == other.subject
    }
}

impl AsmData {
    /// The source position the instruction at `index` was compiled from, or `None` where
    /// the debug info gives it none: no line info at all, an address no row covers, or a
    /// row naming no file or sitting on DWARF's line 0.
    fn position(&self, index: usize) -> Option<LinePos> {
        let lines = self.lines.info.as_ref()?;
        let row = lines.row_at(self.assembly.instructions[index].address)?;
        Some(LinePos {
            file: lines.files().get(row.file?)?.clone(),
            line: row.line?,
        })
    }
}

/// What the instruction rows are built from: the disassembly, the two positions the source
/// pane is pointing at, and the branches of the row the pointer is on. Kept apart from
/// `AsmData` so that a hover cannot re-run anything the disassembly drives.
#[derive(Clone, PartialEq)]
struct AsmRows {
    data: AsmData,
    focus: Option<LinePos>,
    pin: Option<LinePos>,
    /// The edges starting or ending at the hovered row, which every row the gutter draws
    /// them through has to know about. Worked out once here rather than per row.
    touching: Vec<PlacedEdge>,
    /// The run of rows picked out to be copied, or `None` when the selection is the source
    /// pane's or there is none.
    rows: Option<RowSelection>,
}

/// What one row draws in the gutter: its own lanes, and how much of it belongs to a branch
/// of the row under the pointer.
#[derive(Clone, Copy, PartialEq)]
struct RowArrows {
    lanes: RowLanes,
    lit: Lit,
}

impl AsmRows {
    /// Whether the instruction at `index` is what the pointer is on in the source pane,
    /// and whether it is what a click pinned there. One source line is many instructions
    /// and every one of them lights up, so this asks each row's own position rather than
    /// looking for the first match.
    ///
    /// An instruction the debug info places nowhere is neither, which `Option`'s own `==`
    /// would get wrong in the case where nothing is focused either.
    fn lit(&self, index: usize) -> (bool, bool) {
        let Some(at) = self.data.position(index) else {
            return (false, false);
        };
        (
            self.focus.as_ref() == Some(&at),
            self.pin.as_ref() == Some(&at),
        )
    }
}

/// The clickable name of a relocation target, in place of the meaningless numeric operand.
#[derive(Clone)]
struct RelocationLabel {
    object: Arc<Object>,
    target: Arc<SymbolData>,
}

impl PartialEq for RelocationLabel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && Arc::ptr_eq(&self.target, &other.target)
    }
}

impl Component for RelocationLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let open = use_open();
        let history = use_consume::<Hist>().0;
        let symbol = Symbol {
            object: self.object.clone(),
            data: self.target.clone(),
        };
        let text = self.target.display().to_owned();

        CursorArea::new().child(
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
                    // Or the press bubbles into the row, which would pin the line the
                    // instruction being left came from.
                    e.stop_propagation();

                    activate(
                        open,
                        history,
                        Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
                        Visit::Went,
                    );
                })
                .child(label().text(text).max_lines(1).color(if hovering() {
                    palette().name_hover_fg
                } else {
                    palette().name_fg
                })),
        )
    }
}

/// The clickable displacement of a branch that lands inside this symbol: pressing it puts
/// the row it names on screen and pins the line that row came from.
///
/// **Not** a navigation. The document does not change, so nothing is pushed onto the
/// history -- following a jump is reading further down the same listing, and a Back button
/// that undid it would be answering a question nobody asked. It *is* a selection, though:
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
    /// nowhere. The target's own position and not this row's: the pin is the one a click
    /// on the row being jumped to would have made.
    at: Option<LinePos>,
    /// The listing's own scroll, and how tall it is: `reveal_row` needs both, and needs
    /// them at the moment of the press rather than at the render that drew this label.
    controller: ScrollController,
    viewport: State<f32>,
}

impl Component for BranchLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let text = self.text.clone();
        let to = self.to;
        let at = self.at.clone();
        let mut controller = self.controller;
        let viewport = self.viewport;

        CursorArea::new().child(
            rect()
                .maybe(hovering(), |rect| {
                    rect.background(palette().link_hover_bg)
                        .corner_radius(6.0)
                        .border(
                            Border::new()
                                .fill(palette().branch_hover_fg)
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
                    // Or the press bubbles into the row and pins the line *this*
                    // instruction came from, where the reader asked for the one it jumps
                    // to.
                    e.stop_propagation();
                    reveal_row(&mut controller, *viewport.peek(), to);
                    // The row landed on becomes the picked-out one, replacing the row the
                    // press started on -- which `pointer_down` has already marked, that
                    // being the one handler a stopped press does not undo. This is the
                    // half of "the selection follows the jump" that holds for a binary
                    // with no line info at all, where the pin below has nothing to say.
                    mark_row(marked, Pane::Assembly, to);
                    // The same rule the row itself obeys: a target the debug info places
                    // nowhere pins nothing rather than clearing what is pinned, so a jump
                    // into a prologue is not a way of losing the line the reader put
                    // there. The Assembly pane is not owed the scroll -- it has just been
                    // given one, above.
                    if let Some(at) = at.clone() {
                        pinned.set(Some(Pin {
                            at,
                            reveal: Owed::by(Pane::Source),
                            landed: false,
                        }));
                    }
                })
                .child(label().text(text).max_lines(1).color(if hovering() {
                    palette().branch_hover_fg
                } else {
                    kind_color(SpanKind::Address)
                })),
        )
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
    // lit exactly when a branch of the hovered row has an end in this one.
    let lit = arrows.lit.corner;

    let stroke = move |left: f32, top: f32, wide: f32, tall: f32, lit: bool| {
        rect()
            .position(Position::new_absolute().left(left).top(top))
            .width(Size::px(wide))
            .height(Size::px(tall))
            .background(if lit {
                palette().branch_hover_fg
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
fn block_rule() -> impl IntoElement {
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
struct SeparatorRow {
    /// The listing row this is, for the picked-out run: a sweep that crosses a boundary
    /// must not stop tracking the pointer, and a copy takes the blank line it draws.
    row: usize,
    /// Whether it is inside the picked-out run.
    selected: bool,
    /// The gutter's width for the whole symbol, and the lanes crossing this boundary.
    width: usize,
    arrows: RowArrows,
    key: DiffKey,
}

impl KeyExt for SeparatorRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SeparatorRow {
    fn render(&self) -> impl IntoElement {
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let row = self.row;
        let width = self.width;

        rect()
            .horizontal()
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            // The same horizontal padding the instruction rows take. Without it the
            // gutter's lines step three pixels sideways at every boundary they cross.
            .padding(Gaps::new_symmetric(0.0, 3.0))
            .background(row_background(false, false, false, self.selected))
            // The same two handlers the instruction rows carry, so a sweep down the
            // listing is not cut in half by every boundary it crosses.
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Assembly, row);
                }
            })
            .on_pointer_over(move |_| mark_drag(marked, Pane::Assembly, row))
            .maybe(width > 0, |el| el.child(gutter(width, self.arrows)))
            .child(block_rule())
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
struct InstructionRow {
    data: AsmData,
    /// Which instruction this row draws.
    index: usize,
    /// Which row of the listing it is drawn in, which is `index` plus every separator
    /// above it. The picked-out run and the scroll speak this one; everything else --
    /// the gutter, the line info, the branch edges -- speaks `index`. See [`Lanes`].
    row: usize,
    /// What this row draws in the gutter, worked out by the list for the reason `focused`
    /// is: the lanes lit in row 40 belong to a branch of row 12.
    arrows: RowArrows,
    /// Where the pointer is, which this row writes and does not read. Kept out of the
    /// `PartialEq` below: it is the same handle for the whole life of the list.
    hover: State<Option<usize>>,
    /// The listing's scroll and its height, for a branch operand to scroll to the row it
    /// names. Out of the `PartialEq` for the same reason `hover` is: both are the list's
    /// own handles and neither changes while it lives.
    controller: ScrollController,
    viewport: State<f32>,
    /// Whether the source line the pointer is on is the one this instruction was compiled
    /// from. Worked out by the list rather than read here, so that a focus moving between
    /// two instructions of one line leaves every row untouched.
    focused: bool,
    /// Whether the source line a click pinned is that same line.
    pinned: bool,
    /// Whether this row is one of the run picked out to be copied. Worked out by the list
    /// too, so a row re-renders on its own membership changing and not on every row a drag
    /// passes over.
    selected: bool,
    key: DiffKey,
}

impl PartialEq for InstructionRow {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
            && self.index == other.index
            && self.row == other.row
            && self.focused == other.focused
            && self.pinned == other.pinned
            && self.selected == other.selected
            && self.arrows == other.arrows
    }
}

impl KeyExt for InstructionRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for InstructionRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut focused = use_consume::<Focused>().0;
        let mut pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        // Consumed here, in the render, because the menu handler may not run a hook.
        let located = use_consume::<Locations>().0;
        let dock = use_consume::<ContentDock>().0;
        // The source-driven tab this listing is the assembly side of, if it is one: a
        // location found from it is chosen for it.
        let subject = self.data.subject.clone();
        let mut hover = self.hover;
        let index = self.index;
        let row = self.row;
        let width = self.data.lanes.width;
        let instruction = &self.data.assembly.instructions[self.index];

        // Where this row points on the source side. Worked out once here rather than in
        // each of the three handlers, which all need the same answer.
        let at = self.data.position(self.index);
        let focus = at.clone().map(|at| LineFocus {
            at,
            from: FocusOrigin::Instruction(instruction.address),
        });
        let taken = focus.clone();

        let relocation = instruction
            .relocation
            .as_ref()
            .map(|target| RelocationLabel {
                object: self.data.object.clone(),
                target: target.clone(),
            });

        // A branch's displacement is a link only where this listing has the row it lands
        // on -- the same set the gutter draws an arrow for, since both are asking
        // `edges`. A tail call, or a jump into the middle of an instruction, keeps its
        // plain operand.
        let branch = match (
            instruction.branch_span,
            self.data.assembly.edge_from(self.index),
        ) {
            (Some(span), Some(edge)) => instruction.format.get(span).map(|(text, _)| BranchLabel {
                text: text.clone(),
                to: self.data.lanes.row_of(edge.to),
                at: self.data.position(edge.to),
                controller: self.controller,
                viewport: self.viewport,
            }),
            _ => None,
        };

        // The disassembler says which span the link replaced, so the row is three
        // children: the text before that span, the link, and the text after it. That keeps
        // the link in the operand's own position, inside the brackets of a memory operand
        // and after the `rip+` of a rip-relative one. The two spans are exclusive -- a
        // branch whose displacement is a relocation placeholder names no address of its
        // own -- so there is at most one link, and an instruction with no span has an
        // empty tail and the relocation's name appended.
        let link = match instruction.relocation_span {
            Some(i) if relocation.is_some() => Some(i),
            _ => branch.as_ref().and(instruction.branch_span),
        };
        let (head, tail) = match link {
            Some(i) if i < instruction.format.len() => {
                (&instruction.format[..i], &instruction.format[i + 1..])
            }
            _ => (&instruction.format[..], &[][..]),
        };

        // Whatever text runs up to the link ends in the formatter's padding to the
        // operand column, and Skia trims trailing whitespace when it measures a
        // paragraph — which would butt the name right up against the mnemonic. Make
        // that padding non-breaking to keep the column.
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

        let head = paragraph()
            .max_lines(1)
            .spans_iter(spans(head, link.is_some()).into_iter());
        let tail = (!tail.is_empty()).then(|| {
            paragraph()
                .max_lines(1)
                .spans_iter(spans(tail, false).into_iter())
        });

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            // Horizontally only: the gutter's lines run to the row's own top and bottom
            // edges, and padding there would break every line in the column once per row.
            .padding(Gaps::new_symmetric(0.0, 3.0))
            .assembly_font()
            .background(row_background(
                hovering(),
                self.focused,
                self.pinned,
                self.selected,
            ))
            // The *down* and not the press: a drag is over by the time a press fires, so a
            // selection swept out with the button held has to begin as it goes down.
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Assembly, row);
                }
            })
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                // Two hovers: this row's own background, and the index shared with the
                // whole list, which the gutter uses for rows the pointer is nowhere near.
                hover.set_if_modified(Some(index));
                focused.set_if_modified(taken.clone());
                // Sweeping a selection out to here, in the handler the cross-view focus
                // already uses -- a second `pointer_over` would answer the same event
                // twice.
                mark_drag(marked, Pane::Assembly, row);
            })
            .on_pointer_out(move |_| {
                hovering.set_if_modified(false);
                // `pointerout` on the row being left and `pointerover` on the row being
                // entered are not ordered against each other, so a row may only take back
                // what is still its own. See `release_focus`.
                if *hover.peek() == Some(index) {
                    hover.set(None);
                }
                release_focus(focused, focus.as_ref());
            })
            // And offers no menu either: there is no line to find the locations of.
            .maybe(at.is_some(), {
                let at = at.clone();
                move |row| {
                    let Some(at) = at else {
                        return row;
                    };
                    row.on_secondary_down(move |e: Event<PressEventData>| {
                        ContextMenu::open_from_event(
                            &e,
                            locate_menu(located, dock, at.clone(), subject.clone(), None),
                        );
                    })
                }
            })
            .on_press(move |_| {
                // An instruction the debug info places nowhere pins nothing rather than
                // clearing what is pinned: a click on a prologue byte is not a way of
                // losing the line the reader put there.
                if let Some(at) = at.clone() {
                    pinned.set(Some(Pin {
                        at,
                        reveal: Owed::by(Pane::Source),
                        landed: false,
                    }));
                }
            })
            // Nothing at all for a symbol that branches nowhere inside itself, which most
            // do: an empty column would still be a column.
            .maybe(width > 0, |el| el.child(gutter(width, self.arrows)))
            .child(
                label()
                    .text(format!("{:016X} ", instruction.address))
                    .min_width(Size::px(200.0))
                    .color(palette().address_fg)
                    .max_lines(1),
            )
            .child(head)
            .maybe_child(relocation)
            .maybe_child(branch)
            .maybe_child(tail)
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
    assembly: Arc<Assembly>,
    /// The whole symbol and not just its object, because these rows draw a disassembly
    /// *and* answer to it -- a relocation label navigates to a symbol in the same object.
    symbol: Symbol,
    /// The question this listing answers, and **not** the one being asked: while the
    /// worker catches up the pane is still drawing the listing being left. Two things
    /// come out of it -- [`asked_of`], the tab whose viewing position this is (the file's
    /// tab for a source-driven one, never the resolved symbol's, which is very likely not
    /// open at all), and the line a source-driven tab is driven from, which lights its
    /// rows when nothing is pinned.
    asked: Ask,
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && self.symbol == other.symbol
            && self.asked == other.asked
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
    }
}

impl Component for InstructionList {
    fn render(&self) -> impl IntoElement {
        // Only the position, not the origin the focus also carries: a focus moving between
        // two instructions compiled from one line leaves this data equal and the whole list
        // untouched.
        let focus = use_consume::<Focused>()
            .0
            .read()
            .as_ref()
            .map(|focus| focus.at.clone());
        let pinned = use_consume::<Pinned>().0;
        // The pin, falling back to the line this listing was asked for. In a
        // source-driven tab those are the same thing until `use_clear_focus` drops the
        // pin with the tab, and coming back to a listing with nothing lit and no reason
        // given is worse than lighting the line it is of.
        let pin = pinned
            .read()
            .as_ref()
            .map(|pin| pin.at.clone())
            .or(match &self.asked {
                Ask::Source { at, .. } => Some(at.clone()),
                Ask::Symbol(_) => None,
            });
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Assembly);
        // The box the keyboard reaches this pane through: a `pointer_down` anywhere inside
        // it bubbles to here and asks for focus, which is what makes Ctrl+C mean this
        // listing.
        let a11y = use_a11y();

        let controller = use_scroll_controller(ScrollConfig::default);
        // How tall the list is, which `reveal_row` needs to know whether the row it was
        // asked for is on screen already. `VirtualScrollView` measures itself but keeps
        // the answer, so the rect wrapping it is measured here instead.
        let mut viewport = use_state(|| 0.0f32);

        // Which row the pointer is on, which the rows write and the gutter reads. It lives
        // here because it is what the rows *around* the pointer need: hovering a `jne`
        // lights its line all the way down to where it lands.
        let hover = use_state(|| None::<usize>);

        let data = AsmData {
            assembly: self.assembly.clone(),
            object: self.symbol.object.clone(),
            lanes: self.lanes.clone(),
            lines: self.lines.clone(),
            subject: match &self.asked {
                Ask::Source { at, .. } => Some(at.file.clone()),
                Ask::Symbol(_) => None,
            },
        };
        // The listing's rows, which is the instructions plus a separator above every row a
        // branch lands on. Everything below that scrolls, picks out or counts rows is in
        // this space; `AsmData::position`, the gutter and the edges are in the
        // instructions'. `Lanes` converts, and is the only thing that may.
        let length = data.lanes.listing_rows(data.assembly.instructions.len());
        // Where this tab was left, put back when it is switched to and written down as it
        // is scrolled -- and the scroll a pin is owed, which wins over it.
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<AsmAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
            {
                let data = data.clone();
                move |controller: &mut ScrollController| {
                    let Some(at) = owed_reveal(pinned, Pane::Assembly) else {
                        return false;
                    };
                    // Nothing at all when the line produced no instruction here -- one
                    // the optimiser folded away, or one belonging to another function,
                    // or, in a source-driven tab, the listing this very click is asking
                    // for not having arrived yet. Scrolling somewhere arbitrary would be
                    // worse than not scrolling, and **the request is left owed**, so the
                    // listing that can answer it still finds it.
                    let Some(index) = (0..data.assembly.instructions.len())
                        .find(|&index| data.position(index).as_ref() == Some(&at))
                    else {
                        return false;
                    };
                    reveal_made(pinned, Pane::Assembly);
                    reveal_row(controller, *viewport.peek(), data.lanes.row_of(index));
                    true
                }
            },
            controller,
            &asked_of(&self.asked),
            length,
            // The top: a listing *is* the symbol, so its first row is its own first line.
            0,
        );
        let touching = hover()
            .map(|row| data.lanes.touching(row))
            .unwrap_or_default();

        let on_key_down = {
            let assembly = self.assembly.clone();
            let lanes = self.lanes.clone();
            // A separator copies as the blank line it is drawn as, so a run lifted out of
            // the listing keeps the blocks apart on the way to the clipboard.
            on_listing_key(marked, Pane::Assembly, length, move |row| {
                lanes
                    .instruction_at(row)
                    .and_then(|index| assembly.instructions.get(index))
                    .map(asm_line)
                    .unwrap_or_default()
            })
        };

        rect()
            .expanded()
            .a11y_id(a11y)
            .a11y_focusable(true)
            .on_pointer_down(move |_| a11y.request_focus())
            .on_key_down(on_key_down)
            .on_sized(move |e: Event<SizedEventData>| viewport.set_if_modified(e.area.height()))
            .child(
                VirtualScrollView::new_with_data_controlled(
                    AsmRows {
                        data,
                        focus,
                        pin,
                        touching,
                        rows,
                    },
                    move |i, rows: &AsmRows| {
                        let selected = rows.rows.is_some_and(|run| run.contains(i));
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
                                selected,
                                width: rows.data.lanes.width,
                                arrows: RowArrows {
                                    lanes: rows.data.lanes.boundary(below),
                                    lit,
                                },
                                key: DiffKey::None,
                            }
                            .key((true, address))
                            .into();
                        };

                        let (focused, pinned) = rows.lit(index);
                        InstructionRow {
                            data: rows.data.clone(),
                            index,
                            row: i,
                            focused,
                            pinned,
                            selected,
                            arrows: RowArrows {
                                lanes: rows.data.lanes.row(index),
                                lit: lanes::lit(&rows.touching, index),
                            },
                            hover,
                            controller,
                            viewport,
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

/// The Assembly pane: a dispatch over the things [`Analyzed`] can be saying, and no work
/// of its own.
///
/// It reads the analysis and not the active document for everything it draws, which keeps
/// the listing and the rows in step: while the worker is catching up the two disagree, and
/// it is the analysis that says which symbol is actually in hand. The one thing it asks
/// the document is the word for having been asked nothing, which differs by the kind of
/// tab -- a source-driven one is waiting for a line to be clicked in it.
#[derive(Clone)]
pub(crate) struct AssemblyPane {
    pub(crate) document: Document,
}

impl PartialEq for AssemblyPane {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
    }
}

impl Component for AssemblyPane {
    fn render(&self) -> impl IntoElement {
        let analysis = use_consume::<Analysis>().0.read().clone();
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
            .width(Size::fill())
            .height(Size::fill())
            .padding(5.0)
            .background(palette().asm_pane_bg)
            .child(InstructionList {
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

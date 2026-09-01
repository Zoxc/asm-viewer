//! The assembly half of a document, from the row up: what a row is drawn out of, the
//! branch gutter, the clickable name of a relocation target, the virtual list of
//! instructions and the pane holding it.
//!
//! The gutter is drawn with **rects and not `canvas()`**, whose `RenderCallback` compares
//! equal unconditionally -- exactly wrong for a row a scroll view recycles. And a row's
//! height must equal the [`code_row_height`] the view over it was given, or scrolling
//! misaligns.

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
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && Arc::ptr_eq(&self.object, &other.object)
            && Arc::ptr_eq(&self.lanes, &other.lanes)
            && self.lines == other.lines
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
fn gutter(width: usize, arrows: RowArrows) -> impl IntoElement {
    let height = code_row_height();
    let middle = height / 2.0;
    // Where an arrowhead points, and where a horizontal run ends. Lane 0 is the innermost,
    // so the lanes are laid out leftwards from here.
    let tip = width as f32 * LANE_WIDTH + ARROW_WIDTH;
    let lane_x = move |lane: usize| (width - 1 - lane) as f32 * LANE_WIDTH + LANE_WIDTH / 2.0;

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
        .width(Size::px(tip + GUTTER_PAD))
        .height(Size::px(code_row_height()))
        .children((0..width).filter_map(move |lane| {
            let vertical = arrows.lanes.lanes[lane];
            let (top, tall) = match (vertical.top, vertical.bottom) {
                (true, true) => (0.0, height),
                (true, false) => (0.0, middle),
                (false, true) => (middle, height - middle),
                (false, false) => return None,
            };

            Some(
                stroke(
                    lane_x(lane) - BRANCH_STROKE / 2.0,
                    top,
                    BRANCH_STROKE,
                    tall,
                    arrows.lit.lanes[lane],
                )
                .into_element(),
            )
        }))
        .maybe_child(arrows.lanes.stub.map(|lane| {
            stroke(
                lane_x(lane),
                middle - BRANCH_STROKE / 2.0,
                tip - lane_x(lane),
                BRANCH_STROKE,
                lit,
            )
        }))
        // The two strokes of the arrowhead are one stroke turned about its right end,
        // which is the tip, once each way.
        .maybe(arrows.lanes.arrow, |el| {
            el.children([ARROW_ANGLE, -ARROW_ANGLE].map(|angle| {
                stroke(
                    tip - ARROW_STROKE,
                    middle - BRANCH_STROKE / 2.0,
                    ARROW_STROKE,
                    BRANCH_STROKE,
                    lit,
                )
                .rotate(angle)
                .transform_origin(TransformOrigin::right())
                .into_element()
            }))
        })
}

#[derive(Clone)]
struct InstructionRow {
    data: AsmData,
    index: usize,
    /// What this row draws in the gutter, worked out by the list for the reason `focused`
    /// is: the lanes lit in row 40 belong to a branch of row 12.
    arrows: RowArrows,
    /// Where the pointer is, which this row writes and does not read. Kept out of the
    /// `PartialEq` below: it is the same handle for the whole life of the list.
    hover: State<Option<usize>>,
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
        let mut hover = self.hover;
        let index = self.index;
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

        // The disassembler says which span the target's name landed in, so the row is
        // three children: the text before that span, the name as a clickable link, and the
        // text after it. That keeps the link in the operand's own position, inside the
        // brackets of a memory operand and after the `rip+` of a rip-relative one. An
        // instruction with no such span has an empty tail and the link is appended.
        let (head, tail) = match instruction.relocation_span {
            Some(i) if relocation.is_some() && i < instruction.format.len() => {
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
            .spans_iter(spans(head, relocation.is_some()).into_iter());
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
                    mark_press(marked, *shift.peek(), Pane::Assembly, index);
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
                mark_drag(marked, Pane::Assembly, index);
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
            .on_press(move |_| {
                // An instruction the debug info places nowhere pins nothing rather than
                // clearing what is pinned: a click on a prologue byte is not a way of
                // losing the line the reader put there.
                if let Some(at) = at.clone() {
                    pinned.set(Some(Pin {
                        at,
                        reveal: Some(Pane::Source),
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
    /// The whole symbol and not just its object, because these rows answer to a *tab* as
    /// well as to a disassembly: `Document::Assembly(Selection::Symbol(symbol))` is the
    /// key its viewing position is kept under.
    symbol: Symbol,
    lanes: Arc<Lanes>,
    lines: SymbolLines,
}

impl PartialEq for InstructionList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly)
            && self.symbol == other.symbol
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
        let pin = pinned.read().as_ref().map(|pin| pin.at.clone());
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Assembly);
        // The box the keyboard reaches this pane through: a `pointer_down` anywhere inside
        // it bubbles to here and asks for focus, which is what makes Ctrl+C mean this
        // listing.
        let a11y = use_a11y();

        let mut controller = use_scroll_controller(ScrollConfig::default);
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
        };
        let length = data.assembly.instructions.len();
        // Where this tab was left, put back when it is switched to and written down as it
        // is scrolled. A `Pin::reveal` (the effect below) wins over it.
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<AsmAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
            controller,
            &Document::Assembly(Selection::Symbol(self.symbol.clone())),
            length,
        );
        let touching = hover()
            .map(|row| data.lanes.touching(row))
            .unwrap_or_default();

        let on_key_down = {
            let assembly = self.assembly.clone();
            on_listing_key(marked, Pane::Assembly, length, move |index| {
                assembly
                    .instructions
                    .get(index)
                    .map(asm_line)
                    .unwrap_or_default()
            })
        };

        // The deps are the disassembly and nothing the pointer touches, so this is armed
        // once per symbol; `use_side_effect`'s callback is built by a `use_hook` and would
        // otherwise still be holding the first symbol ever selected.
        use_side_effect_with_deps(&data, move |data: &AsmData| {
            let Some(at) = take_reveal(pinned, Pane::Assembly) else {
                return;
            };

            // Nothing at all when the line produced no instruction here -- one the
            // optimiser folded away, or one belonging to another function. Scrolling
            // somewhere arbitrary would be worse than not scrolling.
            let Some(index) = (0..data.assembly.instructions.len())
                .find(|&index| data.position(index).as_ref() == Some(&at))
            else {
                return;
            };

            reveal_row(&mut controller, viewport(), index);
        });

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
                        let (focused, pinned) = rows.lit(i);
                        InstructionRow {
                            data: rows.data.clone(),
                            index: i,
                            focused,
                            pinned,
                            selected: rows.rows.is_some_and(|run| run.contains(i)),
                            arrows: RowArrows {
                                lanes: rows.data.lanes.row(i),
                                lit: lanes::lit(&rows.touching, i),
                            },
                            hover,
                            key: DiffKey::None,
                        }
                        .key(rows.data.assembly.instructions[i].address)
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
/// it is the analysis that says which symbol is actually in hand. The one thing it asks the
/// document is what *kind* of tab this is: a source-driven one has no symbol to show yet,
/// and draws nothing rather than the analysis' answer for another tab.
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
        let analysis = use_consume::<Analysis>().0;

        let source_driven = matches!(self.document, Document::Source(_));
        if source_driven {
            return rect().expanded().background(palette().asm_pane_bg).into();
        }

        let analysis = analysis.read().clone();
        let studied = match analysis.showing() {
            Showing::Listing(studied) => studied,
            Showing::Message(text) => return placeholder(text),
            Showing::Nothing => return rect().expanded().background(palette().asm_pane_bg).into(),
        };
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
                lanes: studied.lanes.clone(),
                lines: studied.lines.clone(),
            })
            .into()
    }
}

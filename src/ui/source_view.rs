//! The source half of a document, from the row up: one line of the file drawn with the
//! spans a parse resolved, the list of them, and the pane deciding which file that is.
//!
//! [`source_side`] is the one place either pane decides which file is up, so the pane and
//! the effect that drops its picked-out rows cannot disagree about which listing is being
//! shown. A **subject** is a source-driven tab's own file; a **companion** is the file the
//! drawn symbol was compiled from, and it comes out of the analysis rather than out of
//! `Active` because the two arrive from different threads and disagree for as long as the
//! worker takes.
//!
//! The rows are the app's own and not freya's `CodeEditor`, which paints a background only
//! for the cursor's row and keeps its scroll state private -- which is to say it can do
//! neither of the two things this pane exists to do.

use super::*;

/// What the source rows are built from: the file's text and highlighting, which file it
/// is -- a row hovered points the assembly pane at a position, and a line number is not
/// one on its own -- and which of its lines the assembly pane is pointing at, by the
/// pointer and by a click.
///
/// Both of those are line numbers rather than positions because the file has already been
/// matched: a position naming another of the symbol's files is not a row of this one, and
/// answering that once here beats answering it per visible row.
#[derive(Clone)]
struct SourceData {
    source: SourceText,
    file: Arc<str>,
    focus: Option<u32>,
    pin: Option<u32>,
    /// The run of rows picked out to be copied, or `None` when the selection is the
    /// assembly pane's or there is none.
    rows: Option<RowSelection>,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.focus == other.focus
            && self.pin == other.pin
            && self.rows == other.rows
    }
}

/// One line of a source file: its number in a gutter, then its text.
///
/// `file` is carried to be pointed at rather than to be drawn: hovering the row tells the
/// assembly pane which position to light up, and a line number without the file it is a
/// line of is not one.
#[derive(Clone)]
struct SourceRow {
    source: SourceText,
    file: Arc<str>,
    index: usize,
    /// Whether the instruction the pointer is on was compiled from this line.
    focused: bool,
    /// Whether the instruction a click pinned was compiled from this line.
    pinned: bool,
    /// Whether this row is one of the run picked out to be copied, told to it by the list
    /// for the reason `InstructionRow`'s is.
    selected: bool,
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.index == other.index
            && self.focused == other.focused
            && self.pinned == other.pinned
            && self.selected == other.selected
    }
}

impl KeyExt for SourceRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SourceRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let mut focused = use_consume::<Focused>().0;
        let mut pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        let index = self.index;
        let source = &self.source.0;

        // The position this row is, and so the one it points the assembly pane at: the
        // file the pane opened, at this row's own line -- its index plus one, for the same
        // reason the gutter below draws that number.
        let at = LinePos {
            file: self.file.clone(),
            line: self.index as u32 + 1,
        };
        let focus = Some(LineFocus {
            at: at.clone(),
            from: FocusOrigin::Source,
        });
        let taken = focus.clone();

        // In range because the list's length is this file's own `lines`, which is at most
        // `blocks.len()` -- and `SyntaxBlocks::get_line` unwraps rather than answering
        // `None`, so being in range is this row's responsibility.
        let spans = source
            .blocks
            .get_line(self.index)
            .iter()
            .map(|(color, node)| {
                let text = match node {
                    TextNode::Range(range) => source.rope.slice(range.clone()).to_string(),
                    // A run of leading indentation, which the highlighter hands over as a
                    // length rather than as text so an editor can draw it as dots. Here it
                    // is plain spaces, since this pane shows a file rather than edits one.
                    TextNode::LineOfChars { len, .. } => " ".repeat(*len),
                };
                Span::new(text).color(*color).assembly_font()
            })
            .collect::<Vec<_>>();

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            .padding(3.0)
            .assembly_font()
            .background(row_background(
                hovering(),
                self.focused,
                self.pinned,
                self.selected,
            ))
            // The same gesture as the assembly pane's, in the same order and for the same
            // reasons: the two panes show code and a reader picking lines out of one of
            // them must not have to learn the other.
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, *shift.peek(), Pane::Source, index);
                }
            })
            .on_pointer_over(move |_| {
                hovering.set_if_modified(true);
                focused.set_if_modified(taken.clone());
                mark_drag(marked, Pane::Source, index);
            })
            .on_pointer_out(move |_| {
                hovering.set_if_modified(false);
                release_focus(focused, focus.as_ref());
            })
            // Every source row is a position, so unlike an instruction row this one always
            // has something to pin -- a line no instruction was compiled from included,
            // which the assembly pane answers by staying where it is.
            .on_press(move |_| {
                pinned.set(Some(Pin {
                    at: at.clone(),
                    reveal: Some(Pane::Assembly),
                }));
            })
            .child(
                label()
                    // Line numbers are 1-based, as DWARF's are, so the gutter reads the
                    // way an editor's does. Right-aligned in a column of its own so the
                    // text of every line starts at the same x whatever the number's
                    // width -- and the width is fixed rather than a minimum, because
                    // skia lays a paragraph out to the width it is given and aligns
                    // within *that*: a label free to be wider puts its number at the far
                    // right of the row, on top of the source text.
                    //
                    // The gap after the number is a non-breaking space for the reason
                    // `InstructionRow` uses one: skia trims trailing whitespace when it
                    // measures, which would butt the number against the text.
                    .text(format!("{}\u{a0}", self.index + 1))
                    .width(Size::px(60.0))
                    .text_align(TextAlign::Right)
                    .color(palette().address_fg)
                    .max_lines(1),
            )
            .child(paragraph().max_lines(1).spans_iter(spans.into_iter()))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The source rows themselves, split out of `SourceTab` the way `InstructionList` is out
/// of `AssemblyTab` -- here not because the pane above is expensive to render, which it is
/// not, but because it has several early returns before it knows which file it is showing.
/// Hooks have to run on every render, and the scroll controller these rows are driven by
/// cannot be armed before the file it would scroll through is known.
#[derive(Clone)]
struct SourceList {
    source: SourceText,
    file: Arc<str>,
    /// The tab these rows belong to, which is what the viewing position is kept under and
    /// is **not** the same as the file being shown: this pane draws a source-driven tab's
    /// own file *and* an assembly-driven tab's companion, and two functions compiled from
    /// one file are two tabs with one file between them. Keying by the document is what
    /// stops them sharing a position they have no reason to share.
    document: Document,
}

impl PartialEq for SourceList {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.document == other.document
    }
}

impl Component for SourceList {
    fn render(&self) -> impl IntoElement {
        let focused = use_consume::<Focused>().0;
        let pinned = use_consume::<Pinned>().0;
        let marked = use_consume::<Marked>().0;
        let rows = marked_rows(marked, Pane::Source);
        let a11y = use_a11y();

        let mut controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);

        // Which line of *this* file each of the two cross-view positions names: a symbol's
        // rows can name several files and the pane has one of them open, so a position in
        // another of them is no line here at all.
        let line_here = |at: &LinePos| (at.file == self.file).then_some(at.line);
        let focus = focused
            .read()
            .as_ref()
            .and_then(|focus| line_here(&focus.at));
        let pin = pinned.read().as_ref().and_then(|pin| line_here(&pin.at));

        let length = self.source.0.lines;
        // The tab and not the file: see `SourceList::document`.
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<SrcAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
            controller,
            &self.document,
            length,
        );

        let on_key_down = {
            let source = self.source.clone();
            on_listing_key(marked, Pane::Source, length, move |index| {
                // The file's own text and not the row's spans: what the reader wants
                // pasted is the line as it is on disk, tabs and all, where the row draws
                // a run of leading whitespace as the plain spaces the highlighter hands
                // it over as. The newline is the join's business, not a line's.
                source
                    .0
                    .rope
                    .get_line(index)
                    .map(|line| {
                        let line = line.to_string();
                        line.trim_end_matches(|c| c == '\n' || c == '\r').to_owned()
                    })
                    .unwrap_or_default()
            })
        };

        use_side_effect_with_deps(self, move |list: &SourceList| {
            let Some(at) = take_reveal(pinned, Pane::Source) else {
                return;
            };

            // Nothing to scroll to when the instruction clicked came from a file this pane
            // is not showing, which is the same answer the highlight gives it: an inlined
            // header's line 42 is not line 42 of the file on screen. Nor when the line is
            // past the end of the file, which is source that has moved on since it was
            // compiled rather than debug info to be believed.
            if at.file != list.file {
                return;
            }
            let Some(index) = (at.line as usize)
                .checked_sub(1)
                .filter(|index| *index < list.source.0.lines)
            else {
                return;
            };

            reveal_row(&mut controller, viewport(), index);
        });

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .padding(5.0)
            .child(
                rect()
                    .expanded()
                    .a11y_id(a11y)
                    .a11y_focusable(true)
                    .on_pointer_down(move |_| a11y.request_focus())
                    .on_key_down(on_key_down)
                    .on_sized(move |e: Event<SizedEventData>| {
                        viewport.set_if_modified(e.area.height())
                    })
                    .child(
                        VirtualScrollView::new_with_data_controlled(
                            SourceData {
                                source: self.source.clone(),
                                file: self.file.clone(),
                                focus,
                                pin,
                                rows,
                            },
                            |i, data: &SourceData| {
                                let line = Some(i as u32 + 1);
                                SourceRow {
                                    source: data.source.clone(),
                                    file: data.file.clone(),
                                    index: i,
                                    focused: data.focus == line,
                                    pinned: data.pin == line,
                                    selected: data.rows.is_some_and(|rows| rows.contains(i)),
                                    key: DiffKey::None,
                                }
                                .key(i)
                                .into()
                            },
                            controller,
                        )
                        .length(length)
                        .item_size(code_row_height()),
                    ),
            )
    }
}

/// Which file the Source pane is drawing, and whose side of the tab it is.
///
/// The one place either pane decides that, so the Source pane and the effect that drops
/// its picked-out rows cannot disagree about which listing is up. A **subject** is the
/// tab's own file, a **companion** is the file the drawn symbol was compiled from — and
/// which of the two it is comes from the active document's kind and from nothing else.
///
/// The companion comes out of the *analysis* and not out of `Active`, because the two
/// disagree for as long as the worker takes and it is the analysis that says which symbol
/// is actually drawn. `SymbolLines` carries the file beside the line info for exactly
/// this reason.
pub(crate) enum SourceSide {
    Subject(Arc<str>),
    Companion(Arc<str>),
}

impl SourceSide {
    pub(crate) fn file(&self) -> &Arc<str> {
        match self {
            SourceSide::Subject(file) | SourceSide::Companion(file) => file,
        }
    }
}

pub(crate) fn source_side(active: Option<&Document>, analysis: &Analyzed) -> Option<SourceSide> {
    match active? {
        Document::Source(file) => Some(SourceSide::Subject(file.clone())),
        Document::Assembly(_) => {
            let shown = analysis.shown.as_ref()?;
            shown.lines.file.clone().map(SourceSide::Companion)
        }
    }
}

/// The bar over the Source pane naming the file it is showing as a **companion**, and
/// opening that file as a tab of its own when it is pressed.
///
/// It exists because the strip no longer does the job. A companion file is not a tab —
/// it is one side of the function's tab — so nothing else in the window says which file
/// the pane is drawing, and the whole path used to be a tooltip on a chip that is gone.
///
/// Pressing it is also the way a **source-driven tab is made**: the reader is looking at a
/// file and says "this file, on its own", and what they get is the same kind of thing the
/// symbol list gives them. Until the project explorer and the source search land
/// (`notes/Goals.md`, *Panels and tabs*) this is the only door into one, which is why it
/// is a press and not a label.
///
/// A subject file gets no header: it is the tab, and the strip already names it.
///
/// The two states come in as arguments and are not consumed here: this is called from
/// inside a `match`, and a hook may only run unconditionally in a component's body.
fn companion_header(open: Open, history: State<History>, file: Arc<str>) -> Element {
    let document = Document::Source(file.clone());

    row_tooltip(
        file.to_string(),
        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .padding(Gaps::new_symmetric(0.0, 8.0))
            .spacing(6.0)
            .background(palette().header_bg)
            .border(bottom_hairline())
            .on_press(move |_| activate(open, history, Some(document.clone()), Visit::Went))
            .child(entry_icon(&Document::Source(file.clone())))
            .child(label().text(file_name(&file)).max_lines(1)),
    )
    .into_element()
}

/// The Source pane: the tab's source side, whichever of the two sides that is.
#[derive(Clone)]
pub(crate) struct SourcePane {
    pub(crate) document: Document,
}

impl PartialEq for SourcePane {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
    }
}

impl Component for SourcePane {
    fn render(&self) -> impl IntoElement {
        let open = use_open();
        let history = use_consume::<Hist>().0;
        // Consumed unconditionally, hooks having to run on every render, and read here
        // because the companion file comes out of it -- and because reading it is what
        // subscribes this tab to it, so the pane fills in when a newly selected symbol's
        // line info is worked out, without the root re-rendering.
        let analysis = use_consume::<Analysis>().0.read().clone();
        // The tab's own document and not `Active`: this pane is only ever mounted for the
        // tab it belongs to, and the document is in hand synchronously where `Active` is
        // a memo that catches up a beat later.
        let side = source_side(Some(&self.document), &analysis);

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, so the two
            // panes cannot disagree about whether anything is selected -- with one more
            // case of its own, since a symbol can be analysed and still name no file.
            return match analysis.showing() {
                Showing::Message(text) => placeholder(text),
                Showing::Nothing => rect().expanded().background(palette().pane_bg).into(),
                Showing::Listing(studied) if studied.lines.info.is_some() => {
                    placeholder("No source file for this symbol")
                }
                Showing::Listing(_) => placeholder("No line info"),
            };
        };

        let file = side.file().clone();
        let document = match &side {
            SourceSide::Subject(file) => Document::Source(file.clone()),
            // The *drawn* symbol's tab and not the active one, which is the same rule the
            // assembly side follows: while the worker is catching up the two disagree, and
            // a row written down against the tab that is arriving would be a row of the
            // listing that is leaving.
            SourceSide::Companion(_) => match analysis.shown.as_ref() {
                Some(studied) => Document::Assembly(Selection::Symbol(studied.symbol.clone())),
                None => return rect().expanded().background(palette().pane_bg).into(),
            },
        };

        rect()
            .expanded()
            // The header takes its own height and the list is given the rest, which torin
            // only works out for a `flex` child of a `Content::Flex` parent.
            .content(Content::Flex)
            .background(palette().pane_bg)
            .maybe_child(match &side {
                SourceSide::Companion(file) => Some(companion_header(open, history, file.clone())),
                SourceSide::Subject(_) => None,
            })
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // Named in the message because the path is the only clue to *why*:
                    // source built on another machine, moved, or deleted since all look
                    // alike from here.
                    .child(match source_text(Path::new(&*file)) {
                        Some(source) => SourceList {
                            source,
                            file,
                            document,
                        }
                        .into_element(),
                        None => placeholder(format!("Source file not found: {file}")),
                    }),
            )
            .into()
    }
}

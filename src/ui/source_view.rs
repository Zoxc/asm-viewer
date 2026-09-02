//! The source half of a document, from the row up: one line of the file drawn with the
//! spans a parse resolved, the list of them, and the pane deciding which file that is.
//!
//! [`source_side`] is the one place either pane decides which file is up, so the pane and
//! the effect that drops its picked-out rows cannot disagree about which listing is being
//! shown. Only the symbol's **own** file is ever drawn, never the rest of
//! `LineInfo::files`. The rows are the app's own and not freya's `CodeEditor`, which paints
//! a background only for the cursor's row and keeps its scroll state private.

use super::*;

/// What the source rows are built from: the file's text and highlighting, which file it is
/// -- a row hovered points the assembly pane at a position, and a line number is not one on
/// its own -- and which of its lines the assembly pane is pointing at, by the pointer and
/// by a click.
///
/// Those two are line numbers rather than positions because the file has already been
/// matched here rather than per visible row.
#[derive(Clone)]
struct SourceData {
    source: SourceText,
    file: Arc<str>,
    focus: Option<u32>,
    pin: Option<u32>,
    /// The run of rows picked out to be copied, or `None` when the selection is the
    /// assembly pane's or there is none.
    rows: Option<RowSelection>,
    /// Whether these rows *drive* the tab they are in -- true for a source-driven tab,
    /// where a click also says which assembly the other side shows, and false for the
    /// companion file beside a symbol, where the click is only a pin.
    ///
    /// It travels here and through `new_with_data` rather than being captured by the
    /// builder closure, which is never compared across renders.
    drives: bool,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.focus == other.focus
            && self.pin == other.pin
            && self.rows == other.rows
            && self.drives == other.drives
    }
}

/// One line of a source file: its number in a gutter, then its text. `file` is carried to
/// be pointed at rather than drawn: a line number without the file it is a line of is no
/// position for the assembly pane to light up.
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
    /// Whether a click here also drives the tab's assembly side. See [`SourceData`].
    drives: bool,
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
            && self.drives == other.drives
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
        let mut driven = use_consume::<Drives>().0;
        let marked = use_consume::<Marked>().0;
        let shift = use_consume::<Shift>().0;
        // Consumed here, in the render, because the menu handler may not run a hook.
        let located = use_consume::<Locations>().0;
        let dock = use_consume::<ContentDock>().0;
        let index = self.index;
        let source = &self.source.0;

        // The position this row is, and so the one it points the assembly pane at. Lines
        // are 1-based, as DWARF's are.
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
                    // Leading indentation, handed over as a length so an editor can draw it
                    // as dots. Plain spaces here, this pane showing a file and not editing
                    // one.
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
            // The same gesture as the assembly pane's, in the same order.
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
            .on_secondary_down({
                let at = at.clone();
                // A location found from the file a source-driven tab is about is chosen
                // for that tab; from a companion it opens the symbol.
                let subject = self.drives.then(|| self.file.clone());
                move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(
                        &e,
                        locate_menu(located, dock, at.clone(), subject.clone()),
                    );
                }
            })
            // Every source row is a position, so unlike an instruction row this one always
            // has something to pin.
            .on_press({
                let drives = self.drives;
                move |_| {
                    // **The only writer of `Driven`.** A click in the file a
                    // source-driven tab is about is what says which assembly its other
                    // side shows; a click in a companion file is a pin and nothing more,
                    // and a click in the assembly pane never comes here at all, so there
                    // is no way for the listing to re-drive itself.
                    if drives {
                        driven
                            .write()
                            .remember(Document::Source(at.file.clone()), at.line);
                    }
                    pinned.set(Some(Pin {
                        at: at.clone(),
                        reveal: Owed::by(Pane::Assembly),
                        landed: false,
                    }));
                }
            })
            .child(
                label()
                    // A fixed width and not a minimum: skia lays a paragraph out to the
                    // width it is given and aligns within *that*, so a label free to be
                    // wider puts its number at the far right of the row, on top of the
                    // text. The gap is non-breaking because skia trims trailing whitespace
                    // when it measures.
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

/// The source rows themselves, split out of the pane because that has several early
/// returns before it knows which file it is showing, and a hook has to run on every render.
#[derive(Clone)]
struct SourceList {
    source: SourceText,
    file: Arc<str>,
    /// The tab these rows belong to, which is what the viewing position is kept under and
    /// is **not** the same as the file being shown: two functions compiled from one file
    /// are two tabs, and keying by the file would have them share a position.
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

        let controller = use_scroll_controller(ScrollConfig::default);
        let mut viewport = use_state(|| 0.0f32);

        // Which line of *this* file each cross-view position names: a symbol's rows can
        // name several files, so a position in another of them is no line here at all.
        let line_here = |at: &LinePos| (at.file == self.file).then_some(at.line);
        let focus = focused
            .read()
            .as_ref()
            .and_then(|focus| line_here(&focus.at));
        // The pin, falling back to the line this tab is driven from, for the reason
        // `InstructionList`'s does: `use_clear_focus` drops the pin with the tab, and
        // coming back to a tab whose assembly side is a listing of a line nothing points
        // at reads as an accident. A companion file's tab is never driven, so this is the
        // pin alone there.
        let driven = use_consume::<Drives>().0;
        let pin = pinned
            .read()
            .as_ref()
            .and_then(|pin| line_here(&pin.at))
            .or_else(|| driven.read().line(&self.document));

        let length = self.source.0.lines;
        // The tab and not the file: see `SourceList::document`.
        let docs = use_consume::<OpenDocs>().0;
        use_kept_position(
            use_consume::<SrcAt>().0,
            move |document: &Document| docs.peek().id_of(document).is_some(),
            {
                let file = self.file.clone();
                move |controller: &mut ScrollController| {
                    let Some(at) = owed_reveal(pinned, Pane::Source) else {
                        return false;
                    };
                    // Nothing to scroll to when the instruction came from a file this
                    // pane is not showing -- an inlined header's line 42 is not line 42
                    // of the file on screen -- nor when the line is past the end of a
                    // file that has moved on since it was compiled.
                    if at.file != file {
                        return false;
                    }
                    let Some(index) = (at.line as usize)
                        .checked_sub(1)
                        .filter(|index| *index < length)
                    else {
                        return false;
                    };
                    reveal_made(pinned, Pane::Source);
                    reveal_row(controller, *viewport.peek(), index);
                    true
                }
            },
            controller,
            &self.document,
            length,
        );

        let on_key_down = {
            let source = self.source.clone();
            on_listing_key(marked, Pane::Source, length, move |index| {
                // The file's own text and not the row's spans: what is pasted is the line
                // as it is on disk, tabs and all. The newline is the join's business.
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
                                // A source-driven tab's subject is the file its own
                                // document names; a companion's tab is a symbol's.
                                drives: matches!(self.document, Document::Source(_)),
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
                                    drives: data.drives,
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

/// Which file the Source pane is drawing, and whose side of the tab it is: a **subject** is
/// a source-driven tab's own file, a **companion** the file the drawn symbol was compiled
/// from.
///
/// The companion comes out of the *analysis* and not out of `Active`, because the two
/// disagree for as long as the worker takes and it is the analysis that says which symbol
/// is actually drawn.
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

/// Which file the Source pane draws for `active`: a source-driven tab's own file, or the
/// drawn symbol's companion. The companion is the symbol's own file -- the one its first
/// instruction was compiled from -- except under a **landed** pin naming another file
/// the listing's line info knows: a row in the Locations panel opens a symbol on a line,
/// and a symbol whose prologue was inlined from elsewhere would otherwise open on that
/// elsewhere, with the line the reader asked for in a file that is not up. A pin made
/// inside the panes changes no file, so clicking an inlined instruction leaves the
/// symbol's own file on screen as it always did.
pub(crate) fn source_side(
    active: Option<&Document>,
    analysis: &Analyzed,
    pin: Option<&Pin>,
) -> Option<SourceSide> {
    match active? {
        Document::Source(file) => Some(SourceSide::Subject(file.clone())),
        Document::Assembly(_) => {
            let shown = analysis.shown.as_ref()?;
            let lines = &shown.studied.lines;
            let landed = pin
                .filter(|pin| pin.landed)
                .map(|pin| &pin.at.file)
                .filter(|file| {
                    lines
                        .info
                        .as_ref()
                        .is_some_and(|info| info.files().iter().any(|named| named == *file))
                });
            landed
                .cloned()
                .or_else(|| lines.file.clone())
                .map(SourceSide::Companion)
        }
    }
}

/// The bar over the Source pane naming the file it is showing as a **companion** -- a
/// subject gets none, being named by its own tab -- and opening that file as a
/// source-driven tab when it is pressed, which until the project explorer lands is the only
/// door into one.
///
/// The states come in as arguments: this is called from inside a `match`, and a hook may
/// only run unconditionally in a component's body.
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
        // Reading it is what subscribes this tab to the analysis, so the pane fills in when
        // a newly selected symbol's line info is worked out.
        let analysis = use_consume::<Analysis>().0.read().clone();
        // The tab's own document and not `Active`, which is a memo and a beat behind: this
        // pane is only ever mounted for the tab it belongs to.
        let pin = use_consume::<Pinned>().0.read().clone();
        let side = source_side(Some(&self.document), &analysis, pin.as_ref());

        let Some(side) = side else {
            // The same answer the assembly pane gives, from the same place, plus one case
            // of its own: a symbol can be analysed and still name no file.
            return match analysis.showing(&self.document) {
                Showing::Message(text) => placeholder(text),
                Showing::Nothing => rect().expanded().background(palette().pane_bg).into(),
                Showing::Listing(shown) if shown.studied.lines.info.is_some() => {
                    placeholder("No source file for this symbol")
                }
                Showing::Listing(_) => placeholder("No line info"),
            };
        };

        let file = side.file().clone();
        let document = match &side {
            SourceSide::Subject(file) => Document::Source(file.clone()),
            // The *drawn* symbol's tab and not the active one: a row written down against
            // the tab that is arriving would be a row of the listing that is leaving.
            SourceSide::Companion(_) => match analysis.shown.as_ref() {
                Some(shown) => asked_of(&shown.ask),
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
                    // The path is named in the message because it is the only clue to
                    // *why*: built elsewhere, moved and deleted all look alike from here.
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

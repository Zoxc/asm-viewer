//! The scratchpad's pane: the source the reader types into, the crates beside it, what the
//! compiler said, and what the program printed. The model and the worker are the file
//! beside this one.
//!
//! Every bad dependency row is marked, not the first, and a failed build points back at a
//! row structurally rather than by looking for a crate name in a sentence. A diagnostic's
//! **place is a target**, pressing it putting the cursor on the line and column rustc
//! named -- but only for a span in the pad's own source, there being nowhere to put a
//! cursor in a dependency's file. stdout and stderr are told apart by colour and by
//! nothing else -- stderr is not an error, it is the other stream, so it takes the
//! palette's one warm hue rather than the red. The output pane follows the newest line
//! while the reader is at the bottom of it and leaves them where they are the moment they
//! are not.
//!
//! **A line too wide for the pane wraps in the diagnostics and scrolls sideways in the
//! output**, and which of the two it is follows from the list it is in rather than from
//! anything about the text. A build says dozens of things, so the diagnostics are a plain
//! `ScrollView` of paragraphs whose heights are whatever they turn out to be; a run says
//! thousands, so the output stays a `VirtualScrollView`, which builds the rows it draws by
//! stepping one `item_size` at a time and therefore has to know a row's height before it
//! has built one.

use super::*;
use std::cell::Cell;

/// How wide the delete question is: a pad's name over the path its package is at, which is
/// the longest thing it draws.
const DELETE_WIDTH: f32 = 520.0;

/// How much of a dependency row the crate name takes against the version beside it.
const NAME_FLEX: f32 = 2.0;
const VERSION_FLEX: f32 = 1.0;

/// The place a diagnostic points at, as this pane can draw it: a [`PlaceTarget`] for the
/// pad's own source, pressing which puts the pad's cursor on that line and that column,
/// and a plain label for anywhere else.
///
/// **Only a span in the pad's own source is a target.** cargo names a file in a dependency
/// as readily as it names `src/main.rs`, and there is nowhere to put a cursor in one: the
/// editor holds the pad's source and this app opens no other file for editing. So those
/// keep the plain label they always had, with no wash, no pointer and no press. A target
/// that did nothing when pressed would be the worse of the two answers — the hover is a
/// promise, and one that is kept for `src/main.rs` and broken for everything else is worse
/// than never making it.
///
/// `text` is the pad's buffers, consumed by the pane and handed down: a hook may only be
/// called while a component renders, and this is called once per diagnostic.
fn pad_place(text: State<PadBuffers>, pad: &PadId, diagnostic: &Diagnostic) -> Option<Element> {
    let span = diagnostic.span.as_ref()?;
    let own = is_source_file(&span.file);
    let place = diagnostic_place(span, own);

    Some(match own {
        true => {
            let (pad, span) = (pad.clone(), span.clone());
            PlaceTarget {
                text: place,
                press: EventHandler::new(move |_| jump_to_span(text, &pad, &span)),
            }
            .into_element()
        }
        false => label()
            .text(place)
            .color(palette().address_fg)
            .max_lines(1)
            .into_element(),
    })
}

/// One `[dependencies]` row: the crate, the version required of it, and the × that drops
/// it. The problem is a prop because it is a property of the *list* -- `Problem::Repeated`
/// is about two rows -- and every bad row is marked rather than the first.
///
/// **Keyed by the row's id, which is what keeps a box with the row it was drawn for.**
/// freya compares any two `Writable`s as equal (`notes/upstream/freya.md`), so a row
/// holding one is never told it now points somewhere else. Keyed by position, the boxes
/// under a deleted row would keep the positions they mounted with, and the reader's caret
/// would be left in the row that has moved up into its place -- the reordering under an
/// edit a list of text boxes must not do.
#[derive(Clone, PartialEq)]
struct DependencyRow {
    id: RowId,
    dependency: Dependency,
    problem: Option<Problem>,
    key: DiffKey,
}

impl KeyExt for DependencyRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for DependencyRow {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let id = self.id;
        let problem = self.problem.clone();
        // Which box is wrong is the model's answer: `Repeated` is about the name, and
        // nothing in its wording says so.
        let half = problem.as_ref().map(Problem::half);

        // The two boxes write straight into the row they are drawn from, and find it by
        // its id. The × below can take the row away while the boxes of the rows under it
        // are still taking events out of that same press, so the lookup answers for a row
        // that has gone (`Scratchpad::dependency_mut`).
        let name = pad.into_writable().map(
            move |pads: &Pads| &pads.state().scratchpad.dependency(id).name,
            move |pads: &mut Pads| &mut pads.state_mut().scratchpad.dependency_mut(id).name,
        );
        let version = pad.into_writable().map(
            move |pads: &Pads| &pads.state().scratchpad.dependency(id).version,
            move |pads: &mut Pads| &mut pads.state_mut().scratchpad.dependency_mut(id).version,
        );

        let marked = |input: Input, box_half: Half| {
            input.maybe(half == Some(box_half), |input: Input| {
                input
                    .color(palette().invalid_fg)
                    .focus_border_fill(palette().invalid_fg)
            })
        };

        rect()
            .width(Size::fill())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(text_box_height()))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .content(Content::Flex)
                    .spacing(6.0)
                    .child(marked(
                        Input::new(name)
                            .placeholder("crate")
                            .compact()
                            .width(Size::flex(NAME_FLEX)),
                        Half::Name,
                    ))
                    .child(marked(
                        Input::new(version)
                            .placeholder("version")
                            .compact()
                            .width(Size::flex(VERSION_FLEX)),
                        Half::Version,
                    ))
                    .child(
                        Button::new()
                            .compact()
                            .on_press(move |_| {
                                pad.write().state_mut().scratchpad.remove_dependency(id);
                            })
                            .child("\u{00d7}"),
                    ),
            )
            // Against the row it belongs to and never as one message at the top.
            .maybe_child(problem.map(|problem| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new(0.0, 0.0, 4.0, 2.0))
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .text(problem.to_string())
                            .color(palette().invalid_fg)
                            .max_lines(1),
                    )
            }))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The shown scratchpad's source, in freya's own `CodeEditor` -- which the read-only source
/// pane rejected, both of its objections being about painting and scrolling a listing from
/// outside and neither surviving a pane the reader is typing in. What is ours is the
/// colours, out of the palette, and the font.
///
/// The pad is a prop and the buffer is that pad's own, so a switch does not hand the
/// arriving pad the buffer the leaving one was being typed into. It is mounted only for a
/// pad the table [`PadBuffers::holds`], which is not on its own enough: the press that
/// deletes a pad and this editor's own global press are one batch of events against one
/// measured tree, so the index outlives the buffer by the tail of that batch, and
/// [`PadBuffers`]'s index is total for it.
///
/// **Keyed by the pad, which is what keeps the map and the rows one pad's.** freya compares
/// any two `Writable`s as equal (`notes/upstream/freya.md`), so a component holding one is
/// never told it now points somewhere else: the editor's rows keep the map they mounted
/// with, and a switch to a pad already read leaves them drawing the pad that was left --
/// and, once that pad is deleted, drawing a buffer with no lines in it, which panics inside
/// freya. The key makes a pad change a different element, so the editor and every row of it
/// are taken down and built again against the pad on screen.
#[derive(Clone, PartialEq)]
struct SourceEditor {
    pad: PadId,
}

impl Component for SourceEditor {
    fn render_key(&self) -> DiffKey {
        DiffKey::from(&self.pad)
    }

    fn render(&self) -> impl IntoElement {
        let text = use_consume::<PadText>().0;
        let a11y_id = use_hook(AccessibilityId::new_unique);
        use_tab_keyboard(None, a11y_id);
        let (reading, writing) = (self.pad.clone(), self.pad.clone());
        let text = text.into_writable().map(
            move |buffers: &PadBuffers| buffers.get(&reading),
            move |buffers: &mut PadBuffers| buffers.get_mut(&writing),
        );

        let font = fonts();
        let size = font.mono.size();
        // The editor takes **one** family where everything else takes a chain; the rest of
        // it arrives by inheritance from the box around it (`Font::family`).
        let family = font.mono.family();
        // The editor multiplies its font size by this and floors the answer, so half a
        // pixel of slack is what lands the product on `code_row_height()` exactly.
        let line_height = (code_row_height() + 0.5) / size;

        rect()
            .expanded()
            .background(palette().pane_bg)
            .assembly_font()
            .child(
                CodeEditor::new(text, a11y_id)
                    .font_size(size)
                    .font_family(family)
                    .line_height(line_height)
                    .show_whitespace(false)
                    .background(palette().pane_bg)
                    .text(palette().name_fg)
                    .cursor(palette().text_fg)
                    // What would land on the clipboard, which is what `text_select_bg`
                    // already says in both code panes.
                    // already says in both code panes -- a character selection here where
                    // it is a run of rows there, and the same question either way.
                    .highlight(palette().text_select_bg)
                    // "You are here": the cursor's line, in the green the two code panes
                    // light the other side's place in. Safe to reuse, the editor being
                    // no pane's pair.
                    .line_selected_background(palette().pair_bg)
                    .gutter_selected(palette().text_fg)
                    .gutter_unselected(palette().address_fg)
                    .whitespace(palette().punctuation_fg)
                    // The window's chords, declined before the edit (`chords.rs`). The
                    // editor keeps its own tail, which types Tab rather than moving the
                    // keyboard on.
                    .on_pre_key_down(box_keys(Boxed::Editor, &[], |_, _| {})),
            )
    }
}

/// The lines a running program has written, as the row builder is handed them. `PartialEq`
/// is `Arc::ptr_eq`, which is load-bearing rather than an optimisation: deriving it would
/// compare thousands of strings on every render of a pane being appended to.
#[derive(Clone)]
struct OutputRows(Arc<RunOutput>);

impl PartialEq for OutputRows {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One line, in the colour of the stream it came from.
///
/// **As wide as the line is**: the row is what gives the list around it something to
/// scroll sideways over, and it is exactly the width the `max_lines(1)` label measures to
/// -- freya lays a one-line label out against `f32::MAX` and reports its longest line, so
/// a row asking for its content's width gets the whole of the line and not the pane's
/// share of it. The code panes' rows scroll sideways the same way but are never narrower
/// than their pane (`ui/width.rs`), because they carry a wash; this row carries none.
///
/// The one thing that cannot change is the **height**, which is `code_row_height()`
/// because that is the `item_size` the [`OutputPane`]'s `VirtualScrollView` steps by. That
/// is the whole of why this row cannot wrap the way a diagnostic does: a wrapped row is a
/// row whose height depends on its text, and a virtual list has to know every row's height
/// before it has built one.
fn output_row(line: &OutputLine) -> Element {
    let color = match line.stream {
        Stream::Out => palette().text_fg,
        Stream::Err => palette().string_fg,
    };

    rect()
        .width(Size::auto())
        .height(Size::px(code_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 12.0))
        .child(
            label()
                .text(line.text.to_string())
                .assembly_font()
                .color(color)
                .max_lines(1),
        )
        .into_element()
}

/// Follow the newest row: keep `controller` against the bottom of a list that is being
/// appended to, for exactly as long as the reader is at the bottom of it.
///
/// One effect, and what it does depends on what woke it. **Lines arriving are not an
/// occasion to re-judge where the reader is**: the row that has just been added is below
/// the viewport by definition, so a run that asked would find the pane scrolled away on
/// the first line of every run and never follow anything. So arriving lines only *spend*
/// the answer, and a scroll, a resize -- and the scroll this effect itself makes -- are
/// what write it.
///
/// The two ends of that are told apart by the output's **identity**, which is the pane's
/// `PartialEq` again and not its length: at `MAX_OUTPUT_LINES` the oldest line drops off
/// the front as a new one lands, so the count stops changing while the rows go on being
/// replaced. For the same reason the bottom is worked out from the rows there are *now*
/// and is never a row index written down on an earlier run -- every index shifts by one
/// each time the cap drops a line. `output` is that identity, and `viewport` is the height
/// of the rows' own box rather than the pane's -- the heading over them is no part of what
/// scrolls.
fn use_follow_tail(mut controller: ScrollController, viewport: f32, output: usize, length: usize) {
    // Whether the pane is following. A `Cell` and not a `State`, `use_kept_position`'s
    // `held` for the same reason: nothing renders from it, and a state would cost the
    // pane a render on every wheel event.
    let following = use_hook(|| Rc::new(Cell::new(true)));
    // The list the last run saw, which is what tells lines arriving from the reader moving.
    let seen = use_hook(|| Rc::new(Cell::new(None::<usize>)));

    // With deps and not a bare `use_side_effect`, whose callback is built in a `use_hook`
    // and would go on reading the first list this pane was ever handed.
    use_side_effect_with_deps(
        &(output, length, viewport),
        move |&(output, length, viewport): &(usize, usize, f32)| {
            // Subscribes this effect to the pane's own scroll, so it comes before any
            // return: a run that did not read it is a run no wheel event wakes.
            let (_, offset) = <(i32, i32)>::from(controller);

            // Before the first layout there is no viewport to judge against and no bottom
            // to scroll to. Nothing is written down either, so the run the size arrives on
            // is the one that pins the pane for the first time.
            if viewport <= 0.0 {
                return;
            }

            let height = code_row_height();
            let bottom = -(scroll_extent(length, height, viewport) as i32);

            if seen.replace(Some(output)) != Some(output) {
                // Only when it moves: `scroll_to_y` notifies whether or not the position
                // changes, and this effect is subscribed to what it writes.
                if following.get() && offset != bottom {
                    controller.scroll_to_y(bottom);
                }
                return;
            }

            // Judged in rows against the viewport as it is now, which is `reveal_row`'s
            // shape. **The newest row being drawn at all** is what counts as being at the
            // bottom, rather than being drawn entire: a scroll offset is a whole number of
            // pixels where a list of rows is not, so a list clamped hard against its end
            // stands a fraction of a pixel short of showing its last row and would arm
            // nothing, ever.
            let top = (-offset).max(0) as f32;
            let newest = length.saturating_sub(1) as f32 * height;
            following.set(newest < top + viewport);
        },
    );
}

/// What the program has written, under a line saying where the run got to.
///
/// A `VirtualScrollView` and not the diagnostics' plain one: a run's output is bounded at
/// `MAX_OUTPUT_LINES` and nothing else, so the rows have to be built as they are drawn --
/// which is the whole of why a row here cannot wrap and takes a **sideways scroll**
/// instead. That scroll costs one honest thing: the width the list can be moved over is
/// the widest row it has *built*, so a wide line further down the output is not reachable
/// until it has been scrolled to vertically. A virtual list has no other answer, having
/// never measured the rows it did not draw.
///
/// A component of its own for the sake of [`use_follow_tail`]: **keyed on the pad**, so
/// the scroll and the follow are that pad's output's and not one position dragged between
/// them by a switch. What a switch costs is that a pad comes back following again, having
/// been remounted -- the follow is what a pane arrives armed with rather than something
/// carried across a switch, and the pad being looked at is the one whose scrolling is
/// worth keeping.
#[derive(Clone)]
pub(crate) struct OutputPane {
    pub(crate) pad: PadId,
    pub(crate) lines: Arc<RunOutput>,
    /// Where the run got to -- [`PadState::run_verdict`]'s answer.
    pub(crate) verdict: Verdict,
    pub(crate) key: DiffKey,
}

impl PartialEq for OutputPane {
    fn eq(&self, other: &Self) -> bool {
        self.pad == other.pad
            && Arc::ptr_eq(&self.lines, &other.lines)
            && self.verdict == other.verdict
    }
}

impl KeyExt for OutputPane {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for OutputPane {
    fn render(&self) -> impl IntoElement {
        let lines = self.lines.clone();
        let length = lines.len();
        let verdict = self.verdict.clone();

        let controller = use_scroll_controller(ScrollConfig::default);
        // How tall the list is, which the follow needs to know where the bottom of it is.
        // `VirtualScrollView` measures itself but keeps the answer, so the rect wrapping
        // it is measured here instead -- the rect around *the rows*, the heading above
        // them being no part of what scrolls.
        let mut viewport = use_state(|| 0.0f32);
        use_follow_tail(controller, viewport(), Arc::as_ptr(&lines).addr(), length);

        rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .background(palette().asm_pane_bg)
            .border(bottom_hairline())
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::px(list_row_height()))
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .padding(Gaps::new_symmetric(0.0, 12.0))
                    .spacing(8.0)
                    .content(Content::Flex)
                    .overflow(Overflow::Clip)
                    .child(label().text("Output").font_weight(FontWeight::BOLD))
                    .child(
                        label()
                            .text(verdict.text)
                            .width(Size::flex(1.0))
                            .color(verdict_fg(verdict.bad))
                            .max_lines(1),
                    ),
            )
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::fill())
                    .on_sized(move |e: Event<SizedEventData>| {
                        viewport.set_if_modified(e.area.height())
                    })
                    .child(
                        // The lines go through the view's data and are not captured: the
                        // builder closure is never compared across renders, so a captured
                        // `Arc` would draw the first batch of output for ever.
                        VirtualScrollView::new_with_data_controlled(
                            OutputRows(lines),
                            |index, rows: &OutputRows| match rows.0.line(index) {
                                Some(line) => output_row(line),
                                // Only reachable if the list shortened between the length
                                // being read and the row being asked for, which the cap
                                // cannot do.
                                None => rect().height(Size::px(code_row_height())).into_element(),
                            },
                            controller,
                        )
                        .length(length)
                        .item_size(code_row_height()),
                    ),
            )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What a pad is called on screen: the name the reader gave it, or — for one they have not
/// named — the app's own label, which is its id in angle brackets.
///
/// The brackets are `<entry point>`'s device in a second place: they say the label is the
/// app's and not the reader's, so a row reading `<pad-3>` is plainly a pad with no name
/// rather than a pad someone called that. That is also the whole of why an id may be drawn
/// here at all, having no business anywhere a reader reads a *name* — in brackets it is not
/// being offered as one. A plain "Unnamed" was the alternative and is worse: three fresh
/// pads would be three identical rows.
fn pad_label(id: &PadId, name: &str) -> String {
    match name.trim() {
        "" => format!("<{}>", id.as_str()),
        named => named.to_owned(),
    }
}

/// Where a pad's package is, as the two places that draw it say so: the pane, under the
/// name, and the delete question, over the buttons. The package cargo is handed **is** the
/// storage, so this is the whole of what a pad is on disk.
fn package_path(store: &Option<Store>, scratchpad: Option<&Scratchpad>) -> String {
    match (store, scratchpad) {
        (Some(store), Some(scratchpad)) => {
            scratchpad.directory(store).to_string_lossy().into_owned()
        }
        _ => "nowhere to keep a scratchpad".to_owned(),
    }
}

/// The menu a pad's row opens on a right-click: one item, and it does not delete anything.
/// It asks, which is what [`Pads::confirming`] is, and [`DeletePopup`] is the question.
///
/// A right-click and not the × a dependency row has: a × there is one press away from a
/// list one row shorter, and this is one press away from the reader's own source being
/// gone.
fn delete_menu(pad: State<Pads>, id: PadId) -> Menu {
    Menu::new().child(
        MenuButton::new()
            .on_press(move |_| {
                let mut pad = pad;
                pad.write().confirming = Some(id.clone());
            })
            .child("Delete scratchpad"),
    )
}

/// The pad a delete question is about: what it is filed under, what the reader calls it,
/// and where its package is.
#[derive(Clone, PartialEq)]
struct PadToDelete {
    id: PadId,
    name: String,
    package: String,
}

/// The question a delete is behind: freya's `Popup`, which dims what is under it and takes
/// Escape or a press outside as no — [`RescuedPopup`]'s window in a second place, mounted
/// the same way. It shows exactly when it has children, so a pane nobody has asked anything
/// of draws no window.
///
/// It says what will go and where it is, the name being the reader's word for the pad and
/// the path being where they would look to get any of it back. There is nothing to get
/// back: no pad is kept anywhere else, and the app has no undo.
///
/// It reads [`Pads::confirming`] itself rather than being handed it, so the pad being asked
/// about is looked up where it is drawn and nowhere else.
#[derive(Clone, PartialEq)]
struct DeletePopup;

impl Component for DeletePopup {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let text = use_consume::<PadText>().0;
        let jobs = use_consume::<PadJobs>();
        let store = use_consume::<Storage>().0;

        // The pad the reader is being asked about, which need not be the shown one: any
        // row can be right-clicked.
        let asking = {
            let pads = pad.read();
            pads.confirming.clone().map(|id| {
                let scratchpad = pads.get(&id).map(|state| state.scratchpad.clone());
                PadToDelete {
                    name: scratchpad
                        .as_ref()
                        .map(|scratchpad| scratchpad.name.clone())
                        .unwrap_or_default(),
                    package: package_path(&store.peek(), scratchpad.as_ref()),
                    id,
                }
            })
        };

        Popup::new()
            .width(Size::px(DELETE_WIDTH))
            .on_close_request(move |_| pad.write().confirming = None)
            .map(asking, |popup, asking| {
                let id = asking.id.clone();
                popup
                    .child(
                        rect()
                            .padding(8.0)
                            .spacing(8.0)
                            .font(&fonts().ui)
                            .color(palette().text_fg)
                            .child(
                                label().text(format!(
                                    "Delete {}?",
                                    pad_label(&asking.id, &asking.name)
                                )),
                            )
                            .child(
                                label()
                                    .text(
                                        "Its source, its crates and everything built from it \
                                         go with it."
                                            .to_owned(),
                                    )
                                    .color(palette().address_fg),
                            )
                            // A path is as long as it is, and one that is cut off is one
                            // the reader cannot go and look at.
                            .child(
                                paragraph()
                                    .assembly_font()
                                    .color(palette().address_fg)
                                    .span(asking.package.clone()),
                            ),
                    )
                    .child(
                        PopupButtons::new()
                            .child(
                                Button::new()
                                    .on_press(move |_| pad.write().confirming = None)
                                    .child("Cancel"),
                            )
                            .child(
                                Button::new()
                                    .filled()
                                    .on_press(move |_| {
                                        request_delete_pad(pad, text, &jobs, id.clone())
                                    })
                                    .child("Delete"),
                            ),
                    )
            })
    }
}

/// One row of the pad list: a scratchpad that can be switched to, drawn by the name the
/// reader gave it — never by the id it is filed under.
///
/// The whole row is the press target, as a recent project's is, and the frame is
/// [`list_row`]: the shown pad is [`Chosen::Live`], so it wears the same selection a
/// picked-out row wears anywhere else. The name is a prop and the id is a prop, so a
/// rename in the box beside it redraws the row and nothing else has to be told.
#[derive(Clone, PartialEq)]
struct PadRow {
    id: PadId,
    name: String,
    shown: bool,
    key: DiffKey,
}

impl KeyExt for PadRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for PadRow {
    fn render(&self) -> impl IntoElement {
        let hovering = use_state(|| false);
        let fitted = use_fitted();
        let pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let (id, deleting) = (self.id.clone(), self.id.clone());

        let chosen = match self.shown {
            true => Chosen::Live,
            false => Chosen::No,
        };

        let unnamed = self.name.trim().is_empty();
        let label = pad_label(&self.id, &self.name);

        cut_tooltip(
            fitted.cut(),
            label.clone(),
            list_row(hovering, chosen)
                .on_press(move |_| show_pad(pad, &jobs, id.clone()))
                // Needs the `ContextMenuViewer` mounted at the root of `app()`; opening one
                // without it panics.
                .on_secondary_down(move |e: Event<PressEventData>| {
                    ContextMenu::open_from_event(&e, delete_menu(pad, deleting.clone()));
                })
                // Dimmed when it is the placeholder and not something the reader wrote,
                // which is how the recent-projects list draws a project with no name.
                .child(tree_name_fitted(fitted, label, unnamed, &[])),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// What the pane says over the listing once the reader has typed since the build. The
/// Source pane's checksum row in a second place, and exact where that one is a guess: the
/// app wrote the source this program was built from and kept it.
pub(crate) const STALE_PROGRAM: &str = "Edited since this was built";

/// The Scratchpad's assembly side: the program the pad last built, drawn as the unified
/// view of an object's whole code.
///
/// Its own component, and mounted only where there **is** a program, so the claim on the
/// reading (`use_code_beside`) and the drive off the editor's cursor are hooks that exist
/// exactly while the listing does, rather than hooks in `ScratchpadTab` that would have to
/// answer for a pad with nothing built.
#[derive(Clone)]
pub(crate) struct PadAssembly {
    pub(crate) object: Arc<Object>,
    /// Where the listing opens: the lowest placed address the pad's own code sits at.
    pub(crate) opening: Option<u64>,
    /// The pad's own source as this program spells it, which is what the run the cursor
    /// writes is a run *of*. `None` leaves the listing undriven: there is no name to
    /// compare a row's own against.
    pub(crate) file: Option<Arc<str>>,
    /// Which pad's buffer the cursor is read from.
    pub(crate) pad: PadId,
    /// Whether what is on screen has moved on from the program under it.
    pub(crate) stale: bool,
}

impl PartialEq for PadAssembly {
    /// By pointer for the object and by value for the rest. Every field is one the pane
    /// behaves differently for -- the file is what the drive writes a run *of* -- so a
    /// hand-written `PartialEq` that compared the object alone would be a program
    /// described differently and drawn the same.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object)
            && self.pad == other.pad
            && self.file == other.file
            && self.opening == other.opening
            && self.stale == other.stale
    }
}

/// Drive the listing from the editor's own cursor: the line it is on is the pad's source
/// run, which is what the listing lights the pair of and owes a scroll to.
///
/// `mark_line` is the same door a click on a source row goes through, so the editor and a
/// Source pane say the same thing to the same listing -- and `Owed::by(Pane::Assembly)`
/// because only that side can answer: freya's editor can neither light a set of lines nor
/// be scrolled from outside (`notes/upstream/freya.md`), so the pad's two panes point at
/// each other one way only.
///
/// **Compared against the run on screen and never against a line remembered here.**
/// `use_land` puts `Marks::default()` back on every change of the active entry, and the
/// Scratchpad page becoming the tab on screen is one, so a drive that remembered would be
/// wiped a beat after the page arrived and would never say it again. The comparison is
/// also what makes typing along one line write nothing.
fn use_driving_cursor(
    text: State<PadBuffers>,
    marked: State<Marks>,
    pad: PadId,
    file: Option<Arc<str>>,
) {
    // With deps, and not a bare effect: its closure is built once, so one built over the
    // pad and the file it first mounted with would go on driving those.
    use_side_effect_with_deps(
        &(pad, file),
        move |(pad, file): &(PadId, Option<Arc<str>>)| {
            // A program whose debug info names the pad's file nowhere is a listing nothing can
            // drive: there is no name to compare a row's own against.
            let Some(file) = file else {
                return;
            };
            // Reading the buffers is what subscribes this to the cursor: the editor writes
            // through its `Writable` for a bare move as much as for an edit.
            let buffers = text.read();
            if !buffers.holds(pad) {
                return;
            }
            let line = buffers.get(pad).cursor_row() as u32 + 1;
            drop(buffers);

            // Bound to a `let` of its own before the write below, the read's guard living to
            // the end of the statement it is in.
            let standing = marked
                .read()
                .source
                .as_ref()
                .is_some_and(|run| run.is_line(file, line));
            if !standing {
                mark_line(marked, file.clone(), line, None, Owed::by(Pane::Assembly));
            }
        },
    );
}

impl Component for PadAssembly {
    /// Keyed by the program, so a rebuild takes this and its listing down and builds them
    /// against the new one -- which is what makes the claim, the opening place and the
    /// listing's own hooks start again rather than carry a number that names nothing in
    /// the new program.
    fn render_key(&self) -> DiffKey {
        DiffKey::from(&Arc::as_ptr(&self.object).addr())
    }

    fn render(&self) -> impl IntoElement {
        let beside = use_consume::<Beside>().0;
        use_code_beside(beside, &self.object);

        // Open on the pad's own code rather than at the top, which for a linked Rust
        // program is the runtime's. A `Planting` and not a place in `Places::code_at`: the listing
        // keeps no place of its own, and an entry there would hold this program's bytes
        // with nothing that would ever forget them -- where a planting is taken once by
        // the pane below and put back to `None`. Written from the render, which is the
        // parent's and so runs before the listing's first.
        // The drive: the editor's cursor line is the run the listing lights the pair of.
        // Unconditional, as a hook must be -- the effect inside declines a program whose
        // debug info names the pad's file nowhere.
        let text = use_consume::<PadText>().0;
        let marked = use_consume::<Marked>().0;
        use_driving_cursor(text, marked, self.pad.clone(), self.file.clone());

        let mut plant = use_doors().plant;
        let opening = self.opening.map(|address| Planting {
            tab: Document::Code(self.object.clone()),
            address,
        });
        use_hook(move || {
            if opening.is_some() {
                plant.set(opening);
            }
        });

        rect()
            .expanded()
            .content(Content::Flex)
            .background(palette().asm_pane_bg)
            // Over the listing and not inside it: a notice about the whole program, where
            // the bar naming what a document's pane is drawing would be.
            .maybe_child(self.stale.then(|| stale_banner(STALE_PROGRAM)))
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    // The listing's own inset, the assembly pane's exactly.
                    .padding(5.0)
                    .child(SectionList {
                        place: Placing::Pad,
                        object: self.object.clone(),
                    }),
            )
            // Last, so the listing above is given what is left: the code makes room for
            // the find bar rather than being covered by it, as a document's pane does.
            .maybe_child(find_bar_over((Placing::Pad, Pane::Assembly)))
    }
}

/// The panel down the side: every pad there is, the New button over them, and under them
/// the sentence the last New or Delete was refused with.
///
/// It reads the order, a name per pad and [`Pads::refused`], and nothing else. A name each
/// and not a state each: the table holds every pad, and a row wants a string per pad, not a
/// source per pad.
#[derive(Clone, PartialEq)]
struct PadList;

impl Component for PadList {
    fn render(&self) -> impl IntoElement {
        let pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();

        let (rows, refused) = {
            let pads = pad.read();
            let shown = pads.shown().clone();
            let rows: Vec<Element> = pads
                .order
                .entries()
                .iter()
                .map(|id| {
                    let name = pads
                        .get(id)
                        .map(|state| state.scratchpad.name.clone())
                        .unwrap_or_default();
                    PadRow {
                        shown: *id == shown,
                        id: id.clone(),
                        name,
                        key: DiffKey::None,
                    }
                    .key(id.as_str().to_owned())
                    .into()
                })
                .collect();
            (rows, pads.refused.clone())
        };

        rect()
            .width(Size::px(PAD_LIST_WIDTH))
            .height(Size::fill())
            .border(right_hairline())
            .child(section_heading(
                "Scratchpads",
                Some(
                    Button::new()
                        .compact()
                        .on_press(move |_| request_new_pad(&jobs))
                        .child("New")
                        .into_element(),
                ),
            ))
            // A plain `ScrollView` and not a `VirtualScrollView`: these are one-label rows
            // and there are a handful of them, which is the History list's shape rather
            // than the symbol list's.
            .child(
                ScrollView::new().child(rect().width(Size::fill()).children(rows).into_element()),
            )
            // What the panel can be told no about: a New, and a delete. Under the list
            // rather than over it, so a list that fills the panel is not pushed down by a
            // line that is there once in a blue moon.
            .maybe_child(refused.map(|refused| verdict_line(Verdict::bad_news(refused))))
    }
}

/// The heading over the pad, and the pad's own strip of controls: Build, Run and the one
/// that puts the listing away.
///
/// **One Run button, because there is one program.** While something is running the only
/// thing to want from it is to stop it.
#[derive(Clone, PartialEq)]
struct PadHeader;

impl Component for PadHeader {
    fn render(&self) -> impl IntoElement {
        let pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let run_jobs = jobs.clone();

        let (opened, building, running, runnable) = {
            let pads = pad.read();
            let state = pads.state();
            (
                state.opened(),
                state.building,
                state.is_running(),
                state.executable().is_some(),
            )
        };

        section_heading(
            "Scratchpad",
            Some(
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .spacing(6.0)
                    .child(
                        Button::new()
                            // "Two builds cannot be started at once" and "nothing is
                            // written until the disk has been read", on the control as
                            // well as in `request_build`.
                            .enabled(opened && !building)
                            .on_press(move |_| request_build(pad, &jobs))
                            .child(match building {
                                true => cargo::BUILDING,
                                false => "Build",
                            }),
                    )
                    .child(
                        Button::new()
                            // "Nothing runs while a build does", on the control as well as
                            // in `request_run`: cargo is writing over the executable.
                            .enabled(running || (runnable && !building))
                            .on_press(move |_| match running {
                                true => stop_run(pad),
                                false => request_run(pad, &run_jobs),
                            })
                            .child(match running {
                                true => "Stop",
                                false => "Run",
                            }),
                    )
                    // The control that puts the listing away, where a document's sits on
                    // the leading pane's bar: this heading row is the pad's own strip of
                    // controls and the one thing here that is always up, the editor having
                    // no bar.
                    .child(PaneToggle { of: Placing::Pad })
                    .into_element(),
            ),
        )
    }
}

/// What the pad is called, where its package is, and where the last build got to.
#[derive(Clone, PartialEq)]
struct PadDetails;

impl Component for PadDetails {
    fn render(&self) -> impl IntoElement {
        // The one line of this pane that is a path and so can outrun its column.
        let packaged = use_fitted();
        let pad = use_consume::<Pad>().0;
        let store = use_consume::<Storage>().0;

        let (shown, package, verdict) = {
            let pads = pad.read();
            let state = pads.state();
            (
                pads.shown().clone(),
                package_path(&store.peek(), Some(&state.scratchpad)),
                state.verdict(),
            )
        };

        rect()
            .width(Size::fill())
            .spacing(6.0)
            // An ordinary bound box, exactly the project view's: the name is a value in the
            // pad's own package and nothing is filed under it, so a keystroke is a state
            // change the save effect writes out and there is nothing to refuse, nothing to
            // apply and no gesture to discover. It is what the id being hidden buys.
            .child(field_row(
                "Name",
                Input::new(pad.into_writable().map(
                    |pads: &Pads| &pads.state().scratchpad.name,
                    |pads: &mut Pads| &mut pads.state_mut().scratchpad.name,
                ))
                .compact()
                // The label the list is drawing, so an empty box says what the pad is
                // called elsewhere rather than a word that is true of any of them -- and
                // typing replaces it, where a seeded name would have to be cleared first.
                .placeholder(pad_label(&shown, ""))
                .width(Size::flex(1.0)),
            ))
            // Where it is on disk: the package cargo is handed *is* the storage. In a
            // tooltip too, a state directory being longer than any pane.
            .child(cut_tooltip(
                packaged.cut(),
                package.clone(),
                field_row(
                    "Package",
                    one_line_fitted(packaged, package)
                        .width(Size::flex(1.0))
                        .color(palette().address_fg),
                ),
            ))
            .maybe_child(verdict.map(verdict_line))
    }
}

/// The `[dependencies]` rows, the Add over them, and the two lines that say the package on
/// disk is not what is on screen: why nothing was written, and cargo's own words when they
/// are about these rows.
#[derive(Clone, PartialEq)]
struct DependencyList;

impl Component for DependencyList {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;

        let (rows, unsaved, refusal) = {
            let pads = pad.read();
            let state = pads.state();
            // The problems are the list's -- `Repeated` is about two rows -- so they are
            // worked out here and each row is handed its own. Every bad row is marked, not
            // the first.
            let problems: HashMap<RowId, Problem> =
                state.scratchpad.problems().into_iter().collect();
            let rows: Vec<Element> = state
                .scratchpad
                .dependencies()
                .iter()
                .map(|dependency| {
                    DependencyRow {
                        id: dependency.id,
                        dependency: dependency.clone(),
                        problem: problems.get(&dependency.id).cloned(),
                        key: DiffKey::None,
                    }
                    .key(dependency.id)
                    .into()
                })
                .collect();
            (
                rows,
                state.unsaved.clone(),
                state
                    .refusal()
                    .map(|message| text_block(message, palette().text_fg)),
            )
        };

        rect()
            .width(Size::fill())
            .spacing(6.0)
            .child(section_heading(
                "Dependencies",
                Some(
                    Button::new()
                        .compact()
                        .on_press(move |_| {
                            pad.write().state_mut().scratchpad.add_dependency("", "");
                        })
                        .child("Add")
                        .into_element(),
                ),
            ))
            .child(match rows.is_empty() {
                true => info_line("No crates asked for".to_owned()).into_element(),
                false => rect().width(Size::fill()).children(rows).into_element(),
            })
            .maybe_child(
                unsaved.map(|failure| {
                    verdict_line(Verdict::bad_news(format!("Not saved: {failure}")))
                }),
            )
            .maybe_child(refusal)
    }
}

/// What the compiler said about the last build, under the source it is about.
///
/// A plain `ScrollView` and never a virtual one, which is what lets the blocks in it wrap:
/// a virtual list steps by one `item_size`, and a row that wraps is a row whose height is
/// not known until it has been laid out. A build says dozens of things, so there is nothing
/// to virtualise away.
#[derive(Clone, PartialEq)]
struct DiagnosticsPane;

impl Component for DiagnosticsPane {
    fn render(&self) -> impl IntoElement {
        let pad = use_consume::<Pad>().0;
        // Here rather than in the row: a press puts the cursor in the pad's buffer, and
        // the hook that reaches for them may only be called while a component renders.
        let text = use_consume::<PadText>().0;

        let blocks: Vec<Element> = {
            let pads = pad.read();
            let shown = pads.shown().clone();
            pads.state()
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic_block(diagnostic, pad_place(text, &shown, diagnostic)))
                .collect()
        };

        match blocks.is_empty() {
            // Nothing said, nothing drawn: a bare rect measures nothing and takes no
            // share of the column, exactly as leaving the child out did.
            true => rect().into_element(),
            false => rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .background(palette().asm_pane_bg)
                .child(
                    ScrollView::new().child(
                        rect()
                            .width(Size::fill())
                            .padding(Gaps::new_symmetric(4.0, 12.0))
                            .children(blocks)
                            .into_element(),
                    ),
                )
                .into_element(),
        }
    }
}

/// The Scratchpad pane: the pads there are down one side, and beside it the shown one --
/// a source file the reader edits, the crates it asks for, a build, and what the compiler
/// said about it. What it *builds* goes through `open_files` like any other binary.
///
/// The skeleton and no more. Each piece of it reads the slice of [`Pads`] it draws, so
/// nothing here copies a pad's state out whole for the pieces below to pick over.
#[derive(PartialEq)]
pub(crate) struct ScratchpadTab;

impl Component for ScratchpadTab {
    fn render(&self) -> impl IntoElement {
        let pad = use_consume::<Pad>().0;
        let text = use_consume::<PadText>().0;
        // Consumed here, because the key handler below runs no hook. The four chords are
        // the four controls' own, so the pane holds them for exactly as long as it draws
        // the controls.
        let jobs = use_consume::<PadJobs>();

        // What the editor and the listing under it are drawn of: which pad, the program
        // it last built, and where its run got to. The rest of the pad is read by the
        // piece that draws it.
        let (shown, program, building, stale, ran) = {
            let pads = pad.read();
            let state = pads.state();
            (
                pads.shown().clone(),
                state.program.clone(),
                state.building,
                state.out_of_date(),
                state
                    .run_verdict()
                    .map(|verdict| (verdict, state.output.clone())),
            )
        };

        let editing = text.read().holds(&shown).then(|| shown.clone());

        let ratio = use_consume::<PadSplit>().0;
        let splits = use_consume::<PadSplits>().0;
        let following = *use_consume::<PadFollows>().0.read();
        // Where the reader left the handle, written back as they drag it, and read back
        // with a `peek` for the reason `use_dragged_size` gives.
        use_dragged_size(splits, ratio);
        let leading = ratio.peek().clamp(1.0, 99.0);

        let output = ran.map(|(verdict, lines)| {
            OutputPane {
                pad: shown.clone(),
                lines,
                verdict,
                key: DiffKey::None,
            }
            .key(shown.as_str().to_owned())
            .into_element()
        });

        // The reader's own side of the split: the file, then what the compiler said about
        // the file directly above it, then what the program it built printed.
        let source_column = rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .child(
                rect()
                    .width(Size::fill())
                    .height(Size::flex(2.0))
                    .border(bottom_hairline())
                    // Only once the pad's source has arrived and its buffer has been made:
                    // the editor indexes that buffer, and there is nothing yet to type into
                    // while the worker is still reading the disk.
                    .maybe_child(editing.map(|pad| SourceEditor { pad }.into_element())),
            )
            .child(DiagnosticsPane)
            // Under the diagnostics rather than over them: what the compiler said is about
            // the source directly above it, and what the program said is the newest thing
            // in the pane.
            .maybe_child(output)
            .into_element();

        // What the split's other side draws: the program the pad built, or the one line
        // saying why there is none. Always something while the listing is up, so the
        // handle does not jump when a build lands.
        let assembly = match (&program, building) {
            (Some(program), _) => PadAssembly {
                object: program.object.clone(),
                opening: program.opening,
                file: program.file.clone(),
                pad: shown.clone(),
                stale,
            }
            .into_element(),
            (None, true) => placeholder("Building..."),
            (None, false) => placeholder("Nothing built yet"),
        };

        // The split takes everything under the block above: the listing is a whole
        // program's code and wants the height, and the Build and Run buttons must not move
        // when the handle does.
        let split = rect()
            .width(Size::fill())
            .height(Size::flex(1.0))
            .child(match following {
                false => source_column,
                true => ResizableContainer::new()
                    .direction(Direction::Horizontal)
                    .controller(splits)
                    .panel(
                        // The editor leads, as a source-driven tab's own side does -- and
                        // because the keyboard goes to the first box a tab registers.
                        ResizablePanel::new(PanelSize::percent(leading))
                            .min_size(10.0)
                            .child(source_column),
                    )
                    .panel(
                        ResizablePanel::new(PanelSize::percent(100.0 - leading))
                            .min_size(10.0)
                            .child(assembly),
                    )
                    .into_element(),
            });

        let body = rect()
            .width(Size::flex(1.0))
            .height(Size::fill())
            .content(Content::Flex)
            .child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new_symmetric(8.0, 12.0))
                    .spacing(6.0)
                    .child(PadHeader)
                    .child(PadDetails)
                    .child(DependencyList),
            )
            .child(split);

        rect()
            .expanded()
            .horizontal()
            .content(Content::Flex)
            .background(palette().pane_bg)
            // Global, so the four chords are answered wherever the keyboard is in the
            // window -- the editor included, which declines them (`chords.rs`). And on
            // this rect, so they are answered only while the page is the tab on screen:
            // `page_body` mounts a page's body for that tab alone.
            .on_global_key_down(move |e: Event<KeyboardEventData>| {
                pad_key(pad, &jobs, &e.key, e.modifiers);
            })
            // Over both of them, and drawn as nothing at all until a row has been asked
            // about.
            .child(DeletePopup)
            .child(PadList)
            .child(body)
    }
}

/// The four chords the pad answers: Build, Run, the run stopped, and a new pad.
///
/// Each is the request the matching control presses, and **nothing but that request**:
/// every refusal -- two builds at once, a run while a build is on, a pad whose disk has
/// not been read yet -- lives in the request rather than in a button's `enabled`, so
/// calling it obeys them all, and asking them again here would be a second reading to
/// drift.
fn pad_key(pad: State<Pads>, jobs: &PadJobs, key: &Key, modifiers: Modifiers) {
    if Chord::Build.is(key, modifiers) {
        request_build(pad, jobs);
    } else if Chord::Run.is(key, modifiers) {
        request_run(pad, jobs);
    } else if Chord::StopRun.is(key, modifiers) {
        stop_run(pad);
    } else if Chord::NewPad.is(key, modifiers) {
        request_new_pad(jobs);
    }
}

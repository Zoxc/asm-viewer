//! The scratchpad's pane: the source the reader types into, the crates beside it, what the
//! compiler said, and what the program printed. The model and the worker are the file
//! beside this one.
//!
//! Every bad dependency row is marked, not the first, and a failed build points back at a
//! row structurally rather than by looking for a crate name in a sentence. stdout and
//! stderr are told apart by colour and by nothing else -- stderr is not an error, it is
//! the other stream, so it takes the palette's one warm hue rather than the red.

use super::*;

/// The file a scratchpad's source is, as cargo and rustc spell it.
pub(crate) const SOURCE_FILE: &str = "src/main.rs";

/// How much of a dependency row the crate name takes against the version beside it.
const NAME_FLEX: f32 = 2.0;
const VERSION_FLEX: f32 = 1.0;

/// A block of a tool's own output, laid out the way it wrote it: one label per line, in
/// the fixed-width font, so rustc's carets sit under what they point at. A paragraph that
/// wrapped would put a caret under the wrong character.
fn text_block(text: &str, color: Color) -> Element {
    rect()
        .width(Size::fill())
        .overflow(Overflow::Clip)
        .children(
            text.lines()
                .map(|line| {
                    label()
                        .text(line.to_owned())
                        .assembly_font()
                        .color(color)
                        .max_lines(1)
                        .into()
                })
                .collect::<Vec<Element>>(),
        )
        .into_element()
}

/// One thing the compiler said: a line that can be scanned, and cargo's own rendering of
/// it under that. The header adds the **place**, taken from the span rather than from the
/// text.
fn diagnostic_block(diagnostic: &Diagnostic) -> Element {
    let place = diagnostic.span.as_ref().map(|span| {
        let file = match span.file == SOURCE_FILE {
            true => span.file.clone(),
            // A registry path is most of a line on its own, and which crate it is in is
            // the useful half of it.
            false => file_name(&span.file),
        };

        format!("{file}:{}:{}", span.line, span.column)
    });

    rect()
        .width(Size::fill())
        .padding(Gaps::new(2.0, 0.0, 6.0, 0.0))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .content(Content::Flex)
                .child(
                    label()
                        .text(match diagnostic.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            Level::Note => "note",
                        })
                        // An error is the red every invalid thing wears, a warning the one
                        // warm hue in the palette, and a note recedes.
                        .color(match diagnostic.level {
                            Level::Error => palette().invalid_fg,
                            Level::Warning => palette().string_fg,
                            Level::Note => palette().address_fg,
                        })
                        .max_lines(1),
                )
                .maybe_child(place.map(|place| {
                    label()
                        .text(place)
                        .color(palette().address_fg)
                        .max_lines(1)
                        .into_element()
                }))
                .child(
                    label()
                        .text(diagnostic.message.clone())
                        .width(Size::flex(1.0))
                        .max_lines(1),
                ),
        )
        .child(text_block(&diagnostic.rendered, palette().text_fg))
        .into_element()
}

/// One `[dependencies]` row: the crate, the version required of it, and the × that drops
/// it. The problem is a prop because it is a property of the *list* -- `Problem::Repeated`
/// is about two rows -- and every bad row is marked rather than the first.
#[derive(Clone, PartialEq)]
struct DependencyRow {
    index: usize,
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
        let index = self.index;
        let problem = self.problem.clone();
        // Which box is wrong is the model's answer: `Repeated` is about the name, and
        // nothing in its wording says so.
        let half = problem.as_ref().map(Problem::half);

        // The two boxes write straight into the row they are drawn from. Indexing is safe
        // because a row is mounted only for an index the list has: the × below shortens
        // the list, and the rows are rebuilt before either box is read again.
        let name = pad.into_writable().map(
            move |pads: &Pads| &pads.state().scratchpad.dependencies[index].name,
            move |pads: &mut Pads| &mut pads.state_mut().scratchpad.dependencies[index].name,
        );
        let version = pad.into_writable().map(
            move |pads: &Pads| &pads.state().scratchpad.dependencies[index].version,
            move |pads: &mut Pads| &mut pads.state_mut().scratchpad.dependencies[index].version,
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
                    .height(Size::px(list_row_height() + 8.0))
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
                                pad.write()
                                    .state_mut()
                                    .scratchpad
                                    .dependencies
                                    .remove(index);
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
/// pad the table [`PadBuffers::holds`], which is what makes the mapped `Writable` safe --
/// a dependency row's two boxes are indexed the same way for the same reason.
#[derive(Clone, PartialEq)]
struct SourceEditor {
    pad: PadId,
}

impl Component for SourceEditor {
    fn render(&self) -> impl IntoElement {
        let text = use_consume::<PadText>().0;
        let a11y_id = use_hook(AccessibilityId::new_unique);
        let (reading, writing) = (self.pad.clone(), self.pad.clone());
        let text = text.into_writable().map(
            move |buffers: &PadBuffers| buffers.get(&reading),
            move |buffers: &mut PadBuffers| buffers.get_mut(&writing),
        );

        let font = fonts();
        let size = font.mono.size();
        // The editor takes **one** family where everything else takes a chain, and freya
        // appends the parent element's families behind an element's own -- so the rest of
        // the chain arrives by inheritance from the box around it.
        let family = font
            .mono
            .families
            .first()
            .map(|family| family.to_string())
            .unwrap_or_default();
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
                    // What would land on the clipboard, which is what `row_select_bg`
                    // already says in both code panes.
                    // already says in both code panes -- a character selection here where
                    // it is a run of rows there, and the same question either way.
                    .highlight(palette().row_select_bg)
                    // "You are here", which is `code_row_hover_bg`'s job in the other two
                    // panes. Safe to reuse, the editor painting no pointer hover at all.
                    .line_selected_background(palette().code_row_hover_bg)
                    .gutter_selected(palette().text_fg)
                    .gutter_unselected(palette().address_fg)
                    .whitespace(palette().punctuation_fg),
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
fn output_row(line: &crate::scratchpad::OutputLine) -> Element {
    let color = match line.stream {
        Stream::Out => palette().text_fg,
        Stream::Err => palette().string_fg,
    };

    rect()
        .width(Size::fill())
        .height(Size::px(code_row_height()))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 12.0))
        .overflow(Overflow::Clip)
        .child(
            label()
                .text(line.text.to_string())
                .assembly_font()
                .color(color)
                .max_lines(1),
        )
        .into_element()
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

/// One row of the pad list: a scratchpad that can be switched to, drawn by the name the
/// reader gave it — never by the id it is filed under.
///
/// The whole row is the press target, as a recent project's is; the shown pad wears
/// `selected_bg` and the one under the pointer `object_hover_bg`, which is what every list
/// in the sidebar already does. The name is a prop and the id is a prop, so a rename in the
/// box beside it redraws the row and nothing else has to be told.
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
        let mut hovering = use_state(|| false);
        let pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let id = self.id.clone();

        let background = match (self.shown, hovering()) {
            (true, _) => palette().selected_bg,
            (false, true) => palette().object_hover_bg,
            (false, false) => Color::TRANSPARENT,
        };

        let unnamed = self.name.trim().is_empty();
        let label = pad_label(&self.id, &self.name);

        row_tooltip(
            label.clone(),
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .horizontal()
                .cross_align(Alignment::Center)
                .padding(Gaps::new_symmetric(0.0, 6.0))
                .content(Content::Flex)
                .background(background)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| show_pad(pad, &jobs, id.clone()))
                // Dimmed when it is the placeholder and not something the reader wrote,
                // which is how the recent-projects list draws a project with no name.
                .child(tree_name(label, unnamed)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The Scratchpad pane: the pads there are down one side, and beside it the shown one --
/// a source file the reader edits, the crates it asks for, a build, and what the compiler
/// said about it. What it *builds* goes through `open_files` like any other binary.
#[derive(PartialEq)]
pub(crate) struct ScratchpadTab;

impl Component for ScratchpadTab {
    fn render(&self) -> impl IntoElement {
        let mut pad = use_consume::<Pad>().0;
        let jobs = use_consume::<PadJobs>();
        let new_jobs = jobs.clone();
        // The shown pad's own state and no more: the table holds every pad, and cloning
        // all of them on every render would clone every source the app is holding. The
        // rows want a name each, which is a string per pad and not a source per pad.
        let pads = pad.read();
        let (shown, state) = (pads.shown().clone(), pads.state().clone());
        let listed: Vec<(PadId, String)> = pads
            .order
            .ids()
            .iter()
            .map(|id| {
                let name = pads.get(id).map(|state| state.scratchpad.name.clone());
                (id.clone(), name.unwrap_or_default())
            })
            .collect();
        let refused = pads.refused.clone();
        drop(pads);

        let text = use_consume::<PadText>().0;
        let editing = text.read().holds(&shown).then(|| shown.clone());

        let problems: HashMap<usize, Problem> = state.scratchpad.problems().into_iter().collect();
        let rows: Vec<Element> = state
            .scratchpad
            .dependencies
            .iter()
            .enumerate()
            .map(|(index, dependency)| {
                DependencyRow {
                    index,
                    dependency: dependency.clone(),
                    problem: problems.get(&index).cloned(),
                    key: DiffKey::None,
                }
                .key(index)
                .into()
            })
            .collect();

        let diagnostics: Vec<Element> = state.diagnostics().iter().map(diagnostic_block).collect();
        let refusal = state
            .refusal()
            .map(|message| text_block(message, palette().text_fg));
        let package = match state.scratchpad.directory() {
            Some(directory) => directory.to_string_lossy().into_owned(),
            None => "nowhere to keep a scratchpad".to_owned(),
        };

        // **One button, because there is one program.** While something is running the
        // only thing to want from it is to stop it.
        let running = state.is_running();
        let run_jobs = jobs.clone();
        let run = Button::new()
            .enabled(running || (state.executable().is_some() && !state.building))
            .on_press(move |_| match running {
                true => stop_run(pad),
                false => request_run(pad, &run_jobs),
            })
            .child(match running {
                true => "Stop",
                false => "Run",
            })
            .into_element();

        let output = state.run_status().map(|(text, bad)| {
            let lines = state.output.clone();
            let length = lines.len();

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
                                .text(text)
                                .width(Size::flex(1.0))
                                .color(match bad {
                                    true => palette().invalid_fg,
                                    false => palette().address_fg,
                                })
                                .max_lines(1),
                        ),
                )
                .child(
                    // The lines go through `new_with_data` and are not captured: the builder
                    // closure is never compared across renders, so a captured `Arc` would
                    // draw the first batch of output for ever.
                    VirtualScrollView::new_with_data(
                        OutputRows(lines),
                        |index, rows: &OutputRows| match rows.0.line(index) {
                            Some(line) => output_row(line),
                            // Only reachable if the list shortened between the length being
                            // read and the row being asked for, which the cap cannot do.
                            None => rect().height(Size::px(code_row_height())).into_element(),
                        },
                    )
                    .length(length)
                    .item_size(code_row_height()),
                )
                .into_element()
        });

        // A plain `ScrollView` and not a `VirtualScrollView`: these are one-label rows and
        // there are a handful of them, which is the History list's shape rather than the
        // symbol list's.
        let pads: Vec<Element> = listed
            .into_iter()
            .map(|(id, name)| {
                let key = id.as_str().to_owned();
                PadRow {
                    shown: id == shown,
                    id,
                    name,
                    key: DiffKey::None,
                }
                .key(key)
                .into()
            })
            .collect();

        let panel = rect()
            .width(Size::px(PAD_LIST_WIDTH))
            .height(Size::fill())
            .border(right_hairline())
            .child(section_heading(
                "Scratchpads",
                Some(
                    Button::new()
                        .compact()
                        .on_press(move |_| request_new_pad(&new_jobs))
                        .child("New")
                        .into_element(),
                ),
            ))
            .child(
                ScrollView::new().child(rect().width(Size::fill()).children(pads).into_element()),
            )
            // The one thing the panel can be told no about. Under the list rather than
            // over it, so a list that fills the panel is not pushed down by a line that is
            // there once in a blue moon.
            .maybe_child(refused.map(|failure| {
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new_symmetric(2.0, 6.0))
                    .overflow(Overflow::Clip)
                    .child(
                        label()
                            .text(format!("Not made: {failure}"))
                            .color(palette().invalid_fg)
                            .max_lines(1),
                    )
            }));

        let body = rect()
            .width(Size::flex(1.0))
            .height(Size::fill())
            .content(Content::Flex)
            .child(
                rect()
                    .width(Size::fill())
                    .padding(Gaps::new_symmetric(8.0, 12.0))
                    .spacing(6.0)
                    .child(section_heading(
                        "Scratchpad",
                        Some(
                            rect()
                                .horizontal()
                                .cross_align(Alignment::Center)
                                .spacing(6.0)
                                .child(
                                    Button::new()
                                        // "Two builds cannot be started at once", on the
                                        // control as well as in `request_build`.
                                        .enabled(!state.building)
                                        .on_press(move |_| request_build(pad, &jobs))
                                        .child(match state.building {
                                            true => "Building...",
                                            false => "Build",
                                        }),
                                )
                                .child(run)
                                .into_element(),
                        ),
                    ))
                    // An ordinary bound box, exactly the project view's: the name is a
                    // value in the pad's own package and nothing is filed under it, so a
                    // keystroke is a state change the save effect writes out and there is
                    // nothing to refuse, nothing to apply and no gesture to discover. It
                    // is what the id being hidden buys.
                    .child(field_row(
                        "Name",
                        Input::new(pad.into_writable().map(
                            |pads: &Pads| &pads.state().scratchpad.name,
                            |pads: &mut Pads| &mut pads.state_mut().scratchpad.name,
                        ))
                        .compact()
                        // The label the row is drawing, so an empty box says what the pad
                        // is called elsewhere rather than a word that is true of any of
                        // them -- and typing replaces it, where a seeded name would have
                        // to be cleared first.
                        .placeholder(pad_label(&shown, ""))
                        .width(Size::flex(1.0)),
                    ))
                    // Where it is on disk: the package cargo is handed *is* the storage. In
                    // a tooltip too, a state directory being longer than any pane.
                    .child(row_tooltip(
                        package.clone(),
                        field_row(
                            "Package",
                            label()
                                .text(package)
                                .width(Size::flex(1.0))
                                .color(palette().address_fg)
                                .max_lines(1),
                        ),
                    ))
                    .maybe_child(state.status().map(|(text, bad)| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(text)
                                    .color(match bad {
                                        true => palette().invalid_fg,
                                        false => palette().address_fg,
                                    })
                                    .max_lines(1),
                            )
                    }))
                    .child(section_heading(
                        "Dependencies",
                        Some(
                            Button::new()
                                .compact()
                                .on_press(move |_| {
                                    pad.write()
                                        .state_mut()
                                        .scratchpad
                                        .dependencies
                                        .push(Dependency::default());
                                })
                                .child("Add")
                                .into_element(),
                        ),
                    ))
                    .child(match rows.is_empty() {
                        true => info_line("No crates asked for".to_owned()).into_element(),
                        false => rect().width(Size::fill()).children(rows).into_element(),
                    })
                    .maybe_child(state.unsaved.map(|failure| {
                        rect()
                            .padding(Gaps::new(2.0, 0.0, 2.0, 0.0))
                            .overflow(Overflow::Clip)
                            .child(
                                label()
                                    .text(format!("Not saved: {failure}"))
                                    .color(palette().invalid_fg)
                                    .max_lines(1),
                            )
                    }))
                    .maybe_child(refusal),
            )
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
            .maybe_child((!diagnostics.is_empty()).then(|| {
                rect()
                    .width(Size::fill())
                    .height(Size::flex(1.0))
                    .background(palette().asm_pane_bg)
                    .child(
                        ScrollView::new().child(
                            rect()
                                .width(Size::fill())
                                .padding(Gaps::new_symmetric(4.0, 12.0))
                                .children(diagnostics)
                                .into_element(),
                        ),
                    )
                    .into_element()
            }))
            // Under the diagnostics rather than over them: what the compiler said is about
            // the source directly above it, and what the program said is the newest thing
            // in the pane.
            .maybe_child(output);

        rect()
            .expanded()
            .horizontal()
            .content(Content::Flex)
            .background(palette().pane_bg)
            .child(panel)
            .child(body)
    }
}

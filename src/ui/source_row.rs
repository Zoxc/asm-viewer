//! One line of a source file as the pane draws it: the text cut from the parse, the names
//! the language server placed on it, and what a press, a menu or a key asks about one.
//!
//! The rows are the app's own and not freya's `CodeEditor`, which paints a background only
//! for the cursor's row and keeps its scroll state private. Each row's gutter marks whether
//! anything open has code from that line ([`Coded`]), so a reader can tell what was
//! compiled from what produced nothing without picking a line out, and before they have
//! picked one out at all.
//!
//! This file begins `use super::*` as every `ui/` file does, so what a row reaches for is
//! not written out at the top. What crosses the other way is: [`SourceData`] and
//! [`source_row`], which the list builds its rows from; [`caret_questions`], the keys it
//! wraps them in; and [`source_line`], for the find bar.

use super::*;

/// Everything every row of the file shares: the text they are cut from, which file it is
/// -- a row picked out is a line of a file, and a line number is not a place on its own --
/// and everything a row's names and its menu reach for.
///
/// Built once per render of the list and handed to every row as one `Rc`, so a row clones
/// a refcount and not six values. **The states are consumed where the list renders**,
/// since a handler may not run a hook and every one of these is read from a handler: the
/// menu, the press that follows a link, the pointer moving onto a name.
pub(crate) struct Common {
    pub(crate) source: SourceText,
    pub(crate) file: Arc<str>,
    /// The tab these rows *drive*, for a source-driven tab, where a click also says which
    /// assembly the other side shows -- and `None` for the companion file beside a
    /// symbol, where the click picks the line out and no more.
    ///
    /// It travels here and through `new_with_data` rather than being captured by the
    /// builder closure, which is never compared across renders.
    pub(crate) drives: Option<DocId>,
    /// Which of this file's names the language server placed, and so which are links.
    /// Read once for the list and carried, never asked per row: what a server is doing
    /// changes with every word it says about its progress, and every mounted row would be
    /// drawn again for it.
    pub(crate) links: links::Links,
    /// What the find bar is looking for, compiled once for the list; `None` where no bar
    /// is open (`find_bar.rs`).
    pub(crate) marking: Option<Marking>,
    /// Where a row's menu and the caret's four questions send their answers.
    pub(crate) asking: RowStates,
    /// Whether Ctrl is held, which is whether a link opens in a tab of its own.
    pub(crate) ctrl: State<bool>,
    /// Whom a press on a link or a name asks, and [`None`] where there is nobody: a pane
    /// mounted without a server draws its text and no links (`links_in`).
    pub(crate) server: Option<Server>,
    /// Where the name under the pointer is written, and [`None`] where there is nobody to
    /// write it: a pane mounted without it says nothing about a name.
    pub(crate) hover: Option<State<Hover>>,
}

impl PartialEq for Common {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.file, &other.file)
            && self.drives == other.drives
            && self.links == other.links
            && self.marking == other.marking
        // `asking`, `ctrl`, `server` and `hover` are left out. They are handles the root
        // provides and never replaces, and a row only reads them from a handler, so
        // comparing them would cost it a render for nothing.
    }
}

impl Common {
    /// The tab this file's own line questions are answered for, which is the tab it
    /// drives: a companion file beside a symbol drives none and is nobody's subject.
    fn subject(&self) -> Option<Subject> {
        self.drives.map(|tab| Subject {
            tab,
            file: self.file.clone(),
        })
    }
}

/// What the source rows are built from: what they all share, and which of the file's lines
/// the assembly pane's picked-out run was compiled from.
///
/// Those are line numbers rather than positions because the file has already been
/// matched here rather than per visible row.
#[derive(Clone)]
pub(crate) struct SourceData {
    pub(crate) common: Rc<Common>,
    /// The lines of this file the assembly pane's run was compiled from: its pair here.
    /// Out of a memo, so an unchanged set is the same `Arc` and the rows compare it by
    /// pointer.
    pub(crate) pairs: Arc<HashSet<u32>>,
    /// The lines of this file the listing beside it has instructions for at all: what the
    /// gutter marks.
    pub(crate) compiled: Arc<HashSet<u32>>,
    /// The run picked out here -- the caret, the characters, and so the rows -- for each
    /// row to draw its part of, or `None` when there is none.
    pub(crate) chars: Option<CharSelection>,
}

impl PartialEq for SourceData {
    fn eq(&self, other: &Self) -> bool {
        // By contents and not by pointer: the list builds a fresh one every render, so a
        // pointer compare would rebuild every row for every render of the pane.
        self.common == other.common
            && Arc::ptr_eq(&self.pairs, &other.pairs)
            // By contents too: the pane makes an empty set of its own for a file nothing
            // has answered about yet, so there is no pointer to compare.
            && self.compiled == other.compiled
            && self.chars == other.chars
    }
}

/// One line of a source file: its number in a gutter, then its text. The file is in
/// [`Common`] to be picked out rather than drawn: a line number without the file it is a
/// line of is no place for the assembly pane to light up.
#[derive(Clone)]
struct SourceRow {
    /// Everything this row shares with every other row of the file.
    common: Rc<Common>,
    index: usize,
    /// Whether an instruction of the assembly pane's picked-out run was compiled from
    /// this line, and if so which of its edges end the run of such lines.
    paired: Option<Edges>,
    /// Whether the listing beside this pane has an instruction from this line: what the
    /// mark in the gutter says.
    compiled: bool,
    /// The wash of its pane's selection, told to it by the list for the reason
    /// `InstructionRow`'s is.
    wash: Wash,
    /// The columns of this row inside the pane's character selection, likewise.
    chars: RowChars,
    key: DiffKey,
}

impl PartialEq for SourceRow {
    fn eq(&self, other: &Self) -> bool {
        self.common == other.common
            && self.index == other.index
            && self.paired == other.paired
            && self.compiled == other.compiled
            && self.wash == other.wash
            && self.chars == other.chars
    }
}

keyed!(SourceRow);

/// The text of `columns` on row `index`, or `None` where they name nothing of it.
///
/// The columns are the byte offsets a language server counts in (`src/links.rs`), cut out
/// of the row's own text -- which is what a menu built from a press calls the name, so
/// what the reader right-clicked is what the menu says. Columns inside a character name
/// nothing.
fn name_at(source: &SourceText, index: usize, columns: &Range<usize>) -> Option<String> {
    let cut = source.0.text(index);
    let name = cut.whole.get(columns.clone())?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// The text row `index` draws, as the clipboard sees a character selection of it.
///
/// One piece and not one per span. A row's spans are all text and adjacent text is one
/// run to every reader of a `Line` -- what it copies, how wide it is, where a find hits
/// (`src/find.rs`) -- so cutting it up again here would say the same thing at a cost.
pub(crate) fn source_line(source: &SourceText, index: usize) -> Line {
    Line::text(source.0.text(index).whole.clone())
}

/// What a press, a right-click or the pointer on one of a row's names is answered from:
/// where the row is, and the rest of the file
/// -- the names the language server placed, and whom to ask about one.
///
/// Cloned once into each closure a row's names need, rather than the same values cloned
/// one by one into every one; the file itself is one refcount. The rules those closures
/// carry out are the methods below, so a closure is the call and nothing else.
#[derive(Clone)]
struct Named {
    /// The position this row is, and so the one its questions are about. Lines are
    /// 1-based, as DWARF's are.
    at: LinePos,
    /// The file this row is a line of, and everything a question about it is put through.
    common: Rc<Common>,
}

impl Named {
    /// A column of this row as the place a language server is asked about.
    fn lookup(&self, column: usize) -> Lookup {
        Lookup::at(&self.at, column)
    }

    /// Every name the server placed on this row, in the order they are drawn: the slice
    /// both rules below read, taken once because finding it is a pair of binary searches.
    fn on_line(&self) -> &[links::Link] {
        self.common.links.on_line(self.at.line)
    }

    /// What the server placed on this row, as the row draws and hit-tests it: which names
    /// a press can follow and what such a press does -- ask the server where that name is,
    /// and go to what it answers -- and all of `on_line` beside them for the pointer to be
    /// answered about.
    ///
    /// The names are every one of them, links and the places where a name is defined
    /// alike: what a reader hovers is a name and not a door, and a hover over the name
    /// where a function is defined is where its own signature and doc comment are.
    ///
    /// [`None`] with nobody to ask, so no link is ever drawn that could not be followed
    /// and no name is hovered that nobody could be asked about. It is one answer for both
    /// because there is one server behind both.
    ///
    /// A press with Ctrl held opens what it names in a tab of its own, the rule every
    /// door inside a pane follows.
    fn linked(&self, on_line: &[links::Link]) -> Option<TextLinks> {
        let server = self.common.server.clone()?;
        let (open, ctrl) = (self.common.asking.doors.open, self.common.ctrl);
        let hover = self.common.hover;
        let named = self.clone();
        let pointed = self.clone();
        Some(TextLinks {
            columns: links::followed(on_line).cloned().collect(),
            // Always a door: nothing here is a link until the server has said the name is
            // one, so there is nothing to hold a modifier back for.
            is_link: Rc::new(|| true),
            lit_fg: palette().name_hover_fg,
            follow: Rc::new(move |columns: Range<usize>| {
                let column = columns.start;
                follow_link(
                    &server,
                    &named.common.links,
                    open,
                    &named.at,
                    column,
                    Reach::inside(ctrl),
                );
            }),
            names: on_line.iter().map(|link| link.columns.clone()).collect(),
            // What the pointer on one of them says, and nothing where there is nobody to
            // write it to.
            on_hover: hover.map(|hover| {
                Rc::new(move |under: Under| {
                    write_if(hover, |waiting| pointed.pointed(waiting, under));
                }) as Rc<dyn Fn(Under)>
            }),
        })
    }

    /// What the pointer moving onto one of those names, off them all, or out from under
    /// the row itself says: `waiting` as the move leaves it, and whether it moved at all,
    /// so the caller writes only then.
    fn pointed(&self, waiting: &mut Hover, under: Under) -> bool {
        match under {
            Under::Name(columns, drawn) => waiting.enter(Pointed {
                at: self.lookup(columns.start),
                drawn,
            }),
            Under::Off => waiting.left_name(),
            Under::Moved => waiting.gone(),
        }
    }

    /// The name at `column` of this row, which the three questions a menu offers are
    /// about: [`None`] over no name, and over one this row draws nothing of.
    fn at_column(&self, column: usize) -> Option<NameAt> {
        name_at_column(&self.common.source, &self.at, &self.common.links, column)
    }
}

/// The name at a place in `source`: the line `at` names, and the column. [`None`] over no
/// name the server placed, which is what a place on whitespace, on a keyword or past the
/// end of a row is.
///
/// **The place is one value**, so the file and the row cannot be said twice and disagree.
/// One rule for both ways of pointing at a name: [`Named::at_column`] asks it about the
/// pointer and [`caret_questions`] about the caret, so a key and the menu item beside it
/// cannot come to ask about two different places.
fn name_at_column(
    source: &SourceText,
    at: &LinePos,
    links: &links::Links,
    column: usize,
) -> Option<NameAt> {
    let row = at.row()?;
    let link = links.at(at.line, column)?;
    Some(NameAt {
        at: at.clone(),
        name: name_at(source, row, &link.columns)?,
        column: link.columns.start,
    })
}

/// What the Source pane answers over and above its own keys and the find bar's: **the
/// four questions a row's menu offers, asked about the caret** -- F12 for where the name
/// under it is defined, Shift+F12 for what refers to it, Ctrl+F12 for what implements it,
/// and Alt+F12 for every symbol the caret's line was compiled into.
///
/// They are the same three calls the menu makes ([`name_menu`], [`locate_menu`]), so a
/// key and the item beside it cannot come to mean two things; all that differs is where
/// the place comes from -- the run's lead, and not the pointer.
///
/// **A caret on no name asks nothing.** The three about a name have nothing to ask about
/// without one, and nobody to ask without a server. The line's locations are about the
/// row and not about a name, exactly as the menu item is, so what they want is a caret;
/// a pane with no run at all has none, and answers none of the four.
///
/// Wrapped around the pane's own handler as the find chord is, and offered in this pane
/// alone: the two assembly listings draw no names.
pub(crate) fn caret_questions(
    marked: State<Marks>,
    common: Rc<Common>,
    mut keys: impl FnMut(Event<KeyboardEventData>) + 'static,
) -> impl FnMut(Event<KeyboardEventData>) + 'static {
    move |e: Event<KeyboardEventData>| {
        let Some(
            chord @ (Chord::Definition
            | Chord::References
            | Chord::Implementations
            | Chord::AllLocations),
        ) = Chord::of(&e.key, e.modifiers)
        else {
            return keys(e);
        };
        // Bound before anything is written, the read being a guard.
        let caret = marked
            .peek()
            .of(Pane::Source)
            .as_ref()
            .map(|picked| picked.chars.lead());
        let Some(caret) = caret else {
            return;
        };
        let locating = common.asking.locating;
        // Where the caret is, said once: every question below is about this place.
        let at = LinePos::of_row(common.file.clone(), caret.row);
        if chord == Chord::AllLocations {
            locating.find(Query::line(at), common.subject());
            return;
        }
        let named = name_at_column(&common.source, &at, &common.links, caret.col);
        let (Some(named), Some(server)) = (named, common.server.as_ref()) else {
            return;
        };
        match chord {
            Chord::Definition => follow_name(
                server,
                common.asking.doors.open,
                Lookup::at(&named.at, named.column),
                lsp::Followed::Definition,
                Reach::InPlace,
            ),
            Chord::References => locating.listed(server, named, lsp::Listed::References),
            _ => locating.listed(server, named, lsp::Listed::Implementations),
        }
    }
}

/// What the right button offers on the row `named` is: the three questions for the server
/// where the press was on a name, then the line's locations and, inside a function as the
/// file's parse says, the function's instances. A location found from the file a
/// source-driven tab is about is chosen for that tab; from a companion it opens the
/// symbol, and the menu offers the file itself as a tab of its own.
///
/// Both the name under the pointer and the function this row is a line of are looked for
/// on the press and not per render -- the function being a walk of the file's own -- since
/// a row is rendered far more often than it is right-clicked.
///
/// **The row is one value.** Where it is and which file it is in are [`Named`]'s, so
/// nothing here can name a second place and drift from the keys ([`caret_questions`]),
/// and the states the menu writes come with it, reaching for a context being a hook the
/// handler may not run.
fn source_menu(named: Named) -> RowMenu {
    let RowStates {
        doors, locating, ..
    } = named.common.asking;
    let open = doors.open;
    // The file this row is in, where the pane is showing it beside somebody else's tab. A
    // subject is that tab already and has nothing to open.
    let opens = named
        .common
        .drives
        .is_none()
        .then(|| named.common.file.clone());
    let subject = named.common.subject();
    // Whom to ask about a name, where this row's names are links at all. A row drawing
    // none is a row over no server, and a question nobody could answer is not offered.
    let server = (!named.common.links.is_empty())
        .then(|| named.common.server.clone())
        .flatten();

    Rc::new(move |e: Event<PressEventData>, column| {
        let at = named.at.clone();
        let spans = &named.common.source.0.functions;
        let function = functions::enclosing(spans, at.line).cloned();
        // The name the press was on, which the three questions are about.
        let name = column
            .and_then(|column| named.at_column(column))
            .zip(server.clone())
            .map(|(name, server)| name_menu(&server, locating, open, name))
            .unwrap_or_default();
        let menu = locate_menu(
            locating,
            at.clone(),
            subject.clone(),
            function,
            name,
            // The pane this menu is in is the one that answers Alt+F12.
            Some(shortcuts::key!(AllLocations)),
        );
        // The door into the file itself, which the tab has only beside it: the same
        // arrival every other door into a source file makes, so the assembly side follows
        // this line as it follows a clicked one.
        let menu = menu.maybe_child(opens.clone().map(|file| {
            let (line, name) = (at.line, source::name_of(Path::new(&*file)));
            MenuButton::new()
                .on_press(move |_| {
                    open_source_place(doors, Path::new(&*file), line, None, Reach::NewTab)
                })
                .child(format!("Open {name}"))
        }));
        ContextMenu::open_from_event(&e, menu);
    })
}

/// A press in the file a source-driven tab is about, which is what says which assembly
/// its other side shows.
///
/// **The only writer of `Driven` inside the panes.** A click in a companion file picks
/// the line out and no more, and a click in the assembly pane never comes here at all, so
/// there is no way for the listing to re-drive itself.
fn drive(docs: State<Docs>, mut driven: State<Driven>, tab: DocId, at: &LinePos) {
    // The place the tab is at and not the file: two lines of one file reached along one
    // trail are two entries, and a drive written under the wrong one is a drive nothing
    // reads. Bound before the write, the guard being live until the end of the statement.
    let entry = place_at(&docs.peek(), tab, &Document::Source(at.file.clone()));
    driven.write().remember((tab, entry), at.line);
}

impl Component for SourceRow {
    fn render(&self) -> impl IntoElement {
        // Nothing is reached for here. Every state a handler below needs was consumed
        // where the list rendered and travels in `Common`, a handler being no place to
        // call a hook -- and asking per row was nine context walks a render.
        let common = &self.common;
        let index = self.index;

        // The line as it is drawn, taken from the file's own cut rather than made again:
        // a row is drawn afresh for a scroll, a modifier and every keystroke in the find
        // bar, and the cut is the same every time ([`Highlighted::text`]).
        let cut = common.source.0.text(index);
        let line = Line::text(cut.whole.clone());
        let named = Named {
            at: LinePos::of_row(common.file.clone(), index),
            common: common.clone(),
        };
        // The names on this row, found once: what the row draws as links and what it
        // hover-tests are both cut from the same slice.
        let on_line = named.on_line();

        let text = Text {
            marking: common.marking.clone(),
            line,
            // The one allocation a drawn row still owes: freya's `Span` holds a
            // `Cow<'static, str>`, so a span cannot borrow the cut it was taken from.
            spans: common
                .source
                .0
                .pieces(cut)
                .map(|piece| {
                    Span::new(piece.text.to_string())
                        .color(piece.colour)
                        .assembly_font()
                })
                .collect(),
            chars: self.chars,
            links: named.linked(on_line),
        };

        let at = named.at.clone();
        let menu = source_menu(named);

        // The line number, which is gutter: a press on it picks the row out and no
        // characters. A fixed width and not a minimum: skia lays a paragraph out to the
        // width it is given and aligns within *that*, so a label free to be wider puts
        // its number at the far right of the row, on top of the text. The gap is
        // non-breaking because skia trims trailing whitespace when it measures.
        let number = label()
            .text(format!("{}\u{a0}", index + 1))
            .width(Size::px(60.0))
            .text_align(TextAlign::Right)
            .color(palette().address_fg)
            .max_lines(1)
            .into_element();

        let mark = code_mark(self.compiled);

        // The same gesture as the assembly pane's, from the same chrome. The run is a
        // run of this file.
        code_row(
            Chrome {
                pane: Pane::Source,
                row: index,
                file: Some(common.file.clone()),
                paired: self.paired,
                wash: self.wash,
                measured: true,
            },
            vec![mark, number],
            Some(text),
            Some(menu),
        )
        // A press in a source-driven tab's own file also says which listing the
        // other side shows; the row is picked out by `pointer_down` either way.
        .maybe(common.drives.is_some(), |el| {
            let (drives, docs) = (common.drives, common.asking.doors.open.docs);
            let driven = common.asking.doors.places.driven;
            el.on_press(move |_| {
                if let Some(tab) = drives {
                    drive(docs, driven, tab, &at);
                }
            })
        })
    }

    fn render_key(&self) -> DiffKey {
        self.keyed()
    }
}

/// One row of the list, built from what every row of the file shares and the index it is
/// drawn at. The builder the list hands `VirtualScrollView`, which never compares the
/// closure it was given, so everything a row depends on arrives in `data`.
pub(crate) fn source_row(index: usize, data: &SourceData) -> Element {
    let paired_at = |row: usize| data.pairs.contains(&LinePos::line_of(row));
    SourceRow {
        common: data.common.clone(),
        index,
        paired: paired_at(index).then(|| Edges::of(index, paired_at)),
        compiled: data.compiled.contains(&LinePos::line_of(index)),
        wash: wash_of(data.chars, index),
        chars: RowChars::of(data.chars, index),
        key: DiffKey::None,
    }
    .key(index)
    .into()
}

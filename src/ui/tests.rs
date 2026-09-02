//! The tests that run the UI rather than the logic under it, and the palette's, which have
//! nowhere else to live.
//!
//! The runner tests exist for the class of bug the framework-free modules' own tests are
//! blind to by construction: a `State` borrow that is legal to the compiler and panics at
//! the moment a gesture ends. The palette's are here because a `Color` is a freya type;
//! they assert the properties a second set of values can silently break rather than the
//! values themselves.
use super::*;
// Named again: `use super::*` offers two `use_theme`s -- ours and freya's own out of the
// prelude -- and two globs offering one name is an ambiguity rather than a shadowing. An
// explicit import wins over a glob, so this is what the name means here: ours.
use super::settings_view::use_theme;
use freya_testing::TestingRunner;

/// Three rows wired exactly the way the two panes are: the press that starts a run, the
/// `pointer_over` that sweeps it, and the release watched globally at the root, because the
/// button very often comes up somewhere the run does not reach.
fn harness() -> impl IntoElement {
    let marked = use_consume::<Marked>().0;

    let row = |index: usize| {
        rect()
            .width(Size::fill())
            .height(Size::px(20.0))
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, false, Pane::Assembly, index);
                }
            })
            .on_pointer_over(move |_| mark_drag(marked, Pane::Assembly, index))
            .into_element()
    };

    rect()
        .expanded()
        .on_global_pointer_press(move |_| mark_release(marked))
        .child(row(0))
        .child(row(1))
        .child(row(2))
}

/// The five states [`scrolling_harness`] is wired to, as context types of their own so that
/// three `State<usize>`s cannot be confused for one another.
#[derive(Clone, Copy)]
struct KeptTab(State<String>);
#[derive(Clone, Copy)]
struct KeptAt(State<Positions<String>>);
/// The tabs that are open, which is what a position is only kept for.
#[derive(Clone, Copy)]
struct KeptOpen(State<Vec<String>>);
#[derive(Clone, Copy)]
struct KeptLength(State<usize>);
/// The last row the pointer was over, which is how the test asks where the view actually
/// is rather than believing what the map says about it.
#[derive(Clone, Copy)]
struct KeptTop(State<usize>);

/// A scroll view wired the way both **code** panes are: one `ScrollController` reused across
/// every tab the pane shows, `use_kept_position` between them, and [`code_row_height`] on
/// both halves of the view.
fn scrolling_harness() -> impl IntoElement {
    let tab = use_consume::<KeptTab>().0;
    let at = use_consume::<KeptAt>().0;
    let open = use_consume::<KeptOpen>().0;
    let length = use_consume::<KeptLength>().0;
    let mut top = use_consume::<KeptTop>().0;

    let controller = use_scroll_controller(ScrollConfig::default);
    let showing = tab.read().clone();
    let rows = *length.read();
    use_kept_position(
        at,
        move |tab: &String| open.peek().contains(tab),
        |_| false,
        controller,
        &showing,
        rows,
    );

    rect().expanded().child(
        VirtualScrollView::new_with_data_controlled(
            rows,
            move |index, _: &usize| {
                rect()
                    .width(Size::fill())
                    .height(Size::px(code_row_height()))
                    .on_pointer_over(move |_| top.set(index))
                    .key(index)
                    .into()
            },
            controller,
        )
        .length(rows)
        .item_size(code_row_height()),
    )
}

/// A sidebar list's shape: the same view over [`list_row_height`], and no kept position.
/// It exists so the agreement between an `item_size` and its rows is asserted for *both*
/// heights rather than for one and assumed for the other.
fn list_scrolling_harness() -> impl IntoElement {
    let mut top = use_consume::<KeptTop>().0;

    rect().expanded().child(
        VirtualScrollView::new_with_data(0usize, move |index, _: &usize| {
            rect()
                .width(Size::fill())
                .height(Size::px(list_row_height()))
                .on_pointer_over(move |_| top.set(index))
                .key(index)
                .into()
        })
        .length(100usize)
        .item_size(list_row_height()),
    )
}

/// Switching tab puts the pane back where that tab was left, and a tab seen for the first
/// time opens at the top rather than at the last one's offset. Headless because the
/// position is read out of a `ScrollController` inside an effect a scroll wakes, and is
/// asserted against which row a real `VirtualScrollView` put under the pointer.
#[test]
fn a_tab_comes_back_to_the_row_it_was_left_at() {
    let (mut test, (tab, at, open, _length, top)) = TestingRunner::new(
        scrolling_harness,
        (100., 100.).into(),
        |runner| {
            let tabs = vec!["a".to_owned(), "b".to_owned()];
            (
                runner
                    .provide_root_context(|| KeptTab(State::create("a".to_owned())))
                    .0,
                runner
                    .provide_root_context(|| KeptAt(State::create(Positions::default())))
                    .0,
                runner
                    .provide_root_context(|| KeptOpen(State::create(tabs)))
                    .0,
                runner
                    .provide_root_context(|| KeptLength(State::create(100)))
                    .0,
                runner.provide_root_context(|| KeptTop(State::create(0))).0,
            )
        },
        1.,
    );
    let mut tab = tab;
    test.sync_and_update();

    // Where the top of the view is, asked the only way a pane can be asked: the
    // pointer is moved away first, or entering the same row twice is no event at all.
    let top_row = |test: &mut TestingRunner| {
        // Settled first: the scroll an effect asks for lands a poll after the state change
        // that asked for it.
        for _ in 0..4 {
            test.sync_and_update();
        }
        test.move_cursor((50., 90.));
        test.sync_and_update();
        test.move_cursor((50., 5.));
        test.sync_and_update();
        *top.peek()
    };

    test.scroll((50., 50.), (0., -300.));
    test.sync_and_update();
    let left_at = top_row(&mut test);
    assert!(left_at > 0, "the wheel moved nothing");
    // The scroll was written down as it happened, which is what survives the window merely
    // being closed.
    assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

    // A tab this pane has never shown starts at the top, and pointedly not at the offset
    // the tab before it was at.
    tab.set("b".to_owned());
    test.sync_and_update();
    assert_eq!(top_row(&mut test), 0);
    // And the tab left behind is remembered, not overwritten by where the new one is.
    assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

    tab.set("a".to_owned());
    test.sync_and_update();
    assert_eq!(top_row(&mut test), left_at);

    // And closing the tab on screen does not put it back: `close_tab` forgets the position
    // and then moves to a neighbour, so the run that follows is holding a tab that is gone.
    let (mut open, mut at) = (open, at);
    open.write().retain(|tab| tab != "a");
    at.write().forget(&"a".to_owned());
    tab.set("b".to_owned());
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(at.peek().at(&"a".to_owned()), None);
}

/// [`scrolling_harness`] with the panes' reveal made through the kept position, as
/// theirs is: a `Pin` whose line is taken as a row index of whatever tab is shown.
fn revealing_harness() -> impl IntoElement {
    let tab = use_consume::<KeptTab>().0;
    let at = use_consume::<KeptAt>().0;
    let open = use_consume::<KeptOpen>().0;
    let length = use_consume::<KeptLength>().0;
    let mut top = use_consume::<KeptTop>().0;
    let pinned = use_consume::<Pinned>().0;

    let controller = use_scroll_controller(ScrollConfig::default);
    let showing = tab.read().clone();
    let rows = *length.read();
    use_kept_position(
        at,
        move |tab: &String| open.peek().contains(tab),
        move |controller: &mut ScrollController| {
            let Some(at) = owed_reveal(pinned, Pane::Assembly) else {
                return false;
            };
            reveal_made(pinned, Pane::Assembly);
            reveal_row(controller, 100.0, at.line as usize);
            true
        },
        controller,
        &showing,
        rows,
    );

    rect().expanded().child(
        VirtualScrollView::new_with_data_controlled(
            rows,
            move |index, _: &usize| {
                rect()
                    .width(Size::fill())
                    .height(Size::px(code_row_height()))
                    .on_pointer_over(move |_| top.set(index))
                    .key(index)
                    .into()
            },
            controller,
        )
        .length(rows)
        .item_size(code_row_height()),
    )
}

/// A reveal owed when the tab changes wins over where the tab was left: the two are
/// owed at once when a Locations row opens a symbol on a line, and the kept position
/// putting the arriving tab at its top would undo the scroll the reveal made.
#[test]
fn a_reveal_owed_when_the_tab_changes_wins_over_the_kept_position() {
    let (mut test, (tab, top, pinned)) = TestingRunner::new(
        revealing_harness,
        (100., 100.).into(),
        |runner| {
            let tabs = vec!["a".to_owned(), "b".to_owned()];
            runner.provide_root_context(|| KeptAt(State::create(Positions::default())));
            runner.provide_root_context(|| KeptOpen(State::create(tabs)));
            runner.provide_root_context(|| KeptLength(State::create(100)));
            (
                runner
                    .provide_root_context(|| KeptTab(State::create("a".to_owned())))
                    .0,
                runner.provide_root_context(|| KeptTop(State::create(0))).0,
                runner
                    .provide_root_context(|| Pinned(State::create(None)))
                    .0,
            )
        },
        1.,
    );
    let (mut tab, mut pinned) = (tab, pinned);
    test.sync_and_update();

    let top_row = |test: &mut TestingRunner| {
        for _ in 0..4 {
            test.sync_and_update();
        }
        test.move_cursor((50., 90.));
        test.sync_and_update();
        test.move_cursor((50., 5.));
        test.sync_and_update();
        *top.peek()
    };

    // Tab "a" scrolled somewhere; then, in one handler's worth of writes, a reveal owed
    // for a row of tab "b" and the switch to it.
    test.scroll((50., 50.), (0., -300.));
    assert!(top_row(&mut test) > 0, "the wheel moved nothing");
    pinned.set(Some(Pin {
        at: LinePos {
            file: "b.rs".into(),
            line: 40,
        },
        reveal: Owed::by(Pane::Assembly),
        landed: false,
    }));
    tab.set("b".to_owned());
    let landed = top_row(&mut test);
    assert!(
        (30..=40).contains(&landed),
        "the arriving tab was put at row {landed} rather than at the revealed row"
    );
    assert!(owed_reveal(pinned, Pane::Assembly).is_none());
}

fn area(groups: Vec<Vec<Tab>>) -> DockArea {
    DockArea::row(groups).with_documents(DOCUMENT_PANEL)
}

fn panels(area: &DockArea) -> Vec<PanelId> {
    fn walk(node: &DockNode<Tab, PanelId>, into: &mut Vec<PanelId>) {
        match node {
            DockNode::Panel(panel) => into.push(panel.panel_id),
            DockNode::Split { children, .. } => children.iter().for_each(|child| walk(child, into)),
        }
    }
    let mut found = Vec::new();
    walk(&area.tree, &mut found);
    found
}

/// The whole reason the panel is designated: freya's own sweep retains every non-empty
/// child with no exemption, so the panel documents live in would fold away the moment the
/// reader closed the last one.
#[test]
fn the_document_panel_survives_being_emptied() {
    let mut dock = area(vec![vec![], vec![Tab::View(View::Settings)]]);
    dock.tidy();
    assert_eq!(panels(&dock), [DOCUMENT_PANEL, 1]);
}

/// A view panel is not spared: emptying one folds it away, which is what keeps the
/// layout from filling up with the ghosts of panels the reader dragged out of.
#[test]
fn an_emptied_view_panel_folds_away() {
    let mut dock = area(vec![vec![Tab::View(View::Project)], vec![]]);
    dock.tidy();
    assert_eq!(panels(&dock), [DOCUMENT_PANEL]);
}

/// A document goes in the document panel and nowhere else. The drop is refused rather
/// than redirected, so the drag springs back to where it started.
#[test]
fn a_document_may_only_be_dropped_into_the_document_panel() {
    let mut dock = area(vec![vec![], vec![Tab::View(View::Settings)]]);
    let tab = Tab::Document(Docs::default().open(Document::Source(Arc::from("a.rs"))));

    assert!(!dock.accepts(tab, &DropTarget::Center(1)));
    assert!(!dock.accepts(
        tab,
        &DropTarget::Tab {
            panel_id: 1,
            position: 0
        }
    ));
    // A split always makes a new panel, which is never the designated one.
    assert!(!dock.accepts(
        tab,
        &DropTarget::Split {
            panel_id: DOCUMENT_PANEL,
            side: Side::Right
        }
    ));
    assert!(dock.accepts(tab, &DropTarget::Center(DOCUMENT_PANEL)));
    assert!(!dock.on_drop(tab, DropTarget::Center(1)));
}

/// And a view goes anywhere, the document panel included -- which is what keeps
/// Project, Settings and the Scratchpad tabbed beside the documents.
#[test]
fn a_view_may_be_dropped_anywhere() {
    let dock = area(vec![vec![], vec![Tab::View(View::Settings)]]);
    let tab = Tab::View(View::Project);
    assert!(dock.accepts(tab, &DropTarget::Center(DOCUMENT_PANEL)));
    assert!(dock.accepts(tab, &DropTarget::Center(1)));
    assert!(dock.accepts(
        tab,
        &DropTarget::Split {
            panel_id: 1,
            side: Side::Right
        }
    ));
}

/// Nothing on screen: what a project switch does is to the states. A runner all the same,
/// because a `State` needs a runtime and because a borrow held across a write is a runtime
/// panic rather than a compile error.
fn project_harness() -> impl IntoElement {
    rect().expanded()
}

/// The nine contexts `app()` provides, in one `ProjectStates`. A macro and not a
/// function: the runner's type is `freya_core::integration::Runner`, which freya's prelude
/// does not re-export, so naming it would mean naming a crate the app does not depend on.
macro_rules! project_states {
    () => {
        |runner: &mut _| project_states!(runner)
    };
    ($runner:expr) => {{
        // The two states that are what is open, and the derivation over them, in the same
        // order `app()` uses. `Active` is provided but not returned: it is not one of the
        // project's states, it is a reading of two of them.
        let dock = $runner
            .provide_root_context(|| {
                ContentDock(State::create(
                    DockArea::row(vec![vec![]]).with_documents(DOCUMENT_PANEL),
                ))
            })
            .0;
        let docs = $runner
            .provide_root_context(|| OpenDocs(State::create(Docs::default())))
            .0;
        $runner.provide_root_context(move || {
            Active(Memo::create(move || {
                active_document(&dock.read(), &docs.read())
            }))
        });

        ProjectStates {
            proj: $runner
                .provide_root_context(|| Proj(State::create(OpenProject::default())))
                .0,
            objects: $runner
                .provide_root_context(|| Objects(State::create(Vec::new())))
                .0,
            loading: $runner
                .provide_root_context(|| Loading(State::create(Loads::default())))
                .0,
            open: Open { dock, docs },
            asm_at: $runner
                .provide_root_context(|| AsmAt(State::create(Positions::default())))
                .0,
            driven: $runner
                .provide_root_context(|| Drives(State::create(Driven::default())))
                .0,
            src_at: $runner
                .provide_root_context(|| SrcAt(State::create(Positions::default())))
                .0,
            history: $runner
                .provide_root_context(|| Hist(State::create(History::default())))
                .0,
        }
    }};
}

/// Leaving a project leaves nothing of it behind: no object, no tab of either kind, no
/// viewing position, no history entry and nothing active.
///
/// Headless because `close_binary` and `close_tab` each read a state and then write it,
/// which is legal to the compiler and panics at the moment it runs if the read is still
/// borrowed. The source-driven tab is the case a binary close deliberately leaves
/// standing, so it is the one only this walk reaches.
#[test]
fn leaving_a_project_leaves_nothing_of_it_behind() {
    let symbols = fixture_symbols();
    let (first, second) = (symbols[0].clone(), symbols[1].clone());
    let object = first.object.clone();
    let source = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();

    // The app as a session leaves it: a binary open, two of its functions in the strip with
    // a row remembered for one of them, a source file open beside them, somewhere to go.
    let (mut objects, mut asm_at, mut src_at) = (states.objects, states.asm_at, states.src_at);
    objects.write().push(object.clone());
    let tab = |symbol: &Symbol| Document::Assembly(Selection::Symbol(symbol.clone()));
    let went = |target: Document| activate(states.open, states.history, Some(target), Visit::Went);
    went(tab(&first));
    went(tab(&second));
    went(source.clone());
    asm_at.write().remember(tab(&first), 12);
    src_at.write().remember(source.clone(), 7);
    test.sync_and_update();

    assert_eq!(states.open.documents().len(), 3);
    // Three visits, the source file included: the history records documents.
    assert_eq!(states.history.peek().entries().len(), 3);

    clear_project(states);
    test.sync_and_update();

    assert!(
        states.objects.peek().is_empty(),
        "an object was left behind"
    );
    assert!(states.open.documents().is_empty(), "a tab was left behind");
    assert!(
        states.history.peek().entries().is_empty(),
        "a history entry was left behind"
    );
    // Not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points into.
    assert_eq!(
        states.asm_at.peek().at(&tab(&first)),
        None,
        "a viewing position was left behind"
    );
    assert_eq!(
        states.src_at.peek().at(&source),
        None,
        "a source position was left behind"
    );
    assert!(
        states.open.active().is_none(),
        "the app still points into the project just left"
    );
}

/// The History panel and nothing else, over the project's states.
fn history_harness() -> impl IntoElement {
    rect().expanded().child(HistoryTab)
}

/// A row is named after the function and not after the whole of what the demangler said.
/// `entry_text` is the one spelling a tab and a history row share, and it is [`short_name`]
/// over the demangled name; the whole of it stays on the entry, where the tooltip and the
/// filter read it.
#[test]
fn a_history_row_names_the_function_and_not_the_whole_symbol() {
    let object = fixture_symbols()[0].object.clone();
    let demangled =
        "<viewer::ui::pad_view::ScratchpadTab as freya_core::element::Component>::render";
    let symbol = Symbol {
        object,
        data: Arc::new(SymbolData {
            name: "_RNvXsa_".to_owned(),
            demangled: Some(demangled.to_owned()),
            address: 0x1000,
            section: None,
            size: 0,
        }),
    };

    let (mut test, states) =
        TestingRunner::new(history_harness, (400., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    activate(
        states.open,
        states.history,
        Some(Document::Assembly(Selection::Symbol(symbol))),
        Visit::Went,
    );
    test.sync_and_update();

    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "ScratchpadTab::render"),
        "the row is not named after the function: {drawn:?}"
    );
    assert!(
        !drawn
            .iter()
            .any(|text| text.starts_with("<viewer::ui::pad_view")),
        "the whole demangled name is on screen: {drawn:?}"
    );
}

/// Nothing but the overflow control, so the press has one thing to land on.
fn menu_harness() -> impl IntoElement {
    rect()
        .expanded()
        .horizontal()
        .content(Content::Flex)
        .child(rect().width(Size::flex(1.0)).height(Size::px(25.0)))
        .child(DocumentMenuButton)
}

/// And it stays hanging from that edge when the list grows underneath it. `MenuContainer`
/// measures itself once and keeps the offset it worked out then (`menu.rs:236`), so a menu
/// that widens after that hangs off the side of the window -- by 315px here. Not a
/// contrived case: the tab list fills in from a worker.
#[test]
fn a_menu_open_while_the_list_grows_stays_on_the_edge() {
    let (mut test, states) =
        TestingRunner::new(menu_harness, (600., 300.).into(), project_states!(), 1.);
    test.sync_and_update();

    // The app's panel always holds the three views, so the button is there before any
    // document is.
    {
        let mut dock = states.open.dock;
        let mut dock = dock.write();
        let panel = dock.document_panel_mut().expect("the document panel");
        panel.tabs.push(Tab::View(View::Project));
        panel.active_tab_id = Some(Tab::View(View::Project));
    }
    test.sync_and_update();

    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == DOCUMENT_MENU_WIDTH).then_some(area)
        })
        .expect("the button is in the bar");
    let button_right = button.origin.x + button.width();

    let at = ((button.origin.x + 10.0) as f64, 10.0);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.sync_and_update();
    test.sync_and_update();

    // A row far wider than the three-view menu, arriving after it is on screen.
    activate(
        states.open,
        states.history,
        Some(Document::Source(Arc::from(
            "/x/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.rs",
        ))),
        Visit::Went,
    );
    for _ in 0..6 {
        test.sync_and_update();
    }

    let popup: Vec<_> = test.find_many(|node, _| {
        let area = node.layout().area;
        (area.origin.y == list_row_height()).then_some(area)
    });
    let left = popup
        .iter()
        .map(|area| area.origin.x)
        .fold(f32::MAX, f32::min);
    let width = popup
        .iter()
        .map(|area| area.width())
        .fold(0.0_f32, f32::max);
    assert!(
        width > DOCUMENT_MENU_WIDTH * 4.0,
        "the menu did not grow to hold the new row"
    );
    assert_eq!(
        left + width,
        button_right,
        "the widened menu hangs {} off the button's right edge",
        left + width - button_right
    );
}

/// The menu hangs from the button's right-hand edge rather than off the side of the window,
/// and is positioned *vertically only*. `MenuContainer` corrects its own overflow and
/// latches the first position it is measured at (`menu.rs:236`), so a `right(0.)` of ours
/// lands it on the button's edge and freya's correction then shifts it a whole menu-width
/// further left. The harness puts the button hard against the right edge, which is the
/// only place the correction fires.
#[test]
fn the_tab_menu_hangs_from_the_buttons_right_edge() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let (mut test, states) =
        TestingRunner::new(menu_harness, (600., 300.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);
    for symbol in symbols.iter().take(2) {
        activate(
            states.open,
            states.history,
            Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
            Visit::Went,
        );
    }
    test.sync_and_update();

    // The button is the only thing in the bar, so it is the one box of its own width.
    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == DOCUMENT_MENU_WIDTH).then_some(area)
        })
        .expect("the button is in the bar");
    let button_right = button.origin.x + button.width();

    let at = ((button.origin.x + 10.0) as f64, 10.0);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.sync_and_update();
    test.sync_and_update();

    // Everything the popup is made of hangs below the bar. The leftmost of those is
    // the container as it was actually placed, corrections included.
    let popup: Vec<_> = test.find_many(|node, _| {
        let area = node.layout().area;
        (area.origin.y == list_row_height()).then_some(area)
    });
    assert!(!popup.is_empty(), "the press did not open the menu");

    let left = popup
        .iter()
        .map(|area| area.origin.x)
        .fold(f32::MAX, f32::min);
    let width = popup
        .iter()
        .map(|area| area.width())
        .fold(0.0_f32, f32::max);
    assert_eq!(
        left + width,
        button_right,
        "the menu is {} off the button's right edge",
        button_right - (left + width)
    );
}

/// The overflow menu opens on a press and closes on the next one. The assertion is that
/// the element tree *grew*: what is checked is that the control does anything at all.
///
/// It needs no guard against `Menu`'s close-on-any-global-press, where `ContextMenu` has
/// one: the listeners for a global event are collected before a single handler runs, and
/// this opens on `on_press`, derived from the very `MouseUp` that emits the global press.
#[test]
fn the_document_menu_opens_and_closes() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();

    let (mut test, states) =
        TestingRunner::new(menu_harness, (400., 200.).into(), project_states!(), 1.);
    test.sync_and_update();

    let mut objects = states.objects;
    objects.write().push(object);
    for symbol in symbols.iter().take(2) {
        activate(
            states.open,
            states.history,
            Some(Document::Assembly(Selection::Symbol(symbol.clone()))),
            Visit::Went,
        );
    }
    test.sync_and_update();

    let nodes = |test: &TestingRunner| test.find_many(|_, _| Some(())).len();
    let shut = nodes(&test);

    // The button sits at the right-hand end of the bar, where the tab bar puts it.
    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == DOCUMENT_MENU_WIDTH).then_some(area)
        })
        .expect("the button is in the bar");
    let at = ((button.origin.x + 10.0) as f64, 10.0);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.sync_and_update();
    let open = nodes(&test);
    assert!(
        open > shut,
        "the press did not open the menu: {shut} nodes before, {open} after"
    );

    // And the next press shuts it, which is the other half of the same guard.
    test.press_cursor(at);
    test.release_cursor(at);
    test.sync_and_update();
    assert_eq!(nodes(&test), shut, "the menu did not close");
}

/// The toolbar's two history buttons and nothing else, abutting at the window's corner so
/// the test can work out where each is from [`toggle_size`] alone.
fn nav_harness() -> impl IntoElement {
    rect()
        .expanded()
        .horizontal()
        .child(NavButton { back: true })
        .child(NavButton { back: false })
}

/// The x each button was laid out at, back first. Deduplicated: a `TooltipContainer` wraps
/// its child in a box of the child's own size, so every button is two nodes of one square.
fn nav_button_columns(test: &TestingRunner) -> Vec<f32> {
    let side = toggle_size();
    let mut columns: Vec<f32> = test.find_many(|node, _| {
        let area = node.layout().area;
        (area.width() == side && area.height() == side).then_some(area.origin.x)
    });
    columns.dedup();
    columns
}

/// Whether the button at `at` washes under the pointer, which is the only half of "drawn
/// but disabled" the runner can be asked about: the chevron's own colour is baked into a
/// rasterised SVG and is not in the element tree at all. The pointer is taken off the pair
/// again, so the next question starts from nothing hovered.
fn washes_under_the_pointer(test: &mut TestingRunner, at: (f64, f64)) -> bool {
    test.move_cursor(at);
    test.sync_and_update();
    let washed = test
        .find(|_, element| {
            (element.style().background == Fill::Color(Palette::LIGHT.toggle_hover_bg))
                .then_some(())
        })
        .is_some();

    test.move_cursor((0.0, 90.0));
    test.sync_and_update();
    washed
}

fn press_at(test: &mut TestingRunner, at: (f64, f64)) {
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.sync_and_update();
}

/// The toolbar's buttons step the history, a button with nothing in its direction takes no
/// press, and both of them follow the cursor the other one just moved.
///
/// Headless because the pair is nothing but a reading of `Hist`: that a press reaches
/// `navigate` at all, that a dimmed button is inert, and that a button repaints because the
/// cursor moved and not because anything re-rendered it are three claims about the wiring
/// and none of them is visible to a unit test.
#[test]
fn the_toolbar_buttons_step_the_history_and_follow_the_cursor() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();

    let (mut test, states) =
        TestingRunner::new(nav_harness, (200., 100.).into(), project_states!(), 1.);
    test.sync_and_update();

    let mut objects = states.objects;
    objects.write().push(object);
    let documents: Vec<Document> = symbols
        .iter()
        .take(3)
        .map(|symbol| Document::Assembly(Selection::Symbol(symbol.clone())))
        .collect();
    for document in &documents {
        activate(
            states.open,
            states.history,
            Some(document.clone()),
            Visit::Went,
        );
    }
    test.sync_and_update();
    assert_eq!(states.history.peek().cursor(), Some(2));

    let side = toggle_size();
    let columns = nav_button_columns(&test);
    assert_eq!(columns.len(), 2, "both buttons are in the bar");
    let at = |x: f32| ((x + side / 2.0) as f64, (side / 2.0) as f64);
    let (back, forward) = (at(columns[0]), at(columns[1]));

    assert!(
        washes_under_the_pointer(&mut test, back),
        "back has two entries behind it and is drawn dead"
    );
    assert!(
        !washes_under_the_pointer(&mut test, forward),
        "there is nothing in front of the newest entry"
    );

    press_at(&mut test, back);
    assert_eq!(states.history.peek().cursor(), Some(1));
    assert!(
        states.open.active().as_ref() == Some(&documents[1]),
        "the step back did not land on the entry before it"
    );
    // Nothing re-rendered this button: it read the cursor the press beside it moved.
    assert!(
        washes_under_the_pointer(&mut test, forward),
        "the forward button did not follow the cursor"
    );

    press_at(&mut test, back);
    assert_eq!(states.history.peek().cursor(), Some(0));
    assert!(
        !washes_under_the_pointer(&mut test, back),
        "back is on the oldest entry and still looks live"
    );

    // A press on a dimmed button is not a press at all.
    press_at(&mut test, back);
    assert_eq!(
        states.history.peek().cursor(),
        Some(0),
        "a dimmed button navigated"
    );

    press_at(&mut test, forward);
    assert_eq!(states.history.peek().cursor(), Some(1));
    assert!(
        states.open.active().as_ref() == Some(&documents[1]),
        "the step forward did not land on the entry after it"
    );
}

/// A button with nothing in its direction keeps its box: dimmed rather than hidden, so the
/// pair does not shuffle under the pointer as the reader walks the history, and a reader
/// who has been nowhere yet can still see that it is there.
#[test]
fn a_history_button_with_nowhere_to_go_is_still_drawn() {
    let (mut test, _states) =
        TestingRunner::new(nav_harness, (200., 100.).into(), project_states!(), 1.);
    test.sync_and_update();

    let side = toggle_size();
    assert_eq!(
        nav_button_columns(&test),
        vec![0.0, side],
        "an empty history left a button out of the bar"
    );
}

/// The × on a document's tab, mounted on its own: how big a target it is and what the
/// pointer does to it are questions about the control rather than about the strip around
/// it. It takes its document the way a header does, from the panel's first document tab.
fn close_harness() -> impl IntoElement {
    let open = use_open();
    let id = open.dock.read().document_panel().and_then(|panel| {
        panel.tabs.iter().find_map(|tab| match tab {
            Tab::Document(id) => Some(*id),
            Tab::View(_) => None,
        })
    });

    rect()
        .expanded()
        .maybe_child(id.map(|id| TabClose { id }.into_element()))
}

/// One open document, and the × that closes it as it was actually laid out -- asserting on
/// the way past that the glyph has air around it, which is the whole of what the target is.
/// A comparison between the target and the glyph inside it and never an absolute width: the
/// second would be an assertion about the fonts on whoever ran it.
fn one_close_target(test: &mut TestingRunner, states: &ProjectStates) -> Area {
    let symbols = fixture_symbols();
    let document = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    activate(states.open, states.history, Some(document), Visit::Went);
    test.sync_and_update();

    let target = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == close_target() && area.height() == close_target()).then_some(area)
        })
        .expect("the × is a target of its own");

    // The only thing narrower than the target is the glyph centred in it; everything above
    // it is the harness, at the window's own width.
    let glyph = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() < target.width()).then_some(area)
        })
        .expect("the × draws a glyph");
    let (left, right) = (
        glyph.origin.x - target.origin.x,
        target.max_x() - glyph.max_x(),
    );
    assert!(
        left >= 2.0 && right >= 2.0,
        "the glyph fills its target: {left} left of it and {right} right of it"
    );

    target
}

/// A press that lands in the target but nowhere near the glyph still closes the tab: what
/// grew is the padding around the ×, not the × itself. The offset is measured from the
/// target's own centre -- which is the glyph's -- so the assertion is about how much room
/// there is around the glyph and not about where the control happens to sit.
#[test]
fn a_press_beside_the_glyph_still_closes_the_tab() {
    let (mut test, states) =
        TestingRunner::new(close_harness, (200., 100.).into(), project_states!(), 1.);
    test.sync_and_update();
    let target = one_close_target(&mut test, &states);

    let centre = (
        (target.origin.x + target.width() / 2.0) as f64,
        (target.origin.y + target.height() / 2.0) as f64,
    );
    let press = |test: &mut TestingRunner, at: (f64, f64)| {
        test.move_cursor(at);
        test.press_cursor(at);
        test.release_cursor(at);
        test.sync_and_update();
    };

    // Past the target's own edge is not the tab's business, which is what says the close
    // below is the target answering rather than any press anywhere.
    press(&mut test, (centre.0 + close_target() as f64, centre.1));
    assert_eq!(
        states.open.docs.peek().len(),
        1,
        "a press outside the × closed the tab"
    );

    // And two pixels inside its left edge, which is the padding and not the glyph.
    press(
        &mut test,
        (centre.0 - close_target() as f64 / 2.0 + 2.0, centre.1),
    );
    assert_eq!(
        states.open.docs.peek().len(),
        0,
        "a press in the × missed the glyph and did nothing"
    );
    let documents = states
        .open
        .dock
        .peek()
        .document_panel()
        .map(|panel| panel.tabs.len())
        .unwrap_or_default();
    assert_eq!(documents, 0, "the tab outlived the document it stood for");
}

/// The × lights under the pointer in a wash of its own, and puts itself back when the
/// pointer moves off it. freya has no hover pseudo-state, so this is the control's own
/// `use_state` and the pair of `over`/`out` handlers around it; that the wash is louder
/// than the tab's own is `every_wash_reads_against_the_pane_under_it`.
#[test]
fn the_close_target_lights_under_the_pointer() {
    let (mut test, states) =
        TestingRunner::new(close_harness, (200., 100.).into(), project_states!(), 1.);
    test.sync_and_update();
    let target = one_close_target(&mut test, &states);

    // The one paintable thing in the harness is the target itself, so "anything painted"
    // is the wash and nothing else.
    let wash = |test: &TestingRunner| {
        test.find(|_, element| {
            let background = element.style().background.clone();
            (background != Fill::Color(Color::TRANSPARENT)).then_some(background)
        })
    };
    assert_eq!(
        wash(&test),
        None,
        "the × is lit before the pointer is on it"
    );

    let centre = (
        (target.origin.x + target.width() / 2.0) as f64,
        (target.origin.y + target.height() / 2.0) as f64,
    );
    test.move_cursor(centre);
    test.sync_and_update();
    assert_eq!(
        wash(&test),
        Some(Fill::Color(palette().close_hover_bg)),
        "the pointer on the × did not light it"
    );

    // Off the target and still inside the window: a wash that outlived the pointer would
    // be `out` never having run.
    test.move_cursor((centre.0 + close_target() as f64, centre.1));
    test.sync_and_update();
    assert_eq!(wash(&test), None, "the wash outlived the pointer");
}

/// A document has a tab in the panel exactly while it has an entry in the table, which is
/// what makes "the panel's `tabs` vec is the list of open documents" true without a second
/// list. `use_kept_position` leans on it directly.
#[test]
fn the_panel_and_the_table_hold_the_same_documents() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let documents: Vec<Document> = symbols
        .iter()
        .take(2)
        .map(|symbol| Document::Assembly(Selection::Symbol(symbol.clone())))
        .chain([Document::Source(Arc::from("/src/main.rs"))])
        .collect();

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    let agree = |states: &ProjectStates| {
        let open = states.open.documents();
        assert_eq!(
            open.len(),
            states.open.docs.peek().len(),
            "the panel and the table hold different numbers of documents"
        );
        open
    };

    for document in &documents {
        activate(
            states.open,
            states.history,
            Some(document.clone()),
            Visit::Went,
        );
    }
    test.sync_and_update();
    assert!(agree(&states) == documents);

    // Opening one that is already open adds neither a tab nor an entry.
    activate(
        states.open,
        states.history,
        Some(documents[0].clone()),
        Visit::Went,
    );
    test.sync_and_update();
    assert!(agree(&states) == documents);

    for document in &documents {
        close_tab(
            states.open,
            states.history,
            states.asm_at,
            states.src_at,
            states.driven,
            document,
        );
    }
    test.sync_and_update();
    assert!(agree(&states).is_empty());
}

/// Closing a tab lands on the one to its right, where freya would land on the leftmost:
/// `DockNode::remove_tab_except` sets the active tab to `tabs.first()`, so the removal is
/// done by hand and the landing chosen with [`tabs::landing`].
#[test]
fn closing_a_document_lands_on_its_right_hand_neighbour() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let documents: Vec<Document> = symbols
        .iter()
        .take(3)
        .map(|symbol| Document::Assembly(Selection::Symbol(symbol.clone())))
        .collect();

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    for document in &documents {
        activate(
            states.open,
            states.history,
            Some(document.clone()),
            Visit::Went,
        );
    }
    // On the middle one, whose neighbours are on both sides -- the only arrangement
    // in which "the right-hand one" and "the leftmost one" are different answers.
    activate(
        states.open,
        states.history,
        Some(documents[1].clone()),
        Visit::Moved,
    );
    test.sync_and_update();

    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        &documents[1],
    );
    test.sync_and_update();
    assert!(
        states.open.active() == Some(documents[2].clone()),
        "a close landed on the leftmost tab rather than the neighbour"
    );

    // And closing the last one moves left, there being nothing to its right.
    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        &documents[2],
    );
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[0].clone()));

    // Closing the only one left leaves nothing active at all.
    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        &documents[0],
    );
    test.sync_and_update();
    assert!(states.open.active().is_none());
}

/// "Close other tabs" keeps the tab it was opened on and nothing else of the documents,
/// lands on it when the one on screen is among those closing, lets go of the closed tabs'
/// kept positions -- an assembly document's key holds the `Arc<Object>` it points into --
/// and leaves a view sharing the panel alone, a view not being a document.
#[test]
fn closing_the_other_tabs_keeps_the_one_it_was_opened_on() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let documents: Vec<Document> = symbols
        .iter()
        .take(3)
        .map(|symbol| Document::Assembly(Selection::Symbol(symbol.clone())))
        .collect();

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    for document in &documents {
        activate(
            states.open,
            states.history,
            Some(document.clone()),
            Visit::Went,
        );
    }
    // A view dragged into the document panel, which the dock allows and this must not
    // close: the × it has no place for is the whole of the argument.
    {
        let mut dock = states.open.dock;
        let mut dock = dock.write();
        let panel = dock.document_panel_mut().expect("the document panel");
        panel.tabs.push(Tab::View(View::History));
    }
    let mut asm_at = states.asm_at;
    for (row, document) in documents.iter().enumerate() {
        asm_at.write().remember(document.clone(), row + 1);
    }
    test.sync_and_update();

    // Opened on the middle tab while the last one is the tab on screen, so the landing is
    // a move and not a tab simply staying where it was.
    let keep = states
        .open
        .docs
        .peek()
        .id_of(&documents[1])
        .expect("the kept tab is open");
    close_others(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        keep,
    );
    test.sync_and_update();

    assert!(states.open.documents() == documents[1..2]);
    assert!(
        states.open.active() == Some(documents[1].clone()),
        "the tab on screen closed without landing on the one that was kept"
    );
    assert_eq!(
        states.asm_at.peek().at(&documents[1]),
        Some(2),
        "the kept tab lost the row it was left at"
    );
    assert!(
        states.asm_at.peek().at(&documents[0]).is_none()
            && states.asm_at.peek().at(&documents[2]).is_none(),
        "a closed tab's position was kept, and with it the binary it points into"
    );
    assert!(
        states
            .open
            .dock
            .peek()
            .document_panel()
            .is_some_and(|panel| panel.tabs.contains(&Tab::View(View::History))),
        "a view in the document panel was closed with the documents"
    );
}

/// The history records where the reader *went* and not what is on screen: opening a
/// document is a visit, switching to an open tab is not, and the neighbour a close lands
/// on is not either. An effect observing the active document could not tell them apart.
#[test]
fn switching_to_an_open_tab_is_not_a_visit() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let (first, second) = (
        Document::Assembly(Selection::Symbol(symbols[0].clone())),
        Document::Assembly(Selection::Symbol(symbols[1].clone())),
    );

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();

    let mut objects = states.objects;
    objects.write().push(object);
    let go = |target: &Document, visit| {
        activate(states.open, states.history, Some(target.clone()), visit)
    };

    go(&first, Visit::Went);
    go(&second, Visit::Went);
    test.sync_and_update();
    assert!(states.history.peek().entries() == [first.clone(), second.clone()]);

    // Back to the first through the strip: it is already open, so the reader has gone
    // nowhere and the cursor stays where it was.
    go(&first, Visit::Moved);
    test.sync_and_update();
    assert!(states.open.active() == Some(first.clone()));
    assert!(
        states.history.peek().entries() == [first.clone(), second.clone()],
        "a strip click was recorded as a visit"
    );
    assert_eq!(states.history.peek().cursor(), Some(1));

    // Going there deliberately *is* one, and bumps it to the newest position.
    go(&first, Visit::Went);
    test.sync_and_update();
    assert!(states.history.peek().entries() == [second, first.clone()]);

    // And closing the tab lands on the neighbour without recording it.
    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        &first,
    );
    test.sync_and_update();
    assert_eq!(states.open.documents().len(), 1);
    assert_eq!(
        states.history.peek().entries().len(),
        2,
        "closing a tab recorded the neighbour it landed on"
    );
}

/// Closing a binary takes its own tabs and leaves a source-driven one standing: a file tab
/// outlives the binary that led the reader to it. Worth a runner because what has to hold
/// is that `close_binary` lands the *active* document somewhere sensible.
#[test]
fn closing_a_binary_keeps_the_source_tabs() {
    let symbols = fixture_symbols();
    let symbol = symbols[0].clone();
    let object = symbol.object.clone();
    let path = object.path.clone();
    let source = Document::Source(Arc::from("/src/main.rs"));
    let function = Document::Assembly(Selection::Symbol(symbol));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();

    let mut objects = states.objects;
    objects.write().push(object);
    let went = |target: Document| activate(states.open, states.history, Some(target), Visit::Went);
    went(source.clone());
    went(function.clone());
    test.sync_and_update();
    assert_eq!(states.open.documents().len(), 2);

    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.driven,
        states.history,
        &path,
    );
    test.sync_and_update();

    assert!(
        states.open.documents() == [source.clone()],
        "the file tab went with the binary"
    );
    assert!(
        states.open.active() == Some(source),
        "closing the binary did not land on the tab that survived it"
    );
}

/// The channel a load test feeds by hand, and the paths the harness registers as being
/// read, standing in for `open_binaries`' worker thread. The receiver is *taken* rather
/// than cloned: a clone left in the context map would keep the channel open for ever, and
/// the test could never see `take_load` returning and dropping the last receiver.
#[derive(Clone)]
struct Feed(
    Arc<Mutex<Option<async_channel::Receiver<Progress>>>>,
    Arc<Vec<PathBuf>>,
);

/// The real `take_load` over the real Objects tree, with the worker replaced by [`Feed`].
/// The tree is mounted so these tests also build the rows for a file being read --
/// including the row with no group behind it, which no other test reaches.
fn load_harness() -> impl IntoElement {
    let objects = use_consume::<Objects>().0;
    let loading = use_consume::<Loading>().0;
    let feed = use_consume::<Feed>().clone();

    use_hook(move || {
        let Feed(events, paths) = feed;
        let events = events
            .lock()
            .expect("the feed is not poisoned")
            .take()
            .expect("the harness is mounted once");
        // Bound out of its own statement, so the guard is gone before anything else
        // touches the state.
        let id = {
            let mut loading = loading;
            loading.write().begin(&paths)
        };
        spawn(async move { take_load(objects, loading, id, events).await });
    });

    rect().expanded().child(ObjectsTab)
}

/// `n` objects that all came out of one path, which is what an archive's members look like
/// above the analysis crate. Parsed `n` times rather than cloned, so they are `n` distinct
/// `Arc`s exactly as real members are.
fn fixture_objects(n: usize) -> (PathBuf, Vec<Arc<Object>>) {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = (0..n)
        .map(|_| {
            analysis::open_files(vec![path.clone()])
                .first()
                .expect("the fixture parses")
                .clone()
        })
        .collect();
    (path, objects)
}

/// Mount [`load_harness`] over one path, and hand back the states and the sender the
/// test plays the worker with.
fn mount_load(
    path: &Path,
) -> (
    TestingRunner,
    ProjectStates,
    async_channel::Sender<Progress>,
) {
    let (sender, events) = async_channel::unbounded::<Progress>();
    let paths = Arc::new(vec![path.to_path_buf()]);
    let events = Arc::new(Mutex::new(Some(events)));
    let (test, states) = TestingRunner::new(
        load_harness,
        (300., 300.).into(),
        move |runner| {
            runner.provide_root_context(|| Feed(events.clone(), paths.clone()));
            project_states!(runner)
        },
        1.,
    );
    (test, states, sender)
}

/// How the Objects tree describes what is on screen: a file being read has a row before it
/// has an object, and stops saying so when the last of them has landed.
fn reading(states: &ProjectStates) -> Vec<(String, usize, bool)> {
    let tree = ObjectTree::new(
        &states.objects.peek(),
        &states.loading.peek(),
        &Filter::default().matcher(),
        &HashSet::new(),
    );
    (0..tree.len())
        .filter_map(|row| match tree.row(row) {
            TreeRow::File {
                name,
                members,
                loading,
                ..
            } => Some((name.clone(), *members, *loading)),
            TreeRow::Object { .. } => None,
        })
        .collect()
}

/// The objects of one file reach the sidebar one at a time, and the row for that file is
/// there before the first of them is.
#[test]
fn objects_reach_the_sidebar_as_they_are_parsed() {
    let (path, objects) = fixture_objects(3);
    let (mut test, states, sender) = mount_load(&path);
    test.sync_and_update();

    // Before a single byte has been parsed, which nothing could be in while the parse
    // handed back one `Vec` at the end.
    assert_eq!(reading(&states), [("line_fixture.o".to_owned(), 0, true)]);
    assert!(states.objects.peek().is_empty());

    for (arrived, object) in objects.iter().enumerate() {
        sender
            .send_blocking(Progress::Parsed(object.clone()))
            .expect("the app is still listening");
        pump(&mut test, || states.objects.peek().len() == arrived + 1);
        assert_eq!(
            reading(&states),
            [("line_fixture.o".to_owned(), arrived + 1, true)],
            "the file stopped saying it was being read before it was finished"
        );
        // The save side: the path joins the binaries with its first object, so a session
        // written half way through a parse names the file.
        assert_eq!(project::binaries(&states.objects.peek()), [path.clone()]);
    }

    sender
        .send_blocking(Progress::Finished(path.clone()))
        .expect("the app is still listening");
    pump(&mut test, || !states.loading.peek().is_loading(&path));

    // Done, so the ordinary rules take over: three objects out of one file is an
    // archive-shaped row.
    assert_eq!(reading(&states), [("line_fixture.o".to_owned(), 3, false)]);
}

/// Closing a file half way through reading it takes the objects that have already arrived
/// *and* the ones that have not. The second half is what needs a test: the worker is
/// already parsing when the row is closed.
#[test]
fn a_file_closed_while_it_is_read_takes_the_rest_of_its_objects_with_it() {
    let (path, objects) = fixture_objects(2);
    let (mut test, states, sender) = mount_load(&path);
    test.sync_and_update();

    sender
        .send_blocking(Progress::Parsed(objects[0].clone()))
        .expect("the app is still listening");
    pump(&mut test, || states.objects.peek().len() == 1);

    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.driven,
        states.history,
        &path,
    );
    test.sync_and_update();
    assert!(states.objects.peek().is_empty());
    assert!(reading(&states).is_empty(), "a closed file is still a row");

    // The answer that was already on its way.
    sender
        .send_blocking(Progress::Parsed(objects[1].clone()))
        .expect("the worker has not been told yet");
    for _ in 0..8 {
        test.sync_and_update();
    }
    assert!(
        states.objects.peek().is_empty(),
        "an object arrived for a file that had been closed"
    );

    // And the worker is told, by the only thing that can tell it: the receiver is gone, so
    // its next send fails and the walk stops.
    assert!(sender.send_blocking(Progress::Finished(path)).is_err());
}

/// Leaving a project while one of its files is being read. The load is cancelled by
/// `clear_project` itself and not through `close_binary`, a file that has produced
/// nothing yet not being in the objects list for the per-path walk to reach.
#[test]
fn leaving_a_project_while_a_file_is_read_drops_what_was_still_coming() {
    let (path, objects) = fixture_objects(2);
    let (mut test, states, sender) = mount_load(&path);
    test.sync_and_update();

    clear_project(states);
    test.sync_and_update();
    assert!(states.loading.peek().paths().is_empty());
    assert!(reading(&states).is_empty());

    sender
        .send_blocking(Progress::Parsed(objects[0].clone()))
        .expect("the worker has not been told yet");
    for _ in 0..8 {
        test.sync_and_update();
    }
    assert!(
        states.objects.peek().is_empty(),
        "the project just left got an object out of the load it abandoned"
    );
    assert!(sender
        .send_blocking(Progress::Parsed(objects[1].clone()))
        .is_err());
}

/// The Objects tree and nothing else, at whatever width the window is. A pane's width is
/// all a row of it knows about the split, so the window is the split here.
fn objects_harness() -> impl IntoElement {
    rect().expanded().child(ObjectsTab)
}

/// Where the archive row's name and its member count were laid out, in a pane `width`
/// wide. The runner is gone by the time the areas are handed back, so two widths are two
/// mounts and never two runners at once.
fn archive_row(width: f32, objects: &[Arc<Object>]) -> (Area, Area) {
    let (mut test, mut states) = TestingRunner::new(
        objects_harness,
        (width, 300.).into(),
        |runner| project_states!(runner),
        1.,
    );
    states.objects.write().extend(objects.iter().cloned());
    settle(&mut test);
    (
        label_area(&test, "line_fixture.o").expect("the archive has a row"),
        label_area(&test, "3").expect("the row says how many members it has"),
    )
}

/// A sidebar dragged narrow is taken out of the *name*. The member count keeps the width
/// its digits need and the gutter beside them; the name is the row's flex child and gives
/// up every pixel of the difference, which is what leaves the name ellipsised rather than
/// the count pushed past the row's edge and clipped away with it.
///
/// The two mounts are compared against each other and never against a number of their
/// own: text is really shaped here, so what a digit measures is whatever fonts the
/// machine running the test has.
#[test]
fn a_narrow_sidebar_ellipsises_the_name_and_keeps_the_count() {
    let (_path, objects) = fixture_objects(3);
    let (wide_name, wide_count) = archive_row(300.0, &objects);
    let (name, count) = archive_row(150.0, &objects);

    assert_eq!(
        count.width(),
        wide_count.width(),
        "the count was squeezed by a narrower pane"
    );
    assert!(
        (wide_name.width() - name.width() - 150.0).abs() < 0.01,
        "the name gave up {} of the 150 the pane lost",
        wide_name.width() - name.width()
    );
    // The row is `Overflow::Clip` over a 5px horizontal padding, so a count past that edge
    // is a count the reader never sees.
    assert!(
        count.max_x() <= 145.01,
        "the count was pushed past the row's edge, where its clip takes it"
    );
    assert!(
        count.min_x() - name.max_x() >= COUNT_GUTTER,
        "the ellipsised name ran into the digits"
    );
}

/// The analysis worker's work, handed in through a context so a test can substitute one
/// that stops when it is told to.
#[derive(Clone)]
struct Work(Arc<dyn Fn(Question) -> Answer + Send + Sync>);

/// Every distinct symbol the panes were told to draw, in order: what the superseding rule
/// is about is what was *never* on screen, and only a recording can say that.
#[derive(Clone, Copy)]
struct Seen(State<Vec<Symbol>>);

/// The question as the analysis tests drive it. Deliberately not [`Asked`], which in the
/// app is a memo over the dock beside the driven lines: these tests have no business
/// building a dock to say what is being asked, and `use_analysis_with` takes anything
/// that reads and peeks.
#[derive(Clone, Copy)]
struct Wanted(State<Option<Ask>>);

/// The analysis wiring and nothing else: no panes, since what is under test is which
/// answers reach them rather than what they draw.
fn analysis_harness() -> impl IntoElement {
    let asking = use_consume::<Wanted>().0;
    let analysis = use_consume::<Analysis>().0;
    let objects = use_consume::<Objects>().0;
    let history = use_consume::<Hist>().0;
    let work = use_consume::<Work>().0;
    let mut seen = use_consume::<Seen>().0;
    let located = use_consume::<Locations>().0;

    use_analysis_with(
        asking,
        objects,
        history,
        analysis,
        located,
        move |question| work(question),
    );

    use_side_effect(move || {
        let shown = analysis.read().shown.clone();
        let Some(shown) = shown else {
            return;
        };
        // `peek` on the state it writes, or the effect would wake itself for ever.
        let repeat = seen
            .peek()
            .last()
            .is_some_and(|last| *last == shown.studied.symbol);
        if !repeat {
            seen.write().push(shown.studied.symbol);
        }
    });

    rect().expanded()
}

/// The states an analysis test drives the harness through, provided in the order `app()`
/// provides them. A macro for [`project_states!`]'s reason.
macro_rules! analysis_states {
    ($runner:expr, $work:expr) => {{
        $runner.provide_root_context(|| Work(Arc::new($work)));
        (
            $runner
                .provide_root_context(|| Wanted(State::create(None)))
                .0,
            $runner
                .provide_root_context(|| Analysis(State::create(Analyzed::default())))
                .0,
            $runner
                .provide_root_context(|| Seen(State::create(Vec::new())))
                .0,
            $runner
                .provide_root_context(|| Objects(State::create(Vec::new())))
                .0,
            $runner
                .provide_root_context(|| Hist(State::create(History::default())))
                .0,
            $runner
                .provide_root_context(|| Locations(State::create(Located::default())))
                .0,
        )
    }};
}

/// Run the test runner until `ready` answers, and then a little further so that whatever
/// the answer woke has run too. A worker thread and two channels sit between a state
/// change and the state it ends in, so how many turns that takes is not something a test
/// can know, only that it is finite. Failing loudly, since "the answer never came" and
/// "the answer was wrong" are different bugs.
fn pump(test: &mut TestingRunner, ready: impl Fn() -> bool) {
    for _ in 0..200 {
        test.sync_and_update();
        if ready() {
            for _ in 0..4 {
                test.sync_and_update();
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the worker's answer never landed");
}

/// The committed gcc fixture the analysis crate is pinned against, parsed the way the app
/// parses it: small, real DWARF, three functions.
fn fixture_symbols() -> Vec<Symbol> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o");
    let objects = analysis::open_files(vec![path]);
    let object = objects.first().expect("the fixture parses").clone();

    object
        .symbols_sorted
        .iter()
        .cloned()
        .map(|data| Symbol {
            object: object.clone(),
            data,
        })
        .collect()
}

/// An answer for a symbol the reader has already clicked past must never reach the panes.
///
/// Staged rather than raced: the worker is real, but the work it does is a gate the test
/// opens one job at a time, which is the only way to be sure the stale answer was
/// produced, delivered and then dropped rather than merely being slow. That the
/// selection can be set twice while the worker sits blocked is the other half of it.
#[test]
fn an_answer_for_a_symbol_no_longer_selected_is_dropped() {
    let symbols = fixture_symbols();
    let (first, second) = (symbols[0].clone(), symbols[1].clone());

    // The worker announces each job as it takes it and then waits to be let go.
    // `async_channel` on both sides and not `std::sync::mpsc`, whose `Receiver` is not
    // `Sync` and so cannot sit inside a shared `Fn`.
    let (started, starts) = async_channel::unbounded::<Symbol>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let Question::Study(symbol) = question else {
            panic!("this test asks only about symbols");
        };
        let _ = started.send_blocking(symbol.clone());
        let _ = gated.recv_blocking();
        answer(Question::Study(symbol))
    };

    let (mut test, (asking, analysis, seen, _objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        move |runner| analysis_states!(runner, work),
        1.,
    );
    let mut asking = asking;
    let settle = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    // The first click. The worker takes it and stops inside it.
    asking.set(Some(Ask::Symbol(first.clone())));
    pump(&mut test, || !starts.is_empty());
    assert!(starts.recv_blocking().expect("the worker started") == first);
    assert!(
        analysis.peek().shown.is_none(),
        "the pane was handed a listing the worker has not produced"
    );

    // The second click, while the first is still being worked on. That the UI takes
    // it at all is the other half of what this sub-step is for.
    asking.set(Some(Ask::Symbol(second.clone())));
    settle(&mut test);

    // Let the first one finish. Its answer is on the channel by the time the worker
    // announces the second job, so what follows is not a race with it.
    gate.send_blocking(()).expect("the gate");
    assert!(starts.recv_blocking().expect("the worker started") == second);
    settle(&mut test);

    assert!(
        analysis.peek().shown.is_none(),
        "an answer for a symbol the reader had left was put on screen"
    );
    assert!(analysis.peek().pending == Some(Ask::Symbol(second.clone())));

    // And the answer that is wanted lands.
    gate.send_blocking(()).expect("the gate");
    pump(&mut test, || analysis.peek().shown.is_some());

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the second symbol was analysed");
    assert!(shown.studied.symbol == second);
    assert!(state.pending.is_none());
    assert!(!state.slow);
    assert_eq!(
        seen.peek().len(),
        1,
        "a superseded listing reached the panes"
    );
    assert!(seen.peek()[0] == second);
}

/// The happy path, over the real work rather than a gate: a symbol selected comes back
/// disassembled, with the line info and the file the Source pane draws beside it,
/// and with the panes told about it exactly once.
#[test]
fn a_selected_symbol_comes_back_disassembled_and_mapped() {
    let symbol = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");

    let (mut test, (asking, analysis, seen, _objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let mut asking = asking;
    test.sync_and_update();

    asking.set(Some(Ask::Symbol(symbol.clone())));
    pump(&mut test, || analysis.peek().shown.is_some());

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the symbol was analysed");
    assert!(shown.studied.symbol == symbol);
    assert!(state.pending.is_none());
    let studied = shown.studied;
    let assembly = studied.assembly.expect("sum_to holds code");
    assert!(!assembly.instructions.is_empty());
    let lines = studied.lines.info.expect("the fixture has DWARF");
    assert!(!lines.files().is_empty());
    assert!(studied
        .lines
        .file
        .as_deref()
        .is_some_and(|file| file.ends_with("line_fixture.c")));
    assert_eq!(seen.peek().len(), 1);

    // Being asked nothing is answered on the spot: clearing does not wait on the worker,
    // only replacing does.
    asking.set(None);
    test.sync_and_update();
    assert!(analysis.peek().clone() == Analyzed::default());
}

/// The file and a line inside `symbol`, out of its own line info — so nothing here
/// hard-codes what the fixture's DWARF happens to spell, and the round trip the crate
/// asserts is what makes the answer findable again.
fn a_line_of(symbol: &Symbol) -> LinePos {
    let info = symbol
        .data
        .line_info(&symbol.object)
        .expect("the fixture has DWARF");
    let row = info
        .rows()
        .iter()
        .find(|row| row.line.is_some() && row.file.is_some())
        .expect("sum_to's rows name a place");

    LinePos {
        file: info.files()[row.file.expect("filtered")].clone(),
        line: row.line.expect("filtered"),
    }
}

/// The step's own claim: a source line is a question the worker answers with the symbol
/// that line was compiled into, over every object that is open.
#[test]
fn a_source_line_answers_with_the_symbol_it_was_compiled_into() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);

    let (mut test, (asking, analysis, _seen, objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut asking, mut objects) = (asking, objects);
    objects.set(vec![wanted.object.clone()]);
    test.sync_and_update();

    asking.set(Some(Ask::Source {
        at: at.clone(),
        chosen: None,
    }));
    pump(&mut test, || analysis.peek().shown.is_some());

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the line was resolved");
    assert!(shown.studied.symbol == wanted, "another symbol was picked");
    assert!(
        shown.ask
            == Ask::Source {
                at: at.clone(),
                chosen: None
            }
    );
    assert!(state.pending.is_none());
    assert!(shown
        .studied
        .assembly
        .is_some_and(|assembly| !assembly.instructions.is_empty()));

    // The tab such an answer belongs to is the *file's* tab, never the symbol's: it is
    // what the assembly pane keeps its row under and what a listing change is measured
    // against.
    assert!(asked_of(&shown.ask) == Document::Source(at.file.clone()));
}

/// A line no open object holds code from leaves the listing that is up — the click loses
/// the pin's highlight and nothing else — but only while that listing is this tab's own.
#[test]
fn a_line_holding_no_code_leaves_this_tabs_listing_and_no_others() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    // Past the end of any file the fixture names, so nothing was compiled from it.
    let barren = LinePos {
        file: at.file.clone(),
        line: 999_999,
    };

    let (mut test, (asking, analysis, _seen, objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut asking, mut objects) = (asking, objects);
    objects.set(vec![wanted.object.clone()]);
    test.sync_and_update();

    asking.set(Some(Ask::Source {
        at: at.clone(),
        chosen: None,
    }));
    pump(&mut test, || analysis.peek().shown.is_some());

    asking.set(Some(Ask::Source {
        at: barren.clone(),
        chosen: None,
    }));
    pump(&mut test, || {
        analysis.peek().answered
            == Some(Ask::Source {
                at: barren.clone(),
                chosen: None,
            })
    });

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the listing was taken down");
    assert!(shown.studied.symbol == wanted);
    // And it is still filed under the question it was worked out for, not the one that
    // answered with nothing.
    assert!(
        shown.ask
            == Ask::Source {
                at: at.clone(),
                chosen: None
            }
    );
    assert!(state.pending.is_none());

    // The same barren question against another tab's listing takes it down instead:
    // leaving it up would put a function the reader never asked for on screen for good.
    asking.set(Some(Ask::Symbol(wanted.clone())));
    pump(&mut test, || {
        analysis.peek().answered == Some(Ask::Symbol(wanted.clone()))
    });
    asking.set(Some(Ask::Source {
        at: barren.clone(),
        chosen: None,
    }));
    pump(&mut test, || {
        analysis.peek().answered
            == Some(Ask::Source {
                at: barren.clone(),
                chosen: None,
            })
    });
    assert!(
        analysis.peek().shown.is_none(),
        "a listing belonging to another tab was left up"
    );
}

/// The queue is drained to the newest question of each kind and not to the newest
/// overall: a locate behind a listing question cancels neither, and the listing is
/// worked first.
#[test]
fn the_queue_keeps_the_newest_question_of_each_kind() {
    let symbols = fixture_symbols();
    let at = a_line_of(&symbols[0]);
    let locate = || Question::Locate {
        at: at.clone(),
        objects: Vec::new(),
    };

    let kinds = |questions: &[Question]| -> Vec<&'static str> {
        questions
            .iter()
            .map(|question| match question {
                Question::Study(_) => "study",
                Question::Resolve { .. } => "resolve",
                Question::Locate { .. } => "locate",
            })
            .collect()
    };

    // A listing behind a listing supersedes it; a locate behind either is kept beside it.
    let drained = newest(
        Question::Study(symbols[0].clone()),
        vec![locate(), Question::Study(symbols[1].clone())].into_iter(),
    );
    assert_eq!(kinds(&drained), ["study", "locate"]);
    let Question::Study(kept) = &drained[0] else {
        panic!("the listing kind went missing");
    };
    assert!(*kept == symbols[1], "the older listing question won");

    // The listing first whichever arrived first.
    let drained = newest(
        locate(),
        vec![Question::Study(symbols[0].clone())].into_iter(),
    );
    assert_eq!(kinds(&drained), ["study", "locate"]);

    // And one of a kind is simply itself.
    assert_eq!(kinds(&newest(locate(), std::iter::empty())), ["locate"]);
}

/// A line's locations are every symbol compiled from it over every open object, and a
/// line nothing was compiled from is answered -- with nothing -- rather than left pending.
#[test]
fn a_lines_locations_come_back_from_every_open_object() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    // The same file parsed twice is two objects, and both answer.
    let twin = fixture_symbols()[0].object.clone();

    let (mut test, (_asking, _analysis, _seen, objects, _history, located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![wanted.object.clone(), twin.clone()]);
    test.sync_and_update();

    located.write().asked = Some(at.clone());
    assert!(located.peek().pending() == Some(&at));
    pump(&mut test, || located.peek().found.is_some());

    let state = located.peek().clone();
    assert!(state.pending().is_none());
    let found = state.found.expect("the line was looked for");
    assert!(found.at == at);
    let names: Vec<&str> = found
        .symbols
        .0
        .iter()
        .map(|symbol| symbol.data.name.as_str())
        .collect();
    assert_eq!(names, ["sum_to", "sum_to"]);
    assert!(Arc::ptr_eq(&found.symbols.0[0].object, &wanted.object));
    assert!(Arc::ptr_eq(&found.symbols.0[1].object, &twin));

    // Past the end of any file the fixture names.
    let barren = LinePos {
        file: at.file.clone(),
        line: 999_999,
    };
    located.write().asked = Some(barren.clone());
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.at == barren)
    });
    let state = located.peek().clone();
    assert!(state.pending().is_none());
    assert!(state.found.expect("answered").symbols.0.is_empty());
}

/// An answer for a line the reader has since asked about another line instead of is
/// dropped, and the queue drained to the newest line means the middle one is never
/// worked at all. Staged through a gate, as the listing's version of this test is.
#[test]
fn locations_for_a_line_no_longer_asked_about_are_dropped() {
    let symbols = fixture_symbols();
    let (first, second) = (a_line_of(&symbols[0]), a_line_of(&symbols[1]));
    assert!(
        first != second,
        "the fixture's first two symbols share a line"
    );

    let (started, starts) = async_channel::unbounded::<LinePos>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let Question::Locate { at, objects } = question else {
            panic!("this test asks only about locations");
        };
        let _ = started.send_blocking(at.clone());
        let _ = gated.recv_blocking();
        answer(Question::Locate { at, objects })
    };

    let (mut test, (_asking, _analysis, _seen, objects, _history, located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        move |runner| analysis_states!(runner, work),
        1.,
    );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![symbols[0].object.clone()]);
    let settle = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    located.write().asked = Some(first.clone());
    pump(&mut test, || !starts.is_empty());
    assert!(starts.recv_blocking().expect("the worker started") == first);

    located.write().asked = Some(second.clone());
    settle(&mut test);

    gate.send_blocking(()).expect("the gate");
    assert!(starts.recv_blocking().expect("the worker started") == second);
    settle(&mut test);
    assert!(
        located.peek().found.is_none(),
        "locations for a line the reader had left were put in the panel"
    );
    assert!(located.peek().pending() == Some(&second));

    gate.send_blocking(()).expect("the gate");
    pump(&mut test, || located.peek().found.is_some());
    assert!(located.peek().found.as_ref().expect("answered").at == second);
    assert!(located.peek().pending().is_none());
}

/// A locate queued behind a listing question cancels it in neither direction: both
/// answers land. What the drain test above says of the function, said of the worker.
#[test]
fn a_locate_behind_a_symbol_in_the_queue_cancels_neither() {
    let symbols = fixture_symbols();
    let symbol = symbols[0].clone();
    let at = a_line_of(&symbol);

    // The gate holds the worker inside its *first* job, so the two behind it are queued
    // together and drained together.
    let (started, starts) = async_channel::unbounded::<()>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let _ = started.send_blocking(());
        let _ = gated.recv_blocking();
        answer(question)
    };

    let (mut test, (asking, analysis, _seen, objects, _history, located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        move |runner| analysis_states!(runner, work),
        1.,
    );
    let (mut asking, mut objects, mut located) = (asking, objects, located);
    objects.set(vec![symbol.object.clone()]);
    let settle = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    // The job the worker is held inside, then the two behind it.
    asking.set(Some(Ask::Symbol(symbols[1].clone())));
    pump(&mut test, || !starts.is_empty());
    starts.recv_blocking().expect("the worker started");
    located.write().asked = Some(at.clone());
    asking.set(Some(Ask::Symbol(symbol.clone())));
    settle(&mut test);

    // Let the held job go; the worker then takes both queued jobs, each behind the gate.
    for _ in 0..3 {
        gate.send_blocking(()).expect("the gate");
    }
    pump(&mut test, || {
        located.peek().found.is_some()
            && analysis
                .peek()
                .shown
                .as_ref()
                .is_some_and(|shown| shown.studied.symbol == symbol)
    });
    let found = located
        .peek()
        .found
        .clone()
        .expect("the locate was answered");
    assert!(found.at == at);
    assert!(!found.symbols.0.is_empty());
    assert!(analysis.peek().pending.is_none());
}

/// A binary closed takes its locations with it -- the ones already in the panel and the
/// ones still on their way out of the worker -- while another binary's stand.
#[test]
fn closing_a_binary_takes_its_locations_with_it() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    let twin = fixture_symbols()[0].object.clone();

    let (mut test, (_asking, _analysis, _seen, objects, _history, located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![wanted.object.clone(), twin.clone()]);
    test.sync_and_update();

    located.write().asked = Some(at.clone());
    pump(&mut test, || located.peek().found.is_some());
    assert_eq!(
        located
            .peek()
            .found
            .as_ref()
            .expect("answered")
            .symbols
            .0
            .len(),
        2
    );

    // One file closed: its row goes, the other's stays, and the answer is still about
    // the line it was asked for.
    objects.set(vec![twin.clone()]);
    for _ in 0..4 {
        test.sync_and_update();
    }
    let found = located.peek().found.clone().expect("the answer stands");
    assert!(found.at == at);
    assert_eq!(found.symbols.0.len(), 1);
    assert!(Arc::ptr_eq(&found.symbols.0[0].object, &twin));

    // A load that adds an object changes nothing already found.
    objects.set(vec![twin.clone(), wanted.object.clone()]);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(
        located
            .peek()
            .found
            .as_ref()
            .expect("stands")
            .symbols
            .0
            .len(),
        1
    );

    // And the other file closed empties it without forgetting what was asked.
    objects.set(Vec::new());
    for _ in 0..4 {
        test.sync_and_update();
    }
    let state = located.peek().clone();
    assert!(state.asked == Some(at.clone()));
    assert!(state.found.expect("stands").symbols.0.is_empty());
}

/// The Locations view and nothing else, over the project's states and a `Located` the
/// test writes directly: what is under test is what the panel draws of an answer and
/// what a row does, not how the answer got there. `use_clear_focus` is mounted because a
/// row's press is answered by it.
fn locations_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let focused = use_consume::<Focused>().0;
    let pinned = use_consume::<Pinned>().0;
    let landing = use_consume::<Land>().0;
    use_clear_focus(active, focused, pinned, landing);

    rect().expanded().child(LocationsTab)
}

/// The contexts [`locations_harness`] reads beside the project's.
#[derive(Clone, Copy)]
struct LocationStates {
    located: State<Located>,
    pinned: State<Option<Pin>>,
    landing: State<Option<Landing>>,
}

macro_rules! location_states {
    ($runner:expr) => {{
        let states = project_states!($runner);
        $runner.provide_root_context(|| Focused(State::create(None)));
        let pinned = $runner
            .provide_root_context(|| Pinned(State::create(None)))
            .0;
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        let located = $runner
            .provide_root_context(|| Locations(State::create(Located::default())))
            .0;
        (
            states,
            LocationStates {
                located,
                pinned,
                landing,
            },
        )
    }};
}

/// Where the label reading `text` was laid out, for a press on it.
fn label_area(test: &TestingRunner, text: &str) -> Option<Area> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .filter(|label| label.text == text)
            .map(|_| node.layout().area)
    })
}

/// A few passes, since the rows sit behind a memo over the answer: the memo is
/// recomputed by a task woken on the write, a pass later than the write itself.
fn settle(test: &mut TestingRunner) {
    for _ in 0..4 {
        test.sync_and_update();
    }
}

/// The panel says which of its four states it is in off `Located`'s two fields, and a
/// found answer is a heading naming the line over one row per symbol -- the same name
/// twice, being in two objects, is two rows.
#[test]
fn the_locations_panel_draws_a_row_per_symbol() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let twin = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let at = a_line_of(&wanted);

    let (mut test, (_states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    settle(&mut test);
    assert!(labels(&test).contains(&"Nothing looked for yet".to_owned()));

    located.write().asked = Some(at.clone());
    settle(&mut test);
    let finding = format!(
        "Finding locations for {}:{}\u{2026}",
        file_name(&at.file),
        at.line
    );
    assert!(labels(&test).contains(&finding), "{:?}", labels(&test));

    located.write().found = Some(Found::new(at.clone(), Vec::new()));
    settle(&mut test);
    let nothing = format!("No code compiled from {}:{}", file_name(&at.file), at.line);
    assert!(labels(&test).contains(&nothing), "{:?}", labels(&test));

    located.write().found = Some(Found::new(at.clone(), vec![wanted.clone(), twin]));
    settle(&mut test);
    let drawn = labels(&test);
    let heading = format!("2 locations for {}:{}", file_name(&at.file), at.line);
    assert!(drawn.contains(&heading), "{drawn:?}");
    assert_eq!(
        drawn.iter().filter(|text| **text == "sum_to").count(),
        2,
        "{drawn:?}"
    );
    assert_eq!(
        drawn
            .iter()
            .filter(|text| **text == wanted.object.name)
            .count(),
        2,
        "{drawn:?}"
    );
}

/// Pressing a row is a navigation: the symbol becomes the active document and the
/// history records the visit, exactly as a press in the Symbols list does.
#[test]
fn a_location_row_opens_its_symbol() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    located.write().asked = Some(at.clone());
    located.write().found = Some(Found::new(at.clone(), vec![wanted.clone()]));
    settle(&mut test);
    assert!(states.open.active().is_none());

    let row = label_area(&test, "sum_to").expect("the row is drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    assert!(states.open.active() == Some(document.clone()));
    assert!(states
        .history
        .peek()
        .recent()
        .any(|(_, entry)| *entry == document));
}

/// A row's press lands on the line as well as opening the symbol: the pin is the line,
/// owed by both panes, and it survives the change of document that opening is --
/// which is the mechanism, since that change is what drops every other pin.
#[test]
fn a_location_row_lands_on_its_line() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    // Another document on top first, so the press is a change of document.
    activate(
        states.open,
        states.history,
        Some(Document::Assembly(Selection::Symbol(symbols[0].clone()))),
        Visit::Went,
    );
    located.write().asked = Some(at.clone());
    located.write().found = Some(Found::new(at.clone(), vec![wanted.clone()]));
    settle(&mut test);
    assert!(location.pinned.peek().is_none());

    let row = label_area(&test, "sum_to").expect("the row is drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    assert!(states.open.active() == Some(document));
    let pin = location.pinned.peek().clone().expect("the line was pinned");
    assert!(pin.at == at);
    assert!(pin.reveal == Owed::BOTH);
    assert!(
        location.landing.peek().is_none(),
        "the landing was not spent by the document it named"
    );
    // Both panes are owed the scroll, and each pays its own.
    assert!(owed_reveal(location.pinned, Pane::Assembly).as_ref() == Some(&at));
    assert!(owed_reveal(location.pinned, Pane::Source).as_ref() == Some(&at));
    reveal_made(location.pinned, Pane::Source);
    assert!(owed_reveal(location.pinned, Pane::Assembly).as_ref() == Some(&at));
    assert!(owed_reveal(location.pinned, Pane::Source).is_none());
}

/// Landing on the document already on top pins at once: `activate` then changes
/// nothing, so no effect would run to spend a landing.
#[test]
fn landing_on_the_document_already_on_top_pins_at_once() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    activate(
        states.open,
        states.history,
        Some(document.clone()),
        Visit::Went,
    );
    settle(&mut test);

    land(
        states.open,
        states.history,
        location.pinned,
        location.landing,
        document.clone(),
        at.clone(),
    );
    assert!(
        location.landing.peek().is_none(),
        "a landing was left to an effect that cannot run"
    );
    assert!(location
        .pinned
        .peek()
        .as_ref()
        .is_some_and(|pin| pin.at == at));
    settle(&mut test);
    assert!(
        location
            .pinned
            .peek()
            .as_ref()
            .is_some_and(|pin| pin.at == at),
        "the pin was dropped though no document changed"
    );
    assert!(states.open.active() == Some(document));
}

/// The companion file follows a **landed** pin when the symbol's line info names its
/// file, and the symbol's own file otherwise -- so a Locations row opens on the file the
/// line is in, while a click inside the panes changes no file.
#[test]
fn a_landed_pin_names_the_companion_file() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let mut studied = Studied::new(wanted.clone());
    let named = studied
        .lines
        .info
        .as_ref()
        .expect("the fixture has DWARF")
        .files()
        .to_vec();
    // The symbol's own file made distinct from every file the info names, so that the
    // switch is observable whether or not the fixture's one function inlines anything.
    let own: Arc<str> = "own.c".into();
    studied.lines.file = Some(own.clone());
    let analysis = Analyzed {
        shown: Some(Shown {
            ask: Ask::Symbol(wanted.clone()),
            studied,
        }),
        ..Default::default()
    };
    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    let file_of = |pin: Option<&Pin>| {
        source_side(Some(&document), &analysis, pin)
            .expect("a companion")
            .file()
            .clone()
    };
    let pin = |file: &str, landed: bool| Pin {
        at: LinePos {
            file: file.into(),
            line: 1,
        },
        reveal: Owed::BOTH,
        landed,
    };

    // A distinct allocation of a file the info names, as the app hands about.
    let elsewhere: String = named[0].to_string();
    assert!(file_of(None) == own);
    assert!(file_of(Some(&pin(&elsewhere, true))).as_ref() == elsewhere.as_str());
    // Not landed: the same pin changes nothing.
    assert!(file_of(Some(&pin(&elsewhere, false))) == own);
    // Landed but naming a file the symbol knows nothing of: nothing to switch to.
    assert!(file_of(Some(&pin("nowhere.rs", true))) == own);
    // And a source-driven tab's subject is its own file whatever is pinned.
    let subject = Document::Source("subject.rs".into());
    assert!(
        source_side(Some(&subject), &analysis, Some(&pin(&elsewhere, true)))
            .expect("a subject")
            .file()
            .as_ref()
            == "subject.rs"
    );
}

/// A source-driven tab's ask carries the symbol the reader chose among the many, and
/// the worker's pick answers with it -- over the symbol on screen, which is what would
/// otherwise win -- and falls back where the choice was not compiled from the line.
#[test]
fn a_chosen_symbol_wins_the_pick_for_its_line() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    let twin = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let other = symbols
        .iter()
        .find(|symbol| symbol.data.name != "sum_to")
        .expect("the fixture holds another function")
        .clone();

    let (mut test, (asking, analysis, _seen, objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut asking, mut objects) = (asking, objects);
    objects.set(vec![wanted.object.clone(), twin.object.clone()]);
    test.sync_and_update();

    // Unchosen, the first object's copy is on screen.
    asking.set(Some(Ask::Source {
        at: at.clone(),
        chosen: None,
    }));
    pump(&mut test, || analysis.peek().shown.is_some());
    assert!(
        analysis
            .peek()
            .shown
            .as_ref()
            .expect("resolved")
            .studied
            .symbol
            == wanted
    );

    // Chosen, the twin's is, though the first is the symbol on screen.
    let chosen = Ask::Source {
        at: at.clone(),
        chosen: Some(twin.clone()),
    };
    asking.set(Some(chosen.clone()));
    pump(&mut test, || {
        analysis.peek().answered == Some(chosen.clone())
    });
    assert!(
        analysis
            .peek()
            .shown
            .as_ref()
            .expect("resolved")
            .studied
            .symbol
            == twin
    );

    // A choice the line was not compiled from is no choice: the symbol on screen stays.
    let elsewhere = Ask::Source {
        at: at.clone(),
        chosen: Some(other),
    };
    asking.set(Some(elsewhere.clone()));
    pump(&mut test, || {
        analysis.peek().answered == Some(elsewhere.clone())
    });
    assert!(
        analysis
            .peek()
            .shown
            .as_ref()
            .expect("resolved")
            .studied
            .symbol
            == twin
    );
}

/// Locations asked for from a source-driven tab are chosen **for** it: pressing a row
/// keeps the reader in that tab, drives it from the line and has its assembly side
/// follow the symbol. Once the tab is gone the same press opens the symbol instead.
#[test]
fn a_location_chosen_from_a_source_driven_tab_changes_its_assembly_side() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    let tab = Document::Source(at.file.clone());

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    activate(states.open, states.history, Some(tab.clone()), Visit::Went);
    located.write().asked = Some(at.clone());
    located.write().subject = Some(at.file.clone());
    located.write().found = Some(Found::new(at.clone(), vec![wanted.clone()]));
    settle(&mut test);

    let row = label_area(&test, "sum_to").expect("the row is drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    assert!(
        states.open.active() == Some(tab.clone()),
        "the press left the tab"
    );
    assert_eq!(
        states.open.documents().len(),
        1,
        "a tab was opened for the symbol"
    );
    assert!(states.driven.peek().choice(&tab) == Some(wanted.clone()));
    assert_eq!(states.driven.peek().line(&tab), Some(at.line));
    assert!(location
        .pinned
        .peek()
        .as_ref()
        .is_some_and(|pin| pin.at == at));
    // Which is the question the tab now asks.
    assert!(
        ask(Some(&tab), &states.driven.peek())
            == Some(Ask::Source {
                at: at.clone(),
                chosen: Some(wanted.clone()),
            })
    );

    // The tab closed, the same row opens the symbol as a tab of its own.
    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.driven,
        &tab,
    );
    settle(&mut test);
    let row = label_area(&test, "sum_to").expect("the row is still drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);
    assert!(states.open.active() == Some(Document::Assembly(Selection::Symbol(wanted))));
}

/// A landing is for the next arrival only: whichever document arrives spends it, and
/// one for another document pins nothing.
#[test]
fn a_landing_is_spent_by_whichever_document_arrives() {
    let symbols = fixture_symbols();
    let at = a_line_of(&symbols[0]);

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    settle(&mut test);

    let mut landing = location.landing;
    landing.set(Some(Landing {
        tab: Document::Assembly(Selection::Symbol(symbols[0].clone())),
        at: at.clone(),
    }));
    activate(
        states.open,
        states.history,
        Some(Document::Assembly(Selection::Symbol(symbols[1].clone()))),
        Visit::Went,
    );
    settle(&mut test);

    assert!(
        location.pinned.peek().is_none(),
        "a landing pinned a line in another document"
    );
    assert!(
        location.landing.peek().is_none(),
        "a spent landing was left lying"
    );
}

/// The sidebar's dock, which `app()` keeps as a local state and a test has to provide
/// somewhere.
#[derive(Clone, Copy)]
struct SidebarDock(State<DockArea>);

/// Asking for a line's locations is one write and one dock change: the question reaches
/// the worker and is answered, and the Locations view is brought to the top of whichever
/// panel holds it -- the sidebar's here, behind History -- looked for through the content
/// dock the row can reach. Asking for the line already answered asks again.
#[test]
fn finding_a_line_asks_the_worker_and_brings_the_panel_to_the_front() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);

    let (mut test, ((_asking, _analysis, _seen, objects, _history, located), content, sidebar)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            |runner| {
                let states = analysis_states!(runner, answer);
                // The two areas as `app()` wires them, the content one knowing the
                // sidebar. The content dock holds no view at all, so the search has to
                // cross over.
                let sidebar = runner
                    .provide_root_context(|| {
                        SidebarDock(State::create(DockArea::column(vec![vec![
                            Tab::View(View::History),
                            Tab::View(View::Locations),
                        ]])))
                    })
                    .0;
                let content = runner
                    .provide_root_context(|| {
                        let mut area = DockArea::row(vec![vec![]]).with_documents(DOCUMENT_PANEL);
                        area.other = Some(sidebar);
                        ContentDock(State::create(area))
                    })
                    .0;
                (states, content, sidebar)
            },
            1.,
        );
    let mut objects = objects;
    objects.set(vec![wanted.object.clone()]);
    test.sync_and_update();

    let on_top = |dock: State<DockArea>| {
        let dock = dock.peek();
        let (panel, _) = dock.tree.find_tab(&Tab::View(View::Locations))?;
        dock.tree.panel(&panel)?.active_tab_id
    };
    assert!(on_top(sidebar) == Some(Tab::View(View::History)));

    find_locations(located, content, at.clone(), None);
    assert!(on_top(sidebar) == Some(Tab::View(View::Locations)));
    assert!(located.peek().pending() == Some(&at));
    pump(&mut test, || located.peek().found.is_some());
    let found = located.peek().found.clone().expect("answered");
    assert!(found.at == at);
    assert_eq!(found.symbols.0.len(), 1);

    // The same line again is asked again, out of whatever is open now.
    let mut objects = objects;
    objects.set(vec![
        wanted.object.clone(),
        fixture_symbols()[0].object.clone(),
    ]);
    test.sync_and_update();
    assert_eq!(
        located
            .peek()
            .found
            .as_ref()
            .expect("stands")
            .symbols
            .0
            .len(),
        1,
        "an answer re-asked itself when an object was opened"
    );
    find_locations(located, content, at.clone(), None);
    assert!(located.peek().pending() == Some(&at));
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.symbols.0.len() == 2)
    });
}

/// An answer for a line the reader has clicked past must never reach the panes.
/// Supersession across the *new* kind of question is a different claim from supersession
/// across symbols: it is the `Ask` comparison being right for a `LinePos`, which is the
/// one `Arc` in the UI compared by its text.
#[test]
fn an_answer_for_a_line_no_longer_asked_about_is_dropped() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    let later = LinePos {
        // A distinct `Arc<str>` of the same path, which is exactly what the app hands
        // about: a tab's file and a `LineInfo`'s are two allocations of one string.
        file: at.file.to_string().into(),
        line: at.line,
    };

    let (started, starts) = async_channel::unbounded::<LinePos>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let Question::Resolve { at, .. } = &question else {
            panic!("this test asks only about lines");
        };
        let _ = started.send_blocking(at.clone());
        let _ = gated.recv_blocking();
        answer(question)
    };

    let (mut test, (asking, analysis, seen, objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        move |runner| analysis_states!(runner, work),
        1.,
    );
    let (mut asking, mut objects) = (asking, objects);
    objects.set(vec![wanted.object.clone()]);
    let settle = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    let barren = LinePos {
        file: at.file.clone(),
        line: 999_999,
    };
    asking.set(Some(Ask::Source {
        at: barren.clone(),
        chosen: None,
    }));
    pump(&mut test, || !starts.is_empty());
    assert!(starts.recv_blocking().expect("the worker started") == barren);

    // Clicked past while the first is still being worked on.
    asking.set(Some(Ask::Source {
        at: later.clone(),
        chosen: None,
    }));
    settle(&mut test);

    gate.send_blocking(()).expect("the gate");
    assert!(starts.recv_blocking().expect("the worker started") == later);
    settle(&mut test);
    assert!(
        analysis.peek().answered.is_none(),
        "an answer for a line the reader had left was taken"
    );

    gate.send_blocking(()).expect("the gate");
    pump(&mut test, || analysis.peek().shown.is_some());
    assert!(seen.peek().len() == 1);
    assert!(seen.peek()[0] == wanted);
    // Two `Arc<str>`s of one path are one question, or every tab switch would re-resolve.
    assert!(analysis.peek().answered == Some(Ask::Source { at, chosen: None }));
}

/// [`ask`] on its own, which is the whole of "what is this tab asking": no runner, since
/// it is a function of two values.
#[test]
fn what_a_tab_asks_follows_its_kind_and_its_driven_line() {
    let symbols = fixture_symbols();
    let symbol = symbols[0].clone();
    let object = symbol.object.clone();
    let file: Arc<str> = "src/main.rs".into();
    let tab = Document::Source(file.clone());
    let mut driven = Driven::default();

    assert!(ask(None, &driven).is_none(), "nothing open asks nothing");
    assert!(
        ask(
            Some(&Document::Assembly(Selection::Symbol(symbol.clone()))),
            &driven
        ) == Some(Ask::Symbol(symbol.clone()))
    );
    // An object is a place in a binary but not one with a listing.
    assert!(ask(
        Some(&Document::Assembly(Selection::Object(object))),
        &driven
    )
    .is_none());
    // A source-driven tab nothing has been clicked in yet.
    assert!(ask(Some(&tab), &driven).is_none());

    driven.remember(tab.clone(), 42);
    assert!(
        ask(Some(&tab), &driven)
            == Some(Ask::Source {
                at: LinePos { file, line: 42 },
                chosen: None
            })
    );
    // And the line belongs to that tab and not to source-driven tabs at large.
    assert!(ask(Some(&Document::Source("other.rs".into())), &driven).is_none());
}

/// The one thing in the analysis that can outlive the document that named it: a
/// source-driven tab survives a binary close by doctrine, so the answer resolved out of
/// that binary must not go on being drawn -- nor go on holding the file's bytes, a
/// `Studied` holding a `Symbol` holding the `Arc<Object>`.
#[test]
fn closing_a_binary_lets_go_of_the_listing_it_answered() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    let object = wanted.object.clone();
    drop(symbols);
    let before = Arc::strong_count(&object);

    let (mut test, (asking, analysis, seen, objects, _history, _located)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| analysis_states!(runner, answer),
        1.,
    );
    let (mut asking, mut objects, mut seen) = (asking, objects, seen);
    objects.set(vec![object.clone()]);
    test.sync_and_update();

    asking.set(Some(Ask::Source {
        at: at.clone(),
        chosen: None,
    }));
    pump(&mut test, || analysis.peek().shown.is_some());
    assert!(Arc::strong_count(&object) > before, "the listing holds it");

    // What `close_binary` does to the objects, which is the whole of what this can see
    // of it: the tab, being source-driven, is deliberately left standing and so the
    // question is unchanged.
    objects.set(Vec::new());
    // Waited on the *answer* and not on the listing going: the listing goes the moment
    // the effect sees the objects change, and the question it then asks again is what
    // this is about.
    pump(&mut test, || {
        let held = analysis.peek();
        held.answered.is_some() && held.pending.is_none()
    });

    assert!(
        analysis.peek().shown.is_none(),
        "a listing out of a closed binary was left on screen"
    );
    // Asked again out of what is left, which is nothing, and said so.
    assert!(analysis.peek().answered == Some(Ask::Source { at, chosen: None }));
    assert!(analysis.peek().pending.is_none());
    // The recorder this harness keeps for the supersession tests holds every symbol it
    // was told about, which is this test's own doing and not the app's.
    seen.set(Vec::new());
    test.sync_and_update();
    assert_eq!(
        Arc::strong_count(&object),
        before,
        "the closed file's bytes are still held"
    );
}

/// A reveal is owed until it has been *made*, not until it has been looked at.
///
/// In a source-driven tab the click that pins is the click that asks for the listing, so
/// the assembly pane's first run after it is still holding the listing that cannot answer
/// -- and a request spent there is a scroll the reader never gets, the listing that can
/// answer arriving to nothing owed.
#[test]
fn a_reveal_the_listing_cannot_answer_is_left_owed() {
    let at = LinePos {
        file: "main.rs".into(),
        line: 42,
    };

    let (mut test, pinned) = TestingRunner::new(
        project_harness,
        (100., 100.).into(),
        |runner| {
            runner
                .provide_root_context(|| Pinned(State::create(None)))
                .0
        },
        1.,
    );
    let mut pinned = pinned;
    test.sync_and_update();

    pinned.set(Some(Pin {
        at: at.clone(),
        reveal: Owed::by(Pane::Assembly),
        landed: false,
    }));
    test.sync_and_update();

    // The pane that is owed it looks, twice, and the request is still there both times:
    // the first look is the listing being left, the second the one that arrived.
    assert!(owed_reveal(pinned, Pane::Assembly).as_ref() == Some(&at));
    assert!(owed_reveal(pinned, Pane::Assembly).as_ref() == Some(&at));
    // And the other pane is owed nothing: a click asks the pane it was not made in.
    assert!(owed_reveal(pinned, Pane::Source).is_none());
    // Which is also what a `reveal_made` from it must not undo.
    reveal_made(pinned, Pane::Source);
    test.sync_and_update();
    assert!(owed_reveal(pinned, Pane::Assembly).as_ref() == Some(&at));

    // Made, and so owed exactly once. The pin itself stays: it is what lights the rows.
    reveal_made(pinned, Pane::Assembly);
    test.sync_and_update();
    assert!(owed_reveal(pinned, Pane::Assembly).is_none());
    assert!(pinned.peek().as_ref().is_some_and(|pin| pin.at == at));

    // A second click on the same line is a second request.
    pinned.set(Some(Pin {
        at: at.clone(),
        reveal: Owed::by(Pane::Assembly),
        landed: false,
    }));
    test.sync_and_update();
    assert!(owed_reveal(pinned, Pane::Assembly).as_ref() == Some(&at));
}

/// The Assembly pane over a listing the test puts into [`Analysis`] itself, with no worker
/// between the two. The document it is handed is the tab the drawn listing belongs to,
/// which is what `app()` hands it.
fn listing_harness() -> impl IntoElement {
    let analysis = use_consume::<Analysis>().0;
    let document = analysis
        .read()
        .shown
        .as_ref()
        .map(|shown| asked_of(&shown.ask))
        .unwrap_or_else(|| Document::Source(Arc::from("")));

    rect().expanded().child(AssemblyPane { document })
}

/// The contexts a listing's rows read, beside the project's.
macro_rules! listing_states {
    ($runner:expr, $shown:expr) => {{
        let states = project_states!($runner);
        $runner.provide_root_context(|| Focused(State::create(None)));
        $runner.provide_root_context(|| Marked(State::create(None)));
        $runner.provide_root_context(|| Shift(State::create(false)));
        $runner.provide_root_context(|| Locations(State::create(Located::default())));
        let pinned = $runner
            .provide_root_context(|| Pinned(State::create(None)))
            .0;
        $runner.provide_root_context(|| {
            Analysis(State::create(Analyzed {
                shown: Some($shown),
                ..Analyzed::default()
            }))
        });
        (states, pinned)
    }};
}

/// Pressing a branch's displacement puts the row it lands on on screen **and pins the line
/// that row was compiled from** -- the pin a press on the target row itself would have
/// made, with the Source pane owed the scroll and the Assembly pane not, since it has just
/// been given one. It is still not a navigation: the document does not change and nothing
/// is pushed onto the history, so a Back button never has to undo reading further down the
/// same function.
///
/// Headless because every part of it is a question about the real tree: which spans a row
/// is drawn out of, which rows a `VirtualScrollView` built, where the operand was laid
/// out, and whether a press on it reaches the row underneath.
#[test]
fn following_a_jump_scrolls_to_the_row_it_lands_on() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    // The lines this test tells apart, worked out from the line info rather than from the
    // pane. The fixture has two branches and only the second is any use for telling *which*
    // row was pinned: the forward `jmp` and the row at 61h it lands on are both line 35, so
    // it is the backward one -- 67h, line 35, landing on 4Bh, line 36 -- that says the pin
    // followed the jump instead of staying where the press started. The test asserts that
    // pairing before it leans on it.
    let line_at = |address: u64| {
        let info = studied
            .lines
            .info
            .as_ref()
            .expect("the gcc fixture carries line info");
        let row = info.row_at(address).expect("the address is in a line row");
        LinePos {
            file: info.files()[row.file.expect("the row names a file")].clone(),
            line: row.line.expect("the row names a line"),
        }
    };
    let forward_lands_on = line_at(0x61);
    let backward_starts_at = line_at(0x67);
    let backward_lands_on = line_at(0x4B);
    // `LinePos` carries no `Debug` and is not given one for a test's benefit, so the
    // failures below spell a position out themselves.
    let spell = |at: &LinePos| format!("{}:{}", at.file, at.line);
    assert!(
        backward_starts_at != backward_lands_on,
        "the backward jump and its target share {}, so a pin cannot tell them apart",
        spell(&backward_lands_on)
    );

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    // `jmp short 61h` is the sixth instruction of the fixture's loop and lands on the
    // fifteenth, far enough down that a pane this tall is not showing it.
    let landing = "0000000000000061 ";

    let (mut test, (states, pinned)) = TestingRunner::new(
        listing_harness,
        (500., 200.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "61h"),
        "the jump's operand is not drawn as a span of its own: {drawn:?}"
    );
    assert!(
        !drawn.iter().any(|text| text == landing),
        "the row it lands on is on screen already: {drawn:?}"
    );

    let operand = label_area(&test, "61h").expect("the operand is laid out");
    let at = (
        (operand.origin.x + operand.width() as f32 / 2.0) as f64,
        (operand.origin.y + operand.height() as f32 / 2.0) as f64,
    );
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);

    assert!(
        labels(&test).iter().any(|text| text == landing),
        "the listing did not scroll to the row the jump lands on: {:?}",
        labels(&test)
    );
    // The selection followed it too, and the Source pane owes the scroll: the Assembly
    // pane has just been given one and must not be asked for a second.
    let pin = pinned
        .peek()
        .clone()
        .expect("following a jump pinned nothing");
    assert!(
        pin.at == forward_lands_on,
        "the pin is {} where the jump lands on {}",
        spell(&pin.at),
        spell(&forward_lands_on)
    );
    assert!(pin.reveal.source, "the source side was not owed the scroll");
    assert!(
        !pin.reveal.assembly,
        "the listing was asked to scroll twice"
    );

    // And again on the backward jump, which is the one whose line differs from its
    // target's: the press lands on 67h, line 35, and what is pinned afterwards is 4Bh's
    // line 36 -- the row jumped *to*, not the row the pointer was over.
    let operand = label_area(&test, "4Bh").expect("the backward jump is on screen now");
    let at = (
        (operand.origin.x + operand.width() as f32 / 2.0) as f64,
        (operand.origin.y + operand.height() as f32 / 2.0) as f64,
    );
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);

    let pin = pinned
        .peek()
        .clone()
        .expect("the backward jump pinned nothing");
    assert!(
        pin.at == backward_lands_on,
        "the pin is {} where the backward jump lands on {}",
        spell(&pin.at),
        spell(&backward_lands_on)
    );
    assert!(
        pin.at != backward_starts_at,
        "the press bubbled into the row and pinned where it started, {}",
        spell(&backward_starts_at)
    );

    // Still not a navigation: nothing was opened or visited by either press.
    assert!(states.open.active().is_none());
    assert_eq!(states.history.peek().recent().count(), 0);
}

/// A row a branch lands on wears a hairline across its top edge, so the listing reads as
/// the basic blocks it is -- and **the row is the height it always was**: a border is paint
/// and not layout, which is the whole reason the mark is a rule inside the row rather than
/// a gap above it. A row that is nobody's target wears nothing.
///
/// Headless because both halves are questions about the real tree: which of the rows a
/// `VirtualScrollView` built carry the border, and what those rows measured once it was on
/// them.
#[test]
fn a_row_a_branch_lands_on_starts_a_block() {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let assembly = studied.assembly.clone().expect("the fixture disassembles");
    // The rows the gutter already puts an arrowhead on, by address: the separator is that
    // set drawn again and not a second answer of its own.
    let mut targets: Vec<u64> = assembly
        .edges
        .iter()
        .map(|edge| assembly.instructions[edge.to].address)
        .collect();
    targets.sort_unstable();
    targets.dedup();

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    // Tall enough for the whole of `sum_to`, so what is drawn is what the symbol holds.
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    // Every element the separator was drawn on, by the area it was laid out in.
    let ruled: Vec<Area> = test.find_many(|node, element| {
        element
            .style()
            .borders
            .iter()
            .any(|border| border.fill == palette().block_rule && border.width.top > 0.0)
            .then(|| node.layout().area)
    });
    // And every instruction row, by the address column it is drawn with -- sixteen hex
    // digits and a trailing space, which nothing else in the listing is.
    let rows: Vec<(u64, Area)> = test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .map(|label| label.text.to_string())
            .filter(|text| text.trim_end().len() == 16)
            .and_then(|text| u64::from_str_radix(text.trim_end(), 16).ok())
            .map(|address| (address, node.layout().area))
    });

    let drawn: Vec<u64> = rows.iter().map(|(address, _)| *address).collect();
    let expected: Vec<u64> = targets
        .iter()
        .copied()
        .filter(|address| drawn.contains(address))
        .collect();
    assert!(
        expected.len() >= 2,
        "the fixture's sum_to is branched to {} times: {expected:0X?}",
        expected.len()
    );
    assert!(
        drawn.len() > expected.len(),
        "every drawn row is a branch target, so a mark on all of them would pass"
    );

    let mut started: Vec<u64> = rows
        .iter()
        .filter(|(_, area)| {
            let middle = area.origin.y + area.height() / 2.0;
            ruled
                .iter()
                .any(|row| row.origin.y <= middle && middle < row.origin.y + row.height())
        })
        .map(|(address, _)| *address)
        .collect();
    started.sort_unstable();
    assert_eq!(
        started, expected,
        "the separators are not on the rows the branches land on"
    );

    // And the mark cost the row nothing: it is still exactly the `item_size` the scroll
    // view over it was given, or every row below the first block would be drawn a pixel
    // further down than the view believes.
    for area in &ruled {
        assert_eq!(
            area.height(),
            code_row_height(),
            "the separator moved the row it is on"
        );
    }
}

/// A component with no props at all, which is what every view in the app is. Its parent
/// reads nothing coloured, so the theme has to reach it on its own.
#[derive(PartialEq)]
struct ThemedRow;

impl Component for ThemedRow {
    fn render(&self) -> impl IntoElement {
        rect().expanded().background(palette().pane_bg)
    }
}

fn theme_harness() -> impl IntoElement {
    rect().expanded().child(ThemedRow)
}

/// The same row under the wiring that resolves the theme, with the choice handed in so the
/// machine's own settings file has no vote. The root reads the appearance as well, as
/// `app()` does, so the write `use_theme` makes during the render body wakes the very
/// scope that made it -- which settles only because that write is idempotent.
fn desktop_theme_harness() -> impl IntoElement {
    use_theme(ThemeChoice::Desktop);
    let _ = appearance();

    rect().expanded().child(ThemedRow)
}

/// The first background anything paints, which is the row's: the harness's own rect has
/// none.
fn painted(test: &TestingRunner) -> Fill {
    test.find(|_, element| {
        let background = element.style().background.clone();
        (background != Fill::Color(Color::TRANSPARENT)).then_some(background)
    })
    .expect("a painted row")
}

/// `HIGHLIGHTED` is process-wide while the appearance is per-thread, so the tests that
/// switch themes have to be the only one doing it at a time.
static SWITCHING: Mutex<()> = Mutex::new(());

/// A theme switch repaints a component that did not change and whose parent did not
/// either. Nothing about `ThemedRow` differs across the switch, so freya will not
/// re-render it for any reason except that it read the state that changed -- and asking
/// for a colour is that read.
#[test]
fn a_theme_switch_repaints_a_component_nothing_else_woke() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    set_appearance(Appearance::Light);

    let (mut test, ()) = TestingRunner::new(theme_harness, (100., 100.).into(), |_| (), 1.);
    test.sync_and_update();

    assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));

    set_appearance(Appearance::Dark);
    test.sync_and_update();
    assert_eq!(painted(&test), Fill::Color(Palette::DARK.pane_bg));

    // And back, so the thread is left as it was found.
    set_appearance(Appearance::Light);
    test.sync_and_update();
    assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));
}

/// The other half: the source pane's spans are cached with the palette resolved into them,
/// so a switch has to throw the cache away and parse again. Nothing re-renders a
/// `SyntaxBlocks`, which is why the reactivity above cannot cover it.
#[test]
fn a_theme_switch_empties_the_highlighted_cache() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    set_appearance(Appearance::Light);

    let directory =
        std::env::temp_dir().join(format!("assembly-viewer-theme-test-{}", std::process::id()));
    let path = directory.join("themed.rs");
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    std::fs::write(&path, b"fn main() {}\n").expect("writing the source file");

    // A keyword, which is the one span whose colour is a palette entry rather than the
    // text colour -- and the reason this is a `.rs` file and not any file at all.
    let keyword = |path: &Path| {
        let text = source_text(path).expect("the file");
        let line = text.0.blocks.get_line(0);
        line.first().expect("a first span").0
    };

    assert_eq!(keyword(&path), Palette::LIGHT.keyword_fg);
    assert!(!highlighted().is_empty());

    set_appearance(Appearance::Dark);
    assert!(
        highlighted().is_empty(),
        "the switch left the old theme's spans behind"
    );
    assert_eq!(keyword(&path), Palette::DARK.keyword_fg);

    set_appearance(Appearance::Light);
    highlighted().clear();
    let _ = std::fs::remove_dir_all(&directory);
}

/// The windowing system changing its mind about the theme, *after* the window is open,
/// repaints it. freya keeps `Platform::preferred_theme` from winit and re-sets it on the
/// OS's `ThemeChanged` event, so setting it here is what that event does.
#[test]
fn a_desktop_that_changes_its_mind_repaints_the_window() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    // Left on the wrong one on purpose, so that the mount below has to be a real write
    // rather than a value that happened to already be there.
    set_appearance(Appearance::Dark);

    // `provide_root_context` runs its closure in the root scope, where freya-testing has
    // already put the `Platform`.
    let (mut test, platform) = TestingRunner::new(
        desktop_theme_harness,
        (100., 100.).into(),
        |runner| runner.provide_root_context(Platform::get),
        1.,
    );
    test.sync_and_update();

    // freya-testing mounts on `PreferredTheme::Light`, and the choice is a question, so the
    // answer arrived on the first render.
    assert_eq!(appearance(), Appearance::Light);
    assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));

    // **Two passes, and the second is not padding.** The change reaches the window in two
    // hops -- the platform state wakes the scope holding `use_theme`, and the write that
    // scope makes wakes everything that drew a colour -- and a pass renders the dirty
    // scopes it *began* with.
    let mut preferred = platform.preferred_theme;
    preferred.set(PreferredTheme::Dark);
    test.sync_and_update();
    assert_eq!(appearance(), Appearance::Dark);
    test.sync_and_update();

    assert_eq!(painted(&test), Fill::Color(Palette::DARK.pane_bg));

    // And back again, both to prove the wire runs in both directions and to leave the
    // thread as it was found.
    preferred.set(PreferredTheme::Light);
    test.sync_and_update();
    test.sync_and_update();
    assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));
}

/// sRGB relative luminance, and the contrast ratio between two colours, as WCAG defines
/// them.
fn luminance(color: Color) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };

    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Every foreground is legible on the surface it is actually drawn on, in both palettes.
/// The floor is 3.0 and not WCAG AA's 4.5: two of the light palette's own colours sit
/// between 3 and 3.5, both of which are *meant* to recede.
#[test]
fn every_foreground_is_legible_on_its_own_surface() {
    for (theme, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
        // The code colours, on the one surface both code panes are drawn on. Which of
        // them a given pane shows differs -- the assembly has no comments and no strings
        // -- but the ground no longer does, so there is one judgement to make.
        for (name, color) in [
            ("address_fg", palette.address_fg),
            ("keyword_fg", palette.keyword_fg),
            ("operand_fg", palette.operand_fg),
            ("literal_fg", palette.literal_fg),
            ("string_fg", palette.string_fg),
            ("comment_fg", palette.comment_fg),
            ("punctuation_fg", palette.punctuation_fg),
            ("name_fg", palette.name_fg),
            ("name_hover_fg", palette.name_hover_fg),
        ] {
            let ratio = contrast(color, palette.pane_bg);
            assert!(ratio >= 3.0, "{theme} {name} on pane_bg: {ratio:.2}");
        }

        // The chrome, on all three of the surfaces it is written over.
        for (name, color) in [
            ("text_fg", palette.text_fg),
            ("icon_fg", palette.icon_fg),
            ("invalid_fg", palette.invalid_fg),
        ] {
            for (surface, background) in [
                ("pane_bg", palette.pane_bg),
                ("header_bg", palette.header_bg),
                ("symbol_pane_bg", palette.symbol_pane_bg),
            ] {
                let ratio = contrast(color, background);
                assert!(ratio >= 3.0, "{theme} {name} on {surface}: {ratio:.2}");
            }
        }

        // The × on a tab comes up to the interface text under the pointer, over a wash
        // that is over whichever of the two grounds the tab is on. Composited, because
        // that is what the reader is aiming at when legibility matters most.
        for (surface, ground) in [
            ("an active tab", palette.pane_bg),
            ("a hovered tab", palette.toggle_hover_bg),
        ] {
            let ratio = contrast(palette.text_fg, blend(palette.close_hover_bg, ground));
            assert!(ratio >= 3.0, "{theme} the × on {surface}: {ratio:.2}");
        }

        // The branch gutter is a diagram and is drawn quiet deliberately, so its floor is
        // only against a line that has disappeared into the pane altogether.
        let line = contrast(palette.branch_fg, palette.pane_bg);
        let lit = contrast(palette.branch_hover_fg, palette.pane_bg);
        assert!(line >= 1.5, "{theme} branch_fg: {line:.2}");
        assert!(lit > line, "{theme} branch_hover_fg: {lit:.2} vs {line:.2}");

        // The rule that starts a basic block runs the whole width of the pane where the
        // gutter's stroke is a few pixels long, so it is held to a floor of its own and
        // required to stay quieter than that stroke rather than merely legible.
        let rule = contrast(palette.block_rule, palette.pane_bg);
        assert!(rule >= 1.2, "{theme} block_rule: {rule:.2}");
        assert!(
            rule < line,
            "{theme} block_rule: {rule:.2} vs branch_fg {line:.2}"
        );
    }
}

/// Every translucent wash still says something once it is composited: `blend` puts the pane
/// under these, so the same alpha over a dark ground is a fraction of the step it was over
/// white. The pin, which is the focus said louder, has to stay louder.
#[test]
fn every_wash_reads_against_the_pane_under_it() {
    // How far a wash moves the surface it is over, in the channel it moves most.
    let step = |wash: Color, ground: Color| {
        let over = blend(wash, ground);
        let channel = |top: u8, bottom: u8| (top as i32 - bottom as i32).unsigned_abs();
        channel(over.r(), ground.r())
            .max(channel(over.g(), ground.g()))
            .max(channel(over.b(), ground.b()))
    };

    for (theme, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
        for (name, wash, ground) in [
            (
                "code_row_hover_bg",
                palette.code_row_hover_bg,
                palette.pane_bg,
            ),
            ("line_focus_bg", palette.line_focus_bg, palette.pane_bg),
            ("line_pin_bg", palette.line_pin_bg, palette.pane_bg),
            ("row_select_bg", palette.row_select_bg, palette.pane_bg),
            // The one wash that is never over the bare pane: a link is under the pointer
            // only while the row it is on is, so what it has to lighten is the hover.
            // It is white in both palettes, and over a code pane that is now white
            // itself the plain surface would be no test at all -- it moves it nowhere.
            (
                "link_hover_bg",
                palette.link_hover_bg,
                blend(palette.code_row_hover_bg, palette.pane_bg),
            ),
            ("drop_preview_bg", palette.drop_preview_bg, palette.pane_bg),
            // The × on a tab sits on either of two grounds and has to say the same thing
            // over both: the active tab's own pane, and a hovered tab's grey.
            ("close_hover_bg", palette.close_hover_bg, palette.pane_bg),
            (
                "close_hover_bg over a hovered tab",
                palette.close_hover_bg,
                palette.toggle_hover_bg,
            ),
        ] {
            let step = step(wash, ground);
            assert!(step >= 10, "{theme} {name}: {step} levels");
        }

        let focus = step(palette.line_focus_bg, palette.pane_bg);
        let pin = step(palette.line_pin_bg, palette.pane_bg);
        assert!(pin > focus, "{theme} pin {pin} vs focus {focus}");

        // And the × has to be told apart from the tab under it, which is lit at the same
        // time: the two hovers differ by strength on the same surface, the close moving
        // the tab further than the tab's own hover moves the bar it sits in. `step` of an
        // opaque colour is that colour, the bottom being fully covered.
        let tab = step(palette.toggle_hover_bg, palette.header_bg);
        let close = step(palette.close_hover_bg, palette.toggle_hover_bg);
        assert!(close > tab, "{theme} close {close} vs tab {tab}");
    }
}

/// A control that cannot be used recedes without disappearing: `dimmed` lands between the
/// surface it is drawn on and the colour it has when it is live. A floor rather than a
/// value, the way the branch gutter's is -- it is meant to be quiet.
#[test]
fn a_dimmed_control_recedes_without_disappearing() {
    for (theme, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
        let live = contrast(palette.icon_fg, palette.pane_bg);
        let dim = contrast(dimmed(palette.icon_fg, palette.pane_bg), palette.pane_bg);
        assert!(
            dim < live,
            "{theme}: dimmed is {dim:.2} against the live {live:.2}"
        );
        assert!(
            dim >= 1.5,
            "{theme}: dimmed {dim:.2} has gone into the surface"
        );
    }
}

/// The `resolve_capture_color` trap, in both palettes: it decides a capture is unmapped by
/// comparing its colour to `text` and then walks *up* the dotted name, so a child field
/// holding the text colour while its parent holds another is painted in the parent's.
#[test]
fn captures_do_not_walk_up() {
    for (name, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
        let theme = palette.syntax();
        let dotted = [
            ("function.macro", theme.function_macro, theme.function),
            ("function.method", theme.function_method, theme.function),
            (
                "punctuation.bracket",
                theme.punctuation_bracket,
                theme.punctuation,
            ),
            (
                "punctuation.delimiter",
                theme.punctuation_delimiter,
                theme.punctuation,
            ),
            (
                "punctuation.special",
                theme.punctuation_special,
                theme.punctuation,
            ),
            ("string.escape", theme.string_escape, theme.string),
            ("string.special", theme.string_special, theme.string),
            // A `text.*` capture's parent is `text` itself, which `capture_color`
            // answers for with the text colour, so these can only ever agree.
            ("text.literal", theme.text_literal, theme.text),
            ("text.reference", theme.text_reference, theme.text),
            ("text.title", theme.text_title, theme.text),
            ("text.uri", theme.text_uri, theme.text),
            ("text.emphasis", theme.text_emphasis, theme.text),
            ("variable.builtin", theme.variable_builtin, theme.variable),
            (
                "variable.parameter",
                theme.variable_parameter,
                theme.variable,
            ),
        ];

        for (capture, child, parent) in dotted {
            assert!(
                child != theme.text || parent == theme.text,
                "{name}: {capture} takes the text colour while its parent does not, \
                     so it would be painted in the parent's",
            );
        }
    }
}

/// A `Fonts` with nothing left to ask the desktop about, so a test asserting a size
/// asserts a size and not whatever `kreadconfig` answers on the machine running it.
fn fixed_fonts(ui: f32, mono: f32) -> Fonts {
    fonts::resolve(&Settings {
        theme: ThemeChoice::Desktop,
        interface: FontSetting {
            family: Some("Interface".to_owned()),
            size: Some(ui),
        },
        fixed: FontSetting {
            family: Some("Fixed".to_owned()),
            size: Some(mono),
        },
    })
}

/// The same pair as an [`EditedSettings`], which is what the page holds.
fn fixed_edited(ui: f32, mono: f32) -> EditedSettings {
    EditedSettings {
        theme: ThemeChoice::Desktop,
        interface: EditedFont {
            family: "Interface".to_owned(),
            size: Some(ui),
        },
        fixed: EditedFont {
            family: "Fixed".to_owned(),
            size: Some(mono),
        },
    }
}

/// Two components with no props at all, one row at each of the two heights. `ThemedRow`'s
/// twins: nothing about either changes across a font change, so freya has no reason to
/// re-render them except that they read the state. The two fills only have to *differ*,
/// `painted_height` being how the rows are told apart; neither says anything about where
/// a row of that height is really drawn.
#[derive(PartialEq)]
struct FontedRow;

impl Component for FontedRow {
    fn render(&self) -> impl IntoElement {
        rect()
            .width(Size::fill())
            .height(Size::px(list_row_height()))
            .background(palette().pane_bg)
    }
}

#[derive(PartialEq)]
struct FontedCodeRow;

impl Component for FontedCodeRow {
    fn render(&self) -> impl IntoElement {
        rect()
            .width(Size::fill())
            .height(Size::px(code_row_height()))
            .background(palette().header_bg)
    }
}

fn font_harness() -> impl IntoElement {
    rect().expanded().child(FontedRow).child(FontedCodeRow)
}

/// The height of the row painted in `fill`, as it was actually laid out -- not as it was
/// asked for: a component that was never re-rendered is still the old height on screen.
fn painted_height(test: &TestingRunner, fill: Color) -> f32 {
    test.find(|node, element| {
        let background = element.style().background.clone();
        (background == Fill::Color(fill)).then(|| node.layout().area.height())
    })
    .expect("a painted row")
}

/// A font change repaints a component nothing else woke, *and* moves it, the row heights
/// being derived from the fonts. The two heights are **independent**: no row mixes the
/// fonts, so each change below leaves the other row exactly where it was.
#[test]
fn a_font_change_repaints_and_resizes_a_component_nothing_else_woke() {
    set_fonts(fixed_fonts(9.0, 10.5));

    let (mut test, ()) = TestingRunner::new(font_harness, (200., 200.).into(), |_| (), 1.);
    test.sync_and_update();

    let list = palette().pane_bg;
    let code = palette().header_bg;

    assert_eq!((list_row_height(), code_row_height()), (24.0, 26.0));
    assert_eq!(painted_height(&test, list), 24.0);
    assert_eq!(painted_height(&test, code), 26.0);

    // 18pt is 24 logical pixels, so the code row is 36 -- and the list row is still
    // the 24 it was, the assembly font having nothing to say about it.
    set_fonts(fixed_fonts(9.0, 18.0));
    test.sync_and_update();
    assert_eq!((list_row_height(), code_row_height()), (24.0, 36.0));
    assert_eq!(painted_height(&test, list), 24.0);
    assert_eq!(painted_height(&test, code), 36.0);

    // And the other way: 21pt is 28 pixels, so the list row is 40 and the code row is
    // back to the 26 its own unchanged font asks for.
    set_fonts(fixed_fonts(21.0, 10.5));
    test.sync_and_update();
    assert_eq!((list_row_height(), code_row_height()), (40.0, 26.0));
    assert_eq!(painted_height(&test, list), 40.0);
    assert_eq!(painted_height(&test, code), 26.0);
}

/// A `VirtualScrollView`'s `item_size` and the height its rows actually draw at must be the
/// same number, or scrolling misaligns silently. Two claims since the height was split in
/// two, so it is asserted over a code pane and a sidebar list.
///
/// Asked through real scroll views, by which row is under a given y: at the top of the
/// list row *k* covers `[k*h, (k+1)*h)`, so a pointer at 90 is row 3 at 26px and row 2 at
/// 36px. Each half also steps the font it is *not* drawn in and asserts nothing moved.
#[test]
fn a_scroll_view_and_its_rows_agree_at_every_font_size() {
    set_fonts(fixed_fonts(9.0, 10.5));

    // Away and back, or entering the same row twice is no event at all.
    fn row_under(test: &mut TestingRunner, top: State<usize>, y: f64) -> usize {
        test.move_cursor((50., 5.));
        test.sync_and_update();
        test.move_cursor((50., y));
        test.sync_and_update();
        *top.peek()
    }

    // A font change wakes the rows through the state they read; several passes because the
    // scroll view answers the new item size on the render after the one that moved them.
    fn settle(test: &mut TestingRunner) {
        for _ in 0..4 {
            test.sync_and_update();
        }
    }

    {
        let (mut test, top) = TestingRunner::new(
            scrolling_harness,
            (200., 200.).into(),
            |runner| {
                let tabs = vec!["a".to_owned()];
                runner.provide_root_context(|| KeptTab(State::create("a".to_owned())));
                runner.provide_root_context(|| KeptAt(State::create(Positions::default())));
                runner.provide_root_context(|| KeptOpen(State::create(tabs)));
                runner.provide_root_context(|| KeptLength(State::create(100)));
                runner.provide_root_context(|| KeptTop(State::create(0))).0
            },
            1.,
        );
        test.sync_and_update();

        assert_eq!(code_row_height(), 26.0);
        assert_eq!(row_under(&mut test, top, 90.), 3);

        // The interface font is not this pane's font, so stepping it moves nothing.
        set_fonts(fixed_fonts(21.0, 10.5));
        settle(&mut test);
        assert_eq!(code_row_height(), 26.0);
        assert_eq!(row_under(&mut test, top, 90.), 3);

        set_fonts(fixed_fonts(9.0, 18.0));
        settle(&mut test);
        assert_eq!(code_row_height(), 36.0);
        assert_eq!(row_under(&mut test, top, 90.), 2);
    }

    set_fonts(fixed_fonts(9.0, 10.5));

    {
        let (mut test, top) = TestingRunner::new(
            list_scrolling_harness,
            (200., 200.).into(),
            |runner| runner.provide_root_context(|| KeptTop(State::create(0))).0,
            1.,
        );
        test.sync_and_update();

        // 24 rather than 26: a sidebar row is the interface font's 12 pixels plus the
        // leading, and 90 is three of them down.
        assert_eq!(list_row_height(), 24.0);
        assert_eq!(row_under(&mut test, top, 90.), 3);

        // And the fixed-width font is not this list's font.
        set_fonts(fixed_fonts(9.0, 18.0));
        settle(&mut test);
        assert_eq!(list_row_height(), 24.0);
        assert_eq!(row_under(&mut test, top, 90.), 3);

        set_fonts(fixed_fonts(21.0, 10.5));
        settle(&mut test);
        assert_eq!(list_row_height(), 40.0);
        assert_eq!(row_under(&mut test, top, 90.), 2);
    }
}

/// Everything the settings write, recorded rather than performed.
#[derive(Clone, Copy)]
struct Saved(State<Vec<Settings>>);

fn settings_harness() -> impl IntoElement {
    let prefs = use_consume::<Prefs>().0;
    let mut saved = use_consume::<Saved>().0;

    use_settings_with(prefs, move |settings: &Settings| {
        saved.write().push(settings.clone())
    });

    // The **code** row, because what this test steps is the fixed-width size.
    rect().expanded().child(FontedCodeRow)
}

/// One state, and the theme, the fonts and the file all following from it -- with the
/// write handed in, because the real one edits the settings of whoever runs the tests.
/// A run that never opens the page writes **nothing**, and changing a setting *back*
/// writes again.
#[test]
fn the_settings_reach_the_theme_the_fonts_and_the_file() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    // Both left on the wrong answer on purpose, so that arriving at the right one has
    // to be a real write rather than a value that happened to already be there.
    set_appearance(Appearance::Dark);
    set_fonts(fixed_fonts(21.0, 21.0));

    let (mut test, (prefs, saved)) = TestingRunner::new(
        settings_harness,
        (200., 200.).into(),
        |runner| {
            (
                runner
                    .provide_root_context(|| Prefs(State::create(fixed_edited(9.0, 10.5))))
                    .0,
                runner
                    .provide_root_context(|| Saved(State::create(Vec::new())))
                    .0,
            )
        },
        1.,
    );
    let mut prefs = prefs;
    for _ in 0..4 {
        test.sync_and_update();
    }

    // Mounting is not a change: the settings were read off disk, and writing them
    // straight back would create the file on a launch where the reader did nothing.
    assert!(
        saved.peek().is_empty(),
        "a run that changed nothing wrote the settings file"
    );
    // But the app is drawn in them: the choice is `Desktop` and freya-testing mounts on
    // `PreferredTheme::Light`, and the fonts are the ones the state holds.
    assert_eq!(appearance(), Appearance::Light);
    assert_eq!(fonts().mono.points, 10.5);
    assert_eq!(painted_height(&test, palette().header_bg), 26.0);

    // A theme chosen. Two passes: the write the root makes wakes the scopes that drew a
    // colour in the pass after the one it was made in.
    prefs.write().theme = ThemeChoice::Dark;
    test.sync_and_update();
    assert_eq!(appearance(), Appearance::Dark);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(saved.peek().len(), 1);
    assert_eq!(saved.peek()[0].theme, ThemeChoice::Dark);

    // A size chosen: the fonts follow, and the rows with them.
    prefs.write().fixed.size = Some(18.0);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(fonts().mono.points, 18.0);
    assert_eq!(painted_height(&test, palette().header_bg), 36.0);
    assert_eq!(saved.peek().len(), 2);
    assert_eq!(saved.peek()[1].fixed.size, Some(18.0));

    // Cleared again: the write happens even though this is the value the run started from
    // -- the baseline is what was last *written*, not what was loaded.
    prefs.write().fixed.size = Some(10.5);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(saved.peek().len(), 3);
    assert_eq!(saved.peek()[2].fixed.size, Some(10.5));
    assert_eq!(painted_height(&test, palette().header_bg), 26.0);

    // And the thread is left as it was found.
    set_appearance(Appearance::Light);
}

#[test]
fn a_swept_run_survives_the_button_coming_up() {
    let (mut test, marked) = TestingRunner::new(
        harness,
        (100., 100.).into(),
        |runner| {
            runner.provide_root_context(|| Shift(State::create(false)));
            runner
                .provide_root_context(|| Marked(State::create(None)))
                .0
        },
        1.,
    );
    test.sync_and_update();

    test.press_cursor((10., 10.));
    test.move_cursor((10., 30.));
    test.sync_and_update();
    assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);

    // The line that panicked, and the assertion that it no longer does is the test
    // getting this far at all.
    test.release_cursor((10., 30.));
    assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);

    // And the gesture really is over: a row entered afterwards is the pointer passing
    // over it, which is the panes' hover and not a sweep.
    test.move_cursor((10., 50.));
    test.sync_and_update();
    assert_eq!(marked.peek().unwrap().rows.rows(), 0..=1);
}

/// The scratchpad worker's work, handed in through a context so a test can answer without
/// the machine's own state directory and without waiting on a compiler.
#[derive(Clone)]
struct Working(Arc<dyn Fn(PadJob) -> PadAnswer + Send + Sync>);

/// The way to ask the worker for a build, as the wiring hands it back.
#[derive(Clone, Copy)]
struct Asking(State<Option<PadJobs>>);

/// What the worker was handed, in the order it was handed it.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    List,
    New,
    Open(String),
    Save(String),
    Build(String),
    Run,
}

/// The scratchpad wiring and nothing else: no pane, since what is under test is which
/// jobs the worker is handed and what its answers do to the app.
fn scratchpad_harness() -> impl IntoElement {
    scratchpad_wiring();

    rect().expanded()
}

/// The same wiring under the real pane, for the one thing only the pane can be asked:
/// whether its rows survive one of them being taken away.
fn scratchpad_view_harness() -> impl IntoElement {
    scratchpad_wiring();

    rect().expanded().child(ScratchpadTab)
}

fn scratchpad_wiring() {
    let pad = use_consume::<Pad>().0;
    let text = use_consume::<PadText>().0;
    let work = use_consume::<Working>().0;
    let mut asking = use_consume::<Asking>().0;
    let states = use_project_states();

    let jobs = use_scratchpad_with(pad, text, states, move |job| work(job));
    use_hook(move || asking.set(Some(jobs)));
}

/// Type into the shown pad's buffer, which is the one the reader would be typing into.
fn edit_shown(text: State<PadBuffers>, pad: State<Pads>, edit: impl FnOnce(&mut CodeEditorData)) {
    let shown = pad.peek().shown().clone();
    let mut text = text;
    edit(text.write().get_mut(&shown));
}

/// What that buffer is holding.
fn shown_rope(text: State<PadBuffers>, pad: State<Pads>) -> String {
    let shown = pad.peek().shown().clone();
    text.peek().get(&shown).rope.to_string()
}

/// The committed gcc fixture again, standing in for what a build produced.
fn fixture_artifact() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o")
}

/// Mount the wiring over a worker that records every job and answers from `answer`. A
/// macro for `project_states!`'s reason: the runner's type is not one this crate can name.
macro_rules! mount_scratchpad {
    ($harness:expr, $answer:expr) => {{
        let (asked, asks) = async_channel::unbounded::<Asked>();
        let answer = $answer;
        let work = move |job: PadJob| {
            let recorded = match &job {
                PadJob::List => Asked::List,
                PadJob::New => Asked::New,
                PadJob::Open(scratchpad) => Asked::Open(scratchpad.id().as_str().to_owned()),
                PadJob::Save(scratchpad) => Asked::Save(scratchpad.source.clone()),
                PadJob::Build(scratchpad) => Asked::Build(scratchpad.source.clone()),
                PadJob::Run { .. } => Asked::Run,
            };
            let _ = asked.send_blocking(recorded);
            answer(job)
        };

        let (mut test, (states, pad, text, asking)) = TestingRunner::new(
            $harness,
            (400., 400.).into(),
            move |runner: &mut _| {
                let states = project_states!(runner);
                runner.provide_root_context(move || Working(Arc::new(work)));
                let pad = runner
                    .provide_root_context(|| Pad(State::create(Pads::default())))
                    .0;
                let text = runner
                    .provide_root_context(|| PadText(State::create(PadBuffers::default())))
                    .0;
                let asking = runner
                    .provide_root_context(|| Asking(State::create(None)))
                    .0;

                (states, pad, text, asking)
            },
            1.,
        );
        test.sync_and_update();

        (test, states, pad, text, asking, asks)
    }};
}

/// Every `label()` on screen, by its text. `ElementExt` reads no text through the prelude,
/// so this downcasts past it -- `agents/Headless.md` spells the recipe out.
fn labels(test: &TestingRunner) -> Vec<String> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .map(|label| label.text.to_string())
    })
}

/// An id, for the tests below that name pads the app would have generated ids for.
fn pad_id(text: &str) -> PadId {
    PadId::new(text).expect("an id")
}

/// One row of what the worker's listing answers with: a pad on disk that nobody has named,
/// which is what a new one is until the reader says otherwise.
fn pad_listing(text: &str) -> PadListing {
    PadListing {
        id: pad_id(text),
        name: String::new(),
    }
}

/// Answer an `Open` with what a pad's directory would have in it: a source naming the pad
/// it belongs to, so a test can tell which pad's text is on screen.
fn pad_on_disk(scratchpad: Scratchpad) -> Scratchpad {
    let mut opened = scratchpad;
    opened.source = format!("// {}\n", opened.id().as_str());
    opened
}

/// Which pad opens is the **front of the order**, not the pad the app boots holding — that
/// one is only what there is to show before the worker has said what pads exist. And every
/// pad there is gets a row, in that order, or the reader has no way back to one.
#[test]
fn the_front_of_the_order_is_the_pad_that_opens() {
    let (mut test, _states, pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("second"), pad_listing("first"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    assert_eq!(pad.peek().shown().as_str(), "second");
    assert_eq!(shown_rope(text, pad), "// second\n");
    assert_eq!(
        pad.peek()
            .order
            .ids()
            .iter()
            .map(PadId::as_str)
            .collect::<Vec<_>>(),
        ["second", "first"]
    );

    // The listing, then the one pad it named as the front. The pad the app booted holding
    // is not opened at all.
    assert_eq!(asks.try_recv(), Ok(Asked::List));
    assert_eq!(asks.try_recv(), Ok(Asked::Open("second".to_owned())));
    assert!(asks.is_empty());
}

/// Switching pads writes the one being left **before** it opens the next, and does it
/// through the worker: the jobs are one ordered queue, so a save that went after the open
/// -- or that was left to the effect, which wakes only after the handler is over -- would
/// arrive behind the next pad's read. The pad arrived at is read once and never again, its
/// buffer, its model and its baseline all being held from then on.
#[test]
fn switching_writes_the_pad_being_left_before_it_opens_the_next() {
    let (mut test, _states, pad, text, asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    assert_eq!(asks.try_recv(), Ok(Asked::List));
    assert_eq!(asks.try_recv(), Ok(Asked::Open("one".to_owned())));

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    let two = pad_id("two");

    // The state a keystroke leaves behind for one pass: the buffer has been mirrored into
    // the model and the effect that writes the model out has not run yet, those being two
    // effects and the second woken by the first. Written straight into the model, because
    // a test cannot ask freya to run one of the two and not the other.
    let mut pad = pad;
    pad.write().state_mut().scratchpad.source = "// edited\n".to_owned();
    show_pad(pad, &jobs, two.clone());
    pump(&mut test, || {
        pad.peek().shown() == &two && pad.peek().state().opened
    });

    // The order the worker was handed them, which is the order it did them in.
    assert_eq!(asks.try_recv(), Ok(Asked::Save("// edited\n".to_owned())));
    assert_eq!(asks.try_recv(), Ok(Asked::Open("two".to_owned())));

    assert_eq!(pad.peek().shown(), &two);
    assert_eq!(shown_rope(text, pad), "// two\n");
    assert!(asks.is_empty(), "the switch asked for more than it had to");

    // Back again: nothing is read a second time, the pad having been held all along, and
    // the buffer that comes back is the one it was left with -- so is the cursor in it and
    // so is its undo history, which is what a buffer each buys over one replaced on every
    // switch.
    let one = pad_id("one");
    show_pad(pad, &jobs, one.clone());
    for _ in 0..4 {
        test.sync_and_update();
    }

    assert_eq!(pad.peek().shown(), &one);
    assert_eq!(shown_rope(text, pad), "// one\n");
    assert!(
        !matches!(asks.try_recv(), Ok(Asked::Open(_))),
        "the pad was read again on the way back"
    );
}

/// The panel draws the **name** the reader gave a pad and never the id it is filed under,
/// which is the whole of what the id being hidden means. The two are different strings here
/// on purpose: a pad whose name has been changed, and a pad with no name at all, which
/// falls back to a placeholder rather than to its id.
#[test]
fn the_panel_draws_names_and_never_ids() {
    let (mut test, _states, pad, _text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![
                PadListing {
                    id: pad_id("pad-7"),
                    name: "Parser notes".to_owned(),
                },
                PadListing {
                    id: pad_id("pad-8"),
                    name: String::new(),
                },
            ]),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::New => unreachable!("this test never makes one"),
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "Parser notes"),
        "the pad's name is not on screen: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|text| text == "<pad-8>"),
        "a pad with no name drew something else: {drawn:?}"
    );
    // No row is a bare id. An unnamed pad's label carries one, in brackets that say it is
    // the app's word and not the reader's; the other place an id is on screen is the
    // package's path, which is a path and not an identity -- and now the only way to find
    // a pad's directory, the name no longer spelling it.
    assert!(
        !drawn.iter().any(|text| text == "pad-7" || text == "pad-8"),
        "an id was drawn as a pad's name: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|text| text.ends_with("scratchpads/pad-7")),
        "the package's path stopped saying where the pad is: {drawn:?}"
    );
}

/// A pad made from the panel is written and shown at once: pressing New is a deliberate
/// act, so the pad appears where the reader is looking, at the front of the order.
#[test]
fn a_new_pad_is_written_and_shown_at_once() {
    let made = Scratchpad::new("pad-1").expect("an id");
    let answering = made.clone();
    let (mut test, _states, pad, text, asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("pad")]),
            PadJob::New => PadAnswer::Created(Ok(answering.clone())),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    assert_eq!(pad.peek().shown().as_str(), "pad");
    while asks.try_recv().is_ok() {}

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_new_pad(&jobs);
    pump(&mut test, || pad.peek().shown().as_str() == "pad-1");

    pump(&mut test, || pad.peek().state().opened);

    assert_eq!(asks.try_recv(), Ok(Asked::New));
    // Read straight back: the worker wrote the package on the way, and reading it is what
    // seeds the baseline, so the first keystroke in it is a change and not a first write.
    assert_eq!(asks.try_recv(), Ok(Asked::Open("pad-1".to_owned())));
    assert_eq!(shown_rope(text, pad), "// pad-1\n");

    // At the front of the order, and the pad it was made beside is still there.
    assert_eq!(
        pad.peek()
            .order
            .ids()
            .iter()
            .map(PadId::as_str)
            .collect::<Vec<_>>(),
        ["pad-1", "pad"]
    );
}

/// A rename is a keystroke and nothing more: the name is a value in the pad's own package
/// and nothing is filed under it, so the ordinary save writes it out and the row beside the
/// box follows. Nothing moves, nothing can be refused, and two pads may be called the same
/// thing -- which is the whole of what hiding the id buys.
#[test]
fn renaming_a_pad_is_a_save_and_moves_nothing() {
    let (mut test, _states, pad, _text, _asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two")]),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::New => unreachable!("this test never makes one"),
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    while asks.try_recv().is_ok() {}

    // What the box does, which is all a rename is now.
    let mut pad = pad;
    pad.write().state_mut().scratchpad.name = "two".to_owned();
    pump(&mut test, || !asks.is_empty());

    // Written out by the ordinary save, under the id it was always filed under -- and a
    // name another pad already has is simply a name another pad already has.
    assert_eq!(asks.try_recv(), Ok(Asked::Save("// one\n".to_owned())));
    assert!(asks.is_empty());
    assert_eq!(pad.peek().shown().as_str(), "one");
    assert_eq!(pad.peek().state().scratchpad.name, "two");
    assert_eq!(
        pad.peek()
            .order
            .ids()
            .iter()
            .map(PadId::as_str)
            .collect::<Vec<_>>(),
        ["one", "two"],
        "a rename moved the pad in the order"
    );
}

/// A run belongs to its pad, so leaving that pad does not stop it and its lines keep
/// landing in **its own** list. There is one output pane, and untangled from the pad it
/// would show one program's output under another program's name.
#[test]
fn a_program_goes_on_running_in_a_pad_that_is_not_shown() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Run {
                run,
                scratchpad,
                executable,
                emit,
            } => PadAnswer::Started {
                pad: scratchpad.id().clone(),
                run,
                started: crate::scratchpad::run_in(&executable, &cwd, emit),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
        });

    pump(&mut test, || pad.peek().state().opened);
    let one = pad.peek().shown().clone();
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || pad.peek().state().output.len() > 0);

    // Away to the other pad, which has never run anything.
    let two = pad_id("two");
    show_pad(pad, &jobs, two.clone());
    pump(&mut test, || pad.peek().shown() == &two);

    assert!(pad.peek().state().run_status().is_none());
    let left = pad.peek().get(&one).expect("the pad that was left").clone();
    assert!(left.is_running(), "leaving the pad stopped its program");
    assert_eq!(
        left.output.line(0).map(|line| line.text.to_string()),
        Some("from the program".to_owned())
    );

    // Ended while another pad is on screen: the event carries the pad it is about, so it
    // is that pad that stops rather than the one being looked at.
    let RunState::Going(running) = &left.run_state else {
        panic!("a going run, got {:?}", left.run_status());
    };
    running.stop();
    pump(&mut test, || {
        !pad.peek().get(&one).is_some_and(|state| state.is_running())
    });

    assert!(matches!(
        pad.peek().get(&one).expect("the pad").run_state,
        RunState::Over(Ended::Stopped)
    ));
    assert!(
        pad.peek().state().run_status().is_none(),
        "another pad's program ended into the pad on screen"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A save of one pad may not be dropped in favour of a job for another. The supersede rule
/// is what makes a burst of keystrokes one write, and keyed on nothing it would also make a
/// switch away from a pad throw away the last thing typed in it -- silently, since the pad
/// that lost the write is the one nobody is looking at.
#[test]
fn a_save_is_superseded_only_by_a_job_for_the_same_pad() {
    let pad = |id: &str, source: &str| {
        let mut scratchpad = Scratchpad::new(id).expect("an id");
        scratchpad.source = source.to_owned();
        scratchpad
    };

    let mut queue = VecDeque::from([
        PadJob::Save(pad("one", "second")),
        PadJob::Save(pad("two", "other pad")),
        PadJob::Save(pad("one", "third")),
    ]);
    let mut held = Vec::new();

    let job = superseded(
        PadJob::Save(pad("one", "first")),
        || queue.pop_front(),
        |newer| held.push(newer),
    );

    // The two saves of `one` collapsed into the newer of them, and the save of `two`
    // stopped the drain rather than replacing it.
    let PadJob::Save(scratchpad) = &job else {
        panic!("a save");
    };
    assert_eq!(scratchpad.id().as_str(), "one");
    assert_eq!(scratchpad.source, "second");

    assert_eq!(held.len(), 1);
    assert_eq!(held[0].pad().map(PadId::as_str), Some("two"));
    // And what was behind it is still queued, in order.
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].pad().map(PadId::as_str), Some("one"));
}

/// The scratchpad on disk is what the app opens on, and **nothing is written until it has
/// arrived** -- the app boots holding `Scratchpad::default`, and a save before the answer
/// lands would put that default over a scratchpad someone had been keeping.
#[test]
fn a_scratchpad_is_read_before_anything_is_written_over_it() {
    let mut saved = Scratchpad::default();
    saved.source = "fn kept() {}\n".to_owned();
    saved.dependencies = vec![Dependency {
        name: "anyhow".to_owned(),
        version: "1.0.86".to_owned(),
    }];

    let answering = saved.clone();
    let (mut test, _states, pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(_) => PadAnswer::Opened(answering.clone()),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: scratchpad.manifest().err(),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    assert_eq!(pad.peek().state().scratchpad, saved);
    // The editor is holding it too, which is the half a reader can see.
    assert_eq!(shown_rope(text, pad), saved.source);

    assert_eq!(asks.try_recv(), Ok(Asked::List));
    assert_eq!(
        asks.try_recv(),
        Ok(Asked::Open(crate::scratchpad::DEFAULT_ID.to_owned()))
    );
    assert!(
        asks.is_empty(),
        "the package was written before the app knew what was in it"
    );
}

/// An edit is written out, and a row that cannot be written says so against itself.
/// `Failure::Dependencies` carries the **index** of every row that is wrong, which is
/// what lets the pane mark them in place.
#[test]
fn an_edit_is_written_and_a_bad_row_says_which_row() {
    let (mut test, _states, pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            // The real refusal, without a disk: `write` fails on exactly what
            // `manifest` fails on, the manifest being what it refuses to generate.
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: scratchpad.manifest().err(),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    assert_eq!(asks.try_recv(), Ok(Asked::List));
    assert_eq!(
        asks.try_recv(),
        Ok(Asked::Open(crate::scratchpad::DEFAULT_ID.to_owned()))
    );

    // Typing: the rope is what the keyboard edits and the model is what is written.
    edit_shown(text, pad, |editor| editor.rope.insert(0, "// typed\n"));
    pump(&mut test, || !asks.is_empty());

    let typed = format!("// typed\n{}", crate::scratchpad::DEFAULT_SOURCE);
    assert_eq!(asks.try_recv(), Ok(Asked::Save(typed.clone())));
    assert_eq!(pad.peek().state().scratchpad.source, typed);
    assert!(pad.peek().state().unsaved.is_none());

    // A row that names no crate. It is the *second* row, so the index in the answer is the
    // assertion.
    let mut pad = pad;
    {
        let mut state = pad.write();
        state.state_mut().scratchpad.dependencies = vec![
            Dependency {
                name: "anyhow".to_owned(),
                version: "1.0.86".to_owned(),
            },
            Dependency::default(),
        ];
    }
    pump(&mut test, || pad.peek().state().unsaved.is_some());

    assert_eq!(
        pad.peek().state().unsaved,
        Some(Failure::Dependencies(vec![(1, Problem::NoName)]))
    );

    // And fixing it writes again, rather than leaving the disk holding the last good
    // version for ever.
    pad.write().state_mut().scratchpad.dependencies[1] = Dependency {
        name: "rand".to_owned(),
        version: "0.8".to_owned(),
    };
    pump(&mut test, || pad.peek().state().unsaved.is_none());
}

/// A build is asked for once however often the reader presses, and what it made is opened
/// **in place of** what the build before it made: a rebuild writes the same path with
/// different bytes, and a binary is identified by its path.
#[test]
fn a_build_runs_once_and_replaces_what_the_last_one_opened() {
    let artifact = fixture_artifact();
    let built = artifact.clone();
    let (mut test, states, pad, _text, asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(scratchpad) => PadAnswer::Built {
                pad: scratchpad.id().clone(),
                build: Build::Built {
                    executable: built.clone(),
                    diagnostics: Vec::new(),
                },
            },
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    assert_eq!(asks.try_recv(), Ok(Asked::List));
    assert_eq!(
        asks.try_recv(),
        Ok(Asked::Open(crate::scratchpad::DEFAULT_ID.to_owned()))
    );

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_build(pad, &jobs);
    // The second press, while the first is still in flight. Nothing at all happens.
    request_build(pad, &jobs);
    assert!(pad.peek().state().building);

    pump(&mut test, || !states.objects.peek().is_empty());
    assert!(!pad.peek().state().building);
    assert!(matches!(
        pad.peek().state().built,
        Some(Build::Built { .. })
    ));

    let opened = |states: &ProjectStates| {
        states
            .objects
            .peek()
            .iter()
            .filter(|object| object.path == artifact)
            .count()
    };
    let first = opened(&states);
    assert!(first > 0, "the artifact was never opened");
    assert_eq!(
        asks.try_recv(),
        Ok(Asked::Build(pad.peek().state().scratchpad.source.clone()))
    );
    assert!(
        asks.is_empty(),
        "the second press started a second build of the same scratchpad"
    );

    // And again. The path is the same one, so what the first build left has to go rather
    // than sit beside it -- waited for on the *objects*, a rebuild being a close followed
    // by a streaming reopen.
    request_build(pad, &jobs);
    pump(&mut test, || {
        !pad.peek().state().building && opened(&states) > 0
    });

    assert_eq!(
        opened(&states),
        first,
        "a rebuild left the objects of the build before it in the list"
    );
}

/// Taking a dependency row away does not take the pane with it: each box writes into
/// `dependencies[index]` through a mapped `Writable`, so a row that outlived the list
/// being shortened would index past the end at the moment it was next read -- a panic,
/// not a compile error.
#[test]
fn removing_a_dependency_row_does_not_take_the_pane_with_it() {
    let (mut test, _states, pad, _text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: scratchpad.manifest().err(),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    let mut pad = pad;
    pad.write().state_mut().scratchpad.dependencies = vec![
        Dependency {
            name: "anyhow".to_owned(),
            version: "1.0.86".to_owned(),
        },
        Dependency {
            name: "rand".to_owned(),
            version: "0.8".to_owned(),
        },
    ];
    for _ in 0..4 {
        test.sync_and_update();
    }

    // The first row, which is what the × on it does -- so the row left behind is the
    // one that was drawn at index 1.
    pad.write().state_mut().scratchpad.dependencies.remove(0);
    for _ in 0..4 {
        test.sync_and_update();
    }

    assert_eq!(pad.peek().state().scratchpad.dependencies.len(), 1);
    assert_eq!(pad.peek().state().scratchpad.dependencies[0].name(), "rand");
}

/// A directory of this test's own, named after the line that asked for it.
fn run_directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-run-test-{}-{line}",
        std::process::id()
    ))
}

/// Build a program that says something and then never exits, and say where it is. A real
/// `cargo build`: it is hermetic (no dependencies, so one rustc invocation), and nothing
/// short of a real process can say whether a stop actually killed anything.
fn looping_program(directory: &Path) -> PathBuf {
    let mut scratchpad = Scratchpad::new("looper").expect("an id");
    scratchpad.source = "fn main() {\n\
             \x20   println!(\"from the program\");\n\
             \x20   loop { std::thread::sleep(std::time::Duration::from_millis(50)); }\n\
             }\n"
    .to_owned();

    let build = scratchpad.build_in(directory);
    let Build::Built { executable, .. } = &build else {
        panic!("a build, got {build:?}");
    };
    executable.clone()
}

/// What a build left behind, put where a build would have put it -- written into the
/// state rather than answered through `PadJob::Build`, so it does not go through
/// `reopen_binary` on the way.
fn already_built(mut pad: State<Pads>, executable: PathBuf) {
    pad.write().state_mut().built = Some(Build::Built {
        executable,
        diagnostics: Vec::new(),
    });
}

/// A program that prints and then loops for ever has said something, and it is on screen
/// while it is still going. Asking it to stop really kills it: `Over(Stopped)` is
/// written by the run's own `Ended`, which is emitted after the process has been reaped.
/// Only a real process can say either.
#[test]
fn a_run_streams_while_it_is_going_and_a_stop_really_ends_it() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            // Nothing about the run is faked: the real spawn, the real pipes and the
            // real kill, reached through the same job the button sends.
            PadJob::Run {
                run,
                scratchpad,
                executable,
                emit,
            } => PadAnswer::Started {
                pad: scratchpad.id().clone(),
                run,
                started: crate::scratchpad::run_in(&executable, &cwd, emit),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
        });

    pump(&mut test, || pad.peek().state().opened);
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);

    pump(&mut test, || pad.peek().state().output.len() > 0);
    let state = pad.peek().state().clone();
    assert_eq!(
        state
            .output
            .line(0)
            .map(|line| (line.stream, line.text.to_string())),
        Some((Stream::Out, "from the program".to_owned()))
    );
    assert!(state.is_running(), "it ended by itself");

    stop_run(pad);
    pump(&mut test, || !pad.peek().state().is_running());
    let state = pad.peek().state().clone();
    assert!(
        matches!(state.run_state, RunState::Over(Ended::Stopped)),
        "{:?}",
        state.run_status()
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A rebuild stops the program the last one started: cargo is about to write over the
/// executable that process *is*, and `reopen_binary` is about to close the objects
/// describing those bytes. Through `request_build` rather than the button, the guard
/// being a property of asking.
#[test]
fn a_rebuild_stops_the_program_the_last_one_started() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Run {
                run,
                scratchpad,
                executable,
                emit,
            } => PadAnswer::Started {
                pad: scratchpad.id().clone(),
                run,
                started: crate::scratchpad::run_in(&executable, &cwd, emit),
            },
            // What the build itself answers does not matter here: the run is stopped
            // on the way to sending the job, before cargo would have been asked
            // anything at all.
            PadJob::Build(scratchpad) => PadAnswer::Built {
                pad: scratchpad.id().clone(),
                build: Build::Unavailable(Failure::NoArtifact),
            },
        });

    pump(&mut test, || pad.peek().state().opened);
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || pad.peek().state().output.len() > 0);
    assert!(pad.peek().state().is_running());

    request_build(pad, &jobs);
    pump(&mut test, || !pad.peek().state().is_running());
    let state = pad.peek().state().clone();
    assert!(
        matches!(state.run_state, RunState::Over(Ended::Stopped)),
        "{:?}",
        state.run_status()
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A program that will not start is a sentence, not a pane that sits on "Starting..."
/// for ever. No subprocess: what is under test is that the worker's failure reaches the
/// line the reader reads.
#[test]
fn a_run_that_cannot_start_says_why() {
    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            // Nothing on this machine's disk: the pad the app booted holding is the one
            // that is opened.
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Run {
                run, scratchpad, ..
            } => PadAnswer::Started {
                pad: scratchpad.id().clone(),
                run,
                started: Err(Failure::NoProgram("No such file or directory".to_owned())),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
        });

    pump(&mut test, || pad.peek().state().opened);
    already_built(pad, fixture_artifact());
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || !pad.peek().state().is_running());

    let (text, bad) = pad.peek().state().run_status().expect("a status");
    assert!(text.contains("No such file or directory"), "{text}");
    assert!(bad);
}

/// The lines a run has written, for [`output_harness`] to draw and a test to push into.
#[derive(Clone, Copy)]
struct RunLines(State<Arc<RunOutput>>);

/// The run output pane on its own, at a size a test can count rows in. Only the pane and
/// not the whole Scratchpad view: what is under test is where the rows are, and in the
/// view the same pane is a third of what is left after the editor.
fn output_harness() -> impl IntoElement {
    let lines = use_consume::<RunLines>().0;

    rect().expanded().content(Content::Flex).child(OutputPane {
        pad: pad_id("pad"),
        lines: lines.read().clone(),
        status: "Running".to_owned(),
        bad: false,
        key: DiffKey::None,
    })
}

/// One more line, exactly as an arriving [`RunEvent::Wrote`] adds it.
fn wrote(mut lines: State<Arc<RunOutput>>, text: &str) {
    Arc::make_mut(&mut lines.write()).push(crate::scratchpad::OutputLine {
        stream: Stream::Out,
        text: text.into(),
    });
}

/// Which output lines the pane is actually drawing. A `VirtualScrollView` builds only the
/// rows inside its viewport, so this is the answer to "where is the view", asked of the
/// rows themselves rather than of a number the pane kept.
fn drawn_lines(test: &TestingRunner) -> Vec<String> {
    labels(test)
        .into_iter()
        .filter(|text| text.starts_with("line "))
        .collect()
}

/// The output pane follows the newest line while the reader is at the bottom of it, lets
/// go the moment they scroll away, and takes it up again when they come back.
///
/// Headless because every part of it is a laid-out one: whether the newest row is on
/// screen is judged in rows against the viewport the pane was given, and the only honest
/// way to ask where the view ended up is which rows it built.
#[test]
fn the_output_pane_follows_the_newest_line_until_the_reader_scrolls_away() {
    let (mut test, lines) = TestingRunner::new(
        output_harness,
        (200., 200.).into(),
        |runner| {
            runner
                .provide_root_context(|| RunLines(State::create(Arc::new(RunOutput::default()))))
                .0
        },
        1.,
    );
    let settle = |test: &mut TestingRunner| {
        for _ in 0..4 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    // Enough lines to overflow the viewport several times over.
    for index in 0..12 {
        wrote(lines, &format!("line {index}"));
        settle(&mut test);
    }

    let following = drawn_lines(&test);
    assert!(
        following.contains(&"line 11".to_owned()),
        "the newest line is not on screen: {following:?}"
    );
    assert!(
        !following.contains(&"line 0".to_owned()),
        "nothing scrolled at all: {following:?}"
    );

    // The escape hatch: a wheel away from the bottom.
    test.scroll((100., 120.), (0., 100.));
    settle(&mut test);
    let away = drawn_lines(&test);
    assert!(
        !away.contains(&"line 11".to_owned()),
        "the wheel moved nothing: {away:?}"
    );

    // And the follow is released, not merely interrupted: three more lines arrive and the
    // reader is left looking at exactly the rows they scrolled to.
    for index in 12..15 {
        wrote(lines, &format!("line {index}"));
        settle(&mut test);
    }
    assert_eq!(drawn_lines(&test), away, "an arriving line pulled the view");

    // Back to the bottom by hand, which arms it again -- judged against the list as it is
    // now, which is three lines longer than the one they scrolled away from.
    test.scroll((100., 120.), (0., -1000.));
    settle(&mut test);
    let back = drawn_lines(&test);
    assert!(
        back.contains(&"line 14".to_owned()),
        "the wheel did not reach the bottom: {back:?}"
    );

    wrote(lines, "line 15");
    settle(&mut test);
    let rearmed = drawn_lines(&test);
    assert!(
        rearmed.contains(&"line 15".to_owned()),
        "coming back to the bottom did not take the follow up again: {rearmed:?}"
    );
}

/// The laid-out box of every `label()` whose text starts with `prefix`, in document order.
/// A wrap is a fact about the layout and about nothing else -- the same string is drawn
/// either way -- so a test about one has to read the areas rather than the texts.
fn label_boxes(test: &TestingRunner, prefix: &str) -> Vec<Area> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    let prefix = prefix.to_owned();
    test.find_many(move |node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .filter(|label| label.text.starts_with(&prefix))
            .map(|_| node.layout().area)
    })
}

/// A diagnostic too wide for the pane **wraps** instead of being cut off at its right
/// edge, which is what taking the list out of a fixed row height buys: both the sentence
/// rustc wrote and its own rendered block are paragraphs in a plain `ScrollView`.
///
/// Headless because a wrap is only ever a laid-out thing. The two diagnostics are the same
/// shape and differ only in how long their text is, so every number below is the long one
/// against the short one and none of them is an assertion about this machine's fonts: a
/// 300-character line does not fit in a 400-pixel window under any font there is, so a
/// label no wider than the window that is several lines tall is a label that wrapped.
#[test]
fn a_diagnostic_too_wide_for_the_pane_wraps_rather_than_being_cut() {
    let (mut test, _states, pad, _text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: scratchpad.manifest().err(),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    // Two errors of the same shape: one that fits and one that cannot. The rendered block
    // of the second is the `-->` line a span carries, which is the line the goal is about.
    let diagnostic = |message: &str, rendered: &str| Diagnostic {
        level: Level::Error,
        message: message.to_owned(),
        rendered: rendered.to_owned(),
        span: Some(crate::scratchpad::Span {
            file: SOURCE_FILE.to_owned(),
            line: 1,
            column: 1,
        }),
    };
    let mut pad = pad;
    pad.write().state_mut().built = Some(Build::Rejected {
        diagnostics: vec![
            diagnostic("short: nope", "  --> short"),
            diagnostic(
                &format!("long: {}", "mismatched ".repeat(40)),
                &format!("  --> long {}", "y".repeat(300)),
            ),
        ],
        message: String::new(),
    });
    for _ in 0..6 {
        test.sync_and_update();
    }

    let one = |prefix: &str| {
        let boxes = label_boxes(&test, prefix);
        assert_eq!(boxes.len(), 1, "{prefix} is not drawn exactly once");
        boxes[0]
    };
    let (short_message, long_message) = (one("short: "), one("long: "));
    let (short_rendered, long_rendered) = (one("  --> short"), one("  --> long"));

    // The sentence rustc wrote, beside the level and the place, is a paragraph now: the
    // long one stands where the short one does and is several times as tall.
    assert!(
        long_message.height() > short_message.height() * 2.0,
        "the message was cut rather than wrapped: {long_message:?} against {short_message:?}"
    );

    // And so is the rendered block under it. Bounded by the pane on one side -- a label
    // measuring out to its natural width is one that is about to be clipped -- and taller
    // than one line on the other, which together is what wrapping means.
    assert!(
        long_rendered.width() <= 400.0,
        "the span line measured out past the window: {long_rendered:?}"
    );
    assert!(
        long_rendered.height() > short_rendered.height() * 2.0,
        "the span line was cut rather than wrapped: {long_rendered:?}"
    );
}

/// The run output is the other answer to the same question: it stays a `VirtualScrollView`
/// stepping by one `item_size`, so a row cannot wrap, and a line too wide for the pane is
/// reached by scrolling sideways instead.
///
/// The load-bearing assertion is the wheel. A row measured to its content is what gives
/// the list something wider than its viewport to scroll over; a row filling the pane
/// leaves the two the same width, and freya scrolls a list no further than its content
/// goes -- so with the row filling the pane the wheel below moves nothing at all.
#[test]
fn a_wide_output_line_is_reached_by_scrolling_sideways() {
    let (mut test, lines) = TestingRunner::new(
        output_harness,
        (200., 200.).into(),
        |runner| {
            runner
                .provide_root_context(|| RunLines(State::create(Arc::new(RunOutput::default()))))
                .0
        },
        1.,
    );
    let settle = |test: &mut TestingRunner| {
        for _ in 0..4 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    wrote(lines, &format!("wide {}", "x".repeat(400)));
    settle(&mut test);

    let before = label_boxes(&test, "wide ");
    assert_eq!(before.len(), 1, "the line is not drawn exactly once");
    let before = before[0];

    // The premise, in two halves. One row and not two, the height being the `item_size`
    // the list steps by and a wrapping row being what would break it; and the whole line
    // measured, which is what there is to scroll sideways over. Neither is the assertion
    // -- `max_lines(1)` was already both of these -- they are what the row's own width now
    // inherits.
    assert!(
        before.height() <= code_row_height(),
        "an output row grew past the item size the list steps by: {before:?}"
    );
    assert!(
        before.width() > 200.0,
        "the line was cut to the pane instead of measured: {before:?}"
    );

    // The wheel, sideways. `freya`'s scroll views take `delta_x` as well as `delta_y`, and
    // what bounds it is the content's width against the viewport's.
    test.scroll((100., 120.), (-150., 0.));
    settle(&mut test);

    let after = label_boxes(&test, "wide ");
    let after = after[0];
    assert_eq!(
        after.width(),
        before.width(),
        "scrolling re-measured the row"
    );
    assert!(
        after.min_x() < before.min_x() - 100.0,
        "the sideways wheel moved nothing: {after:?} against {before:?}"
    );
}

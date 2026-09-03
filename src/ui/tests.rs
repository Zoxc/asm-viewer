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
/// `pointer_move` that sweeps it, and the release watched globally at the root, because the
/// button very often comes up somewhere the run does not reach.
fn harness() -> impl IntoElement {
    let marked = use_consume::<Marked>().0;

    let row = |index: usize| {
        rect()
            .width(Size::fill())
            .height(Size::px(20.0))
            .on_pointer_down(move |e: Event<PointerEventData>| {
                if e.button() == Some(MouseButton::Left) {
                    mark_press(marked, false, Pane::Assembly, None, index, None);
                }
            })
            .on_pointer_move(move |_| mark_drag(marked, Pane::Assembly, index, None))
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
        0,
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
/// theirs is: the source pane's run, whose first row is taken as a row index of whatever
/// tab is shown.
fn revealing_harness() -> impl IntoElement {
    let tab = use_consume::<KeptTab>().0;
    let at = use_consume::<KeptAt>().0;
    let open = use_consume::<KeptOpen>().0;
    let length = use_consume::<KeptLength>().0;
    let mut top = use_consume::<KeptTop>().0;
    let marked = use_consume::<Marked>().0;

    let controller = use_scroll_controller(ScrollConfig::default);
    let showing = tab.read().clone();
    let rows = *length.read();
    use_kept_position(
        at,
        move |tab: &String| open.peek().contains(tab),
        move |controller: &mut ScrollController| {
            let row = match owed_reveal(marked, Pane::Assembly) {
                None => return false,
                Some(Owing::Own(rows)) => *rows.rows().start(),
                Some(Owing::Pair(pair)) => *pair.rows.rows().start(),
            };
            reveal_made(marked, Pane::Assembly);
            reveal_row(controller, 100.0, row);
            true
        },
        controller,
        &showing,
        rows,
        0,
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
    let (mut test, (tab, top, marked)) = TestingRunner::new(
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
                    .provide_root_context(|| Marked(State::create(Marks::default())))
                    .0,
            )
        },
        1.,
    );
    let (mut tab, mut marked) = (tab, marked);
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
    marked.set(Marks {
        assembly: None,
        source: Some(picked_row(40, "b.rs", Owed::by(Pane::Assembly))),
    });
    tab.set("b".to_owned());
    let landed = top_row(&mut test);
    assert!(
        (30..=40).contains(&landed),
        "the arriving tab was put at row {landed} rather than at the revealed row"
    );
    assert!(owed_reveal(marked, Pane::Assembly).is_none());
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

/// The ten contexts `app()` provides, in one `ProjectStates`. A macro and not a
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
        // Provided but not returned, like `Active`: nothing here asserts on it, and the
        // Assembly pane's bar reads it wherever a harness mounts one.
        $runner.provide_root_context(|| Expanded(State::create(HashSet::new())));

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
            code_at: $runner
                .provide_root_context(|| CodeAt(State::create(Positions::default())))
                .0,
            history: $runner
                .provide_root_context(|| Hist(State::create(History::default())))
                .0,
            bookmarks: $runner
                .provide_root_context(|| Bookmarked(State::create(Bookmarks::default())))
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

/// A press that lands in the target but nowhere near the glyph still closes the tab: the
/// padding is as much of the control as the × is. The offset is measured from the
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
            states.code_at,
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
        states.code_at,
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
        states.code_at,
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
        states.code_at,
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
        states.code_at,
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
        states.code_at,
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
        states.code_at,
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
        states.code_at,
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
    let reading = use_consume::<Sections>().0;
    let window = use_consume::<Window>().0;

    use_analysis_with(
        asking,
        objects,
        history,
        analysis,
        located,
        reading,
        window,
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
            $runner
                .provide_root_context(|| Sections(State::create(Reading::default())))
                .0,
            $runner
                .provide_root_context(|| Window(State::create(None)))
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

    let (mut test, (asking, analysis, seen, _objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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

    let (mut test, (asking, analysis, seen, _objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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

/// A run of the one row `row` of `file`, picked out in the source pane with `owed` yet to
/// scroll to it.
fn picked_row(row: usize, file: &str, owed: Owed) -> Picked {
    Picked {
        rows: RowSelection {
            anchor: row,
            lead: row,
            dragging: false,
        },
        chars: None,
        file: Some(file.into()),
        owed,
    }
}

/// The same for the line `at`, as a landing plants it.
fn picked_line(at: &LinePos, owed: Owed) -> Picked {
    picked_row((at.line as usize).saturating_sub(1), &at.file, owed)
}

/// The line the source pane's run is of, where it is one row.
fn source_line(marked: State<Marks>) -> Option<LinePos> {
    let marks = marked.peek();
    let picked = marks.source.as_ref()?;
    Some(LinePos {
        file: picked.file.clone()?,
        line: picked.rows.anchor as u32 + 1,
    })
}

/// Whether `pane` still owes a scroll to the other pane's run.
fn owes_pair(marked: State<Marks>, pane: Pane) -> bool {
    matches!(owed_reveal(marked, pane), Some(Owing::Pair(_)))
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

    let (mut test, (asking, analysis, _seen, objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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

    let (mut test, (asking, analysis, _seen, objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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
        query: Query::line(at.clone()),
        objects: Vec::new(),
    };

    let kinds = |questions: &[Question]| -> Vec<&'static str> {
        questions
            .iter()
            .map(|question| match question {
                Question::Study(_) => "study",
                Question::Resolve { .. } => "resolve",
                Question::Locate { .. } => "locate",
                Question::Code(_) => "code",
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

    // A window of an object's code is a third kind, worked between the two: a newer
    // window replaces an older one and cancels neither of the others.
    let object = symbols[0].object.clone();
    let code = |window: Vec<usize>| {
        Question::Code(CodeAsk {
            object: object.clone(),
            code: None,
            window,
        })
    };
    let drained = newest(
        code(vec![0]),
        vec![locate(), Question::Study(symbols[0].clone()), code(vec![1])].into_iter(),
    );
    assert_eq!(kinds(&drained), ["study", "code", "locate"]);
    let Question::Code(kept) = &drained[1] else {
        panic!("the window kind went missing");
    };
    assert_eq!(kept.window, [1], "the older window won");
}

/// A window lands in the reading it was asked for: the skeleton with the first answer,
/// each stretch decoded the way its own tab decodes it, and the ask no longer pending.
#[test]
fn a_window_lands_in_the_reading_with_the_skeleton() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let (mut test, (_asking, _analysis, _seen, open, _history, _located, reading, window)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            move |runner| analysis_states!(runner, answer),
            1.,
        );
    let (mut open, mut reading, mut window) = (open, reading, window);
    open.write().push(object.clone());
    reading.set(Reading::of(Some(object.clone())));
    settle(&mut test);

    let ask = CodeAsk {
        object: object.clone(),
        code: None,
        window: vec![1, 2],
    };
    window.set(Some(ask.clone()));
    // Pending while the worker has it and idle once the answer lands; on a loaded machine
    // the two can be one pump apart, so only the landing is waited for.
    pump(&mut test, || {
        reading.peek().pending.is_none() && reading.peek().code.is_some()
    });

    let landed = reading.peek().clone();
    let code = landed
        .code
        .clone()
        .expect("the skeleton came with the answer");
    assert_eq!(code.sections().len(), 1);
    assert_eq!(landed.held.keys().copied().collect::<Vec<_>>(), [1, 2]);
    assert_eq!(landed.generation, 1);
    // The second stretch is `twice`, decoded exactly as its own tab would be.
    let twice = landed.held[&1].clone();
    let studied = twice.code.as_ref().expect("twice has a symbol");
    assert_eq!(studied.symbol.data.name, "twice");
    let own = Studied::new(studied.symbol.clone());
    assert_eq!(
        studied.assembly.as_ref().map(|a| a.instructions.len()),
        own.assembly.as_ref().map(|a| a.instructions.len())
    );
    // The line info is asked for afresh either way, so it is the answers that agree.
    assert_eq!(studied.lines.file, own.lines.file);
    assert_eq!(studied.lines.line, own.lines.line);
    assert_eq!(
        studied.lines.info.as_ref().map(|info| info.rows().len()),
        own.lines.info.as_ref().map(|info| info.rows().len())
    );
}

/// The worker decodes at most a chunk of the window it is asked for, in the order it was
/// asked, so a symbol click queued behind a window waits for a few functions and not for
/// the whole screen.
#[test]
fn a_window_is_decoded_a_chunk_at_a_time() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    // Nine stretches named, over an object that has three: the first eight are taken,
    // repeats and all, before a single one is looked at.
    let window: Vec<usize> = (0..9).map(|i| i % 3).collect();
    let Answer::Code { decoded, code, .. } = answer(Question::Code(CodeAsk {
        object: object.clone(),
        code: None,
        window: window.clone(),
    })) else {
        panic!("a window is answered with a window");
    };
    assert_eq!(decoded.len(), CHUNK);
    assert_eq!(
        decoded.iter().map(|(flat, _)| *flat).collect::<Vec<_>>(),
        window[..CHUNK]
    );
    assert_eq!(code.sections().len(), 1);
    // A stretch the listing has no place for is skipped, not answered.
    let Answer::Code { decoded, .. } = answer(Question::Code(CodeAsk {
        object,
        code: Some(code),
        window: vec![7],
    })) else {
        panic!("a window is answered with a window");
    };
    assert!(decoded.is_empty());
}

/// A window answer is taken only into the reading it is about: an answer arriving after
/// the reader moved to another object's code is dropped, and so is one out of a binary
/// closed since it was asked for.
#[test]
fn a_window_answer_for_a_reading_that_moved_on_is_dropped() {
    let (_path, objects) = fixture_objects(2);
    let (first, second) = (objects[0].clone(), objects[1].clone());

    let (started, starts) = async_channel::unbounded::<()>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let _ = started.send_blocking(());
        let _ = gated.recv_blocking();
        answer(question)
    };
    let (mut test, (_asking, _analysis, _seen, open, _history, _located, reading, window)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            move |runner| analysis_states!(runner, work),
            1.,
        );
    let (mut open, mut reading, mut window) = (open, reading, window);
    open.write().extend([first.clone(), second.clone()]);
    reading.set(Reading::of(Some(first.clone())));
    settle(&mut test);

    // Asked for the first object's code; the worker takes it and stops inside it.
    window.set(Some(CodeAsk {
        object: first.clone(),
        code: None,
        window: vec![0],
    }));
    pump(&mut test, || !starts.is_empty());
    starts.recv_blocking().expect("the worker started");

    // Meanwhile the reader is reading the second object's code.
    reading.set(Reading::of(Some(second.clone())));
    settle(&mut test);
    gate.send_blocking(()).expect("the gate");
    for _ in 0..40 {
        test.sync_and_update();
        std::thread::sleep(Duration::from_millis(2));
    }
    let landed = reading.peek().clone();
    assert!(landed.is_about(&second));
    assert!(
        landed.held.is_empty(),
        "an answer about the first object landed in the second's reading"
    );
    assert!(landed.code.is_none());

    // And the second's own window, asked while its file closes under it, lands nowhere.
    window.set(Some(CodeAsk {
        object: second.clone(),
        code: None,
        window: vec![0],
    }));
    pump(&mut test, || !starts.is_empty());
    starts.recv_blocking().expect("the worker started");
    open.write().retain(|object| !Arc::ptr_eq(object, &second));
    settle(&mut test);
    gate.send_blocking(()).expect("the gate");
    for _ in 0..40 {
        test.sync_and_update();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        reading.peek().held.is_empty(),
        "an answer out of a closed binary landed"
    );
}

/// What is held is bounded: a stretch farther than `KEEP` from the window an answer was
/// asked for is let go when the answer lands.
#[test]
fn a_stretch_far_from_the_window_is_let_go() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let code = Arc::new(CodeListing::new(&object));
    let empty = || Stretched {
        code: None,
        gap: None,
    };
    let mut reading = Reading::of(Some(object.clone()));
    let ask = |window: Vec<usize>| CodeAsk {
        object: object.clone(),
        code: Some(code.clone()),
        window,
    };

    // Two answers, one at each end of a long listing (the indices need not exist to be
    // held; only the worker cares).
    assert!(reading.take(&ask(vec![0]), code.clone(), vec![(0, empty())]));
    assert!(reading.take(
        &ask(vec![KEEP * 3]),
        code.clone(),
        vec![(KEEP * 3, empty())]
    ));
    assert_eq!(reading.held.keys().copied().collect::<Vec<_>>(), [KEEP * 3]);
    // One within reach of the window stays.
    assert!(reading.take(
        &ask(vec![KEEP * 2]),
        code.clone(),
        vec![(KEEP * 2, empty())]
    ));
    assert_eq!(
        reading.held.keys().copied().collect::<Vec<_>>(),
        [KEEP * 2, KEEP * 3]
    );
    assert_eq!(reading.generation, 3);

    // An answer with another skeleton is not this reading's.
    let other = Arc::new(CodeListing::new(&object));
    assert!(!reading.take(&ask(vec![1]), other, vec![(1, empty())]));
    assert_eq!(reading.generation, 3);
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

    let (mut test, (_asking, _analysis, _seen, objects, _history, located, _reading, _window)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            |runner| analysis_states!(runner, answer),
            1.,
        );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![wanted.object.clone(), twin.clone()]);
    test.sync_and_update();

    located.write().asked = Some(Query::line(at.clone()));
    assert!(located.peek().pending() == Some(&Query::line(at.clone())));
    pump(&mut test, || located.peek().found.is_some());

    let state = located.peek().clone();
    assert!(state.pending().is_none());
    let found = state.found.expect("the line was looked for");
    assert!(found.of.at == at);
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
    located.write().asked = Some(Query::line(barren.clone()));
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.of.at == barren)
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
    let (first, second) = (
        Query::line(a_line_of(&symbols[0])),
        Query::line(a_line_of(&symbols[1])),
    );
    assert!(
        first != second,
        "the fixture's first two symbols share a line"
    );

    let (started, starts) = async_channel::unbounded::<Query>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let work = move |question: Question| {
        let Question::Locate { query, objects } = question else {
            panic!("this test asks only about locations");
        };
        let _ = started.send_blocking(query.clone());
        let _ = gated.recv_blocking();
        answer(Question::Locate { query, objects })
    };

    let (mut test, (_asking, _analysis, _seen, objects, _history, located, _reading, _window)) =
        TestingRunner::new(
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
    assert!(located.peek().found.as_ref().expect("answered").of == second);
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

    let (mut test, (asking, analysis, _seen, objects, _history, located, _reading, _window)) =
        TestingRunner::new(
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
    located.write().asked = Some(Query::line(at.clone()));
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
    assert!(found.of.at == at);
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

    let (mut test, (_asking, _analysis, _seen, objects, _history, located, _reading, _window)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            |runner| analysis_states!(runner, answer),
            1.,
        );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![wanted.object.clone(), twin.clone()]);
    test.sync_and_update();

    located.write().asked = Some(Query::line(at.clone()));
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
    assert!(found.of.at == at);
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
    assert!(state.asked == Some(Query::line(at.clone())));
    assert!(state.found.expect("stands").symbols.0.is_empty());
}

/// The Locations view and nothing else, over the project's states and a `Located` the
/// test writes directly: what is under test is what the panel draws of an answer and
/// what a row does, not how the answer got there. `use_clear_focus` is mounted because a
/// row's press is answered by it.
fn locations_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let driven = use_consume::<Drives>().0;
    use_land(active, marked, landing, driven);

    rect().expanded().child(LocationsTab)
}

/// The contexts [`locations_harness`] reads beside the project's.
#[derive(Clone, Copy)]
struct LocationStates {
    located: State<Located>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    analysis: State<Analyzed>,
}

macro_rules! location_states {
    ($runner:expr) => {{
        let states = project_states!($runner);
        let marked = $runner
            .provide_root_context(|| Marked(State::create(Marks::default())))
            .0;
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        let located = $runner
            .provide_root_context(|| Locations(State::create(Located::default())))
            .0;
        let analysis = $runner
            .provide_root_context(|| Analysis(State::create(Analyzed::default())))
            .0;
        (
            states,
            LocationStates {
                located,
                marked,
                landing,
                analysis,
            },
        )
    }};
}

/// Where the text reading `text` was laid out, for a press on it: a `label()` of it, or
/// the paragraph one of whose spans it is -- a code row's text is one paragraph of spans.
fn label_area(test: &TestingRunner, text: &str) -> Option<Area> {
    use freya::elements::{label::LabelElement, paragraph::ParagraphElement};
    use std::any::Any;

    test.find(|node, _element| {
        let element = node.element();
        let element = element.as_ref() as &dyn Any;
        let label = element
            .downcast_ref::<LabelElement>()
            .is_some_and(|label| label.text == text);
        let span = element
            .downcast_ref::<ParagraphElement>()
            .is_some_and(|paragraph| paragraph.spans.iter().any(|span| span.text == text));
        (label || span).then(|| node.layout().area)
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

    located.write().asked = Some(Query::line(at.clone()));
    settle(&mut test);
    let finding = format!(
        "Finding locations for {}:{}\u{2026}",
        file_name(&at.file),
        at.line
    );
    assert!(labels(&test).contains(&finding), "{:?}", labels(&test));

    located.write().found = Some(Found::new(Query::line(at.clone()), Vec::new()));
    settle(&mut test);
    let nothing = format!("No code compiled from {}:{}", file_name(&at.file), at.line);
    assert!(labels(&test).contains(&nothing), "{:?}", labels(&test));

    located.write().found = Some(Found::new(
        Query::line(at.clone()),
        vec![wanted.clone(), twin],
    ));
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
    located.write().asked = Some(Query::line(at.clone()));
    located.write().found = Some(Found::new(Query::line(at.clone()), vec![wanted.clone()]));
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
    located.write().asked = Some(Query::line(at.clone()));
    located.write().found = Some(Found::new(Query::line(at.clone()), vec![wanted.clone()]));
    settle(&mut test);
    assert!(location.marked.peek().source.is_none());

    let row = label_area(&test, "sum_to").expect("the row is drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    assert!(states.open.active() == Some(document));
    assert!(
        source_line(location.marked) == Some(at.clone()),
        "the line was not picked out"
    );
    let picked = location
        .marked
        .peek()
        .source
        .clone()
        .expect("checked above");
    assert!(picked.owed == Owed::BOTH);
    assert!(
        location.landing.peek().is_none(),
        "the landing was not spent by the document it named"
    );
    // Both panes are owed the scroll -- the source pane to its own run, the assembly
    // pane to the pair -- and each pays its own.
    assert!(owes_pair(location.marked, Pane::Assembly));
    assert!(matches!(
        owed_reveal(location.marked, Pane::Source),
        Some(Owing::Own(_))
    ));
    reveal_made(location.marked, Pane::Source);
    assert!(owes_pair(location.marked, Pane::Assembly));
    assert!(owed_reveal(location.marked, Pane::Source).is_none());
}

/// Landing on the document already on top picks the line out at once: `activate` then
/// changes nothing, so no effect would run to spend a landing.
#[test]
fn landing_on_the_document_already_on_top_picks_the_line_out_at_once() {
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
        location.marked,
        location.landing,
        document.clone(),
        at.clone(),
    );
    assert!(
        location.landing.peek().is_none(),
        "a landing was left to an effect that cannot run"
    );
    assert!(source_line(location.marked) == Some(at.clone()));
    settle(&mut test);
    assert!(
        source_line(location.marked) == Some(at.clone()),
        "the run was dropped though no document changed"
    );
    assert!(states.open.active() == Some(document));
}

/// The companion file follows the source pane's run when the symbol's line info names
/// its file, and is the symbol's own file otherwise -- so a Locations row opens on the
/// file the line is in, while a run in a file the listing knows nothing of changes none.
#[test]
fn a_run_in_a_file_the_listing_names_is_the_companion() {
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
    let marks = |file: Option<&str>| Marks {
        assembly: None,
        source: file.map(|file| picked_row(0, file, Owed::BOTH)),
    };
    let file_of = |file: Option<&str>| {
        source_side(Some(&document), &analysis, &marks(file))
            .expect("a companion")
            .file()
            .clone()
    };

    // A distinct allocation of a file the info names, as the app hands about.
    let elsewhere: String = named[0].to_string();
    assert!(file_of(None) == own);
    assert!(file_of(Some(&elsewhere)).as_ref() == elsewhere.as_str());
    // A run in a file the symbol knows nothing of: nothing to switch to.
    assert!(file_of(Some("nowhere.rs")) == own);
    // And a source-driven tab's subject is its own file whatever is picked out.
    let subject = Document::Source("subject.rs".into());
    assert!(
        source_side(Some(&subject), &analysis, &marks(Some(&elsewhere)))
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

    let (mut test, (asking, analysis, _seen, objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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
    located.write().asked = Some(Query::line(at.clone()));
    located.write().subject = Some(at.file.clone());
    located.write().found = Some(Found::new(Query::line(at.clone()), vec![wanted.clone()]));
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
    assert!(source_line(location.marked) == Some(at.clone()));
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
        states.code_at,
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
/// one for another document picks nothing out.
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
        location.marked.peek().source.is_none(),
        "a landing picked a line out in another document"
    );
    assert!(
        location.landing.peek().is_none(),
        "a spent landing was left lying"
    );
}

/// A function is the same question over its lines: asked of every line the gcc fixture's
/// three functions span, the worker answers with each **once**, though each function's
/// rows name most of its lines, and the answer stays about the row it was asked from.
#[test]
fn an_instance_query_answers_each_symbol_once() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let at = a_line_of(&wanted);
    // `add` is on 21-24, `twice` on 26-29 and `sum_to` on 31-39 of the fixture; the
    // comment above them holds no code at all.
    let function = |name: &str, lines: RangeInclusive<u32>| Function {
        name: name.to_owned(),
        lines,
    };
    let query = Query::function(at.clone(), &function("everything", 21..=39));

    let (mut test, (_asking, _analysis, _seen, objects, _history, located, _reading, _window)) =
        TestingRunner::new(
            analysis_harness,
            (100., 100.).into(),
            |runner| analysis_states!(runner, answer),
            1.,
        );
    let (mut objects, mut located) = (objects, located);
    objects.set(vec![wanted.object.clone()]);
    test.sync_and_update();

    located.write().asked = Some(query.clone());
    assert!(located.peek().pending() == Some(&query));
    pump(&mut test, || located.peek().found.is_some());
    let found = located.peek().found.clone().expect("answered");
    assert!(found.of == query);
    assert!(found.of.at == at);
    let names: Vec<&str> = found
        .symbols
        .0
        .iter()
        .map(|symbol| symbol.data.name.as_str())
        .collect();
    assert_eq!(names, ["add", "twice", "sum_to"]);

    // The one function alone, and the lines nothing was compiled from.
    let query = Query::function(at.clone(), &function("sum_to", 31..=39));
    located.write().asked = Some(query.clone());
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.of == query)
    });
    let found = located.peek().found.clone().expect("answered");
    assert_eq!(found.symbols.0.len(), 1);
    assert_eq!(found.symbols.0[0].data.name, "sum_to");

    let query = Query::function(at.clone(), &function("comment", 1..=19));
    located.write().asked = Some(query.clone());
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.of == query)
    });
    assert!(located
        .peek()
        .found
        .as_ref()
        .expect("answered")
        .symbols
        .0
        .is_empty());
}

/// Asked about a function, the panel says so in each of its states: it names the
/// function rather than a line, and its rows are instances.
#[test]
fn the_locations_panel_names_the_function_an_instance_query_is_of() {
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
    let query = Query::function(
        at.clone(),
        &Function {
            name: "sum_to".to_owned(),
            lines: 31..=39,
        },
    );

    let (mut test, (_states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;

    located.write().asked = Some(query.clone());
    settle(&mut test);
    assert!(
        labels(&test).contains(&"Finding instances of sum_to\u{2026}".to_owned()),
        "{:?}",
        labels(&test)
    );

    located.write().found = Some(Found::new(query.clone(), Vec::new()));
    settle(&mut test);
    assert!(
        labels(&test).contains(&"No code compiled from sum_to".to_owned()),
        "{:?}",
        labels(&test)
    );

    located.write().found = Some(Found::new(query.clone(), vec![wanted.clone()]));
    settle(&mut test);
    assert!(
        labels(&test).contains(&"1 instance of sum_to".to_owned()),
        "{:?}",
        labels(&test)
    );

    located.write().found = Some(Found::new(query, vec![wanted, twin]));
    settle(&mut test);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"2 instances of sum_to".to_owned()),
        "{drawn:?}"
    );
    assert_eq!(
        drawn.iter().filter(|text| **text == "sum_to").count(),
        2,
        "{drawn:?}"
    );
}

/// The row lit is the symbol the panes are **drawing**, not the active document: in a
/// source-driven tab the active document is a file, and the lit row is the one answer
/// the panel gives to which instance the tab's assembly side is on.
#[test]
fn the_row_lit_is_the_symbol_drawn_and_not_the_active_document() {
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
    let tab = Document::Source(at.file.clone());

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let (mut located, mut analysis) = (location.located, location.analysis);
    activate(states.open, states.history, Some(tab.clone()), Visit::Went);
    located.write().asked = Some(Query::line(at.clone()));
    located.write().found = Some(Found::new(
        Query::line(at.clone()),
        vec![wanted.clone(), twin.clone()],
    ));
    settle(&mut test);

    // Where the two rows are, by the labels they carry, in the answer's order.
    let rows: Vec<Area> = test.find_many(|node, _element| {
        use freya::elements::label::LabelElement;
        use std::any::Any;
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .filter(|label| label.text == "sum_to")
            .map(|_| node.layout().area)
    });
    assert_eq!(rows.len(), 2, "two rows are drawn");
    let lit = |test: &TestingRunner| -> Vec<f32> {
        test.find_many(|node, element| {
            (element.style().background == Fill::Color(palette().selected_bg))
                .then_some(node.layout().area.origin.y)
        })
    };
    let holds = |row: &Area, y: f32| row.origin.y >= y && row.origin.y < y + list_row_height();

    // Nothing drawn: nothing lit, though a tab is active.
    assert!(lit(&test).is_empty(), "a row is lit with nothing drawn");

    // The twin drawn for the tab: its row and not the first's.
    analysis.write().shown = Some(Shown {
        ask: Ask::Source {
            at: at.clone(),
            chosen: Some(twin.clone()),
        },
        studied: Studied::new(twin.clone()),
    });
    settle(&mut test);
    let lit_rows = lit(&test);
    assert_eq!(lit_rows.len(), 1, "{lit_rows:?}");
    assert!(
        holds(&rows[1], lit_rows[0]),
        "the lit row is not the twin's"
    );
    assert!(!holds(&rows[0], lit_rows[0]));

    // The other drawn: the light moves.
    analysis.write().shown = Some(Shown {
        ask: Ask::Source {
            at: at.clone(),
            chosen: Some(wanted.clone()),
        },
        studied: Studied::new(wanted.clone()),
    });
    settle(&mut test);
    let lit_rows = lit(&test);
    assert_eq!(lit_rows.len(), 1, "{lit_rows:?}");
    assert!(
        holds(&rows[0], lit_rows[0]),
        "the lit row is not the first's"
    );
}

/// The file a source-driven tab is about, for [`source_menu_harness`].
#[derive(Clone)]
struct Subject(Arc<str>);

/// The Source pane over a source-driven tab, with the viewer a context menu needs in an
/// ancestor scope -- which `app()` mounts on the root and no other harness here does.
fn source_menu_harness() -> impl IntoElement {
    let file = use_consume::<Subject>().0;
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(SourcePane {
            document: Document::Source(file),
        })
}

/// Where the label reading `text` is, as a point to put the pointer on.
fn centre_of(test: &TestingRunner, text: &str) -> (f64, f64) {
    let area = label_area(test, text).unwrap_or_else(|| panic!("{text:?} is drawn"));
    (
        (area.origin.x + area.width() / 2.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    )
}

/// A right-click, which no `TestingRunner` method sends. The popup is placed at the last
/// global pointer move and not at the event's point, so the pointer is moved there first;
/// and the button is released, because the menu opens on the down and the up of the same
/// gesture is the one global press `ContextMenuViewer` swallows -- left out, it would
/// swallow the click on an entry instead and the menu would stay open.
fn right_click(test: &mut TestingRunner, at: (f64, f64)) {
    use freya::prelude::platform::{MouseEventName, PlatformEvent};

    test.move_cursor(at);
    test.sync_and_update();
    for name in [MouseEventName::MouseDown, MouseEventName::MouseUp] {
        test.send_event(PlatformEvent::Mouse {
            name,
            cursor: at.into(),
            button: Some(MouseButton::Right),
        });
        test.sync_and_update();
    }
    settle(test);
}

/// A source row inside a function offers that function's instances beside the line's
/// locations, and choosing them asks for the function's lines from that row, chosen for
/// the tab; a row outside any function offers the line alone.
#[test]
fn a_source_row_inside_a_function_offers_its_instances() {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-instances-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("instances.c");
    std::fs::write(
        &path,
        "int add(int a, int b)\n{\n    return a + b;\n}\n\nint x;\n",
    )
    .expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));

    let (mut test, (states, located)) = TestingRunner::new(
        source_menu_harness,
        (500., 400.).into(),
        {
            let file = file.clone();
            move |runner| {
                let states = project_states!(runner);
                runner.provide_root_context(|| Marked(State::create(Marks::default())));
                runner.provide_root_context(|| Shift(State::create(false)));
                runner.provide_root_context(|| CodeRows(State::create(None)));
                runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
                runner.provide_root_context(|| Subject(file.clone()));
                let located = runner
                    .provide_root_context(|| Locations(State::create(Located::default())))
                    .0;
                (states, located)
            }
        },
        1.,
    );
    activate(
        states.open,
        states.history,
        Some(Document::Source(file.clone())),
        Visit::Went,
    );
    settle(&mut test);

    // The third row, by its gutter number, is inside `add`.
    let row = centre_of(&test, "3\u{a0}");
    right_click(&mut test, row);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"Find all locations".to_owned()),
        "{drawn:?}"
    );
    assert!(
        drawn.contains(&"Find instances of add".to_owned()),
        "{drawn:?}"
    );

    let entry = centre_of(&test, "Find instances of add");
    test.move_cursor(entry);
    test.press_cursor(entry);
    test.release_cursor(entry);
    settle(&mut test);
    let at = LinePos {
        file: file.clone(),
        line: 3,
    };
    let asked = located.peek().asked.clone().expect("the entry asked");
    assert!(
        asked
            == Query::function(
                at,
                &Function {
                    name: "add".to_owned(),
                    lines: 1..=4,
                }
            )
    );
    assert!(located.peek().subject.as_deref() == Some(&*file));
    assert!(
        !labels(&test).contains(&"Find instances of add".to_owned()),
        "the menu stayed open"
    );

    // The sixth row is outside any function.
    let row = centre_of(&test, "6\u{a0}");
    right_click(&mut test, row);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"Find all locations".to_owned()),
        "{drawn:?}"
    );
    assert!(
        !drawn.iter().any(|text| text.starts_with("Find instances")),
        "{drawn:?}"
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

    let (
        mut test,
        (
            (_asking, _analysis, _seen, objects, _history, located, _reading, _window),
            content,
            sidebar,
        ),
    ) = TestingRunner::new(
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

    find_locations(located, content, Query::line(at.clone()), None);
    assert!(on_top(sidebar) == Some(Tab::View(View::Locations)));
    assert!(located.peek().pending() == Some(&Query::line(at.clone())));
    pump(&mut test, || located.peek().found.is_some());
    let found = located.peek().found.clone().expect("answered");
    assert!(found.of.at == at);
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
    find_locations(located, content, Query::line(at.clone()), None);
    assert!(located.peek().pending() == Some(&Query::line(at.clone())));
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

    let (mut test, (asking, analysis, seen, objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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

    let (mut test, (asking, analysis, seen, objects, _history, _located, _reading, _window)) =
        TestingRunner::new(
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

    let (mut test, marked) = TestingRunner::new(
        project_harness,
        (100., 100.).into(),
        |runner| {
            runner
                .provide_root_context(|| Marked(State::create(Marks::default())))
                .0
        },
        1.,
    );
    let mut marked = marked;
    test.sync_and_update();

    let click = |marked: &mut State<Marks>| {
        marked.set(Marks {
            assembly: None,
            source: Some(picked_line(&at, Owed::by(Pane::Assembly))),
        })
    };
    click(&mut marked);
    test.sync_and_update();

    // The pane that is owed it looks, twice, and the request is still there both times:
    // the first look is the listing being left, the second the one that arrived.
    assert!(owes_pair(marked, Pane::Assembly));
    assert!(owes_pair(marked, Pane::Assembly));
    // And the other pane is owed nothing: a click asks the pane it was not made in.
    assert!(owed_reveal(marked, Pane::Source).is_none());
    // Which is also what a `reveal_made` from it must not undo.
    reveal_made(marked, Pane::Source);
    test.sync_and_update();
    assert!(owes_pair(marked, Pane::Assembly));

    // Made, and so owed exactly once. The run itself stays: it is what lights the rows.
    reveal_made(marked, Pane::Assembly);
    test.sync_and_update();
    assert!(owed_reveal(marked, Pane::Assembly).is_none());
    assert!(source_line(marked) == Some(at.clone()));

    // A second click on the same line is a second request.
    click(&mut marked);
    test.sync_and_update();
    assert!(owes_pair(marked, Pane::Assembly));
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
        let marked = $runner
            .provide_root_context(|| Marked(State::create(Marks::default())))
            .0;
        $runner.provide_root_context(|| Shift(State::create(false)));
        $runner.provide_root_context(|| Locations(State::create(Located::default())));
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        $runner.provide_root_context(|| {
            Analysis(State::create(Analyzed {
                shown: Some($shown),
                ..Analyzed::default()
            }))
        });
        // The row's door into the object's code reads these three, and lands through
        // the last.
        $runner.provide_root_context(|| Sections(State::create(Reading::default())));
        $runner.provide_root_context(|| Window(State::create(None)));
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        (states, marked, landing)
    }};
}

/// A listing scrolled by a separator's distance puts a *different* separator in the slot
/// one was in. Freya matches siblings by key alone, so separators sharing the type's
/// default key are taken for one row that never moved, the moves around it leave the
/// scope graph disagreeing with the element tree, and an instruction row's props reach a
/// separator's scope -- where the downcast inside `freya-core`'s `element.rs` unwraps
/// `None`. Keyed apart, the scroll just scrolls (`notes/upstream/freya.md`).
///
/// `sum_to`'s separators sit at listing rows 7 and 15; eight rows is the distance, and a
/// pane this tall shows eleven, so the slot a separator was in holds the other afterwards.
#[test]
fn scrolling_past_a_separator_keeps_every_row_its_own() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    assert_eq!(
        studied.lanes.row_of(7),
        8,
        "the fixture's first block boundary moved"
    );
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };

    let (mut test, (_states, _marked, _landing)) = TestingRunner::new(
        listing_harness,
        (500., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let drawn = labels(&test);
    // `sum_to` is the fixture's third function, at 30h.
    assert!(
        drawn.iter().any(|text| text == "0000000000000030 "),
        "the listing did not open at its top: {drawn:?}"
    );

    test.scroll((250., 100.), (0., -8.0 * code_row_height() as f64));
    settle(&mut test);
    let drawn = labels(&test);
    assert!(
        !drawn.iter().any(|text| text == "0000000000000030 "),
        "the listing did not scroll: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|text| text == "0000000000000061 "),
        "the row past the boundary is not drawn: {drawn:?}"
    );
}

/// `sum_to`, shown: the fixture's one function with branches inside it, so its listing has
/// a gutter and two separators, and rows wider than a 250px pane.
fn shown_sum_to() -> Shown {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to),
    }
}

/// Every rect drawn in `color`, by its box.
fn rects_with(test: &TestingRunner, color: Color) -> Vec<Area> {
    test.find_many(move |node, element| {
        (element.style().background == Fill::Color(color)).then_some(node.layout().area)
    })
}

/// Every paragraph drawn, by its box: a source line's text and an instruction's operands
/// are paragraphs where an address or a line number is a label.
fn paragraph_boxes(test: &TestingRunner) -> Vec<Area> {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .map(|_| node.layout().area)
    })
}

/// The right edge of the widest text drawn: the furthest any label or paragraph reaches.
fn content_right(test: &TestingRunner) -> f32 {
    labels_with_areas(test)
        .into_iter()
        .map(|(_, area)| area)
        .chain(paragraph_boxes(test))
        .map(|area| area.max_x())
        .fold(0.0, f32::max)
}

/// The gutter's vertical strokes, by their box: see
/// `every_lane_gets_a_column_of_its_own`.
fn lane_strokes(test: &TestingRunner) -> Vec<Area> {
    let height = code_row_height();
    test.find_many(move |node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().branch_fg)
            && area.width() == BRANCH_STROKE
            && area.height() >= height / 2.0)
            .then_some(area)
    })
}

/// The leftmost of `areas`.
fn leftmost(areas: &[Area]) -> f32 {
    areas
        .iter()
        .map(|area| area.min_x())
        .fold(f32::MAX, f32::min)
}

/// An instruction too wide for the Assembly pane is reached by scrolling sideways, and
/// its gutter goes with it.
///
/// The load-bearing assertion is the wheel. A row measured to its content is what gives
/// the list something wider than its viewport to scroll over; a row filling the pane
/// leaves the two the same width, and freya scrolls a list no further than its content
/// goes -- so with the row filling the pane the wheel below moves nothing. The gutter is
/// a child of the row and its strokes are placed from the gutter's own left edge, so it
/// moves exactly as far as the addresses do.
#[test]
fn a_wide_instruction_is_reached_by_scrolling_sideways() {
    let shown = shown_sum_to();
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (250., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    // The premise: a row reaches past the pane, and the gutter is drawn.
    let right = content_right(&test);
    assert!(
        right > 250.0,
        "no row is wider than the pane, so the wheel below proves nothing: {right}"
    );
    let strokes = lane_strokes(&test);
    assert!(
        !strokes.is_empty(),
        "no lane is drawn in the rows on screen"
    );
    let addresses = labels_with_areas(&test)
        .into_iter()
        .filter(|(text, _)| text.trim_end().len() == 16)
        .map(|(_, area)| area)
        .collect::<Vec<_>>();
    assert!(!addresses.is_empty());
    let (labels_before, strokes_before) = (leftmost(&addresses), leftmost(&strokes));

    test.scroll((125., 150.), (-150., 0.));
    settle(&mut test);

    let addresses = labels_with_areas(&test)
        .into_iter()
        .filter(|(text, _)| text.trim_end().len() == 16)
        .map(|(_, area)| area)
        .collect::<Vec<_>>();
    let strokes = lane_strokes(&test);
    let (labels_after, strokes_after) = (leftmost(&addresses), leftmost(&strokes));
    assert!(
        labels_after < labels_before - 100.0,
        "the sideways wheel moved nothing: {labels_after} against {labels_before}"
    );
    assert_eq!(
        strokes_after - strokes_before,
        labels_after - labels_before,
        "the gutter did not scroll with its rows"
    );
}

/// A line too wide for the Source pane is reached by scrolling sideways: the other pane,
/// and the other kind of row, of the test above.
#[test]
fn a_wide_source_line_is_reached_by_scrolling_sideways() {
    let directory = run_directory(line!());
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("wide.c");
    std::fs::write(&path, format!("int x;\n// {}\nint y;\n", "x".repeat(400)))
        .expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));
    let (mut test, _states, _showing) = source_file_harness(&file, (300., 200.));

    let widest = |test: &TestingRunner| {
        paragraph_boxes(test)
            .into_iter()
            .max_by(|a, b| a.width().total_cmp(&b.width()))
            .expect("a line is drawn")
    };
    let before = widest(&test);
    // The premise: the paragraph measures to its whole line. It did before too -- a
    // paragraph inside a row filling the pane measured itself out past the row; it was
    // the row around it that left the list nothing wider than itself to scroll over.
    assert!(
        before.width() > 300.0,
        "the line was cut to the pane instead of measured: {before:?}"
    );

    test.scroll((150., 100.), (-150., 0.));
    settle(&mut test);

    let after = widest(&test);
    assert_eq!(
        after.width(),
        before.width(),
        "scrolling re-measured the line"
    );
    assert!(
        after.min_x() < before.min_x() - 100.0,
        "the sideways wheel moved nothing: {after:?} against {before:?}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A picked-out row's wash runs as wide as the widest row drawn, and no wider than the
/// pane where nothing overflows it.
///
/// The wash is the row's own background, so a row measured to its content would wash
/// only as far as its text; every row is floored to the widest instead, the pane's width
/// being the floor's own floor. The block rule between two blocks fills its row, so it
/// runs as far too. The two halves fail for opposite reasons: with plain content-sized
/// rows the first is short, and with the pane's width folded into the floor the second is
/// long.
#[test]
fn a_picked_rows_wash_runs_as_wide_as_the_widest_row() {
    let wash_at = |width: f32| {
        let shown = shown_sum_to();
        let (mut test, (_states, marked, _landing)) = TestingRunner::new(
            listing_harness,
            (width, 300.).into(),
            |runner| listing_states!(runner, shown),
            1.,
        );
        let mut marked = marked;
        settle(&mut test);
        // The first row, `push rbp`: the shortest an instruction row gets.
        marked.set(Marks {
            assembly: Some(picked_row(
                0,
                "/fixture/line_fixture.c",
                Owed::by(Pane::Source),
            )),
            source: None,
        });
        settle(&mut test);
        let wash = rects_with(&test, palette().text_select_bg);
        assert_eq!(wash.len(), 1, "one row is picked out: {wash:?}");
        let rules = rects_with(&test, palette().block_rule);
        assert!(
            !rules.is_empty(),
            "no block rule is drawn in the rows on screen"
        );
        (wash[0], rules, content_right(&test))
    };

    // Narrow: the wash reaches as far as the widest row does, and so does every rule.
    let (wash, rules, right) = wash_at(250.);
    assert!(
        right > 250.0,
        "no row is wider than the pane, so the wash proves nothing: {right}"
    );
    assert!(
        wash.max_x() >= right,
        "the wash stops at {} where the widest row reaches {right}",
        wash.max_x()
    );
    for rule in rules {
        assert!(
            rule.max_x() >= right - 1.0,
            "a block rule stops at {} where the widest row reaches {right}",
            rule.max_x()
        );
    }

    // Wide: the wash is the pane's and no more.
    let (wash, _rules, right) = wash_at(600.);
    assert!(right < 600.0, "a row is wider than the pane: {right}");
    assert!(
        wash.width() > 500.0 && wash.max_x() <= 600.0,
        "the wash is not the pane's width: {wash:?}"
    );
}

/// The file the Source pane shows, as a state the test moves.
#[derive(Clone)]
struct Showing(State<Arc<str>>);

/// The Source pane over whatever file [`Showing`] names.
fn showing_harness() -> impl IntoElement {
    let showing = use_consume::<Showing>().0;
    rect().expanded().child(SourcePane {
        document: Document::Source(showing.read().clone()),
    })
}

/// The Source pane over `file`, with the contexts its rows read, in a window `size`,
/// the file's document activated so the tab's place-keeping sees it open.
fn source_file_harness(
    file: &Arc<str>,
    size: (f32, f32),
) -> (TestingRunner, ProjectStates, State<Arc<str>>) {
    let (mut test, (states, showing)) = TestingRunner::new(
        showing_harness,
        size.into(),
        {
            let file = file.clone();
            move |runner| {
                let states = project_states!(runner);
                runner.provide_root_context(|| Marked(State::create(Marks::default())));
                runner.provide_root_context(|| Shift(State::create(false)));
                runner.provide_root_context(|| CodeRows(State::create(None)));
                runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
                runner.provide_root_context(|| Locations(State::create(Located::default())));
                let showing = runner
                    .provide_root_context(|| Showing(State::create(file.clone())))
                    .0;
                (states, showing)
            }
        },
        1.,
    );
    activate(
        states.open,
        states.history,
        Some(Document::Source(file.clone())),
        Visit::Went,
    );
    settle(&mut test);
    (test, states, showing)
}

/// The widest row is the widest row **of this listing**: a pane moved from a file with a
/// long line to one whose lines all fit has nothing to scroll sideways over.
///
/// The list is one component across the switch, and the widest row it holds is kept under
/// the file's identity: a key that no longer matches is a floor of nothing. Without that
/// the second file's rows would still be floored to the first's widest, and the wheel
/// below would move them.
#[test]
fn a_listings_extent_does_not_outlive_it() {
    let directory = run_directory(line!());
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let wide = directory.join("wide.c");
    std::fs::write(&wide, format!("// {}\nint x;\n", "x".repeat(400)))
        .expect("writing the source file");
    let narrow = directory.join("narrow.c");
    std::fs::write(&narrow, "int y;\nint z;\n").expect("writing the source file");
    let wide: Arc<str> = Arc::from(wide.to_str().expect("a utf-8 temporary path"));
    let narrow: Arc<str> = Arc::from(narrow.to_str().expect("a utf-8 temporary path"));
    let (mut test, states, mut showing) = source_file_harness(&wide, (300., 200.));

    // The line numbers: the one label every row of either file draws.
    let numbers = |test: &TestingRunner| {
        labels_with_areas(test)
            .into_iter()
            .filter(|(text, _)| text.ends_with('\u{a0}'))
            .map(|(_, area)| area)
            .collect::<Vec<_>>()
    };

    let before = leftmost(&numbers(&test));
    test.scroll((150., 100.), (-150., 0.));
    settle(&mut test);
    let scrolled = leftmost(&numbers(&test));
    assert!(
        scrolled < before - 100.0,
        "the wide file did not scroll sideways: {scrolled} against {before}"
    );

    showing.set(narrow.clone());
    activate(
        states.open,
        states.history,
        Some(Document::Source(narrow.clone())),
        Visit::Went,
    );
    settle(&mut test);
    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "2\u{a0}"),
        "the second file is not shown: {drawn:?}"
    );

    let before = leftmost(&numbers(&test));
    test.scroll((150., 100.), (-150., 0.));
    settle(&mut test);
    let after = leftmost(&numbers(&test));
    assert_eq!(
        after, before,
        "the second file scrolled sideways over the first file's widest row"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Pressing a branch's displacement puts the row it lands on on screen **and picks that
/// row out** -- the run a press on the target row itself would have made, of the file the
/// target was compiled from, with the Source pane owed the scroll and the Assembly pane
/// not, since it has just been given one. It is still not a navigation: the document does
/// not change and nothing is pushed onto the history, so a Back button never has to undo
/// reading further down the same function.
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
    // The rows this test tells apart, worked out from the lanes rather than from the
    // pane: the row the forward `jmp` lands on, and for the backward jump both its own
    // row and its target's, since the press starts on the one and must end up on the
    // other.
    let assembly = studied.assembly.clone().expect("sum_to decodes");
    let row_at = |address: u64| {
        let index = assembly
            .instructions
            .iter()
            .position(|instruction| instruction.address == address)
            .unwrap_or_else(|| panic!("no instruction at {address:X}"));
        studied.lanes.row_of(index)
    };
    let landing_row = row_at(0x61);
    let backward_row = row_at(0x67);
    let backward_lands_on = row_at(0x4B);
    let target_file = studied
        .position(
            assembly
                .instructions
                .iter()
                .position(|instruction| instruction.address == 0x61)
                .expect("checked above"),
        )
        .expect("the row the jump lands on is on a line")
        .file;

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    // `jmp short 61h` is the sixth instruction of the fixture's loop and lands on the
    // fifteenth, far enough down that a pane this tall is not showing it.
    let landing = "0000000000000061 ";

    let (mut test, (states, marked, _landing)) = TestingRunner::new(
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
    // The row landed on is the picked-out one now, and the row the press started on is
    // not: this is the half that holds whether or not the object carries line info.
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("following a jump picked out no row");
    assert_eq!(
        picked.rows.rows().collect::<Vec<_>>(),
        vec![landing_row],
        "the row picked out is not the one the jump lands on"
    );
    assert!(
        picked.file.as_ref() == Some(&target_file),
        "the run is of {:?} where the jump lands in {target_file}",
        picked.file
    );
    assert!(
        picked.owed.source,
        "the source side was not owed the scroll"
    );
    assert!(
        !picked.owed.assembly,
        "the listing was asked to scroll twice"
    );

    // And again on the backward jump: the press lands on 67h, and what is picked out
    // afterwards is 4Bh's row -- the row jumped *to*, not the row the pointer was over.
    let operand = label_area(&test, "4Bh").expect("the backward jump is on screen now");
    let at = (
        (operand.origin.x + operand.width() as f32 / 2.0) as f64,
        (operand.origin.y + operand.height() as f32 / 2.0) as f64,
    );
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);

    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the backward jump picked out no row");
    assert_eq!(
        picked.rows.rows().collect::<Vec<_>>(),
        vec![backward_lands_on],
        "the row picked out is not the one the backward jump lands on"
    );
    assert!(
        !picked.rows.contains(backward_row),
        "the press bubbled into the row and picked out where it started"
    );

    // Still not a navigation: nothing was opened or visited by either press.
    assert!(states.open.active().is_none());
    assert_eq!(states.history.peek().recent().count(), 0);
}

/// A row a branch lands on has a separator **row of its own** above it, so the listing
/// reads as the basic blocks it is: the gap between two instruction rows across a boundary
/// is two row heights where everywhere else it is one. A row that is nobody's target has
/// nothing above it, and every row is still exactly the `item_size` the scroll view was
/// given -- one number for the whole listing, which is why the separator is a row and not
/// a taller one.
///
/// Headless because every part of it is a question about the real tree: which rows a
/// `VirtualScrollView` built, where it put them, and what carries the rule.
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

    // Every rule the separators drew, by the area it was laid out in.
    let ruled: Vec<Area> = test.find_many(|node, element| {
        (element.style().background == Fill::Color(palette().block_rule))
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

    let mut rows = rows;
    rows.sort_by(|(_, a), (_, b)| a.origin.y.total_cmp(&b.origin.y));
    let drawn: Vec<u64> = rows.iter().map(|(address, _)| *address).collect();
    // The first row is never separated -- a boundary over the top of the symbol says
    // nothing -- so it is not among the gaps this counts.
    let expected: Vec<u64> = targets
        .iter()
        .copied()
        .filter(|address| drawn.iter().skip(1).any(|drawn| drawn == address))
        .collect();
    assert!(
        expected.len() >= 2,
        "the fixture's sum_to is branched to {} times: {expected:0X?}",
        expected.len()
    );
    assert!(
        drawn.len() > expected.len() + 1,
        "every drawn row is a branch target, so a gap above all of them would pass"
    );

    // The whole of the claim: consecutive instruction rows sit one row height apart, and
    // two apart exactly where a branch lands -- which is the separator taking a row.
    let height = code_row_height();
    let mut separated: Vec<u64> = Vec::new();
    // Where a rule is owed: the separator row's own middle, which is the midpoint of the
    // two instruction rows it holds apart -- their labels being centred in them.
    let mut middles: Vec<f32> = Vec::new();
    for pair in rows.windows(2) {
        let [(_, above), (address, below)] = pair else {
            continue;
        };
        let gap = below.origin.y - above.origin.y;
        if (gap - height * 2.0).abs() < 0.5 {
            separated.push(*address);
            let centre = |area: &Area| area.origin.y + area.height() / 2.0;
            middles.push((centre(above) + centre(below)) / 2.0);
        } else {
            assert!(
                (gap - height).abs() < 0.5,
                "the rows above {address:0X} are {gap} apart, neither one row nor two"
            );
        }
    }
    separated.sort_unstable();
    assert_eq!(
        separated, expected,
        "the separator rows are not above the rows the branches land on"
    );

    // The pitch above is the height claim: every row is exactly the `item_size` the scroll
    // view was given, or the rows would drift out of the pitch a virtual list lays them
    // out on. What is left to say is that the rule lives *inside* its row rather than
    // adding to it, and that nothing else in the listing carries one.
    assert_eq!(
        ruled.len(),
        expected.len(),
        "the rule is drawn {} times for {} separators",
        ruled.len(),
        expected.len()
    );
    // And it is centred in the row it is drawn in, and clear of the gutter: a rule struck
    // through the branch lines would read as one of them breaking.
    let gutter_right = rows
        .iter()
        .map(|(_, area)| area.origin.x)
        .fold(f32::MAX, f32::min);
    for middle in &middles {
        assert!(
            ruled.iter().any(|area| {
                (area.origin.y + area.height() / 2.0 - middle).abs() <= 1.0
                    && area.height() <= 1.0
                    && area.origin.x <= gutter_right
            }),
            "no hairline centred at {middle} and clear of the gutter at {gutter_right}: \
             {ruled:?}"
        );
    }
}

/// The gutter runs straight through a separator: its lanes are drawn in the same columns
/// there as in the rows above and below, and a branch crossing a boundary is drawn the
/// whole height of it rather than stopping at the gap.
///
/// This is what was wrong when the separator first landed. An instruction row takes three
/// pixels of horizontal padding and the separator took none, so every lane stepped three
/// pixels sideways at every block it crossed and each branch line in the listing came out
/// kinked. Nothing else in the suite could see it: the model `Lanes` hands the rows was
/// right the whole time, and only the laid-out strokes say where they really went.
#[test]
fn the_gutter_runs_straight_through_a_separator() {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let lanes = studied.lanes.clone();
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    // The gutter's vertical strokes only: the horizontal run to the listing and the two
    // strokes of an arrowhead are the same colour and are not in a lane.
    let height = code_row_height();
    let strokes: Vec<Area> = test.find_many(|node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().branch_fg)
            && area.width() == BRANCH_STROKE
            && area.height() >= height / 2.0)
            .then_some(area)
    });

    let mut columns: Vec<String> = strokes
        .iter()
        .map(|area| area.origin.x.to_string())
        .collect();
    columns.sort();
    columns.dedup();
    assert!(
        lanes.width >= 2,
        "the fixture draws {} lane, so one column would pass",
        lanes.width
    );
    assert_eq!(
        columns.len(),
        lanes.width,
        "the gutter is drawn in {} columns for {} lanes: {columns:?}",
        columns.len(),
        lanes.width
    );

    // And a lane really does cross a boundary: a stroke a whole row tall, drawn where no
    // instruction row is -- which is the separator.
    let rows: Vec<f32> = test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .map(|label| label.text.to_string())
            .filter(|text| text.trim_end().len() == 16)
            .map(|_| node.layout().area.origin.y + node.layout().area.height() / 2.0)
    });
    let crossing = strokes.iter().filter(|area| {
        area.height() == height
            && !rows
                .iter()
                .any(|centre| (area.origin.y + height / 2.0 - centre).abs() < 1.0)
    });
    assert!(
        crossing.count() > 0,
        "no lane is drawn through a separator, so the columns above prove nothing"
    );
}

/// The tab a pane harness is mounted for, where the test needs it to be something other
/// than the tab the listing in [`Analysis`] belongs to -- which is the disagreement the
/// Assembly pane's bar has a rule about -- or needs to change it, which is what switching
/// tabs does to this pane. Named around the dock's own `Tab`.
#[derive(Clone, Copy)]
struct PaneTab(State<Document>);

/// The Assembly pane for the tab [`PaneTab`] names, over whatever listing is in
/// [`Analysis`].
fn tab_pane_harness() -> impl IntoElement {
    let document = use_consume::<PaneTab>().0.read().clone();

    rect().expanded().child(AssemblyPane { document })
}

/// A symbol with both spellings, out of the fixture's object so that it is a symbol of an
/// open binary. Its address is nowhere in that object, so nothing is decoded for it and
/// the pane draws a word instead of a listing -- which the bar is above either way.
fn mangled_symbol() -> Symbol {
    Symbol {
        object: fixture_symbols()[0].object.clone(),
        data: Arc::new(SymbolData {
            name: "_ZN6viewer2ui8assembly12AssemblyPane6render17h0123456789abcdefE".to_owned(),
            demangled: Some("viewer::ui::assembly::AssemblyPane::render".to_owned()),
            address: 0x1000,
            section: None,
            size: 0,
        }),
    }
}

/// The bar names the symbol in both spellings, the mangled one under the demangled: a
/// symbol is named nowhere else in the window but on its tab, where `short_name` has cut
/// it down and the mangled original never appears at all.
#[test]
fn the_assembly_pane_names_the_symbol_in_both_spellings() {
    let symbol = mangled_symbol();
    let shown = Shown {
        ask: Ask::Symbol(symbol.clone()),
        studied: Studied::new(symbol.clone()),
    };

    let (mut test, (_states, _marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    let drawn = labels(&test);
    assert!(
        drawn.contains(&symbol.data.demangled.clone().expect("it is demangled")),
        "{drawn:?}"
    );
    assert!(drawn.contains(&symbol.data.name), "{drawn:?}");
}

/// **The bar names what the pane is drawing and not what is selected.** The two disagree
/// for as long as the worker takes, and the Info pane this replaces read the selection --
/// so a bar over a listing of one function would have been naming another.
#[test]
fn the_bar_names_the_drawn_symbol_and_not_the_tab() {
    let drawn_symbol = mangled_symbol();
    let elsewhere = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let shown = Shown {
        ask: Ask::Symbol(drawn_symbol.clone()),
        studied: Studied::new(drawn_symbol.clone()),
    };
    let tab = Document::Assembly(Selection::Symbol(elsewhere.clone()));

    let (mut test, (_states, _marked, _landing)) = TestingRunner::new(
        tab_pane_harness,
        (600., 300.).into(),
        move |runner| {
            runner.provide_root_context(|| PaneTab(State::create(tab.clone())));
            listing_states!(runner, shown)
        },
        1.,
    );
    settle(&mut test);

    let drawn = labels(&test);
    assert!(drawn.contains(&drawn_symbol.data.name), "{drawn:?}");
    assert!(
        !drawn.contains(&elsewhere.data.name),
        "the bar named the tab's symbol and not the drawn one: {drawn:?}"
    );
}

/// A tab that is a whole object is the one selection no listing is ever worked out for
/// (`ask` answers `None` for it), so the bar has nothing in hand to name and falls back to
/// the document. It is what the Info pane said about an object, and it has to keep being
/// said somewhere.
#[test]
fn an_object_tab_is_named_by_its_object() {
    let object = fixture_symbols()[0].object.clone();
    let tab = Document::Assembly(Selection::Object(object.clone()));

    let (mut test, _states) = TestingRunner::new(
        tab_pane_harness,
        (600., 300.).into(),
        move |runner| {
            runner.provide_root_context(|| PaneTab(State::create(tab.clone())));
            runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
            project_states!(runner)
        },
        1.,
    );
    settle(&mut test);

    let drawn = labels(&test);
    assert!(drawn.contains(&object.name), "{drawn:?}");
    // The body still says there is no listing, which is the other half of the answer.
    assert!(
        drawn.contains(&"No symbol selected".to_owned()),
        "{drawn:?}"
    );
}

/// The bar's disclosure triangle, wherever it was laid out.
fn triangle_of(test: &TestingRunner) -> (f64, f64) {
    match label_area(test, "\u{25b8}") {
        Some(_) => centre_of(test, "\u{25b8}"),
        None => centre_of(test, "\u{25be}"),
    }
}

/// The section under the bar says what the Info pane said, and a little more -- the address
/// and the object a symbol came from, neither of which was shown anywhere before.
#[test]
fn the_expanded_section_says_what_the_info_pane_said() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let section = sum_to
        .data
        .section
        .as_ref()
        .expect("sum_to is in a section")
        .name
        .clone();
    let object = sum_to.object.name.clone();
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to.clone()),
    };

    let (mut test, (states, _marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 400.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    // Opened, or the table has no id for this tab and the bar files its flag nowhere --
    // which is a bar with no triangle to press.
    activate(
        states.open,
        states.history,
        Some(Document::Assembly(Selection::Symbol(sum_to.clone()))),
        Visit::Went,
    );
    settle(&mut test);
    assert!(
        !labels(&test).contains(&section),
        "the section was open before anything was pressed"
    );

    let triangle = triangle_of(&test);
    test.move_cursor(triangle);
    test.press_cursor(triangle);
    test.release_cursor(triangle);
    settle(&mut test);

    let drawn = labels(&test);
    for field in ["Section", "Address", "Declared", "Extent", "Object"] {
        assert!(drawn.contains(&field.to_owned()), "{field}: {drawn:?}");
    }
    assert!(drawn.contains(&section), "{drawn:?}");
    assert!(drawn.contains(&object), "{drawn:?}");
    assert!(
        drawn.contains(&format!("{:016X}", sum_to.data.address)),
        "{drawn:?}"
    );
}

/// **Open or shut is the tab's and not the pane's.** Both panes are mounted afresh for
/// every document, so a flag inside this one would go with the tab the reader left -- and a
/// section that shuts itself every time the reader looks at something else reads as a bug
/// rather than as a setting.
///
/// Two objects rather than two symbols, an object being the selection that needs no
/// analysis to be named; they are two parses of one file, so they say the same things and
/// it is whether the section is there at all that answers.
#[test]
fn the_symbol_section_is_remembered_per_tab() {
    let (_path, objects) = fixture_objects(2);
    let (first, second) = (objects[0].clone(), objects[1].clone());
    let tab = |object: &Arc<Object>| Document::Assembly(Selection::Object(object.clone()));

    let (mut test, (states, showing)) = TestingRunner::new(
        tab_pane_harness,
        (600., 400.).into(),
        {
            let first = first.clone();
            move |runner| {
                let showing = runner
                    .provide_root_context(|| PaneTab(State::create(tab(&first))))
                    .0;
                runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
                (project_states!(runner), showing)
            }
        },
        1.,
    );
    // Both tabs open, or the table has no id for either and the bar files its flag nowhere.
    let went = |target: Document| activate(states.open, states.history, Some(target), Visit::Went);
    went(tab(&first));
    went(tab(&second));
    settle(&mut test);

    let triangle = triangle_of(&test);
    test.move_cursor(triangle);
    test.press_cursor(triangle);
    test.release_cursor(triangle);
    settle(&mut test);
    assert!(
        labels(&test).contains(&"Format".to_owned()),
        "the triangle did not open the section: {:?}",
        labels(&test)
    );

    let mut showing = showing;
    showing.set(tab(&second));
    settle(&mut test);
    assert!(
        !labels(&test).contains(&"Format".to_owned()),
        "the other tab came up with the first tab's section open: {:?}",
        labels(&test)
    );

    showing.set(tab(&first));
    settle(&mut test);
    assert!(
        labels(&test).contains(&"Format".to_owned()),
        "the section was not there when the tab that opened it came back: {:?}",
        labels(&test)
    );
}

/// Whether [`source_pane_harness`] has the pane mounted at all, which is how a test asks
/// for the first open of a tab and then for a later one: `app()` mounts both panes afresh
/// for every document, so an unmount and a remount is what leaving a tab and coming back
/// does to this pane.
#[derive(Clone, Copy)]
struct Mounted(State<bool>);

/// The Source pane over a listing the test puts into [`Analysis`] itself, with no worker
/// between the two, mounted on demand. The document it draws is the tab the listing
/// belongs to, which is what `app()` hands it.
fn source_pane_harness() -> impl IntoElement {
    let analysis = use_consume::<Analysis>().0;
    let mounted = use_consume::<Mounted>().0;
    let document = analysis
        .read()
        .shown
        .as_ref()
        .map(|shown| asked_of(&shown.ask))
        .unwrap_or_else(|| Document::Source(Arc::from("")));

    rect()
        .expanded()
        .maybe_child(mounted().then(|| SourcePane { document }.into_element()))
}

/// A tab opened for the first time puts its source side on the **symbol's own lines**,
/// and not at the top of a file the symbol may be a hundred lines into. A row remembered
/// for the tab still wins: this is the first open it answers and not every one.
///
/// Headless because the answer is a scroll offset a `VirtualScrollView` turns into rows,
/// asked of the pane the way the reader asks it -- by which line numbers are drawn.
#[test]
fn a_tab_opens_its_source_side_on_the_symbols_own_lines() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let mut studied = Studied::new(sum_to.clone());
    let line = studied
        .lines
        .line
        .expect("the gcc fixture opens sum_to on a line");

    // A file of this machine's own, the path the fixture's DWARF names being the build
    // machine's, and long enough that the symbol's line is nowhere near the top of it.
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-opening-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("opening.c");
    let text: String = (1..=200)
        .map(|n| format!("int line_{n}(void);\n"))
        .collect();
    std::fs::write(&path, text).expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));
    studied.lines.file = Some(file.clone());
    assert!(
        line > CONTEXT_ROWS as u32 + 1,
        "sum_to opens on line {line}, which is the top of the file anyway"
    );

    let document = Document::Assembly(Selection::Symbol(sum_to.clone()));
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, (states, mounted)) = TestingRunner::new(
        source_pane_harness,
        (500., 300.).into(),
        |runner| {
            let mounted = runner
                .provide_root_context(|| Mounted(State::create(true)))
                .0;
            let (states, _marked, _landing) = listing_states!(runner, shown);
            (states, mounted)
        },
        1.,
    );
    // Open before the first pass: a position is written down only for a tab that is open,
    // which is what the second half of this test leans on.
    activate(
        states.open,
        states.history,
        Some(document.clone()),
        Visit::Went,
    );

    // Which lines the gutter is drawing, which is where the pane is. The number carries
    // the non-breaking space skia is stopped from trimming; the companion header's label
    // is the file's name and parses as nothing.
    let drawn = |test: &TestingRunner| {
        let mut rows: Vec<u32> = labels(test)
            .into_iter()
            .filter_map(|text| {
                text.strip_suffix('\u{a0}')
                    .and_then(|number| number.parse().ok())
            })
            .collect();
        rows.sort_unstable();
        rows
    };
    let land = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
        drawn(test)
    };

    let rows = land(&mut test);
    assert!(
        rows.contains(&line),
        "the first open does not show line {line}, where sum_to begins: {rows:?}"
    );
    assert!(
        !rows.contains(&1),
        "the first open is at the top of the file rather than at the symbol: {rows:?}"
    );
    // With the margin a reveal keeps above the row it scrolls to, and no more: the line
    // is meant to be readable in place, not pushed to the bottom of the pane.
    let top = *rows.first().expect("the gutter drew no line numbers");
    assert!(
        (line.saturating_sub(CONTEXT_ROWS as u32)..=line).contains(&top),
        "the pane opened at line {top}, not on sum_to's own line {line}"
    );

    // And a tab that has been somewhere comes back to where it was, over the symbol's
    // own lines: the first open is the only one this answers.
    let mut src_at = states.src_at;
    src_at.write().remember(document.clone(), 120);
    let mut mounted = mounted;
    mounted.set(false);
    land(&mut test);
    mounted.set(true);

    let rows = land(&mut test);
    assert!(
        rows.contains(&121) && !rows.contains(&line),
        "the tab did not come back to the row it was left at: {rows:?}"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// The rule a separator row draws between two basic blocks is laid out on whole device
/// pixels too, and in the same row of them the gutter's horizontal run takes -- a rule
/// and a run crossing one row must not be half a pixel apart.
///
/// The same even row height as the gutter's own test, and for the same reason: centring a
/// one-pixel rect in a 26px row put it at 12.5, spread over the two pixels either side.
#[test]
fn a_block_rule_lands_on_whole_device_pixels() {
    // 10.5pt is 14 logical pixels, so a code row is 26.
    set_fonts(fixed_fonts(9.0, 10.5));
    assert_eq!(code_row_height(), 26.0);

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let instructions = studied
        .assembly
        .as_ref()
        .expect("the fixture disassembles sum_to")
        .instructions
        .len();
    let separators = studied.lanes.listing_rows(instructions) - instructions;
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    let rules: Vec<Area> = test.find_many(|node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().block_rule)).then_some(area)
    });
    assert!(
        separators > 0 && !rules.is_empty(),
        "the fixture draws no separator, so there is no rule to place"
    );

    let whole = |edge: f32| edge == edge.round();
    for area in &rules {
        assert!(
            whole(area.origin.y) && whole(area.origin.y + area.height()),
            "a block rule was laid out at {area:?}, which is spread across two device \
             pixels and drawn as two grey ones"
        );
    }

    // And in the row of pixels the gutter's own horizontal run is drawn in, measured off
    // the run rather than worked out again here: the two are one line across the row
    // where a branch lands on a block boundary, and half a pixel apart reads as a step.
    let runs: Vec<Area> = test.find_many(|node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().branch_fg)
            && area.height() == BRANCH_STROKE
            && area.width() > BRANCH_STROKE)
            .then_some(area)
    });
    let run = runs
        .first()
        .expect("the fixture draws no horizontal run, so there is nothing to agree with");
    // Where in its own row each sits. Every row is `code_row_height()` tall, so the
    // listing's own origin drops out of both sides.
    let within = |area: &Area| area.origin.y.rem_euclid(code_row_height());
    for area in &rules {
        assert_eq!(
            within(area),
            within(run),
            "a block rule sits at {area:?}, not where the gutter's run crosses a row"
        );
    }
}

/// Every axis-aligned stroke of the gutter is laid out on whole device pixels, so a
/// one-pixel line is one lit row of pixels and not two grey ones beside crisp text.
///
/// The row height here is deliberately **even**: a code row is the mono size plus 12, so
/// half of it -- the line the horizontal run and the arrowhead are drawn along -- is a
/// whole pixel, and placing a stroke's *centre* there left it spanning 12.5 to 13.5. An
/// odd row height hid the fault by accident, which is why this pins a font size rather
/// than taking whatever the desktop resolves to.
#[test]
fn the_gutter_puts_its_strokes_on_whole_device_pixels() {
    // 10.5pt is 14 logical pixels, so a code row is 26.
    set_fonts(fixed_fonts(9.0, 10.5));
    assert_eq!(code_row_height(), 26.0);

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to.clone()),
    };
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    // The lanes and the horizontal runs: a stroke exactly one pixel thick one way. The
    // two barbs of an arrowhead are neither, being drawn thicker on purpose.
    let strokes: Vec<Area> = test.find_many(|node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().branch_fg)
            && (area.width() == BRANCH_STROKE || area.height() == BRANCH_STROKE))
            .then_some(area)
    });
    assert!(
        strokes.iter().any(|area| area.width() > BRANCH_STROKE),
        "no horizontal run was drawn, so the y of a stroke proves nothing"
    );

    let whole = |edge: f32| edge == edge.round();
    for area in &strokes {
        assert!(
            whole(area.origin.x)
                && whole(area.origin.x + area.width())
                && whole(area.origin.y)
                && whole(area.origin.y + area.height()),
            "a gutter stroke was laid out at {area:?}, which is spread across two \
             device pixels and drawn as two grey ones"
        );
    }

    // And the arrowhead is the one thing deliberately off the grid: no placement can
    // align a 30 degree diagonal, so it is weighted instead -- half a device pixel more
    // ink, so the two rows the antialiasing spreads it over do not read lighter than the
    // run it points along.
    let barbs: Vec<Area> = test.find_many(|node, element| {
        let area = node.layout().area;
        (element.style().background == Fill::Color(palette().branch_fg)
            && area.width() == ARROW_STROKE)
            .then_some(area)
    });
    assert!(!barbs.is_empty(), "the fixture draws no arrowhead");
    for area in &barbs {
        assert_eq!(area.height(), BRANCH_STROKE + 0.5, "at {area:?}");
    }
}

/// A document's two panes as the dock mounts them: the same [`DocumentBody`] `tab_content`
/// builds, over whichever document is active. The dock itself is left out -- the strip has
/// no vote in which pane is which -- but the two states the split is held in are not, being
/// what the panels are sized from.
fn panes_harness() -> impl IntoElement {
    let open = use_open();
    // Read and not peeked: this is the harness's whole subscription to a tab being
    // activated, and `Active` is a memo and a beat behind.
    let id = {
        let (dock, docs) = (open.dock.read(), open.docs.read());
        active_document(&dock, &docs).and_then(|document| docs.id_of(&document))
    };

    rect()
        .expanded()
        .maybe_child(id.map(|id| DocumentBody { id }.into_element()))
}

/// **The side a tab is driven from is the left-hand pane.** An assembly-driven tab keeps
/// its listing there with the file it was compiled from beside it; a source-driven tab is
/// the other way round, the file the reader is reading leading and the symbol its clicked
/// line compiled into following. Headless because the two panes are the same two
/// components in both kinds of tab and neither is told where it was put, so only the boxes
/// they were laid out in can say which side is which.
#[test]
fn the_side_a_tab_is_driven_from_is_the_left_hand_pane() {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    /// What the leading pane is given, as a percentage of the window.
    const LEADING: f32 = 70.0;

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let mut studied = Studied::new(sum_to.clone());
    // A companion file no filesystem has, so the source side of the assembly-driven tab is
    // one findable label rather than a listing of somebody else's build directory.
    studied.lines.file = Some("own.c".into());
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };

    let (mut test, (states, _marked, _landing)) = TestingRunner::new(
        panes_harness,
        (600., 300.).into(),
        |runner| {
            let states = listing_states!(runner, shown);
            // The two `app()` provides beside the project's, which `DocumentBody` sizes
            // its panels from. Deliberately *uneven*: the number is the leading pane's
            // width in both kinds of tab, so the wide half moving with the swap is half of
            // what is asserted below.
            runner.provide_root_context(|| SplitRatio(State::create(LEADING)));
            runner.provide_root_context(|| {
                Splits(State::create(ResizableContext {
                    direction: Direction::Horizontal,
                    ..Default::default()
                }))
            });
            states
        },
        1.,
    );
    settle(&mut test);

    // The assembly side draws one 16-digit address per row and the source side one label
    // saying the file could not be opened: where each of those was laid out is where that
    // pane is.
    let assembly_at = |test: &TestingRunner| {
        test.find_many(|node, _element| {
            (node.element().as_ref() as &dyn Any)
                .downcast_ref::<LabelElement>()
                .map(|label| label.text.to_string())
                .filter(|text| text.trim_end().len() == 16)
                .map(|_| node.layout().area.origin.x)
        })
        .into_iter()
        .fold(f32::MAX, f32::min)
    };
    let source_at = |test: &TestingRunner, file: &str| {
        label_area(test, &format!("Source file not found: {file}"))
            .expect("the source side says it could not open the file")
            .origin
            .x
    };

    // Where the leading panel ends: its share of the window less the handle between the
    // two. A pane's own padding is a few pixels and cannot carry a label across it.
    let boundary = (600.0 - ResizableContext::HANDLE_SIZE) * LEADING / 100.0;
    let went = |target: Document| activate(states.open, states.history, Some(target), Visit::Went);

    went(Document::Assembly(Selection::Symbol(sum_to.clone())));
    settle(&mut test);
    let (asm, src) = (assembly_at(&test), source_at(&test, "own.c"));
    assert!(
        asm < boundary && boundary < src,
        "an assembly-driven tab drew its listing at {asm} and its source at {src}"
    );

    let file = "/nowhere/main.rs";
    went(Document::Source(Arc::from(file)));
    settle(&mut test);
    let (asm, src) = (assembly_at(&test), source_at(&test, file));
    assert!(
        src < boundary && boundary < asm,
        "a source-driven tab drew its source at {src} and its listing at {asm}"
    );
}

/// A source file the debug info recorded a checksum for is compared with the file the pane
/// opened: a row over the source says so when the bytes differ, and nothing is said over a
/// file that matches or over one no checksum was recorded for. Headless because the notice
/// is a row the pane draws or does not; the file is a temporary one holding `abc`, whose
/// MD5 is the published vector, and the line info naming it is built by hand as the PDB
/// backend would have built it.
#[test]
fn a_source_file_that_differs_from_the_one_compiled_is_flagged() {
    use analysis::{LineInfo, LineRow, SourceHash};

    let dir = std::env::temp_dir().join(format!("viewer-stale-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the temp directory is writable");
    let path = dir.join("own.c");
    std::fs::write(&path, b"abc").expect("the temp directory is writable");
    let file: Arc<str> = Arc::from(path.to_str().expect("a UTF-8 temp path"));

    let compiled = SourceHash::Md5([
        0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1, 0x7f,
        0x72,
    ]);
    let another = SourceHash::Md5([0; 16]);

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");

    for (recorded, flagged) in [
        (Some(another), true),
        (Some(compiled), false),
        (None, false),
    ] {
        let mut studied = Studied::new(sum_to.clone());
        let rows = vec![LineRow {
            range: sum_to.data.address..sum_to.data.address + 1,
            file: Some(0),
            line: Some(1),
            column: None,
        }];
        studied.lines.info = LineInfo::new(rows, vec![(file.clone(), recorded)]).map(Arc::new);
        studied.lines.file = Some(file.clone());
        studied.lines.line = Some(1);
        let shown = Shown {
            ask: Ask::Symbol(sum_to.clone()),
            studied,
        };

        let (mut test, (states, _marked, _landing)) = TestingRunner::new(
            panes_harness,
            (600., 300.).into(),
            |runner| {
                let states = listing_states!(runner, shown);
                runner.provide_root_context(|| SplitRatio(State::create(50.0)));
                runner.provide_root_context(|| {
                    Splits(State::create(ResizableContext {
                        direction: Direction::Horizontal,
                        ..Default::default()
                    }))
                });
                states
            },
            1.,
        );
        settle(&mut test);
        activate(
            states.open,
            states.history,
            Some(Document::Assembly(Selection::Symbol(sum_to.clone()))),
            Visit::Went,
        );
        settle(&mut test);

        // The file itself is up either way: the notice is over it, not instead of it.
        assert!(
            label_area(&test, &format!("Source file not found: {file}")).is_none(),
            "{recorded:?}: the file was not opened"
        );
        assert_eq!(
            label_area(&test, STALE_SOURCE).is_some(),
            flagged,
            "{recorded:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
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
        // The code colours, on the pane each is drawn on: the assembly pane has no
        // comments and no strings, and the source pane is the plain one.
        let both = [
            ("address_fg", palette.address_fg),
            ("keyword_fg", palette.keyword_fg),
            ("operand_fg", palette.operand_fg),
            ("literal_fg", palette.literal_fg),
            ("punctuation_fg", palette.punctuation_fg),
            ("name_fg", palette.name_fg),
            ("name_hover_fg", palette.name_hover_fg),
        ];
        for (name, color) in both {
            for (surface, background) in [
                ("asm_pane_bg", palette.asm_pane_bg),
                ("pane_bg", palette.pane_bg),
            ] {
                let ratio = contrast(color, background);
                assert!(ratio >= 3.0, "{theme} {name} on {surface}: {ratio:.2}");
            }
            // And on the selection, which the text engine paints under the characters
            // and the row wears whole: a translucent wash the code was always read
            // through when its rows were picked out, so the floor is the one the address
            // column recedes to and not the pane's -- what is read there is what is about
            // to be copied, and the wash is what says so.
            let ratio = contrast(color, blend(palette.text_select_bg, palette.asm_pane_bg));
            let floor = 2.0;
            assert!(
                ratio >= floor,
                "{theme} {name} on text_select_bg: {ratio:.2}"
            );
        }

        // The five the source pane has to itself: a disassembly holds no strings,
        // comments, attributes, types or call names, so these are only ever read on
        // `pane_bg`.
        for (name, color) in [
            ("string_fg", palette.string_fg),
            ("comment_fg", palette.comment_fg),
            ("attribute_fg", palette.attribute_fg),
            ("type_fg", palette.type_fg),
            ("function_fg", palette.function_fg),
        ] {
            let ratio = contrast(color, palette.pane_bg);
            assert!(ratio >= 3.0, "{theme} {name} on pane_bg: {ratio:.2}");
        }

        // An attribute is scaffolding around the code and is meant to recede: legible,
        // and quieter than both the keyword it was drawn in before and the punctuation
        // it sits among. A relationship rather than a value, since what makes it read as
        // scaffolding is the gap and not the grey.
        let attribute = contrast(palette.attribute_fg, palette.pane_bg);
        for (name, color) in [
            ("keyword_fg", palette.keyword_fg),
            ("punctuation_fg", palette.punctuation_fg),
            ("name_fg", palette.name_fg),
        ] {
            let louder = contrast(color, palette.pane_bg);
            assert!(
                attribute < louder,
                "{theme} attribute_fg {attribute:.2} vs {name} {louder:.2}"
            );
        }

        // The chrome, on all three of the surfaces it is written over. `address_fg` is
        // here as well as over the code panes above: it is the app's dim text everywhere,
        // and the Assembly pane's bar draws a symbol's mangled spelling in it on
        // `header_bg`.
        for (name, color) in [
            ("text_fg", palette.text_fg),
            ("icon_fg", palette.icon_fg),
            ("invalid_fg", palette.invalid_fg),
            ("address_fg", palette.address_fg),
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
        let line = contrast(palette.branch_fg, palette.asm_pane_bg);
        let lit = contrast(palette.branch_lit_fg, palette.asm_pane_bg);
        assert!(line >= 1.5, "{theme} branch_fg: {line:.2}");
        assert!(lit > line, "{theme} branch_lit_fg: {lit:.2} vs {line:.2}");

        // The rule that starts a basic block runs the whole width of the pane where the
        // gutter's stroke is a few pixels long, so it is held to a floor of its own and
        // required to stay quieter than that stroke rather than merely legible.
        let rule = contrast(palette.block_rule, palette.asm_pane_bg);
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
            ("pair_bg", palette.pair_bg, palette.asm_pane_bg),
            (
                "pair_selected_bg",
                palette.pair_selected_bg,
                palette.asm_pane_bg,
            ),
            // The selection -- the characters a sweep picked out, or whole rows -- over
            // the pane and over a paired row's green, which stays under the characters;
            // and the caret's row, the selection faded, over the pane.
            ("cursor_row_bg", palette.cursor_row_bg, palette.asm_pane_bg),
            (
                "text_select_bg",
                palette.text_select_bg,
                palette.asm_pane_bg,
            ),
            (
                "text_select_bg over a paired row",
                palette.text_select_bg,
                blend(palette.pair_bg, palette.asm_pane_bg),
            ),
            ("pair_edge", palette.pair_edge, palette.asm_pane_bg),
            ("drop_preview_bg", palette.drop_preview_bg, palette.pane_bg),
            // The × on a tab sits on either of two grounds and has to say the same thing
            // over both: the active tab's own pane, and a hovered tab's grey.
            ("close_hover_bg", palette.close_hover_bg, palette.pane_bg),
            (
                "close_hover_bg over a hovered tab",
                palette.close_hover_bg,
                palette.toggle_hover_bg,
            ),
            // The grey a chrome control takes under the pointer: a hovered tab in the
            // bar, and a name in the Assembly pane's bar, both over the header's own grey.
            (
                "toggle_hover_bg",
                palette.toggle_hover_bg,
                palette.header_bg,
            ),
        ] {
            let step = step(wash, ground);
            assert!(step >= 10, "{theme} {name}: {step} levels");
        }

        // A row that is both picked out and the pair has to be told from one that is only
        // the pair: the same green, moved further.
        let pair = step(palette.pair_bg, palette.asm_pane_bg);
        let both = step(palette.pair_selected_bg, palette.asm_pane_bg);
        assert!(both > pair + 20, "{theme} pair {pair} vs both {both}");
        // And the rule along a run of paired rows is deeper into the green than the wash
        // it edges: `step` of an opaque colour is its distance from the pane.
        let edge = step(palette.pair_edge, palette.asm_pane_bg);
        assert!(edge > pair + 10, "{theme} pair {pair} vs edge {edge}");

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

/// An attribute, a type and a call are three colours and not two of somebody else's.
/// The mapping is the whole of the feature, and it is one `syntax()` line per capture, so
/// what is pinned here is that no one of them has drifted back onto the entry it was
/// carved out of -- `attribute` and `type` off `keyword`, `function` and its two children
/// off the plain text.
#[test]
fn attributes_types_and_calls_are_their_own_colours() {
    for (name, palette) in [("light", &Palette::LIGHT), ("dark", &Palette::DARK)] {
        let theme = palette.syntax();
        for (capture, color, taken, other) in [
            ("attribute", theme.attribute, "keyword", theme.keyword),
            ("type", theme.type_, "keyword", theme.keyword),
            ("function", theme.function, "text", theme.text),
            ("function.method", theme.function_method, "text", theme.text),
            ("function.macro", theme.function_macro, "text", theme.text),
        ] {
            assert!(
                color != other,
                "{name}: {capture} is still the {taken} colour"
            );
        }

        // And the three are told from each other, not merely from what they left.
        for (a, first, b, second) in [
            ("attribute", theme.attribute, "type", theme.type_),
            ("attribute", theme.attribute, "function", theme.function),
            ("type", theme.type_, "function", theme.function),
        ] {
            assert!(first != second, "{name}: {a} and {b} are one colour");
        }

        // A call site is one colour whichever of the three shapes it is written in.
        assert_eq!(theme.function, theme.function_method, "{name}: a method");
        assert_eq!(theme.function, theme.function_macro, "{name}: a macro");
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
/// re-render them except that they read the state.
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
            .background(palette().asm_pane_bg)
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
    let code = palette().asm_pane_bg;

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
    assert_eq!(painted_height(&test, palette().asm_pane_bg), 26.0);

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
    assert_eq!(painted_height(&test, palette().asm_pane_bg), 36.0);
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
    assert_eq!(painted_height(&test, palette().asm_pane_bg), 26.0);

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
                .provide_root_context(|| Marked(State::create(Marks::default())))
                .0
        },
        1.,
    );
    test.sync_and_update();

    test.press_cursor((10., 10.));
    test.move_cursor((10., 30.));
    test.sync_and_update();
    assert_eq!(marked.peek().assembly.as_ref().unwrap().rows.rows(), 0..=1);

    // The line that panicked, and the assertion that it no longer does is the test
    // getting this far at all.
    test.release_cursor((10., 30.));
    assert_eq!(marked.peek().assembly.as_ref().unwrap().rows.rows(), 0..=1);

    // And the gesture really is over: a row entered afterwards is the pointer passing
    // over it, which is the panes' hover and not a sweep.
    test.move_cursor((10., 50.));
    test.sync_and_update();
    assert_eq!(marked.peek().assembly.as_ref().unwrap().rows.rows(), 0..=1);
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

/// Every `label()` on screen, by its text, and every span of every paragraph -- a code
/// row's text being one paragraph of spans. `ElementExt` reads no text through the
/// prelude, so this downcasts past it -- `agents/Headless.md` spells the recipe out.
fn labels(test: &TestingRunner) -> Vec<String> {
    use freya::elements::{label::LabelElement, paragraph::ParagraphElement};
    use std::any::Any;

    test.find_many(|node, _element| {
        let element = node.element();
        let element = element.as_ref() as &dyn Any;
        if let Some(label) = element.downcast_ref::<LabelElement>() {
            return Some(vec![label.text.to_string()]);
        }
        element.downcast_ref::<ParagraphElement>().map(|paragraph| {
            paragraph
                .spans
                .iter()
                .map(|span| span.text.to_string())
                .collect()
        })
    })
    .into_iter()
    .flatten()
    .collect()
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

/// Where a label with this exact text was laid out, as a point to press: the middle of it,
/// since hit testing is `is_point_inside` and an edge is nobody's.
fn label_centre(test: &TestingRunner, text: &str) -> Option<(f64, f64)> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find(|node, _element| {
        let drawn = (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .map(|label| label.text.to_string())?;
        if drawn != text {
            return None;
        }
        let area = node.layout().area;
        Some((
            (area.origin.x + area.width() / 2.0) as f64,
            (area.origin.y + area.height() / 2.0) as f64,
        ))
    })
}

/// **A diagnostic's span is a target.** rustc says where an error is and the editor has a
/// cursor that can be put there, so the place under the message is pressed rather than
/// counted to: the press puts the cursor on that line and that column, in the pad's own
/// buffer.
///
/// The build is put in directly rather than answered by the worker, which would reopen the
/// artifact as a binary on its way past; what is under test is the pane.
#[test]
fn pressing_a_span_puts_the_cursor_where_the_compiler_pointed() {
    let (mut test, _states, pad, text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    // Line 3, column 5 of `DEFAULT_SOURCE` is the `x` of `x * 3 + 1`.
    let mut pad = pad;
    pad.write().state_mut().built = Some(Build::Built {
        executable: fixture_artifact(),
        diagnostics: vec![Diagnostic {
            level: Level::Warning,
            message: "unused variable: `x`".to_owned(),
            rendered: "warning: unused variable: `x`\n".to_owned(),
            // `Span` is freya's own name in this file, the text kind: a diagnostic's is
            // the model's, spelt out.
            span: Some(crate::scratchpad::Span {
                file: SOURCE_FILE.to_owned(),
                line: 3,
                column: 5,
            }),
        }],
    });
    for _ in 0..4 {
        test.sync_and_update();
    }

    let place = label_centre(&test, "src/main.rs:3:5").expect("the span is drawn");

    // The cue first: a place that can be gone to wears the relocation link's own wash
    // while the pointer is on it, which is what says it can be pressed at all.
    test.move_cursor(place);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(
        washed(&test),
        1,
        "the span drew no hover, so nothing on screen offers the press"
    );

    test.click_cursor(place);
    for _ in 0..4 {
        test.sync_and_update();
    }

    let shown = pad.peek().shown().clone();
    let buffers = text.peek();
    let editor = buffers.get(&shown);
    // Both counted from zero here, where rustc counts from one.
    assert_eq!(
        (editor.cursor_row(), editor.cursor_col()),
        (2, 4),
        "the span was pressed and the cursor did not move to it"
    );
}

/// How many things on screen are wearing a link's hover wash.
fn washed(test: &TestingRunner) -> usize {
    test.find_many(|_node, element| {
        (element.style().background == Fill::Color(palette().link_hover_bg)).then_some(())
    })
    .len()
}

/// **A span in a file that is not the pad's own is not a target.** cargo names a file in a
/// dependency as readily as it names `src/main.rs`, and this app has nowhere to put a
/// cursor in one — the editor holds the pad's source and nothing else. So the place is
/// still drawn, cut down to the file's own name, and it is drawn as text: no hover to
/// promise a press, and no press. An affordance that did nothing would be the worse answer.
#[test]
fn a_span_in_a_dependency_is_drawn_and_is_not_a_target() {
    let (mut test, _states, pad, text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);

    let mut pad = pad;
    pad.write().state_mut().built = Some(Build::Built {
        executable: fixture_artifact(),
        diagnostics: vec![Diagnostic {
            level: Level::Warning,
            message: "unused import".to_owned(),
            rendered: "warning: unused import\n".to_owned(),
            span: Some(crate::scratchpad::Span {
                file: "/home/reader/.cargo/registry/src/index.crates.io-6f17/rand-0.8.5/src/lib.rs"
                    .to_owned(),
                line: 3,
                column: 5,
            }),
        }],
    });
    for _ in 0..4 {
        test.sync_and_update();
    }

    // Drawn, and by the file's own name: a registry path is most of a line on its own.
    let place = label_centre(&test, "lib.rs:3:5").expect("the span is drawn");

    test.move_cursor(place);
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(
        washed(&test),
        0,
        "a span nothing can be done about offered a press"
    );

    test.click_cursor(place);
    for _ in 0..4 {
        test.sync_and_update();
    }

    let shown = pad.peek().shown().clone();
    let buffers = text.peek();
    let editor = buffers.get(&shown);
    assert_eq!(
        (editor.cursor_row(), editor.cursor_col()),
        (0, 0),
        "a dependency's line was pressed and something moved in the pad's own source"
    );
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

/// The picked-out run is **listing rows**, so one state can serve a listing of many
/// symbols, and the list converts it back to the instructions it holds before asking
/// `Lanes` what they touch. Below a separator the two spaces differ by one, so a press
/// there lights the branch of the instruction pressed only if the conversion is made --
/// taken as an index, it would light some other row's branch, or none. And the pointer
/// alone lights nothing: only a run does.
#[test]
fn picking_out_a_row_below_a_separator_lights_that_rows_own_branch() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let assembly = studied.assembly.clone().expect("sum_to decodes");
    let lanes = studied.lanes.clone();

    // A branching instruction drawn below a separator, whose listing row is no branch's
    // index: the fixture guard for what this test tells apart.
    let edge = assembly
        .edges
        .iter()
        .copied()
        .find(|edge| {
            let row = lanes.row_of(edge.from);
            row != edge.from && assembly.edge_from(row).is_none()
        })
        .expect("sum_to branches from below a block boundary");
    let address = format!("{:016X} ", assembly.instructions[edge.from].address);

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, _) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    let lit = |test: &TestingRunner| -> usize {
        test.find_many(|_node, element| {
            (element.style().background == Fill::Color(palette().branch_lit_fg)).then_some(())
        })
        .len()
    };
    assert_eq!(
        lit(&test),
        0,
        "nothing is lit before anything is picked out"
    );

    let row = label_area(&test, &address).expect("the branching row is drawn");
    let at = (
        (row.origin.x + 5.0) as f64,
        (row.origin.y + row.height() / 2.0) as f64,
    );
    test.move_cursor(at);
    settle(&mut test);
    assert_eq!(lit(&test), 0, "the pointer alone lit a branch");
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);
    assert!(
        lit(&test) > 0,
        "picking out row {} (instruction {}) lit no stroke of its branch",
        lanes.row_of(edge.from),
        edge.from
    );
}

/// Each pane keeps a run of its own: a press in one leaves the other's where it was, and
/// letting go ends whichever drag is under way without touching the rows of either.
#[test]
fn a_press_in_one_pane_leaves_the_others_run_alone() {
    let (mut test, marked) = TestingRunner::new(
        project_harness,
        (100., 100.).into(),
        |runner| {
            runner
                .provide_root_context(|| Marked(State::create(Marks::default())))
                .0
        },
        1.,
    );
    test.sync_and_update();

    mark_press(marked, false, Pane::Source, Some("a.c".into()), 3, None);
    mark_press(marked, false, Pane::Assembly, None, 7, None);
    let marks = marked.peek().clone();
    let source = marks.source.as_ref().expect("the source run was dropped");
    let assembly = marks
        .assembly
        .as_ref()
        .expect("the assembly run was not started");
    assert_eq!(source.rows.rows(), 3..=3);
    assert_eq!(assembly.rows.rows(), 7..=7);
    assert!(source.file.as_deref() == Some("a.c"));
    // Each asks the other pane for the scroll, and neither its own.
    assert!(source.owed == Owed::by(Pane::Assembly));
    assert!(assembly.owed == Owed::by(Pane::Source));

    // A reach in one pane is a reach in that pane alone.
    mark_press(marked, true, Pane::Assembly, None, 9, None);
    let marks = marked.peek().clone();
    assert_eq!(marks.assembly.as_ref().unwrap().rows.rows(), 7..=9);
    assert_eq!(marks.source.as_ref().unwrap().rows.rows(), 3..=3);

    mark_release(marked);
    let marks = marked.peek().clone();
    assert!(!marks.assembly.as_ref().unwrap().rows.dragging);
    assert!(!marks.source.as_ref().unwrap().rows.dragging);
    assert_eq!(marks.source.as_ref().unwrap().rows.rows(), 3..=3);
}

/// A line picked out in the source pane lights, in the listing, every instruction it was
/// compiled from and nothing else -- and the pointer lights nothing: moving over the rows
/// leaves the pair as it was and picks nothing out.
#[test]
fn a_picked_out_line_lights_the_instructions_it_was_compiled_from() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let at = a_line_of(&sum_to);
    let assembly = studied.assembly.clone().expect("sum_to decodes");
    // How many instructions the line produced, which is how many rows should light.
    let compiled = (0..assembly.instructions.len())
        .filter(|&index| studied.position(index).as_ref() == Some(&at))
        .count();
    assert!(compiled > 0, "the line produced no instruction");
    // A row that is not one of them, for the pointer to pass over.
    let other = (0..assembly.instructions.len())
        .find(|&index| studied.position(index).as_ref() != Some(&at))
        .expect("sum_to has an instruction on another line");
    let other = format!("{:016X} ", assembly.instructions[other].address);

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (500., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    let mut marked = marked;
    settle(&mut test);

    let wearing = |test: &TestingRunner, colour: Color| -> usize {
        test.find_many(|_node, element| {
            (element.style().background == Fill::Color(colour)).then_some(())
        })
        .len()
    };
    assert_eq!(
        wearing(&test, palette().pair_bg),
        0,
        "a pair is lit with nothing picked out"
    );

    marked.set(Marks {
        assembly: None,
        source: Some(picked_line(&at, Owed::default())),
    });
    settle(&mut test);
    assert_eq!(
        wearing(&test, palette().pair_bg),
        compiled,
        "the rows lit are not the instructions the line was compiled from"
    );

    // The run of lit rows wears its rule at its ends and not between: a row's top rule
    // exactly where the row above is not lit, its bottom one where the row below is not.
    let lit: Vec<(Area, Vec<Border>)> = test.find_many(|node, element| {
        (element.style().background == Fill::Color(palette().pair_bg))
            .then(|| (node.layout().area, element.style().borders.clone()))
    });
    let lit_at = |y: f32| lit.iter().any(|(area, _)| (area.origin.y - y).abs() < 0.5);
    for (area, borders) in &lit {
        let rule = borders
            .iter()
            .find(|border| border.fill == palette().pair_edge)
            .map(|border| (border.width.top > 0.0, border.width.bottom > 0.0))
            .unwrap_or((false, false));
        let above = lit_at(area.origin.y - area.height());
        let below = lit_at(area.origin.y + area.height());
        assert_eq!(
            rule,
            (!above, !below),
            "the row at {} wears its rule at {rule:?} with lit rows above {above}, below {below}",
            area.origin.y
        );
    }

    // The pointer over a row that is not one of them: nothing changes.
    let row = label_area(&test, &other).expect("the other row is drawn");
    test.move_cursor((
        (row.origin.x + 5.0) as f64,
        (row.origin.y + row.height() / 2.0) as f64,
    ));
    settle(&mut test);
    assert_eq!(
        wearing(&test, palette().pair_bg),
        compiled,
        "the pointer moved the pair"
    );
    assert_eq!(
        wearing(&test, palette().text_select_bg),
        0,
        "the pointer picked a row out"
    );
    assert!(marked.peek().assembly.is_none());
}

/// An instruction picked out in the listing lights, in the source pane, the line it was
/// compiled from; one placed nowhere lights no line. The line info is built by hand over a
/// file of this machine's own, so the pane can open it.
#[test]
fn a_picked_out_instruction_lights_its_line() {
    use analysis::{LineInfo, LineRow};

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let directory =
        std::env::temp_dir().join(format!("assembly-viewer-pair-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("pair.c");
    let text: String = (1..=20).map(|n| format!("int line_{n}(void);\n")).collect();
    std::fs::write(&path, text).expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));

    let mut studied = Studied::new(sum_to.clone());
    let first = studied
        .assembly
        .as_ref()
        .expect("sum_to decodes")
        .instructions[0]
        .address;
    // The first instruction on line 5, and nothing else placed anywhere.
    studied.lines.info = LineInfo::new(
        vec![LineRow {
            range: first..first + 1,
            file: Some(0),
            line: Some(5),
            column: None,
        }],
        vec![(file.clone(), None)],
    )
    .map(Arc::new);
    studied.lines.file = Some(file.clone());
    studied.lines.line = Some(5);
    let placed = studied.lanes.row_of(0);
    let unplaced = studied.lanes.row_of(1);
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let document = Document::Assembly(Selection::Symbol(sum_to.clone()));
    let (mut test, (states, marked)) = TestingRunner::new(
        source_pane_harness,
        (500., 600.).into(),
        |runner| {
            runner.provide_root_context(|| Mounted(State::create(true)));
            let (states, marked, _landing) = listing_states!(runner, shown);
            (states, marked)
        },
        1.,
    );
    let mut marked = marked;
    activate(states.open, states.history, Some(document), Visit::Went);
    settle(&mut test);
    settle(&mut test);

    // Which line numbers sit in a row lit as the pair: the number label inside a rect
    // wearing `pair_bg`.
    let paired = |test: &TestingRunner| -> Vec<u32> {
        let lit: Vec<Area> = test.find_many(|node, element| {
            (element.style().background == Fill::Color(palette().pair_bg))
                .then_some(node.layout().area)
        });
        labels_with_areas(test)
            .into_iter()
            .filter(|(_, area)| lit.iter().any(|row| row.contains_rect(area)))
            .filter_map(|(text, _)| {
                text.strip_suffix('\u{a0}')
                    .and_then(|number| number.parse().ok())
            })
            .collect()
    };
    assert!(
        paired(&test).is_empty(),
        "a line is lit with nothing picked out"
    );

    marked.set(Marks {
        assembly: Some(picked_row(placed, &file, Owed::default())),
        source: None,
    });
    settle(&mut test);
    assert_eq!(
        paired(&test),
        vec![5],
        "the instruction's line is not the one lit"
    );

    marked.set(Marks {
        assembly: Some(picked_row(unplaced, &file, Owed::default())),
        source: None,
    });
    settle(&mut test);
    assert!(
        paired(&test).is_empty(),
        "an instruction placed nowhere lit {:?}",
        paired(&test)
    );
}

/// Every label on screen with the area it was laid out in.
fn labels_with_areas(test: &TestingRunner) -> Vec<(String, Area)> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .map(|label| (label.text.to_string(), node.layout().area))
    })
}

/// A source-driven tab comes back with the line it is driven from picked out, so the
/// listing of that line's instructions says which line and why -- and with no scroll
/// owed, the kept positions being what puts each side back. Another document arriving
/// drops it.
#[test]
fn a_source_driven_tab_comes_back_with_its_line_picked_out() {
    let symbols = fixture_symbols();
    let file: Arc<str> = "driven.c".into();
    let tab = Document::Source(file.clone());

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut driven = states.driven;
    driven.write().remember(tab.clone(), 7);
    activate(states.open, states.history, Some(tab.clone()), Visit::Went);
    settle(&mut test);

    let expected = LinePos {
        file: file.clone(),
        line: 7,
    };
    assert!(
        source_line(location.marked) == Some(expected.clone()),
        "the driven line was not picked out"
    );
    let picked = location
        .marked
        .peek()
        .source
        .clone()
        .expect("checked above");
    assert!(
        picked.owed == Owed::default(),
        "a scroll was owed to the driven line"
    );

    activate(
        states.open,
        states.history,
        Some(Document::Assembly(Selection::Symbol(symbols[0].clone()))),
        Visit::Went,
    );
    settle(&mut test);
    assert!(
        location.marked.peek().source.is_none(),
        "the run outlived its tab"
    );

    activate(states.open, states.history, Some(tab), Visit::Went);
    settle(&mut test);
    assert!(source_line(location.marked) == Some(expected));
}

/// The address a copied line spells is the listing's, which is the instruction's own plus
/// what the listing adds: nothing for a symbol read alone, and the section's place in the
/// object's layout in a listing of all its code.
#[test]
fn a_copied_line_spells_the_address_the_listing_draws() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let assembly = sum_to
        .data
        .assembly(&sum_to.object)
        .expect("sum_to decodes");
    let first = &assembly.instructions[0];

    assert!(asm_line(first, 0).starts_with("0000000000000030 "));
    assert!(asm_line(first, 0x1000).starts_with("0000000000001030 "));
}

/// Pressing an object in the Objects list opens all of its code as one listing -- a
/// document of its own kind, named after the object, visited like any other.
#[test]
fn pressing_an_object_row_opens_its_code() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let (mut test, mut states) = TestingRunner::new(
        objects_harness,
        (300., 300.).into(),
        |runner| project_states!(runner),
        1.,
    );
    states.objects.write().push(object.clone());
    settle(&mut test);

    let row = label_area(&test, "line_fixture.o").expect("the object has a row");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    let document = Document::Code(object.clone());
    assert!(states.open.active() == Some(document.clone()));
    assert!(states
        .history
        .peek()
        .recent()
        .any(|(_, entry)| *entry == document));
    assert!(
        states.open.active() != Some(Document::Assembly(Selection::Object(object))),
        "the object tab is not what opened"
    );
    assert_eq!(entry_text(&document), "line_fixture.o");
}

use crate::section::{Row, Rows};

/// Nothing at all: for a test that drives the states and mounts no pane.
fn bare_harness() -> impl IntoElement {
    rect().expanded()
}

/// The Assembly pane over an object's code, the reading seeded by the test: the skeleton
/// and whatever stretches it says are decoded, with no worker between the two.
fn code_harness() -> impl IntoElement {
    let reading = use_consume::<Sections>().0;
    let object = reading.read().object.clone();
    match object {
        Some(object) => rect().expanded().child(AssemblyPane {
            document: Document::Code(object),
        }),
        None => rect().expanded(),
    }
}

/// The contexts the section view reads, beside the project's: the listing's own states
/// and a reading of `object` with `held` decoded.
macro_rules! code_states {
    ($runner:expr, $reading:expr) => {{
        let states = project_states!($runner);
        let marked = $runner
            .provide_root_context(|| Marked(State::create(Marks::default())))
            .0;
        $runner.provide_root_context(|| Shift(State::create(false)));
        $runner.provide_root_context(|| Locations(State::create(Located::default())));
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        $runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
        let reading = $runner
            .provide_root_context(|| Sections(State::create($reading)))
            .0;
        let window = $runner
            .provide_root_context(|| Window(State::create(None)))
            .0;
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        let ctrl = $runner
            .provide_root_context(|| Ctrl(State::create(false)))
            .0;
        (states, marked, reading, window, landing, ctrl)
    }};
}

/// A reading of `object`'s code with its skeleton and the stretches in `held` decoded the
/// way the worker decodes them.
fn reading_of(object: &Arc<Object>, held: &[usize]) -> Reading {
    let code = Arc::new(CodeListing::new(object));
    let mut reading = Reading::of(Some(object.clone()));
    let decoded: Vec<(usize, Stretched)> = held
        .iter()
        .filter_map(|&flat| {
            let Answer::Code { mut decoded, .. } = answer(Question::Code(CodeAsk {
                object: object.clone(),
                code: Some(code.clone()),
                window: vec![flat],
            })) else {
                return None;
            };
            decoded.pop()
        })
        .collect();
    let ask = CodeAsk {
        object: object.clone(),
        code: Some(code.clone()),
        window: held.to_vec(),
    };
    assert!(reading.take(&ask, code, decoded));
    reading
}

/// The rows of `reading` as the view counts them.
fn rows_of(reading: &Reading) -> Arc<Rows> {
    let code = reading.code.clone().expect("the reading has a skeleton");
    Arc::new(Rows::new(code, |flat| reading.body(flat)))
}

/// The address labels drawn, top to bottom.
fn address_labels(test: &TestingRunner) -> Vec<String> {
    let mut drawn: Vec<(f32, String)> = test.find_many(|node, element| {
        use freya::elements::label::LabelElement;
        use std::any::Any;
        let _ = element;
        let element = node.element();
        let label = (element.as_ref() as &dyn Any).downcast_ref::<LabelElement>()?;
        let text = label.text.to_string();
        (text.len() == 17 && text.ends_with(' ') && u64::from_str_radix(text.trim(), 16).is_ok())
            .then(|| (node.layout().area.origin.y, text))
    });
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
    drawn.into_iter().map(|(_, text)| text).collect()
}

/// Before a byte is decoded, a code tab draws its section's header, a label for every
/// symbol at the address the layout places it, and empty rows between them -- the whole
/// listing's length, from the first frame.
#[test]
fn a_code_tab_draws_its_labels_and_empty_rows_before_a_byte_is_decoded() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let rows = rows_of(&reading);
    let (mut test, _) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    settle(&mut test);

    let drawn = labels(&test);
    for text in ["section .text", "<add>:", "<twice>:", "<sum_to>:"] {
        assert!(
            drawn.contains(&text.to_string()),
            "{text} is not drawn: {drawn:?}"
        );
    }
    assert_eq!(
        address_labels(&test),
        [
            "0000000000000000 ",
            "0000000000000000 ",
            "0000000000000014 ",
            "0000000000000030 "
        ],
        "the header and the three labels, at their addresses, and nothing between them"
    );
    // The empty rows are there to be scrolled over: the listing is as long as the
    // estimate says, which is far more than its four rows of text.
    assert!(rows.len() > 4);
    let label_rows: Vec<usize> = (0..rows.len())
        .filter(|&row| matches!(rows.row(row), Some(Row::Label { .. })))
        .collect();
    let top = label_area(&test, "section .text").expect("the header is drawn");
    let bottom = label_area(&test, "<sum_to>:").expect("sum_to is labelled");
    assert_eq!(
        bottom.origin.y - top.origin.y,
        label_rows[2] as f32 * code_row_height(),
        "the last label sits below the empty rows the others were guessed to take"
    );
}

/// When a stretch above the viewport decodes, its guess is replaced by its rows and the
/// row under the reader stays where it is: the view keeps its place by address.
#[test]
fn a_decoded_stretch_fills_its_rows_in_and_the_row_under_the_reader_stays_put() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let rows = rows_of(&reading);
    let (mut test, (states, _marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut sections = sections;
    let document = Document::Code(object.clone());
    // Open, as a tab is in the app: a place is written down only for an open tab.
    activate(
        states.open,
        states.history,
        Some(document.clone()),
        Visit::Went,
    );
    settle(&mut test);

    // Scroll so that `sum_to`'s label is the row at the top.
    let label = (0..rows.len())
        .find(|&row| {
            rows.address_of(row) == Some(0x30) && matches!(rows.row(row), Some(Row::Label { .. }))
        })
        .expect("sum_to has a label row");
    test.scroll(
        (300., 150.),
        (0., -(label as f64) * code_row_height() as f64),
    );
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000030 ");
    assert_eq!(
        states.code_at.peek().at(&document),
        Some(Spot {
            address: 0x30,
            rows: 0
        }),
        "the place is written down as the reader scrolls"
    );

    let before = label_area(&test, "<sum_to>:").expect("sum_to is labelled");

    // `add` decodes, above the viewport: its guess becomes its instruction rows, and every
    // row below it moves in the listing -- but not on screen.
    let mut decoded = reading_of(&object, &[0]);
    let grown = rows_of(&decoded);
    assert_ne!(grown.len(), rows.len(), "the guess was not the truth");
    // Both readings were built from nothing and count their answers alike; the view
    // tells them apart by the generation, which in the app only ever goes up.
    decoded.generation = sections.peek().generation + 1;
    sections.set(decoded);
    settle(&mut test);
    settle(&mut test);
    let after = label_area(&test, "<sum_to>:").expect("sum_to is still labelled");
    assert_eq!(
        after.origin.y, before.origin.y,
        "the row under the reader moved"
    );
    assert_eq!(address_labels(&test)[0], "0000000000000030 ");
}

/// A code tab comes back to the address it was left at: the place written down for it
/// is what its first run scrolls to.
#[test]
fn a_code_tab_comes_back_to_the_address_it_was_left_at() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (mut states, ..)) = TestingRunner::new(
        code_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let document = Document::Code(object.clone());
    activate(
        states.open,
        states.history,
        Some(document.clone()),
        Visit::Went,
    );
    states.code_at.write().remember(
        document.clone(),
        Spot {
            address: 0x14,
            rows: 0,
        },
    );
    settle(&mut test);
    settle(&mut test);

    assert_eq!(address_labels(&test)[0], "0000000000000014 ");
    assert!(labels(&test).contains(&"<twice>:".to_string()));
}

/// What the view asks for is the stretches within a buffer of screens of the viewport
/// that are not held, nearest the reader first -- and at the top of a listing that fits
/// in the buffer, that is every stretch from the first.
#[test]
fn scrolling_asks_for_a_buffer_of_screens_nearest_the_reader_first() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let rows = rows_of(&reading);
    let (mut test, (_states, _marked, _sections, window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    settle(&mut test);
    settle(&mut test);

    let asked = window.peek().clone().expect("the view asked for a window");
    assert!(Arc::ptr_eq(&asked.object, &object));
    assert!(asked.code.is_some(), "the skeleton travels with the ask");
    assert_eq!(asked.window, [0, 1, 2]);

    // Scrolled to the bottom, the last stretch is nearest and the buffer still reaches
    // the first.
    test.scroll(
        (300., 150.),
        (0., -(rows.len() as f64) * code_row_height() as f64),
    );
    settle(&mut test);
    settle(&mut test);
    let asked = window.peek().clone().expect("the view asked again");
    assert_eq!(asked.window, [2, 1, 0]);
}

/// Closing a code tab forgets the address it was left at, with the tab -- a place kept
/// for a closed tab would hold its object's bytes.
#[test]
fn closing_a_code_tab_forgets_its_address() {
    let (_path, objects) = fixture_objects(1);
    let document = Document::Code(objects[0].clone());
    let (mut test, mut states) = TestingRunner::new(
        bare_harness,
        (100., 100.).into(),
        |runner| project_states!(runner),
        1.,
    );
    activate(
        states.open,
        states.history,
        Some(document.clone()),
        Visit::Went,
    );
    states.code_at.write().remember(
        document.clone(),
        Spot {
            address: 0x30,
            rows: 2,
        },
    );
    test.sync_and_update();
    assert!(states.code_at.peek().at(&document).is_some());

    close_tab(
        states.open,
        states.history,
        states.asm_at,
        states.src_at,
        states.code_at,
        states.driven,
        &document,
    );
    test.sync_and_update();
    assert!(states.code_at.peek().at(&document).is_none());
    assert!(states.open.active().is_none());
}

/// The Source pane beside an object's code, the reading seeded by the test.
fn code_source_harness() -> impl IntoElement {
    let reading = use_consume::<Sections>().0;
    let object = reading.read().object.clone();
    match object {
        Some(object) => rect().expanded().child(SourcePane {
            document: Document::Code(object),
        }),
        None => rect().expanded(),
    }
}

/// Beside an object's code the Source pane draws the file of whatever the reader picked
/// out in it, and nothing until they have: the listing draws no symbol of its own to
/// name one.
#[test]
fn a_run_in_the_section_view_opens_its_file_beside_it() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (_states, marked, ..)) = TestingRunner::new(
        code_source_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut marked = marked;
    settle(&mut test);
    assert!(
        labels(&test).contains(&"Click an instruction".to_string()),
        "{:?}",
        labels(&test)
    );

    // The run is of a file of the fixture's source, which this machine does not have:
    // the pane names the file it went looking for, which is the whole of what is asked
    // here.
    marked.set(Marks {
        assembly: Some(picked_row(
            4,
            "/fixture/line_fixture.c",
            Owed::by(Pane::Source),
        )),
        source: None,
    });
    settle(&mut test);
    let drawn = labels(&test);
    assert!(
        drawn
            .iter()
            .any(|text| text.contains("/fixture/line_fixture.c")),
        "the run's file is not what the pane went to: {drawn:?}"
    );
}

/// The run picked out of an object's code is listing rows, and the rows are counted
/// afresh with every answer that lands: the run goes when they do, and stays through an
/// ask, which writes the same state and changes no row.
#[test]
fn a_chunk_landing_drops_the_run_picked_out_over_it() {
    fn marks_harness() -> impl IntoElement {
        let active = use_consume::<Active>().0;
        let driven = use_consume::<Drives>().0;
        let analysis = use_consume::<Analysis>().0;
        let reading = use_consume::<Sections>().0;
        let marked = use_consume::<Marked>().0;
        use_clear_marks(
            active,
            crate::ui::analyzed::Asked { active, driven },
            analysis,
            reading,
            marked,
        );
        rect().expanded()
    }

    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (_states, marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        marks_harness,
        (100., 100.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut sections = sections;
    settle(&mut test);

    mark_row(marked, None, 3);
    settle(&mut test);
    assert!(
        marked.peek().assembly.is_some(),
        "the run was not picked out"
    );

    // An ask: the same state written, no row changed.
    let mut asked = sections.peek().clone();
    asked.pending = Some(CodeAsk {
        object: object.clone(),
        code: asked.code.clone(),
        window: vec![1],
    });
    sections.set(asked);
    settle(&mut test);
    assert!(marked.peek().assembly.is_some(), "an ask dropped the run");

    // An answer: the rows are counted afresh, and the run with them.
    let mut landed = sections.peek().clone();
    landed.generation += 1;
    sections.set(landed);
    settle(&mut test);
    assert!(
        marked.peek().assembly.is_none(),
        "the rows changed under the run and it stayed"
    );
}

/// A run copied out of an object's code spells each kind of row as it is drawn: the
/// section's header, a symbol's label after its address, an instruction as its own tab
/// copies it, and an empty row as the blank line it is.
#[test]
fn a_copied_run_of_the_section_view_spells_each_kind_of_row() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[0]);
    let rows = rows_of(&reading);

    let lines: Vec<String> = (0..rows.len())
        .map(|row| row_line(&rows, &reading, row))
        .collect();
    assert_eq!(lines[0], "section .text");
    assert_eq!(lines[1], "0000000000000000 <add>:");
    let add = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "add")
        .expect("the fixture holds add");
    let own = add.data.assembly(&add.object).expect("add decodes");
    assert_eq!(lines[2], asm_line(&own.instructions[0], 0));
    // `twice` is not decoded: its label, then blank lines.
    let twice = lines
        .iter()
        .position(|line| line == "0000000000000014 <twice>:")
        .expect("twice is labelled");
    assert_eq!(lines[twice + 1], "");
}

/// A click on a source row beside an object's code owes the listing a scroll to the
/// instruction compiled from that line, paid out of whichever held stretch has one -- and
/// left owed, for the answer that decodes the stretch to pay, while none does.
#[test]
fn a_source_click_beside_the_section_view_reveals_its_instruction() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let sum_to = Symbol {
        object: object.clone(),
        data: object
            .symbols_sorted
            .iter()
            .find(|data| data.name == "sum_to")
            .expect("the fixture holds sum_to")
            .clone(),
    };
    let at = a_line_of(&sum_to);
    let reading = reading_of(&object, &[]);
    let (mut test, (_states, marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let (mut marked, mut sections) = (marked, sections);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000000 ");

    // Picked out on the source side while `sum_to` is still empty rows: owed, and
    // unpaid.
    marked.set(Marks {
        assembly: None,
        source: Some(picked_line(&at, Owed::by(Pane::Assembly))),
    });
    settle(&mut test);
    assert!(
        owes_pair(marked, Pane::Assembly),
        "the reveal was paid with nothing to pay it"
    );

    // `sum_to` decodes: the row is there now, and the listing scrolls to it.
    let mut decoded = reading_of(&object, &[2]);
    decoded.generation = sections.peek().generation + 1;
    sections.set(decoded);
    settle(&mut test);
    settle(&mut test);
    assert!(
        owed_reveal(marked, Pane::Assembly).is_none(),
        "the reveal is still owed"
    );
    let drawn = address_labels(&test);
    assert_ne!(
        drawn[0], "0000000000000000 ",
        "the listing did not scroll: {drawn:?}"
    );
    assert!(
        drawn
            .iter()
            .all(|text| u64::from_str_radix(text.trim(), 16).unwrap() >= 0x30),
        "sum_to's rows are not what is on screen: {drawn:?}"
    );
}

/// Ctrl-pressing a symbol's label in an object's code opens that symbol's own tab: the
/// door back from a function read among its neighbours to reading it alone, a visit like
/// any opening from a list. A plain press is a plain press.
#[test]
fn pressing_a_label_opens_the_symbols_own_tab() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (states, _marked, _sections, _window, _landing, ctrl)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut ctrl = ctrl;
    let code = Document::Code(object.clone());
    activate(states.open, states.history, Some(code.clone()), Visit::Went);
    settle(&mut test);

    // A plain press is a plain press: the tab stays.
    let label = centre_of(&test, "<twice>:");
    press_at(&mut test, label);
    settle(&mut test);
    assert!(states.open.active() == Some(code.clone()));

    // With Ctrl held it is the door.
    ctrl.set(true);
    settle(&mut test);
    press_at(&mut test, label);
    settle(&mut test);
    let twice = Symbol {
        object: object.clone(),
        data: object
            .symbols_sorted
            .iter()
            .find(|data| data.name == "twice")
            .expect("the fixture holds twice")
            .clone(),
    };
    let symbol = Document::Assembly(Selection::Symbol(twice));
    assert!(states.open.active() == Some(symbol.clone()));
    assert!(states
        .history
        .peek()
        .recent()
        .any(|(_, entry)| *entry == symbol));
}

/// The Assembly pane over a symbol's listing, with a menu viewer above it so a row's
/// menu can open.
fn menu_listing_harness() -> impl IntoElement {
    let analysis = use_consume::<Analysis>().0;
    let document = analysis
        .read()
        .shown
        .as_ref()
        .map(|shown| asked_of(&shown.ask))
        .unwrap_or_else(|| Document::Source(Arc::from("")));

    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(AssemblyPane { document })
}

/// An instruction's menu offers to show it among its neighbours: the object's code tab
/// opens with its place set on that instruction's address -- written before the tab is
/// opened, the order a restore uses -- and the line it was compiled from left as a
/// landing for both panes.
#[test]
fn show_in_object_lands_the_code_tab_on_the_instruction() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let object = sum_to.object.clone();
    let studied = Studied::new(sum_to.clone());
    let first = studied.assembly.as_ref().unwrap().instructions[0].address;
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, (states, _marked, landing)) = TestingRunner::new(
        menu_listing_harness,
        (600., 400.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    let symbol = Document::Assembly(Selection::Symbol(sum_to.clone()));
    activate(states.open, states.history, Some(symbol), Visit::Went);
    settle(&mut test);

    let row = centre_of(&test, &format!("{first:016X} "));
    right_click(&mut test, row);
    let entry = "Show in unified view".to_string();
    assert!(labels(&test).contains(&entry), "{:?}", labels(&test));
    let item = centre_of(&test, &entry);
    press_at(&mut test, item);
    settle(&mut test);

    let code = Document::Code(object.clone());
    assert!(states.open.active() == Some(code.clone()));
    assert_eq!(
        states.code_at.peek().at(&code),
        Some(Spot {
            address: first,
            rows: 0
        })
    );
    let landed = landing.peek().clone().expect("the line is left to land");
    assert!(landed.tab == code);
    assert!(landed.at == a_line_of(&sum_to));
}

/// Shown among its neighbours while the object's code is already on top, the view
/// scrolls to the instruction and nothing else changes: the place is written and read
/// back by the pane, and the document stays.
#[test]
fn show_in_object_while_the_code_is_on_top_scrolls_without_a_switch() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (states, marked, _sections, _window, landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 300.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let code = Document::Code(object.clone());
    activate(states.open, states.history, Some(code.clone()), Visit::Went);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000000 ");
    let visits = states.history.peek().recent().count();

    show_in_code(
        states.open,
        states.history,
        marked,
        landing,
        states.code_at,
        object.clone(),
        0x30,
        None,
    );
    settle(&mut test);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000030 ");
    assert!(states.open.active() == Some(code));
    assert_eq!(states.history.peek().recent().count(), visits);
}

/// The rows of an object's code that are bytes and not instructions read as data and not
/// as assembly: a data directive in front of the values -- `dq` for a row that divides
/// into quadwords, down to `db` -- and the bytes as characters after them, which no
/// instruction row has; and a copied line says the same.
#[test]
fn a_gap_row_is_marked_as_data() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    // A stretch decoded as sixteen bytes of gap and no code, put into the reading by hand:
    // the fixture's own functions leave no padding between them.
    let mut reading = reading_of(&object, &[]);
    let code = reading.code.clone().expect("the skeleton");
    let ask = CodeAsk {
        object: object.clone(),
        code: Some(code.clone()),
        window: vec![0],
    };
    let gap = analysis::Gap {
        range: 0..16,
        kind: analysis::GapKind::Bytes,
    };
    assert!(reading.take(
        &ask,
        code,
        vec![(
            0,
            Stretched {
                code: None,
                gap: Some(gap),
            }
        )]
    ));
    let rows = rows_of(&reading);
    let gap_row = (0..rows.len())
        .find(|&row| matches!(rows.row(row), Some(Row::Gap { .. })))
        .expect("the stretch has a gap row");
    // Sixteen bytes divide into quadwords, little-endian; a row of them is `dq`.
    let copied = row_line(&rows, &reading, gap_row);
    let value = |bytes: &[u8]| {
        bytes
            .iter()
            .rev()
            .fold(0u64, |value, &byte| (value << 8) | u64::from(byte))
    };
    let section = object
        .sections
        .iter()
        .find(|section| section.name == ".text")
        .expect(".text is there");
    let ascii: String = section.data[0..16]
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    assert_eq!(
        copied,
        format!(
            "0000000000000000 dq {:016X}, {:016X}{} |{ascii}|",
            value(&section.data[0..8]),
            value(&section.data[8..16]),
            " ".repeat(47 - 34),
        )
    );

    let (mut test, _) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    settle(&mut test);
    let drawn = labels(&test);
    assert!(drawn.contains(&"dq\u{a0}".to_string()), "{drawn:?}");
    assert!(
        drawn
            .iter()
            .any(|text| text.ends_with('|') && text.contains('|')),
        "no row draws its bytes as characters: {drawn:?}"
    );
}

/// The Assembly pane over an object's code with a menu viewer above it, so a row's menu
/// can open.
fn menu_code_harness() -> impl IntoElement {
    let reading = use_consume::<Sections>().0;
    let object = reading.read().object.clone();
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .maybe_child(object.map(|object| {
            AssemblyPane {
                document: Document::Code(object),
            }
            .into_element()
        }))
}

/// An instruction's menu in the unified view offers the symbol it belongs to as a tab of
/// its own, landed on the row's line: the door back from reading a function among its
/// neighbours to reading it alone, and not only through its label.
#[test]
fn open_as_symbol_from_the_unified_view_opens_the_symbols_tab() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let twice = Symbol {
        object: object.clone(),
        data: object
            .symbols_sorted
            .iter()
            .find(|data| data.name == "twice")
            .expect("the fixture holds twice")
            .clone(),
    };
    let reading = reading_of(&object, &[1]);
    let (mut test, (states, _marked, _sections, _window, landing, _ctrl)) = TestingRunner::new(
        menu_code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let code = Document::Code(object.clone());
    activate(states.open, states.history, Some(code), Visit::Went);
    settle(&mut test);

    // The second instruction: the first shares its address text with the label above
    // it, which is not a row with a menu.
    let second = format!("{:016X} ", twice.data.address + 1);
    let row = centre_of(&test, &second);
    right_click(&mut test, row);
    let drawn = labels(&test);
    assert!(drawn.contains(&"Open as symbol".to_string()), "{drawn:?}");
    assert!(
        !drawn.contains(&"Show in unified view".to_string()),
        "the unified view offered itself: {drawn:?}"
    );
    let item = centre_of(&test, "Open as symbol");
    press_at(&mut test, item);
    settle(&mut test);

    let symbol = Document::Assembly(Selection::Symbol(twice.clone()));
    assert!(states.open.active() == Some(symbol.clone()));
    let landed = landing
        .peek()
        .clone()
        .expect("the row's line is left to land");
    assert!(landed.tab == symbol);
    assert!(landed.at.file == a_line_of(&twice).file);
}

/// The Assembly pane over an object's code as `app()` mounts it: the pane first, and the
/// reading following the active document a beat later through `use_reading_of`.
fn app_like_code_harness() -> impl IntoElement {
    let object = use_consume::<PaneObject>().0;
    let active = use_consume::<Active>().0;
    let objects = use_consume::<Objects>().0;
    let reading = use_consume::<Sections>().0;
    let window = use_consume::<Window>().0;
    use_reading_of(active, objects, reading, window);
    rect().expanded().child(AssemblyPane {
        document: Document::Code(object),
    })
}

/// The object [`app_like_code_harness`] mounts the pane over.
#[derive(Clone)]
struct PaneObject(Arc<Object>);

/// The unified view's pane mounts a beat before its reading is its own -- the active
/// document is a memo, and the reading follows the memo -- and it has to ask for its
/// skeleton once the reading catches up, with nothing else moving in between. It once
/// asked only when the viewport did, and a tab opened this way stayed empty until the
/// pane was resized.
#[test]
fn a_unified_view_asks_for_its_skeleton_once_the_reading_is_its_own() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let (mut test, (states, _marked, sections, window, _landing, _ctrl)) = TestingRunner::new(
        app_like_code_harness,
        (600., 300.).into(),
        {
            let object = object.clone();
            move |runner| {
                runner.provide_root_context(|| PaneObject(object.clone()));
                code_states!(runner, Reading::default())
            }
        },
        1.,
    );
    let mut open = states.objects;
    open.write().push(object.clone());
    settle(&mut test);
    // Mounted over a reading of nothing: nothing to ask for yet.
    assert!(window.peek().is_none());
    assert!(sections.peek().object.is_none());

    // The tab is opened; the memo catches up, the reading becomes the object's, and the
    // pane -- untouched otherwise -- asks for the skeleton.
    activate(
        states.open,
        states.history,
        Some(Document::Code(object.clone())),
        Visit::Went,
    );
    settle(&mut test);
    settle(&mut test);
    assert!(
        sections.peek().is_about(&object),
        "the reading did not follow the tab"
    );
    let asked = window.peek().clone().expect("the skeleton is asked for");
    assert!(Arc::ptr_eq(&asked.object, &object));
    assert!(asked.code.is_none());
}

/// The rows are rebuilt a pass after an answer lands, and for that pass the pane can read
/// a reading newer than the rows on screen. A stretch the answer let go of is still drawn
/// from the old rows, and has to be drawn from what it was counted from: against the new
/// reading it found no bytes, every one of its rows fell back to one key, and freya's diff
/// panicked on the duplicate.
#[test]
fn a_stretch_let_go_under_the_rows_on_screen_still_draws_as_it_was() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    // Stretch 0 as a gap of sixteen bytes, by hand, so the listing has gap rows.
    let mut reading = reading_of(&object, &[]);
    let code = reading.code.clone().expect("the skeleton");
    let ask = CodeAsk {
        object: object.clone(),
        code: Some(code.clone()),
        window: vec![0],
    };
    assert!(reading.take(
        &ask,
        code.clone(),
        vec![(
            0,
            Stretched {
                code: None,
                gap: Some(analysis::Gap {
                    range: 0..16,
                    kind: analysis::GapKind::Bytes,
                }),
            }
        )]
    ));
    let (mut test, (_states, _marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut sections = sections;
    settle(&mut test);
    assert!(labels(&test).contains(&"dq\u{a0}".to_string()));

    // The answer that lets stretch 0 go: the same skeleton, nothing held, a new generation.
    let mut let_go = Reading::of(Some(object.clone()));
    let_go.code = Some(code);
    let_go.generation = sections.peek().generation + 1;
    sections.set(let_go);
    // One pass: the render before the effect that rebuilds the rows. This is where the
    // gap rows were drawn against a reading that no longer held them.
    test.sync_and_update();
    assert!(
        labels(&test).contains(&"dq\u{a0}".to_string()),
        "drawn from the old rows still"
    );
    settle(&mut test);
    assert!(
        !labels(&test).contains(&"dq\u{a0}".to_string()),
        "the rows caught up with the reading"
    );
}

/// A Caps Lock the desktop has made into Ctrl names itself Caps Lock, and a key event's
/// mask is the state *before* the key: its press comes over a mask without Ctrl, its
/// release over a mask with Ctrl still in it. The keyboard is learnt from that release --
/// after it, the key's press counts and its release clears -- and a Caps Lock let go under
/// a real Ctrl teaches nothing.
#[test]
fn a_caps_lock_that_acts_as_ctrl_is_learnt_from_its_release() {
    let (mut test, (keys, ctrl)) = TestingRunner::new(
        bare_harness,
        (100., 100.).into(),
        |runner| {
            let shift = runner
                .provide_root_context(|| Shift(State::create(false)))
                .0;
            let ctrl = runner.provide_root_context(|| Ctrl(State::create(false))).0;
            // Two states of the root's own, made where a state can be: in a context.
            #[derive(Clone, Copy)]
            struct CapsIsCtrl(State<bool>);
            #[derive(Clone, Copy)]
            struct ControlHeld(State<bool>);
            let caps = runner
                .provide_root_context(|| CapsIsCtrl(State::create(false)))
                .0;
            let held = runner
                .provide_root_context(|| ControlHeld(State::create(false)))
                .0;
            (ModifierKeys::new(shift, ctrl, caps, held), ctrl)
        },
        1.,
    );
    let caps = Key::Named(NamedKey::CapsLock);
    let control = Key::Named(NamedKey::Control);
    test.sync_and_update();

    // A real Ctrl: known by its name both ways.
    keys.down(&control, Modifiers::empty());
    assert!(*ctrl.peek());
    keys.up(&control, Modifiers::CONTROL);
    assert!(!*ctrl.peek());

    // A Caps Lock let go under a real Ctrl is a plain Caps Lock: nothing learnt, and Ctrl
    // still down.
    keys.down(&control, Modifiers::empty());
    keys.down(&caps, Modifiers::CONTROL);
    keys.up(&caps, Modifiers::CONTROL);
    assert!(*ctrl.peek());
    keys.up(&control, Modifiers::CONTROL);
    assert!(!*ctrl.peek());
    keys.down(&caps, Modifiers::empty());
    assert!(!*ctrl.peek(), "an unlearnt Caps Lock is not Ctrl");

    // The first release of a Caps Lock that *is* Ctrl: the mask says Ctrl and no Control
    // key is down. Before this it was left stuck on; now it is learnt and cleared.
    keys.up(&caps, Modifiers::CONTROL);
    assert!(!*ctrl.peek(), "the release left Ctrl on");
    keys.down(&caps, Modifiers::empty());
    assert!(*ctrl.peek(), "the learnt key's press does not count");
    keys.up(&caps, Modifiers::CONTROL);
    assert!(!*ctrl.peek());
}

/// The Bookmarks panel and nothing else, with the context-menu viewer a right-click on a
/// row needs, over the project's states.
fn bookmarks_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(BookmarksTab)
}

/// A bookmark of `symbol`, the way a gesture on a live document would make one.
fn bookmark_of(document: &Document) -> Bookmark {
    Bookmark {
        name: entry_name(document),
        document: SavedDocument::from_document(document),
    }
}

/// Pressing a live bookmark is a navigation: the place becomes the active document and
/// the history records the visit, exactly as a press in the Symbols list does.
#[test]
fn a_bookmark_row_opens_its_place() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let document = Document::Assembly(Selection::Symbol(wanted.clone()));

    let (mut test, states) = TestingRunner::new(
        bookmarks_harness,
        (300., 300.).into(),
        project_states!(),
        1.,
    );
    let (mut objects, mut bookmarks) = (states.objects, states.bookmarks);
    objects.set(vec![wanted.object.clone()]);
    bookmarks.set(Bookmarks::from_entries(vec![bookmark_of(&document)]));
    settle(&mut test);
    assert!(states.open.active().is_none());

    let press = centre_of(&test, "sum_to");
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);

    assert!(states.open.active() == Some(document.clone()));
    assert!(states
        .history
        .peek()
        .recent()
        .any(|(_, entry)| *entry == document));
}

/// A bookmark outlives its binary: with the object gone the row is still drawn, under the
/// name it was made with, and a press on it goes nowhere. Opening the binary again brings
/// it back to life without the list having changed.
#[test]
fn a_bookmark_is_kept_when_its_binary_closes() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();
    let document = Document::Assembly(Selection::Symbol(wanted.clone()));

    let (mut test, states) = TestingRunner::new(
        bookmarks_harness,
        (300., 300.).into(),
        project_states!(),
        1.,
    );
    let (mut objects, mut bookmarks) = (states.objects, states.bookmarks);
    bookmarks.set(Bookmarks::from_entries(vec![bookmark_of(&document)]));
    settle(&mut test);

    // Dead: nothing loaded. Drawn, inert.
    let press = centre_of(&test, "sum_to");
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);
    assert!(states.open.active().is_none());
    assert_eq!(bookmarks.peek().entries().len(), 1);

    // Alive again once the object is there, with the list untouched.
    objects.set(vec![wanted.object.clone()]);
    settle(&mut test);
    let press = centre_of(&test, "sum_to");
    test.move_cursor(press);
    test.press_cursor(press);
    test.release_cursor(press);
    settle(&mut test);
    assert!(states.open.active() == Some(document));
}

/// A dead bookmark matches no document, so its row's own menu is how it goes: right-click,
/// "Remove bookmark", and the list is shorter by that one.
#[test]
fn a_bookmark_row_is_removed_from_its_menu() {
    let symbols = fixture_symbols();
    let document = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let file = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) = TestingRunner::new(
        bookmarks_harness,
        (300., 300.).into(),
        project_states!(),
        1.,
    );
    let mut bookmarks = states.bookmarks;
    bookmarks.set(Bookmarks::from_entries(vec![
        bookmark_of(&document),
        bookmark_of(&file),
    ]));
    settle(&mut test);

    let row = centre_of(&test, "main.rs");
    right_click(&mut test, row);
    let entry = centre_of(&test, "Remove bookmark");
    test.move_cursor(entry);
    test.press_cursor(entry);
    test.release_cursor(entry);
    settle(&mut test);

    let left = bookmarks.peek().entries().to_vec();
    let left: Vec<&str> = left.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(left, [symbols[0].data.display()]);
}

/// The Symbols list with the context-menu viewer a right-click on a row needs, over the
/// project's states and the `Symbols` memo `app()` derives from the objects.
fn symbols_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(SymbolsTab)
}

/// The History list, the same way.
fn history_menu_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(HistoryTab)
}

/// The project's states plus the `Symbols` memo, built over the objects the way `app()`
/// builds it.
macro_rules! symbol_states {
    () => {
        |runner: &mut _| {
            let states = project_states!(runner);
            let objects = states.objects;
            runner.provide_root_context(move || {
                Symbols(Memo::create(move || {
                    SymbolList(Arc::new(
                        objects
                            .read()
                            .iter()
                            .flat_map(|object| {
                                object.symbols_sorted.iter().cloned().map(|data| Symbol {
                                    object: object.clone(),
                                    data,
                                })
                            })
                            .collect(),
                    ))
                }))
            });
            states
        }
    };
}

/// Presses the one entry of the menu a right-click at `row` opened, and says what it read.
fn choose_from_menu(test: &mut TestingRunner, row: (f64, f64), entry: &str) {
    right_click(test, row);
    let at = centre_of(test, entry);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(test);
}

/// A right-click on a symbol row offers to bookmark it, and the bookmark made is the row's
/// symbol under its whole name; the same gesture on a bookmarked symbol offers to remove it,
/// and does.
#[test]
fn a_symbol_row_bookmarks_its_symbol_from_its_menu() {
    let symbols = fixture_symbols();
    let wanted = symbols
        .iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to")
        .clone();

    let (mut test, states) =
        TestingRunner::new(symbols_harness, (300., 300.).into(), symbol_states!(), 1.);
    let (mut objects, bookmarks) = (states.objects, states.bookmarks);
    objects.set(vec![wanted.object.clone()]);
    settle(&mut test);

    let row = centre_of(&test, "sum_to");
    choose_from_menu(&mut test, row, "Add bookmark");
    let made = bookmarks.peek().entries().to_vec();
    let document = Document::Assembly(Selection::Symbol(wanted.clone()));
    assert_eq!(
        made,
        [Bookmark {
            name: entry_name(&document),
            document: SavedDocument::from_document(&document),
        }]
    );
    assert!(
        states.open.active().is_none(),
        "a right-click is not a press"
    );

    // Offered the other way round now, and taken.
    right_click(&mut test, row);
    let drawn = labels(&test);
    assert!(drawn.contains(&"Remove bookmark".to_owned()), "{drawn:?}");
    assert!(!drawn.contains(&"Add bookmark".to_owned()), "{drawn:?}");
    let at = centre_of(&test, "Remove bookmark");
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);
    assert!(bookmarks.peek().entries().is_empty());
}

/// A history row offers the same, for whatever kind of place it is: a file's row makes a
/// bookmark of the file.
#[test]
fn a_history_row_bookmarks_its_place_from_its_menu() {
    let file = Document::Source(Arc::from("/src/main.rs"));
    let (mut test, states) = TestingRunner::new(
        history_menu_harness,
        (300., 300.).into(),
        project_states!(),
        1.,
    );
    activate(states.open, states.history, Some(file.clone()), Visit::Went);
    settle(&mut test);

    let row = centre_of(&test, "main.rs");
    choose_from_menu(&mut test, row, "Add bookmark");
    let made = states.bookmarks.peek().entries().to_vec();
    assert_eq!(
        made,
        [Bookmark {
            name: "main.rs".into(),
            document: SavedDocument::Source {
                path: "/src/main.rs".into(),
            },
        }]
    );
}

/// A document's own header with the context-menu viewer a right-click on it needs. It
/// takes its document the way `close_harness` does, from the panel's first document tab.
fn header_menu_harness() -> impl IntoElement {
    let open = use_open();
    let id = open.dock.read().document_panel().and_then(|panel| {
        panel.tabs.iter().find_map(|tab| match tab {
            Tab::Document(id) => Some(*id),
            Tab::View(_) => None,
        })
    });

    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .maybe_child(id.map(|id| {
            DocumentHeader {
                id,
                active: true,
                key: DiffKey::None,
            }
            .into_element()
        }))
}

/// A tab's menu bookmarks the tab's document, and opens for a lone tab -- without the one
/// row that would do nothing -- where it used to open nothing at all; with company, both
/// rows are there.
#[test]
fn a_tabs_menu_bookmarks_its_document() {
    let symbols = fixture_symbols();
    let (first, second) = (
        Document::Assembly(Selection::Symbol(symbols[0].clone())),
        Document::Assembly(Selection::Symbol(symbols[1].clone())),
    );
    let (mut test, states) = TestingRunner::new(
        header_menu_harness,
        (300., 100.).into(),
        project_states!(),
        1.,
    );
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    activate(
        states.open,
        states.history,
        Some(first.clone()),
        Visit::Went,
    );
    settle(&mut test);

    let tab = centre_of(&test, &entry_text(&first));
    right_click(&mut test, tab);
    let drawn = labels(&test);
    assert!(drawn.contains(&"Add bookmark".to_owned()), "{drawn:?}");
    assert!(
        !drawn.contains(&"Close other tabs".to_owned()),
        "a lone tab offered to close the others: {drawn:?}"
    );
    let item = centre_of(&test, "Add bookmark");
    press_at(&mut test, item);
    settle(&mut test);
    assert_eq!(
        states.bookmarks.peek().entries().to_vec(),
        [bookmark_of(&first)]
    );

    activate(states.open, states.history, Some(second), Visit::Went);
    settle(&mut test);
    right_click(&mut test, tab);
    let drawn = labels(&test);
    assert!(drawn.contains(&"Close other tabs".to_owned()), "{drawn:?}");
    assert!(drawn.contains(&"Remove bookmark".to_owned()), "{drawn:?}");
}

/// An instruction row's menu bookmarks the symbol the row is code of, and says so, the
/// row being an instruction and not the symbol.
#[test]
fn an_instruction_rows_menu_bookmarks_its_symbol() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let studied = Studied::new(sum_to.clone());
    let first = studied.assembly.as_ref().unwrap().instructions[0].address;
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (mut test, (states, _marked, _landing)) = TestingRunner::new(
        menu_listing_harness,
        (600., 400.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    let mut objects = states.objects;
    objects.set(vec![sum_to.object.clone()]);
    let symbol = Document::Assembly(Selection::Symbol(sum_to.clone()));
    activate(
        states.open,
        states.history,
        Some(symbol.clone()),
        Visit::Went,
    );
    settle(&mut test);

    let row = centre_of(&test, &format!("{first:016X} "));
    right_click(&mut test, row);
    let item = centre_of(&test, "Bookmark symbol");
    press_at(&mut test, item);
    settle(&mut test);

    assert_eq!(
        states.bookmarks.peek().entries().to_vec(),
        [bookmark_of(&symbol)]
    );
    assert!(
        states.open.active() == Some(symbol),
        "the menu moved the reader"
    );
}

/// The Files panel and nothing else, with the context-menu viewer a right-click on an
/// object's row needs, over the project's states.
fn files_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(FilesTab)
}

/// A project directory of this test's own, named after the line that asked for it, and
/// the panel mounted over it as the project's directory.
fn files_over(line: u32) -> (TestingRunner, ProjectStates, PathBuf) {
    let directory = run_directory(line).join("project");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let (mut test, states) =
        TestingRunner::new(files_harness, (300., 400.).into(), project_states!(), 1.);
    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    settle(&mut test);
    (test, states, directory)
}

fn press(test: &mut TestingRunner, text: &str) {
    let at = centre_of(test, text);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(test);
}

/// A directory's row folds: what is under it is not drawn until the row is pressed, is
/// after, and is gone again after a second press.
#[test]
fn a_directory_row_unfolds_on_press() {
    let (mut test, _states, directory) = files_over(line!());
    std::fs::create_dir_all(directory.join("a")).expect("creating the test directory");
    std::fs::write(directory.join("a/b.c"), "int x;\n").expect("writing the source");
    // Made after the mount, so the root has to be read again to see it.
    press(&mut test, "project");
    press(&mut test, "project");

    assert!(label_area(&test, "a").is_some());
    assert!(label_area(&test, "b.c").is_none());

    press(&mut test, "a");
    assert!(label_area(&test, "b.c").is_some());

    press(&mut test, "a");
    assert!(label_area(&test, "b.c").is_none());
    let _ = std::fs::remove_dir_all(&directory);
}

/// Pressing a source file's row opens it as a source-driven tab, spelled as the project
/// directory joined with the entry's own name, and the history records the visit.
#[test]
fn a_source_row_opens_a_source_driven_tab() {
    let (mut test, states, directory) = files_over(line!());
    let path = directory.join("x.c");
    std::fs::write(&path, "int x;\n").expect("writing the source");
    press(&mut test, "project");
    press(&mut test, "project");
    assert!(states.open.active().is_none());

    press(&mut test, "x.c");

    let document = Document::Source(Arc::from(&*path.to_string_lossy()));
    assert!(states.open.active() == Some(document.clone()));
    assert!(states
        .history
        .peek()
        .recent()
        .any(|(_, entry)| *entry == document));
    let _ = std::fs::remove_dir_all(&directory);
}

/// A file's menu offers "Open file", which loads it the way the toolbar's Open does --
/// the parser deciding whether it is an object -- and once it is loaded offers "Close
/// file" instead, so a path is never opened twice.
#[test]
fn an_object_row_opens_from_its_menu() {
    let (mut test, states, directory) = files_over(line!());
    let (fixture, _) = fixture_objects(1);
    let path = directory.join("fixture.o");
    std::fs::copy(&fixture, &path).expect("copying the fixture");
    press(&mut test, "project");
    press(&mut test, "project");
    assert!(states.objects.peek().is_empty());

    let row = centre_of(&test, "fixture.o");
    right_click(&mut test, row);
    settle(&mut test);
    assert!(label_area(&test, "Close file").is_none());
    press(&mut test, "Open file");
    let objects = states.objects;
    pump(&mut test, || !objects.peek().is_empty());
    assert!(objects.peek().iter().all(|object| object.path == path));

    let row = centre_of(&test, "fixture.o");
    right_click(&mut test, row);
    settle(&mut test);
    assert!(label_area(&test, "Open file").is_none());
    assert!(label_area(&test, "Close file").is_some());
    let _ = std::fs::remove_dir_all(&directory);
}

/// A file past what the source cache will read opens nothing when pressed -- the tab
/// could only say so -- and still has its menu, since what it is is not judged here.
#[test]
fn a_file_past_the_source_bound_does_nothing_when_pressed() {
    let (mut test, states, directory) = files_over(line!());
    let path = directory.join("huge.txt");
    // Sparse, so the bound is crossed without writing it.
    std::fs::File::create(&path)
        .and_then(|file| file.set_len(source::MAX_SIZE + 1))
        .expect("writing the file");
    press(&mut test, "project");
    press(&mut test, "project");

    press(&mut test, "huge.txt");
    assert!(states.open.active().is_none());

    let row = centre_of(&test, "huge.txt");
    right_click(&mut test, row);
    settle(&mut test);
    assert!(label_area(&test, "Open file").is_some());
    let _ = std::fs::remove_dir_all(&directory);
}

/// With no directory set the panel says so and points at the Project view; setting one
/// is what brings the tree up, and clearing it takes the tree down again.
#[test]
fn no_directory_draws_the_placeholder() {
    let (mut test, states) =
        TestingRunner::new(files_harness, (300., 400.).into(), project_states!(), 1.);
    settle(&mut test);
    assert!(label_area(&test, "No project directory. Set one in the Project view.").is_some());

    let directory = run_directory(line!()).join("project");
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    std::fs::write(directory.join("main.rs"), "fn main() {}\n").expect("writing the source");
    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    // Two settles: the write wakes the effect, the effect's write wakes the memo, and the
    // memo's write is what the rows are drawn from.
    settle(&mut test);
    settle(&mut test);
    assert!(label_area(&test, "No project directory. Set one in the Project view.").is_none());
    assert!(label_area(&test, "main.rs").is_some());

    proj.write().directory = String::from("   ");
    settle(&mut test);
    settle(&mut test);
    assert!(label_area(&test, "main.rs").is_none());
    assert!(label_area(&test, "No project directory. Set one in the Project view.").is_some());
    let _ = std::fs::remove_dir_all(&directory);
}

/// The two configuration grammars colour what they parse: a TOML key is a property and a
/// JSON key is a special string, each the palette's own colour rather than the text's.
#[test]
fn toml_and_json_files_are_highlighted() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    set_appearance(Appearance::Light);

    let directory = run_directory(line!());
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let toml = directory.join("Cargo.toml");
    let json = directory.join("package.json");
    std::fs::write(&toml, b"name = \"viewer\"\n").expect("writing the file");
    std::fs::write(&json, b"{\"name\": 1}\n").expect("writing the file");

    // The span at `at` on the first line.
    let colour = |path: &Path, at: usize| {
        let text = source_text(path).expect("the file");
        let line = text.0.blocks.get_line(0);
        line.get(at).expect("a span there").0
    };
    let theme = Palette::LIGHT.syntax();

    assert_eq!(colour(&toml, 0), theme.property);
    assert_ne!(theme.property, theme.text);
    assert_eq!(colour(&json, 1), theme.string_special);
    assert_ne!(theme.string_special, theme.text);

    highlighted().clear();
    let _ = std::fs::remove_dir_all(&directory);
}

/// Every paragraph on screen, top to bottom: its box, its text -- the spans joined, an
/// inline child counting for nothing here -- and the highlight it draws.
fn paragraphs(test: &TestingRunner) -> Vec<(Area, String, Vec<(usize, usize)>)> {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    let mut found = test.find_many(|node, _element| {
        let element = node.element();
        (element.as_ref() as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .map(|paragraph| {
                let text: String = paragraph
                    .spans
                    .iter()
                    .map(|span| span.text.to_string())
                    .collect();
                (node.layout().area, text, paragraph.highlights.clone())
            })
    });
    found.sort_by(|a, b| a.0.origin.y.total_cmp(&b.0.origin.y));
    found
}

/// Every caret drawn, top to bottom: the strokes in the caret's colour, by their box.
fn carets(test: &TestingRunner) -> Vec<Area> {
    let mut found = rects_with(test, palette().caret_fg);
    found.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
    found
}

/// A point just inside the left edge of `area`, on its middle line.
fn left_of(area: &Area) -> (f64, f64) {
    (
        (area.origin.x + 1.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    )
}

/// A point just inside the right edge of `area`, on its middle line.
fn right_of(area: &Area) -> (f64, f64) {
    (
        (area.max_x() - 1.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    )
}

/// A sweep along a row's text picks characters out, from the pressed column to the one
/// under the pointer on whichever row it is over, and every row of it draws its own part:
/// the first from the pressed column to its end, the last from its start to the pointer.
/// While there are characters the grey row wash gives way to them; a plain press leaves
/// it, having picked out no characters yet. The ends of a row's text are what is pressed
/// on, so nothing here measures a font.
#[test]
fn a_sweep_along_the_text_picks_characters_out() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);

    let drawn = paragraphs(&test);
    let (first, first_text, _) = drawn[0].clone();
    let (second, second_text, _) = drawn[1].clone();
    let units = |text: &str| text.encode_utf16().count();

    // The text takes the row's whole height, so one row's highlight runs into the
    // next's with no gap between them.
    assert_eq!(first.height(), code_row_height(), "{first:?}");
    assert!(carets(&test).is_empty(), "a caret before any press");

    // The press: a run of one row, an empty run of characters under it, and the caret
    // where it was pressed.
    test.move_cursor(left_of(&first));
    test.press_cursor(left_of(&first));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), 0..=0);
    let chars = picked
        .chars
        .expect("a press on the text anchors the characters");
    assert!(chars.is_empty(), "nothing is swept yet: {chars:?}");
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert_eq!(
        caret[0].origin.x, first.origin.x,
        "the caret is not on column 0"
    );
    assert_eq!(
        rects_with(&test, palette().cursor_row_bg).len(),
        1,
        "a plain press washes the caret's row"
    );
    assert!(
        rects_with(&test, palette().text_select_bg).is_empty(),
        "a press on the text selected a row whole"
    );
    assert!(paragraphs(&test).iter().all(|(_, _, h)| h.is_empty()));

    // The sweep, to the end of the row below.
    test.move_cursor(right_of(&second));
    settle(&mut test);
    let drawn = paragraphs(&test);
    assert_eq!(drawn[0].2, vec![(0, units(&first_text))], "{drawn:?}");
    assert_eq!(drawn[1].2, vec![(0, units(&second_text))], "{drawn:?}");
    assert!(drawn[2..].iter().all(|(_, _, h)| h.is_empty()));
    // No caret while there is a highlight: the selection is what shows.
    assert!(carets(&test).is_empty());
    assert!(
        rects_with(&test, palette().cursor_row_bg).is_empty(),
        "the caret's wash stayed under the characters"
    );
    assert!(rects_with(&test, palette().text_select_bg).is_empty());
    let picked = marked.peek().assembly.clone().expect("the run stays");
    assert_eq!(
        picked.rows.rows(),
        0..=1,
        "the rows swept with the characters"
    );

    // Letting go ends the sweep and keeps what it picked out. The release is watched at
    // the app's root and not in this harness, so it is ended here as the root would.
    test.release_cursor(right_of(&second));
    mark_release(marked);
    test.move_cursor(left_of(&drawn[2].0));
    settle(&mut test);
    let after = paragraphs(&test);
    assert_eq!(after[1].2, vec![(0, units(&second_text))]);
    assert!(after[2].2.is_empty(), "the pointer alone swept on");
}

/// The address column is gutter: a press on it picks the row out and no characters, and
/// a sweep from it is a sweep of rows, washed grey as ever.
#[test]
fn a_press_on_the_address_picks_rows_out_alone() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let addresses = labels_with_areas(&test)
        .into_iter()
        .filter(|(text, _)| text.len() == 17 && text.ends_with(' '))
        .map(|(_, area)| area)
        .collect::<Vec<_>>();
    let (first, second) = (addresses[0], addresses[1]);
    let centre = |area: Area| {
        (
            (area.origin.x + area.width() / 2.0) as f64,
            (area.origin.y + area.height() / 2.0) as f64,
        )
    };

    test.move_cursor(centre(first));
    test.press_cursor(centre(first));
    test.move_cursor(centre(second));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), 0..=1);
    assert_eq!(
        picked.chars, None,
        "a press on the address picked characters out"
    );
    assert_eq!(rects_with(&test, palette().text_select_bg).len(), 2);
    assert!(paragraphs(&test).iter().all(|(_, _, h)| h.is_empty()));
}

/// Ctrl+C takes the characters where a sweep picked any out, and the rows otherwise; and
/// Escape peels the selection back the same way, the characters first and the rows on a
/// second press.
#[test]
fn the_characters_are_copied_before_the_rows_and_dropped_before_them() {
    let line = |row: usize| format!("row {row}");
    let text = |row: usize| Line::text(format!("text {row}"));
    let rows = RowSelection {
        anchor: 0,
        lead: 1,
        dragging: false,
    };
    let picked = |chars: Option<CharSelection>| Picked {
        rows,
        chars,
        file: None,
        owed: Owed::default(),
    };
    let swept = CharSelection::at(Caret { row: 0, col: 5 }).extended(Caret { row: 1, col: 4 });

    let marks = Marks {
        assembly: Some(picked(Some(swept))),
        source: Some(picked(None)),
    };
    assert_eq!(
        copy_text(&marks, Pane::Assembly, line, text).as_deref(),
        Some("0\ntext")
    );
    assert_eq!(
        copy_text(&marks, Pane::Source, line, text).as_deref(),
        Some("row 0\nrow 1")
    );
    // An empty run of characters -- a press without a sweep -- is no run at all.
    let pressed = Marks {
        assembly: Some(picked(Some(CharSelection::at(Caret { row: 0, col: 5 })))),
        source: None,
    };
    assert_eq!(
        copy_text(&pressed, Pane::Assembly, line, text).as_deref(),
        Some("row 0\nrow 1")
    );
    assert_eq!(copy_text(&pressed, Pane::Source, line, text), None);

    // Escape, through the pane's own key handler: the box has to have the keyboard,
    // which a press in it asks for.
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    let mut marked = marked;
    settle(&mut test);
    let first = paragraphs(&test)[0].0;
    test.move_cursor(left_of(&first));
    test.press_cursor(left_of(&first));
    test.release_cursor(left_of(&first));
    settle(&mut test);
    marked.set(marks.clone());
    settle(&mut test);
    assert!(
        !paragraphs(&test)[0].2.is_empty(),
        "the characters are drawn"
    );

    test.press_key(Key::Named(NamedKey::Escape));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the rows survive the first Escape");
    assert_eq!(picked.chars, None);
    assert_eq!(picked.rows.rows(), 0..=1);
    assert!(paragraphs(&test)[0].2.is_empty());
    assert_eq!(rects_with(&test, palette().text_select_bg).len(), 2);

    test.press_key(Key::Named(NamedKey::Escape));
    settle(&mut test);
    assert!(
        marked.peek().assembly.is_none(),
        "the rows survive the second"
    );
    assert!(
        marked.peek().source.is_some(),
        "the other pane's run is not this pane's to drop"
    );
}

/// A relocation link is an inline child of the row's text: to the text engine it is one
/// unit, at the column the pieces before it add up to, and it is still the link it was --
/// a press on it opens the target's tab.
#[test]
fn a_link_in_the_text_is_one_unit_and_still_opens_its_symbol() {
    let shown = shown_sum_to();
    let assembly = shown.studied.assembly.clone().expect("sum_to decodes");
    let lanes = shown.studied.lanes.clone();
    let (index, instruction) = assembly
        .instructions
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.relocation.is_some())
        .expect("sum_to calls add");
    let target = instruction.relocation.clone().expect("a target");
    let row = lanes.row_of(index);
    let line = instruction_line(&assembly, index);
    let before = Line {
        pieces: line
            .pieces
            .iter()
            .take_while(|piece| !matches!(piece, crate::chars::Piece::Inline(_)))
            .cloned()
            .collect(),
    }
    .units();
    assert!(before > 0 && before < line.units(), "{line:?}");

    let (mut test, (states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let link = label_area(&test, target.display()).expect("the link is drawn");

    // The left edge of the link is the column before it; its right edge is one unit on.
    test.move_cursor(left_of(&link));
    test.press_cursor(left_of(&link));
    test.move_cursor(right_of(&link));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), row..=row);
    let chars = picked
        .chars
        .expect("a press on the link is a press on the text");
    assert_eq!(
        chars.ends(),
        (
            Caret { row, col: before },
            Caret {
                row,
                col: before + 1
            }
        )
    );
    assert!(
        paragraphs(&test)
            .iter()
            .any(|(_, _, h)| h == &[(before, before + 1)]),
        "the link's unit is not highlighted"
    );

    // And the press still opens the symbol.
    test.release_cursor(right_of(&link));
    settle(&mut test);
    let opened = Document::Assembly(Selection::Symbol(Symbol {
        object: fixture_symbols()[0].object.clone(),
        data: target,
    }));
    assert!(
        states
            .open
            .active()
            .is_some_and(|active| match (&active, &opened) {
                (
                    Document::Assembly(Selection::Symbol(a)),
                    Document::Assembly(Selection::Symbol(b)),
                ) => a.data.name == b.data.name,
                _ => false,
            }),
        "the link did not open its symbol"
    );
}

/// Two presses on a word take the word, and a sweep after them goes on by character.
#[test]
fn a_double_press_takes_the_word_under_it() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let (first, text, _) = paragraphs(&test)[0].clone();
    let word = text.split(' ').next().expect("a mnemonic").len();
    assert!(word > 1 && word < text.len(), "{text:?}");

    let at = left_of(&first);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.press_cursor(at);
    settle(&mut test);
    let chars = marked
        .peek()
        .assembly
        .clone()
        .and_then(|picked| picked.chars)
        .expect("the presses picked characters out");
    assert_eq!(
        chars.ends(),
        (Caret { row: 0, col: 0 }, Caret { row: 0, col: word }),
        "{text:?}"
    );

    test.move_cursor(right_of(&first));
    settle(&mut test);
    let chars = marked
        .peek()
        .assembly
        .clone()
        .and_then(|picked| picked.chars)
        .expect("the sweep keeps the characters");
    assert_eq!(
        chars.ends(),
        (
            Caret { row: 0, col: 0 },
            Caret {
                row: 0,
                col: text.encode_utf16().count()
            }
        )
    );
}

/// The row copy and the character copy are one text: `asm_line` is the address column
/// and then the row's text as the characters see it, link and all, for every instruction
/// of a real object.
#[test]
fn a_rows_copy_is_its_address_and_its_text() {
    for symbol in fixture_symbols() {
        let Some(assembly) = symbol.data.assembly(&symbol.object) else {
            continue;
        };
        for (index, instruction) in assembly.instructions.iter().enumerate() {
            let line = instruction_line(&assembly, index);
            assert_eq!(
                asm_line(instruction, 0),
                format!("{:016X} {line}", instruction.address),
                "{}: instruction {index}",
                symbol.data.name
            );
        }
    }
}

/// The listing under half a pixel of something above it: what the real window does to it
/// through whatever the dock, the bars and the fonts add up to.
fn offset_listing_harness() -> impl IntoElement {
    let analysis = use_consume::<Analysis>().0;
    let document = analysis
        .read()
        .shown
        .as_ref()
        .map(|shown| asked_of(&shown.ask))
        .unwrap_or_else(|| Document::Source(Arc::from("")));

    rect()
        .expanded()
        .child(rect().height(Size::px(0.5)).width(Size::fill()))
        .child(rect().expanded().child(AssemblyPane { document }))
}

/// A listing laid out half a pixel down still draws its rows on whole device pixels: the
/// list pads its top by the fraction that puts its first row on the grid, so the washes of
/// two rows meet on a pixel edge instead of each fading into the other over the pixel
/// they share. The caret is a stroke on the same grid.
#[test]
fn a_listings_rows_sit_on_whole_device_pixels_wherever_it_is_laid_out() {
    let shown = shown_sum_to();
    let (mut test, (_states, _marked, _landing)) = TestingRunner::new(
        offset_listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let rows = paragraphs(&test);
    assert!(rows.len() > 2);
    for (area, text, _) in &rows {
        assert_eq!(area.origin.y.fract(), 0.0, "{text:?} at {}", area.origin.y);
        assert_eq!(
            area.height().fract(),
            0.0,
            "{text:?} is {} tall",
            area.height()
        );
    }

    // The caret too: pressed on the first column, it stands on the paragraph's own left
    // edge, one whole pixel wide.
    let first = rows[0].0;
    test.move_cursor(left_of(&first));
    test.press_cursor(left_of(&first));
    settle(&mut test);
    let carets = carets(&test);
    assert_eq!(carets.len(), 1, "{carets:?}");
    assert_eq!(carets[0].origin.x, first.origin.x);
    assert_eq!(carets[0].width(), 1.0);
    assert_eq!(carets[0].origin.y, first.origin.y);
    assert_eq!(carets[0].height(), first.height());
}

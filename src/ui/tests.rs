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
use crate::search::{Hit, SearchEvent, SearchQuery};
use crate::walk::WalkEvent;
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
        // No landing machinery here, so no landing to take.
        |_: &Landing, _: &mut ScrollController| false,
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
    at.write().forgetting(|tab| tab != "a");
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
        // No landing machinery here, so no landing to take.
        |_: &Landing, _: &mut ScrollController| false,
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

fn groups(area: &DockArea) -> Vec<PanelId> {
    fn walk(node: &DockNode<Panel, PanelId>, into: &mut Vec<PanelId>) {
        match node {
            DockNode::Panel(group) => into.push(group.panel_id),
            DockNode::Split { children, .. } => children.iter().for_each(|child| walk(child, into)),
        }
    }
    let mut found = Vec::new();
    walk(&area.tree, &mut found);
    found
}

/// A group the reader has dragged everything out of folds away, which is what keeps the
/// sidebar from filling up with the ghosts of groups.
#[test]
fn an_emptied_group_folds_away() {
    let mut dock = DockArea::column(vec![vec![Panel::Objects], vec![]]);
    dock.tidy();
    assert_eq!(groups(&dock), [0]);
}

/// The last group standing is kept even when it is empty, so the sidebar stays on screen
/// as somewhere to drop a panel back into.
#[test]
fn the_last_group_is_kept_when_it_empties() {
    let mut dock = DockArea::column(vec![vec![], vec![]]);
    dock.tidy();
    assert_eq!(groups(&dock).len(), 1);
}

/// Nothing on screen: what a project switch does is to the states. A runner all the same,
/// because a `State` needs a runtime and because a borrow held across a write is a runtime
/// panic rather than a compile error.
fn project_harness() -> impl IntoElement {
    rect().expanded()
}

/// The twelve contexts `app()` provides, in one `ProjectStates`. A macro and not a
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
        let strip = $runner
            .provide_root_context(|| OpenTabs(State::create(Strip::default())))
            .0;
        // The sidebar as `app()` builds it: a panel that brings another to the front
        // reaches for this, so a harness mounting one needs it provided.
        $runner.provide_root_context(|| {
            SidebarDock(State::create(DockArea::column(vec![
                vec![Panel::Objects, Panel::Files, Panel::Search],
                vec![Panel::Symbols],
                vec![Panel::History, Panel::Bookmarks, Panel::Locations],
            ])))
        });
        let docs = $runner
            .provide_root_context(|| OpenDocs(State::create(Docs::default())))
            .0;
        $runner.provide_root_context(move || {
            Active(Memo::create(move || {
                active_tab(&strip.read(), &docs.read())
            }))
        });
        // Provided but not returned, like `Active`: nothing here asserts on it, and the
        // Assembly pane's bar reads it wherever a harness mounts one.
        $runner.provide_root_context(|| Expanded(State::create(HashSet::new())));
        // Likewise: every pane registers its focusable box here, and every chip asks it
        // whether the keyboard is in the tab.
        $runner.provide_root_context(|| Keyboard(State::create(Vec::new())));
        // Likewise: both panes' bars read it, and `DocumentBody` asks it which panes
        // there are.
        $runner.provide_root_context(|| Follows(State::create(HashMap::new())));
        // Likewise: every row and link asks it whether a press opens a tab of its own.
        $runner.provide_root_context(|| Ctrl(State::create(false)));
        // And whether it is a door at all: Alt held says it is not.
        $runner.provide_root_context(|| Alt(State::create(false)));
        // Likewise: a recent project's row hands it to the switch, which is one of the two
        // places a file can be moved aside.
        $runner.provide_root_context(|| Rescued(State::create(Vec::new())));

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
            open: Open { strip, docs },
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
            marks_at: $runner
                .provide_root_context(|| MarksAt(State::create(Positions::default())))
                .0,
            visits: $runner
                .provide_root_context(|| Visited(State::create(Visits::default())))
                .0,
            bookmarks: $runner
                .provide_root_context(|| Bookmarked(State::create(Bookmarks::default())))
                .0,
            searched: $runner
                .provide_root_context(|| Searching(State::create(Searched::default())))
                .0,
            build: $runner
                .provide_root_context(|| Building(State::create(Builds::default())))
                .0,
        }
    }};
}

/// The documents on the trail behind `id`, oldest first: what a test asserts a trail by,
/// a stop's own address being the business of the tests that walk one inside a listing.
fn stops_of(states: &ProjectStates, id: DocId) -> Vec<Stop> {
    states
        .open
        .docs
        .peek()
        .trail(id)
        .map(|trail| trail.entries().to_vec())
        .unwrap_or_default()
}

fn trail_of(states: &ProjectStates, id: DocId) -> Vec<Document> {
    states
        .open
        .docs
        .peek()
        .trail(id)
        .map(|trail| {
            trail
                .entries()
                .iter()
                .map(|stop| stop.document.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The tab showing `document`, for the tests that speak of tabs by what they show.
fn tab_showing(states: &ProjectStates, document: &Document) -> Option<DocId> {
    states.open.docs.peek().showing(document)
}

/// The entry of the tab showing `document`: what its positions are kept under.
fn entry_of(states: &ProjectStates, document: &Document) -> Entry {
    (
        tab_showing(states, document).expect("the document is open"),
        Stop::whole(document.clone()),
    )
}

/// The entry of a place *inside* an object's code: the tab showing it, and the stop at
/// `address`, which is what two places in one listing are told apart by.
fn code_entry_of(states: &ProjectStates, document: &Document, address: u64) -> Entry {
    (
        tab_showing(states, document).expect("the document is open"),
        Stop::at(document.clone(), address),
    )
}

/// `close_tab` on the tab showing `document`.
fn close_document(states: &ProjectStates, document: &Document) {
    if let Some(id) = tab_showing(states, document) {
        close_tab(
            states.open,
            states.asm_at,
            states.src_at,
            states.code_at,
            states.driven,
            states.marks_at,
            id,
        );
    }
}

/// `raise` on the tab showing `document`: the strip's own move.
fn raise_document(states: &ProjectStates, document: &Document) {
    if let Some(id) = tab_showing(states, document) {
        raise(states.open, id);
    }
}

/// Where the active tab's trail cursor is.
fn cursor_of(states: &ProjectStates) -> Option<usize> {
    let id = states.open.active_id()?;
    states.open.docs.peek().trail(id)?.cursor()
}

/// The tab a harness mounts a pane in: the one showing `document`, or a stray id for a
/// pane mounted with no tab behind it, whose positions then go nowhere.
fn pane_tab(document: &Document) -> DocId {
    use_consume::<OpenDocs>()
        .0
        .read()
        .showing(document)
        .unwrap_or(DocId::stray())
}

/// Leaving a project leaves nothing of it behind: no object, no tab of either kind, no
/// viewing position, no visit and nothing active.
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
    let went = |target: Document| open_document(states.open, states.visits, target, Reach::NewTab);
    went(tab(&first));
    went(tab(&second));
    went(source.clone());
    let (first_entry, source_entry) = (entry_of(&states, &tab(&first)), entry_of(&states, &source));
    asm_at.write().remember(first_entry.clone(), 12);
    src_at.write().remember(source_entry.clone(), 7);
    test.sync_and_update();

    assert_eq!(states.open.documents().len(), 3);
    // Three visits, the source file included: the history records documents.
    assert_eq!(states.visits.peek().entries().len(), 3);

    clear_project(states);
    test.sync_and_update();

    assert!(
        states.objects.peek().is_empty(),
        "an object was left behind"
    );
    assert!(states.open.documents().is_empty(), "a tab was left behind");
    assert!(
        states.visits.peek().entries().is_empty(),
        "a history entry was left behind"
    );
    // Not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points into.
    assert_eq!(
        states.asm_at.peek().at(&first_entry),
        None,
        "a viewing position was left behind"
    );
    assert_eq!(
        states.src_at.peek().at(&source_entry),
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
    rect().expanded().child(HistoryPanel)
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
    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(symbol)),
        Reach::NewTab,
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
        .child(TabListButton)
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

    // A page is a tab, so the button is there before any document is.
    {
        let mut strip = states.open.strip;
        strip.write().show(Tab::Page(Page::Project));
    }
    test.sync_and_update();

    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == TAB_LIST_WIDTH).then_some(area)
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
    open_document(
        states.open,
        states.visits,
        Document::Source(Arc::from(
            "/x/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.rs",
        )),
        Reach::NewTab,
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
        width > TAB_LIST_WIDTH * 4.0,
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
        open_document(
            states.open,
            states.visits,
            Document::Assembly(Selection::Symbol(symbol.clone())),
            Reach::NewTab,
        );
    }
    test.sync_and_update();

    // The button is the only thing in the bar, so it is the one box of its own width.
    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == TAB_LIST_WIDTH).then_some(area)
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
        open_document(
            states.open,
            states.visits,
            Document::Assembly(Selection::Symbol(symbol.clone())),
            Reach::NewTab,
        );
    }
    test.sync_and_update();

    let nodes = |test: &TestingRunner| test.find_many(|_, _| Some(())).len();
    let shut = nodes(&test);

    // The button sits at the right-hand end of the bar, where the tab bar puts it.
    let button = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == TAB_LIST_WIDTH).then_some(area)
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
    // Along one tab's trail: the first opens the tab, the next two are followed in it.
    open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    );
    for document in &documents[1..] {
        open_document(states.open, states.visits, document.clone(), Reach::InPlace);
    }
    // Settled and not synced once: the pair reads `Active`, a memo, which is a beat
    // behind the states it is over.
    settle(&mut test);
    assert_eq!(states.open.documents().len(), 1);
    assert_eq!(cursor_of(&states), Some(2));

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
    assert_eq!(cursor_of(&states), Some(1));
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
    assert_eq!(cursor_of(&states), Some(0));
    assert!(
        !washes_under_the_pointer(&mut test, back),
        "back is on the oldest entry and still looks live"
    );

    // A press on a dimmed button is not a press at all.
    press_at(&mut test, back);
    assert_eq!(cursor_of(&states), Some(0), "a dimmed button navigated");

    press_at(&mut test, forward);
    assert_eq!(cursor_of(&states), Some(1));
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

/// The chips over a box the keyboard can be in, which is what a code pane is: pressing it
/// puts the keyboard inside the tab, and nothing else here can take it.
fn marker_harness() -> impl IntoElement {
    let a11y = use_a11y();
    use_tab_keyboard(a11y);

    rect()
        .expanded()
        .child(chips_harness().into_element())
        .child(
            rect()
                .width(Size::fill())
                .height(Size::px(40.0))
                .a11y_id(a11y)
                .a11y_focusable(true)
                .on_pointer_down(move |_| a11y.request_focus())
                .child(label().text("the pane")),
        )
}

/// The tab on screen wears a rule along its top, and its colour says where the keyboard
/// is: the gutter marks' purple while it is inside the tab, a dim grey while it is
/// anywhere else. Only the tab on screen wears one.
#[test]
fn the_tab_on_screen_is_marked_and_the_mark_says_where_the_keyboard_is() {
    let (mut test, states) =
        TestingRunner::new(marker_harness, (400., 200.).into(), project_states!(), 1.);
    for name in ["/src/one.rs", "/src/two.rs"] {
        let document = Document::Source(Arc::from(name));
        open_document(states.open, states.visits, document, Reach::NewTab);
    }
    test.sync_and_update();

    let markers = |test: &TestingRunner| -> Vec<Color> {
        test.find_many(|_node, element| {
            element
                .style()
                .borders
                .iter()
                .find(|border| border.width.top == TAB_MARKER)
                .map(|border| border.fill)
        })
    };

    let dim = dimmed(palette().icon_fg, palette().pane_bg);
    assert_eq!(
        markers(&test),
        [dim],
        "one mark, on the tab on screen, and grey while the keyboard is elsewhere"
    );

    let pane = centre_of(&test, "the pane");
    press_at(&mut test, pane);
    settle(&mut test);
    assert_eq!(
        markers(&test),
        [palette().compiled_fg],
        "the mark did not follow the keyboard into the tab"
    );
}

/// The menu at the top left is the whole of the way back to a closed page, so it lists all
/// three whether or not they are open, marks the ones that are, and opens a closed one
/// beside the tab on screen. It is mounted alone: where it sits in the toolbar is not what
/// this is about.
fn pages_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(PagesButton)
}

#[test]
fn the_menu_at_the_top_left_opens_a_page_and_marks_the_open_ones() {
    let (mut test, states) =
        TestingRunner::new(pages_harness, (300., 300.).into(), project_states!(), 1.);
    let document = Document::Source(Arc::from("/src/one.rs"));
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    {
        let mut strip = states.open.strip;
        strip.write().show(Tab::Page(Page::Settings));
    }
    // Back on the document, so a page opened from the menu has somewhere to land beside.
    raise_tab(states.open, Tab::Document(states.open.ids()[0]));
    test.sync_and_update();

    // The glyph is an image and not a label, so the button is found as the one box the
    // harness draws at the toggle's own size.
    let area = test
        .find(|node, _| {
            let area = node.layout().area;
            (area.width() == toggle_size() && area.height() == toggle_size()).then_some(area)
        })
        .expect("the button is a square of its own");
    let button = (
        (area.origin.x + area.width() / 2.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    );
    press_at(&mut test, button);
    settle(&mut test);

    let rows = labels(&test);
    for page in Page::ALL {
        assert!(
            rows.iter().any(|row| row == page.title()),
            "{} is not in the menu: {rows:?}",
            page.title()
        );
    }

    let row = centre_of(&test, "Project");
    press_at(&mut test, row);
    settle(&mut test);
    let strip = states.open.strip.peek();
    assert_eq!(
        strip.tabs(),
        [
            Tab::Document(strip.documents().next().expect("the document")),
            Tab::Page(Page::Project),
            Tab::Page(Page::Settings),
        ],
        "the page did not open beside the tab on screen"
    );
    assert_eq!(strip.active(), Some(Tab::Page(Page::Project)));
}

/// A page closes like any other tab, landing on the neighbour, and what it was showing is
/// there again when it is reopened: the state it draws lives at the root of the app.
#[test]
fn closing_a_page_lands_on_its_neighbour_and_keeps_what_it_held() {
    let (mut test, states) =
        TestingRunner::new(chips_harness, (400., 100.).into(), project_states!(), 1.);
    {
        let mut strip = states.open.strip;
        let mut strip = strip.write();
        strip.show(Tab::Page(Page::Project));
        strip.show(Tab::Page(Page::Settings));
        strip.show(Tab::Page(Page::Scratchpad));
        strip.raise(Tab::Page(Page::Settings));
    }
    test.sync_and_update();

    // Pressed, and not called: a page's chip draws a × of its own now, and that it does
    // is half of what this is about.
    let name = label_area(&test, "Settings").expect("the page's chip");
    let close = test
        .find_many(|node, _| {
            let area = node.layout().area;
            (area.width() == close_target()
                && area.height() == close_target()
                && area.origin.x > name.max_x())
            .then_some(area)
        })
        .into_iter()
        .min_by(|left, right| left.origin.x.total_cmp(&right.origin.x))
        .expect("the × on the page's chip");
    press_at(
        &mut test,
        (
            (close.origin.x + close.width() / 2.0) as f64,
            (close.origin.y + close.height() / 2.0) as f64,
        ),
    );
    settle(&mut test);
    let strip = states.open.strip.peek();
    assert_eq!(
        strip.tabs(),
        [Tab::Page(Page::Project), Tab::Page(Page::Scratchpad)]
    );
    assert_eq!(
        strip.active(),
        Some(Tab::Page(Page::Scratchpad)),
        "closing the page on screen did not land on its neighbour"
    );
}

/// A page resolves against no object, so a session that saved one puts it back **without
/// waiting for a binary** -- and a project with no binaries at all still opens on the page
/// the reader left it on. Written as the file spells it, so this pins the format as well
/// as the restore.
#[test]
fn a_saved_page_comes_back_with_no_binaries() {
    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    let session: Session = toml::from_str(
        "active_page = \"settings\"\n\n[[tabs]]\npage = \"project\"\n\n[[tabs]]\npage = \"settings\"\n",
    )
    .expect("a session naming two pages");

    restore_project(states, Project::default(), session);
    test.sync_and_update();

    let strip = states.open.strip.peek();
    assert_eq!(
        strip.tabs(),
        [Tab::Page(Page::Project), Tab::Page(Page::Settings)],
        "the pages did not come back"
    );
    assert_eq!(strip.active(), Some(Tab::Page(Page::Settings)));
}

/// Every open tab's chip, drawn as the bar draws them: what a press on one has to answer
/// for now that the bar is the app's own. freya's docking wrapped a header in a
/// `rect().on_press(set_active)` and the chip did nothing; here the chip is the whole of
/// it.
fn chips_harness() -> impl IntoElement {
    let open = use_open();
    let (tabs, active) = {
        let strip = open.strip.read();
        (strip.tabs().to_vec(), strip.active())
    };

    rect().expanded().horizontal().children(
        tabs.into_iter()
            .map(|tab| {
                TabHeader {
                    tab,
                    active: Some(tab) == active,
                    key: DiffKey::None,
                }
                .key(tab)
                .into_element()
            })
            .collect::<Vec<Element>>(),
    )
}

/// **The chip activates its own tab.** Nothing wraps it any more, so a press that reaches
/// the chip and stops there would leave the bar drawing tabs that cannot be switched to.
#[test]
fn pressing_a_chip_shows_its_tab() {
    let (mut test, states) =
        TestingRunner::new(chips_harness, (400., 100.).into(), project_states!(), 1.);
    let documents = [
        Document::Source(Arc::from("/src/one.rs")),
        Document::Source(Arc::from("/src/two.rs")),
    ];
    for document in &documents {
        open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    }
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[1].clone()));

    let at = centre_of(&test, "one.rs");
    press_at(&mut test, at);
    settle(&mut test);
    assert!(
        states.open.active() == Some(documents[0].clone()),
        "a press on a chip did not show its tab"
    );
}

/// The × on a document's tab, mounted on its own: how big a target it is and what the
/// pointer does to it are questions about the control rather than about the strip around
/// it. It takes its document the way a chip does, from the strip's first document tab.
fn close_harness() -> impl IntoElement {
    let open = use_open();
    let id = open.strip.read().documents().next();

    rect().expanded().maybe_child(id.map(|id| {
        TabClose {
            tab: Tab::Document(id),
        }
        .into_element()
    }))
}

/// One open document, and the × that closes it as it was actually laid out -- asserting on
/// the way past that the glyph has air around it, which is the whole of what the target is.
/// A comparison between the target and the glyph inside it and never an absolute width: the
/// second would be an assertion about the fonts on whoever ran it.
fn one_close_target(test: &mut TestingRunner, states: &ProjectStates) -> Area {
    let symbols = fixture_symbols();
    let document = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    open_document(states.open, states.visits, document, Reach::NewTab);
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
    assert_eq!(
        states.open.strip.peek().tabs().len(),
        0,
        "the tab outlived the document it stood for"
    );
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
        open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    }
    test.sync_and_update();
    assert!(agree(&states) == documents);

    // Opening one that is already open adds neither a tab nor an entry.
    open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    );
    test.sync_and_update();
    assert!(agree(&states) == documents);

    for document in &documents {
        close_document(&states, &document);
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
        open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    }
    // On the middle one, whose neighbours are on both sides -- the only arrangement
    // in which "the right-hand one" and "the leftmost one" are different answers.
    raise_document(&states, &documents[1]);
    test.sync_and_update();

    close_document(&states, &documents[1]);
    test.sync_and_update();
    assert!(
        states.open.active() == Some(documents[2].clone()),
        "a close landed on the leftmost tab rather than the neighbour"
    );

    // And closing the last one moves left, there being nothing to its right.
    close_document(&states, &documents[2]);
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[0].clone()));

    // Closing the only one left leaves nothing active at all.
    close_document(&states, &documents[0]);
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

    // A page in the bar, which this closes with the documents: what the reader asked for
    // is every other tab. Opened first, so the last document is still the tab on screen
    // when the close is asked for.
    {
        let mut strip = states.open.strip;
        strip.write().show(Tab::Page(Page::Settings));
    }
    for document in &documents {
        open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    }
    let mut asm_at = states.asm_at;
    let entries: Vec<Entry> = documents
        .iter()
        .map(|document| entry_of(&states, document))
        .collect();
    for (row, entry) in entries.iter().enumerate() {
        asm_at.write().remember(entry.clone(), row + 1);
    }
    test.sync_and_update();

    // Opened on the middle tab while the last one is the tab on screen, so the landing is
    // a move and not a tab simply staying where it was.
    let keep = states
        .open
        .docs
        .peek()
        .showing(&documents[1])
        .expect("the kept tab is open");
    close_others(
        states.open,
        states.asm_at,
        states.src_at,
        states.code_at,
        states.driven,
        states.marks_at,
        Tab::Document(keep),
    );
    test.sync_and_update();

    assert!(states.open.documents() == documents[1..2]);
    assert!(
        states.open.active() == Some(documents[1].clone()),
        "the tab on screen closed without landing on the one that was kept"
    );
    assert_eq!(
        states.asm_at.peek().at(&entries[1]),
        Some(2),
        "the kept tab lost the row it was left at"
    );
    assert!(
        states.asm_at.peek().at(&entries[0]).is_none()
            && states.asm_at.peek().at(&entries[2]).is_none(),
        "a closed tab's position was kept, and with it the binary it points into"
    );
    assert!(
        !states.open.strip.peek().contains(Tab::Page(Page::Settings)),
        "a page in the bar outlived \"Close other tabs\""
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
    let go = |target: &Document| {
        open_document(states.open, states.visits, target.clone(), Reach::NewTab)
    };

    go(&first);
    go(&second);
    test.sync_and_update();
    assert!(states.visits.peek().entries() == [first.clone(), second.clone()]);

    // Back to the first through the strip: it is already open, so the reader has gone
    // nowhere and the record stays as it was.
    raise_document(&states, &first);
    test.sync_and_update();
    assert!(states.open.active() == Some(first.clone()));
    assert!(
        states.visits.peek().entries() == [first.clone(), second.clone()],
        "a strip click was recorded as a visit"
    );

    // Going there deliberately *is* one, and bumps it to the newest position -- and
    // raises the tab that shows it rather than opening another.
    go(&first);
    test.sync_and_update();
    assert!(states.visits.peek().entries() == [second, first.clone()]);
    assert_eq!(states.open.documents().len(), 2);

    // And closing the tab lands on the neighbour without recording it.
    close_document(&states, &first);
    test.sync_and_update();
    assert_eq!(states.open.documents().len(), 1);
    assert_eq!(
        states.visits.peek().entries().len(),
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
    let went = |target: Document| open_document(states.open, states.visits, target, Reach::NewTab);
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
        states.marks_at,
        states.visits,
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

    rect().expanded().child(ObjectsPanel)
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
        states.marks_at,
        states.visits,
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
    rect().expanded().child(ObjectsPanel)
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
struct Driving(State<Option<Ask>>);

/// The analysis wiring and nothing else: no panes, since what is under test is which
/// answers reach them rather than what they draw.
fn analysis_harness() -> impl IntoElement {
    let asking = use_consume::<Driving>().0;
    let analysis = use_consume::<Analysis>().0;
    let objects = use_consume::<Objects>().0;
    let history = use_consume::<Visited>().0;
    let work = use_consume::<Work>().0;
    let mut seen = use_consume::<Seen>().0;
    let located = use_consume::<Locations>().0;
    let coded = use_consume::<Coding>().0;
    let reading = use_consume::<Sections>().0;
    let window = use_consume::<Window>().0;

    use_analysis_with(
        asking,
        objects,
        history,
        analysis,
        located,
        coded,
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
                .provide_root_context(|| Driving(State::create(None)))
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
                .provide_root_context(|| Visited(State::create(Visits::default())))
                .0,
            {
                $runner.provide_root_context(|| Coding(State::create(Coded::default())));
                $runner
                    .provide_root_context(|| Locations(State::create(Located::default())))
                    .0
            },
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
        chars: CharSelection::at(Caret { row, col: 0 }),
        by_rows: false,
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
                Question::Marks { .. } => "marks",
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

    // The gutter's marks are a fourth kind, worked last and cancelling none of the rest:
    // a file the reader has left is the one answer nothing is waiting on.
    let marks = |file: &str| Question::Marks {
        file: Arc::from(file),
        objects: Vec::new(),
    };
    let drained = newest(
        marks("a.c"),
        vec![
            locate(),
            code(vec![2]),
            Question::Study(symbols[0].clone()),
            marks("b.c"),
        ]
        .into_iter(),
    );
    assert_eq!(kinds(&drained), ["study", "code", "locate", "marks"]);
    let Question::Marks { file, .. } = &drained[3] else {
        panic!("the marks kind went missing");
    };
    assert_eq!(&**file, "b.c", "the older marks question won");
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
        .symbols()
        .expect("symbols")
        .0
        .iter()
        .map(|symbol| symbol.data.name.as_str())
        .collect();
    assert_eq!(names, ["sum_to", "sum_to"]);
    assert!(Arc::ptr_eq(
        &found.symbols().expect("symbols").0[0].object,
        &wanted.object
    ));
    assert!(Arc::ptr_eq(
        &found.symbols().expect("symbols").0[1].object,
        &twin
    ));

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
    assert!(state
        .found
        .expect("answered")
        .symbols()
        .expect("symbols")
        .0
        .is_empty());
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
    assert!(!found.symbols().expect("symbols").0.is_empty());
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
            .symbols()
            .expect("symbols")
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
    assert_eq!(found.symbols().expect("symbols").0.len(), 1);
    assert!(Arc::ptr_eq(
        &found.symbols().expect("symbols").0[0].object,
        &twin
    ));

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
            .symbols()
            .expect("symbols")
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
    assert!(state
        .found
        .expect("stands")
        .symbols()
        .expect("symbols")
        .0
        .is_empty());
}

/// The Locations view and nothing else, over the project's states and a `Located` the
/// test writes directly: what is under test is what the panel draws of an answer and
/// what a row does, not how the answer got there. `use_clear_focus` is mounted because a
/// row's press is answered by it.
fn locations_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let open = use_open();
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );

    rect().expanded().child(LocationsPanel)
}

/// The contexts [`locations_harness`] reads beside the project's.
#[derive(Clone, Copy)]
struct LocationStates {
    located: State<Located>,
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    analysis: State<Analyzed>,
}

macro_rules! location_states {
    ($runner:expr) => {{
        let states = project_states!($runner);
        let marked = $runner
            .provide_root_context(|| Marked(State::create(Marks::default())))
            .0;
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        let plant = $runner
            .provide_root_context(|| Plant(State::create(None)))
            .0;
        $runner.provide_root_context(|| Coding(State::create(Coded::default())));
        let located = $runner
            .provide_root_context(|| Locations(State::create(Located::default())))
            .0;
        let analysis = $runner
            .provide_root_context(|| Analysis(State::create(Analyzed::default())))
            .0;
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        (
            states,
            LocationStates {
                located,
                marked,
                landing,
                plant,
                analysis,
            },
        )
    }};
}

/// Pressing a use opens its file on its line with the name selected there, and the
/// assembly side follows that line as it follows a clicked one.
#[test]
fn a_reference_row_opens_its_file_on_the_line_with_the_name_selected() {
    let directory =
        std::env::temp_dir().join(format!("assembly-viewer-uses-row-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("used.rs");
    std::fs::write(&path, "fn main() {\n    let n = helper(1);\n}\n")
        .expect("writing the source file");
    let used = path.to_str().expect("a utf-8 temporary path").to_owned();

    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    settle(&mut test);

    let at = LinePos {
        file: Arc::from("/p/src/main.rs"),
        line: 2,
    };
    located.set(found_references(at, "helper", &[(&used, 2, 12..18)]));
    settle(&mut test);

    // The row draws the line it is on, as a search hit's row does -- the file was read
    // where the answer was taken -- cut into the name and what is around it, so the name
    // can be marked.
    let drawn = labels(&test);
    let line: Vec<String> = drawn
        .iter()
        .skip_while(|text| *text != "2")
        .skip(1)
        .cloned()
        .collect();
    assert_eq!(
        line,
        ["let n = ", "helper", "(1);"],
        "the use's line is not drawn: {drawn:?}"
    );

    let row = label_area(&test, "2").expect("the use's row is drawn");
    let press = ((row.origin.x + 5.0) as f64, (row.origin.y + 5.0) as f64);
    press_at(&mut test, press);
    settle(&mut test);

    let opened: Arc<str> = Arc::from(&*used);
    let document = Document::Source(opened.clone());
    assert!(
        states.open.active() == Some(document.clone()),
        "the row opened nothing"
    );
    assert!(
        source_line(location.marked)
            == Some(LinePos {
                file: opened,
                line: 2,
            }),
        "the line was not picked out"
    );
    // The name is selected there, which is what the answer's columns are for.
    let picked = location
        .marked
        .peek()
        .source
        .clone()
        .expect("checked above");
    assert_eq!(
        picked.chars.ends(),
        (Caret { row: 1, col: 12 }, Caret { row: 1, col: 18 }),
        "the name was not selected"
    );
    let id = states.open.active_id().expect("a tab");
    assert_eq!(
        states.driven.peek().line(&(id, Stop::on(document, 2))),
        Some(2),
        "the assembly side follows no line"
    );
}

/// A uses answer as the panel takes one: the question, and the places the server named.
fn found_references(at: LinePos, name: &str, places: &[(&str, u32, Range<u32>)]) -> Located {
    let query = Query::references(at, name.to_owned(), 0, 7);
    let places: Vec<lsp::Place> = places
        .iter()
        .map(|(file, line, columns)| lsp::Place {
            file: PathBuf::from(file),
            line: *line,
            columns: columns.clone(),
        })
        .collect();
    let mut located = Located {
        asked: Some(query),
        ..Located::default()
    };
    // Grouped as the worker groups it, reading each file's text off the disk.
    let found = references::References::of(&places, |path| std::fs::read_to_string(path).ok());
    assert!(located.answer_places(7, found), "the answer was not taken");
    located
}

/// The panel says which of the uses states it is in, groups what it found under the file
/// each use is in, and folds a file away when its row is pressed.
#[test]
fn the_panel_groups_a_names_references_under_their_files_and_folds_one_away() {
    let (mut test, (_states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    let at = LinePos {
        file: Arc::from("/p/src/main.rs"),
        line: 2,
    };
    settle(&mut test);

    located.write().asked = Some(Query::references(at.clone(), "helper".to_owned(), 12, 7));
    settle(&mut test);
    assert!(
        labels(&test).contains(&"Finding references to helper\u{2026}".to_owned()),
        "{:?}",
        labels(&test)
    );

    located.set(found_references(
        at.clone(),
        "helper",
        &[
            ("/p/src/other.rs", 9, 4..10),
            ("/p/src/main.rs", 2, 12..18),
            ("/p/src/main.rs", 7, 4..10),
        ],
    ));
    settle(&mut test);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"3 references to helper".to_owned()),
        "{drawn:?}"
    );
    // A file row per file, by path, each with its fold and its count, and its uses under
    // it by line. Everything the list draws, from the heading down.
    let listed: Vec<String> = drawn
        .iter()
        .skip_while(|text| *text != "3 references to helper")
        .cloned()
        .collect();
    assert_eq!(
        listed,
        [
            "3 references to helper",
            "\u{25be}",
            "main.rs",
            "2",
            "2",
            "7",
            "\u{25be}",
            "other.rs",
            "1",
            "9",
        ],
        "{drawn:?}"
    );

    // The file row folds its uses away, and the count stays.
    let file_row = centre_of(&test, "main.rs");
    press_at(&mut test, file_row);
    settle(&mut test);
    let folded: Vec<String> = labels(&test)
        .into_iter()
        .skip_while(|text| text != "3 references to helper")
        .collect();
    assert_eq!(
        folded,
        [
            "3 references to helper",
            // Folded, and its count stays: the heading and the row both count what was
            // found and not what is drawn.
            "\u{25b8}",
            "main.rs",
            "2",
            "\u{25be}",
            "other.rs",
            "1",
            "9",
        ],
        "the fold did not take main.rs's uses away"
    );
}

/// Nothing found says so, whatever way the question came back with nothing: a server
/// that answered no places, one that refused the question, and one that stopped
/// answering all leave the panel saying there are none.
#[test]
fn a_references_question_that_answers_nothing_says_there_are_none() {
    let (mut test, (_states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    let mut located = location.located;
    let at = LinePos {
        file: Arc::from("/p/src/main.rs"),
        line: 2,
    };
    settle(&mut test);

    located.set(found_references(at.clone(), "helper", &[]));
    settle(&mut test);
    assert!(
        labels(&test).contains(&"No references to helper".to_owned()),
        "{:?}",
        labels(&test)
    );

    // An answer under a run this did not ask in is an answer to nobody: the question
    // stands and the panel is still looking for it.
    let mut asking = Located {
        asked: Some(Query::references(at, "helper".to_owned(), 12, 7)),
        ..Located::default()
    };
    assert!(
        !asking.answer_places(8, references::References::default()),
        "an answer from another server"
    );
    assert!(asking.pending().is_some());
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
/// A server that is running and has answered what the file's names are.
///
/// Setting the state is not enough on its own any more: a link is drawn because the
/// server said the name is one, and saying so is a question sent to the worker and an
/// answer coming back, which is two turns of the loop and not one.
fn serving(test: &mut TestingRunner, language: &mut State<Language>) {
    language.write().state = Lsp::Running;
    // The worker is a real thread, so the answer arrives in its own time and not in this
    // one's: polling the executor without giving it any cannot see an answer that has not
    // been sent yet. A millisecond a turn, twenty turns, and the whole suite still runs
    // in seconds.
    for _ in 0..20 {
        settle(test);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn settle(test: &mut TestingRunner) {
    for _ in 0..4 {
        test.sync_and_update();
    }
}

/// What a call is, and what it is not. The colour says a name is a function's, a
/// method's or a macro's; what says it is being *called* is that the row does not begin a
/// function the file's own parse found and the name does not follow the `fn` keyword --
/// which is what covers a trait's method, declared with no body and so in no such list.
/// The spans a row draws, as a test reads them: the text of each, in order.
fn texts_of(spans: &[Span<'static>]) -> Vec<String> {
    spans.iter().map(|span| span.text.to_string()).collect()
}

/// One span per piece of text, all in one style.
fn spans_of(pieces: &[&str]) -> Vec<Span<'static>> {
    pieces
        .iter()
        .map(|text| Span::new(text.to_string()))
        .collect()
}

/// A link taken from a colour run is that whole run, so there is nothing to cut: the
/// spans come back as they went in. This is every link the app drew before a language
/// server said which names are links.
#[test]
fn a_link_that_is_a_whole_span_leaves_the_spans_alone() {
    let head = spans_of(&["let n = ", "helper", "(1);"]);
    let cut = cut_at(head, &[8..14]);

    assert_eq!(texts_of(&cut), vec!["let n = ", "helper", "(1);"]);
}

/// A link the server placed need not be a colour run, and a span holding one is cut into
/// the link and what was around it -- so the link is a span of its own to light, and the
/// cut is the same whether or not the pointer is anywhere near it.
#[test]
fn a_link_inside_a_span_cuts_it_into_the_link_and_the_rest() {
    let head = spans_of(&["w.count + 1"]);
    let cut = cut_at(head, &[2..7]);

    assert_eq!(texts_of(&cut), vec!["w.", "count", " + 1"]);
}

/// Two links in one span are both cut out, in the order they are drawn.
#[test]
fn two_links_in_one_span_are_both_cut_out() {
    let head = spans_of(&["a.one().two()"]);
    let cut = cut_at(head, &[2..5, 8..11]);

    assert_eq!(texts_of(&cut), vec!["a.", "one", "().", "two", "()"]);
}

/// A link may cross a colour boundary, which is why `light` draws every span the run
/// covers rather than one: the cut leaves the pieces whole and adds no boundary of its
/// own inside the link.
#[test]
fn a_link_across_two_spans_cuts_only_at_its_own_edges() {
    let head = spans_of(&["x = Vec", "::new()"]);
    let cut = cut_at(head, &[4..12]);

    assert_eq!(texts_of(&cut), vec!["x = ", "Vec", "::new", "()"]);
}

/// The columns are UTF-16 units, so a character outside the basic plane is two of them
/// and the boundaries inside a span are not every number. A cut that would fall between
/// the halves of one is not made -- a `char` is never sliced down the middle -- and the
/// cuts around it still are.
#[test]
fn a_cut_inside_a_character_is_not_made() {
    // `\u{1f600}` is two UTF-16 units, so this span is 1 + 2 + 1 = 4 units wide and the
    // only boundaries in it are 1 and 3.
    let head = spans_of(&["a\u{1f600}b"]);

    // A run beginning between its halves is cut where it ends, and not where it begins.
    assert_eq!(
        texts_of(&cut_at(head.clone(), &[2..3])),
        vec!["a\u{1f600}", "b"]
    );
    // A run on the character's own edges is cut on both.
    assert_eq!(
        texts_of(&cut_at(head.clone(), &[1..3])),
        vec!["a", "\u{1f600}", "b"]
    );
    // Whatever is cut, the row still draws the text it was given.
    for links in [vec![2..3], vec![1..3], vec![0..2], vec![2..4]] {
        assert_eq!(
            texts_of(&cut_at(head.clone(), &links)).concat(),
            "a\u{1f600}b"
        );
    }
}

/// Two lines of one file reached one after the other are two places on the tab's trail,
/// so Back returns to the line the reader came from. A source file used to be "one
/// place", and a door that landed on a line of the file already on screen left nothing
/// to come back to.
#[test]
fn two_lines_of_one_file_are_two_places_and_back_returns_to_the_first() {
    let file: Arc<str> = Arc::from("/src/main.rs");
    let document = Document::Source(file.clone());
    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    settle(&mut test);
    let id = open_document(states.open, states.visits, document.clone(), Reach::NewTab)
        .expect("the file opens");
    settle(&mut test);
    assert!(
        stops_of(&states, id) == vec![Stop::whole(document.clone())],
        "a file asked for by name is the file and not a line of it"
    );

    // The columns are a search hit's: what the landing carries beyond the line, and the
    // one part of it nothing else can put back.
    let at = |line: u32| Landing {
        tab: document.clone(),
        at: Some(LinePos {
            file: file.clone(),
            line,
        }),
        address: None,
        columns: Some(3..7),
    };
    let land_on_line = |test: &mut TestingRunner, line: u32| {
        land(
            states.open,
            states.visits,
            location.marked,
            location.landing,
            location.plant,
            at(line),
            Reach::InPlace,
        );
        settle(test);
    };

    // Two doors, one after the other, into the file that is already on screen.
    land_on_line(&mut test, 20);
    land_on_line(&mut test, 40);
    assert!(
        stops_of(&states, id)
            == vec![
                Stop::whole(document.clone()),
                Stop::on(document.clone(), 20),
                Stop::on(document.clone(), 40),
            ],
        "the two lines are not two places behind the file"
    );

    // And the same place again is no move at all.
    land_on_line(&mut test, 40);
    assert_eq!(stops_of(&states, id).len(), 3, "landing again was a move");

    // And the landing is picked out at the place landed on, columns and all: moving to a
    // new place inside the file must leave the same run behind it as arriving at one in
    // another file does.
    let (_, source) = runs_of(location.marked);
    let source = source.expect("the landing picked nothing out");
    assert!(
        source_line(location.marked)
            == Some(LinePos {
                file: file.clone(),
                line: 40,
            }),
        "the move landed on no line"
    );
    assert!(
        source.chars.ends() == (Caret { row: 39, col: 3 }, Caret { row: 39, col: 7 }),
        "the move landed without the landing's columns"
    );

    navigate(states.open, Nav::Back);
    settle(&mut test);
    assert!(
        states.open.active_stop().map(|(_, stop)| stop) == Some(Stop::on(document.clone(), 20)),
        "Back left the file it was inside"
    );
    navigate(states.open, Nav::Back);
    settle(&mut test);
    assert!(states.open.active_stop().map(|(_, stop)| stop) == Some(Stop::whole(document)));
}

/// A door into the document already on top lands through the change of *place* it makes,
/// as one into another document lands through the change of document: the run is the
/// arriving place's, so the columns the door named and the scroll it owes both panes
/// survive, and the place left keeps the run it was left with. `land` used to pick the
/// line out itself: the run was then on screen under the place being left, saved there,
/// and wiped a pass later by the arrival, which found no landing and fell back to the
/// place's own line.
#[test]
fn a_landing_into_the_document_on_top_keeps_its_columns_and_its_scroll() {
    let file: Arc<str> = Arc::from("/src/main.rs");
    let document = Document::Source(file.clone());
    let (mut test, (states, location)) = TestingRunner::new(
        locations_harness,
        (300., 300.).into(),
        |runner| location_states!(runner),
        1.,
    );
    settle(&mut test);
    open_document(states.open, states.visits, document.clone(), Reach::NewTab)
        .expect("the file opens");
    settle(&mut test);

    // The columns are a search hit's: what the landing carries beyond the line.
    let land_on_line = |test: &mut TestingRunner, line: u32| {
        land(
            states.open,
            states.visits,
            location.marked,
            location.landing,
            location.plant,
            Landing {
                tab: document.clone(),
                at: Some(LinePos {
                    file: file.clone(),
                    line,
                }),
                address: None,
                columns: Some(3..7),
            },
            Reach::InPlace,
        );
        settle(test);
    };

    land_on_line(&mut test, 20);
    land_on_line(&mut test, 40);
    let (_, source) = runs_of(location.marked);
    let source = source.expect("the landing picked nothing out");
    assert!(
        source.chars.ends() == (Caret { row: 39, col: 3 }, Caret { row: 39, col: 7 }),
        "the match itself is not selected, only its line"
    );
    assert!(source.owed == Owed::BOTH, "neither pane was owed a scroll");

    // And the run the first line was left with is the first line's, not the second's:
    // what is on screen when the place changes belongs to the place being left.
    navigate(states.open, Nav::Back);
    settle(&mut test);
    let (_, source) = runs_of(location.marked);
    let source = source.expect("the place came back with no run");
    assert_eq!(
        source.rows.anchor, 19,
        "Back came back holding the line it left for"
    );
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
        .visits
        .peek()
        .recent()
        .any(|entry| *entry == document));
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
    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(symbols[0].clone())),
        Reach::NewTab,
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
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    settle(&mut test);

    land(
        states.open,
        states.visits,
        location.marked,
        location.landing,
        location.plant,
        Landing {
            tab: document.clone(),
            at: Some(at.clone()),
            address: None,
            columns: None,
        },
        Reach::NewTab,
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
    open_document(states.open, states.visits, tab.clone(), Reach::NewTab);
    let entry = entry_of(&states, &tab);
    located.write().asked = Some(Query::line(at.clone()));
    located.write().subject = Some((entry.0, at.file.clone()));
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
    assert!(states.driven.peek().choice(&entry) == Some(wanted.clone()));
    assert_eq!(states.driven.peek().line(&entry), Some(at.line));
    assert!(source_line(location.marked) == Some(at.clone()));
    // Which is the question the tab now asks.
    assert!(
        ask(Some(&entry), &states.driven.peek())
            == Some(Ask::Source {
                at: at.clone(),
                chosen: Some(wanted.clone()),
            })
    );

    // The tab closed, the same row opens the symbol as a tab of its own.
    close_document(&states, &tab);
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
        at: Some(at.clone()),
        address: None,
        columns: None,
    }));
    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(symbols[1].clone())),
        Reach::NewTab,
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
        .symbols()
        .expect("symbols")
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
    assert_eq!(found.symbols().expect("symbols").0.len(), 1);
    assert_eq!(found.symbols().expect("symbols").0[0].data.name, "sum_to");

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
        .symbols()
        .expect("symbols")
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
    open_document(states.open, states.visits, tab.clone(), Reach::NewTab);
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
    rect().expanded().child(ContextMenuViewer::new()).child({
        let document = Document::Source(file);
        SourcePane {
            tab: pane_tab(&document),
            document,
        }
    })
}

/// The Source pane over a source-driven tab with a language server behind it: what a
/// press on a call in the text reaches, and what its answer opens.
fn linking_harness() -> impl IntoElement {
    let states = use_project_states();
    let language = use_consume::<Talking>().0;
    let follow = use_consume::<Following>().0;
    let located = use_consume::<Locations>().0;
    let linked = use_consume::<Linking>().0;
    let work = use_consume::<ServerWorking>().0;
    let jobs = use_language_with(language, follow, located, linked, states.proj, move |job| {
        work(job)
    });
    use_linking(language, linked, jobs);

    let open = use_open();
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    let active = use_consume::<Active>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    use_follow(follow, open, states.visits, marked, landing, plant, driven);

    let file = use_consume::<Subject>().0;
    let document = Document::Source(file);
    // The viewer a context menu is drawn into, which `app()` mounts at its root: what a
    // right-click on a link offers is one of the things asked of this harness.
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(SourcePane {
            tab: pane_tab(&document),
            document,
        })
}

/// The names a server would classify in [`calling_file`]'s text, which is
/// `fn main() {` and `    let n = helper(1);`.
///
/// The pane draws a link because the server said the name is one, so a test about links
/// has to say so: `helper` is a call, `main` is where a function is defined, and `n` is
/// where a local is bound. Only the first is a link; the other two are names the menu is
/// still offered on.
fn calling_links() -> links::Links {
    let legend = lsp::Legend::of(&["function", "variable"], &["declaration"]);
    let token = |line: u32, columns: Range<u32>, kind: u32, modifiers: u32| lsp::Token {
        line,
        columns,
        kind,
        modifiers,
    };
    links::Links::of(
        &legend,
        &[
            token(1, 3..7, 0, 0b1),
            token(2, 8..9, 1, 0b1),
            token(2, 12..18, 0, 0),
        ],
    )
}

/// Mount [`linking_harness`] over `file`, with a worker that records every job and
/// answers from `answer`.
///
/// The question about a file's names is answered for it, with [`calling_links`]: every
/// test here is over the one file, and none of them is about what a server calls a name.
macro_rules! mount_linking {
    ($answer:expr, $file:expr) => {
        mount_linking!($answer, $file, calling_links())
    };
    ($answer:expr, $file:expr, $links:expr) => {{
        let (asked, asks) = async_channel::unbounded::<AskedOfServer>();
        let answer = $answer;
        let links: links::Links = $links;
        let work = move |job: LspJob| {
            let recorded = match &job {
                LspJob::Start { directory, .. } => AskedOfServer::Start(directory.clone()),
                LspJob::Ask { at, want, .. } => AskedOfServer::Ask(at.clone(), *want),
                LspJob::Tokens { file, .. } => AskedOfServer::Tokens(file.clone()),
                LspJob::ReadSettings { directory } => AskedOfServer::Read(directory.clone()),
                LspJob::Stop => AskedOfServer::Stop,
            };
            let _ = asked.send_blocking(recorded);
            match &job {
                LspJob::Tokens { run, file } => Some(LspAnswer::Linked {
                    run: *run,
                    file: file.clone(),
                    links: Ok(links.clone()),
                }),
                _ => answer(job),
            }
        };
        let file: Arc<str> = $file;
        let (mut test, (states, language, location, driven)) = TestingRunner::new(
            linking_harness,
            (700., 400.).into(),
            move |runner: &mut _| {
                let (states, location) = location_states!(runner);
                runner.provide_root_context(|| Shift(State::create(false)));
                runner.provide_root_context(move || ServerWorking(Arc::new(work)));
                let language = runner
                    .provide_root_context(|| Talking(State::create(Language::default())))
                    .0;
                runner.provide_root_context(|| Following(State::create(Follow::default())));
                runner.provide_root_context(|| Linking(State::create(Linked::default())));
                runner.provide_root_context(move || Subject(file.clone()));
                // Which line each tab's assembly side follows: what the answer writes.
                let driven = runner
                    .provide_root_context(|| Drives(State::create(Driven::default())))
                    .0;
                (states, language, location, driven)
            },
            1.,
        );
        test.sync_and_update();
        (test, states, language, location, driven, asks)
    }};
}

/// The middle of the run reading `word` in the one code row that draws it.
///
/// `label_area` answers with the whole paragraph for a span, a code row's text being one
/// paragraph; a press has to land on the word. The font is fixed-width, so a column is a
/// column's width along the paragraph and the arithmetic is exact enough to land inside a
/// name.
fn word_point(test: &TestingRunner, word: &str) -> (f64, f64) {
    let (area, text, _) = paragraphs(test)
        .into_iter()
        .find(|(_, text, _)| text.contains(word))
        .unwrap_or_else(|| panic!("{word:?} is drawn"));
    let at = text.find(word).expect("found just above");
    let column = text[..at].encode_utf16().count() as f32;
    let width = area.width() / text.encode_utf16().count() as f32;
    let middle = area.min_x() + (column + word.encode_utf16().count() as f32 / 2.0) * width;
    (middle as f64, (area.origin.y + area.height() / 2.0) as f64)
}

/// The place the server was asked about, waited for: the worker takes its jobs on a
/// thread of its own, and the stop that follows the project on mount comes first.
fn next_ask(
    test: &mut TestingRunner,
    asks: &async_channel::Receiver<AskedOfServer>,
) -> Option<(Lookup, Wanted)> {
    // The press writes; the worker is handed its job a pass later.
    settle(test);
    while let Some(job) = next_job(asks) {
        if let AskedOfServer::Ask(at, want) = job {
            return Some((at, want));
        }
    }
    None
}

/// The file a call-following test reads, written where a test can put one.
fn calling_file(name: &str) -> (Arc<str>, PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-following-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("calls.rs");
    std::fs::write(&path, "fn main() {\n    let n = helper(1);\n}\n")
        .expect("writing the source file");
    (
        Arc::from(path.to_str().expect("a utf-8 temporary path")),
        directory,
    )
}

/// The answer opens the definition: the tab goes to the file and line the server named,
/// the assembly side follows that line as it follows a clicked one, and the place goes on
/// the trail so Back returns to the call.
#[test]
fn a_definition_answer_opens_the_file_and_line_it_names() {
    let (file, directory) = calling_file("opens");
    let defined = directory.join("helper.rs");
    std::fs::write(&defined, "fn helper(n: u32) -> u32 {\n    n\n}\n")
        .expect("writing the definition's file");
    let place = lsp::Place {
        file: defined.clone(),
        line: 1,
        columns: 3..9,
    };
    let (mut test, states, language, location, driven, _asks) = mount_linking!(
        move |job: LspJob| match job {
            LspJob::Ask { run, want, .. } => Some(LspAnswer::Answered {
                run,
                want,
                reply: Ok(Reply::Defined(vec![place.clone()])),
            }),
            _ => None,
        },
        file.clone()
    );
    let mut language = language;
    let calling = Document::Source(file.clone());
    open_document(states.open, states.visits, calling.clone(), Reach::NewTab);
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    for _ in 0..8 {
        settle(&mut test);
    }

    let opened: Arc<str> = Arc::from(defined.to_str().expect("a utf-8 temporary path"));
    let document = Document::Source(opened.clone());
    assert!(
        states.open.active() == Some(document.clone()),
        "the answer opened nothing"
    );
    assert!(
        states.open.active_stop().map(|(_, stop)| stop.line) == Some(Some(1)),
        "the place is the file and not the line in it"
    );
    // The assembly side follows that line, which is what a source-driven tab is driven
    // from -- and it is written under the place, not the file.
    let id = states.open.active_id().expect("a tab");
    let entry = (id, Stop::on(document, 1));
    assert_eq!(
        location
            .marked
            .peek()
            .source
            .as_ref()
            .map(|picked| picked.rows.anchor),
        Some(0),
        "the definition's line was not picked out"
    );
    assert_eq!(
        driven.peek().line(&entry),
        Some(1),
        "the assembly side follows no line"
    );

    // And Back returns to the call.
    navigate(states.open, Nav::Back);
    settle(&mut test);
    assert!(
        states.open.active() == Some(calling),
        "Back did not return to the call"
    );
}

/// The caret lands on the **name** and not at the start of its line: the column the
/// server named is where the answer's run is, and the run is empty, so nothing is
/// selected.
#[test]
fn a_definition_answer_puts_the_caret_on_the_name_it_names() {
    let (file, directory) = calling_file("column");
    let defined = directory.join("helper.rs");
    std::fs::write(&defined, "pub fn helper(n: u32) -> u32 {\n    n\n}\n")
        .expect("writing the definition's file");
    // `helper` is the seventh column of the definition's first line, counted from zero
    // in UTF-16 units, which is what the protocol answers in and what a row is drawn in.
    let place = lsp::Place {
        file: defined.clone(),
        line: 1,
        columns: 7..13,
    };
    let (mut test, states, language, location, _driven, _asks) = mount_linking!(
        move |job: LspJob| match job {
            LspJob::Ask { run, want, .. } => Some(LspAnswer::Answered {
                run,
                want,
                reply: Ok(Reply::Defined(vec![place.clone()])),
            }),
            _ => None,
        },
        file.clone()
    );
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    for _ in 0..8 {
        settle(&mut test);
    }

    let picked = location
        .marked
        .peek()
        .source
        .clone()
        .expect("the definition's line was not picked out");
    assert_eq!(
        picked.chars.lead(),
        Caret { row: 0, col: 7 },
        "the caret is not on the name"
    );
    assert!(
        picked.chars.is_empty(),
        "the name was selected where a caret was wanted"
    );
}

/// A name defined in the file the tab already shows lands the caret on it too. That door
/// marks its line itself and leaves no landing, the document not having changed, so the
/// new place woke the effect that rebuilds the runs and it found only the driven line --
/// a line and no column, which put the caret back at the row's start.
#[test]
fn a_definition_in_the_file_on_top_puts_the_caret_on_the_name_too() {
    let (file, directory) = calling_file("on-top");
    let _ = &directory;
    // The call's own file: line 2 is `    let n = helper(1);`, whose column 12 is the
    // `h`. A definition in the file already shown is the same door and a different path.
    let place = lsp::Place {
        file: PathBuf::from(&*file),
        line: 2,
        columns: 12..18,
    };
    let (mut test, states, language, location, _driven, _asks) = mount_linking!(
        move |job: LspJob| match job {
            LspJob::Ask { run, want, .. } => Some(LspAnswer::Answered {
                run,
                want,
                reply: Ok(Reply::Defined(vec![place.clone()])),
            }),
            _ => None,
        },
        file.clone()
    );
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    for _ in 0..8 {
        settle(&mut test);
    }

    let picked = location
        .marked
        .peek()
        .source
        .clone()
        .expect("the definition's line was not picked out");
    assert_eq!(
        picked.chars.lead(),
        Caret { row: 1, col: 12 },
        "the caret is not on the name"
    );
}

/// A right-click on a link offers the name's uses, and asks for them where the pointer
/// was: the question carries the name it was on and the column it begins at. A press
/// elsewhere in the row offers no such thing -- the answer would be to no name.
#[test]
fn a_right_click_on_a_link_offers_the_names_references() {
    let (file, _directory) = calling_file("uses");
    let (mut test, states, language, location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    // Off the link, on the row's own gutter: the line's locations and nothing about a
    // name.
    let gutter = centre_of(&test, "2\u{a0}");
    right_click(&mut test, gutter);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"Find all locations".to_owned()),
        "{drawn:?}"
    );
    assert!(
        !drawn.iter().any(|text| text.starts_with("Find references")),
        "{drawn:?}"
    );
    // The menu is closed by pressing away from it, as a reader closes one.
    press_at(&mut test, (600.0, 380.0));
    settle(&mut test);

    let call = word_point(&test, "helper");
    right_click(&mut test, call);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"Find references to helper".to_owned()),
        "{drawn:?}"
    );

    let entry = centre_of(&test, "Find references to helper");
    press_at(&mut test, entry);
    settle(&mut test);

    // The question the panel now holds: the name, and where it was asked about.
    let asked = location.located.peek().asked.clone().expect("a question");
    assert_eq!(asked.at.line, 2, "the question is about the wrong line");
    let Scope::References { name, column, .. } = &asked.scope else {
        panic!("the question is not about a name's references");
    };
    assert_eq!(name, "helper");
    // Where `helper` begins on `    let n = helper(1);`.
    assert_eq!(*column, 12);

    // And the server was asked, at the same place, in the units the protocol takes.
    let (asked_of, _) = next_ask(&mut test, &asks).expect("the server was asked");
    assert_eq!((asked_of.line, asked_of.column), (1, 12));
}

/// A name where one is **defined** is not a link, and the server saying so is the whole
/// of how the app knows: `main` here is a `function` carrying `declaration`. Pressing it
/// asks nobody anything and picks the line out, as a press on plain text does.
///
/// This is what replaced guessing from the `fn` keyword in front of a name.
#[test]
fn a_name_where_one_is_defined_is_not_a_link() {
    let (file, _directory) = calling_file("defined");
    let (mut test, states, language, location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    // `main` on `fn main() {`, which `calling_links` marks as a declaration.
    let defined = word_point(&test, "main");
    press_at(&mut test, defined);
    settle(&mut test);

    assert!(
        next_ask(&mut test, &asks).is_none(),
        "a definition's own name asked the server where it is"
    );
    assert!(
        location.marked.peek().source.is_some(),
        "a press on it did not pick its line out, as a press on text does"
    );
}

/// Some links, so a test can tell an answer from an empty one.
fn a_link() -> links::Links {
    links::Links::of(
        &lsp::Legend::of(&["function"], &[]),
        &[lsp::Token {
            line: 1,
            columns: 0..4,
            kind: 0,
            modifiers: 0,
        }],
    )
}

/// **A refused question answers empty, and it must not land on top of the names that
/// came back.**
///
/// The server says how far through the project it has got over and over, and every word
/// of it is a reason to look again; a second question going out while the first was in
/// flight came back refused, and wrote nothing over something. What stops it is holding
/// the question that is on its way, as `Follow` and `Located` both do.
#[test]
fn an_answer_to_a_question_nobody_asked_is_not_taken() {
    let file: Arc<str> = Arc::from("/p/src/main.rs");
    let mut linked = Linked::default();
    linked.wanted = Some(file.clone());

    assert!(linked.asking(1, file.clone()), "the question went out");
    assert!(linked.answer(1, file.clone(), a_link()), "and was answered");
    assert_eq!(
        linked.links_in(&file).map(links::Links::is_empty),
        Some(false)
    );

    // The same question again, refused and so empty. Nobody is waiting for it.
    assert!(
        !linked.answer(1, file.clone(), links::Links::default()),
        "an answer arriving twice was taken twice"
    );
    assert_eq!(
        linked.links_in(&file).map(links::Links::is_empty),
        Some(false),
        "an empty answer wrote over the names that came back"
    );
}

/// A question already on its way is not asked again, which is what keeps there being one
/// answer to take.
#[test]
fn a_question_on_its_way_is_not_asked_again() {
    let file: Arc<str> = Arc::from("/p/src/main.rs");
    let mut linked = Linked::default();
    linked.wanted = Some(file.clone());

    assert_eq!(
        linked.pending(1),
        Some(&file),
        "nothing asked, nothing held"
    );
    linked.asking(1, file.clone());
    assert_eq!(linked.pending(1), None, "the question went out twice");

    // A server that has been restarted is a different question: what is in flight is the
    // old one's, and its answer will be an answer to nobody.
    assert_eq!(linked.pending(2), Some(&file));

    linked.answer(1, file.clone(), a_link());
    assert_eq!(
        linked.pending(1),
        None,
        "an answered question was asked again"
    );

    // And the pane moving to another file is a question of its own.
    linked.wanted = Some(Arc::from("/p/src/other.rs"));
    assert!(linked.pending(1).is_some());
}

/// A **declaration** the server places on the line it was asked about opens nothing. A
/// trait's own method declaration is the case that makes this real: nothing the server
/// says tells it from a trait `impl`'s item, so it is a link, and asking where it is
/// declared answers with itself.
///
/// A *definition* naming that line is a different matter and still opens -- a name defined
/// where it is used is a place like any other
/// (`a_definition_in_the_file_on_top_puts_the_caret_on_the_name_too`).
#[test]
fn a_declaration_the_server_places_on_its_own_line_opens_nothing() {
    let (file, _directory) = calling_file("itself");
    // Classified as a trait `impl`'s item, so the press asks where it is *declared*.
    let legend = lsp::Legend::of(&["function"], &["declaration", "trait"]);
    let in_an_impl = links::Links::of(
        &legend,
        &[lsp::Token {
            line: 2,
            columns: 12..18,
            kind: 0,
            modifiers: 0b11,
        }],
    );
    // The answer names the very line the question was asked on: row 2 of the file, which
    // is where `helper` is.
    let itself = lsp::Place {
        file: PathBuf::from(&*file),
        line: 2,
        columns: 12..18,
    };
    let (mut test, states, language, location, _driven, _asks) = mount_linking!(
        move |job: LspJob| match job {
            LspJob::Ask { run, want, .. } => Some(LspAnswer::Answered {
                run,
                want,
                reply: Ok(Reply::Defined(vec![itself.clone()])),
            }),
            _ => None,
        },
        file.clone(),
        in_an_impl
    );
    let mut language = language;
    let calling = Document::Source(file.clone());
    open_document(states.open, states.visits, calling.clone(), Reach::NewTab);
    settle(&mut test);
    serving(&mut test, &mut language);

    let tab = states.open.active_tab().expect("a tab").0;
    let before = stops_of(&states, tab).len();
    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    for _ in 0..8 {
        settle(&mut test);
    }

    assert!(
        location.marked.peek().source.is_none(),
        "an answer naming the line it was asked about picked a line out"
    );
    assert_eq!(
        stops_of(&states, tab).len(),
        before,
        "it put a step on the trail that goes nowhere"
    );
}

/// The spans each drawn paragraph is made of, in the order they are drawn.
fn drawn_spans(test: &TestingRunner) -> Vec<Vec<String>> {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    let mut found = test.find_many(|node, _element| {
        let element = node.element();
        (element.as_ref() as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .map(|paragraph| {
                let texts: Vec<String> = paragraph
                    .spans
                    .iter()
                    .map(|span| span.text.to_string())
                    .collect();
                (node.layout().area.origin.y, texts)
            })
    });
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, texts)| texts).collect()
}

/// A link the server placed need not line up with a colour boundary, and the row is cut at
/// its edges so that it does: without that the run to light would be part of a span, `light`
/// would match nothing, and the reader would get a name they can click and cannot see.
///
/// The link here is deliberately **not** a whole colour run -- it is `elper` inside
/// `helper` -- since a link that lines up would prove nothing about the cutting.
#[test]
fn a_link_that_is_not_a_colour_run_is_still_a_span_of_its_own() {
    let (file, _directory) = calling_file("cuts");
    let legend = lsp::Legend::of(&["function"], &[]);
    let inside_a_run = links::Links::of(
        &legend,
        &[lsp::Token {
            line: 2,
            // `    let n = helper(1);` -- `elper`, four columns into the name.
            columns: 13..18,
            kind: 0,
            modifiers: 0,
        }],
    );
    let (mut test, states, language, _location, _driven, _asks) =
        mount_linking!(|_job: LspJob| None, file.clone(), inside_a_run);
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let drawn = drawn_spans(&test);
    let row = drawn
        .iter()
        .find(|spans| spans.concat().contains("helper"))
        .expect("the row with the call in it");
    assert!(
        row.contains(&"elper".to_owned()),
        "the link is not a span of its own: {row:?}"
    );
    // And the row still draws the text it was given, cut or not.
    assert!(
        row.concat().contains("let n = helper(1);"),
        "the cut lost or reordered the row's text: {row:?}"
    );
}

/// An item in a trait `impl` is the one name that asks the **other** question: its
/// definition is itself, and the trait is where a reader following it wants to go. The
/// server says which by putting `declaration` and `trait` on it together.
#[test]
fn an_item_in_a_trait_impl_asks_the_server_for_its_declaration() {
    let (file, _directory) = calling_file("impls");
    // The same file, with `helper` classified as a trait `impl`'s method rather than a
    // call: the text is not what decides this, the server is.
    let legend = lsp::Legend::of(&["function"], &["declaration", "trait"]);
    let in_an_impl = links::Links::of(
        &legend,
        &[lsp::Token {
            line: 2,
            columns: 12..18,
            kind: 0,
            modifiers: 0b11,
        }],
    );
    let (mut test, states, language, _location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone(), in_an_impl);
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    settle(&mut test);

    let (asked, want) = next_ask(&mut test, &asks).expect("the press asked the server");
    assert_eq!((asked.line, asked.column), (1, 12));
    assert_eq!(
        want,
        Wanted::Declaration,
        "a trait impl's item asked where it is defined, which is itself"
    );
}

/// The three questions a click cannot ask are all on the name, and "Find implementations"
/// is the panel's second: it holds a question of its own kind and asks the server at the
/// place the reader pointed.
#[test]
fn a_right_click_on_a_name_offers_the_three_questions_for_the_server() {
    let (file, _directory) = calling_file("asks3");
    let (mut test, states, language, location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    right_click(&mut test, call);
    let drawn = labels(&test);
    for offered in [
        "Go to definition",
        "Find references to helper",
        "Find implementations",
    ] {
        assert!(drawn.contains(&offered.to_owned()), "{offered}: {drawn:?}");
    }

    let entry = centre_of(&test, "Find implementations");
    press_at(&mut test, entry);
    settle(&mut test);

    // Its own kind of question, and not the references one under another name: the two
    // supersede each other in the panel, which is why they are told apart at all.
    let asked = location.located.peek().asked.clone().expect("a question");
    assert_eq!(asked.at.line, 2, "the question is about the wrong line");
    let Scope::Implementations { name, column, .. } = &asked.scope else {
        panic!("the question is not about what implements a name");
    };
    assert_eq!(name, "helper");
    assert_eq!(*column, 12);

    let (asked_of, _) = next_ask(&mut test, &asks).expect("the server was asked");
    assert_eq!((asked_of.line, asked_of.column), (1, 12));
}

/// The name where a function is **defined** offers its references too, though it is no
/// link: where a name is defined is where a reader asks what refers to it.
#[test]
fn a_right_click_on_a_definitions_own_name_offers_its_references() {
    let (file, _directory) = calling_file("defuses");
    let (mut test, states, language, location, _driven, _asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let defined = word_point(&test, "main");
    right_click(&mut test, defined);
    let drawn = labels(&test);
    assert!(
        drawn.contains(&"Find references to main".to_owned()),
        "{drawn:?}"
    );

    let entry = centre_of(&test, "Find references to main");
    press_at(&mut test, entry);
    settle(&mut test);

    let asked = location.located.peek().asked.clone().expect("a question");
    assert_eq!(asked.at.line, 1);
    let Scope::References { name, column, .. } = &asked.scope else {
        panic!("the question is not about a name's references");
    };
    assert_eq!(name, "main");
    // Where `main` begins on `fn main() {`.
    assert_eq!(*column, 3);
}

/// A server that refuses the question answers it as far as the panel is concerned: the
/// list says there are no uses, where a question left pending would say it was still
/// looking for ever.
#[test]
fn a_refused_references_question_leaves_the_panel_saying_there_are_none() {
    let (file, _directory) = calling_file("refused");
    let (mut test, states, language, location, _driven, _asks) = mount_linking!(
        |job: LspJob| match job {
            LspJob::Ask { run, want, .. } => Some(LspAnswer::Answered {
                run,
                want,
                reply: Err(lsp::Failure::Refused {
                    code: -32603,
                    said: "file not found".to_owned(),
                }),
            }),
            _ => None,
        },
        file.clone()
    );
    let mut language = language;
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    right_click(&mut test, call);
    let entry = centre_of(&test, "Find references to helper");
    press_at(&mut test, entry);
    // Waited for rather than counted in passes: two workers stand between the press and
    // the panel, and how many turns they take is not something a test can know.
    pump(&mut test, || location.located.peek().found.is_some());

    let state = location.located.peek().clone();
    assert!(
        state.pending().is_none(),
        "the panel is still looking for an answer that will not come"
    );
    assert_eq!(
        state
            .found
            .as_ref()
            .and_then(Found::places)
            .map(references::References::count),
        Some(0),
        "a refusal is not an empty answer"
    );
    // And the server is still a server: refusing a question is answering it.
    assert!(
        matches!(language.peek().state, Lsp::Running),
        "the control was told the server broke"
    );
}

/// With no server there are no links, so there is no name to ask about and the menu
/// offers nothing: a question is not what starts a server.
#[test]
fn a_right_click_with_no_server_offers_no_references() {
    let (file, _directory) = calling_file("nouses");
    let (mut test, states, _language, _location, _driven, _asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);

    let call = word_point(&test, "helper");
    right_click(&mut test, call);
    let drawn = labels(&test);
    assert!(
        !drawn.iter().any(|text| text.starts_with("Find references")),
        "{drawn:?}"
    );
}

/// A press on a call asks the server where the name is defined, at the place the pointer
/// was on and in the units the protocol takes -- the line counted from zero and the
/// column in UTF-16 units, which is what a source row's columns already are. And it picks
/// no line out: the press is the question and not a place in the file.
#[test]
fn a_press_on_a_call_asks_where_the_name_is_defined() {
    let (file, _directory) = calling_file("asks");
    let (mut test, states, language, location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let mut language = language;
    let document = Document::Source(file.clone());
    open_document(states.open, states.visits, document, Reach::NewTab);
    settle(&mut test);
    // A server there to be asked: nothing is a link without one. Written after the
    // project has settled, since a project arriving is what stops a server.
    serving(&mut test, &mut language);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    settle(&mut test);

    let (asked, want) = next_ask(&mut test, &asks).expect("the press asked the server");
    assert_eq!(
        asked,
        Lookup {
            file: PathBuf::from(&*file),
            // `let n = helper(1);` is the file's second row, and `helper` its twelfth
            // column: both counted from zero, which is the protocol's own counting.
            line: 1,
            column: 12,
        }
    );
    // An ordinary name asks where it is defined. The other question is for an item in a
    // trait `impl`, whose definition is itself.
    assert_eq!(want, Wanted::Definition);
    assert!(
        location.marked.peek().source.is_none(),
        "the press picked a line out"
    );
}

/// With no server there is nothing to ask, so a press on the same name is a press on the
/// row: the line is picked out, as any other press in a source-driven tab picks it out.
#[test]
fn a_press_on_a_call_with_no_server_picks_the_line_out() {
    let (file, _directory) = calling_file("nobody");
    let (mut test, states, _language, location, _driven, asks) =
        mount_linking!(|_job: LspJob| None, file.clone());
    let document = Document::Source(file.clone());
    open_document(states.open, states.visits, document, Reach::NewTab);
    settle(&mut test);

    let call = word_point(&test, "helper");
    press_at(&mut test, call);
    settle(&mut test);

    settle(&mut test);
    assert!(
        std::iter::from_fn(|| asks.try_recv().ok())
            .all(|asked| !matches!(asked, AskedOfServer::Ask(..))),
        "a question was asked with no server"
    );
    assert!(
        location.marked.peek().source.is_some(),
        "the press picked no line out"
    );
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
                runner.provide_root_context(|| Coding(State::create(Coded::default())));
                let located = runner
                    .provide_root_context(|| Locations(State::create(Located::default())))
                    .0;
                (states, located)
            }
        },
        1.,
    );
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
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
    assert!(
        located
            .peek()
            .subject
            .as_ref()
            .map(|(_, subject)| &**subject)
            == Some(&*file)
    );
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

/// Asking for a line's locations is one write and one dock change: the question reaches
/// the worker and is answered, and the Locations panel is brought to the top of whichever
/// group holds it -- here it is behind History. Asking for the line already answered asks
/// again.
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
        ((_asking, _analysis, _seen, objects, _history, located, _reading, _window), sidebar),
    ) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| {
            let states = analysis_states!(runner, answer);
            // The sidebar as `app()` builds it, with Locations behind History in one
            // group.
            let sidebar = runner
                .provide_root_context(|| {
                    SidebarDock(State::create(DockArea::column(vec![vec![
                        Panel::History,
                        Panel::Locations,
                    ]])))
                })
                .0;
            (states, sidebar)
        },
        1.,
    );
    let mut objects = objects;
    objects.set(vec![wanted.object.clone()]);
    test.sync_and_update();

    let on_top = |dock: State<DockArea>| {
        let dock = dock.peek();
        let (group, _) = dock.tree.find_tab(&Panel::Locations)?;
        dock.tree.panel(&group)?.active_tab_id
    };
    assert!(on_top(sidebar) == Some(Panel::History));

    find_locations(located, sidebar, Query::line(at.clone()), None);
    assert!(on_top(sidebar) == Some(Panel::Locations));
    assert!(located.peek().pending() == Some(&Query::line(at.clone())));
    pump(&mut test, || located.peek().found.is_some());
    let found = located.peek().found.clone().expect("answered");
    assert!(found.of.at == at);
    assert_eq!(found.symbols().expect("symbols").0.len(), 1);

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
            .symbols()
            .expect("symbols")
            .0
            .len(),
        1,
        "an answer re-asked itself when an object was opened"
    );
    find_locations(located, sidebar, Query::line(at.clone()), None);
    assert!(located.peek().pending() == Some(&Query::line(at.clone())));
    pump(&mut test, || {
        located
            .peek()
            .found
            .as_ref()
            .is_some_and(|found| found.symbols().expect("symbols").0.len() == 2)
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
    // Two tabs, so a line can be seen to belong to one entry and not to a file.
    let mut docs = Docs::default();
    let (first, second) = (docs.open(tab.clone()), docs.open(tab.clone()));
    let on = |id: DocId, document: Document| (id, Stop::whole(document));

    assert!(ask(None, &driven).is_none(), "nothing open asks nothing");
    assert!(
        ask(
            Some(&on(
                first,
                Document::Assembly(Selection::Symbol(symbol.clone()))
            )),
            &driven
        ) == Some(Ask::Symbol(symbol.clone()))
    );
    // An object is a place in a binary but not one with a listing.
    assert!(ask(
        Some(&on(first, Document::Assembly(Selection::Object(object)))),
        &driven
    )
    .is_none());
    // A source-driven tab nothing has been clicked in yet.
    assert!(ask(Some(&on(first, tab.clone())), &driven).is_none());

    driven.remember(on(first, tab.clone()), 42);
    assert!(
        ask(Some(&on(first, tab.clone())), &driven)
            == Some(Ask::Source {
                at: LinePos { file, line: 42 },
                chosen: None
            })
    );
    // And the line belongs to that tab's entry: not to source-driven tabs at large, and
    // not to another tab on the same file.
    assert!(ask(
        Some(&on(first, Document::Source("other.rs".into()))),
        &driven
    )
    .is_none());
    assert!(ask(Some(&on(second, tab)), &driven).is_none());
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

    rect().expanded().child(AssemblyPane {
        tab: pane_tab(&document),
        document,
    })
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
        $runner.provide_root_context(|| Coding(State::create(Coded::default())));
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        $runner.provide_root_context(|| {
            Analysis(State::create(Analyzed {
                shown: Some($shown),
                ..Analyzed::default()
            }))
        });
        // The row's door into the object's code reads these four, and lands through
        // the last two.
        $runner.provide_root_context(|| Sections(State::create(Reading::default())));
        $runner.provide_root_context(|| Window(State::create(None)));
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        $runner.provide_root_context(|| Plant(State::create(None)));
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
        let wash = rects_with(&test, palette().cursor_row_bg);
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
    rect().expanded().child({
        let document = Document::Source(showing.read().clone());
        SourcePane {
            tab: pane_tab(&document),
            document,
        }
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
                runner.provide_root_context(|| Coding(State::create(Coded::default())));
                let showing = runner
                    .provide_root_context(|| Showing(State::create(file.clone())))
                    .0;
                (states, showing)
            }
        },
        1.,
    );
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
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
    open_document(
        states.open,
        states.visits,
        Document::Source(narrow.clone()),
        Reach::NewTab,
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
    assert_eq!(states.visits.peek().recent().count(), 0);
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

    rect().expanded().child(AssemblyPane {
        tab: pane_tab(&document),
        document,
    })
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
    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(sum_to.clone())),
        Reach::NewTab,
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
    let went = |target: Document| open_document(states.open, states.visits, target, Reach::NewTab);
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

    let tab = pane_tab(&document);
    rect()
        .expanded()
        .maybe_child(mounted().then(|| SourcePane { tab, document }.into_element()))
}

/// The line numbers the Source pane's gutter is drawing, which is where the pane is. The
/// number carries the non-breaking space skia is stopped from trimming; the companion
/// header's label is the file's name and parses as nothing.
fn gutter_lines(test: &TestingRunner) -> Vec<u32> {
    let mut rows: Vec<u32> = labels(test)
        .into_iter()
        .filter_map(|text| {
            text.strip_suffix('\u{a0}')
                .and_then(|number| number.parse().ok())
        })
        .collect();
    rows.sort_unstable();
    rows
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
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);

    let land = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
        gutter_lines(test)
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
    src_at.write().remember(entry_of(&states, &document), 120);
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

/// A document's two panes as the strip mounts them: the same [`DocumentBody`] the content
/// area builds, over whichever document is active. The bar itself is left out -- it has no
/// vote in which pane is which -- but the two states the split is held in are not, being
/// what the panels are sized from.
fn panes_harness() -> impl IntoElement {
    let open = use_open();
    // Read and not peeked: this is the harness's whole subscription to a tab being
    // activated, and `Active` is a memo and a beat behind.
    let id = {
        let (strip, docs) = (open.strip.read(), open.docs.read());
        active_document(&strip, &docs).and_then(|document| docs.showing(&document))
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
    let went = |target: Document| open_document(states.open, states.visits, target, Reach::NewTab);

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

/// A tab on a file nothing compiles opens as the source pane alone, where a tab on source
/// a compiler reads keeps its assembly side. Headless because what is asserted is whether
/// the second pane was mounted at all, which only the laid-out tree can say: the analysis
/// is the same listing in both, so a pane that is there draws it.
#[test]
fn a_file_in_no_compiled_language_opens_without_an_assembly_side() {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to.clone()),
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

    // The assembly side draws one 16-digit address per row, so whether any was laid out
    // is whether that pane is there.
    let addresses = |test: &TestingRunner| {
        test.find_many(|node, _element| {
            (node.element().as_ref() as &dyn Any)
                .downcast_ref::<LabelElement>()
                .map(|label| label.text.to_string())
                .filter(|text| text.trim_end().len() == 16)
        })
        .len()
    };
    let went = |file: &str| {
        open_document(
            states.open,
            states.visits,
            Document::Source(Arc::from(file)),
            Reach::NewTab,
        )
    };

    // A language named as compiled, one named as not, and one the app cannot place --
    // which is answered the same as a configuration file, an assembly side being offered
    // only where the extension says the file becomes machine code.
    for (file, assembly) in [
        ("/nowhere/main.rs", true),
        ("/nowhere/Cargo.toml", false),
        ("/nowhere/notes.md", false),
    ] {
        went(file);
        settle(&mut test);
        assert!(
            (addresses(&test) > 0) == assembly,
            "{file} was drawn with {} assembly rows",
            addresses(&test)
        );
        // And the source side is up either way, so what differs is the pane beside it.
        assert!(label_area(&test, &format!("Source file not found: {file}")).is_some());
    }
}

/// The toggle on the leading pane's bar puts the pane the tab is not driven from away and
/// brings it back, the following pane's own bar carries none, and what the reader said
/// holds for that tab alone. Headless because what is asserted is whether the second pane
/// was mounted at all, and where the control that mounts it was laid out.
#[test]
fn the_leading_bar_puts_the_following_pane_away() {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to.clone()),
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

    // The assembly side draws one 16-digit address per row, so whether any was laid out
    // is whether that pane is there.
    let assembly = |test: &TestingRunner| {
        !test
            .find_many(|node, _element| {
                (node.element().as_ref() as &dyn Any)
                    .downcast_ref::<LabelElement>()
                    .map(|label| label.text.to_string())
                    .filter(|text| text.trim_end().len() == 16)
            })
            .is_empty()
    };
    // Where each bar's toggle is: the one square of `toggle_size()` a pane's bar draws,
    // in the order the panes are in. Every wrapper around it takes the same box, so the
    // points are deduplicated by where they are.
    let toggles = |test: &TestingRunner| {
        let mut at: Vec<(f32, f32)> = test.find_many(|node, _element| {
            let area = node.layout().area;
            let square = area.width() == toggle_size() && area.height() == toggle_size();
            square.then(|| {
                (
                    area.origin.x + area.width() / 2.0,
                    area.origin.y + area.height() / 2.0,
                )
            })
        });
        at.sort_by(|a, b| a.0.total_cmp(&b.0));
        at.dedup();
        at
    };
    let went = |file: &str| {
        open_document(
            states.open,
            states.visits,
            Document::Source(Arc::from(file)),
            Reach::NewTab,
        )
    };
    let press = |test: &mut TestingRunner, at: (f32, f32)| {
        test.click_cursor((at.0 as f64, at.1 as f64));
        settle(test);
    };

    went("/nowhere/main.rs");
    settle(&mut test);
    assert!(assembly(&test), "a `.rs` file opens with its assembly side");

    // One toggle with both panes up, not two. That it survives the press is what says it
    // was the leading bar's: a toggle on the assembly side would go away with the pane.
    let one = toggles(&test);
    assert_eq!(one.len(), 1, "only one bar carries a toggle: {one:?}");

    press(&mut test, one[0]);
    assert!(
        !assembly(&test),
        "the leading bar's toggle put nothing away"
    );
    let alone = toggles(&test);
    assert_eq!(alone.len(), 1, "the leading bar kept its toggle: {alone:?}");

    press(&mut test, alone[0]);
    assert!(assembly(&test), "the same toggle did not bring it back");

    // Left in a state the file alone would not open in, for the next two switches.
    let again = toggles(&test)[0];
    press(&mut test, again);
    assert!(!assembly(&test), "the toggle stopped answering");

    // A second tab opens as its own file says, and the first is still as it was left.
    went("/nowhere/other.rs");
    settle(&mut test);
    assert!(
        assembly(&test),
        "the answer followed the reader to another tab"
    );
    went("/nowhere/main.rs");
    settle(&mut test);
    assert!(!assembly(&test), "the tab did not come back as it was left");

    // The mirror: an assembly-driven tab leads with its listing, so the toggle is the
    // symbol bar's and what it puts away is the source side.
    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(sum_to.clone())),
        Reach::NewTab,
    );
    settle(&mut test);
    let source_up = |test: &TestingRunner| {
        label_area(test, "Source file not found: /fixture/line_fixture.c").is_some()
    };
    assert!(
        source_up(&test),
        "an assembly-driven tab opens with its source side"
    );
    let led = toggles(&test);
    assert_eq!(
        led.len(),
        1,
        "only one bar carries a toggle here either: {led:?}"
    );

    press(&mut test, led[0]);
    assert!(
        !source_up(&test),
        "the symbol bar's toggle put nothing away"
    );
    assert!(
        assembly(&test),
        "it put the listing away instead of the source"
    );
    assert_eq!(toggles(&test).len(), 1, "the leading bar kept its toggle");
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
        open_document(
            states.open,
            states.visits,
            Document::Assembly(Selection::Symbol(sum_to.clone())),
            Reach::NewTab,
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

        // The five the source pane has to itself, and the Search panel's mark, which is
        // read on a sidebar row: a disassembly holds no strings, comments, attributes,
        // types or call names, so these are only ever read on `pane_bg`.
        for (name, color) in [
            ("string_fg", palette.string_fg),
            ("comment_fg", palette.comment_fg),
            ("attribute_fg", palette.attribute_fg),
            ("type_fg", palette.type_fg),
            ("function_fg", palette.function_fg),
            ("match_fg", palette.match_fg),
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

        // The one control with a colour of its own, read on that colour: its name and its
        // icon in `icon_fg`, and both in `invalid_fg` when a server would not start.
        // Only those two, since a control that is not running has no colour under it.
        for (name, color) in [
            ("icon_fg", palette.icon_fg),
            ("invalid_fg", palette.invalid_fg),
        ] {
            let ratio = contrast(color, palette.server_bg);
            assert!(ratio >= 3.0, "{theme} {name} on server_bg: {ratio:.2}");
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

        // The source gutter's mark is a drawing as well, and one read as a column rather
        // than a dot at a time: a floor of its own, and quieter than the line number it
        // stands beside, which is read one at a time and has to stay the louder of the
        // two.
        let mark = contrast(palette.compiled_fg, palette.pane_bg);
        let number = contrast(palette.address_fg, palette.pane_bg);
        assert!(mark >= 2.0, "{theme} compiled_fg: {mark:.2}");
        assert!(
            mark < number,
            "{theme} compiled_fg: {mark:.2} vs address_fg {number:.2}"
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
            // Under the file finder's panel, falling on whatever the window was showing:
            // a pane and, where the finder is wider than one, the chrome around it.
            ("panel_shadow", palette.panel_shadow, palette.pane_bg),
            (
                "panel_shadow over the chrome",
                palette.panel_shadow,
                palette.header_bg,
            ),
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
    Delete(String),
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

/// The same wiring under the real pane, for what only the pane can be asked: whether its
/// rows survive one of them being taken away, and what a row's own menu does. The viewer is
/// what `app()` mounts on its root, and opening a menu without one panics.
fn scratchpad_view_harness() -> impl IntoElement {
    scratchpad_wiring();

    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(ScratchpadTab)
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
                PadJob::Delete(name) => Asked::Delete(name.as_str().to_owned()),
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

/// What the code editor is drawing, as one string: every paragraph on screen with its own
/// spans joined, one line of the pad's source per paragraph.
fn drawn_source(test: &TestingRunner) -> String {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    test.find_many(|node, _element| {
        let element = node.element();
        (element.as_ref() as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .map(|paragraph| {
                paragraph
                    .spans
                    .iter()
                    .map(|span| span.text.to_string())
                    .collect::<String>()
            })
    })
    .join("\n")
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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

/// A delete is **asked for**. The row's menu item takes nothing away: it opens the question,
/// and until that question is answered the pad is exactly where it was and the worker has
/// been told nothing. Cancel is the same again. This is the one operation here that destroys
/// the reader's own source, so a menu item that did it outright would be one slip from a pad
/// being gone.
#[test]
fn a_delete_is_asked_for_before_anything_goes() {
    let (mut test, _states, pad, _text, _asking, asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Delete(_) => unreachable!("a pad was deleted without being asked about"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    while asks.try_recv().is_ok() {}

    // The row of the pad that is *not* on screen, since any row can be asked about.
    let two = pad_id("two");
    let row = centre_of(&test, "<two>");
    right_click(&mut test, row);
    let entry = centre_of(&test, "Delete scratchpad");
    test.move_cursor(entry);
    test.press_cursor(entry);
    test.release_cursor(entry);
    settle(&mut test);

    // Asked, and nothing more: the pad is still there and the worker has heard nothing.
    assert_eq!(pad.peek().confirming.as_ref(), Some(&two));
    assert!(pad.peek().get(&two).is_some());
    assert!(asks.is_empty(), "the worker was asked to delete a pad");
    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "Delete <two>?"),
        "the question does not name the pad: {drawn:?}"
    );

    // And no is no.
    let cancel = centre_of(&test, "Cancel");
    test.move_cursor(cancel);
    test.press_cursor(cancel);
    test.release_cursor(cancel);
    settle(&mut test);

    assert!(pad.peek().confirming.is_none());
    assert!(pad.peek().get(&two).is_some());
    assert!(asks.is_empty(), "cancelling deleted the pad");
}

/// A delete takes the whole pad and not only its row: the program it started is killed, the
/// directory it was run in being about to go and a program left behind by that being one
/// nothing could ever find again, and its buffer goes with it. **There is always a pad to draw** -- the next in the order
/// takes over, and when the last one goes the table comes back to the pad a first run
/// holds, which is what keeps `Pads::state` free of an `Option`.
#[test]
fn deleting_a_pad_stops_its_program_and_leaves_a_pad_to_show() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, text, asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            // The directory here is the test's own; what is under test is what the app
            // lets go of when it asks for one of these.
            PadJob::Delete(_) => PadAnswer::Deleted(None),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
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
        });

    pump(&mut test, || pad.peek().state().opened);
    let one = pad.peek().shown().clone();
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || pad.peek().state().output.len() > 0);

    let running = pad.peek().state().run_state.clone();
    let RunState::Going(running) = running else {
        panic!("a going run, got {:?}", pad.peek().state().run_status());
    };
    while asks.try_recv().is_ok() {}

    request_delete_pad(pad, text, &jobs, one.clone());
    pump(&mut test, || pad.peek().state().opened);

    // Really dead, which nothing short of a real process can say.
    pump(&mut test, || running.finished());
    assert!(
        running.finished(),
        "the deleted pad's program is still going"
    );

    // Out of the table, out of the buffers, off the list -- and the pad beside it is what
    // the pane draws now.
    assert_eq!(pad.peek().shown().as_str(), "two");
    assert!(pad.peek().get(&one).is_none());
    assert!(!text.peek().holds(&one));
    assert_eq!(
        pad.peek()
            .order
            .ids()
            .iter()
            .map(PadId::as_str)
            .collect::<Vec<_>>(),
        ["two"]
    );
    assert_eq!(asks.try_recv(), Ok(Asked::Delete("one".to_owned())));
    // The pad taking its place had never been shown, so it is read -- and behind the
    // delete, the jobs being one ordered queue.
    assert_eq!(asks.try_recv(), Ok(Asked::Open("two".to_owned())));

    // The last one. A table with nothing in it would be a pane with nothing to draw, so
    // what comes back is the pad the app boots holding.
    let two = pad_id("two");
    request_delete_pad(pad, text, &jobs, two.clone());
    pump(&mut test, || pad.peek().state().opened);

    assert_eq!(pad.peek().shown().as_str(), crate::scratchpad::DEFAULT_ID);
    assert!(pad.peek().get(&two).is_none());
    assert_eq!(asks.try_recv(), Ok(Asked::Delete("two".to_owned())));
    assert_eq!(
        asks.try_recv(),
        Ok(Asked::Open(crate::scratchpad::DEFAULT_ID.to_owned()))
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// Confirming a delete does not take the editor down with the buffer it lets go of.
///
/// One mouse-up is one batch of events, emitted against the tree freya measured before any
/// of them ran, and the Delete button's press and the editor's own global press are both
/// in it. The press lets go of the shown pad's buffer; the editor is still mounted behind
/// the question, and its handler still writes through a `Writable` mapped through the
/// table by that pad's id. So the index has to answer for a pad that has gone, which is not
/// something the render before it can rule out. Both ways the shown pad goes are here: with
/// a pad behind it, and as the last one.
#[test]
fn confirming_a_delete_does_not_crash_the_editor_it_takes_the_buffer_from() {
    let (mut test, _states, mut pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Delete(_) => PadAnswer::Deleted(None),
            PadJob::Open(scratchpad) => PadAnswer::Opened(pad_on_disk(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    let (one, two) = (pad_id("one"), pad_id("two"));
    assert!(text.peek().holds(&one), "the shown pad has a buffer");
    while asks.try_recv().is_ok() {}

    // The press on the pane's Delete and not `request_delete_pad`, since the press is what
    // is under test. The question is opened by hand: what a row's menu does is pinned by
    // the test above, and a second right-click here would have to wait out the closed
    // popup's fade, whose overlay is still over the row.
    let confirm_delete = |test: &mut TestingRunner| {
        let at = centre_of(test, "Delete");
        press_at(test, at);
        settle(test);
    };

    pad.write().confirming = Some(one.clone());
    settle(&mut test);
    confirm_delete(&mut test);

    assert!(!text.peek().holds(&one));
    assert!(pad.peek().get(&one).is_none());
    assert_eq!(asks.try_recv(), Ok(Asked::Delete("one".to_owned())));

    // The pad behind it, read and drawn -- and then deleted in its turn, this time as the
    // last one, which comes back to the pad a first run holds.
    pump(&mut test, || text.peek().holds(&two));
    assert_eq!(pad.peek().shown(), &two);
    while asks.try_recv().is_ok() {}

    pad.write().confirming = Some(two.clone());
    settle(&mut test);
    confirm_delete(&mut test);

    assert!(!text.peek().holds(&two));
    assert_eq!(pad.peek().shown().as_str(), crate::scratchpad::DEFAULT_ID);
    assert_eq!(asks.try_recv(), Ok(Asked::Delete("two".to_owned())));
}

/// The source of two pads, so a test can tell from the screen which one the editor is
/// drawing. The first is the longer: an editor still drawing it asks for lines the other
/// one does not have.
fn two_sources(scratchpad: Scratchpad) -> Scratchpad {
    let mut opened = scratchpad;
    opened.source = if opened.id().as_str() == "one" {
        "// pad one\nfn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n".to_owned()
    } else {
        "// pad two\n".to_owned()
    };
    opened
}

/// Coming back to a pad whose buffer is already held draws **that** pad's text.
///
/// The editor reaches its buffer through a `Writable` mapped by the pad's id, and freya
/// compares any two `Writable`s as equal, so a `CodeEditor` whose other props have not
/// moved is never handed the new one: the map it keeps is the one it was mounted with.
/// Switching to a pad that has never been read gets away with it, the editor being
/// unmounted while the worker reads the disk and mounted again after; switching to one
/// already read has no such gap, and the editor goes on drawing the pad it was left on.
#[test]
fn coming_back_to_a_pad_already_read_draws_its_own_buffer() {
    let (mut test, _states, pad, text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Delete(_) => unreachable!("no pad is deleted here"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(two_sources(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    let (one, two) = (pad_id("one"), pad_id("two"));
    assert!(drawn_source(&test).contains("// pad one"));

    let row = centre_of(&test, "<two>");
    press_at(&mut test, row);
    pump(&mut test, || text.peek().holds(&two));
    assert!(drawn_source(&test).contains("// pad two"));

    // Back to the first, whose buffer is still held: no gap, and so no remount.
    let row = centre_of(&test, "<one>");
    press_at(&mut test, row);
    settle(&mut test);

    assert_eq!(pad.peek().shown(), &one);
    let drawn = drawn_source(&test);
    assert!(
        drawn.contains("// pad one") && !drawn.contains("// pad two"),
        "the editor is drawing the pad that was left rather than the one shown: {drawn:?}"
    );
}

/// Deleting a pad that is **not** the one on screen does not take the editor down.
///
/// Same map, one step further on. The editor left over from a switch back is drawing a
/// buffer that belongs to another pad, so deleting that other pad takes the buffer out from
/// under a mounted editor. What the pane draws is unchanged, so nothing above the rows is
/// re-rendered; the rows are woken by the write on their own, and each asks a buffer with no
/// lines in it for the line it drew last -- inside freya, where nothing here can catch it.
#[test]
fn deleting_a_pad_that_is_not_shown_leaves_the_editor_standing() {
    let (mut test, _states, mut pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(vec![pad_listing("one"), pad_listing("two"),]),
            PadJob::New => unreachable!("no pad is made here"),
            PadJob::Delete(_) => PadAnswer::Deleted(None),
            PadJob::Open(scratchpad) => PadAnswer::Opened(two_sources(scratchpad)),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: None,
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    let (one, two) = (pad_id("one"), pad_id("two"));

    let row = centre_of(&test, "<two>");
    press_at(&mut test, row);
    pump(&mut test, || text.peek().holds(&two));
    let row = centre_of(&test, "<one>");
    press_at(&mut test, row);
    settle(&mut test);
    while asks.try_recv().is_ok() {}

    // The question is opened by hand rather than through the row's menu, which the delete
    // tests above already pin.
    pad.write().confirming = Some(two.clone());
    settle(&mut test);
    let at = centre_of(&test, "Delete");
    press_at(&mut test, at);
    settle(&mut test);

    assert!(!text.peek().holds(&two));
    assert_eq!(asks.try_recv(), Ok(Asked::Delete("two".to_owned())));

    // The pad on screen is untouched, and so is what the editor draws of it.
    assert_eq!(pad.peek().shown(), &one);
    assert!(text.peek().holds(&one));
    let drawn = drawn_source(&test);
    assert!(
        drawn.contains("// pad one"),
        "the editor lost the pad it was drawing: {drawn:?}"
    );
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            span: Some(cargo::Span {
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            span: Some(cargo::Span {
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
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
        span: Some(cargo::Span {
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
    open_document(states.open, states.visits, document, Reach::NewTab);
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

/// The Source pane's gutter marks every line of the file that produced code, and nothing
/// else: a reader scanning it can tell those from the lines that produced none without
/// picking anything out. The set is the file's own, answered for the whole file, so it is
/// **not** bounded by whatever symbol is drawn and an answer about another file marks
/// nothing here.
#[test]
fn the_gutter_marks_the_lines_that_have_code() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let directory = run_directory(line!());
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("marks.c");
    let text: String = (1..=20).map(|n| format!("int line_{n}(void);\n")).collect();
    std::fs::write(&path, text).expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));

    // The companion the pane draws, and nothing about which of its lines have code: the
    // marks come from the answer written below and not from this.
    let mut studied = Studied::new(sum_to.clone());
    studied.lines.file = Some(file.clone());
    studied.lines.line = Some(5);
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let document = Document::Assembly(Selection::Symbol(sum_to.clone()));
    let (mut test, (states, coded)) = TestingRunner::new(
        source_pane_harness,
        (500., 600.).into(),
        |runner| {
            runner.provide_root_context(|| Mounted(State::create(true)));
            let (states, _marked, _landing) = listing_states!(runner, shown);
            // After the macro, which provides one of its own: a root context is
            // overwritten by whoever writes it last, so this is the one the pane reads.
            let coded = runner
                .provide_root_context(|| Coding(State::create(Coded::default())))
                .0;
            (states, coded)
        },
        1.,
    );
    open_document(states.open, states.visits, document, Reach::NewTab);
    settle(&mut test);
    settle(&mut test);

    let answer = |lines: [u32; 2], of: &Arc<str>| Coded {
        wanted: Some(file.clone()),
        found: Some((of.clone(), Arc::new(HashSet::from(lines)))),
        over: Vec::new(),
    };

    let mut coded = coded;
    coded.set(answer([5, 6], &file));
    settle(&mut test);

    let drawn: Vec<u32> = labels(&test)
        .into_iter()
        .filter_map(|text| {
            text.strip_suffix('\u{a0}')
                .and_then(|number| number.parse().ok())
        })
        .collect();
    assert!(
        drawn.contains(&9),
        "line 9 is not on screen, so an unmarked one proves nothing: {drawn:?}"
    );
    assert_eq!(
        marked_lines(&test),
        vec![5, 6],
        "the gutter marks lines nothing has code for, or misses lines something has"
    );

    // An answer about another file is not this file's: the pane draws no marks rather
    // than another file's, which is what it shows in the beat after moving.
    coded.set(answer([5, 6], &Arc::from("elsewhere.c")));
    settle(&mut test);
    assert!(
        marked_lines(&test).is_empty(),
        "another file's answer marked this one: {:?}",
        marked_lines(&test)
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// The bug the file-wide answer exists for: a **source-driven** tab has no drawn symbol
/// until a line is clicked in it, so a mark bounded by one left the gutter bare until the
/// reader guessed where to click. The marks are there on arrival, and the pane asks for
/// the file it is showing without being clicked.
///
/// `panes_harness` and not `source_pane_harness`, which works its document out of the
/// analysis and so cannot be given a tab that has none.
#[test]
fn a_source_driven_tab_is_marked_before_anything_is_clicked() {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let directory = run_directory(line!());
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("driven.c");
    let text: String = (1..=20).map(|n| format!("int line_{n}(void);\n")).collect();
    std::fs::write(&path, text).expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));

    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied: Studied::new(sum_to.clone()),
    };
    let (mut test, (states, coded)) = TestingRunner::new(
        panes_harness,
        (600., 400.).into(),
        |runner| {
            let (states, _marked, _landing) = listing_states!(runner, shown);
            // No listing at all, which is what a source-driven tab has before a line in
            // it has been clicked. Provided after the macro, which fills one in.
            runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
            let coded = runner
                .provide_root_context(|| Coding(State::create(Coded::default())))
                .0;
            runner.provide_root_context(|| SplitRatio(State::create(50.0)));
            runner.provide_root_context(|| {
                Splits(State::create(ResizableContext {
                    direction: Direction::Horizontal,
                    ..Default::default()
                }))
            });
            (states, coded)
        },
        1.,
    );
    settle(&mut test);

    // A file the reader opened, with nothing clicked in it.
    open_document(
        states.open,
        states.visits,
        Document::Source(file.clone()),
        Reach::NewTab,
    );
    settle(&mut test);
    settle(&mut test);

    // The pane asked, which is what reaches the worker.
    assert_eq!(
        coded.read().wanted.as_deref(),
        Some(&*file),
        "the pane never asked which of its lines have code"
    );

    let mut coded = coded;
    coded.set(Coded {
        wanted: Some(file.clone()),
        found: Some((file.clone(), Arc::new(HashSet::from([3, 4])))),
        over: Vec::new(),
    });
    settle(&mut test);
    assert_eq!(
        marked_lines(&test),
        vec![3, 4],
        "a source-driven tab was left unmarked with nothing clicked in it"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// Which line numbers are drawn beside a gutter mark: the number the mark shares a row
/// with, which is the two of them being level. Compared by their middles rather than by
/// one containing the other, the mark being a dot a fraction of the row's height.
fn marked_lines(test: &TestingRunner) -> Vec<u32> {
    let marks: Vec<Area> = test.find_many(|node, element| {
        (element.style().background == Fill::Color(palette().compiled_fg))
            .then_some(node.layout().area)
    });
    let mut lines: Vec<u32> = labels_with_areas(test)
        .into_iter()
        .filter(|(_, area)| {
            let middle = area.origin.y + area.height() / 2.0;
            marks.iter().any(|mark| {
                let mark_middle = mark.origin.y + mark.height() / 2.0;
                (mark_middle - middle).abs() < code_row_height() / 2.0
            })
        })
        .filter_map(|(text, _)| {
            text.strip_suffix('\u{a0}')
                .and_then(|number| number.parse().ok())
        })
        .collect();
    lines.sort_unstable();
    lines
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
    open_document(states.open, states.visits, tab.clone(), Reach::NewTab);
    driven.write().remember(entry_of(&states, &tab), 7);
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

    open_document(
        states.open,
        states.visits,
        Document::Assembly(Selection::Symbol(symbols[0].clone())),
        Reach::NewTab,
    );
    settle(&mut test);
    assert!(
        location.marked.peek().source.is_none(),
        "the run outlived its tab"
    );

    open_document(states.open, states.visits, tab, Reach::NewTab);
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
        .visits
        .peek()
        .recent()
        .any(|entry| *entry == document));
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
/// `use_land` is mounted because a door into the listing leaves a `Landing` for it, and
/// the caret it plants is what these tests ask about.
fn code_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let open = use_open();
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );

    let reading = use_consume::<Sections>().0;
    let object = reading.read().object.clone();
    match object {
        Some(object) => rect().expanded().child({
            let document = Document::Code(object);
            AssemblyPane {
                tab: pane_tab(&document),
                document,
            }
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
        $runner.provide_root_context(|| Coding(State::create(Coded::default())));
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        $runner.provide_root_context(|| Analysis(State::create(Analyzed::default())));
        let reading = $runner
            .provide_root_context(|| Sections(State::create($reading)))
            .0;
        let window = $runner
            .provide_root_context(|| Window(State::create(None)))
            .0;
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        $runner.provide_root_context(|| Plant(State::create(None)));
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
    for text in ["section .text", "add:", "twice:", "sum_to:"] {
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
    let bottom = label_area(&test, "sum_to:").expect("sum_to is labelled");
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
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);
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
        states.code_at.peek().at(&entry_of(&states, &document)),
        Some(Spot {
            address: 0x30,
            rows: 2
        }),
        "the place is written down as the reader scrolls: two rows past the rule over \
         the stretch, which is the row the address itself finds"
    );

    let before = label_area(&test, "sum_to:").expect("sum_to is labelled");

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
    let after = label_area(&test, "sum_to:").expect("sum_to is still labelled");
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
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    states.code_at.write().remember(
        entry_of(&states, &document),
        Spot {
            address: 0x14,
            rows: 0,
        },
    );
    settle(&mut test);
    settle(&mut test);

    assert_eq!(address_labels(&test)[0], "0000000000000014 ");
    assert!(labels(&test).contains(&"twice:".to_string()));
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
    open_document(states.open, states.visits, document.clone(), Reach::NewTab);
    let entry = entry_of(&states, &document);
    states.code_at.write().remember(
        entry.clone(),
        Spot {
            address: 0x30,
            rows: 2,
        },
    );
    test.sync_and_update();
    assert!(states.code_at.peek().at(&entry).is_some());

    close_document(&states, &document);
    test.sync_and_update();
    assert!(states.code_at.peek().at(&entry).is_none());
    assert!(states.open.active().is_none());
}

/// The Source pane beside an object's code, the reading seeded by the test.
fn code_source_harness() -> impl IntoElement {
    let reading = use_consume::<Sections>().0;
    let object = reading.read().object.clone();
    match object {
        Some(object) => rect().expanded().child({
            let document = Document::Code(object);
            SourcePane {
                tab: pane_tab(&document),
                document,
            }
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

/// A run picked out over an object's code survives an answer landing under it: the rows
/// are counted afresh, and the run is carried to the rows it now has through the address
/// each of its rows stood for -- so the caret on a label stays on the label as the stretch
/// above it turns from a guess into instructions, and the source pane keeps the file the
/// run is of. An ask, which changes no row, changes nothing.
#[test]
fn a_run_survives_the_rows_being_counted_afresh_under_it() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, (states, marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut sections = sections;
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);

    // The caret on `twice`'s label, at its start.
    let label = centre_of(&test, "twice:");
    let at = label_area(&test, "twice:").unwrap();
    test.move_cursor(left_of(&at));
    test.press_cursor(left_of(&at));
    test.release_cursor(left_of(&at));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    let was = picked.chars.lead().row;
    assert!(
        was > 1,
        "the label is below the header and add's rows: {was}"
    );
    let _ = label;

    // An ask: the same state written, no row changed.
    let mut asked = sections.peek().clone();
    asked.pending = Some(CodeAsk {
        object: object.clone(),
        code: asked.code.clone(),
        window: vec![1],
    });
    sections.set(asked);
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead().row,
        was
    );

    // An answer decoding `add`, above the label: its guessed rows become its real ones
    // and the label's row moves; the caret moves with it.
    let mut decoded = reading_of(&object, &[0]);
    decoded.generation = sections.peek().generation + 1;
    sections.set(decoded.clone());
    settle(&mut test);
    settle(&mut test);
    let rows = rows_of(&decoded);
    let now = (0..rows.len())
        .find(|&row| row_line(&rows, &decoded, row) == "0000000000000014 twice:")
        .expect("the label has a row");
    assert_ne!(
        now, was,
        "the fixture's guess for add was exact, proving nothing"
    );
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the rows changed under the run and it went");
    assert_eq!(picked.chars.lead(), Caret { row: now, col: 0 });
    assert_eq!(picked.rows.rows(), now..=now);
}

/// A key moves the view only when the caret leaves it: no context rows, as a click's
/// reveal keeps, since a key repeat that scrolled while the caret was still on screen
/// would walk the rows away from under the reader. A caret stepping above the view
/// brings the view up by that one row.
#[test]
fn a_key_moves_the_view_only_when_the_caret_leaves_it() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let height = code_row_height() as f64;
    // Scrolled down five rows, so there are rows above the view to be brought back.
    test.scroll((300., 150.), (0., -5.0 * height));
    settle(&mut test);
    let before = paragraphs(&test);
    let top = before[0].0;
    assert!(top.origin.y < 60.0, "the view did not scroll: {top:?}");
    let second = before[1].0;

    // The caret on the second row on screen; Up puts it on the first, which is on
    // screen, and the view stays.
    test.move_cursor(left_of(&second));
    test.press_cursor(left_of(&second));
    test.release_cursor(left_of(&second));
    settle(&mut test);
    let row = marked.peek().assembly.clone().unwrap().chars.lead().row;
    test.press_key(Key::Named(NamedKey::ArrowUp));
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead().row,
        row - 1
    );
    let after = paragraphs(&test);
    assert_eq!(
        after[0].0.origin.y, top.origin.y,
        "the view moved with the caret still on screen"
    );
    assert_eq!(after[0].1, before[0].1);

    // Up again: the row above the view, and the view comes up by exactly that row.
    test.press_key(Key::Named(NamedKey::ArrowUp));
    settle(&mut test);
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead().row,
        row - 2
    );
    let after = paragraphs(&test);
    assert_eq!(after[0].0.origin.y, top.origin.y, "{after:?}");
    assert_eq!(
        after[1].1, before[0].1,
        "the view moved by more than the one row"
    );
    let caret = carets(&test);
    assert_eq!(caret.len(), 1);
    assert_eq!(caret[0].origin.y, top.origin.y);
}

/// A caret walked past the pane's edge brings the pane sideways to it: End on a row
/// longer than the pane leaves the caret in sight, the rows scrolled to the left.
#[test]
fn a_caret_walked_past_the_panes_edge_brings_the_pane_sideways_to_it() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (300., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let first = paragraphs(&test)[0].0;
    assert!(
        first.max_x() > 300.0,
        "the row is not wider than the pane: {first:?}"
    );
    let at = (
        (first.origin.x + 2.0) as f64,
        (first.origin.y + first.height() / 2.0) as f64,
    );
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);
    assert_eq!(marked.peek().assembly.clone().unwrap().chars.lead().col, 0);

    test.press_key(Key::Named(NamedKey::End));
    settle(&mut test);
    settle(&mut test);
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert!(
        caret[0].max_x() <= 300.0,
        "the caret is out of sight: {caret:?}"
    );
    let scrolled = paragraphs(&test)[0].0;
    assert!(
        scrolled.origin.x < first.origin.x,
        "the rows did not scroll: {scrolled:?}"
    );
}

/// A run copied out of an object's code spells each kind of row as it is drawn: the
/// section's header, a symbol's label after its address, an instruction as its own tab
/// copies it, and a blank row -- the space over a stretch as much as an undecoded one --
/// as the blank line it is.
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
    assert_eq!(lines[1], "", "the blank under the header");
    assert_eq!(lines[2], "0000000000000000 add:");
    let add = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "add")
        .expect("the fixture holds add");
    let own = add.data.assembly(&add.object).expect("add decodes");
    assert_eq!(lines[3], asm_line(&own.instructions[0], 0));
    // `twice` is not decoded: its label, then blank lines.
    let twice = lines
        .iter()
        .position(|line| line == "0000000000000014 twice:")
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
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);

    // A plain press is a plain press: the tab stays.
    let label = centre_of(&test, "twice:");
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
    assert!(states.visits.peek().recent().any(|entry| *entry == symbol));
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
        .child(AssemblyPane {
            tab: pane_tab(&document),
            document,
        })
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
    open_document(states.open, states.visits, symbol, Reach::NewTab);
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
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, first)),
        Some(Spot {
            address: first,
            rows: 0
        })
    );
    let landed = landing.peek().clone().expect("the line is left to land");
    assert!(landed.tab == code);
    assert!(landed.at == Some(a_line_of(&sum_to)));
    assert_eq!(
        landed.address,
        Some(first),
        "the instruction is left to land"
    );
}

/// Shown among its neighbours while the object's code is already on top, the view
/// scrolls to the instruction and nothing else changes: the place is written and read
/// back by the pane, and the document stays.
#[test]
fn show_in_object_while_the_code_is_on_top_scrolls_without_a_switch() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[]);
    let (mut test, ((states, marked, _sections, _window, landing, _ctrl), plant)) =
        TestingRunner::new(
            code_harness,
            (600., 300.).into(),
            |runner| {
                let states = code_states!(runner, reading);
                // Re-provided, as `Ctrl` is, to be handed to the door.
                let plant = runner.provide_root_context(|| Plant(State::create(None))).0;
                (states, plant)
            },
            1.,
        );
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000000 ");
    let visits = states.visits.peek().recent().count();

    show_in_code(
        states.open,
        states.visits,
        marked,
        landing,
        plant,
        states.code_at,
        object.clone(),
        0x30,
        None,
    );
    settle(&mut test);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000030 ");
    assert!(states.open.active() == Some(code));
    assert_eq!(states.visits.peek().recent().count(), visits);
}

/// An object of the kind a linker leaves -- one `.text` at a real address and no
/// relocations -- holding `f` = `call g+11; ret` and `g` = twelve `mov eax, imm32`s and a
/// `ret`: the call lands inside `g`'s third `mov`, where no symbol starts, so its operand
/// is the number it is and the row keeps the address. Built by hand, since every call in
/// the committed fixtures is relocated and so named. Handed back with the address the call
/// goes to, `g`'s third `mov` being the row at or below it.
fn calling_into_the_middle() -> (Arc<Object>, u64) {
    use analysis::{Architecture, BinaryFormat, ObjectData, Section, SectionIndex, SymbolIndex};

    let target = 6 + 11;
    let mut text = vec![0xE8, (target - 5) as u8, 0x00, 0x00, 0x00, 0xC3];
    for value in 0..12u8 {
        text.extend_from_slice(&[0xB8, value, 0x00, 0x00, 0x00]);
    }
    text.push(0xC3);
    let section = Arc::new(Section {
        index: SectionIndex(1),
        name: ".text".into(),
        data: text.clone(),
        address: 0,
        relocations: HashMap::new(),
        symbols: vec![0, 6],
        unwind: Vec::new(),
        code: true,
        bias: 0,
    });
    let symbols_sorted: Vec<Arc<SymbolData>> = [("f", 0, 6), ("g", 6, text.len() as u64 - 6)]
        .into_iter()
        .map(|(name, address, size)| {
            Arc::new(SymbolData {
                name: name.to_owned(),
                demangled: None,
                address,
                section: Some(section.clone()),
                size,
            })
        })
        .collect();
    let symbols = symbols_sorted
        .iter()
        .enumerate()
        .map(|(index, symbol)| (SymbolIndex(index), symbol.clone()))
        .collect();
    let object = Arc::new(Object {
        path: PathBuf::from("/linked/image"),
        name: "image".to_owned(),
        format: BinaryFormat::Elf,
        architecture: Architecture::X86_64,
        symbols,
        symbols_sorted,
        sections: vec![section],
        data: ObjectData::from(text),
        debug_info: Default::default(),
        by_address: Default::default(),
    });
    (object, target)
}

/// The operand of `f`'s call as the disassembler printed it, which is the text of the
/// door: the span `target_span` names.
fn call_operand(f: &Symbol) -> String {
    let assembly = f.data.assembly(&f.object).expect("f decodes");
    let call = &assembly.instructions[0];
    assert!(call.relocation.is_none(), "the call was named");
    let span = call.target_span.expect("the call keeps its address");
    call.format[span].0.clone()
}

/// The address a call with no symbol at its target goes to is a door into the object's
/// code, opened with **Ctrl** as a label's door is: a plain press on the operand picks the
/// row out and opens nothing, and with Ctrl held it opens the object's code tab with its
/// place set on the target's address -- and no line, the target's row not being this one.
#[test]
fn a_call_with_no_symbol_opens_the_code_at_its_target_with_ctrl() {
    let (object, target) = calling_into_the_middle();
    let f = Symbol {
        object: object.clone(),
        data: object.symbols_sorted[0].clone(),
    };
    assert_eq!(f.data.name, "f");
    let operand = call_operand(&f);
    let shown = Shown {
        ask: Ask::Symbol(f.clone()),
        studied: Studied::new(f.clone()),
    };
    let (mut test, ((states, marked, landing), ctrl)) = TestingRunner::new(
        listing_harness,
        (600., 400.).into(),
        |runner| {
            let states = listing_states!(runner, shown);
            // Re-provided, as `code_states!` does, to be driven from the test.
            let ctrl = runner.provide_root_context(|| Ctrl(State::create(false))).0;
            (states, ctrl)
        },
        1.,
    );
    let mut ctrl = ctrl;
    let symbol = Document::Assembly(Selection::Symbol(f.clone()));
    open_document(states.open, states.visits, symbol.clone(), Reach::NewTab);
    settle(&mut test);

    // A plain press is a plain press: the row is picked out and nothing opens.
    let door = centre_of(&test, &operand);
    press_at(&mut test, door);
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), 0..=0);
    let code = Document::Code(object.clone());
    assert!(states.open.active() == Some(symbol.clone()));
    assert!(tab_showing(&states, &code).is_none(), "the code tab opened");

    // With Ctrl held it is the door.
    ctrl.set(true);
    settle(&mut test);
    press_at(&mut test, door);
    settle(&mut test);
    assert!(
        states.open.active() == Some(code.clone()),
        "the code tab is not on top"
    );
    assert!(
        tab_showing(&states, &symbol).is_some(),
        "the symbol's tab was replaced rather than kept beside"
    );
    assert_eq!(
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, target)),
        Some(Spot {
            address: target,
            rows: 0
        })
    );
    // The instruction is left to land, and no line: the target's row is not this one.
    let landed = landing.peek().clone().expect("the target is left to land");
    assert!(landed.at.is_none(), "a line was left to land");
    assert_eq!(landed.address, Some(target));
}

/// A symbol named in an operand of the unified view is a place further down that same
/// listing: a plain press moves to it -- the caret on its first row, the place written
/// down, no tab opened and nothing recorded -- and Ctrl still opens the symbol alone in a
/// tab of its own, as Ctrl does everywhere.
#[test]
fn a_link_in_the_unified_view_moves_the_listing_and_opens_no_tab() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[0, 1, 2]);
    let rows = rows_of(&reading);
    let (mut test, (states, marked, _sections, _window, _landing, ctrl)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let mut ctrl = ctrl;
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);
    let add = object
        .symbols_sorted
        .iter()
        .find(|data| data.name == "add")
        .expect("the fixture holds add")
        .clone();
    let visited = states.visits.peek().entries().len();

    // The operand naming `add`, which `sum_to` calls: the label row over it reads
    // `add:` and is a different string.
    // The link is a `label()` of its own; the `add` mnemonic further up is a span of a
    // row's paragraph, which is why this asks for labels alone.
    let (_, area) = labels_with_areas(&test)
        .into_iter()
        .find(|(text, _)| text == "add")
        .expect("the link is drawn");
    let link = (
        (area.origin.x + area.width() / 2.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    );
    press_at(&mut test, link);
    settle(&mut test);
    settle(&mut test);
    let symbol = Document::Assembly(Selection::Symbol(Symbol {
        object: object.clone(),
        data: add.clone(),
    }));
    assert!(
        states.open.active() == Some(code.clone()),
        "the listing was left"
    );
    assert!(
        tab_showing(&states, &symbol).is_none(),
        "a tab opened for the symbol"
    );
    assert_eq!(
        states.visits.peek().entries().len(),
        visited,
        "moving inside the listing was recorded as a visit"
    );

    // The caret is on the target's first row, and the place kept is its address.
    let landed = rows.body_row_for(add.address).expect("add has a row");
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the caret was not planted");
    assert_eq!(picked.rows.rows(), landed..=landed);
    assert_eq!(
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, add.address)),
        Some(Spot {
            address: add.address,
            rows: 0
        })
    );

    // With Ctrl held it is the other door: the symbol alone, beside the listing.
    ctrl.set(true);
    settle(&mut test);
    press_at(&mut test, link);
    settle(&mut test);
    assert!(
        states.open.active() == Some(symbol.clone()),
        "Ctrl did not open the symbol"
    );
    assert!(
        tab_showing(&states, &code).is_some(),
        "the listing was replaced rather than kept beside"
    );
}

/// One function is told from the next by a rule, the way one basic block is told from the
/// block above it: the row over a stretch carries it, so a symbol's label is never drawn
/// against the last row of the function before it.
#[test]
fn a_rule_is_drawn_over_every_symbol_in_the_unified_view() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[0, 1, 2]);
    let (mut test, _) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    settle(&mut test);

    let rules = rects_with(&test, palette().block_rule);
    assert!(!rules.is_empty(), "no rule is drawn");
    // Every label but the first has one in the row above it; `add` is the listing's first
    // stretch and opens it, so it has none.
    for (name, over) in [("add:", false), ("twice:", true), ("sum_to:", true)] {
        let label = label_area(&test, name).unwrap_or_else(|| panic!("{name} is drawn"));
        // Two rows over the name: the rule, then the blank that keeps it off the name.
        let above = rules.iter().any(|rule| {
            let gap = label.origin.y - rule.origin.y;
            gap > code_row_height() && gap <= 2.0 * code_row_height()
        });
        assert_eq!(above, over, "{name}");
    }
}

/// A place inside an object's code is named by the symbol at that address, so the
/// chevrons say which function a step goes to rather than the object's name twice over.
#[test]
fn a_place_in_a_listing_is_named_by_the_symbol_there() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let code = Document::Code(object.clone());
    let twice = object
        .symbols_sorted
        .iter()
        .find(|data| data.name == "twice")
        .expect("the fixture holds twice")
        .clone();

    // A stop's address is the one the listing draws: the symbol's own plus where the
    // layout put its section.
    let placed = twice
        .address
        .wrapping_add(twice.section.as_ref().map_or(0, |section| section.bias));

    assert_eq!(stop_text(&Stop::whole(code.clone())), object.name);
    assert_eq!(stop_text(&Stop::at(code.clone(), placed)), "twice");
    // A place no symbol starts at is the object again: a call to a function lands on its
    // first byte and is named, and an address into the middle of one has no name to give.
    assert_eq!(stop_text(&Stop::at(code.clone(), placed + 1)), object.name);
    assert_eq!(stop_text(&Stop::at(code, u64::MAX)), object.name);
}

/// Following a link inside an object's code is a place on the tab's trail, so Back comes
/// back to the instruction that was followed and not to where the jump landed. The two
/// places are two entries, each keeping its own row -- which is the whole of why the
/// positions are kept per place and not per document.
#[test]
fn back_returns_to_the_place_a_link_was_followed_from() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[0, 1, 2]);
    let rows = rows_of(&reading);
    let decoded = reading_of(&object, &[0, 1, 2]);
    let (mut test, (states, _marked, _sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 10.0 * code_row_height()).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let code = Document::Code(object.clone());
    let id = open_document(states.open, states.visits, code.clone(), Reach::NewTab)
        .expect("a document panel");
    settle(&mut test);
    let add = object
        .symbols_sorted
        .iter()
        .find(|data| data.name == "add")
        .expect("the fixture holds add")
        .clone();

    // Scroll to the call itself, which is the place being left: the row of `sum_to`'s
    // one instruction carrying a relocation.
    let call = (0..rows.len())
        .find(|&row| row_line(&rows, &decoded, row).contains("call"))
        .expect("sum_to calls add");
    test.scroll(
        (300., 150.),
        (
            0.,
            -(call.saturating_sub(2) as f64) * code_row_height() as f64,
        ),
    );
    settle(&mut test);
    let left = states
        .code_at
        .peek()
        .at(&entry_of(&states, &code))
        .expect("the place the reader is at is written down");
    let shown = address_labels(&test);
    assert!(!shown.is_empty(), "the pane is drawing nothing");

    // Follow the call.
    let (_, area) = labels_with_areas(&test)
        .into_iter()
        .find(|(text, _)| text == "add")
        .expect("the link is drawn");
    press_at(
        &mut test,
        (
            (area.origin.x + area.width() / 2.0) as f64,
            (area.origin.y + area.height() / 2.0) as f64,
        ),
    );
    settle(&mut test);
    settle(&mut test);
    let trail: Vec<Stop> = states
        .open
        .docs
        .peek()
        .trail(id)
        .expect("open")
        .entries()
        .to_vec();
    assert!(
        trail
            == [
                Stop::whole(code.clone()),
                Stop::at(code.clone(), add.address)
            ],
        "the place followed is not on the trail"
    );
    assert_ne!(
        address_labels(&test),
        shown,
        "the press did not move the listing to the target"
    );

    // Back: the tab is on the place it was, with the rows it was left at.
    navigate(states.open, Nav::Back);
    settle(&mut test);
    settle(&mut test);
    assert!(
        states.open.active_stop().map(|(_, stop)| stop) == Some(Stop::whole(code.clone())),
        "the step did not go back"
    );
    assert_eq!(
        states.code_at.peek().at(&entry_of(&states, &code)),
        Some(left),
        "the place left was not kept under its own entry"
    );
    assert_eq!(
        address_labels(&test),
        shown,
        "Back did not come back to the rows the place was left at"
    );
    // And the place jumped to keeps its own row, for Forward to come back to.
    assert_eq!(
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, add.address)),
        Some(Spot {
            address: add.address,
            rows: 0
        })
    );
}

/// A call target no symbol names is a place in the unified view as much as a named one
/// is: a plain press on the bare address moves the listing there, where in a symbol's own
/// listing the same press picks the row out and it takes Ctrl to open the door.
#[test]
fn a_bare_target_in_the_unified_view_moves_on_a_plain_press() {
    let (object, target) = calling_into_the_middle();
    let f = Symbol {
        object: object.clone(),
        data: object.symbols_sorted[0].clone(),
    };
    let operand = call_operand(&f);
    let reading = reading_of(&object, &[0]);
    let (mut test, (states, marked, _sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 6.0 * code_row_height()).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);

    let door = centre_of(&test, &operand);
    press_at(&mut test, door);
    settle(&mut test);
    settle(&mut test);
    assert!(
        states.open.active() == Some(code.clone()),
        "the listing was left"
    );
    assert_eq!(
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, target)),
        Some(Spot {
            address: target,
            rows: 0
        }),
        "the listing did not move to the target"
    );
    assert!(
        marked.peek().assembly.is_some(),
        "the caret was not planted on the target's row"
    );
}

/// The code opened at a call's target lands on the row **at or below** the address: on
/// the guessed row of the stretch while nothing there is decoded, and on the instruction
/// holding the byte once it is -- the exact place the door asked for, not the row the
/// guess was nearest. The door is offered in the unified view's own rows too, where the
/// target may be screens away.
#[test]
fn the_code_opened_at_a_target_lands_on_the_row_at_or_below_it() {
    let (object, target) = calling_into_the_middle();
    let f = Symbol {
        object: object.clone(),
        data: object.symbols_sorted[0].clone(),
    };
    let operand = call_operand(&f);
    let g = object.symbols_sorted[1].clone();
    let holding = g
        .assembly(&object)
        .expect("g decodes")
        .instructions
        .iter()
        .map(|instruction| instruction.address)
        .filter(|&address| address <= target)
        .last()
        .expect("an instruction holds the target");
    assert!(holding < target, "the target is an instruction's own start");

    // `f` decoded and `g` not, so the target's row is a guess.
    let reading = reading_of(&object, &[0]);
    let guessed = rows_of(&reading);
    let (mut test, (states, marked, sections, _window, _landing, ctrl)) = TestingRunner::new(
        code_harness,
        (600., 6.0 * code_row_height()).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let (mut ctrl, mut sections) = (ctrl, sections);
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], "0000000000000000 ");

    ctrl.set(true);
    settle(&mut test);
    let door = centre_of(&test, &operand);
    press_at(&mut test, door);
    settle(&mut test);
    settle(&mut test);
    assert!(
        states.open.active() == Some(code.clone()),
        "the tab changed"
    );
    assert_eq!(
        states
            .code_at
            .peek()
            .at(&code_entry_of(&states, &code, target)),
        Some(Spot {
            address: target,
            rows: 0
        })
    );
    // The view is on the guessed row: `f`'s rows have scrolled off, and the top row is
    // the one the guess puts the address in, which is nobody's address.
    let guess = guessed.row_for(target).expect("the target has a row");
    assert!(matches!(guessed.row(guess), Some(Row::Empty { .. })));
    assert!(
        !labels(&test).iter().any(|text| text == "f:"),
        "the view did not move: {:?}",
        labels(&test)
    );
    assert!(
        address_labels(&test).is_empty(),
        "{:?}",
        address_labels(&test)
    );
    // And the caret is on that row, the door being a caret as well as a scroll: planted
    // by the pane, the door having been opened while the tab was on top, and owing
    // the pane nothing beside the place it wrote.
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the caret was not planted on the target's row");
    assert_eq!(picked.chars.lead(), Caret { row: guess, col: 0 });
    assert_eq!(picked.rows.rows(), guess..=guess);
    assert!(picked.owed == Owed::default());

    // `g` decodes, and the view is on the instruction holding the byte -- the caret
    // with it, on the row that is now the instruction's.
    let mut decoded = reading_of(&object, &[0, 1]);
    decoded.generation = sections.peek().generation + 1;
    let exact = rows_of(&decoded)
        .row_for(target)
        .expect("the target has a row");
    sections.set(decoded);
    settle(&mut test);
    settle(&mut test);
    assert_eq!(address_labels(&test)[0], format!("{holding:016X} "));
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the caret went with the decode");
    assert_eq!(picked.chars.lead(), Caret { row: exact, col: 0 });
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "one caret is drawn");
    let top = paragraphs(&test)[0].0;
    assert!(
        caret[0].origin.y >= top.origin.y - 1.0 && caret[0].origin.y < top.max_y(),
        "the caret is not drawn on the top row: {:?} against {top:?}",
        caret[0]
    );
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
            {
                let document = Document::Code(object);
                AssemblyPane {
                    tab: pane_tab(&document),
                    document,
                }
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
    open_document(states.open, states.visits, code, Reach::NewTab);
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
    assert!(landed.at.as_ref().map(|at| &at.file) == Some(&a_line_of(&twice).file));
    // The symbol's own address: the fixture places its one `.text` at 0, so the one
    // drawn is the one the listing alone will draw.
    assert_eq!(landed.address, Some(twice.data.address + 1));
}

/// The Assembly pane over the active document, whichever kind it is, with what the app
/// puts at the root for a door: `use_land`, so a landing is spent by the arrival, and
/// `use_reading_of`, so an object's code has a reading to draw rows from. The analysis
/// and the reading are the test's to set, standing in for the worker; the pane follows
/// the tab through the dock, read and not peeked, as `panes_harness` does.
fn doors_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let open = use_open();
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    let objects = use_consume::<Objects>().0;
    let reading = use_consume::<Sections>().0;
    let window = use_consume::<Window>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    use_reading_of(active, objects, reading, window);

    let entry = {
        let (strip, docs) = (open.strip.read(), open.docs.read());
        active_document(&strip, &docs)
            .and_then(|document| Some((docs.showing(&document)?, document)))
    };
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .maybe_child(entry.map(|(tab, document)| AssemblyPane { tab, document }.into_element()))
}

/// The contexts [`doors_harness`] reads beside the project's.
#[derive(Clone, Copy)]
struct DoorStates {
    marked: State<Marks>,
    landing: State<Option<Landing>>,
    plant: State<Option<Planting>>,
    analysis: State<Analyzed>,
    sections: State<Reading>,
}

macro_rules! door_states {
    ($runner:expr) => {{
        let states = project_states!($runner);
        let marked = $runner
            .provide_root_context(|| Marked(State::create(Marks::default())))
            .0;
        $runner.provide_root_context(|| Shift(State::create(false)));
        $runner.provide_root_context(|| Locations(State::create(Located::default())));
        $runner.provide_root_context(|| Coding(State::create(Coded::default())));
        $runner.provide_root_context(|| CodeRows(State::create(None)));
        let analysis = $runner
            .provide_root_context(|| Analysis(State::create(Analyzed::default())))
            .0;
        let sections = $runner
            .provide_root_context(|| Sections(State::create(Reading::default())))
            .0;
        $runner.provide_root_context(|| Window(State::create(None)));
        let landing = $runner.provide_root_context(|| Land(State::create(None))).0;
        let plant = $runner
            .provide_root_context(|| Plant(State::create(None)))
            .0;
        (
            states,
            DoorStates {
                marked,
                landing,
                plant,
                analysis,
                sections,
            },
        )
    }};
}

/// "Show in unified view" puts the assembly pane's caret on the instruction and not
/// only the view: the code tab arrives with no rows and the caret waits; once the
/// skeleton has come it is on the row at or below the address, a guessed row; and once
/// the stretch decodes it is on the instruction itself -- the address the door named
/// being what is kept for the caret's row, and not the guessed row's own share of its
/// stretch, which would have put it on the row nearest the guess. The pane owes it no
/// scroll: the tab's place is the same address and is what brings the row to the top.
/// Headless because the caret is planted by the section view's own effect a pass after
/// `use_land` has spent the landing, and moved by the rebuild an answer wakes.
#[test]
fn show_in_unified_view_puts_the_caret_on_the_instruction_once_it_has_a_row() {
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
    let studied = Studied::new(sum_to.clone());
    let assembly = studied.assembly.clone().expect("sum_to decodes");
    let bias = sum_to
        .data
        .section
        .as_ref()
        .map_or(0, |section| section.bias);
    // An instruction whose guessed row is not its row, so the move can be seen: the
    // two stretches above decoded either way, so only `sum_to`'s own guess differs, and
    // not its first instruction, whose address is the label's and lands the view on
    // the label over it.
    let before = reading_of(&object, &[0, 1]);
    let guessed = rows_of(&before);
    let decoded = reading_of(&object, &[0, 1, 2]);
    let exact = rows_of(&decoded);
    let (index, address) = (1..assembly.instructions.len())
        .map(|index| {
            (
                index,
                assembly.instructions[index].address.wrapping_add(bias),
            )
        })
        .find(|&(_, address)| guessed.body_row_for(address) != exact.body_row_for(address))
        .expect("every guess was exact, proving nothing");
    let guess = guessed
        .body_row_for(address)
        .expect("the address has a row");
    let row = exact.body_row_for(address).expect("the address has a row");
    assert!(matches!(guessed.row(guess), Some(Row::Empty { .. })));
    assert!(matches!(exact.row(row), Some(Row::Instruction { index: at, .. }) if at == index));

    let (mut test, (states, doors)) = TestingRunner::new(
        doors_harness,
        (600., 5.0 * code_row_height()).into(),
        |runner| door_states!(runner),
        1.,
    );
    let mut open = states.objects;
    open.write().push(object.clone());
    settle(&mut test);

    show_in_code(
        states.open,
        states.visits,
        doors.marked,
        doors.landing,
        doors.plant,
        states.code_at,
        object.clone(),
        address,
        studied.position(index),
    );
    settle(&mut test);
    settle(&mut test);
    let code = Document::Code(object.clone());
    assert!(states.open.active() == Some(code.clone()));
    assert!(
        doors.sections.peek().is_about(&object),
        "the reading did not follow the tab"
    );
    // No rows yet: the instruction waits for them, and no caret is planted in nothing.
    assert!(doors.marked.peek().assembly.is_none());
    let planting = doors
        .plant
        .peek()
        .clone()
        .expect("the instruction was not left for the rows");
    assert!(planting.tab == code && planting.address == address);

    // The worker's first answer, `sum_to` still a guess: the caret on the guessed row,
    // the planting spent.
    let mut sections = doors.sections;
    sections.set(before);
    settle(&mut test);
    settle(&mut test);
    let picked = doors
        .marked
        .peek()
        .assembly
        .clone()
        .expect("the caret was not planted once there were rows");
    assert_eq!(picked.chars.lead(), Caret { row: guess, col: 0 });
    assert_eq!(picked.rows.rows(), guess..=guess);
    assert!(
        picked.owed == Owed::default(),
        "a scroll is owed beside the place"
    );
    assert!(doors.plant.peek().is_none(), "the planting was left lying");

    // The stretch decodes: the caret on the instruction itself, and the view there.
    let mut decoded = decoded;
    decoded.generation = sections.peek().generation + 1;
    sections.set(decoded);
    settle(&mut test);
    settle(&mut test);
    let picked = doors
        .marked
        .peek()
        .assembly
        .clone()
        .expect("the caret went with the decode");
    assert_eq!(picked.chars.lead(), Caret { row, col: 0 });
    assert_eq!(picked.rows.rows(), row..=row);
    assert_eq!(address_labels(&test)[0], format!("{address:016X} "));
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "one caret is drawn");
    let top = paragraphs(&test)[0].0;
    assert!(
        caret[0].origin.y >= top.origin.y - 1.0 && caret[0].origin.y < top.max_y(),
        "the caret is not drawn on the top row: {:?} against {top:?}",
        caret[0]
    );
}

/// "Open as symbol" puts the caret on the instruction the reader was on, in the symbol's
/// own listing -- which comes from the worker after the tab does, so the caret is
/// planted when that listing is drawn and not when the tab opens: nothing while the
/// pane still says the worker has answered nothing, and the caret at the instruction's
/// row once the answer is drawn, the pane owing it the reveal and the line's run no
/// longer owing the pair's. Headless because the plant is the listing's own effect,
/// woken by the answer, and only the runner can say it waited for it.
#[test]
fn open_as_symbol_puts_the_caret_on_the_instruction_once_the_listing_is_drawn() {
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
    let studied = Studied::new(twice.clone());
    let assembly = studied.assembly.clone().expect("twice decodes");
    let bias = twice
        .data
        .section
        .as_ref()
        .map_or(0, |section| section.bias);
    // The third instruction: the first shares its address text with the label over it.
    let index = 2;
    let address = assembly.instructions[index].address;
    let row = studied.lanes.row_of(index);

    let (mut test, (states, doors)) = TestingRunner::new(
        doors_harness,
        (600., 900.).into(),
        |runner| door_states!(runner),
        1.,
    );
    let mut open = states.objects;
    open.write().push(object.clone());
    settle(&mut test);
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);
    settle(&mut test);
    assert!(doors.sections.peek().is_about(&object));
    let mut sections = doors.sections;
    sections.set(reading_of(&object, &[1]));
    settle(&mut test);
    settle(&mut test);

    let drawn = format!("{:016X} ", address.wrapping_add(bias));
    let at = centre_of(&test, &drawn);
    right_click(&mut test, at);
    let item = centre_of(&test, "Open as symbol");
    press_at(&mut test, item);
    settle(&mut test);
    settle(&mut test);
    let symbol = Document::Assembly(Selection::Symbol(twice.clone()));
    assert!(states.open.active() == Some(symbol.clone()));
    // The tab is up and its listing is not: the caret waits for the listing.
    assert!(
        doors.marked.peek().assembly.is_none(),
        "a caret was planted in a listing that is not drawn"
    );
    let planting = doors
        .plant
        .peek()
        .clone()
        .expect("the instruction was not left for the listing");
    assert!(planting.tab == symbol && planting.address == address);

    // The worker's answer: the listing is drawn, and the caret is on the row.
    let mut analysis = doors.analysis;
    analysis.set(Analyzed {
        shown: Some(Shown {
            ask: Ask::Symbol(twice.clone()),
            studied: studied.clone(),
        }),
        ..Analyzed::default()
    });
    settle(&mut test);
    settle(&mut test);
    let (assembly, source) = runs_of(doors.marked);
    let picked = assembly.expect("the caret was not planted once the listing was drawn");
    assert_eq!(picked.chars.lead(), Caret { row, col: 0 });
    assert_eq!(picked.rows.rows(), row..=row);
    assert!(doors.plant.peek().is_none(), "the planting was left lying");
    let source = source.expect("the row's line was not landed");
    assert!(
        !source.owed.assembly,
        "the pane still owes the line's pair beside the caret's own reveal"
    );
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "one caret is drawn");
    let rows = paragraphs(&test);
    assert!(
        caret[0].origin.y >= rows[row].0.origin.y - 1.0 && caret[0].origin.y < rows[row].0.max_y(),
        "the caret is not drawn on row {row}: {:?} against {:?}",
        caret[0],
        rows[row].0
    );
}

/// A landing's instruction is spent by whichever document arrives, as its line is: left
/// for a symbol whose listing never came and spent by the next arrival, it plants nothing
/// in that symbol's listing when the listing does come. Headless because the spending is
/// `use_land`'s and the planting the listing's own effect, and only the runner has both.
#[test]
fn a_landings_instruction_is_spent_by_whichever_document_arrives() {
    let symbols = fixture_symbols();
    let (first, second) = (symbols[0].clone(), symbols[1].clone());
    let studied = Studied::new(first.clone());
    let (mut test, (states, doors)) = TestingRunner::new(
        doors_harness,
        (600., 400.).into(),
        |runner| door_states!(runner),
        1.,
    );
    settle(&mut test);

    let first_tab = Document::Assembly(Selection::Symbol(first.clone()));
    let mut landing = doors.landing;
    landing.set(Some(Landing {
        tab: first_tab.clone(),
        at: None,
        address: Some(first.data.address),
        columns: None,
    }));
    open_document(states.open, states.visits, first_tab.clone(), Reach::NewTab);
    settle(&mut test);
    settle(&mut test);
    // Arrived, with no listing to plant it in: left for the listing.
    assert!(doors.landing.peek().is_none(), "the landing was not spent");
    let planting = doors
        .plant
        .peek()
        .clone()
        .expect("the instruction was not left for the listing");
    assert!(planting.tab == first_tab);

    // Another document arrives: spent.
    let second_tab = Document::Assembly(Selection::Symbol(second));
    open_document(states.open, states.visits, second_tab, Reach::NewTab);
    settle(&mut test);
    settle(&mut test);
    assert!(doors.plant.peek().is_none(), "a landing was left lying");

    // The first symbol's listing comes, and its tab is raised: nothing is planted.
    let mut analysis = doors.analysis;
    analysis.set(Analyzed {
        shown: Some(Shown {
            ask: Ask::Symbol(first),
            studied,
        }),
        ..Analyzed::default()
    });
    raise_document(&states, &first_tab);
    settle(&mut test);
    settle(&mut test);
    assert!(states.open.active() == Some(first_tab));
    assert!(
        doors.marked.peek().assembly.is_none(),
        "a spent landing planted a caret"
    );
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
    rect().expanded().child({
        let document = Document::Code(object);
        AssemblyPane {
            tab: pane_tab(&document),
            document,
        }
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
    open_document(
        states.open,
        states.visits,
        Document::Code(object.clone()),
        Reach::NewTab,
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
            let alt = runner.provide_root_context(|| Alt(State::create(false))).0;
            (ModifierKeys::new(shift, ctrl, alt, caps, held), ctrl)
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
        .child(BookmarksPanel)
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
        .visits
        .peek()
        .recent()
        .any(|entry| *entry == document));
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
        .child(SymbolsPanel)
}

/// The History list, the same way.
fn history_menu_harness() -> impl IntoElement {
    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .child(HistoryPanel)
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
    open_document(states.open, states.visits, file.clone(), Reach::NewTab);
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

/// A document's own chip with the context-menu viewer a right-click on it needs. It
/// takes its document the way `close_harness` does, from the strip's first document tab.
fn header_menu_harness() -> impl IntoElement {
    let open = use_open();
    let id = open.strip.read().documents().next();

    rect()
        .expanded()
        .child(ContextMenuViewer::new())
        .maybe_child(id.map(|id| {
            TabHeader {
                tab: Tab::Document(id),
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
    open_document(states.open, states.visits, first.clone(), Reach::NewTab);
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

    open_document(states.open, states.visits, second, Reach::NewTab);
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
    open_document(states.open, states.visits, symbol.clone(), Reach::NewTab);
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
        .child(FilesPanel)
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
        .visits
        .peek()
        .recent()
        .any(|entry| *entry == document));
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

/// A directory's row has a menu of its own, and its one item is the file manager's: a
/// folder is as showable as a file, and there is no object inside one to open.
#[test]
fn a_directory_row_offers_the_file_manager_alone() {
    let (mut test, _states, directory) = files_over(line!());
    std::fs::create_dir_all(directory.join("a")).expect("creating the test directory");
    press(&mut test, "project");
    press(&mut test, "project");

    let row = centre_of(&test, "a");
    right_click(&mut test, row);
    settle(&mut test);

    assert!(label_area(&test, "Show in file manager").is_some());
    assert!(label_area(&test, "Open file").is_none());
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

/// A language named for what it compiles to, with no grammar behind it, opens with an
/// assembly side and renders plain. The two questions are separate on purpose: naming a
/// language costs an arm, a grammar costs a dependency.
#[test]
fn a_compiled_language_with_no_grammar_is_still_compiled() {
    for named in ["shader.zig", "server.go", "start.S", "view.mm", "kernel.cu"] {
        let path = Path::new(named);
        assert!(source::compiled(path), "{named} has no assembly side");
        assert!(language(path).is_none(), "{named} claimed a grammar");
    }
    // And the three the app does colour still have theirs.
    for named in ["main.rs", "sum.c", "sum.hpp"] {
        let path = Path::new(named);
        assert!(source::compiled(path), "{named}");
        assert!(language(path).is_some(), "{named} lost its grammar");
    }
}

/// Every paragraph on screen, top to bottom: its box, its text -- the spans joined, an
/// inline child counting for nothing here -- and the highlight its row drew for the
/// character selection, which is the rect in the selection's colour level with it and
/// starting at or after its left edge (a row selected whole starts at the row's).
fn paragraphs(test: &TestingRunner) -> Vec<(Area, String, Option<Area>)> {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    let washes = rects_with(test, palette().text_select_bg);
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
                let area = node.layout().area;
                let highlight = washes
                    .iter()
                    .find(|wash| {
                        (wash.origin.y - area.origin.y).abs() < 1.0
                            && wash.min_x() >= area.min_x() - 1.0
                    })
                    .copied();
                (area, text, highlight)
            })
    });
    found.sort_by(|a, b| a.0.origin.y.total_cmp(&b.0.origin.y));
    found
}

/// Whether `highlight` runs from `from` to `to`, to the pixel either way: the row puts it
/// on the device grid, so an edge may be a pixel off the text's own.
fn spans(highlight: Option<Area>, from: f32, to: f32) -> bool {
    highlight.is_some_and(|h| (h.min_x() - from).abs() <= 1.0 && (h.max_x() - to).abs() <= 1.0)
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
    let (first, _, _) = drawn[0].clone();
    let (second, _, _) = drawn[1].clone();

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
    let chars = picked.chars;
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
    assert!(paragraphs(&test).iter().all(|(_, _, h)| h.is_none()));

    // The sweep, to the end of the row below.
    test.move_cursor(right_of(&second));
    settle(&mut test);
    let drawn = paragraphs(&test);
    assert!(spans(drawn[0].2, first.min_x(), first.max_x()), "{drawn:?}");
    assert!(
        spans(drawn[1].2, second.min_x(), second.max_x()),
        "{drawn:?}"
    );
    assert!(drawn[2..].iter().all(|(_, _, h)| h.is_none()));
    // And to the pixel between the rows: each highlight is the row's whole height, so
    // the two meet on an edge.
    let (top, below) = (drawn[0].2.unwrap(), drawn[1].2.unwrap());
    assert_eq!(top.max_y(), below.min_y(), "a gap between the rows");
    assert_eq!(top.height(), code_row_height());
    // The caret is drawn at the lead over the highlight: it is where the next key moves
    // from.
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert!(
        (caret[0].origin.x - second.max_x()).abs() <= 2.0,
        "{caret:?}"
    );
    assert_eq!(caret[0].origin.y, second.origin.y);
    assert!(
        rects_with(&test, palette().cursor_row_bg).is_empty(),
        "the caret's wash stayed under the characters"
    );
    assert!(
        rects_with(&test, palette().text_select_bg)
            .iter()
            .all(|wash| wash.min_x() >= first.min_x() - 1.0),
        "a row is still washed whole under the characters"
    );
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
    assert!(spans(after[1].2, second.min_x(), second.max_x()));
    assert!(after[2].2.is_none(), "the pointer alone swept on");
}

/// The address column is gutter: a press on it puts the caret at the row's start, with
/// the caret's own wash and nothing selected, and a sweep from it goes by rows -- whole
/// ones, from the first row's start to the last's end, as a sweep down an editor's line
/// numbers does -- with no row washed whole: the selection is what shows.
#[test]
fn a_press_in_the_gutter_places_the_caret_and_a_sweep_takes_whole_rows() {
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
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), 0..=0);
    assert_eq!(picked.chars, CharSelection::at(Caret { row: 0, col: 0 }));
    assert_eq!(rects_with(&test, palette().cursor_row_bg).len(), 1);
    assert!(rects_with(&test, palette().text_select_bg).is_empty());
    let rows = paragraphs(&test);
    assert_eq!(carets(&test)[0].origin.x, rows[0].0.origin.x);

    // The sweep, to the row below: both rows whole.
    test.move_cursor(centre(second));
    settle(&mut test);
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.rows.rows(), 0..=1);
    assert_eq!(
        picked.chars.ends(),
        (
            Caret { row: 0, col: 0 },
            Caret {
                row: 1,
                col: crate::chars::END
            }
        )
    );
    let rows = paragraphs(&test);
    assert!(
        spans(rows[0].2, rows[0].0.min_x(), rows[0].0.max_x()),
        "{rows:?}"
    );
    assert!(
        spans(rows[1].2, rows[1].0.min_x(), rows[1].0.max_x()),
        "{rows:?}"
    );
    assert!(rows[2].2.is_none());
    assert!(rects_with(&test, palette().cursor_row_bg).is_empty());
    assert!(
        rects_with(&test, palette().text_select_bg)
            .iter()
            .all(|wash| wash.min_x() >= rows[0].0.min_x() - 1.0),
        "a row is washed whole"
    );

    // And back up over the anchor's row: the caret the press left.
    test.move_cursor(centre(first));
    settle(&mut test);
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.chars, CharSelection::at(Caret { row: 0, col: 0 }));
    assert_eq!(picked.rows.rows(), 0..=0);
}

/// Ctrl+C takes the characters where any are selected, and the rows otherwise -- the
/// caret's row, or the keyboard's run of rows, as each row's own line; and Escape peels
/// the selection back to the caret first and drops the run on a second press.
#[test]
fn the_characters_are_copied_before_the_rows_and_dropped_before_them() {
    let line = |row: usize| format!("row {row}");
    let text = |row: usize| Line::text(format!("text {row}"));
    let rows = RowSelection {
        anchor: 0,
        lead: 1,
        dragging: false,
    };
    let picked = |chars: CharSelection| Picked {
        rows,
        chars,
        by_rows: false,
        file: None,
        owed: Owed::default(),
    };
    let swept = CharSelection::at(Caret { row: 0, col: 5 }).extended(Caret { row: 1, col: 4 });

    let marks = Marks {
        assembly: Some(picked(swept)),
        source: Some(picked(CharSelection::at(Caret { row: 1, col: 3 }))),
    };
    assert_eq!(
        copy_text(&marks, Pane::Assembly, line, text).as_deref(),
        Some("0\ntext")
    );
    assert_eq!(
        copy_text(&marks, Pane::Source, line, text).as_deref(),
        Some("row 0\nrow 1")
    );
    // A caret alone copies its row, as an editor copies the line under one.
    let pressed = Marks {
        assembly: Some(Picked {
            rows: RowSelection {
                anchor: 1,
                lead: 1,
                dragging: false,
            },
            ..picked(CharSelection::at(Caret { row: 1, col: 5 }))
        }),
        source: None,
    };
    assert_eq!(
        copy_text(&pressed, Pane::Assembly, line, text).as_deref(),
        Some("row 1")
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
    assert!(paragraphs(&test)[0].2.is_some(), "the characters are drawn");

    test.press_key(Key::Named(NamedKey::Escape));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the run survives the first Escape");
    assert!(picked.chars.is_empty(), "{:?}", picked.chars);
    assert_eq!(picked.chars.lead(), Caret { row: 1, col: 4 });
    assert_eq!(picked.rows.rows(), 1..=1, "the rows follow the caret");
    assert!(paragraphs(&test)[0].2.is_none());
    assert!(rects_with(&test, palette().text_select_bg).is_empty());
    assert_eq!(rects_with(&test, palette().cursor_row_bg).len(), 1);

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
    let chars = picked.chars;
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
            .any(|(_, _, h)| spans(*h, link.min_x(), link.max_x())),
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

/// Alt held makes a press on a link the start of a selection and nothing else. Every
/// door in a code row acts on a plain press, which leaves no way to put the pointer down
/// on one and sweep: the release follows the link. Alt is what says "not a door this
/// time", and the selection the row's own `pointer_down` began is what stands.
#[test]
fn alt_held_makes_a_press_on_a_link_a_selection_and_not_a_door() {
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

    let (mut test, (states, marked, _landing, alt)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| {
            let (states, marked, landing) = listing_states!(runner, shown);
            let alt = runner.provide_root_context(|| Alt(State::create(false))).0;
            (states, marked, landing, alt)
        },
        1.,
    );
    let mut alt = alt;
    alt.set(true);
    settle(&mut test);
    let link = label_area(&test, target.display()).expect("the link is drawn");

    // The whole gesture, down on the link and up on it: without Alt this is what opens
    // the symbol (`a_link_in_the_text_is_one_unit_and_still_opens_its_symbol`).
    test.move_cursor(left_of(&link));
    test.press_cursor(left_of(&link));
    test.move_cursor(right_of(&link));
    test.release_cursor(right_of(&link));
    settle(&mut test);

    assert!(
        states.open.active().is_none(),
        "the link opened its symbol with Alt held"
    );
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.rows.rows(), row..=row);
    let (from, to) = picked.chars.ends();
    assert!(
        from.row == row && to.row == row && to.col > from.col,
        "the sweep over the link selected nothing: {from:?} {to:?}"
    );
}

/// And the door the unified view has of its own -- a Ctrl-press on a symbol's label row,
/// which is a press on the row and not on anything inside it -- is shut by Alt the same
/// way.
#[test]
fn alt_held_shuts_the_unified_views_own_door() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let reading = reading_of(&object, &[0, 1, 2]);
    let (mut test, (states, _marked, _sections, _window, _landing, ctrl, alt)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| {
            let (states, marked, sections, window, landing, ctrl) = code_states!(runner, reading);
            let alt = runner.provide_root_context(|| Alt(State::create(false))).0;
            (states, marked, sections, window, landing, ctrl, alt)
        },
        1.,
    );
    let (mut ctrl, mut alt) = (ctrl, alt);
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);

    ctrl.set(true);
    alt.set(true);
    settle(&mut test);
    let label = label_area(&test, "sum_to:").expect("sum_to is labelled");
    press_at(
        &mut test,
        (
            (label.origin.x + label.width() / 2.0) as f64,
            (label.origin.y + label.height() / 2.0) as f64,
        ),
    );
    settle(&mut test);
    assert!(
        states.open.active() == Some(code),
        "the label opened the symbol's tab with Alt held"
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
        .map(|picked| picked.chars)
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
        .map(|picked| picked.chars)
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
        .child(rect().expanded().child(AssemblyPane {
            tab: pane_tab(&document),
            document,
        }))
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
    assert_eq!(carets[0].width(), 2.0);
    assert_eq!(carets[0].origin.y, first.origin.y);
    assert_eq!(carets[0].height(), first.height());
}

/// A sweep carries on once the pointer has left the rows -- the pane, or the window: the
/// platform keeps reporting the pointer while the button is held and freya forwards every
/// move to the listing's global handler, which reaches the row on screen nearest the
/// pointer, from its start on the left and above and to its end on the right and below.
#[test]
fn a_sweep_carries_on_beyond_the_rows_the_pane_and_the_window() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let drawn = paragraphs(&test);
    let (first, third) = (drawn[0].0, drawn[2].0);
    let first_units = drawn[0].1.encode_utf16().count();
    let middle = |area: Area| (area.origin.y + area.height() / 2.0) as f64;

    test.move_cursor(left_of(&first));
    test.press_cursor(left_of(&first));
    // Left of the window, level with the third row: that row, from its start.
    test.move_cursor((-20.0, middle(third)));
    settle(&mut test);
    let chars = marked
        .peek()
        .assembly
        .clone()
        .map(|picked| picked.chars)
        .expect("the sweep is under way");
    assert_eq!(chars.lead(), Caret { row: 2, col: 0 });
    let drawn = paragraphs(&test);
    assert!(spans(drawn[1].2, drawn[1].0.min_x(), drawn[1].0.max_x()));
    assert!(
        drawn[2].2.is_none(),
        "row 2 from its start is nothing of it"
    );

    // Right of the window: that row, to its end.
    test.move_cursor((700.0, middle(third)));
    settle(&mut test);
    let chars = marked.peek().assembly.clone().map(|p| p.chars).unwrap();
    assert_eq!(chars.lead().row, 2);
    let drawn = paragraphs(&test);
    assert!(spans(drawn[2].2, third.min_x(), third.max_x()), "{drawn:?}");

    // Below the window: the last row on screen, cut or whole, to its end; and the rows
    // swept with it.
    test.move_cursor((300.0, 900.0));
    settle(&mut test);
    let picked = marked.peek().assembly.clone().unwrap();
    // The last row on screen may be one cut by the pane's edge that the virtual list
    // has not built, so the lead is the last built row or the one under it.
    let last_built = paragraphs(&test).len() - 1;
    assert!(last_built > 2);
    let lead = picked.chars.lead();
    assert!(
        lead.row == last_built || lead.row == last_built + 1,
        "{lead:?} against {last_built}"
    );
    assert_eq!(picked.rows.rows(), 0..=lead.row);

    // Above the window: the first row on screen, at the column under the pointer's x --
    // inside the row's text, so a run along the anchor's own row.
    test.move_cursor((300.0, -50.0));
    settle(&mut test);
    let lead = marked.peek().assembly.clone().unwrap().chars.lead();
    assert_eq!(lead.row, 0);
    assert!(lead.col > 0 && lead.col < first_units, "{lead:?}");
    let drawn = paragraphs(&test);
    assert!(drawn[0]
        .2
        .is_some_and(|h| h.min_x() == first.min_x() && h.max_x() < first.max_x()));
    assert!(drawn[1..].iter().all(|(_, _, h)| h.is_none()));
}

/// A key under modifiers, which no `TestingRunner` method sends: `press_key` hardcodes
/// none. Goes to the focused node, as every key does.
fn key_with(test: &mut TestingRunner, key: Key, modifiers: Modifiers) {
    use freya::prelude::platform::{KeyboardEventName, PlatformEvent};

    test.send_event(PlatformEvent::Keyboard {
        name: KeyboardEventName::KeyDown,
        key,
        code: Code::Unidentified,
        modifiers,
    });
    settle(test);
}

/// The arrow keys move the caret, and the run of rows and its wash go with it: Right one
/// character along the row, Down to the row below at the same column, End and Home to the
/// row's ends, and Left at a row's start to the end of the row above. The ends of a row's
/// text are what is asserted against, so nothing here measures a font.
#[test]
fn the_arrow_keys_move_the_caret_and_the_run_of_rows_with_it() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let drawn = paragraphs(&test);
    let (first, second) = (drawn[0].0, drawn[1].0);
    let at = left_of(&first);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    mark_release(marked);
    settle(&mut test);
    assert_eq!(carets(&test)[0].origin.x, first.origin.x);

    // Right: one character on, still on the row and still inside its text.
    test.press_key(Key::Named(NamedKey::ArrowRight));
    settle(&mut test);
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert!(
        caret[0].origin.x > first.origin.x && caret[0].origin.x < first.max_x(),
        "the caret did not step along the row: {caret:?} in {first:?}"
    );
    assert_eq!(caret[0].origin.y, first.origin.y);
    let chars = marked.peek().assembly.clone().unwrap().chars;
    assert_eq!(chars.lead(), Caret { row: 0, col: 1 });
    assert!(chars.is_empty(), "a plain key swept characters out");

    // Down: the row below, and the run of rows and the wash with it. No scroll is owed
    // to the other pane -- a key repeat would yank it -- and no drag is under way.
    test.press_key(Key::Named(NamedKey::ArrowDown));
    settle(&mut test);
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert_eq!(caret[0].origin.y, second.origin.y);
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.chars.lead(), Caret { row: 1, col: 1 });
    assert_eq!(
        picked.rows.rows(),
        1..=1,
        "the rows did not follow the caret"
    );
    assert!(!picked.rows.dragging);
    assert!(picked.owed == Owed::default(), "a key move owed a scroll");
    let washes = rects_with(&test, palette().cursor_row_bg);
    assert_eq!(washes.len(), 1, "{washes:?}");
    assert_eq!(washes[0].origin.y, second.origin.y);

    // End and Home: the row's own ends.
    test.press_key(Key::Named(NamedKey::End));
    settle(&mut test);
    let caret = carets(&test);
    assert!(
        (caret[0].origin.x - second.max_x()).abs() <= 1.0,
        "{caret:?} against {second:?}"
    );
    test.press_key(Key::Named(NamedKey::Home));
    settle(&mut test);
    assert_eq!(carets(&test)[0].origin.x, second.origin.x);

    // Left at the row's start: the end of the row above.
    test.press_key(Key::Named(NamedKey::ArrowLeft));
    settle(&mut test);
    let caret = carets(&test);
    assert_eq!(caret[0].origin.y, first.origin.y);
    assert!(
        (caret[0].origin.x - first.max_x()).abs() <= 1.0,
        "{caret:?} against {first:?}"
    );
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.rows.rows(), 0..=0);
}

/// With Shift held a key reaches the run out from its anchor, characters and rows both,
/// and each row between draws its part; a key without Shift collapses it to the caret.
#[test]
fn shift_and_a_key_reach_the_run_out_and_a_key_alone_collapses_it() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 900.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let drawn = paragraphs(&test);
    let (first, second) = (drawn[0].0, drawn[1].0);
    let at = left_of(&first);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    mark_release(marked);
    settle(&mut test);

    // Shift+Down: the first row from the caret to its end, and nothing of the second,
    // the lead being at its start.
    key_with(&mut test, Key::Named(NamedKey::ArrowDown), Modifiers::SHIFT);
    let drawn = paragraphs(&test);
    assert!(spans(drawn[0].2, first.min_x(), first.max_x()), "{drawn:?}");
    assert!(drawn[1].2.is_none(), "{drawn:?}");
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(
        picked.chars.ends(),
        (Caret { row: 0, col: 0 }, Caret { row: 1, col: 0 })
    );
    assert_eq!(picked.rows.rows(), 0..=1);
    assert!(!picked.rows.dragging);
    assert_eq!(
        carets(&test).len(),
        1,
        "the caret is drawn at the lead over a highlight"
    );

    // Shift+End: the second row whole.
    key_with(&mut test, Key::Named(NamedKey::End), Modifiers::SHIFT);
    let drawn = paragraphs(&test);
    assert!(spans(drawn[0].2, first.min_x(), first.max_x()), "{drawn:?}");
    assert!(
        spans(drawn[1].2, second.min_x(), second.max_x()),
        "{drawn:?}"
    );
    // Shift+Down again: a third row in, and the rows with it.
    key_with(&mut test, Key::Named(NamedKey::ArrowDown), Modifiers::SHIFT);
    let drawn = paragraphs(&test);
    assert!(
        spans(drawn[1].2, second.min_x(), second.max_x()),
        "{drawn:?}"
    );
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.rows.rows(), 0..=2);
    assert_eq!(picked.chars.ends().0, Caret { row: 0, col: 0 });
    // Shift+Up: back off the third row, the anchor still where the press was.
    key_with(&mut test, Key::Named(NamedKey::ArrowUp), Modifiers::SHIFT);
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.rows.rows(), 0..=1);
    assert_eq!(picked.chars.ends().0, Caret { row: 0, col: 0 });

    // Down alone: collapsed to the caret, on the row below the lead's, no highlight.
    test.press_key(Key::Named(NamedKey::ArrowDown));
    settle(&mut test);
    let picked = marked.peek().assembly.clone().unwrap();
    let chars = picked.chars;
    assert!(chars.is_empty(), "{chars:?}");
    assert_eq!(chars.lead().row, 2);
    assert_eq!(picked.rows.rows(), 2..=2);
    assert!(paragraphs(&test).iter().all(|(_, _, h)| h.is_none()));
    assert_eq!(carets(&test).len(), 1);
}

/// Ctrl+End puts the caret at the listing's last row's end and scrolls the pane so the
/// row is on screen; Ctrl+Home brings it back to the top.
#[test]
fn ctrl_end_goes_to_the_listings_end_and_the_pane_scrolls_to_it() {
    let shown = shown_sum_to();
    let instructions = &shown.studied.assembly.as_ref().unwrap().instructions;
    let length = shown.studied.lanes.listing_rows(instructions.len());
    let first_address = format!("{:016X} ", instructions[0].address);
    let last_address = format!("{:016X} ", instructions.last().unwrap().address);
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    assert!(
        !labels(&test).contains(&last_address),
        "the last row is on screen before any scroll, so the scroll cannot be seen"
    );
    let first = paragraphs(&test)[0].0;
    let at = left_of(&first);
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    mark_release(marked);
    settle(&mut test);

    key_with(&mut test, Key::Named(NamedKey::End), Modifiers::CONTROL);
    settle(&mut test);
    assert!(
        labels(&test).contains(&last_address),
        "the pane did not scroll to the caret: {:?}",
        labels(&test)
    );
    let picked = marked.peek().assembly.clone().unwrap();
    let lead = picked.chars.lead();
    assert_eq!(lead.row, length - 1);
    assert_eq!(picked.rows.rows(), length - 1..=length - 1);
    let drawn = paragraphs(&test);
    let last = drawn.last().unwrap().0;
    let caret = carets(&test);
    assert_eq!(caret.len(), 1, "{caret:?}");
    assert_eq!(caret[0].origin.y, last.origin.y);
    assert!(
        (caret[0].origin.x - last.max_x()).abs() <= 1.0,
        "{caret:?} against {last:?}"
    );

    key_with(&mut test, Key::Named(NamedKey::Home), Modifiers::CONTROL);
    settle(&mut test);
    assert!(labels(&test).contains(&first_address));
    let picked = marked.peek().assembly.clone().unwrap();
    assert_eq!(picked.chars.lead(), Caret { row: 0, col: 0 });
    assert_eq!(picked.rows.rows(), 0..=0);
}

/// The keys move the caret along a row of the unified view, and the row draws it there:
/// an answer taken into the reading as the app takes it, the code tab opened, a press on
/// an instruction, then Right, End and Home. The drawing is the point: the view's row data
/// compared everything but the caret, so a move along a row -- which changes no row of the
/// run -- rebuilt nothing and the caret stayed drawn where it was, while a vertical move
/// redrew; Left, Right, Home and End read as dead there and nowhere else.
#[test]
fn the_keys_move_the_caret_along_a_row_the_worker_decoded() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let mut reading = Reading::of(Some(object.clone()));
    let ask = CodeAsk {
        object: object.clone(),
        code: None,
        window: vec![0, 1, 2],
    };
    let Answer::Code { decoded, code, .. } = answer(Question::Code(ask.clone())) else {
        panic!("a window is answered with a window");
    };
    assert!(reading.take(&ask, code, decoded));
    let (mut test, (states, marked, _sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_harness,
        (600., 900.).into(),
        |runner| code_states!(runner, reading),
        1.,
    );
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);

    let rows = paragraphs(&test);
    let (area, text, _) = rows
        .iter()
        .find(|(_, text, _)| text.starts_with("push"))
        .expect("an instruction is drawn")
        .clone();
    test.move_cursor(left_of(&area));
    test.press_cursor(left_of(&area));
    test.release_cursor(left_of(&area));
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out");
    assert_eq!(picked.chars.lead().col, 0);
    let row = picked.chars.lead().row;

    test.press_key(Key::Named(NamedKey::ArrowRight));
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead(),
        Caret { row, col: 1 }
    );
    test.press_key(Key::Named(NamedKey::End));
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead(),
        Caret {
            row,
            col: text.encode_utf16().count()
        }
    );
    let caret = carets(&test);
    assert_eq!(caret.len(), 1);
    assert!(
        (caret[0].origin.x - area.max_x()).abs() <= 2.0,
        "{caret:?} against {area:?}"
    );
    test.press_key(Key::Named(NamedKey::Home));
    settle(&mut test);
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead(),
        Caret { row, col: 0 }
    );
}

/// A sweep held past the pane's bottom scrolls the view a row at a time towards the
/// pointer and reaches the run out to each row that comes in, for as long as the button is
/// down and the pointer stays past the edge; back inside, the view stops where it is.
#[test]
fn a_sweep_held_past_the_panes_edge_scrolls_the_view() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (600., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let before = paragraphs(&test);
    let first = before[0].0;
    test.move_cursor(left_of(&first));
    test.press_cursor(left_of(&first));
    test.move_cursor((300.0, 900.0));
    settle(&mut test);
    let reached = marked.peek().assembly.clone().unwrap().chars.lead().row;
    assert!(reached > 2);

    // Held there: the rows move up under the pointer and the run grows.
    test.poll_n(Duration::from_millis(45), 8);
    settle(&mut test);
    let after = paragraphs(&test);
    assert_ne!(
        after[0].1, before[0].1,
        "the view did not scroll: {after:?}"
    );
    let grown = marked.peek().assembly.clone().unwrap().chars.lead().row;
    assert!(grown > reached, "{grown} against {reached}");
    assert_eq!(
        marked.peek().assembly.clone().unwrap().rows.rows(),
        0..=grown
    );

    // Back inside: the view stops.
    test.move_cursor((300.0, 150.0));
    settle(&mut test);
    let stopped = paragraphs(&test)[0].1.clone();
    let lead = marked.peek().assembly.clone().unwrap().chars.lead().row;
    test.poll_n(Duration::from_millis(45), 4);
    settle(&mut test);
    assert_eq!(
        paragraphs(&test)[0].1,
        stopped,
        "the view went on scrolling"
    );
    assert_eq!(
        marked.peek().assembly.clone().unwrap().chars.lead().row,
        lead
    );
    test.release_cursor((300.0, 150.0));
    mark_release(marked);
}

/// A sweep held past the pane's right edge reaches the column at that edge, not the row's
/// end, and scrolls the view sideways under the pointer a little at a time, the run
/// reaching out over what comes in.
#[test]
fn a_sweep_held_past_the_panes_side_scrolls_the_view_sideways() {
    let shown = shown_sum_to();
    let (mut test, (_states, marked, _landing)) = TestingRunner::new(
        listing_harness,
        (300., 300.).into(),
        |runner| listing_states!(runner, shown),
        1.,
    );
    settle(&mut test);
    let rows = paragraphs(&test);
    // A row longer than the pane, with room to scroll.
    let (row, (area, text, _)) = rows
        .iter()
        .enumerate()
        .max_by(|a, b| a.1 .0.max_x().total_cmp(&b.1 .0.max_x()))
        .map(|(i, r)| (i, r.clone()))
        .unwrap();
    assert!(area.max_x() > 320.0, "{area:?}");
    let units = text.encode_utf16().count();
    let at = (
        (area.origin.x + 2.0) as f64,
        (area.origin.y + area.height() / 2.0) as f64,
    );
    test.move_cursor(at);
    test.press_cursor(at);
    // Right of the window, level with the row: the column at the pane's edge.
    test.move_cursor((700.0, at.1));
    settle(&mut test);
    let lead = marked.peek().assembly.clone().unwrap().chars.lead();
    assert_eq!(lead.row, row);
    assert!(lead.col > 0 && lead.col < units, "{lead:?} against {units}");
    let edge = lead.col;

    // Held there: the rows slide left and the run reaches further along the row.
    test.poll_n(Duration::from_millis(45), 8);
    settle(&mut test);
    let slid = paragraphs(&test)[row].0;
    assert!(
        slid.origin.x < area.origin.x,
        "the view did not scroll sideways: {slid:?}"
    );
    let lead = marked.peek().assembly.clone().unwrap().chars.lead();
    assert!(lead.col > edge, "{lead:?} against {edge}");
    test.release_cursor((700.0, at.1));
    mark_release(marked);
}

/// A link followed from inside a tab replaces what the tab shows and leaves the place it
/// showed one Back away: one tab, a trail of three, and the row the first place was left
/// at kept under that place's own entry through it all -- so Back comes back to it. A
/// link to the place already on screen pushes nothing. Headless because `open_document`
/// peeks the states it then writes, which is legal to the compiler and panics at the
/// moment it runs if a read is still borrowed.
#[test]
fn a_link_inside_a_tab_is_followed_in_place_and_back_returns() {
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

    let id = open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    )
    .expect("a document panel");
    let mut asm_at = states.asm_at;
    asm_at
        .write()
        .remember((id, Stop::whole(documents[0].clone())), 12);
    for document in &documents[1..] {
        let landed = open_document(states.open, states.visits, document.clone(), Reach::InPlace);
        assert_eq!(landed, Some(id), "a link opened a tab of its own");
    }
    test.sync_and_update();

    assert!(states.open.documents() == [documents[2].clone()]);
    assert!(trail_of(&states, id) == documents);
    assert_eq!(cursor_of(&states), Some(2));
    assert!(states.visits.peek().entries() == documents);
    assert_eq!(
        states
            .asm_at
            .peek()
            .at(&(id, Stop::whole(documents[0].clone()))),
        Some(12),
        "the row of the place left was forgotten"
    );

    // The place on screen again: nothing to push.
    open_document(
        states.open,
        states.visits,
        documents[2].clone(),
        Reach::InPlace,
    );
    assert!(trail_of(&states, id) == documents);

    navigate(states.open, Nav::Back);
    navigate(states.open, Nav::Back);
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[0].clone()));
    assert_eq!(
        states
            .asm_at
            .peek()
            .at(&(id, Stop::whole(documents[0].clone()))),
        Some(12)
    );
    navigate(states.open, Nav::Forward);
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[1].clone()));
    // Nothing of that was a visit.
    assert!(states.visits.peek().entries() == documents);
}

/// A click from outside the panes opens its place in one temporal tab, which the next such
/// click reuses by pushing onto its trail -- so Back inside it walks the rows clicked --
/// and a tab already showing the place is raised instead, whichever tab that is. Walking
/// the trail leaves the tab temporal.
#[test]
fn a_sidebar_row_opens_the_temporal_tab_and_the_next_row_reuses_it() {
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

    let kept = open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    )
    .expect("a document panel");
    let preview = open_document(
        states.open,
        states.visits,
        documents[1].clone(),
        Reach::Preview,
    )
    .expect("a document panel");
    assert_ne!(kept, preview);
    assert_eq!(states.open.docs.peek().temporal(), Some(preview));
    assert_eq!(states.open.active_id(), Some(preview));

    let again = open_document(
        states.open,
        states.visits,
        documents[2].clone(),
        Reach::Preview,
    );
    assert_eq!(again, Some(preview), "a second row opened a second tab");
    assert_eq!(states.open.documents().len(), 2);
    assert!(trail_of(&states, preview) == documents[1..]);

    // A place a tab already shows: that tab, the temporal one left as it is.
    let raised = open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::Preview,
    );
    assert_eq!(raised, Some(kept));
    assert_eq!(states.open.active_id(), Some(kept));
    assert_eq!(states.open.docs.peek().temporal(), Some(preview));
    let raised = open_document(
        states.open,
        states.visits,
        documents[2].clone(),
        Reach::Preview,
    );
    assert_eq!(raised, Some(preview));
    assert!(
        trail_of(&states, preview) == documents[1..],
        "raising the temporal tab pushed onto it"
    );

    navigate(states.open, Nav::Back);
    test.sync_and_update();
    assert!(states.open.active() == Some(documents[1].clone()));
    assert_eq!(
        states.open.docs.peek().temporal(),
        Some(preview),
        "walking the trail promoted the tab"
    );
    // Every opening was a visit, the raises included.
    assert!(
        states.visits.peek().entries()
            == [
                documents[1].clone(),
                documents[0].clone(),
                documents[2].clone()
            ]
    );
}

/// A new tab opens beside the tab on screen, and at the end of the strip when a view is
/// on top.
#[test]
fn a_new_tab_opens_beside_the_one_on_screen() {
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

    open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    );
    open_document(
        states.open,
        states.visits,
        documents[1].clone(),
        Reach::NewTab,
    );
    raise_document(&states, &documents[0]);
    open_document(
        states.open,
        states.visits,
        documents[2].clone(),
        Reach::NewTab,
    );
    test.sync_and_update();
    assert!(
        states.open.documents()
            == [
                documents[0].clone(),
                documents[2].clone(),
                documents[1].clone()
            ],
        "the new tab did not open beside the one on screen"
    );

    // A page on screen is a tab like any other, so the tab opened over it lands beside it
    // and not at the end of the bar. The page goes in beside the tab on screen, which
    // leaves a document after it to tell the two landings apart.
    {
        let mut strip = states.open.strip;
        strip.write().show(Tab::Page(Page::Settings));
    }
    let source = Document::Source(Arc::from("/src/main.rs"));
    open_document(states.open, states.visits, source.clone(), Reach::Preview);
    test.sync_and_update();
    assert!(
        states.open.documents()
            == [
                documents[0].clone(),
                documents[2].clone(),
                source.clone(),
                documents[1].clone()
            ],
        "a tab opened over a page did not land beside it"
    );
}

/// What makes a temporal tab stay: Ctrl on the place it shows, or navigating in place
/// inside it. Back does not, and the next preview after either opens a temporal tab of
/// its own.
#[test]
fn a_temporal_tab_is_promoted_by_ctrl_and_by_a_link_followed_in_it_and_not_by_back() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let documents: Vec<Document> = symbols
        .iter()
        .take(3)
        .map(|symbol| Document::Assembly(Selection::Symbol(symbol.clone())))
        .collect();
    let source = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    let first = open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::Preview,
    )
    .expect("a document panel");
    open_document(
        states.open,
        states.visits,
        documents[1].clone(),
        Reach::Preview,
    );
    navigate(states.open, Nav::Back);
    assert_eq!(states.open.docs.peek().temporal(), Some(first));
    // Ctrl on the place the temporal tab shows: it stays, and no second tab opens.
    let kept = open_document(
        states.open,
        states.visits,
        documents[0].clone(),
        Reach::NewTab,
    );
    assert_eq!(kept, Some(first));
    assert_eq!(states.open.docs.peek().temporal(), None);
    assert_eq!(states.open.documents().len(), 1);

    let second = open_document(
        states.open,
        states.visits,
        documents[2].clone(),
        Reach::Preview,
    )
    .expect("a document panel");
    assert_ne!(second, first, "the promoted tab was reused as the preview");
    assert_eq!(states.open.docs.peek().temporal(), Some(second));
    // A link followed inside it: reading in it, so it stays.
    open_document(states.open, states.visits, source.clone(), Reach::InPlace);
    assert_eq!(states.open.docs.peek().temporal(), None);
    assert_eq!(states.open.active_id(), Some(second));

    let third = open_document(
        states.open,
        states.visits,
        documents[1].clone(),
        Reach::Preview,
    );
    assert!(third.is_some_and(|third| third != first && third != second));
    test.sync_and_update();
    assert_eq!(states.open.documents().len(), 3);
}

/// Closing a binary closes the tabs showing a place in it and thins the trails of the
/// rest: a source-driven tab reached from a symbol keeps its slot and loses the symbol,
/// with the row kept for it and its visit.
#[test]
fn closing_a_binary_thins_the_trails_of_the_tabs_it_leaves() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let path = object.path.clone();
    let symbol = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let other = Document::Assembly(Selection::Symbol(symbols[1].clone()));
    let source = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    let survivor = open_document(states.open, states.visits, symbol.clone(), Reach::NewTab)
        .expect("a document panel");
    open_document(states.open, states.visits, source.clone(), Reach::InPlace);
    open_document(states.open, states.visits, other.clone(), Reach::NewTab);
    let mut asm_at = states.asm_at;
    asm_at
        .write()
        .remember((survivor, Stop::whole(symbol.clone())), 12);
    asm_at
        .write()
        .remember((survivor, Stop::whole(source.clone())), 3);
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
        states.marks_at,
        states.visits,
        &path,
    );
    test.sync_and_update();

    assert!(states.open.documents() == [source.clone()]);
    assert!(states.open.active() == Some(source.clone()));
    assert!(trail_of(&states, survivor) == [source.clone()]);
    assert_eq!(
        states
            .asm_at
            .peek()
            .at(&(survivor, Stop::whole(symbol.clone()))),
        None,
        "a position into the closed file was kept, and with it the file's bytes"
    );
    assert_eq!(
        states
            .asm_at
            .peek()
            .at(&(survivor, Stop::whole(source.clone()))),
        Some(3)
    );
    assert!(states.visits.peek().entries() == [source]);
}

/// A document's two panes as [`panes_harness`] mounts them, under the two root effects a
/// switch of place goes through: `use_land`, which keeps each pane's run as a place is
/// left and puts it back as it arrives, and `use_clear_marks`, which drops a run whose
/// listing is replaced within one place -- and must not drop one for the switch.
fn navigating_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let open = use_open();
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    let analysis = use_consume::<Analysis>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    use_clear_marks(
        active,
        super::analyzed::Asked { active, driven },
        analysis,
        marked,
    );
    panes_harness()
}

/// An assembly-driven tab on `sum_to` with a companion file of twenty lines that exists,
/// so both panes have rows to press: the runner, the states, the symbol's document and
/// the file.
fn navigating_panes() -> (
    TestingRunner,
    ProjectStates,
    State<Marks>,
    State<Option<Landing>>,
    State<Option<Planting>>,
    Document,
    Arc<str>,
) {
    let sum_to = fixture_symbols()
        .into_iter()
        .find(|symbol| symbol.data.name == "sum_to")
        .expect("the fixture holds sum_to");
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-restore-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let path = directory.join("kept.c");
    let text: String = (1..=20).map(|n| format!("int line_{n}(void);\n")).collect();
    std::fs::write(&path, text).expect("writing the source file");
    let file: Arc<str> = Arc::from(path.to_str().expect("a utf-8 temporary path"));

    let mut studied = Studied::new(sum_to.clone());
    studied.lines.file = Some(file.clone());
    // No line of its own, so the pane opens the file at the top and a row on screen is
    // the listing row it is.
    studied.lines.line = None;
    let shown = Shown {
        ask: Ask::Symbol(sum_to.clone()),
        studied,
    };
    let (test, ((states, marked, landing), plant)) = TestingRunner::new(
        navigating_harness,
        (700., 400.).into(),
        |runner| {
            let states = listing_states!(runner, shown);
            // Re-provided, as `Ctrl` is elsewhere, to be handed to `land`.
            let plant = runner.provide_root_context(|| Plant(State::create(None))).0;
            runner.provide_root_context(|| SplitRatio(State::create(50.0)));
            runner.provide_root_context(|| {
                Splits(State::create(ResizableContext {
                    direction: Direction::Horizontal,
                    ..Default::default()
                }))
            });
            (states, plant)
        },
        1.,
    );
    let document = Document::Assembly(Selection::Symbol(sum_to));
    (test, states, marked, landing, plant, document, file)
}

/// The paragraphs of one pane of [`navigating_panes`]'s window, top to bottom: the
/// assembly pane leads and so is the left half, the source pane the right.
fn pane_paragraphs(test: &TestingRunner, pane: Pane) -> Vec<Area> {
    paragraphs(test)
        .into_iter()
        .filter(|(area, _, _)| match pane {
            Pane::Assembly => area.min_x() < 350.0,
            Pane::Source => area.min_x() >= 350.0,
        })
        .map(|(area, _, _)| area)
        .collect()
}

/// A sweep along the text from the start of row `from` to a few characters into row
/// `to` of `pane`, let go of there -- the release is the root's and not this harness's.
fn sweep(test: &mut TestingRunner, marked: State<Marks>, pane: Pane, from: usize, to: usize) {
    let rows = pane_paragraphs(test, pane);
    let into = |area: &Area| (left_of(area).0 + 30.0, left_of(area).1);
    test.move_cursor(left_of(&rows[from]));
    test.press_cursor(left_of(&rows[from]));
    test.move_cursor(into(&rows[to]));
    settle(test);
    test.release_cursor(into(&rows[to]));
    mark_release(marked);
    settle(test);
}

/// The two runs as a place keeps them: rows and characters, the file, and whether a
/// scroll is owed or a sweep under way.
fn runs_of(marked: State<Marks>) -> (Option<Picked>, Option<Picked>) {
    let marks = marked.peek();
    (marks.assembly.clone(), marks.source.clone())
}

/// Navigating brings back each pane's caret and selection for the place arriving, in
/// both panes -- the companion's run too, in a tab driven from the other side -- and
/// nothing is owed: the kept rows put the view back, and a reveal beside them would fight
/// them. Back returns to the runs the place was left with, Forward to the runs the next
/// place made of its own. Headless because the restore lands through a root effect woken
/// by the same change as the one that drops a run whose listing goes, and only the
/// runner can say which of the two the marks end up as.
#[test]
fn navigating_brings_back_each_panes_caret_and_selection() {
    let symbols = fixture_symbols();
    let (mut test, states, marked, _landing, _plant, sum_to, file) = navigating_panes();
    let add = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let id = open_document(states.open, states.visits, sum_to.clone(), Reach::NewTab)
        .expect("a document panel");
    settle(&mut test);
    settle(&mut test);

    // A run in each pane: two rows of instructions, and two lines of the companion.
    sweep(&mut test, marked, Pane::Assembly, 0, 1);
    sweep(&mut test, marked, Pane::Source, 2, 3);
    let (assembly, source) = runs_of(marked);
    let assembly = assembly.expect("the sweep picked the instructions out");
    let source = source.expect("the sweep picked the lines out");
    assert_eq!(assembly.rows.rows(), 0..=1);
    assert_eq!(source.rows.rows(), 2..=3);
    assert!(!assembly.chars.is_empty() && !source.chars.is_empty());
    assert!(source.file.as_deref() == Some(&*file));
    assert_eq!(carets(&test).len(), 2, "a caret per pane");

    // A link followed in place: the runs are kept under the place left, and the place
    // arriving -- never shown before -- starts with none.
    open_document(states.open, states.visits, add.clone(), Reach::InPlace);
    settle(&mut test);
    settle(&mut test);
    assert!(states.open.active() == Some(add.clone()));
    let kept = states
        .marks_at
        .peek()
        .at(&(id, Stop::whole(sum_to.clone())))
        .expect("the runs of the place left were not kept");
    assert!(kept.marks.assembly.as_ref().map(|p| p.rows) == Some(assembly.rows));
    assert!(kept.marks.source.as_ref().map(|p| p.rows) == Some(source.rows));
    let (now_assembly, now_source) = runs_of(marked);
    assert!(
        now_assembly.is_none() && now_source.is_none(),
        "the runs outlived their place"
    );
    assert!(carets(&test).is_empty(), "a caret with no run");
    // A run of this place's own, in one pane.
    sweep(&mut test, marked, Pane::Assembly, 5, 5);
    let (theirs, _) = runs_of(marked);
    let theirs = theirs.expect("the press picked the row out");
    assert_eq!(theirs.rows.rows(), 5..=5);

    // Back: both runs and both carets are where they were left, and nothing is owed.
    navigate(states.open, Nav::Back);
    settle(&mut test);
    settle(&mut test);
    assert!(states.open.active() == Some(sum_to.clone()));
    let (back_assembly, back_source) = runs_of(marked);
    let back_assembly = back_assembly.expect("the assembly run did not come back");
    let back_source = back_source.expect("the source run did not come back");
    assert_eq!(back_assembly.rows.rows(), assembly.rows.rows());
    assert_eq!(back_assembly.chars, assembly.chars);
    assert_eq!(back_source.rows.rows(), source.rows.rows());
    assert_eq!(back_source.chars, source.chars);
    assert!(back_source.file == source.file);
    assert!(
        back_assembly.owed == Owed::default() && back_source.owed == Owed::default(),
        "a restored run owes a scroll"
    );
    assert!(!back_assembly.rows.dragging && !back_source.rows.dragging);
    assert!(owed_reveal(marked, Pane::Assembly).is_none());
    assert!(owed_reveal(marked, Pane::Source).is_none());
    assert_eq!(carets(&test).len(), 2, "a caret per pane, drawn again");

    // Forward: the other place's own run, and none in the pane it made none in.
    navigate(states.open, Nav::Forward);
    settle(&mut test);
    settle(&mut test);
    let (forward_assembly, forward_source) = runs_of(marked);
    let forward_assembly = forward_assembly.expect("the next place's run did not come back");
    assert_eq!(forward_assembly.rows.rows(), theirs.rows.rows());
    assert!(
        forward_source.is_none(),
        "a run was made up for the source pane"
    );
}

/// A landing wins over what was kept: a click from outside named a line, and the run it
/// makes is the only run in either pane -- the assembly pane's old run beside the pair of
/// the new would light two places at once. Headless for the reason the test above is.
#[test]
fn a_landing_on_arrival_wins_over_the_kept_runs() {
    let symbols = fixture_symbols();
    let (mut test, states, marked, landing, plant, sum_to, file) = navigating_panes();
    let add = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let id = open_document(states.open, states.visits, sum_to.clone(), Reach::NewTab)
        .expect("a document panel");
    settle(&mut test);
    settle(&mut test);
    sweep(&mut test, marked, Pane::Assembly, 0, 1);
    sweep(&mut test, marked, Pane::Source, 2, 3);
    open_document(states.open, states.visits, add, Reach::InPlace);
    settle(&mut test);
    settle(&mut test);
    assert!(states
        .marks_at
        .peek()
        .at(&(id, Stop::whole(sum_to.clone())))
        .is_some());

    // Back to the place through a row outside the panes, on a line of its own.
    let at = LinePos {
        file: file.clone(),
        line: 9,
    };
    land(
        states.open,
        states.visits,
        marked,
        landing,
        plant,
        Landing {
            tab: sum_to.clone(),
            at: Some(at.clone()),
            address: None,
            columns: None,
        },
        Reach::InPlace,
    );
    settle(&mut test);
    settle(&mut test);
    assert!(states.open.active() == Some(sum_to));
    let (assembly, source) = runs_of(marked);
    let source = source.expect("the landing planted nothing");
    assert!(
        source_line(marked) == Some(at),
        "the kept run won over the landing"
    );
    assert!(
        source.chars.is_empty(),
        "the kept characters came back under the landing"
    );
    assert!(
        assembly.is_none(),
        "the kept assembly run came back beside the landing"
    );
}

/// The kept runs go with the entry they are kept under, as the rows do: a closing tab's
/// by id, and a closing binary's by id and by every entry it takes off a surviving trail
/// -- not tidiness, since an entry holds the `Arc<Object>` its document points into.
/// Asserted through the map, which is the only thing that can say an entry is gone.
#[test]
fn closing_a_tab_and_a_binary_forget_the_kept_runs() {
    let symbols = fixture_symbols();
    let object = symbols[0].object.clone();
    let path = object.path.clone();
    let symbol = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let other = Document::Assembly(Selection::Symbol(symbols[1].clone()));
    let source = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();
    let mut objects = states.objects;
    objects.write().push(object);

    let survivor = open_document(states.open, states.visits, symbol.clone(), Reach::NewTab)
        .expect("a document panel");
    open_document(states.open, states.visits, source.clone(), Reach::InPlace);
    let closing = open_document(states.open, states.visits, other.clone(), Reach::NewTab)
        .expect("a document panel");
    let kept = |row: usize| Kept {
        marks: Marks {
            assembly: Some(picked_row(row, "a.c", Owed::default())),
            source: None,
        },
        ..Kept::default()
    };
    let mut marks_at = states.marks_at;
    marks_at
        .write()
        .remember((survivor, Stop::whole(symbol.clone())), kept(1));
    marks_at
        .write()
        .remember((survivor, Stop::whole(source.clone())), kept(2));
    marks_at
        .write()
        .remember((closing, Stop::whole(other.clone())), kept(3));
    test.sync_and_update();

    close_tab(
        states.open,
        states.asm_at,
        states.src_at,
        states.code_at,
        states.driven,
        states.marks_at,
        closing,
    );
    test.sync_and_update();
    assert!(
        states
            .marks_at
            .peek()
            .at(&(closing, Stop::whole(other)))
            .is_none(),
        "the closed tab's runs were kept, and with them the binary they point into"
    );
    assert!(
        states
            .marks_at
            .peek()
            .at(&(survivor, Stop::whole(symbol.clone())))
            == Some(kept(1))
    );

    close_binary(
        states.objects,
        states.loading,
        states.open,
        states.asm_at,
        states.src_at,
        states.code_at,
        states.driven,
        states.marks_at,
        states.visits,
        &path,
    );
    test.sync_and_update();
    assert!(states.open.documents() == [source.clone()]);
    assert!(
        states
            .marks_at
            .peek()
            .at(&(survivor, Stop::whole(symbol)))
            .is_none(),
        "the runs of an entry the closing binary took off the trail were kept"
    );
    assert!(states.marks_at.peek().at(&(survivor, Stop::whole(source))) == Some(kept(2)));
}

/// The unified view's pane as the app mounts it, under the root effects a switch goes
/// through, as [`navigating_harness`] is for a document's two panes.
fn code_navigating_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let open = use_open();
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    let analysis = use_consume::<Analysis>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    use_clear_marks(
        active,
        super::analyzed::Asked { active, driven },
        analysis,
        marked,
    );
    app_like_code_harness()
}

/// An object's code forgets its rows when the tab is left -- the reading is reset, and
/// comes back as guesses -- so its run comes back by the **places** its rows stood for,
/// which the view writes down as the run changes: the caret on a label is on the label
/// again after the tab was left and returned to, though the label's row is another row
/// now. Headless because the places are written by the view's own effect and read back
/// by it a pass after `use_land` has put the kept run back, and only the runner can say
/// the two land in that order.
#[test]
fn a_run_in_an_objects_code_comes_back_by_the_places_its_rows_stood_for() {
    let (_path, objects) = fixture_objects(1);
    let object = objects[0].clone();
    let (mut test, (states, marked, sections, _window, _landing, _ctrl)) = TestingRunner::new(
        code_navigating_harness,
        (600., 900.).into(),
        {
            let object = object.clone();
            move |runner| {
                runner.provide_root_context(|| PaneObject(object.clone()));
                code_states!(runner, Reading::default())
            }
        },
        1.,
    );
    let mut sections = sections;
    let mut open = states.objects;
    open.write().push(object.clone());
    settle(&mut test);

    // The tab, and the worker's first answer: `add` decoded, the rest guessed.
    let code = Document::Code(object.clone());
    open_document(states.open, states.visits, code.clone(), Reach::NewTab);
    settle(&mut test);
    settle(&mut test);
    assert!(sections.peek().is_about(&object));
    sections.set(reading_of(&object, &[0]));
    settle(&mut test);
    settle(&mut test);

    // The caret on `twice`'s label, at its start.
    let at = label_area(&test, "twice:").expect("the label is drawn");
    test.move_cursor(left_of(&at));
    test.press_cursor(left_of(&at));
    test.release_cursor(left_of(&at));
    settle(&mut test);
    let was = marked
        .peek()
        .assembly
        .clone()
        .expect("the press picked the row out")
        .chars
        .lead()
        .row;
    let entry = entry_of(&states, &code);
    let kept = states
        .marks_at
        .peek()
        .at(&entry)
        .expect("the view wrote nothing down for the run");
    assert!(
        kept.spots.iter().any(|(row, _)| *row == was),
        "the caret's row has no place kept: {:?}",
        kept.spots
    );

    // A symbol's tab beside it: the reading is reset, and the run goes with the place.
    let symbol = Document::Assembly(Selection::Symbol(Symbol {
        object: object.clone(),
        data: object.symbols_sorted[0].clone(),
    }));
    open_document(states.open, states.visits, symbol, Reach::NewTab);
    settle(&mut test);
    settle(&mut test);
    assert!(sections.peek().object.is_none(), "the reading was kept");
    assert!(marked.peek().assembly.is_none());

    // Back to the code, and an answer with nothing decoded: the label is on another row.
    raise_document(&states, &code);
    settle(&mut test);
    settle(&mut test);
    assert!(
        sections.peek().is_about(&object),
        "the reading did not follow"
    );
    let guessed = reading_of(&object, &[]);
    let rows = rows_of(&guessed);
    let now = (0..rows.len())
        .find(|&row| row_line(&rows, &guessed, row) == "0000000000000014 twice:")
        .expect("the label has a row");
    assert_ne!(now, was, "the guess for add was exact, proving nothing");
    sections.set(guessed);
    settle(&mut test);
    settle(&mut test);
    let picked = marked
        .peek()
        .assembly
        .clone()
        .expect("the run did not come back");
    assert_eq!(picked.chars.lead(), Caret { row: now, col: 0 });
    assert_eq!(picked.rows.rows(), now..=now);
    assert!(picked.owed == Owed::default());
}

/// The slant of the label reading `text`, or `None` for an upright one.
fn label_slant(test: &TestingRunner, text: &str) -> Option<FontSlant> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find(|node, _element| {
        let element = node.element();
        let element = element.as_ref() as &dyn Any;
        element
            .downcast_ref::<LabelElement>()
            .filter(|label| label.text == text)
            .map(|label| label.text_style_data.font_slant)
    })
    .flatten()
}

/// The temporal tab's name is italic and a tab that stays is upright, which is the whole of
/// how the two are told apart; a double press on the header makes it stay. Headless
/// because the press count is freya's, and whether the header's handler sees it is a
/// question about the wiring.
#[test]
fn the_temporal_tabs_name_is_italic_and_a_double_press_makes_it_stay() {
    let symbols = fixture_symbols();
    let document = Document::Assembly(Selection::Symbol(symbols[0].clone()));
    let (mut test, states) = TestingRunner::new(
        header_menu_harness,
        (300., 100.).into(),
        project_states!(),
        1.,
    );
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    let id = open_document(states.open, states.visits, document.clone(), Reach::Preview)
        .expect("a document panel");
    settle(&mut test);

    let name = entry_text(&document);
    assert_eq!(label_slant(&test, &name), Some(FontSlant::Italic));

    // One press is a press: the tab stays temporal.
    let at = centre_of(&test, &name);
    press_at(&mut test, at);
    settle(&mut test);
    assert_eq!(states.open.docs.peek().temporal(), Some(id));

    // Two in quick succession promote it, and the name goes upright.
    test.move_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    test.press_cursor(at);
    test.release_cursor(at);
    settle(&mut test);
    assert_eq!(states.open.docs.peek().temporal(), None);
    assert_eq!(label_slant(&test, &name), None);
}

/// Under a filter the rows come back by how well they matched -- a prefix, then a word
/// start, then a substring, the shorter name first among equals and the list's own order
/// last -- and with nothing typed the list is left in its own order at no cost.
#[test]
fn a_filtered_list_puts_the_best_match_first() {
    let object = fixture_symbols()[0].object.clone();
    let symbol = |name: &str| Symbol {
        object: object.clone(),
        data: Arc::new(SymbolData {
            name: name.to_owned(),
            demangled: None,
            address: 0,
            section: None,
            size: 0,
        }),
    };
    let list = SymbolList(Arc::new(
        ["zz::next", "next_to", "std::next", "connext", "push"]
            .into_iter()
            .map(symbol)
            .collect(),
    ));

    let filtered = Filtered::new(list.clone(), &Filter::default().matcher());
    assert_eq!(filtered.len(), 5);
    assert!((0..5).all(|row| filtered.index(row) == row));

    let filter = Filter {
        pattern: "next".to_owned(),
        ..Filter::default()
    };
    let filtered = Filtered::new(list, &filter.matcher());
    let rows: Vec<usize> = (0..filtered.len()).map(|row| filtered.index(row)).collect();
    assert_eq!(rows, [1, 0, 2, 3]);
}

/// The chord is Ctrl+F alone -- `F` too, for Caps Lock -- and not Ctrl+Shift+F, which the
/// source search will want, nor a bare `f`.
#[test]
fn the_find_chord_is_ctrl_f_and_nothing_wider() {
    let f = Key::Character("f".into());
    let upper = Key::Character("F".into());
    assert!(is_find_chord(&f, Modifiers::CONTROL));
    assert!(is_find_chord(&upper, Modifiers::CONTROL));
    assert!(!is_find_chord(&f, Modifiers::CONTROL | Modifiers::SHIFT));
    assert!(!is_find_chord(&f, Modifiers::CONTROL | Modifiers::ALT));
    assert!(!is_find_chord(&f, Modifiers::default()));
    assert!(!is_find_chord(
        &Key::Character("g".into()),
        Modifiers::CONTROL
    ));
}

/// Ctrl+F reaches a filter box only from the list it filters: with nothing focused the
/// chord does nothing, and with the rows focused -- which a press on one does -- it puts
/// the keyboard in the box over them, where the text typed next lands and filters the
/// list. Fails on a binding on the root, which would answer the first press too.
#[test]
fn ctrl_f_reaches_the_filter_box_only_from_the_list_under_it() {
    let symbols = fixture_symbols();
    let names: Vec<&str> = symbols.iter().map(|symbol| symbol.data.display()).collect();
    assert!(names.contains(&"sum_to"));
    let other = names
        .iter()
        .find(|name| !name.contains("sum_to"))
        .expect("the fixture has a name beside sum_to")
        .to_string();

    let (mut test, states) =
        TestingRunner::new(symbols_harness, (300., 300.).into(), symbol_states!(), 1.);
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    settle(&mut test);
    assert!(labels(&test).iter().any(|label| label == &other));

    // Nothing focused: neither the text nor the chord reaches a box, and the chord is
    // not a way into one either.
    test.write_text("sum_to");
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    test.write_text("sum_to");
    settle(&mut test);
    assert!(labels(&test).iter().any(|label| label == &other));

    // A press on a row puts the keyboard on the rows, and from there the chord lands.
    // Settled between: a focus request is applied at the top of the pass after the one
    // that made it, and a key event is sent to whatever is focused when it is sent.
    let row = centre_of(&test, &other);
    press_at(&mut test, row);
    settle(&mut test);
    assert!(states.open.active().is_some(), "the press opened the row");
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    test.write_text("sum_to");
    settle(&mut test);
    let shown = labels(&test);
    assert!(shown.iter().any(|label| label == "sum_to"));
    assert!(!shown.iter().any(|label| label == &other));
}

/// The chord in the box it reaches is not typed into it: an `Input` inserts a character
/// it has no chord of its own for, so the pattern would grow an `f` without the bar's
/// pre-key hook declining it.
#[test]
fn the_chord_is_not_typed_into_the_box_it_reaches() {
    let symbols = fixture_symbols();
    let (mut test, states) =
        TestingRunner::new(symbols_harness, (300., 300.).into(), symbol_states!(), 1.);
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    settle(&mut test);

    let row = centre_of(&test, "sum_to");
    press_at(&mut test, row);
    settle(&mut test);
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    test.write_text("sum");
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    settle(&mut test);
    assert!(labels(&test).iter().any(|label| label == "sum_to"));
}

/// The Search panel over a work function the test hands in, so that a search can be held
/// still: a real walk answers faster than the runner settles, and superseding is a race
/// by construction.
#[derive(Clone)]
struct Walk(
    Arc<dyn Fn(&SearchQuery, &mut dyn FnMut(SearchEvent) -> ControlFlow<()>) + Send + Sync>,
);

fn search_harness() -> impl IntoElement {
    let searched = use_consume::<Searching>().0;
    let work = use_consume::<Walk>().0;
    use_search_with(searched, move |query, emit| work(query, emit));

    // What spends the landing a hit's press leaves, as `app()` does: without it a row
    // opens its tab and picks nothing out.
    let active = use_consume::<Active>().0;
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let open = use_open();
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );

    rect().expanded().child(SearchPanel)
}

/// The panel over `work`, with the project's directory set: a real directory of this
/// test's own, since a panel with none says so instead of drawing rows.
fn search_over(
    line: u32,
    work: impl Fn(&SearchQuery, &mut dyn FnMut(SearchEvent) -> ControlFlow<()>) + Send + Sync + 'static,
) -> (TestingRunner, ProjectStates, PathBuf, State<DockArea>) {
    let (test, states, directory, _, _, _, dock) = search_and_modifiers(line, work);
    (test, states, directory, dock)
}

/// The same, and the four states `ModifierKeys` is made of: a test cannot create a
/// `State` outside the runner's own context, so they are made where the rest are.
#[allow(clippy::type_complexity)]
fn search_and_modifiers(
    line: u32,
    work: impl Fn(&SearchQuery, &mut dyn FnMut(SearchEvent) -> ControlFlow<()>) + Send + Sync + 'static,
) -> (
    TestingRunner,
    ProjectStates,
    PathBuf,
    Modifiers5,
    State<Marks>,
    State<Finder>,
    State<DockArea>,
) {
    let directory = run_directory(line).join("searched");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let work = Arc::new(work);
    let (mut test, states) = TestingRunner::new(
        search_harness,
        (300., 400.).into(),
        move |runner: &mut _| {
            runner.provide_root_context({
                let work = work.clone();
                move || Walk(work.clone())
            });
            // What a row's press reaches through: the landing a hit makes, and the runs
            // it picks out on the other side of it.
            runner.provide_root_context(|| Land(State::create(None)));
            runner.provide_root_context(|| Plant(State::create(None)));
            runner.provide_root_context(|| CodeRows(State::create(None)));
            let held = runner.provide_root_context(|| {
                Modifiers5(
                    State::create(false),
                    State::create(false),
                    State::create(false),
                    State::create(false),
                    State::create(false),
                )
            });
            let marked = runner
                .provide_root_context(|| Marked(State::create(Marks::default())))
                .0;
            // The root's key handler answers the finder's chord beside the Search
            // panel's, so it needs the state the finder is opened through.
            let finder = runner
                .provide_root_context(|| Finding(State::create(Finder::default())))
                .0;
            let states = project_states!(runner);
            // The same context again, so this test holds the handle the panel reads.
            let dock = runner
                .provide_root_context(|| {
                    SidebarDock(State::create(DockArea::column(vec![vec![Panel::Search]])))
                })
                .0;
            (states, held, marked, finder, dock)
        },
        1.,
    );
    let mut proj = states.0.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    settle(&mut test);
    (
        test, states.0, directory, states.1, states.2, states.3, states.4,
    )
}

/// The five states `ModifierKeys` is made of, created where freya's context is: a test
/// cannot make a `State` of its own outside the runner.
#[derive(Clone, Copy)]
struct Modifiers5(
    State<bool>,
    State<bool>,
    State<bool>,
    State<bool>,
    State<bool>,
);

/// The panel over a walk that answers nothing, and the modifier states the root's one key
/// handler writes beside the chord.
#[allow(clippy::type_complexity)]
fn search_with_modifiers(
    line: u32,
) -> (
    TestingRunner,
    ProjectStates,
    PathBuf,
    ModifierKeys,
    State<bool>,
    State<bool>,
    State<Finder>,
    State<DockArea>,
) {
    let (test, states, directory, held, _, finder, dock) =
        search_and_modifiers(line, |_query, _emit| {});
    let keys = ModifierKeys::new(held.0, held.1, held.2, held.3, held.4);
    (test, states, directory, keys, held.0, held.1, finder, dock)
}

/// One hit, spelled as the walk spells one.
fn hit_at(path: &Path, line: u32, text: &str) -> Hit {
    Hit {
        path: path.to_path_buf(),
        line,
        text: text.to_owned(),
        spans: Vec::new(),
        columns: None,
    }
}

/// Ask for `pattern`, the way Enter in the box asks.
fn ask_for(states: &ProjectStates, dock: State<DockArea>, directory: &Path, pattern: &str) {
    start_search(
        states.searched,
        dock,
        SearchQuery {
            root: directory.to_path_buf(),
            filter: Filter {
                pattern: pattern.to_owned(),
                ..Filter::default()
            },
        },
    );
}

/// Hits arrive while the search runs, grouped under the file they are in, and a file row
/// folds its own away. Fails on rows built from anything but the streamed state.
#[test]
fn hits_arrive_under_their_file_and_fold() {
    let first = PathBuf::from("/project/one.rs");
    let second = PathBuf::from("/project/two.rs");
    let (one, two) = (first.clone(), second.clone());
    let (mut test, states, directory, dock) = search_over(line!(), move |_query, emit| {
        let _ = emit(SearchEvent::Hit(hit_at(&one, 3, "first hit")));
        let _ = emit(SearchEvent::Hit(hit_at(&one, 9, "second hit")));
        let _ = emit(SearchEvent::Hit(hit_at(&two, 1, "third hit")));
        let _ = emit(SearchEvent::Finished);
    });

    ask_for(&states, dock, &directory, "hit");
    let searched = states.searched;
    pump(&mut test, || !searched.peek().running);

    let shown = labels(&test);
    assert!(shown.iter().any(|label| label == "one.rs"), "{shown:?}");
    assert!(shown.iter().any(|label| label == "first hit"), "{shown:?}");
    assert!(shown.iter().any(|label| label == "third hit"), "{shown:?}");
    assert!(shown.iter().any(|label| label == "3 matches in 2 files"));

    let at = centre_of(&test, "one.rs");
    press_at(&mut test, at);
    settle(&mut test);

    let folded = labels(&test);
    assert!(folded.iter().any(|label| label == "one.rs"));
    assert!(
        !folded.iter().any(|label| label == "first hit"),
        "{folded:?}"
    );
    assert!(folded.iter().any(|label| label == "third hit"));
    let _ = std::fs::remove_dir_all(&directory);
}

/// A hit that lands after its search has been replaced is dropped: the batch is checked
/// against the search it belongs to before anything is written, so the old walk's last
/// answer cannot appear under the new question. Fails on a check made only where the loop
/// ends.
#[test]
fn a_hit_from_a_replaced_search_is_dropped() {
    let (gate, held) = std::sync::mpsc::channel::<()>();
    let held = Arc::new(std::sync::Mutex::new(held));
    let file = PathBuf::from("/project/one.rs");
    let (mut test, states, directory, dock) = search_over(line!(), move |query, emit| {
        if query.filter.pattern == "slow" {
            let _ = emit(SearchEvent::Hit(hit_at(&file, 1, "early answer")));
            // Held until the test has asked for something else.
            let _ = held.lock().expect("the gate").recv();
            let _ = emit(SearchEvent::Hit(hit_at(&file, 2, "late answer")));
            let _ = emit(SearchEvent::Finished);
            return;
        }
        let _ = emit(SearchEvent::Hit(hit_at(&file, 5, "other answer")));
        let _ = emit(SearchEvent::Finished);
    });

    let searched = states.searched;
    ask_for(&states, dock, &directory, "slow");
    pump(&mut test, || searched.peek().hits.counts().0 == 1);
    assert!(labels(&test).iter().any(|label| label == "early answer"));

    ask_for(&states, dock, &directory, "other");
    pump(&mut test, || !searched.peek().running);
    assert!(labels(&test).iter().any(|label| label == "other answer"));

    // The first walk goes on and answers into a channel nobody is taking from.
    gate.send(()).expect("the held search is still running");
    for _ in 0..40 {
        test.sync_and_update();
        std::thread::sleep(Duration::from_millis(2));
    }

    let shown = labels(&test);
    assert!(
        !shown.iter().any(|label| label == "late answer"),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|label| label == "early answer"),
        "{shown:?}"
    );
    assert!(
        shown.iter().any(|label| label == "1 match in 1 file"),
        "{shown:?}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Pressing a match opens its file as a source-driven tab landed on the line it was found
/// at, and a file the source pane would refuse opens nothing.
#[test]
fn pressing_a_hit_opens_its_file_on_the_line() {
    let (mut test, states, directory, _, marked, _, _) =
        search_and_modifiers(line!(), |_query, _emit| {});
    let path = directory.join("x.c");
    std::fs::write(&path, "int x;\nint y;\nint z;\n").expect("writing the source");
    let missing = directory.join("gone.c");

    let mut searched = states.searched;
    searched.write().asked = Some(SearchQuery {
        root: directory.clone(),
        filter: Filter {
            pattern: "y".to_owned(),
            ..Filter::default()
        },
    });
    searched.write().hits.push(Hit {
        columns: Some(4..5),
        ..hit_at(&path, 2, "int y;")
    });
    searched.write().hits.push(hit_at(&missing, 4, "gone"));
    settle(&mut test);

    let at = centre_of(&test, "gone");
    press_at(&mut test, at);
    settle(&mut test);
    assert!(
        states.open.active().is_none(),
        "a file that is not there opens nothing"
    );

    let at = centre_of(&test, "int y;");
    press_at(&mut test, at);
    settle(&mut test);

    let document = Document::Source(Arc::from(&*path.to_string_lossy()));
    assert!(states.open.active() == Some(document));

    // And the match itself is selected, not merely its line: the columns the hit came
    // with, over the file's own line, which is what Ctrl+C there would copy.
    let picked = marked
        .peek()
        .source
        .clone()
        .expect("the hit picked out its line");
    assert!(picked.rows.anchor == 1 && picked.rows.lead == 1);
    assert!(!picked.chars.is_empty(), "the match is selected");
    let copied = picked.chars.copy(|row| {
        assert!(row == 1);
        Line::text("int y;")
    });
    assert!(copied == "y", "{copied:?}");
    let _ = std::fs::remove_dir_all(&directory);
}

/// Enter in the box searches for what is in it, over the project's directory. The box is
/// reached by pressing it, as a reader reaches it.
#[test]
fn enter_in_the_box_asks_for_what_is_in_it() {
    let file = PathBuf::from("/project/one.rs");
    let (mut test, states, directory, _dock) = search_over(line!(), move |query, emit| {
        let _ = emit(SearchEvent::Hit(hit_at(
            &file,
            1,
            &format!("found {}", query.filter.pattern),
        )));
        let _ = emit(SearchEvent::Finished);
    });
    let searched = states.searched;
    assert!(searched.peek().asked.is_none());

    let at = centre_of(&test, "Search");
    press_at(&mut test, at);
    settle(&mut test);
    test.write_text("needle");
    settle(&mut test);
    println!("after typing: {:?}", labels(&test));
    key_with(&mut test, Key::Named(NamedKey::Enter), Modifiers::default());
    settle(&mut test);
    println!("after enter: {:?}", labels(&test));
    pump(&mut test, || !searched.peek().running);

    assert!(searched
        .peek()
        .asked
        .as_ref()
        .is_some_and(|query| query.filter.pattern == "needle" && query.root == directory));
    assert!(labels(&test).iter().any(|label| label == "found needle"));
    let _ = std::fs::remove_dir_all(&directory);
}

/// The root's one global key handler: Ctrl+Shift+F asks for the caret in the Search box,
/// **and** the modifiers go on being tracked, since both are that one handler. A second
/// `on_global_key_down` for the chord would replace the first and take Ctrl-click with it,
/// which nothing else here would notice.
#[test]
fn the_chord_asks_for_the_box_without_losing_the_modifiers() {
    // A runner, because every `State` here belongs to freya's own context, and the panel
    // is what spends what the chord asks for.
    let (mut test, states, directory, keys, shift, ctrl, finder, dock) =
        search_with_modifiers(line!());
    let searched = states.searched;
    let proj = states.proj;

    let chord = |key: Key, modifiers: Modifiers| {
        root_key_down(keys, searched, finder, proj, dock, &key, modifiers)
    };

    chord(Key::Character("f".into()), Modifiers::CONTROL);
    assert!(!searched.peek().focus, "Ctrl+F is the filter boxes' own");
    assert!(*ctrl.peek(), "and the modifier is tracked all the same");

    chord(
        Key::Character("F".into()),
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    assert!(searched.peek().focus, "the chord asks for the box");
    assert!(
        *ctrl.peek() && *shift.peek(),
        "and the modifiers still land"
    );

    // And the panel spends it: the caret is in the box, so what is typed next is the
    // pattern and not a keystroke into nothing.
    settle(&mut test);
    test.write_text("needle");
    settle(&mut test);
    assert!(
        !searched.peek().focus,
        "the flag is spent, not left standing"
    );
    assert!(labels(&test).iter().any(|label| label == "needle"));
    let _ = std::fs::remove_dir_all(&directory);
}

/// The chord is not typed into a filter box: the `Input`'s pre-key hook declines it, which
/// is also what keeps it reaching the root -- the hook's other arms call `prevent_default`,
/// and that cancels the global key event beside them.
#[test]
fn the_search_chord_is_declined_by_a_filter_box() {
    let symbols = fixture_symbols();
    let (mut test, states) =
        TestingRunner::new(symbols_harness, (300., 300.).into(), symbol_states!(), 1.);
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    settle(&mut test);

    let row = centre_of(&test, "sum_to");
    press_at(&mut test, row);
    settle(&mut test);
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    test.write_text("sum");
    key_with(
        &mut test,
        Key::Character("F".into()),
        Modifiers::CONTROL | Modifiers::SHIFT,
    );
    settle(&mut test);

    // The pattern is what was typed: an `F` in it would have filtered the row away.
    assert!(labels(&test).iter().any(|label| label == "sum_to"));
}

// ---------------------------------------------------------------------------------------
// Building the project's own workspace.

#[derive(Clone)]
struct BuildWorking(Arc<dyn Fn(BuildJob) -> BuildAnswer + Send + Sync>);

#[derive(Clone, Copy)]
struct BuildAsking(State<Option<BuildJobs>>);

/// What the worker was asked for, as a test can compare it.
#[derive(Debug, PartialEq, Eq)]
enum AskedToBuild {
    Read,
    Build,
    AddDebugLines,
}

fn project_view_harness() -> Element {
    build_wiring();
    // The section's two buttons reach the worker the same way the top bar's control does,
    // over a worker that starts nothing: what a press does to the state is the question.
    let states = use_project_states();
    let language = use_consume::<Talking>().0;
    // A start is answered, so that a press here can reach a running server the way the top
    // bar's does; and the project's own settings are really read, so the view lists what
    // that read answered and a file on disk is what it is being asked about.
    let follow = use_provide_root_context(|| Following(State::create(Follow::default()))).0;
    let linked = use_provide_root_context(|| Linking(State::create(Linked::default()))).0;
    let located = use_provide_root_context(|| Locations(State::create(Located::default()))).0;
    use_language_with(
        language,
        follow,
        located,
        linked,
        states.proj,
        |job: LspJob| match job {
            LspJob::ReadSettings { directory } => Some(LspAnswer::Settings {
                settings: lsp::settings_in(&directory),
                directory,
            }),
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(lsp::Handle::to_nothing()),
            }),
            _ => None,
        },
    );
    // The prompt the view's own Start button puts, drawn where the root draws it.
    rect()
        .expanded()
        .child(TrustPrompt)
        .child(ProjectTab)
        .into_element()
}

fn build_wiring() {
    let states = use_project_states();
    let work = use_consume::<BuildWorking>().0;
    let mut asking = use_consume::<BuildAsking>().0;

    let jobs = use_building_with(states.build, states, move |job| work(job));
    use_hook(move || asking.set(Some(jobs)));
}

/// Mount the Project view over a worker that records every job and answers from `answer`.
macro_rules! mount_project {
    ($answer:expr) => {{
        let (asked, asks) = async_channel::unbounded::<AskedToBuild>();
        let answer = $answer;
        let work = move |job: BuildJob| {
            let recorded = match &job {
                BuildJob::Read { .. } => AskedToBuild::Read,
                BuildJob::Build { .. } => AskedToBuild::Build,
                BuildJob::AddDebugLines { .. } => AskedToBuild::AddDebugLines,
            };
            let _ = asked.send_blocking(recorded);
            answer(job)
        };

        let (mut test, (states, language, asking)) = TestingRunner::new(
            project_view_harness,
            (600., 700.).into(),
            move |runner: &mut _| {
                let states = project_states!(runner);
                runner.provide_root_context(move || BuildWorking(Arc::new(work)));
                // The view says how the language server went; what it is is the root's.
                let language = runner
                    .provide_root_context(|| Talking(State::create(Language::default())))
                    .0;
                // A diagnostic's place is a link, and a link lands: the three states a
                // landing is left in.
                runner.provide_root_context(|| Marked(State::create(Marks::default())));
                runner.provide_root_context(|| Land(State::create(None)));
                runner.provide_root_context(|| Plant(State::create(None)));
                let asking = runner
                    .provide_root_context(|| BuildAsking(State::create(None)))
                    .0;
                (states, language, asking)
            },
            1.,
        );
        test.sync_and_update();

        (test, states, language, asking, asks)
    }};
}

/// A build whose artifacts are the two committed fixtures, so what is opened is a file
/// that really parses.
fn built(artifacts: &[PathBuf]) -> cargo::Run {
    cargo::Run::Built {
        artifacts: artifacts
            .iter()
            .map(|path| cargo::Artifact {
                path: path.clone(),
                target: "fixture".to_owned(),
                kind: "bin".to_owned(),
            })
            .collect(),
        diagnostics: Vec::new(),
    }
}

/// The whole of a build from the view: the button asks once however often it is pressed,
/// what cargo named becomes the rows, and pressing a row opens that file as a binary.
#[test]
fn a_build_lists_what_cargo_named_and_a_row_opens_it() {
    let artifact = fixture_artifact();
    let answer = {
        let artifact = artifact.clone();
        move |job: BuildJob| match job {
            BuildJob::Build { .. } => BuildAnswer::Done(built(&[artifact.clone()])),
            _ => BuildAnswer::Read {
                manifest: Some(PathBuf::from("/work/app/Cargo.toml")),
                debug_lines: true,
            },
        }
    };
    let (mut test, states, _language, asking, asks) = mount_project!(answer);

    let mut proj = states.proj;
    proj.write().directory = "/work/app".to_owned();
    pump(&mut test, || states.build.peek().manifest.is_some());
    assert_eq!(asks.try_recv(), Ok(AskedToBuild::Read));

    // The file cargo would be run over, named rather than left to be worked out from the
    // directory above it.
    let drawn = labels(&test);
    assert!(drawn.iter().any(|text| text == "Cargo build"), "{drawn:?}");
    assert!(
        drawn.iter().any(|text| text == "/work/app/Cargo.toml"),
        "{drawn:?}"
    );

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    start_build(
        states.build,
        &jobs,
        PathBuf::from("/work/app"),
        Profile::Release,
    );
    // The second press, while the first is still in flight. Nothing at all happens.
    start_build(
        states.build,
        &jobs,
        PathBuf::from("/work/app"),
        Profile::Release,
    );
    assert!(states.build.peek().building);

    pump(&mut test, || !states.build.peek().building);
    assert_eq!(asks.try_recv(), Ok(AskedToBuild::Build));
    assert!(
        asks.is_empty(),
        "the second press started a second build of the same workspace"
    );
    assert_eq!(states.build.peek().artifacts().len(), 1);
    assert_eq!(states.build.peek().previous, vec![artifact.clone()]);

    // A row per artifact, naming the file and the target it came from. Nothing is opened
    // until the reader asks for it.
    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text.contains("line_fixture.o")),
        "{drawn:?}"
    );
    assert!(states.objects.peek().is_empty());

    let row = centre_of(&test, &artifact.to_string_lossy());
    press_at(&mut test, row);
    pump(&mut test, || !states.objects.peek().is_empty());
    assert!(states
        .objects
        .peek()
        .iter()
        .any(|object| object.path == artifact));
}

/// The rule the session carries: a build replaces the artifacts of the build **before**
/// it, and leaves a binary the reader opened some other way alone -- even at a path this
/// build also wrote.
#[test]
fn a_build_replaces_what_the_build_before_it_produced() {
    let artifact = fixture_artifact();
    let other = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/analysis/tests/fixtures/line_fixture_hidden.so");
    let answer = {
        let artifact = artifact.clone();
        move |job: BuildJob| match job {
            BuildJob::Build { .. } => BuildAnswer::Done(built(&[artifact.clone()])),
            _ => BuildAnswer::Read {
                manifest: Some(PathBuf::from("/work/app/Cargo.toml")),
                debug_lines: true,
            },
        }
    };
    let (mut test, states, _language, asking, _asks) = mount_project!(answer);
    let jobs = asking.peek().clone().expect("the wiring handed one back");

    // Both files are open, and only one of them is the last build's.
    let mut build = states.build;
    build.write().previous = vec![artifact.clone()];
    let mut objects = states.objects;
    objects.set(analysis::open_files(vec![artifact.clone(), other.clone()]));
    test.sync_and_update();

    // Identity and not a count: a file closed and reopened has the same number of objects
    // and different `Arc`s, and which of the two happened is the whole of the rule. Pointer
    // identity is what this app means by identity everywhere else, too.
    let held = |path: &Path| {
        states
            .objects
            .peek()
            .iter()
            .filter(|object| object.path == path)
            .map(|object| Arc::as_ptr(object).addr())
            .collect::<Vec<usize>>()
    };
    let (before_built, before_other) = (held(&artifact), held(&other));
    assert!(!before_built.is_empty() && !before_other.is_empty());

    start_build(build, &jobs, PathBuf::from("/work/app"), Profile::Release);
    // Both halves: the close empties the list for that path a beat before the reopen
    // fills it, and waiting only for "different" would land in that gap.
    pump(&mut test, || {
        let now = held(&artifact);
        !build.peek().building && !now.is_empty() && now != before_built
    });

    assert_eq!(
        held(&artifact).len(),
        before_built.len(),
        "the artifact was left in the list twice, or dropped"
    );
    assert_ne!(
        held(&artifact),
        before_built,
        "the artifact was not reopened, so the objects describe bytes that are gone"
    );
    assert_eq!(
        held(&other),
        before_other,
        "a binary the reader opened was closed by a build that did not produce it"
    );
}

/// A directory with no manifest is a placeholder and not an error: the section says so,
/// there is nothing to choose, and the button is dimmed rather than taken away -- a press
/// on it starts nothing.
#[test]
fn a_directory_with_no_manifest_builds_nothing() {
    let (mut test, states, _language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });

    let mut proj = states.proj;
    proj.write().directory = "/work/not-a-workspace".to_owned();
    pump(&mut test, || states.build.peek().manifest.is_none());

    let drawn = labels(&test);
    assert!(
        drawn
            .iter()
            .any(|text| text == "No Cargo.toml in the directory"),
        "{drawn:?}"
    );
    // Nothing to choose either: the profile and the offer are about a manifest.
    assert!(!drawn.iter().any(|text| text == "Release"), "{drawn:?}");

    // The button is drawn and dimmed rather than taken away, and a press on it does
    // nothing at all.
    let button = centre_of(&test, "Build");
    press_at(&mut test, button);
    test.sync_and_update();
    assert!(!states.build.peek().building);
}

/// The offer, which is the whole reason the profile and the manifest are read together: a
/// profile carrying no lines has no source side, and taking the offer is what makes the
/// line go.
#[test]
fn a_profile_with_no_debug_lines_offers_them_and_the_offer_goes() {
    let added = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let answer = {
        let added = added.clone();
        move |job: BuildJob| match job {
            BuildJob::AddDebugLines { .. } => {
                added.store(true, std::sync::atomic::Ordering::SeqCst);
                BuildAnswer::Read {
                    manifest: Some(PathBuf::from("/work/app/Cargo.toml")),
                    debug_lines: true,
                }
            }
            _ => BuildAnswer::Read {
                manifest: Some(PathBuf::from("/work/app/Cargo.toml")),
                debug_lines: added.load(std::sync::atomic::Ordering::SeqCst),
            },
        }
    };
    let (mut test, states, _language, _asking, _asks) = mount_project!(answer);

    let mut proj = states.proj;
    proj.write().directory = "/work/app".to_owned();
    pump(&mut test, || states.build.peek().manifest.is_some());

    let drawn = labels(&test);
    assert!(
        drawn
            .iter()
            .any(|text| text == "Off, so there is no source side"),
        "{drawn:?}"
    );

    let button = centre_of(&test, "Turn on");
    press_at(&mut test, button);
    pump(&mut test, || states.build.peek().debug_lines);

    let drawn = labels(&test);
    assert!(
        !drawn
            .iter()
            .any(|text| text == "Off, so there is no source side"),
        "{drawn:?}"
    );
}

/// A diagnostic's place is a target when this pane can reach it: cargo spells the file
/// relative to where it ran, so the project's directory joined with it is the file, and
/// pressing it opens that file as source on the line the compiler named. A place in a
/// dependency stays a plain label, since the app opens a file it can read and a target that
/// did nothing when pressed would be worse than none.
#[test]
fn a_diagnostics_place_opens_the_file_it_names() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let own = cargo::Span {
        file: "src/main.rs".to_owned(),
        line: 2,
        column: 1,
    };
    let elsewhere = cargo::Span {
        file: "/home/reader/.cargo/registry/src/index/serde-1.0/src/lib.rs".to_owned(),
        line: 9,
        column: 4,
    };
    let said = |span: &cargo::Span| cargo::Diagnostic {
        level: Level::Error,
        message: "mismatched types".to_owned(),
        rendered: "error: mismatched types".to_owned(),
        span: Some(span.clone()),
    };
    let run = cargo::Run::Rejected {
        diagnostics: vec![said(&own), said(&elsewhere)],
        message: String::new(),
    };

    let manifest = directory.join("Cargo.toml");
    let (mut test, states, _language, _asking, _asks) =
        mount_project!(move |_: BuildJob| BuildAnswer::Read {
            manifest: Some(manifest.clone()),
            debug_lines: true,
        });

    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    // The section is drawn once the manifest has been read, which is a worker's answer away.
    pump(&mut test, || states.build.peek().manifest.is_some());
    let mut build = states.build;
    build.write().built = Some(run);
    settle(&mut test);

    // Both places are drawn, each spelled as the file, the line and the column.
    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "src/main.rs:2:1"),
        "{drawn:?}"
    );
    assert!(drawn.iter().any(|text| text == "lib.rs:9:4"), "{drawn:?}");

    // The dependency's is a label and nothing else: pressing it opens nothing.
    let plain = centre_of(&test, "lib.rs:9:4");
    press_at(&mut test, plain);
    assert!(
        states.open.documents().is_empty(),
        "a place with nowhere to go opened a tab"
    );

    let own_place = centre_of(&test, "src/main.rs:2:1");
    press_at(&mut test, own_place);
    settle(&mut test);

    let file = Arc::<str>::from(&*directory.join("src/main.rs").to_string_lossy());
    assert!(
        states.open.documents() == [Document::Source(file)],
        "the place did not open the file it names"
    );
}

/// The window over a rescue: it names every path it was given, and its button empties the
/// list -- which is the same list it is drawn from, so the window goes with it.
#[test]
fn the_rescued_window_names_every_path_and_its_button_empties_the_list() {
    fn rescued_harness() -> impl IntoElement {
        rect().expanded().child(RescuedPopup)
    }

    let paths = [
        "/state/incompatible/settings.toml",
        "/state/incompatible/projects/project-1/session.toml",
    ];
    let (mut test, rescued) = TestingRunner::new(
        rescued_harness,
        (600., 400.).into(),
        |runner: &mut _| {
            runner
                .provide_root_context(|| {
                    Rescued(State::create(paths.iter().map(PathBuf::from).collect()))
                })
                .0
        },
        1.,
    );
    settle(&mut test);

    for path in paths {
        assert!(label_area(&test, path).is_some(), "{path} was not named");
    }

    press(&mut test, "Close");
    assert!(rescued.peek().is_empty(), "the window kept what it named");

    // And a run in which nothing was moved has no window at all, this being mounted at
    // the root of every one of them.
    let (quiet, _) = TestingRunner::new(
        rescued_harness,
        (600., 400.).into(),
        |runner: &mut _| {
            runner
                .provide_root_context(|| Rescued(State::create(Vec::new())))
                .0
        },
        1.,
    );
    assert!(label_area(&quiet, "Close").is_none());
}

// ---------------------------------------------------------------------------------------
// The language server and its control.

#[derive(Clone)]
struct ServerWorking(Arc<dyn Fn(LspJob) -> Option<LspAnswer> + Send + Sync>);

#[derive(Clone, Copy)]
struct ServerAsking(State<Option<LspJobs>>);

/// What the worker was asked for, as a test can compare it.
#[derive(Debug, PartialEq, Eq)]
enum AskedOfServer {
    Start(PathBuf),
    Ask(Lookup, Wanted),
    Tokens(Arc<str>),
    Read(PathBuf),
    Stop,
}

fn server_harness() -> Element {
    let states = use_project_states();
    let language = use_consume::<Talking>().0;
    let work = use_consume::<ServerWorking>().0;
    let mut asking = use_consume::<ServerAsking>().0;

    let follow = use_consume::<Following>().0;
    let located = use_provide_root_context(|| Locations(State::create(Located::default()))).0;
    let linked = use_provide_root_context(|| Linking(State::create(Linked::default()))).0;
    let jobs = use_language_with(language, follow, located, linked, states.proj, move |job| {
        work(job)
    });
    use_hook(move || asking.set(Some(jobs)));
    // The prompt is at the app's root, under the bar the control is in; here it is beside
    // it, which is the same thing to everything that reaches it.
    rect()
        .expanded()
        .child(ServerButton)
        .child(TrustPrompt)
        .into_element()
}

/// Mount the control over a worker that records every job and answers from `answer`.
///
/// The second form mounts over a project that is already there, which is what a restore
/// leaves behind: it is set before the first render, so what the effects see on mount is
/// the reopened project and not the empty one.
macro_rules! mount_server {
    ($answer:expr) => {
        mount_server!($answer, OpenProject::default())
    };
    ($answer:expr, $open:expr) => {{
        let (asked, asks) = async_channel::unbounded::<AskedOfServer>();
        let answer = $answer;
        let work = move |job: LspJob| {
            let recorded = match &job {
                LspJob::Start { directory, .. } => AskedOfServer::Start(directory.clone()),
                LspJob::Ask { at, want, .. } => AskedOfServer::Ask(at.clone(), *want),
                LspJob::Tokens { file, .. } => AskedOfServer::Tokens(file.clone()),
                LspJob::ReadSettings { directory } => AskedOfServer::Read(directory.clone()),
                LspJob::Stop => AskedOfServer::Stop,
            };
            let _ = asked.send_blocking(recorded);
            answer(job)
        };

        let (mut test, (states, language, asking)) = TestingRunner::new(
            server_harness,
            (200., 100.).into(),
            move |runner: &mut _| {
                let states = project_states!(runner);
                let mut proj = states.proj;
                proj.set($open);
                runner.provide_root_context(move || ServerWorking(Arc::new(work)));
                let language = runner
                    .provide_root_context(|| Talking(State::create(Language::default())))
                    .0;
                let asking = runner
                    .provide_root_context(|| ServerAsking(State::create(None)))
                    .0;
                runner.provide_root_context(|| Following(State::create(Follow::default())));
                runner.provide_root_context(|| Linking(State::create(Linked::default())));
                (states, language, asking)
            },
            1.,
        );
        test.sync_and_update();

        (test, states, language, asking, asks)
    }};
}

/// The next job the worker was given, waited for: it takes them on a thread of its own.
///
/// Reading the project's own settings is passed over. It follows the project rather than
/// a press, so every test with a directory in it would otherwise have to say so; the two
/// that are about it read the state it fills instead.
fn next_job(asks: &async_channel::Receiver<AskedOfServer>) -> Option<AskedOfServer> {
    for _ in 0..500 {
        match asks.try_recv() {
            Ok(AskedOfServer::Read(_)) => continue,
            Ok(asked) => return Some(asked),
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    None
}

/// Whether the worker was asked for nothing a press asks for, the settings read that
/// follows a project aside.
fn nothing_pressed(asks: &async_channel::Receiver<AskedOfServer>) -> bool {
    std::iter::from_fn(|| asks.try_recv().ok()).all(|asked| matches!(asked, AskedOfServer::Read(_)))
}

/// The middle of the control, which is the only thing in its harness.
fn the_control() -> (f64, f64) {
    let side = toggle_size();
    ((side / 2.0) as f64, (side / 2.0) as f64)
}

/// Whether anything on screen is bordered in `colour`, which is how the control says which
/// state it is in.
fn bordered_in(test: &TestingRunner, colour: Color) -> bool {
    test.find(|_, element| {
        element
            .style()
            .borders
            .iter()
            .any(|border| border.fill == colour)
            .then_some(())
    })
    .is_some()
}

/// Whether the control is drawn as the app's one coloured control, which is the whole of
/// "a server is running" as the runner can see it.
fn control_is_lit(test: &TestingRunner) -> bool {
    test.find(|_, element| {
        (element.style().background == Fill::Color(Palette::LIGHT.server_bg)).then_some(())
    })
    .is_some()
}

/// Sync until the control is in the state wanted, since the worker is a thread of its own
/// and its answer arrives when it arrives -- and then a little further, so that what is
/// drawn is that state and not the one before it. A pass is a render or a round of task
/// polling and never both, so the pass that takes the answer draws nothing; ending on it
/// leaves the control in the state before, which is what an assertion about its border
/// then reads. `pump` settles again for the same reason.
///
/// Gives up rather than hanging: the assertion after it is what says what went wrong.
fn until_server(test: &mut TestingRunner, language: State<Language>, wanted: &Lsp) {
    for _ in 0..500 {
        settle(test);
        // Bound to a `let` of its own: the settle below polls the task that writes the
        // very state this read.
        let arrived = &language.read().state == wanted;
        if arrived {
            settle(test);
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A project with somewhere to run a server over, and a reader who has agreed to it.
///
/// Two writes and a settle between them: the directory is what drops an agreement, so
/// one made in the same write would be cleared by the effect that notices the change.
/// The prompt is its own tests' subject.
fn with_a_directory(test: &mut TestingRunner, states: &ProjectStates, directory: &str) {
    let mut proj = states.proj;
    proj.write().directory = directory.to_owned();
    settle(test);
    proj.write().trusted = true;
    settle(test);
}

/// The control is what starts a server: nothing else does, the worker is asked over the
/// project's own directory, and the control lights when the answer comes back.
#[test]
fn the_control_starts_a_server_and_lights_when_it_answers() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");

    // Mounting one, and giving the project a directory, are not asking for a server.
    assert!(nothing_pressed(&asks), "a server was started by itself");
    assert_eq!(language.read().state, Lsp::Off);

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);

    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert_eq!(language.read().state, Lsp::Running);
    assert!(control_is_lit(&test), "a running server is not lit");
}

/// The control says what it is and how it is going: its name in words, and a border that
/// is the failure's colour when there is one.
#[test]
fn the_control_is_named_and_bordered_in_the_state_it_is_in() {
    let (mut test, states, language, _asking, _asks) = mount_server!(|job: LspJob| match job {
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Err(lsp::Failure::NoServer("not found".to_owned())),
        }),
        _ => None,
    });
    with_a_directory(&mut test, &states, "/p");

    assert!(
        labels(&test).iter().any(|label| label == "LSP"),
        "the control does not say what it is: {:?}",
        labels(&test)
    );
    // Off and untouched it is text alone: nothing has been started, so nothing is drawn
    // as holding anything.
    assert!(
        !bordered_in(&test, Palette::LIGHT.hairline),
        "a control nobody has asked anything of is drawn as a box"
    );
    test.move_cursor(the_control());
    settle(&mut test);
    assert!(
        bordered_in(&test, Palette::LIGHT.hairline),
        "the pointer over it does not say a press would do something"
    );

    press_at(&mut test, the_control());
    until_server(
        &mut test,
        language,
        &Lsp::Failed(lsp::Failure::NoServer("not found".to_owned()).to_string()),
    );

    assert!(
        bordered_in(&test, Palette::LIGHT.invalid_fg),
        "a failure is not on the control"
    );
    // And the name does not change with the state: the history buttons sit beside it.
    assert!(labels(&test).iter().any(|label| label == "LSP"));
}

/// The next press stops it: the process is killed from here rather than asked to leave,
/// and the worker is told to let go of what it was talking to.
#[test]
fn the_next_press_stops_the_server() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);
    assert_eq!(language.read().state, Lsp::Running);

    press_at(&mut test, the_control());
    settle(&mut test);

    assert_eq!(language.read().state, Lsp::Off);
    assert!(
        handle.finished(),
        "the server was let go of instead of killed"
    );
    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert_eq!(next_job(&asks), Some(AskedOfServer::Stop));
    assert!(!control_is_lit(&test), "a stopped server is still lit");
}

/// A rust-analyzer that is not installed is what most machines will answer, and it is not
/// an error anywhere: the control goes back to off, and the reason is on it.
#[test]
fn a_server_that_will_not_start_leaves_the_reason_on_the_control() {
    let (mut test, states, language, _asking, _asks) = mount_server!(|job: LspJob| match job {
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Err(lsp::Failure::NoServer("not found".to_owned())),
        }),
        _ => None,
    });
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    until_server(
        &mut test,
        language,
        &Lsp::Failed(lsp::Failure::NoServer("not found".to_owned()).to_string()),
    );

    let state = language.read().state.clone();
    let Lsp::Failed(why) = state else {
        panic!("a server that would not start left the control at {state:?}");
    };
    assert!(why.contains("not found"), "{why}");
    assert!(!control_is_lit(&test), "a server that never started is lit");

    // And what the control says on hover is that reason, which is the only place it is
    // said. The tooltip itself is half a second of real time away (`agents/Headless.md`),
    // so what is asserted is the words it would be given.
    assert!(
        language.read().words().contains("not found"),
        "the control would say nothing about why"
    );

    // And leaving the project takes the reason with it: it was about that project.
    with_a_directory(&mut test, &states, "/elsewhere");
    assert_eq!(language.read().state, Lsp::Off);
}

/// An answer about a server nobody is waiting for any more is dropped -- and the handle it
/// carries is stopped rather than dropped, this being the first moment the app holds it.
#[test]
fn an_answer_for_a_server_that_was_stopped_is_dropped() {
    let late = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, _asks) = mount_server!({
        let late = late.clone();
        move |job: LspJob| match job {
            // An answer for the run before this one, which is what a start that was
            // stopped while it was starting looks like from here.
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run: run.saturating_sub(1),
                server: Ok(late.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    for _ in 0..500 {
        settle(&mut test);
        if late.finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(
        language.read().state,
        Lsp::Starting,
        "an answer for another run was taken"
    );
    assert!(
        late.finished(),
        "a server nobody wanted was dropped instead of stopped"
    );
}

/// Leaving the project ends its server: it was started over that directory, and the next
/// project is not what it read.
#[test]
fn changing_the_project_stops_the_server() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);
    assert_eq!(language.read().state, Lsp::Running);

    with_a_directory(&mut test, &states, "/elsewhere");

    assert_eq!(language.read().state, Lsp::Off);
    assert!(
        handle.finished(),
        "the old project's server is still running"
    );
    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert_eq!(next_job(&asks), Some(AskedOfServer::Stop));
}

/// A question is only asked while a server is running: nothing about a question starts
/// one, which is the other half of "the control is what starts it".
#[test]
fn a_question_asked_with_no_server_running_asks_nobody() {
    let (mut test, states, language, asking, asks) = mount_server!(|job: LspJob| match job {
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Err(lsp::Failure::NoServer("not found".to_owned())),
        }),
        _ => None,
    });
    with_a_directory(&mut test, &states, "/p");

    let jobs = asking.read().clone().expect("the worker");
    let at = Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line: 12,
        column: 4,
    };
    ask_where(language, &jobs, at.clone(), Wanted::Definition);
    settle(&mut test);
    assert!(
        nothing_pressed(&asks),
        "a question was asked with nothing to ask"
    );

    // A server that was asked for and did not start is not one to ask either. The state
    // is what says the worker has been round: the start is recorded before it answers.
    press_at(&mut test, the_control());
    until_server(
        &mut test,
        language,
        &Lsp::Failed(lsp::Failure::NoServer("not found".to_owned()).to_string()),
    );
    assert!(matches!(language.read().state, Lsp::Failed(_)));
    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );

    ask_where(language, &jobs, at, Wanted::Definition);
    settle(&mut test);
    assert!(
        nothing_pressed(&asks),
        "a question was asked of a server that never started"
    );
}

/// The Project view says how the language server went, the failure that keeps it from
/// running included: the control in the top bar is a tooltip, and a reason worth reading
/// twice belongs somewhere it stays.
#[test]
fn the_project_view_says_how_the_language_server_went() {
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });

    let says = |test: &TestingRunner, wanted: &str| {
        labels(test).iter().any(|label| label.starts_with(wanted))
    };

    // With no directory there is nothing to run one over, and that is what it says.
    assert!(says(&test, "No directory"));

    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);
    assert!(says(&test, "Not running"), "{:?}", labels(&test));

    let mut language = language;
    let failed = "could not start rust-analyzer: not found";
    let mut next = language.peek().clone();
    next.state = Lsp::Failed(failed.to_owned());
    language.set(next);
    settle(&mut test);
    assert!(
        says(&test, failed),
        "the reason is nowhere in the view: {:?}",
        labels(&test)
    );
}

/// A directory of this test's own with `text` in its `.vscode/settings.json`, named after
/// the line that asked for it.
fn a_project_with_settings(line: u32, text: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-lsp-settings-{}-{line}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join(".vscode")).expect("creating the test directory");
    std::fs::write(directory.join(".vscode").join("settings.json"), text)
        .expect("writing the settings file");
    directory
}

/// The colour a label was drawn in, of the first one holding `has`.
fn label_colour(test: &TestingRunner, has: &str) -> Option<Fill> {
    use freya::elements::label::LabelElement;
    use std::any::Any;

    test.find(|node, _element| {
        let element = node.element();
        (element.as_ref() as &dyn Any)
            .downcast_ref::<LabelElement>()
            .filter(|label| label.text.contains(has))
            .map(|label| label.text_style_data.color.clone())
    })
    .flatten()
}

/// The Project view lists what the project's own settings file gave the server: the name
/// with `rust-analyzer.` off it and the value as it will be sent, so a reader can see what
/// theirs is being told. The editor's own keys are not the server's and are not listed.
#[test]
fn the_project_view_lists_the_settings_the_project_gave_the_server() {
    let directory = a_project_with_settings(
        line!(),
        r#"{
            // the tree's own
            "rust-analyzer.cargo.features": ["one"],
            "git.detectSubmodulesLimit": 20
        }"#,
    );
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });

    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    pump(&mut test, || !language.peek().overrides().is_empty());

    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text == "cargo.features"),
        "{drawn:?}"
    );
    assert!(drawn.iter().any(|text| text == r#"["one"]"#), "{drawn:?}");
    assert!(
        drawn.iter().any(|text| text.contains(lsp::SETTINGS)),
        "the file the settings came out of is not named: {drawn:?}"
    );
    assert!(
        !drawn.iter().any(|text| text.contains("detectSubmodules")),
        "an editor's own setting was listed: {drawn:?}"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// And says why a settings file could not be used, in the colour the other failure in that
/// section is in: it is read whether or not a server is running, and it is the one thing
/// that stops a start before it is one.
#[test]
fn the_project_view_says_why_a_settings_file_could_not_be_used() {
    let directory = a_project_with_settings(
        line!(),
        r#"{ "rust-analyzer.cargo.sysrootSrc": "${userHome}/rust" }"#,
    );
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });

    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    pump(&mut test, || language.peek().unreadable().is_some());

    let drawn = labels(&test);
    assert!(
        drawn.iter().any(|text| text.contains("userHome")),
        "the variable nothing could be put in place of is not named: {drawn:?}"
    );
    assert_eq!(
        label_colour(&test, "userHome"),
        Some(Fill::Color(Palette::LIGHT.invalid_fg)),
        "a settings file that could not be read is not said to be a failure"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// Which language server a project is read with is the project's own setting: typed into
/// the view, saved with it, and what the press actually starts.
#[test]
fn the_project_names_the_language_server_it_is_read_with() {
    let (mut test, states, _language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });
    let mut proj = states.proj;

    // Unsaid, it is the usual one, and nothing is written into the file about it.
    assert_eq!(proj.read().server(), lsp::SERVER);
    assert_eq!(proj.read().details().language_server, None);

    proj.write().language_server = "  ra-multiplex  ".to_owned();
    settle(&mut test);
    assert_eq!(proj.read().server(), "ra-multiplex");
    // Trimmed on the way to the file, as the name and the directory are.
    assert_eq!(
        proj.read().details().language_server.as_deref(),
        Some("ra-multiplex")
    );
}

/// The view's own button starts the server and then stops it, the same two presses the
/// top bar's control is, so a reader who is in the Project view need not go looking.
#[test]
fn the_project_views_button_starts_and_stops_the_language_server() {
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);
    proj.write().trusted = true;
    settle(&mut test);

    let start = centre_of(&test, "Start");
    press_at(&mut test, start);
    until_server(&mut test, language, &Lsp::Running);
    assert_eq!(language.read().state, Lsp::Running);

    // And the button is the other one now, which is the whole of it being one control.
    let stop = centre_of(&test, "Stop");
    press_at(&mut test, stop);
    settle(&mut test);
    assert_eq!(language.read().state, Lsp::Off);
}

/// A server reading the project says so through the same channel it was started with, and
/// the control and the Project view both say it is working rather than that it is idle.
#[test]
fn a_server_reading_the_project_says_so_and_the_control_shows_it() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, _asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, notes, .. } => {
                // What the server's own reader thread does when `$/progress` arrives.
                let _ = notes.send_blocking((run, lsp::Note::Busy(true)));
                Some(LspAnswer::Started {
                    run,
                    server: Ok(handle.clone()),
                })
            }
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);
    for _ in 0..500 {
        settle(&mut test);
        if language.read().working {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let held = language.read().clone();
    assert!(
        held.working,
        "the server's own account of itself was dropped"
    );
    // What the control draws a turning loader for instead of its icon, and what both
    // places say in words.
    assert!(held.busy());
    assert!(
        held.words().contains("reading the project"),
        "{}",
        held.words()
    );
    assert_eq!(held.status(true).0, "Reading the project...");

    // And it is still running: working is what it is doing, not a state of its own.
    assert!(control_is_lit(&test), "a working server is not lit");
}

/// A question asked while the server is still starting is asked all the same: it queues
/// behind the start, and is answered once there is somebody to answer it.
#[test]
fn a_question_asked_while_it_is_starting_waits_for_it() {
    // Nothing answers the start, so the control stays on `Starting`.
    let (mut test, states, language, asking, asks) = mount_server!(|_: LspJob| None);
    with_a_directory(&mut test, &states, "/p");

    press_at(&mut test, the_control());
    settle(&mut test);
    assert_eq!(language.read().state, Lsp::Starting);

    let jobs = asking.read().clone().expect("the worker");
    let at = Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line: 3,
        column: 0,
    };
    ask_where(language, &jobs, at.clone(), Wanted::Definition);

    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Ask(at, Wanted::Definition))
    );
}

/// A start carries what the project's own settings file said, laid over what this app asks
/// of every server. Both halves are silent when they are wrong -- a server ignores a key
/// that kept its prefix and one whose dots were not split -- so what the worker is handed
/// is the assertion.
#[test]
fn a_start_carries_the_projects_own_settings() {
    let read = lsp::settings_from(
        r#"{ "rust-analyzer.cargo.features": ["one"] }"#,
        Path::new("/p"),
    )
    .expect("a file that reads");
    let (sent, options) = async_channel::unbounded::<String>();
    let (mut test, states, language, _asking, _asks) = mount_server!({
        move |job: LspJob| match job {
            LspJob::ReadSettings { directory } => Some(LspAnswer::Settings {
                settings: Ok(read.clone()),
                directory,
            }),
            LspJob::Start { run, settings, .. } => {
                let _ = sent.send_blocking(settings.options().to_string());
                Some(LspAnswer::Started {
                    run,
                    server: Ok(lsp::Handle::to_nothing()),
                })
            }
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");
    pump(&mut test, || !language.peek().overrides().is_empty());

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);

    let options = options.try_recv().expect("the start carried options");
    assert_eq!(
        options,
        r#"{"cargo":{"features":["one"]},"checkOnSave":false}"#
    );
}

/// A settings file that could not be used starts nothing, and the reason is where a
/// failure to start is said. What would otherwise reach the server is a path that silently
/// is not there, which is worse than not starting.
#[test]
fn a_settings_file_that_could_not_be_read_starts_nothing() {
    let (mut test, states, language, _asking, asks) = mount_server!(|job: LspJob| match job {
        LspJob::ReadSettings { directory } => Some(LspAnswer::Settings {
            settings: Err(lsp::Unreadable::NotAnObject),
            directory,
        }),
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Ok(lsp::Handle::to_nothing()),
        }),
        _ => None,
    });
    with_a_directory(&mut test, &states, "/p");
    pump(&mut test, || language.peek().unreadable().is_some());

    press_at(&mut test, the_control());
    settle(&mut test);

    let held = language.read().clone();
    assert_eq!(
        held.state,
        Lsp::Failed(lsp::Unreadable::NotAnObject.to_string())
    );
    assert!(nothing_pressed(&asks), "a server was started all the same");
}

/// The queue is drained to the last question, and to every start and stop: a reader
/// clicking twice wants the second answer, and a press is never dropped.
#[test]
fn the_queue_keeps_the_last_question_and_every_press() {
    let at = |line: u32| Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line,
        column: 0,
    };
    let names = |jobs: &[LspJob]| -> Vec<String> {
        jobs.iter()
            .map(|job| match job {
                LspJob::Start { .. } => "start".to_owned(),
                LspJob::Ask { at, .. } => format!("ask {}", at.line),
                LspJob::Tokens { file, .. } => format!("tokens {file}"),
                LspJob::ReadSettings { directory } => format!("read {}", directory.display()),
                LspJob::Stop => "stop".to_owned(),
            })
            .collect()
    };

    let drained = worth_doing(
        LspJob::Ask {
            run: 1,
            at: at(1),
            want: Wanted::Definition,
        },
        vec![
            LspJob::Ask {
                run: 1,
                at: at(2),
                want: Wanted::Definition,
            },
            LspJob::ReadSettings {
                directory: PathBuf::from("/old"),
            },
            LspJob::Stop,
            LspJob::Start {
                run: 2,
                directory: PathBuf::from("/p"),
                program: "rust-analyzer".to_owned(),
                settings: lsp::Settings::none(),
                notes: async_channel::unbounded().0,
            },
            LspJob::ReadSettings {
                directory: PathBuf::from("/p"),
            },
            LspJob::Ask {
                run: 2,
                at: at(3),
                want: Wanted::Definition,
            },
        ]
        .into_iter(),
    );
    // The last read as well as the last question: a directory typed a letter at a time
    // asks for one a keystroke, and only the last is about the project that is open.
    assert_eq!(names(&drained), ["stop", "start", "read /p", "ask 3"]);

    // One of a kind is simply itself.
    let alone = worth_doing(
        LspJob::Ask {
            run: 1,
            at: at(9),
            want: Wanted::Definition,
        },
        std::iter::empty(),
    );
    assert_eq!(names(&alone), ["ask 9"]);
}

/// The last question of **each** kind survives the drain: a reader asking where a name is
/// used has not taken back the definition they asked for, and the two are answered by
/// different parts of the app.
#[test]
fn a_question_about_references_does_not_cancel_one_about_a_definition() {
    let at = |line: u32| Lookup {
        file: PathBuf::from("/p/src/main.rs"),
        line,
        column: 0,
    };
    let names = |jobs: &[LspJob]| -> Vec<String> {
        jobs.iter()
            .map(|job| match job {
                LspJob::Ask { at, want, .. } => format!("{want:?} {}", at.line),
                _ => "other".to_owned(),
            })
            .collect()
    };

    let drained = worth_doing(
        LspJob::Ask {
            run: 1,
            at: at(1),
            want: Wanted::Definition,
        },
        vec![
            LspJob::Ask {
                run: 1,
                at: at(2),
                want: Wanted::References,
            },
            LspJob::Ask {
                run: 1,
                at: at(3),
                want: Wanted::References,
            },
        ]
        .into_iter(),
    );
    // The definition asked for first is still asked, and the second of the two
    // references questions is the one that stands.
    assert_eq!(names(&drained), ["Definition 1", "References 3"]);
}

/// A start over a directory nobody has agreed to is a question and not a start: a server
/// runs the project's own build scripts and macros, so the reader is asked first.
#[test]
fn a_press_over_a_directory_nobody_agreed_to_asks_before_it_starts() {
    let (mut test, states, language, _asking, asks) = mount_server!(|job: LspJob| match job {
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Ok(lsp::Handle::to_nothing()),
        }),
        _ => None,
    });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);

    press_at(&mut test, the_control());
    settle(&mut test);

    assert_eq!(language.read().state, Lsp::Off, "a server was started");
    assert!(nothing_pressed(&asks), "the worker was told to start one");

    // The question, what it means, the directory it is about, and the two answers.
    let drawn = labels(&test);
    let says = |wanted: &str| drawn.iter().any(|text| text.contains(wanted));
    assert!(says("read this directory"), "{drawn:?}");
    assert!(says("build scripts"), "{drawn:?}");
    assert!(says("/p"), "{drawn:?}");
    assert!(says("Start it") && says("Not now"), "{drawn:?}");
}

/// Agreeing starts it, once, and the project keeps the answer: it is written where the
/// name and the directory are.
#[test]
fn agreeing_starts_the_server_and_the_project_keeps_the_answer() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);
    press_at(&mut test, the_control());
    settle(&mut test);

    let agree = centre_of(&test, "Start it");
    press_at(&mut test, agree);
    until_server(&mut test, language, &Lsp::Running);

    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert!(nothing_pressed(&asks), "the one press started two servers");
    assert_eq!(language.read().state, Lsp::Running);

    // The question is gone, and the answer is in what `project.toml` is written from.
    assert!(!labels(&test).iter().any(|text| text == "Start it"));
    assert!(proj.read().trusted);
    assert!(proj.read().details().trusted);
}

/// Declining starts nothing and is remembered nowhere: the next press asks again, since
/// what was answered was the press and not the project.
#[test]
fn declining_starts_nothing_and_is_not_remembered() {
    let (mut test, states, language, _asking, asks) = mount_server!(|job: LspJob| match job {
        LspJob::Start { run, .. } => Some(LspAnswer::Started {
            run,
            server: Ok(lsp::Handle::to_nothing()),
        }),
        _ => None,
    });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);
    press_at(&mut test, the_control());
    settle(&mut test);

    let decline = centre_of(&test, "Not now");
    press_at(&mut test, decline);
    settle(&mut test);

    assert!(
        nothing_pressed(&asks),
        "a declined start reached the worker"
    );
    assert_eq!(language.read().state, Lsp::Off);
    assert!(language.read().asking.is_none(), "the question is still up");
    assert!(!proj.read().trusted, "a no was written into the project");

    press_at(&mut test, the_control());
    settle(&mut test);
    assert!(
        labels(&test).iter().any(|text| text == "Start it"),
        "the second press did not ask again"
    );
}

/// A project that agreed in some earlier run is not asked again: the answer comes back
/// with the project, and the effect that drops it on a change does not count its own
/// mount as one.
#[test]
fn a_project_that_agreed_before_the_app_opened_is_not_asked_again() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, asks) = mount_server!(
        {
            let handle = handle.clone();
            move |job: LspJob| match job {
                LspJob::Start { run, .. } => Some(LspAnswer::Started {
                    run,
                    server: Ok(handle.clone()),
                }),
                _ => None,
            }
        },
        OpenProject {
            directory: "/p".to_owned(),
            trusted: true,
            ..OpenProject::default()
        }
    );

    // Settled first, so the effect that drops an agreement on a change has been round:
    // its first run is the mount, and the mount is not a change.
    settle(&mut test);
    assert!(
        states.proj.read().trusted,
        "the launch threw away what was restored"
    );

    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);

    assert_eq!(
        next_job(&asks),
        Some(AskedOfServer::Start(PathBuf::from("/p")))
    );
    assert!(
        !labels(&test).iter().any(|text| text == "Start it"),
        "a project that had already agreed was asked again"
    );
}

/// The agreement is about a directory, so editing the directory takes it back: the server
/// stops and the next press asks about the new one.
#[test]
fn changing_the_directory_asks_about_the_new_one() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, _asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");
    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);

    let mut proj = states.proj;
    proj.write().directory = "/elsewhere".to_owned();
    settle(&mut test);

    assert!(!proj.read().trusted, "the agreement outlived its directory");
    assert_eq!(language.read().state, Lsp::Off);

    press_at(&mut test, the_control());
    settle(&mut test);
    let drawn = labels(&test);
    assert!(drawn.iter().any(|text| text == "Start it"), "{drawn:?}");
    assert!(drawn.iter().any(|text| text == "/elsewhere"), "{drawn:?}");
}

/// The Project view says whether the reader has agreed to a server reading this directory,
/// and is the way back: taking it back forgets the answer and stops the server it was
/// given for, since a reader who did not mean to let a program read their project has said
/// something about the one reading it now.
#[test]
fn the_project_view_shows_the_agreement_and_takes_it_back() {
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);

    // Nobody has agreed to anything yet, and it says so rather than saying nothing.
    assert!(labels(&test).iter().any(|text| text == "Not agreed to"));
    assert!(!labels(&test).iter().any(|text| text == "Take it back"));

    // The view's own Start asks, and agreeing is what the question is for.
    let start = centre_of(&test, "Start");
    press_at(&mut test, start);
    settle(&mut test);
    let agree = centre_of(&test, "Start it");
    press_at(&mut test, agree);
    until_server(&mut test, language, &Lsp::Running);
    assert!(proj.read().trusted);
    assert!(labels(&test).iter().any(|text| text == "Agreed to"));

    let back = centre_of(&test, "Take it back");
    press_at(&mut test, back);
    settle(&mut test);

    assert!(!proj.read().trusted, "the answer outlived being taken back");
    assert_eq!(
        language.read().state,
        Lsp::Off,
        "a server kept reading a directory the reader had just refused it"
    );
    assert!(labels(&test).iter().any(|text| text == "Not agreed to"));
}

/// A project arriving is not the reader pointing this one somewhere else: the agreement
/// travels with the project it was given to, and switching to one that agreed before does
/// not ask again -- and, worse than asking, does not write that `false` back into the file
/// it was just read from. The server still stops: it was the other project's.
#[test]
fn switching_projects_keeps_the_answer_the_new_one_brought() {
    let handle = lsp::Handle::to_nothing();
    let (mut test, states, language, _asking, _asks) = mount_server!({
        let handle = handle.clone();
        move |job: LspJob| match job {
            LspJob::Start { run, .. } => Some(LspAnswer::Started {
                run,
                server: Ok(handle.clone()),
            }),
            _ => None,
        }
    });
    with_a_directory(&mut test, &states, "/p");
    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);

    // What a switch does to these states: the id and the directory arrive together, out of
    // the other project's own file, with its own answer.
    let mut proj = states.proj;
    let mut open = proj.peek().clone();
    open.id = ProjectId::new("other-1");
    open.directory = "/elsewhere".to_owned();
    open.trusted = true;
    proj.set(open);
    settle(&mut test);

    assert!(
        proj.read().trusted,
        "the project's own agreement was taken off it on the way in"
    );
    assert_eq!(
        language.read().state,
        Lsp::Off,
        "the server outlived the project it was started for"
    );

    // And a press starts it, rather than asking about a directory this project already
    // answered for.
    press_at(&mut test, the_control());
    until_server(&mut test, language, &Lsp::Running);
    let drawn = labels(&test);
    assert!(!drawn.iter().any(|text| text == "Start it"), "{drawn:?}");
}

/// The Project view's own Start button asks the same question, which is answered in the
/// same place: the two presses are one control.
#[test]
fn the_project_views_button_asks_before_it_starts_too() {
    let (mut test, states, language, _asking, _asks) =
        mount_project!(|_: BuildJob| BuildAnswer::Read {
            manifest: None,
            debug_lines: false,
        });
    let mut proj = states.proj;
    proj.write().directory = "/p".to_owned();
    settle(&mut test);

    let start = centre_of(&test, "Start");
    press_at(&mut test, start);
    settle(&mut test);
    assert_eq!(language.read().state, Lsp::Off, "it started without asking");

    let agree = centre_of(&test, "Start it");
    press_at(&mut test, agree);
    until_server(&mut test, language, &Lsp::Running);

    assert_eq!(language.read().state, Lsp::Running);
    assert!(proj.read().trusted);
}

/// The two panes as the dock mounts them, with the landing machinery behind them: what a
/// door from outside a document reaches, and what answers it.
fn landing_panes_harness() -> impl IntoElement {
    let active = use_consume::<Active>().0;
    let marked = use_consume::<Marked>().0;
    let landing = use_consume::<Land>().0;
    let plant = use_consume::<Plant>().0;
    let driven = use_consume::<Drives>().0;
    let marks_at = use_consume::<MarksAt>().0;
    let code_rows = use_consume::<CodeRows>().0;
    let open = use_open();
    use_land(
        active, open, marked, landing, plant, driven, marks_at, code_rows,
    );
    panes_harness()
}

/// [`landing_panes_harness`] over the contexts `app()` gives it.
fn landing_panes() -> (TestingRunner, ProjectStates, LocationStates) {
    let (test, (states, location)) = TestingRunner::new(
        landing_panes_harness,
        (500., 300.).into(),
        |runner| {
            let (states, location) = location_states!(runner);
            // Whether Shift is held, which every code row reads.
            runner.provide_root_context(|| Shift(State::create(false)));
            // The two `app()` provides beside the project's, which `DocumentBody` sizes
            // its panels from.
            runner.provide_root_context(|| SplitRatio(State::create(50.0)));
            runner.provide_root_context(|| {
                Splits(State::create(ResizableContext {
                    direction: Direction::Horizontal,
                    ..Default::default()
                }))
            });
            (states, location)
        },
        1.,
    );
    (test, states, location)
}

/// **A door into another file scrolls the pane to the line it landed on, in a tab that
/// was already showing something else.** A link followed in the source, and a row in the
/// Search panel, both reach a document through the tab on screen -- in place, or into the
/// temporal one -- and a tab handed another document is not mounted again, so the pane's
/// controller and its hooks are the ones the file before it left behind.
///
/// Headless because the answer is a scroll offset a `VirtualScrollView` turns into rows,
/// asked of the pane the way the reader asks it: by which line numbers are drawn.
#[test]
fn a_door_into_another_file_shows_the_line_it_landed_on() {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-landing-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    // The file the tab is showing when the door is pressed, and the one it opens. The
    // second is long enough that the line landed on is nowhere near the top, and longer
    // than the first, so a reveal measuring against the first refuses it.
    let first = directory.join("first.rs");
    std::fs::write(&first, "fn a() {}\nfn b() {}\n").expect("writing the first file");
    let second = directory.join("second.rs");
    let text: String = (1..=200).map(|n| format!("fn line_{n}() {{}}\n")).collect();
    std::fs::write(&second, text).expect("writing the second file");
    let opened: Arc<str> = Arc::from(first.to_str().expect("a utf-8 temporary path"));
    let landed: Arc<str> = Arc::from(second.to_str().expect("a utf-8 temporary path"));
    const LINE: u32 = 150;

    let (mut test, states, location) = landing_panes();
    open_document(
        states.open,
        states.visits,
        Document::Source(opened.clone()),
        Reach::Preview,
    );
    for _ in 0..8 {
        test.sync_and_update();
    }

    // The door: the same `land` a followed name and a search hit both make, in place, so
    // the tab on screen is handed the second file rather than mounting a new one.
    land(
        states.open,
        states.visits,
        location.marked,
        location.landing,
        location.plant,
        Landing {
            tab: Document::Source(landed.clone()),
            at: Some(LinePos {
                file: landed.clone(),
                line: LINE,
            }),
            address: None,
            columns: None,
        },
        Reach::InPlace,
    );
    for _ in 0..12 {
        test.sync_and_update();
    }

    let rows = gutter_lines(&test);
    assert!(
        rows.contains(&LINE),
        "the pane does not show line {LINE}, which the door landed on: {rows:?}"
    );
    // With the margin a reveal keeps above the row it scrolls to, and no more.
    let top = *rows.first().expect("the gutter drew no line numbers");
    assert!(
        (LINE.saturating_sub(CONTEXT_ROWS as u32)..=LINE).contains(&top),
        "the pane is at line {top}, not on the line {LINE} it landed on"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// **A door moves the pane once: from where it was to the line it landed on, and not by
/// way of the top of the file.** The run a landing names is planted by `use_land`, which
/// runs off `Active` and so a pass later than the switch reaches the pane's own hook. A
/// hook that put the arriving tab at its opening row meanwhile showed the top of the file
/// for those passes, which is the flicker the reader sees; the move is held instead until
/// the landing has been spent.
///
/// Headless because the answer is the pane's position on the passes *between* the press
/// and the pane settling, which only a test that looks at every pass can see.
#[test]
fn a_door_moves_the_pane_once_and_not_by_way_of_the_top() {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-flicker-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    // Two long files, so a move to the top of either is a move and not the offset the
    // pane already had.
    let write = |path: &Path, name: &str| {
        let text: String = (1..=200)
            .map(|n| format!("fn {name}_{n}() {{}}\n"))
            .collect();
        std::fs::write(path, text).expect("writing a source file");
    };
    let first = directory.join("first.rs");
    let second = directory.join("second.rs");
    write(&first, "one");
    write(&second, "two");
    let opened: Arc<str> = Arc::from(first.to_str().expect("a utf-8 temporary path"));
    let landed: Arc<str> = Arc::from(second.to_str().expect("a utf-8 temporary path"));

    let (mut test, states, location) = landing_panes();
    open_document(
        states.open,
        states.visits,
        Document::Source(opened.clone()),
        Reach::Preview,
    );
    for _ in 0..8 {
        test.sync_and_update();
    }

    // The same `land` a followed name and a search hit both make, in place, so the tab on
    // screen is handed the document rather than mounting a pane for it.
    let door = |test: &mut TestingRunner, file: &Arc<str>, line: u32| {
        land(
            states.open,
            states.visits,
            location.marked,
            location.landing,
            location.plant,
            Landing {
                tab: Document::Source(file.clone()),
                at: Some(LinePos {
                    file: file.clone(),
                    line,
                }),
                address: None,
                columns: None,
            },
            Reach::InPlace,
        );
        // The top line of the gutter after each pass, with the runs of equal ones
        // collapsed: where the pane was, where it went, and anywhere it went on the way.
        let mut seen: Vec<u32> = Vec::new();
        for _ in 0..14 {
            test.sync_and_update();
            let top = *gutter_lines(test)
                .first()
                .expect("the gutter drew no numbers");
            if seen.last() != Some(&top) {
                seen.push(top);
            }
        }
        seen
    };

    // Each door, and the margin a reveal keeps above the row it scrolls to. Down the
    // file the pane is already showing: away from the top first, and then between two
    // lines of it, which is where a visit to the top on the way shows plainest.
    let at = |line: u32| line - CONTEXT_ROWS as u32;
    assert_eq!(
        door(&mut test, &opened, 100),
        vec![1, at(100)],
        "the pane did not go straight from the top of the file to line 100"
    );
    assert_eq!(
        door(&mut test, &opened, 150),
        vec![at(100), at(150)],
        "the pane went by way of another line on its way to line 150"
    );
    // And into another file, whose top the pane must not be shown at either.
    assert_eq!(
        door(&mut test, &landed, 40),
        vec![at(150), at(40)],
        "the pane went by way of another line on its way into the second file"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// **A door lands as the pane draws the document it opened**, and not two passes later.
/// `use_land` turns a landing into the source pane's run off `Active`, which is a memo,
/// so the pane that waited for it drew the arriving file at the offset the outgoing
/// place had left -- a line of the new file nobody asked for, held there long enough to
/// read, and then a jump. The pane takes the landing itself instead, on the first run
/// after the switch.
///
/// The two doors follow one another without settling in between, which is what leaves
/// `use_land` an arrival behind: the landing the second leaves must survive the first
/// being answered, or the door plants nothing and the pane opens at the top instead.
///
/// Headless because what is under test is *when*: the pass the landed row first shows,
/// which only a test that looks at every pass can say.
#[test]
fn a_door_lands_as_the_pane_draws_the_document_it_opened() {
    let directory = std::env::temp_dir().join(format!(
        "assembly-viewer-promptly-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let write = |path: &Path, name: &str| {
        let text: String = (1..=200)
            .map(|n| format!("fn {name}_{n}() {{}}\n"))
            .collect();
        std::fs::write(path, text).expect("writing a source file");
    };
    let first = directory.join("first.rs");
    let second = directory.join("second.rs");
    write(&first, "one");
    write(&second, "two");
    let opened: Arc<str> = Arc::from(first.to_str().expect("a utf-8 temporary path"));
    let landed: Arc<str> = Arc::from(second.to_str().expect("a utf-8 temporary path"));

    let (mut test, states, location) = landing_panes();
    open_document(
        states.open,
        states.visits,
        Document::Source(opened.clone()),
        Reach::Preview,
    );
    for _ in 0..8 {
        test.sync_and_update();
    }

    // A door, and the pass on which the row it landed on first shows.
    let door = |test: &mut TestingRunner, file: &Arc<str>, line: u32| -> Option<usize> {
        land(
            states.open,
            states.visits,
            location.marked,
            location.landing,
            location.plant,
            Landing {
                tab: Document::Source(file.clone()),
                at: Some(LinePos {
                    file: file.clone(),
                    line,
                }),
                address: None,
                columns: None,
            },
            Reach::InPlace,
        );
        let wanted = line - CONTEXT_ROWS as u32;
        (0..16).find(|_| {
            test.sync_and_update();
            gutter_lines(test).first() == Some(&wanted)
        })
    };

    // Away from the top first, and then into the other file: the second door is the one
    // under test, and it follows the first without settling, which is also what leaves
    // `use_land` an arrival behind.
    assert!(
        door(&mut test, &opened, 100).is_some(),
        "line 100 never showed"
    );
    let passes = door(&mut test, &landed, 150).expect("line 150 never showed");
    // The render that draws the document, the effect that scrolls it, and the layout that
    // shows the scroll: three passes, and nothing waiting on the landing being spent.
    assert!(
        passes <= 3,
        "the pane took {passes} passes to show the line it landed on, \
         which is long enough to draw the file where it was before"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

// ---------------------------------------------------------------------------------------
// The file finder.

/// The finder over a walk the test hands in, so that one can be held still: a real walk
/// answers faster than the runner settles, and the list that is kept is a race by
/// construction.
#[derive(Clone)]
struct Walking(Arc<dyn Fn(&Path, &mut dyn FnMut(WalkEvent) -> ControlFlow<()>) + Send + Sync>);

fn finder_harness() -> impl IntoElement {
    let finder = use_consume::<Finding>().0;
    let work = use_consume::<Walking>().0;
    use_finder_with(finder, move |root, emit| work(root, emit));

    rect().expanded().child(FinderOverlay)
}

/// The overlay over `work`, with the project's directory set to a real one of this test's
/// own, and the states the root's key handler writes.
fn finder_over(
    line: u32,
    work: impl Fn(&Path, &mut dyn FnMut(WalkEvent) -> ControlFlow<()>) + Send + Sync + 'static,
) -> (
    TestingRunner,
    ProjectStates,
    State<Finder>,
    ModifierKeys,
    PathBuf,
    State<DockArea>,
) {
    let directory = run_directory(line).join("found");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("creating the test directory");
    let work = Arc::new(work);
    let (mut test, states) = TestingRunner::new(
        finder_harness,
        (800., 600.).into(),
        move |runner: &mut _| {
            runner.provide_root_context({
                let work = work.clone();
                move || Walking(work.clone())
            });
            let finder = runner
                .provide_root_context(|| Finding(State::create(Finder::default())))
                .0;
            // The root's one key handler tracks the modifiers beside answering the
            // chord, so the chord is pressed through the real thing.
            let held = runner.provide_root_context(|| {
                Modifiers5(
                    State::create(false),
                    State::create(false),
                    State::create(false),
                    State::create(false),
                    State::create(false),
                )
            });
            let states = project_states!(runner);
            // The same context again, so the chord is pressed with the handle the
            // sidebar reads: the root answers Ctrl+P and Ctrl+Shift+F in the one
            // handler, and the second of them reaches for the dock.
            let dock = runner
                .provide_root_context(|| {
                    SidebarDock(State::create(DockArea::column(vec![vec![Panel::Search]])))
                })
                .0;
            (states, finder, held, dock)
        },
        1.,
    );
    let (states, finder, held, dock) = states;
    let keys = ModifierKeys::new(held.0, held.1, held.2, held.3, held.4);
    let mut proj = states.proj;
    proj.write().directory = directory.to_string_lossy().into_owned();
    settle(&mut test);
    (test, states, finder, keys, directory, dock)
}

/// A file as the walk reports one, under `root`.
fn walked_file(root: &Path, relative: &str) -> WalkEvent {
    WalkEvent::File(
        crate::walk::found_under(root, &root.join(relative)).expect("a path with a name"),
    )
}

/// What each row of the list says: a paragraph's spans joined, so a name cut into marked
/// and unmarked runs reads as the one string a reader sees.
///
/// The box is a paragraph too and comes first, being above the list; it is dropped here so
/// that a row's index is a row's index.
fn finder_rows(test: &TestingRunner) -> Vec<String> {
    use freya::elements::paragraph::ParagraphElement;
    use std::any::Any;

    let mut drawn: Vec<String> = test.find_many(|node, _element| {
        (node.element().as_ref() as &dyn Any)
            .downcast_ref::<ParagraphElement>()
            .map(|paragraph| {
                paragraph
                    .spans
                    .iter()
                    .map(|span| span.text.to_string())
                    .collect::<String>()
            })
    });
    if !drawn.is_empty() {
        drawn.remove(0);
    }
    drawn
}

/// Ctrl+P through the root's one key handler, which is where it is answered: not through
/// the focused node, since the chord works from wherever the keyboard is.
fn press_finder_chord(
    states: &ProjectStates,
    finder: State<Finder>,
    keys: ModifierKeys,
    dock: State<DockArea>,
) {
    root_key_down(
        keys,
        states.searched,
        finder,
        states.proj,
        dock,
        &Key::Character("p".into()),
        Modifiers::CONTROL,
    );
}

/// The overlay is drawn as nothing at all until the chord, and the walk's files are what
/// the box then picks out. Fails on a finder that draws its list before it is opened.
#[test]
fn the_chord_opens_the_finder_over_the_project_files() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "src/ui/files_view.rs"));
            let _ = emit(walked_file(root, "notes/Goals.md"));
            let _ = emit(WalkEvent::Finished);
        });

    assert!(
        finder_rows(&test).is_empty(),
        "the finder is drawn before it is opened"
    );

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("s");
    settle(&mut test);

    let rows = finder_rows(&test);
    assert!(
        rows.iter().any(|row| row.starts_with("files_view.rs")),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|row| row.starts_with("Goals.md")),
        "{rows:?}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// With nothing typed the list is the source files opened most recently, newest first --
/// not everything the walk found. Fails on a finder that lists the walk when the box is
/// empty.
#[test]
fn an_empty_box_lists_the_files_opened_most_recently() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "walked.rs"));
            let _ = emit(WalkEvent::Finished);
        });

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    assert!(
        labels(&test)
            .iter()
            .any(|label| label == "No files opened yet. Type to find one."),
        "a box with nothing typed and nowhere visited lists nothing"
    );

    // Two files visited, the second last.
    for name in ["first.rs", "second.rs"] {
        let path = directory.join(name);
        open_document(
            states.open,
            states.visits,
            Document::Source(Arc::from(&*path.to_string_lossy())),
            Reach::Preview,
        );
    }
    press_finder_chord(&states, finder, keys, dock);
    settle(&mut test);

    let rows = finder_rows(&test);
    assert_eq!(
        rows,
        ["second.rs", "first.rs"],
        "newest first, and only these"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A row is the file's name and then the directories above it, which is not the order the
/// path is written in: a column of names all starting `src/ui/` says nothing.
#[test]
fn a_row_is_the_name_and_then_the_directories_above_it() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "src/ui/files_view.rs"));
            let _ = emit(walked_file(root, "top.rs"));
            let _ = emit(WalkEvent::Finished);
        });

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("s");
    settle(&mut test);

    let rows = finder_rows(&test);
    assert!(
        rows.iter().any(|row| row == "files_view.rs  src/ui"),
        "{rows:?}"
    );
    assert!(rows.iter().any(|row| row == "top.rs"), "{rows:?}");
    let _ = std::fs::remove_dir_all(&directory);
}

/// Typing narrows the list to the paths the characters appear in, in order, and the best
/// match is first. Fails on a finder that filters by anything but the fuzzy match.
#[test]
fn typing_narrows_the_list_to_the_characters_in_order() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "src/ui/files_view.rs"));
            let _ = emit(walked_file(root, "src/ui/source_view.rs"));
            let _ = emit(walked_file(root, "notes/Goals.md"));
            let _ = emit(WalkEvent::Finished);
        });

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("srcuivw");
    settle(&mut test);

    let rows = finder_rows(&test);
    assert!(
        rows.iter().all(|row| !row.starts_with("Goals.md")),
        "the query let a path through that does not hold its characters: {rows:?}"
    );
    assert!(
        rows.first()
            .is_some_and(|row| row.starts_with("files_view.rs")),
        "{rows:?}"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Enter opens the row the keyboard is on, as pressing a Files item does: the temporal
/// tab, or a new one with Ctrl held. And the finder closes behind it.
#[test]
fn enter_opens_the_selected_file_and_ctrl_enter_opens_it_in_a_new_tab() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "first.rs"));
            let _ = emit(walked_file(root, "second.rs"));
            let _ = emit(WalkEvent::Finished);
        });
    // The files have to be there: a file the source pane would refuse opens nothing.
    for name in ["first.rs", "second.rs"] {
        std::fs::write(directory.join(name), "fn one() {}\n").expect("writing the file");
    }
    let opened = |name: &str| Document::Source(Arc::from(&*directory.join(name).to_string_lossy()));

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    // Typed, so the list is the walk's and not the places visited. Which of the two rows
    // is first is the matcher's business and pinned in its own tests; this reads it.
    test.write_text("rs");
    settle(&mut test);
    let rows = finder_rows(&test);
    assert_eq!(rows.len(), 2, "{rows:?}");

    key_with(&mut test, Key::Named(NamedKey::Enter), Modifiers::empty());
    settle(&mut test);

    let first = tab_showing(&states, &opened(&rows[0])).expect("the first row's file opened");
    assert!(!finder.peek().open, "the finder closes behind the file");
    assert_eq!(
        states.open.docs.peek().temporal(),
        Some(first),
        "a row opens in the temporal tab, as a Files row does"
    );

    // The row under it, in a tab of its own.
    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("rs");
    settle(&mut test);
    key_with(
        &mut test,
        Key::Named(NamedKey::ArrowDown),
        Modifiers::empty(),
    );
    settle(&mut test);
    key_with(&mut test, Key::Named(NamedKey::Enter), Modifiers::CONTROL);
    settle(&mut test);

    let second = tab_showing(&states, &opened(&rows[1])).expect("the second row's file opened");
    assert_ne!(
        states.open.docs.peek().temporal(),
        Some(second),
        "Ctrl+Enter opens a tab that stays"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Escape closes it, and keeps nothing of what was typed.
#[test]
fn escape_closes_the_finder_and_keeps_nothing_typed() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "first.rs"));
            let _ = emit(WalkEvent::Finished);
        });

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("first");
    settle(&mut test);
    assert_eq!(finder.peek().typed, "first");

    key_with(&mut test, Key::Named(NamedKey::Escape), Modifiers::empty());
    settle(&mut test);
    assert!(!finder.peek().open);
    assert!(finder_rows(&test).is_empty(), "the overlay is gone");

    press_finder_chord(&states, finder, keys, dock);
    settle(&mut test);
    assert_eq!(finder.peek().typed, "", "the box is empty each time");
    let _ = std::fs::remove_dir_all(&directory);
}

/// The list is kept between opens: the second open draws it before its own walk has said
/// anything. Fails on a finder that empties the list when it opens.
#[test]
fn the_second_open_shows_the_files_the_first_walk_found() {
    let held = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gate = held.clone();
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            // The second walk says nothing at all, so anything drawn after it is the list the
            // first one left.
            if gate.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let _ = emit(walked_file(root, "kept.rs"));
            let _ = emit(WalkEvent::Finished);
        });

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("kept");
    settle(&mut test);
    assert!(finder_rows(&test).iter().any(|row| row == "kept.rs"));

    key_with(&mut test, Key::Named(NamedKey::Escape), Modifiers::empty());
    settle(&mut test);
    held.store(true, std::sync::atomic::Ordering::SeqCst);

    press_finder_chord(&states, finder, keys, dock);
    settle(&mut test);
    test.write_text("kept");
    settle(&mut test);
    assert!(
        finder_rows(&test).iter().any(|row| row == "kept.rs"),
        "the second open walked again instead of showing what it had"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The chord is not typed into a filter box: the `Input`'s pre-key hook declines it, which
/// is also what keeps it reaching the root -- the hook's other arms call `prevent_default`,
/// and that cancels the global key event beside them.
#[test]
fn the_finder_chord_is_declined_by_a_filter_box() {
    let symbols = fixture_symbols();
    let (mut test, states) =
        TestingRunner::new(symbols_harness, (300., 300.).into(), symbol_states!(), 1.);
    let mut objects = states.objects;
    objects.set(vec![symbols[0].object.clone()]);
    settle(&mut test);

    let row = centre_of(&test, "sum_to");
    press_at(&mut test, row);
    settle(&mut test);
    key_with(&mut test, Key::Character("f".into()), Modifiers::CONTROL);
    test.write_text("sum");
    key_with(&mut test, Key::Character("p".into()), Modifiers::CONTROL);
    settle(&mut test);

    // The pattern is what was typed: a `p` in it would have filtered the row away.
    assert!(labels(&test).iter().any(|label| label == "sum_to"));
}

/// The chord is not typed into the scratchpad's editor either. The editor inserts any
/// character it has no chord of its own for, Ctrl held or not, so without the decline
/// Ctrl+P puts a `p` in the source and never opens the finder.
#[test]
fn the_finder_chord_is_declined_by_the_scratchpad_editor() {
    let (mut test, _states, pad, text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::List => PadAnswer::Listed(Vec::new()),
            PadJob::New => unreachable!("this test has one pad"),
            PadJob::Delete(_) => unreachable!("this test deletes nothing"),
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved {
                pad: scratchpad.id().clone(),
                failure: scratchpad.manifest().err(),
            },
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().state().opened);
    let before = shown_rope(text, pad);

    let editor = centre_of(&test, "fn");
    press_at(&mut test, editor);
    settle(&mut test);
    key_with(&mut test, Key::Character("p".into()), Modifiers::CONTROL);
    settle(&mut test);

    assert_eq!(
        shown_rope(text, pad),
        before,
        "the chord was typed into the source"
    );
}

/// The box is the panel's width, less the air around it, and inside the panel. Centring
/// a flex child in a column rect laid it out from the panel's middle at nearly the
/// panel's width, so it ran off the right-hand edge of the window.
#[test]
fn the_box_fills_the_panel_with_air_around_it() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "kept.rs"));
            let _ = emit(WalkEvent::Finished);
        });
    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);

    let drawn = label_area(&test, "Find a file").expect("the box is drawn");
    // The panel is centred across the window the runner was given.
    let left = (800.0 - FINDER_WIDTH) / 2.0;
    assert!(
        drawn.min_x() >= left + FINDER_PAD && drawn.max_x() <= left + FINDER_WIDTH - FINDER_PAD,
        "the box is not inside the panel with air around it: {drawn:?}"
    );
    assert!(
        drawn.width() >= FINDER_WIDTH - 4.0 * FINDER_PAD,
        "the box is {} of the panel's {FINDER_WIDTH}",
        drawn.width()
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A press outside the panel closes it. The rect that takes that press covers the window
/// and draws nothing at all -- the app behind the finder is not dimmed -- so this is also
/// what says a rect with no background is still there to be pressed.
#[test]
fn a_press_outside_the_panel_closes_the_finder() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "kept.rs"));
            let _ = emit(WalkEvent::Finished);
        });
    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    assert!(finder.peek().open);

    // Near the bottom of the window, well under the panel.
    press_at(&mut test, (400.0, 560.0));
    settle(&mut test);
    assert!(!finder.peek().open, "the press outside did not close it");

    // And a press inside it does not: the panel stops it reaching the rect behind.
    press_finder_chord(&states, finder, keys, dock);
    settle(&mut test);
    let drawn = label_area(&test, "Find a file").expect("the box is drawn");
    press_at(
        &mut test,
        (drawn.min_x() as f64 + 4.0, drawn.min_y() as f64 + 4.0),
    );
    settle(&mut test);
    assert!(finder.peek().open, "a press in the box closed the finder");
    let _ = std::fs::remove_dir_all(&directory);
}

/// A source reached through a binary's debug info -- the standard library's own, a
/// dependency's out of the registry -- is not one of the project's files, so the empty
/// box does not list it however recently it was read.
#[test]
fn a_file_outside_the_project_is_not_listed() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "own.rs"));
            let _ = emit(WalkEvent::Finished);
        });

    for path in [
        directory.join("own.rs"),
        PathBuf::from("/rustc/0000/library/std/src/io/mod.rs"),
    ] {
        open_document(
            states.open,
            states.visits,
            Document::Source(Arc::from(&*path.to_string_lossy())),
            Reach::Preview,
        );
    }
    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);

    assert_eq!(
        finder_rows(&test),
        ["own.rs"],
        "a file outside the project's directory was listed"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Pressing a row opens its file, as Enter on it does.
#[test]
fn pressing_a_row_opens_its_file() {
    let (mut test, states, finder, keys, directory, dock) =
        finder_over(line!(), move |root, emit| {
            let _ = emit(walked_file(root, "kept.rs"));
            let _ = emit(WalkEvent::Finished);
        });
    std::fs::write(directory.join("kept.rs"), "fn one() {}\n").expect("writing the file");

    press_finder_chord(&states, finder, keys, dock);
    pump(&mut test, || !finder.peek().walking);
    test.write_text("kept");
    settle(&mut test);

    // The row's second span: the first is the marked run, which the box's own text also
    // reads, and the box is drawn above the list.
    let row = centre_of(&test, ".rs");
    press_at(&mut test, row);
    settle(&mut test);

    let file = Document::Source(Arc::from(&*directory.join("kept.rs").to_string_lossy()));
    let opened = tab_showing(&states, &file).expect("the pressed file opened");
    assert!(!finder.peek().open, "the finder closes behind the file");
    assert_eq!(
        states.open.docs.peek().temporal(),
        Some(opened),
        "a row opens in the temporal tab, as a Files row does"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

//! The tests that run the UI rather than the logic under it, and the
//! palette's, which have nowhere else to live.
//!
//! Everything decided by cases lives in a framework-free module with its own tests
//! ([`crate::rows`] here), and this is deliberately not a second home for that. What is
//! here is what those modules cannot hold. The runner tests exist for the one class of
//! bug they are blind to by construction: a `State` borrow that is legal to the compiler
//! and panics at the moment a gesture ends. `mark_release` shipped holding a `peek` guard
//! across its own write, so *every* mouse-up on a run brought the window down, and no
//! amount of testing `RowSelection` would have said a word about it. A press, a sweep and
//! a release through freya's own headless runner is the smallest thing that would have.
//!
//! The palette's tests are here because a `Color` is a freya type and the palette cannot
//! move out of `ui.rs`. They assert the properties a second set of values can silently
//! break -- a foreground that has gone invisible against its own surface, a translucent
//! wash that says nothing over a dark ground, a capture colour that sends
//! `resolve_capture_color` walking up the dotted name -- rather than the values
//! themselves, which are a design and not an assertion.
use super::*;
// Named again, because `use super::*` offers two `use_theme`s -- ours, re-exported out of
// `settings_view`, and freya's own out of the prelude -- and two globs offering one name is
// an ambiguity at the call site rather than a shadowing. An explicit import wins over a
// glob, so this is what the name means here: ours. It is spelled out in this file and not
// in `ui.rs` because this file is the only one that calls it from outside the module that
// defines it; a re-export up there would be unused by the build that is not running tests.
use super::settings_view::use_theme;
use freya_testing::TestingRunner;

/// Three rows wired exactly the way the two panes are, and no more of them than that:
/// the press that starts a run, the `pointer_over` that sweeps it, and the release
/// watched globally at the root, because the button very often comes up somewhere the
/// run does not reach.
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

/// The five states [`scrolling_harness`] is wired to, as context types of their own
/// so that three `State<usize>`s cannot be confused for one another.
#[derive(Clone, Copy)]
struct KeptTab(State<String>);
#[derive(Clone, Copy)]
struct KeptAt(State<Positions<String>>);
/// The tabs that are open, which is what a position is only kept for. A plain list
/// here where the app asks `Docs`: what the hook wants is an answer that is true
/// *now*, and both of these are.
#[derive(Clone, Copy)]
struct KeptOpen(State<Vec<String>>);
#[derive(Clone, Copy)]
struct KeptLength(State<usize>);
/// The last row the pointer was over, which is how the test asks where the view
/// actually is rather than believing what the map says about it.
#[derive(Clone, Copy)]
struct KeptTop(State<usize>);

/// A scroll view wired the way both **code** panes are: one `ScrollController` reused
/// across every tab the pane shows, `use_kept_position` between them, and
/// [`code_row_height`] on both halves of the view -- which is what those panes are,
/// and the only kind of list that keeps a position at all.
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

/// A sidebar list's shape: the same view over [`list_row_height`], and no kept
/// position, because the Objects and Symbols lists have none. It exists so that the
/// agreement between an `item_size` and its rows is asserted for *both* heights rather
/// than for one and assumed for the other.
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

/// Switching tab puts the pane back where that tab was left, and a tab seen for the
/// first time opens at the top rather than at the last one's offset.
///
/// Headless because none of it is visible to any other kind of test: the position is
/// read out of a `ScrollController` inside an effect that a scroll wakes, and what it
/// is asserted against is which row a real `VirtualScrollView` put under the pointer.
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
        // Settled first: an effect is a spawned task, so the scroll it asks for lands
        // a poll after the state change that asked for it, and a view that moves under
        // a pointer already sitting still sends no `pointerover` to say so.
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
    // The scroll was written down as it happened, which is what makes the position
    // survive the pane being left in any way at all -- including the window closing.
    assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

    // A tab this pane has never shown starts at the top, and pointedly not at the
    // offset the tab before it was at: that is the bug this hook exists for.
    tab.set("b".to_owned());
    test.sync_and_update();
    assert_eq!(top_row(&mut test), 0);
    // And the tab left behind is remembered, not overwritten by where the new one is.
    assert_eq!(at.peek().at(&"a".to_owned()), Some(left_at));

    tab.set("a".to_owned());
    test.sync_and_update();
    assert_eq!(top_row(&mut test), left_at);

    // And closing the tab on screen does not put it back. `close_tab` forgets the
    // position and then moves to a neighbour, so the run that follows is holding a
    // tab that is gone -- which is a `Selection` holding a whole `Object` in the app.
    let (mut open, mut at) = (open, at);
    open.write().retain(|tab| tab != "a");
    at.write().forget(&"a".to_owned());
    tab.set("b".to_owned());
    for _ in 0..4 {
        test.sync_and_update();
    }
    assert_eq!(at.peek().at(&"a".to_owned()), None);
}

// --- the dock's document panel -----------------------------------------
//
// Plain data, so no runner: a `DockArea` is a tree and three rules over it.

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

/// The whole reason the panel is designated. freya's own sweep retains every
/// non-empty child with no exemption, so the panel documents live in would fold away
/// the moment the reader closed the last one -- and the content area would come back
/// as whatever was left beside it.
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

/// Nothing on screen: what a project switch does is to the states, and the states are
/// what this asserts. A runner all the same, because a `State` needs a runtime and
/// because the bug being looked for is a borrow held across a write, which is a
/// runtime panic and not a compile error.
fn project_harness() -> impl IntoElement {
    rect().expanded()
}

/// The eight contexts `app()` provides, in one `ProjectStates`, so a test can drive a
/// switch exactly as the recent list's press does.
///
/// A macro and not a function: the runner's type is `freya_core::integration::Runner`,
/// which freya's prelude does not re-export, so naming it here would mean naming a
/// crate the app does not depend on.
macro_rules! project_states {
    () => {
        |runner: &mut _| project_states!(runner)
    };
    ($runner:expr) => {{
        // The two states that are what is open, and the derivation over them, in the
        // same order and by the same rule `app()` uses -- so a test drives exactly
        // what the app does. `Active` is provided but not returned: it is not one of
        // the project's states, it is a reading of two of them.
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
            src_at: $runner
                .provide_root_context(|| SrcAt(State::create(Positions::default())))
                .0,
            history: $runner
                .provide_root_context(|| Hist(State::create(History::default())))
                .0,
        }
    }};
}

/// Leaving a project leaves nothing of it behind: no object, no tab of either kind,
/// no viewing position, no history entry and nothing active.
///
/// Headless for the reason the swept run below is. `clear_project` goes through
/// `close_binary` and `close_tab`, and each of those reads a state and then writes
/// it -- which is legal to the compiler and panics at the moment it runs if the read
/// is still borrowed. Asserting the emptiness is half of it; the other half is that
/// the whole walk happens at all.
///
/// The source-driven tab is the case a binary close deliberately leaves standing, so
/// it is the one only this walk reaches.
#[test]
fn leaving_a_project_leaves_nothing_of_it_behind() {
    let symbols = fixture_symbols();
    let (first, second) = (symbols[0].clone(), symbols[1].clone());
    let object = first.object.clone();
    let source = Document::Source(Arc::from("/src/main.rs"));

    let (mut test, states) =
        TestingRunner::new(project_harness, (200., 200.).into(), project_states!(), 1.);
    test.sync_and_update();

    // The app as a session leaves it: a binary open, two of its functions in the
    // strip with a row remembered for one of them, a source file open beside them and
    // somewhere to go back to.
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
    // Three visits, the source file included: the history records documents, which is
    // what lets its panel list a file at all.
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
    // Not tidiness: a `Document::Assembly` key holds the `Arc<Object>` it points
    // into, so a position left here would hold the whole binary of the project just
    // left.
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

/// Nothing but the overflow control, so the press has one thing to land on.
fn menu_harness() -> impl IntoElement {
    rect()
        .expanded()
        .horizontal()
        .content(Content::Flex)
        .child(rect().width(Size::flex(1.0)).height(Size::px(25.0)))
        .child(DocumentMenuButton)
}

/// And it stays hanging from that edge when the list grows underneath it.
///
/// `MenuContainer` measures itself once and keeps the offset it worked out then
/// (`menu.rs:236`), so a menu that widens after that is placed for the width it used to
/// be and hangs off the side of the window -- by 315px here. The menu is keyed by its
/// row count so a change remounts it, which is what makes it measure the size it is.
///
/// Not a contrived case: the tab list fills in from a worker, so a menu opened while a
/// binary is still being read is open while rows arrive.
#[test]
fn a_menu_open_while_the_list_grows_stays_on_the_edge() {
    let (mut test, states) =
        TestingRunner::new(menu_harness, (600., 300.).into(), project_states!(), 1.);
    test.sync_and_update();

    // The app's panel always holds the three views, so the button is there before any
    // document is -- which is exactly how the menu comes to be open while rows arrive.
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

/// The menu hangs from the button's right-hand edge rather than off the side of the
/// window.
///
/// It is positioned *vertically only*, and this is what says why. `MenuContainer`
/// corrects its own overflow -- and latches the first position it is measured at
/// (`menu.rs:236`, `measured` is written once) -- so a `right(0.)` of ours lands it on
/// the button's edge and freya's correction then shifts it a whole menu-width further
/// left, which is the misalignment this asserts against. Dropping our half and letting
/// freya pull it back into the window is what puts the two edges together.
///
/// The harness puts the button hard against the right edge, which is where the tab bar
/// really puts it and the only place the correction fires at all.
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

/// The overflow menu opens on a press and closes on the next one.
///
/// The assertion is that the element tree *grew*, rather than that some particular row
/// exists: what is being checked is that the control does anything at all, and a menu
/// that shut in the frame it opened would look exactly like a button that does not
/// work.
///
/// That it *cannot* shut in the frame it opened is the reason there is no guard against
/// `Menu`'s close-on-any-global-press here, where `ContextMenu` has one. The listeners
/// for a global event are collected when it is measured, before a single handler runs,
/// and this opens on `on_press` -- derived from the very `MouseUp` that emits the global
/// press -- so the menu is not in the tree to be asked. A menu opened from a `*_down`
/// handler is the case that needs the swallow, which is what `ContextMenu`'s is for:
/// a right-click menu opens on `on_secondary_down`, and the `MouseUp` ending that same
/// gesture *is* measured against a tree that holds it.
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

/// The invariant the whole two-state arrangement rests on: a document has a tab in the
/// panel exactly while it has an entry in the table.
///
/// It is what makes "the panel's `tabs` vec is the list of open documents" true
/// without a second list, and `use_kept_position` leans on it directly -- it asks
/// `Docs` whether a tab is still open in order to decide whether to write its row
/// down, and would resurrect a just-closed tab's position if the two could drift.
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
            document,
        );
    }
    test.sync_and_update();
    assert!(agree(&states).is_empty());
}

/// Closing a tab lands on the one to its right, and freya would land on the leftmost.
///
/// `DockNode::remove_tab_except` sets a panel's active tab to `tabs.first()` when it
/// removes the active one, so the removal is done by hand and the landing chosen with
/// [`tabs::landing`]. This is the assertion that says the app's rule won.
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
        &documents[0],
    );
    test.sync_and_update();
    assert!(states.open.active().is_none());
}

/// The history records where the reader *went* and not what is on screen.
///
/// The rule Step 1e settled, and the reason `activate` is told why it is being called:
/// opening a document is a visit, switching to a tab that is already open is not, and
/// the neighbour a close lands on is not either. An effect observing the active
/// document could not tell any of these apart, which is why the recording is no longer
/// one.
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

/// Closing a binary takes its own tabs and leaves a source-driven one standing.
///
/// The rule the one strip inherited from the two: a file tab outlives the binary that
/// led the reader to it, because the text stands on its own and nothing records which
/// object opened it. Worth a runner rather than a `Tabs` test, because what has to
/// hold is that `close_binary` lands the *active* document somewhere sensible when the
/// tab it was on goes and a tab of the other kind is what is left.
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

/// The channel a load test feeds by hand, and the paths the harness registers as
/// being read. Standing in for `open_binaries`' worker thread: what has to be
/// asserted is what the app does with an answer that arrives after the reader has
/// moved on, which against a real worker is a race and against a channel is a fact.
/// The receiver is *taken* rather than cloned, because a clone left in the context
/// map would keep the channel open for ever and the test could never see the one
/// thing that stops a worker: `take_load` returning and dropping the last receiver.
#[derive(Clone)]
struct Feed(
    Arc<Mutex<Option<async_channel::Receiver<Progress>>>>,
    Arc<Vec<PathBuf>>,
);

/// The real `take_load` over the real Objects tree, with the worker replaced by
/// [`Feed`]. The tree is mounted rather than left out so that every one of these
/// tests also builds the rows for a file that is being read -- including the row with
/// no group behind it, which is the one shape no other test reaches.
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

/// `n` objects that all came out of one path, which is what an archive's members look
/// like to everything above the analysis crate. Parsed `n` times rather than cloned,
/// so they are `n` distinct `Arc`s exactly as real members are.
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

/// How the Objects tree describes what is on screen, which is the one thing these
/// tests are really about: a file that is being read has a row before it has an
/// object, and stops saying so when the last of them has landed.
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

/// The sub-step in one test: the objects of one file reach the sidebar one at a time,
/// and the row for that file is there before the first of them is.
#[test]
fn objects_reach_the_sidebar_as_they_are_parsed() {
    let (path, objects) = fixture_objects(3);
    let (mut test, states, sender) = mount_load(&path);
    test.sync_and_update();

    // Before a single byte has been parsed. This is the state `Goals.md` asks for an
    // indicator for and which nothing could be in while the parse handed back one
    // `Vec` at the end.
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
        // The save side: the path joins the binaries with its first object, so a
        // session written half way through a parse names the file rather than a
        // truncated version of it. There is nothing else in `binaries` to truncate --
        // it is a list of paths.
        assert_eq!(project::binaries(&states.objects.peek()), [path.clone()]);
    }

    sender
        .send_blocking(Progress::Finished(path.clone()))
        .expect("the app is still listening");
    pump(&mut test, || !states.loading.peek().is_loading(&path));

    // Done, so the ordinary rules take over again: three objects out of one file is
    // an archive-shaped row, and nothing says it is still being read.
    assert_eq!(reading(&states), [("line_fixture.o".to_owned(), 3, false)]);
}

/// Closing a file half way through reading it takes the objects that have already
/// arrived *and* the ones that have not.
///
/// The second half is what needs a test: the worker is already parsing when the row
/// is closed, so the answers exist whatever the app does, and without the check they
/// would put the file back one member at a time.
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

    // And the worker is told, by the only thing that can tell it: the receiver is
    // gone, so its next send fails and the walk stops rather than parsing the rest of
    // a file nobody is waiting for.
    assert!(sender.send_blocking(Progress::Finished(path)).is_err());
}

/// Leaving a project while one of its files is being read. The load is cancelled by
/// `clear_project` itself and not through `close_binary`, because a file that has
/// produced nothing yet is not in the objects list for the per-path walk to reach.
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

/// Reading a file that is still being read, which is the whole point of the sub-step:
/// an object that has arrived is an ordinary row, selecting it opens an ordinary tab,
/// and the members still landing behind it change none of that.
#[test]
fn a_file_still_being_read_can_be_explored() {
    let (path, objects) = fixture_objects(3);
    let (mut test, states, sender) = mount_load(&path);
    test.sync_and_update();

    sender
        .send_blocking(Progress::Parsed(objects[0].clone()))
        .expect("the app is still listening");
    pump(&mut test, || states.objects.peek().len() == 1);

    // Through `activate`, which is the only way anything opens a tab -- a partially
    // read file is not a special case for it.
    let opened = Document::Assembly(Selection::Object(objects[0].clone()));
    activate(
        states.open,
        states.history,
        Some(opened.clone()),
        Visit::Went,
    );
    test.sync_and_update();

    for object in &objects[1..] {
        sender
            .send_blocking(Progress::Parsed(object.clone()))
            .expect("the app is still listening");
    }
    pump(&mut test, || states.objects.peek().len() == 3);
    sender
        .send_blocking(Progress::Finished(path.clone()))
        .expect("the app is still listening");
    pump(&mut test, || !states.loading.peek().is_loading(&path));

    assert!(
        states.open.active() == Some(opened),
        "the active document moved while the rest of the file was arriving"
    );
    assert_eq!(states.open.documents().len(), 1);
    assert_eq!(states.objects.peek().len(), 3);
}

/// What the two text boxes mean, which is the one place the project view's `String`s
/// and `project.toml`'s absent keys meet. An empty box is not a project named the
/// empty string: it is a project the reader has not named, which is what anonymous
/// *is*, and a box holding spaces says exactly as much.
#[test]
fn an_empty_box_is_a_project_that_has_not_been_named() {
    assert_eq!(OpenProject::default().details(), Details::default());

    let blank = OpenProject {
        id: None,
        name: "   ".to_owned(),
        directory: String::new(),
    };
    assert_eq!(blank.details(), Details::default());

    let named = OpenProject {
        id: None,
        name: " kernel ".to_owned(),
        directory: "/src/kernel".to_owned(),
    };
    assert_eq!(
        named.details(),
        Details {
            name: Some("kernel".to_owned()),
            directory: Some(PathBuf::from("/src/kernel")),
        }
    );
}

/// And back the other way, which is what a restore and a switch both do: a project
/// with no name comes back as an empty box rather than as the word "None".
#[test]
fn an_unnamed_project_comes_back_as_an_empty_box() {
    let id = ProjectId::new("project-1").expect("an id");
    let open = OpenProject::opened(id.clone(), &Project::default());
    assert_eq!(open.id, Some(id));
    assert!(open.name.is_empty() && open.directory.is_empty());
    // And a round trip through the two spellings changes nothing.
    assert_eq!(open.details(), Details::default());
}

/// The analysis worker's work, handed in through a context so a test can substitute
/// one that stops when it is told to. `Arc<dyn Fn>` and not a generic, because a
/// context value is one concrete type.
#[derive(Clone)]
struct Study(Arc<dyn Fn(Symbol) -> Studied + Send + Sync>);

/// Every distinct symbol the panes were told to draw, in order. The assertion the
/// superseding rule is really about is not what is on screen at the end but what was
/// *never* on screen, and only a recording can say that.
#[derive(Clone, Copy)]
struct Seen(State<Vec<Symbol>>);

/// The active document as the analysis tests drive it.
///
/// Deliberately not [`Active`], which in the app is a memo over the dock: these tests
/// are about the worker -- which answers reach the panes, and which are dropped -- and
/// have no business building a dock and a document panel to say which symbol is
/// selected. `use_analysis_with` takes anything that reads and peeks, which is what
/// lets them.
#[derive(Clone, Copy)]
struct Selected(State<Option<Document>>);

/// The analysis wiring and nothing else: no panes, since what is under test is which
/// answers reach them rather than what they draw.
fn analysis_harness() -> impl IntoElement {
    let active = use_consume::<Selected>().0;
    let analysis = use_consume::<Analysis>().0;
    let study = use_consume::<Study>().0;
    let mut seen = use_consume::<Seen>().0;

    use_analysis_with(active, analysis, move |symbol| study(symbol));

    use_side_effect(move || {
        let shown = analysis.read().shown.clone();
        let Some(shown) = shown else {
            return;
        };
        // `peek` on the state it writes, or the effect would wake itself for ever.
        let repeat = seen.peek().last().is_some_and(|last| *last == shown.symbol);
        if !repeat {
            seen.write().push(shown.symbol);
        }
    });

    rect().expanded()
}

/// Run the test runner until `ready` answers, and then a little further so that
/// whatever the answer woke has run too.
///
/// A worker thread and two channels sit between a state change and the state it ends
/// in -- the analysis worker's and, since 10c, the scratchpad's -- so how many turns
/// of the loop that takes is not something a test can know, only that it is finite. Failing loudly rather than asserting on what happened to
/// have arrived, since "the answer never came" and "the answer was wrong" are
/// different bugs.
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

/// The committed gcc fixture the analysis crate is pinned against, parsed the way the
/// app parses it. Small, real DWARF, three functions -- so a `Studied` built from one
/// of its symbols has both halves and neither is empty.
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

/// The central correctness question of Step 11c: an answer for a symbol the reader has
/// already clicked past must never reach the panes.
///
/// Staged rather than raced. The worker is a real thread running the real
/// `use_analysis_with` machinery, but the work it does is a gate the test opens one
/// job at a time, which is the only way to be *sure* the stale answer was produced,
/// delivered and then dropped rather than merely being slow. That the test can set the
/// selection twice while the worker sits blocked is itself the other half of the
/// sub-step: the UI thread is not waiting for any of this.
///
/// It also pins the hazard the per-tab viewing position brings: while a symbol is
/// pending, `shown` is not it, so no pane is ever mounted for a tab whose listing does
/// not exist yet -- which is what keeps `use_kept_position` from writing that tab down
/// at row 0 before the reader has seen a single row of it.
#[test]
fn an_answer_for_a_symbol_no_longer_selected_is_dropped() {
    let symbols = fixture_symbols();
    let (first, second) = (symbols[0].clone(), symbols[1].clone());

    // The worker announces each job as it takes it and then waits to be let go.
    // `async_channel` on both sides and not `std::sync::mpsc`, whose `Receiver` is not
    // `Sync` and so cannot sit inside a shared `Fn`.
    let (started, starts) = async_channel::unbounded::<Symbol>();
    let (gate, gated) = async_channel::unbounded::<()>();
    let study = move |symbol: Symbol| {
        let _ = started.send_blocking(symbol.clone());
        let _ = gated.recv_blocking();
        Studied::new(symbol)
    };

    let (mut test, (selection, analysis, seen)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        move |runner| {
            runner.provide_root_context(|| Study(Arc::new(study)));
            (
                runner
                    .provide_root_context(|| Selected(State::create(None)))
                    .0,
                runner
                    .provide_root_context(|| Analysis(State::create(Analyzed::default())))
                    .0,
                runner
                    .provide_root_context(|| Seen(State::create(Vec::new())))
                    .0,
            )
        },
        1.,
    );
    let mut selection = selection;
    let settle = |test: &mut TestingRunner| {
        for _ in 0..8 {
            test.sync_and_update();
        }
    };
    settle(&mut test);

    // The first click. The worker takes it and stops inside it.
    selection.set(Some(Document::Assembly(Selection::Symbol(first.clone()))));
    pump(&mut test, || !starts.is_empty());
    assert!(starts.recv_blocking().expect("the worker started") == first);
    assert!(
        analysis.peek().shown.is_none(),
        "the pane was handed a listing the worker has not produced"
    );

    // The second click, while the first is still being worked on. That the UI takes
    // it at all is the other half of what this sub-step is for.
    selection.set(Some(Document::Assembly(Selection::Symbol(second.clone()))));
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
    assert!(analysis.peek().pending.as_ref() == Some(&second));

    // And the answer that is wanted lands.
    gate.send_blocking(()).expect("the gate");
    pump(&mut test, || analysis.peek().shown.is_some());

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the second symbol was analysed");
    assert!(shown.symbol == second);
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

    let (mut test, (selection, analysis, seen)) = TestingRunner::new(
        analysis_harness,
        (100., 100.).into(),
        |runner| {
            runner.provide_root_context(|| Study(Arc::new(Studied::new)));
            (
                runner
                    .provide_root_context(|| Selected(State::create(None)))
                    .0,
                runner
                    .provide_root_context(|| Analysis(State::create(Analyzed::default())))
                    .0,
                runner
                    .provide_root_context(|| Seen(State::create(Vec::new())))
                    .0,
            )
        },
        1.,
    );
    let mut selection = selection;
    test.sync_and_update();

    selection.set(Some(Document::Assembly(Selection::Symbol(symbol.clone()))));
    pump(&mut test, || analysis.peek().shown.is_some());

    let state = analysis.peek().clone();
    let shown = state.shown.expect("the symbol was analysed");
    assert!(shown.symbol == symbol);
    assert!(state.pending.is_none());
    let assembly = shown.assembly.expect("sum_to holds code");
    assert!(!assembly.instructions.is_empty());
    let lines = shown.lines.info.expect("the fixture has DWARF");
    assert!(!lines.files().is_empty());
    assert!(shown
        .lines
        .file
        .as_deref()
        .is_some_and(|file| file.ends_with("line_fixture.c")));
    assert_eq!(seen.peek().len(), 1);

    // Selecting something that is not a symbol is answered on the spot: clearing does
    // not wait on the worker, only replacing does.
    selection.set(None);
    test.sync_and_update();
    assert!(analysis.peek().clone() == Analyzed::default());
}

/// What the panes are told to say, which is a rule about honesty rather than about
/// pixels: a listing is replaced by the next listing and never by a blank, a wait is
/// only named once it is long enough to have been noticed, and "no symbol selected" is
/// said only when none is.
#[test]
fn nothing_is_said_until_the_wait_is_worth_saying() {
    let symbol = fixture_symbols().into_iter().next().expect("a symbol");
    let studied = Studied::new(symbol.clone());

    let idle = Analyzed::default();
    assert!(matches!(
        idle.showing(),
        Showing::Message("No symbol selected")
    ));

    // Nothing analysed yet and something on its way: an empty pane, not a message.
    let opening = Analyzed {
        pending: Some(symbol.clone()),
        ..Analyzed::default()
    };
    assert!(matches!(opening.showing(), Showing::Nothing));

    // The same wait, once it has gone on long enough to name.
    let slow = Analyzed {
        slow: true,
        ..opening.clone()
    };
    assert!(matches!(slow.showing(), Showing::Message("Analysing...")));

    // A listing in hand is drawn, and goes on being drawn while the next one is worked
    // out -- which is what keeps a click from flashing the pane empty.
    let showing = Analyzed {
        shown: Some(studied),
        ..idle
    };
    assert!(matches!(showing.showing(), Showing::Listing(_)));
    let replacing = Analyzed {
        pending: Some(symbol),
        ..showing
    };
    assert!(matches!(replacing.showing(), Showing::Listing(_)));
    // Until the wait is worth naming, and then the stale listing gives way to it.
    let dragging = Analyzed {
        slow: true,
        ..replacing
    };
    assert!(matches!(
        dragging.showing(),
        Showing::Message("Analysing...")
    ));
}

/// A component with no props at all, which is what every view in this file is: the
/// six dock tabs, every row of every list. Its parent reads nothing coloured, so
/// freya has no reason to re-render it -- the theme has to reach it on its own.
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

/// The same row, under the wiring that resolves the theme -- with the choice handed in
/// rather than loaded, so that the settings file on the machine running the tests has
/// no vote in what they assert.
///
/// The root reads the appearance as well, which is not decoration: `app()` does the
/// same (twice, for freya's own theme sheet), so the write `use_theme` makes during the
/// render body wakes the very scope that made it. That settles only because the write
/// is idempotent, and a test that hangs here is what would say it is not.
fn desktop_theme_harness() -> impl IntoElement {
    use_theme(ThemeChoice::Desktop);
    let _ = appearance();

    rect().expanded().child(ThemedRow)
}

/// The first background anything paints, which is the row's: the harness's own rect
/// has none, and a transparent background is what "none" is.
fn painted(test: &TestingRunner) -> Fill {
    test.find(|_, element| {
        let background = element.style().background.clone();
        (background != Fill::Color(Color::TRANSPARENT)).then_some(background)
    })
    .expect("a painted row")
}

/// `HIGHLIGHTED` is process-wide while the appearance is per-thread, so the two tests
/// that switch themes have to be the only one doing it at a time -- cargo runs them on
/// threads of their own, and one clearing the cache the other has just filled would be
/// a failure that comes and goes.
static SWITCHING: Mutex<()> = Mutex::new(());

/// The reactivity half of dark mode: a switch repaints a component that did not change
/// and whose parent did not either.
///
/// This is the assertion the design is for. Nothing about `ThemedRow` differs across
/// the switch -- same type, same (absent) props, same parent element -- so freya will
/// not re-render it for any reason except that it read the state that changed. Asking
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

/// The other half: the source pane's spans are cached with the palette resolved into
/// them, so a switch has to throw the cache away and parse again in the new colours.
/// Nothing re-renders a `SyntaxBlocks`, which is why this cannot be left to the
/// reactivity above.
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

/// The rule the two enums exist to express: a named theme is its own answer, and
/// `Desktop` is the only one of the three that asks the window anything.
///
/// Pure, so the whole matrix is six lines and needs no window -- which is why
/// `resolve_appearance` takes the platform's answer as an argument instead of reading
/// it. What replaced the old subprocess is not testable at all on the machine running
/// this; the rule in front of it is entirely.
#[test]
fn only_following_the_desktop_asks_the_desktop() {
    for preferred in [PreferredTheme::Light, PreferredTheme::Dark] {
        assert_eq!(
            resolve_appearance(ThemeChoice::Light, preferred),
            Appearance::Light
        );
        assert_eq!(
            resolve_appearance(ThemeChoice::Dark, preferred),
            Appearance::Dark
        );
    }

    assert_eq!(
        resolve_appearance(ThemeChoice::Desktop, PreferredTheme::Light),
        Appearance::Light
    );
    assert_eq!(
        resolve_appearance(ThemeChoice::Desktop, PreferredTheme::Dark),
        Appearance::Dark
    );
}

/// The half of dark mode that the subprocess could never have: the windowing system
/// changing its mind about the theme, *after* the window is open, repaints it.
///
/// freya keeps `Platform::preferred_theme` from winit's `Window::theme()` and re-sets
/// it on the OS's `ThemeChanged` event, so setting it here is exactly what that event
/// does -- and what this asserts is the path from there to `set_appearance` and out to
/// a component that reads no props and was woken by nothing else.
#[test]
fn a_desktop_that_changes_its_mind_repaints_the_window() {
    let _switching = SWITCHING.lock().unwrap_or_else(|error| error.into_inner());
    // Left on the wrong one on purpose, so that the mount below has to be a real write
    // rather than a value that happened to already be there.
    set_appearance(Appearance::Dark);

    // `provide_root_context` runs its closure in the root scope, where freya-testing
    // has already put the `Platform` -- so this is how a test gets hold of the states
    // a renderer would otherwise be the only writer of.
    let (mut test, platform) = TestingRunner::new(
        desktop_theme_harness,
        (100., 100.).into(),
        |runner| runner.provide_root_context(Platform::get),
        1.,
    );
    test.sync_and_update();

    // freya-testing mounts on `PreferredTheme::Light`, and the choice is a question,
    // so the answer arrived on the first render: the appearance the thread was left in
    // is gone, and nothing had to be set by hand to do it.
    assert_eq!(appearance(), Appearance::Light);
    assert_eq!(painted(&test), Fill::Color(Palette::LIGHT.pane_bg));

    // **Two passes, and the second is not padding.** The change reaches the window in
    // two hops -- the platform state wakes the scope holding `use_theme`, and the write
    // that scope makes wakes everything that drew a colour -- and a pass renders the
    // dirty scopes it *began* with, so the second hop lands in the pass after the
    // first. The renderer does the same thing on its own (a marked scope sends a
    // message that brings its loop straight back round and requests a redraw), so the
    // cost of resolving the theme in the render body rather than an effect is one
    // frame, spelled out here rather than hidden behind a loop that polls until it
    // likes the answer.
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

/// sRGB relative luminance, and the contrast ratio between two colours, both as WCAG
/// defines them. Written out rather than pulled in: it is eight lines, and a
/// dependency for eight lines used by two tests is not a trade.
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

/// Every foreground is legible on the surface it is actually drawn on, in both
/// palettes.
///
/// The floor is 3.0 and not WCAG AA's 4.5 on purpose. Two of the light palette's own
/// colours sit between 3 and 3.5 -- the address column and comments, both of which are
/// *meant* to recede -- and this test is not here to redesign the light theme that has
/// been on screen since 5e. It is here so that a value carried over to a dark ground
/// cannot land on top of it: a foreground that came out at 1.5 would be a colour
/// nobody can read, and that is what a second palette gets wrong.
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
        }

        for (name, color) in [
            ("string_fg", palette.string_fg),
            ("comment_fg", palette.comment_fg),
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

        // The branch gutter is a diagram and is drawn quiet deliberately -- 1.8 in the
        // light palette -- so its floor is only against a line that has disappeared
        // into the pane altogether, and the hovered one has to be the louder of the
        // two or hovering a row says nothing.
        let line = contrast(palette.branch_fg, palette.asm_pane_bg);
        let lit = contrast(palette.branch_hover_fg, palette.asm_pane_bg);
        assert!(line >= 1.5, "{theme} branch_fg: {line:.2}");
        assert!(lit > line, "{theme} branch_hover_fg: {lit:.2} vs {line:.2}");
    }
}

/// Every translucent wash still says something once it is composited.
///
/// This is the half of a palette that cannot be carried over by turning its channels
/// through the background: `blend` puts the pane under these, so the same alpha over a
/// dark ground is a fraction of the step it was over white. Each is asserted as what
/// it comes out as -- and the pin, which is the focus said louder, has to stay louder.
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
                palette.asm_pane_bg,
            ),
            ("line_focus_bg", palette.line_focus_bg, palette.asm_pane_bg),
            ("line_pin_bg", palette.line_pin_bg, palette.asm_pane_bg),
            ("row_select_bg", palette.row_select_bg, palette.asm_pane_bg),
            ("drop_preview_bg", palette.drop_preview_bg, palette.pane_bg),
        ] {
            let step = step(wash, ground);
            assert!(step >= 10, "{theme} {name}: {step} levels");
        }

        let focus = step(palette.line_focus_bg, palette.asm_pane_bg);
        let pin = step(palette.line_pin_bg, palette.asm_pane_bg);
        assert!(pin > focus, "{theme} pin {pin} vs focus {focus}");
    }
}

/// The `resolve_capture_color` trap, in both palettes.
///
/// It decides a capture is unmapped by comparing its colour to `text` and then walks
/// *up* the dotted name, so a child field holding the text colour while its parent
/// holds another is silently painted in the parent's. Nothing in either mapping is
/// caught by it -- but that is a fact about which fields share a value, so a second
/// palette can break it by landing two colours on each other by accident.
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
/// asserts a size and not whatever `kreadconfig` happens to answer on the machine
/// running it -- `needs_desktop` declines to spawn anything when both halves are
/// chosen, which is exactly the case here.
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

/// Two components with no props at all, one row at each of the two heights.
/// `ThemedRow`'s twins, and for the same reason: nothing about either changes across a
/// font change, so freya has no reason to re-render them except that they read the
/// state. Their backgrounds differ so that `painted_height` can ask for one of them by
/// name rather than by which came first.
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

/// The height of the row painted in `fill`, as it was actually laid out -- not as it
/// was asked for. That distinction is the test: a row height function returning a new
/// number proves nothing on its own, since a component that was never re-rendered is
/// still the old height on screen.
fn painted_height(test: &TestingRunner, fill: Color) -> f32 {
    test.find(|node, element| {
        let background = element.style().background.clone();
        (background == Fill::Color(fill)).then(|| node.layout().area.height())
    })
    .expect("a painted row")
}

/// The reactivity half of 9c, and the direct analogue of the theme's: a font change
/// repaints a component nothing else woke, *and* moves it, since the row heights are
/// derived from the fonts rather than being constants beside them.
///
/// It is also where the two heights are asserted to be **independent**, which is the
/// whole of the split: no row mixes the fonts, so a size the reader steps must move
/// the rows drawn in *that* font and no others. 9pt and 10.5pt are the app's own
/// defaults -- 12 and 14 logical pixels, so 24 and 26 -- and each of the two changes
/// below leaves the other row exactly where it was.
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

/// The invariant that made `ROW_HEIGHT` a `const` in the first place: a
/// `VirtualScrollView`'s `item_size` and the height its rows actually draw at must be
/// the same number, or scrolling misaligns -- silently, and looking like a rendering
/// glitch rather than a bug.
///
/// **It is two claims since the height was split in two**, so it is asserted over both
/// kinds of list: a code pane, whose rows and `item_size` are [`code_row_height`] and
/// which is the only kind with a kept position, and a sidebar list at
/// [`list_row_height`]. A view handed the *other* height would misalign exactly as one
/// handed a stale one would, and only a view of each kind can catch that.
///
/// Asserted through real scroll views, by asking which row is under a given y: at the
/// top of the list row *k* covers `[k*h, (k+1)*h)`, so a pointer at 90 is row 3 at 26px
/// and row 2 at 36px. If the two numbers came apart, the rows would drift by one per
/// row down the pane and this would answer something else. Each half also steps the
/// font it is *not* drawn in and asserts that nothing moved.
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

    // A font change wakes the rows through the state they read, and the view they sit
    // in re-measures behind them; several passes because the scroll view answers the
    // new item size on the render after the one that moved its rows.
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

/// What the settings page's four boxes mean, which is the one place its `String`s and
/// `settings.toml`'s absent keys meet -- `an_empty_box_is_a_project_that_has_not_been_named`
/// for fonts. An empty family box is not a font family named the empty string: it is a
/// reader who has not chosen one, which is what unspecified *is*.
#[test]
fn an_empty_box_is_a_font_nobody_chose() {
    assert_eq!(EditedSettings::default().settings(), Settings::default());

    let blank = EditedFont {
        family: "   ".to_owned(),
        size: None,
    };
    assert_eq!(blank.setting(), FontSetting::default());

    // And a round trip through the two spellings changes nothing, in either direction:
    // the page is handed what the file says and hands back the same thing.
    let stored = Settings {
        theme: ThemeChoice::Dark,
        interface: FontSetting {
            family: Some("Cantarell".to_owned()),
            size: Some(11.0),
        },
        fixed: FontSetting {
            family: None,
            size: Some(10.5),
        },
    };
    assert_eq!(EditedSettings::of(&stored).settings(), stored);

    // A family the file wrote with spaces around it comes back trimmed, once, and does
    // not then differ from itself on the way out.
    let padded = Settings {
        interface: FontSetting {
            family: Some(" Fira Code ".to_owned()),
            ..FontSetting::default()
        },
        ..Settings::default()
    };
    let edited = EditedSettings::of(&padded);
    assert_eq!(edited.interface.family, "Fira Code");
    assert_eq!(edited.settings(), edited.settings());
}

/// A point size as the page writes it. `9` and not `9.0`, because the size a desktop
/// answers is usually a whole number and a trailing `.0` on every one of them reads as
/// precision that is not there; `10.5` because half-points are what the stepper moves
/// in and what Pango descriptions carry.
#[test]
fn a_point_size_is_written_as_short_as_it_is() {
    assert_eq!(points_text(9.0), "9");
    assert_eq!(points_text(10.5), "10.5");
    assert_eq!(points_text(26.0), "26");
    // Gnome's `text-scaling-factor` multiplies the point size, so a third decimal is
    // reachable without anybody typing one.
    assert_eq!(points_text(13.75), "13.8");
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

    // The **code** row, because what this test steps is the fixed-width size: it is
    // the one whose consequences reach a file, a theme and a row all at once, and a
    // row drawn in the other font would now sit still through the whole of it.
    rect().expanded().child(FontedCodeRow)
}

/// The wiring 9c is: one state, and the theme, the fonts and the file all following
/// from it -- with the write handed in, because the real one edits the settings of
/// whoever runs the tests.
///
/// Three things are asserted that nothing else can say. That a run in which the page is
/// never opened writes **nothing**, so a first launch leaves no `settings.toml` behind.
/// That a change reaches all three consequences from the one write. And that changing a
/// setting *back* writes again -- the baseline moves to what was last written, or the
/// file would be left holding the middle answer of the three.
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

    // A theme chosen. Two passes, for `a_desktop_that_changes_its_mind_repaints_the_window`'s
    // reason: the write the root makes wakes the scopes that drew a colour in the pass
    // after the one it was made in.
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

    // Cleared again, which is the whole of "a way back to unspecified": the override is
    // gone from the file, and the write happens even though this is the value the run
    // started from -- the baseline is what was last *written*, not what was loaded.
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

/// The scratchpad worker's work, handed in through a context so a test can answer
/// without the machine's own state directory and without waiting on a compiler.
/// `Arc<dyn Fn>` and not a generic, for [`Study`]'s reason: a context value is one
/// concrete type.
#[derive(Clone)]
struct Working(Arc<dyn Fn(PadJob) -> PadAnswer + Send + Sync>);

/// The way to ask the worker for a build, as the wiring hands it back. A `State` so
/// that the harness can put it somewhere the test body can reach, which is what lets
/// a build be asked for the way the button asks rather than through coordinates.
#[derive(Clone, Copy)]
struct Asking(State<Option<PadJobs>>);

/// What the worker was handed, in the order it was handed it. The `Save`s carry the
/// source they would have written, because *what* was written is half of what the
/// save policy is for.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Open,
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

/// The committed gcc fixture again, standing in for what a build produced: `open_files`
/// asks nothing of a file but that it parse, so a relocatable object is an artifact as
/// far as everything this test is about is concerned.
fn fixture_artifact() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/analysis/tests/fixtures/line_fixture.o")
}

/// Mount the wiring over a worker that records every job and answers from `answer`.
///
/// A macro rather than a function for `project_states!`'s reason -- the runner's type
/// is not one this crate can name -- and it hands back everything a test then drives:
/// the app's states, the scratchpad's two, the way to ask for a build and the record
/// of what was asked.
macro_rules! mount_scratchpad {
    ($harness:expr, $answer:expr) => {{
        let (asked, asks) = async_channel::unbounded::<Asked>();
        let answer = $answer;
        let work = move |job: PadJob| {
            let recorded = match &job {
                PadJob::Open(_) => Asked::Open,
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
                    .provide_root_context(|| Pad(State::create(PadState::default())))
                    .0;
                let text = runner
                    .provide_root_context(|| {
                        PadText(State::create(CodeEditorData::new(
                            Rope::from_str(""),
                            language(Path::new(SOURCE_FILE)),
                        )))
                    })
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

/// The scratchpad on disk is what the app opens on, and **nothing is written until it
/// has arrived**.
///
/// That second half is the whole reason the save baseline is seeded by the answer and
/// not at mount: the app boots holding `Scratchpad::default`, the reader's own source
/// comes back a worker thread later, and a save in between would put the default
/// source over a scratchpad someone had been keeping. It is also what keeps a run in
/// which the pane was never opened from creating the directory at all.
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
            PadJob::Open(_) => PadAnswer::Opened(answering.clone()),
            PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().opened);

    assert_eq!(pad.peek().scratchpad, saved);
    // The editor is holding it too, which is the half a reader can see: the buffer is
    // the live copy and the model follows it, so a restore that reached only the model
    // would be a pane showing the default source over a scratchpad that is not it.
    assert_eq!(text.peek().rope.to_string(), saved.source);

    assert_eq!(asks.try_recv(), Ok(Asked::Open));
    assert!(
        asks.is_empty(),
        "the package was written before the app knew what was in it"
    );
}

/// An edit is written out, and a row that cannot be written says so against itself.
///
/// Both halves go through the one policy: the model follows the editor, the effect
/// notices it differs from what was last sent, and the worker answers. What comes back
/// for a bad row is `Failure::Dependencies`, carrying the **index** of every row that
/// is wrong -- which is what lets the pane mark them in place rather than printing one
/// sentence at the top.
#[test]
fn an_edit_is_written_and_a_bad_row_says_which_row() {
    let (mut test, _states, pad, text, _asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            // The real refusal, without a disk: `write` fails on exactly what
            // `manifest` fails on, the manifest being what it refuses to generate.
            PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().opened);
    assert_eq!(asks.try_recv(), Ok(Asked::Open));

    // Typing. The rope is what the keyboard edits and the model is what is written, so
    // this is the same path a keystroke takes.
    let mut text = text;
    text.write().rope.insert(0, "// typed\n");
    pump(&mut test, || !asks.is_empty());

    let typed = format!("// typed\n{}", crate::scratchpad::DEFAULT_SOURCE);
    assert_eq!(asks.try_recv(), Ok(Asked::Save(typed.clone())));
    assert_eq!(pad.peek().scratchpad.source, typed);
    assert!(pad.peek().unsaved.is_none());

    // A row that names no crate. It is the *second* row, so the index in the answer is
    // the assertion: a failure that only said "one dependency to fix" would leave the
    // pane guessing which.
    let mut pad = pad;
    {
        let mut state = pad.write();
        state.scratchpad.dependencies = vec![
            Dependency {
                name: "anyhow".to_owned(),
                version: "1.0.86".to_owned(),
            },
            Dependency::default(),
        ];
    }
    pump(&mut test, || pad.peek().unsaved.is_some());

    assert_eq!(
        pad.peek().unsaved,
        Some(Failure::Dependencies(vec![(1, Problem::NoName)]))
    );

    // And fixing it writes again, rather than leaving the disk holding the last good
    // version for ever.
    pad.write().scratchpad.dependencies[1] = Dependency {
        name: "rand".to_owned(),
        version: "0.8".to_owned(),
    };
    pump(&mut test, || pad.peek().unsaved.is_none());
}

/// A build is asked for once however often the reader presses, and what it made is
/// opened **in place of** what the build before it made.
///
/// Both halves are about the same thing being true twice. A build takes seconds, so
/// the pending state has to be honest enough that a second press cannot start a second
/// one; and a rebuild writes the same path with different bytes, so the objects the app
/// is holding for that path describe instructions that are no longer there. Opening
/// without closing would leave two generations of one file in a list where a binary is
/// identified by its path.
#[test]
fn a_build_runs_once_and_replaces_what_the_last_one_opened() {
    let artifact = fixture_artifact();
    let built = artifact.clone();
    let (mut test, states, pad, _text, asking, asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(_) => PadAnswer::Saved(None),
            PadJob::Build(_) => PadAnswer::Built(Build::Built {
                executable: built.clone(),
                diagnostics: Vec::new(),
            }),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().opened);
    assert_eq!(asks.try_recv(), Ok(Asked::Open));

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_build(pad, &jobs);
    // The second press, while the first is still in flight. Nothing at all happens.
    request_build(pad, &jobs);
    assert!(pad.peek().building);

    pump(&mut test, || !states.objects.peek().is_empty());
    assert!(!pad.peek().building);
    assert!(matches!(pad.peek().built, Some(Build::Built { .. })));

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
        Ok(Asked::Build(pad.peek().scratchpad.source.clone()))
    );
    assert!(
        asks.is_empty(),
        "the second press started a second build of the same scratchpad"
    );

    // And again. The path is the same one, so what the first build left has to go
    // rather than sit beside it.
    request_build(pad, &jobs);
    // Waited for on the *objects* and not on the build, because a rebuild is now a
    // close followed by a streaming reopen: the build is over the moment cargo has
    // answered, and the artifact's objects come back over the load after it.
    pump(&mut test, || !pad.peek().building && opened(&states) > 0);

    assert_eq!(
        opened(&states),
        first,
        "a rebuild left the objects of the build before it in the list"
    );
}

/// Taking a dependency row away does not take the pane with it.
///
/// The hazard is the one the gotchas list is about, and it is invisible to every other
/// kind of test here: each box in a row writes into `dependencies[index]` through a
/// mapped `Writable`, so a row that outlived the list being shortened would index past
/// the end at the moment it was next read -- a panic, not a compile error. Mounting the
/// real pane and shortening the list under it is the only thing that would say so.
#[test]
fn removing_a_dependency_row_does_not_take_the_pane_with_it() {
    let (mut test, _states, pad, _text, _asking, _asks) =
        mount_scratchpad!(scratchpad_view_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(scratchpad) => PadAnswer::Saved(scratchpad.manifest().err()),
            PadJob::Build(_) => unreachable!("this test never builds"),
            PadJob::Run { .. } => unreachable!("this test never runs"),
        });

    pump(&mut test, || pad.peek().opened);

    let mut pad = pad;
    pad.write().scratchpad.dependencies = vec![
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
    pad.write().scratchpad.dependencies.remove(0);
    for _ in 0..4 {
        test.sync_and_update();
    }

    assert_eq!(pad.peek().scratchpad.dependencies.len(), 1);
    assert_eq!(pad.peek().scratchpad.dependencies[0].name(), "rand");
}

/// A directory of this test's own, named after the line that asked for it -- the shape
/// `scratchpad.rs`'s own file tests use, so a failing test leaves something
/// identifiable behind.
fn run_directory(line: u32) -> PathBuf {
    std::env::temp_dir().join(format!(
        "assembly-viewer-run-test-{}-{line}",
        std::process::id()
    ))
}

/// Build a program that says something and then never exits, and say where it is.
///
/// A real `cargo build`, for `scratchpad.rs`'s reason: it is hermetic (no dependencies
/// means no registry, so it is one rustc invocation) and it is the only way to have an
/// executable that behaves the way the hazard this sub-step is about behaves. Nothing
/// short of a real process can say whether a stop actually killed anything.
fn looping_program(directory: &Path) -> PathBuf {
    let mut scratchpad = Scratchpad::new("looper").expect("a name");
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

/// What a build left behind, put where a build would have put it.
///
/// Written into the state rather than answered through `PadJob::Build`, so the
/// artifact does not go through `reopen_binary` on the way: what that does with a
/// rebuilt binary is `a_build_runs_once_and_replaces_what_the_last_one_opened`'s
/// question, and it would cost these tests a parse of a real executable for nothing.
fn already_built(mut pad: State<PadState>, executable: PathBuf) {
    pad.write().built = Some(Build::Built {
        executable,
        diagnostics: Vec::new(),
    });
}

/// The two things 10d exists to make true, and only a real process can say either.
///
/// A program that prints and then loops for ever has **said something**, and it is on
/// screen while it is still going -- which is the whole difference between this and
/// `build_in`'s collect-the-output-and-return-it shape, since this program has no exit
/// for such a shape to answer at. And asking it to stop **really kills it**: the state
/// reaches `Over(Stopped)` only when the run's own `Ended` event arrives, and that is
/// emitted after the process has been reaped -- so this waits for the process to be
/// gone rather than for the button to have been pressed.
#[test]
fn a_run_streams_while_it_is_going_and_a_stop_really_ends_it() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(_) => PadAnswer::Saved(None),
            // Nothing about the run is faked: the real spawn, the real pipes and the
            // real kill, reached through the same job the button sends.
            PadJob::Run {
                run,
                executable,
                emit,
                ..
            } => PadAnswer::Started(run, crate::scratchpad::run_in(&executable, &cwd, emit)),
            PadJob::Build(_) => unreachable!("this test never builds"),
        });

    pump(&mut test, || pad.peek().opened);
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);

    pump(&mut test, || pad.peek().output.len() > 0);
    let state = pad.peek().clone();
    assert_eq!(
        state
            .output
            .line(0)
            .map(|line| (line.stream, line.text.to_string())),
        Some((Stream::Out, "from the program".to_owned()))
    );
    assert!(state.is_running(), "it ended by itself");

    stop_run(pad);
    pump(&mut test, || !pad.peek().is_running());
    let state = pad.peek().clone();
    assert!(
        matches!(state.run_state, RunState::Over(Ended::Stopped)),
        "{:?}",
        state.run_status()
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A rebuild stops the program the last one started.
///
/// cargo is about to write over the executable this process *is*, and `reopen_binary`
/// is about to close the objects describing those bytes -- so a program left going
/// across a build would be output arriving into a pane belonging to a build the reader
/// can no longer see. Asserted through `request_build` rather than through the button,
/// because the guard belongs to the request for the reason the two-builds-at-once one
/// does: it has to be a property of asking, not of one control's disabled state.
#[test]
fn a_rebuild_stops_the_program_the_last_one_started() {
    let directory = run_directory(line!());
    let executable = looping_program(&directory);
    let cwd = directory.clone();

    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(_) => PadAnswer::Saved(None),
            PadJob::Run {
                run,
                executable,
                emit,
                ..
            } => PadAnswer::Started(run, crate::scratchpad::run_in(&executable, &cwd, emit)),
            // What the build itself answers does not matter here: the run is stopped
            // on the way to sending the job, before cargo would have been asked
            // anything at all.
            PadJob::Build(_) => PadAnswer::Built(Build::Unavailable(Failure::NoArtifact)),
        });

    pump(&mut test, || pad.peek().opened);
    already_built(pad, executable);
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || pad.peek().output.len() > 0);
    assert!(pad.peek().is_running());

    request_build(pad, &jobs);
    pump(&mut test, || !pad.peek().is_running());
    let state = pad.peek().clone();
    assert!(
        matches!(state.run_state, RunState::Over(Ended::Stopped)),
        "{:?}",
        state.run_status()
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A program that will not start is a sentence, not a pane that sits on "Starting..."
/// for ever. No subprocess: what is under test is that the failure the worker answers
/// with reaches the line the reader reads.
#[test]
fn a_run_that_cannot_start_says_why() {
    let (mut test, _states, pad, _text, asking, _asks) =
        mount_scratchpad!(scratchpad_harness, move |job: PadJob| match job {
            PadJob::Open(scratchpad) => PadAnswer::Opened(scratchpad),
            PadJob::Save(_) => PadAnswer::Saved(None),
            PadJob::Run { run, .. } => PadAnswer::Started(
                run,
                Err(Failure::NoProgram("No such file or directory".to_owned())),
            ),
            PadJob::Build(_) => unreachable!("this test never builds"),
        });

    pump(&mut test, || pad.peek().opened);
    already_built(pad, fixture_artifact());
    test.sync_and_update();

    let jobs = asking.peek().clone().expect("the wiring handed one back");
    request_run(pad, &jobs);
    pump(&mut test, || !pad.peek().is_running());

    let (text, bad) = pad.peek().run_status().expect("a status");
    assert!(text.contains("No such file or directory"), "{text}");
    assert!(bad);
}
